//! 색인시 doc-side expansion (Phase 2) — 청크당 "검색용 별칭" 생성.
//!
//! 설계 spec docs/superpowers/specs/2026-05-30-doc-side-expansion-design.md §3.2 / §5.

use kebab_core::{Chunk, GenerateRequest, LanguageModel};

/// 별칭 1줄의 최대 글자 수(이 이상은 문장형/환각으로 보고 drop).
const MAX_ALIAS_CHARS: usize = 120;

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

    /// gemma 프롬프트(expansion-v1)를 구성한다.
    fn build_request(&self, chunk: &Chunk) -> GenerateRequest {
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
        let req = self.build_request(chunk);
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

/// LLM 출력 문자열 → 검증된 별칭 리스트.
/// 줄 단위 split → trim → 번호/머리기호 접두 제거 → 빈 줄·과길이 drop →
/// 중복 제거 → 상한 N.
fn parse_aliases(raw: &str, max_aliases: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in raw.lines() {
        let t = line.trim();
        // 번호("1." "1)") / 머리기호("- " "* ") 접두 제거.
        let t = t
            .trim_start_matches(|c: char| {
                c.is_ascii_digit() || c == '.' || c == ')' || c == '-' || c == '*'
            })
            .trim();
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
}
