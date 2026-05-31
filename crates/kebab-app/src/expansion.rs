//! 색인시 doc-side expansion (Phase 2) — 청크당 "검색용 별칭" 생성.
//!
//! 설계 spec docs/superpowers/specs/2026-05-30-doc-side-expansion-design.md §3.2 / §5.

use kebab_core::{Chunk, GenerateRequest, LanguageModel};

/// 별칭 1줄의 최대 글자 수(이 이상은 문장형/환각으로 보고 drop).
const MAX_ALIAS_CHARS: usize = 120;

/// 별칭 프롬프트 템플릿 버전. derivation cache 의 alias version_key 에 포함되어
/// (§3.1), 프롬프트를 바꾸면 bump 해 캐시를 무효화한다(전부 miss → 재생성).
/// `build_request` 의 gemma 프롬프트와 한 쌍 — 프롬프트 수정 시 함께 bump.
pub const PROMPT_VERSION: &str = "expansion-v1";

/// 청크당 검색용 별칭을 생성한다.
///
/// 반환: 검증·상한 적용된 별칭들을 개행 join 한 문자열. 생성 0개 / LLM
/// 실패 / 빈 출력이면 `None` (호출측은 chunk.aliases 를 None 으로 두고 진행).
pub struct ExpansionGenerator<'a> {
    llm: &'a dyn LanguageModel,
    max_aliases: usize,
}

impl<'a> ExpansionGenerator<'a> {
    pub fn new(llm: &'a dyn LanguageModel, max_aliases: usize) -> Self {
        Self { llm, max_aliases }
    }

    /// gemma 프롬프트(expansion-v1)를 구성한다. (self 미사용 — associated fn.)
    fn build_request(chunk: &Chunk) -> GenerateRequest {
        let heading = chunk.heading_path.join(" > ");
        let system = "당신은 검색 색인용 별칭 생성기다. 주어진 문단을 찾을 사용자가 \
입력할 법한 짧은 검색어/질문을 생성한다. 동의어·풀어쓴 표현을 포함하라. \
문단이 한국어면 영어 표현도, 영어면 한국어 표현도 섞어라. \
한 줄에 하나씩, 설명·번호·머리기호 없이 검색어만 출력하라."
            .to_string();
        let user = format!(
            "제목 경로: {heading}\n\n문단:\n{}\n\n검색 별칭(한 줄에 하나):",
            chunk.text
        );
        GenerateRequest {
            system,
            user,
            stop: vec![],
            max_tokens: 256,
            temperature: 0.0,
            seed: Some(0),
            images: vec![],
        }
    }

    pub fn generate(&self, chunk: &Chunk) -> Option<String> {
        // 나무위키 네비게이션 boilerplate 청크는 LLM 호출 없이 skip — 별칭
        // 생성 가치가 없고 노이즈 sentinel 벡터만 만든다.
        if is_nav_boilerplate(chunk) {
            return None;
        }
        let req = Self::build_request(chunk);
        let raw = match self.llm.generate_stream(req) {
            Ok(iter) => {
                let mut acc = String::new();
                for ch in iter {
                    match ch {
                        Ok(kebab_core::TokenChunk::Token(t)) => acc.push_str(&t),
                        Ok(kebab_core::TokenChunk::Done { .. }) => {}
                        Err(_) => return None, // fail-soft
                    }
                }
                acc
            }
            Err(_) => return None, // fail-soft (connection refused 등)
        };
        let aliases = parse_aliases(&raw, self.max_aliases);
        if aliases.is_empty() {
            None
        } else {
            Some(aliases.join("\n"))
        }
    }
}

/// 나무위키 네비게이션 boilerplate 청크 판정.
///
/// heading_path 가 비어 있고(문서 본문 섹션이 아닌 머리/꼬리 nav), text 앞부분에
/// nav 키워드("최근 변경" 등)가 하나라도 있으면 boilerplate 로 본다. 둘 다
/// 만족할 때만 true — 정상 본문(heading 있음, 또는 nav 키워드 없음)은 false.
pub fn is_nav_boilerplate(chunk: &Chunk) -> bool {
    const NAV_KEYWORDS: [&str; 5] = [
        "최근 변경",
        "Recent changes",
        "최근 토론",
        "특수 기능",
        "편집 토론 역사",
    ];
    if !chunk.heading_path.is_empty() {
        return false;
    }
    let head: String = chunk.text.chars().take(200).collect();
    NAV_KEYWORDS.iter().any(|kw| head.contains(kw))
}

/// 줄 선두의 목록 마커만 1회 제거한다. **마커 뒤 공백이 필수** — 별칭 내용이
/// 숫자/하이픈/별표로 시작하는 경우(예: "3D 렌더링", "-fast", "2단계")는 보존한다.
/// (Task 4 리뷰 MAJOR-1: 탐욕적 `trim_start_matches` 가 정당한 별칭을 손상시키던 버그 수정.)
fn strip_list_marker(s: &str) -> &str {
    // 1) 머리기호 + 공백 ("- " / "* " / "• ").
    for marker in ["- ", "* ", "• "] {
        if let Some(rest) = s.strip_prefix(marker) {
            return rest.trim_start();
        }
    }
    // 2) 번호 + ('.' | ')') + 공백 ("1. " / "2) "). 마커 뒤 공백이 없으면
    //    ("3D", "2단계") 번호가 아니라 내용으로 보고 보존.
    let digit_end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    if digit_end > 0 {
        let after = &s[digit_end..];
        if let Some(rest) = after.strip_prefix(". ").or_else(|| after.strip_prefix(") ")) {
            return rest.trim_start();
        }
    }
    s
}

/// LLM 출력 문자열 → 검증된 별칭 리스트.
/// 줄 단위 split → trim → 목록 마커 1회 제거 → 빈 줄·과길이 drop →
/// 중복 제거 → 상한 N.
fn parse_aliases(raw: &str, max_aliases: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in raw.lines() {
        let t = strip_list_marker(line.trim());
        if t.is_empty() || t.chars().count() > MAX_ALIAS_CHARS {
            continue;
        }
        let s = t.to_string();
        if !out.contains(&s) {
            out.push(s);
        }
        if out.len() >= max_aliases {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use kebab_core::{ChunkId, ChunkerVersion, DocumentId, FinishReason, TokenUsage};
    use kebab_llm::MockLanguageModel;

    fn mk_chunk(text: &str) -> Chunk {
        Chunk {
            chunk_id: ChunkId("c1".into()),
            doc_id: DocumentId("d1".into()),
            block_ids: vec![],
            text: text.into(),
            heading_path: vec!["Guide".into()],
            source_spans: vec![],
            token_estimate: 3,
            chunker_version: ChunkerVersion("md-heading-v1".into()),
            policy_hash: "h".into(),
            tokenized_korean_text: None,
            aliases: None,
        }
    }

    fn mock(resp: &str) -> MockLanguageModel {
        MockLanguageModel {
            model_id: "gemma4:e4b".into(),
            provider: "ollama".into(),
            context_tokens: 32768,
            canned_response: resp.into(),
            canned_finish: FinishReason::Stop,
            canned_usage: TokenUsage {
                prompt_tokens: 0,
                completion_tokens: 0,
                latency_ms: 0,
            },
        }
    }

    #[test]
    fn parses_lines_strips_bullets_and_caps() {
        let llm = mock("- 메모리 안전성\n1. who owns the value\nborrow checker\n\n* 소유권");
        let generator = ExpansionGenerator::new(&llm, 2);
        let out = generator.generate(&mk_chunk("Rust ownership")).unwrap();
        // 상한 2 → 앞 2개만, 접두 제거됨.
        assert_eq!(out, "메모리 안전성\nwho owns the value");
    }

    #[test]
    fn drops_overlong_lines() {
        let long = "x".repeat(200);
        let llm = mock(&format!("{long}\n짧은 별칭"));
        let generator = ExpansionGenerator::new(&llm, 8);
        let out = generator.generate(&mk_chunk("t")).unwrap();
        assert_eq!(out, "짧은 별칭", "120자 초과 줄은 drop");
    }

    #[test]
    fn empty_output_returns_none() {
        let llm = mock("   \n\n");
        let generator = ExpansionGenerator::new(&llm, 8);
        assert_eq!(generator.generate(&mk_chunk("t")), None);
    }

    /// Task 4 리뷰 MAJOR-1 회귀: 숫자/하이픈/별표로 시작하는 정당한 별칭은
    /// 손상 없이 보존돼야 한다(목록 마커는 마커 뒤 공백이 있을 때만 제거).
    #[test]
    fn preserves_numeric_and_dash_leading_aliases() {
        let llm = mock("3D 렌더링\n2단계 커밋\n-fast 플래그\n- 메모리 안전성\n1. 첫 항목");
        let generator = ExpansionGenerator::new(&llm, 8);
        let out = generator.generate(&mk_chunk("graphics")).unwrap();
        // 마커 없는 선두 숫자/하이픈은 보존; "- "/"1. " 만 마커로 제거.
        assert_eq!(out, "3D 렌더링\n2단계 커밋\n-fast 플래그\n메모리 안전성\n첫 항목");
    }

    fn mk_chunk_nav(text: &str, heading: Vec<String>) -> Chunk {
        let mut c = mk_chunk(text);
        c.heading_path = heading;
        c
    }

    #[test]
    fn nav_boilerplate_skips_alias_generation() {
        // heading 없음 + nav 키워드 → boilerplate → LLM 호출 전에 None.
        let llm = mock("별칭1\n별칭2");
        let generator = ExpansionGenerator::new(&llm, 8);
        let chunk = mk_chunk_nav("최근 변경 최근 토론 특수 기능", vec![]);
        assert_eq!(generator.generate(&chunk), None);
    }

    #[test]
    fn normal_body_chunk_generates_aliases() {
        // heading 없지만 nav 키워드도 없음 → 정상 본문 → 별칭 생성.
        let llm = mock("별칭1\n별칭2");
        let generator = ExpansionGenerator::new(&llm, 8);
        let chunk = mk_chunk_nav("러스트의 소유권과 빌림 검사기 개요", vec![]);
        assert_eq!(generator.generate(&chunk).unwrap(), "별칭1\n별칭2");
    }

    #[test]
    fn nav_keyword_with_heading_is_not_boilerplate() {
        // nav 키워드가 있어도 heading 이 있으면 본문 섹션 → 생성.
        let llm = mock("별칭1");
        let generator = ExpansionGenerator::new(&llm, 8);
        let chunk = mk_chunk_nav("최근 변경 내역 설명", vec!["문서 변경사항".into()]);
        assert_eq!(generator.generate(&chunk).unwrap(), "별칭1");
    }

    #[test]
    fn is_nav_boilerplate_unit() {
        assert!(is_nav_boilerplate(&mk_chunk_nav("Recent changes list", vec![])));
        assert!(is_nav_boilerplate(&mk_chunk_nav("편집 토론 역사", vec![])));
        assert!(!is_nav_boilerplate(&mk_chunk_nav("일반 본문 텍스트", vec![])));
        assert!(!is_nav_boilerplate(&mk_chunk_nav(
            "최근 변경",
            vec!["섹션".into()]
        )));
    }

    #[test]
    fn strip_list_marker_unit() {
        assert_eq!(strip_list_marker("- 메모리"), "메모리");
        assert_eq!(strip_list_marker("* 소유권"), "소유권");
        assert_eq!(strip_list_marker("1. who owns"), "who owns");
        assert_eq!(strip_list_marker("2) 항목"), "항목");
        // 마커 뒤 공백 없음 → 보존.
        assert_eq!(strip_list_marker("3D 렌더링"), "3D 렌더링");
        assert_eq!(strip_list_marker("-fast"), "-fast");
        assert_eq!(strip_list_marker("2단계"), "2단계");
        assert_eq!(strip_list_marker("2.0 릴리스"), "2.0 릴리스");
    }
}
