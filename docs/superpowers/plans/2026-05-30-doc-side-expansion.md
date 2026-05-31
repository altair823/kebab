# 색인시 doc-side expansion (검색용 별칭) 구현 Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 문서 색인 시 각 청크마다 로컬 LLM(gemma)으로 "검색용 별칭"(같은언어 paraphrase + 한↔영 번역)을 1회 생성해 별도 FTS5 테이블에 저장하고, lexical 검색이 본문+별칭을 함께 조회해 어휘격차로 pool 에서 누락되던 정답을 회수한다.

**Architecture:** 별도 `chunk_aliases_fts` 가상 테이블(기존 `chunks_fts` §5.5 verbatim 블록 무수정) + `chunks.aliases` 컬럼 + 별도 sync trigger. ingest 경로에 flag(`[ingest.expansion]`, default off) 게이트로 `ExpansionGenerator`(LanguageModel trait, mock 가능) hook. 검색은 `LexicalRetriever` 가 본문 쿼리 + 별칭 쿼리 결과를 Rust 에서 병합(body 우선, alias-only append) — `HybridRetriever`/`RetrievalDetail`/wire schema 무변경. 별칭 테이블이 비면 기존과 동일 동작(회귀 안전).

**Tech Stack:** Rust 2024 workspace, rusqlite + FTS5(unicode61), refinery migrations, `kebab_llm::LanguageModel`(Ollama), `kebab-eval` variants 측정.

**빌드/테스트 규약 (모든 Run 스텝에 적용):**
- `CARGO_TARGET_DIR=/build/out/cargo-target/target`, `-j 4`(OOM 시 `-j 1`).
- 결과를 파일로 redirect + exit code 확인 후 커밋. `cargo ... | grep` 금지(pipe exit 마스킹).
- 예: `CARGO_TARGET_DIR=/build/out/cargo-target/target cargo test -p kebab-store-sqlite -j 4 > /tmp/t.log 2>&1; echo "EXIT=$?"` → 파일에서 EXIT + 결과 확인.

**참조 spec:** `docs/superpowers/specs/2026-05-30-doc-side-expansion-design.md`

---

## File Structure

| 파일 | 역할 | Task |
|------|------|------|
| `crates/kebab-core/src/chunk.rs` | `Chunk.aliases: Option<String>` 필드 | 1 |
| `migrations/V010__chunk_aliases.sql` | `chunks.aliases` 컬럼 + `chunk_aliases_fts` + trigger 3종 | 2 |
| `crates/kebab-store-sqlite/src/documents.rs` | `put_chunks` INSERT 에 `aliases` 컬럼 | 2 |
| `crates/kebab-store-sqlite/tests/` | migration + put/get + trigger 동기화 테스트 | 2 |
| `crates/kebab-config/src/lib.rs` | `IngestExpansionCfg` + default + env override | 3 |
| `crates/kebab-app/src/expansion.rs` (Create) | `ExpansionGenerator` — 프롬프트·파싱·상한·fail-soft | 4 |
| `crates/kebab-app/src/lib.rs` | ingest hook (flag 게이트, chunk 직후) | 5 |
| `crates/kebab-search/src/lexical.rs` | `run_alias_query` + body/alias 병합 + 컬럼 파라미터화 | 6 |
| README / HANDOFF / ARCHITECTURE / HOTFIXES / release-notes | 문서 동기화 + 측정 기록 | 7 |

각 Task 는 자체로 컴파일·테스트 통과하는 단위다. Task 6 까지 끝나면 flag on 시 end-to-end 동작, Task 7 은 측정/문서.

---

## Task 1: `Chunk.aliases` 필드 추가

**Files:**
- Modify: `crates/kebab-core/src/chunk.rs:16-31`
- Test: 동 파일 인라인(또는 기존 core 테스트) — 직렬화 default 확인

- [ ] **Step 1: 실패 테스트 작성**

`crates/kebab-core/src/chunk.rs` 하단에 `#[cfg(test)]` 모듈(없으면 신설):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aliases_defaults_to_none_on_deserialize() {
        // aliases 필드가 없는 과거 JSON 도 파싱되어야 한다 (#[serde(default)]).
        let json = r#"{
            "chunk_id": "c1",
            "doc_id": "d1",
            "block_ids": [],
            "text": "hello",
            "heading_path": [],
            "source_spans": [],
            "token_estimate": 1,
            "chunker_version": "md-heading-v1",
            "policy_hash": "abc"
        }"#;
        let c: Chunk = serde_json::from_str(json).unwrap();
        assert_eq!(c.aliases, None);
        assert_eq!(c.tokenized_korean_text, None);
    }
}
```

- [ ] **Step 2: 실패 확인**

Run: `CARGO_TARGET_DIR=/build/out/cargo-target/target cargo test -p kebab-core aliases_defaults -j 4 > /tmp/t1.log 2>&1; echo "EXIT=$?"`
Expected: 컴파일 실패 — `Chunk` 에 `aliases` 필드 없음 (`no field 'aliases'`).

- [ ] **Step 3: 필드 추가**

`crates/kebab-core/src/chunk.rs` 의 `Chunk` 구조체에서 `tokenized_korean_text` 바로 아래에 추가:

```rust
    #[serde(default)]
    pub tokenized_korean_text: Option<String>,
    /// 색인시 doc-side expansion (Phase 2) 으로 생성된 "검색용 별칭"
    /// (같은언어 paraphrase + 한↔영 번역, 개행 join). `[ingest.expansion]`
    /// flag off 또는 미생성이면 None — 별도 FTS5 테이블 `chunk_aliases_fts`
    /// 에만 색인되고 본문 매칭/dense 임베딩에는 영향 없음. 설계 spec
    /// `2026-05-30-doc-side-expansion-design.md` §3.3.
    #[serde(default)]
    pub aliases: Option<String>,
```

- [ ] **Step 4: 통과 확인 + 컴파일 영향 점검**

`Chunk` 를 리터럴로 만드는 곳이 `aliases` 누락으로 깨질 수 있다. 점검:

Run: `cd /home/altair823/kebab && grep -rn "Chunk {" crates --include=*.rs | grep -v "test" | head -30`

각 생성 지점에 `aliases: None,` 추가(특히 `crates/kebab-chunk*`/chunker). 그 후:

Run: `CARGO_TARGET_DIR=/build/out/cargo-target/target cargo test -p kebab-core aliases_defaults -j 4 > /tmp/t1.log 2>&1; echo "EXIT=$?"`
Expected: PASS. 이어서 워크스페이스 컴파일 확인:
Run: `CARGO_TARGET_DIR=/build/out/cargo-target/target cargo build -p kebab-chunk -p kebab-store-sqlite -j 4 > /tmp/t1b.log 2>&1; echo "EXIT=$?"`
Expected: EXIT=0 (chunker 가 `aliases: None` 으로 컴파일).

- [ ] **Step 5: 커밋**

```bash
git add crates/kebab-core/src/chunk.rs crates
git commit -m "feat(core): Chunk.aliases 필드 (doc-side expansion)"
```

---

## Task 2: V010 migration + `put_chunks` 별칭 영속화

**Files:**
- Create: `migrations/V010__chunk_aliases.sql`
- Modify: `crates/kebab-store-sqlite/src/documents.rs:103-140` (`put_chunks` INSERT)
- Test: `crates/kebab-store-sqlite/tests/` (기존 `fts.rs` 패턴 따라 신규 `chunk_aliases.rs` 또는 기존 파일에 추가)

- [ ] **Step 1: migration 작성**

`migrations/V010__chunk_aliases.sql` 생성 — 기존 `chunks_fts`/`chunks_ai/ad/au`(§5.5 verbatim CI 대상)는 **건드리지 않는다**:

```sql
-- V010__chunk_aliases.sql — doc-side expansion (Phase 2) 검색용 별칭 채널.
--
-- 설계 spec docs/superpowers/specs/2026-05-30-doc-side-expansion-design.md §4.
-- chunks 에 nullable `aliases` 컬럼 + 별도 FTS5 테이블 chunk_aliases_fts +
-- 별도 sync trigger. 기존 chunks_fts / chunks_ai/ad/au (design §5.5 verbatim,
-- CI test fts_v009_matches_design_section_5_5_verbatim) 는 무수정.
-- aliases 는 additive: 미생성/flag off 이면 NULL → chunk_aliases_fts 빈 채로
-- 시작, 검색 UNION 둘째 절 0행 → 기존 동작과 동일. 자동 backfill 없음.

ALTER TABLE chunks ADD COLUMN aliases TEXT;

CREATE VIRTUAL TABLE chunk_aliases_fts USING fts5(
  chunk_id  UNINDEXED,
  doc_id    UNINDEXED,
  aliases,
  tokenize = 'unicode61'
);

CREATE TRIGGER chunk_aliases_ai AFTER INSERT ON chunks WHEN new.aliases IS NOT NULL BEGIN
  INSERT INTO chunk_aliases_fts(chunk_id, doc_id, aliases)
  VALUES (new.chunk_id, new.doc_id, new.aliases);
END;
CREATE TRIGGER chunk_aliases_ad AFTER DELETE ON chunks BEGIN
  DELETE FROM chunk_aliases_fts WHERE chunk_id = old.chunk_id;
END;
CREATE TRIGGER chunk_aliases_au AFTER UPDATE ON chunks BEGIN
  DELETE FROM chunk_aliases_fts WHERE chunk_id = old.chunk_id;
  INSERT INTO chunk_aliases_fts(chunk_id, doc_id, aliases)
    SELECT new.chunk_id, new.doc_id, new.aliases WHERE new.aliases IS NOT NULL;
END;

-- in-process LRU search cache 무효화 (V009 와 동일 패턴).
UPDATE kv SET value = CAST(CAST(value AS INTEGER) + 1 AS TEXT) WHERE key = 'corpus_revision';
```

- [ ] **Step 2: 실패 테스트 작성**

먼저 기존 store 테스트가 임시 SqliteStore 를 어떻게 여는지 확인:
Run: `cd /home/altair823/kebab && sed -n '1,60p' crates/kebab-store-sqlite/tests/fts.rs`

그 헬퍼 패턴(보통 `SqliteStore::open(tempfile)` 가 모든 migration 적용)을 따라 `crates/kebab-store-sqlite/tests/chunk_aliases.rs` 생성. `put_chunks` 로 `aliases=Some(..)` 청크를 저장하면 `chunk_aliases_fts` MATCH 로 회수되고, `aliases=None` 이면 안 들어가는지 검증:

```rust
// 기존 fts.rs 의 store 오픈 + Chunk 생성 헬퍼를 동일하게 재사용/복제할 것.
// 아래는 검증 핵심부 — 헬퍼 시그니처는 fts.rs 실제 코드에 맞춘다.
use kebab_core::{Chunk, ChunkId, ChunkerVersion, DocumentId};

#[test]
fn aliases_indexed_into_chunk_aliases_fts() {
    let store = open_temp_store_with_one_document(); // fts.rs 헬퍼 패턴
    let doc = DocumentId("d1".into());
    let chunk = Chunk {
        chunk_id: ChunkId("c1".into()),
        doc_id: doc.clone(),
        block_ids: vec![],
        text: "Rust ownership and borrowing".into(),
        heading_path: vec![],
        source_spans: vec![],
        token_estimate: 5,
        chunker_version: ChunkerVersion("md-heading-v1".into()),
        policy_hash: "h".into(),
        tokenized_korean_text: None,
        aliases: Some("메모리 안전성\nwho owns the value".into()),
    };
    store.put_chunks(&doc, &[chunk]).unwrap();

    let conn = store.read_conn();
    // 별칭에만 있는 한국어 term 으로 chunk_aliases_fts 검색 → c1 회수.
    let n: i64 = conn
        .query_row(
            "SELECT count(*) FROM chunk_aliases_fts WHERE chunk_aliases_fts MATCH 'aliases : (\"메모리\")'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1, "aliases 의 한국어 term 이 chunk_aliases_fts 에 색인돼야 한다");
}

#[test]
fn none_aliases_not_indexed() {
    let store = open_temp_store_with_one_document();
    let doc = DocumentId("d1".into());
    let chunk = Chunk { /* 위와 동일하되 */ aliases: None, ..base_chunk("c1", &doc) };
    store.put_chunks(&doc, &[chunk]).unwrap();
    let conn = store.read_conn();
    let n: i64 = conn
        .query_row("SELECT count(*) FROM chunk_aliases_fts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0, "aliases=None 이면 chunk_aliases_fts 에 행이 없어야 한다");
}
```

- [ ] **Step 3: 실패 확인**

Run: `CARGO_TARGET_DIR=/build/out/cargo-target/target cargo test -p kebab-store-sqlite --test chunk_aliases -j 4 > /tmp/t2.log 2>&1; echo "EXIT=$?"`
Expected: 실패 — `put_chunks` INSERT 에 `aliases` 컬럼이 없어 `chunk_aliases_fts` 가 비어 있음 (또는 SQL 컬럼 수 불일치). 파일에서 실패 사유 확인.

- [ ] **Step 4: `put_chunks` 수정**

`crates/kebab-store-sqlite/src/documents.rs` 의 INSERT 문(라인 103-110)과 `stmt.execute`(126-139) 에 `aliases` 컬럼 추가:

```rust
        let mut stmt = tx
            .prepare(
                "INSERT INTO chunks (
                    chunk_id, doc_id, text, heading_path_json,
                    section_label, source_spans_json, token_estimate,
                    chunker_version, policy_hash, block_ids_json, created_at,
                    tokenized_korean_text, aliases
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .map_err(StoreError::from)?;
```

`stmt.execute(params![ ... ])` 의 마지막(`chunk.tokenized_korean_text.as_deref(),`) 다음에:

```rust
                chunk.tokenized_korean_text.as_deref(),
                chunk.aliases.as_deref(),
```

- [ ] **Step 5: 통과 확인**

Run: `CARGO_TARGET_DIR=/build/out/cargo-target/target cargo test -p kebab-store-sqlite -j 4 > /tmp/t2.log 2>&1; echo "EXIT=$?"`
Expected: EXIT=0, `aliases_indexed_into_chunk_aliases_fts` + `none_aliases_not_indexed` PASS, 기존 store 테스트 전부 PASS(특히 `fts_v009_matches_design_section_5_5_verbatim` — V010 이 §5.5 블록을 안 건드리므로 그대로 통과해야 함). 파일에서 통과 수 확인.

- [ ] **Step 6: 커밋**

```bash
git add migrations/V010__chunk_aliases.sql crates/kebab-store-sqlite
git commit -m "feat(store): V010 chunk_aliases_fts + put_chunks 별칭 영속화"
```

---

## Task 3: `[ingest.expansion]` config

**Files:**
- Modify: `crates/kebab-config/src/lib.rs` (`IngestCfg` 확장 + `IngestExpansionCfg` + `defaults()` + `apply_env`)
- Test: 동 crate 인라인 테스트

- [ ] **Step 1: 실패 테스트 작성**

`crates/kebab-config/src/lib.rs` 의 기존 `#[cfg(test)] mod tests` 에 추가(없으면 신설):

```rust
    #[test]
    fn expansion_defaults_off() {
        let cfg = Config::defaults();
        assert!(!cfg.ingest.expansion.enabled, "expansion 은 기본 off");
        assert_eq!(cfg.ingest.expansion.max_aliases_per_chunk, 8);
        assert_eq!(cfg.ingest.expansion.prompt_version, "expansion-v1");
        // model 비면 models.llm.model 로 폴백할 수 있게 빈 문자열 default.
        assert_eq!(cfg.ingest.expansion.model, "");
    }

    #[test]
    fn expansion_env_override() {
        let mut cfg = Config::defaults();
        let env: std::collections::HashMap<String, String> = [
            ("KEBAB_INGEST_EXPANSION_ENABLED".to_string(), "true".to_string()),
            ("KEBAB_INGEST_EXPANSION_MAX_ALIASES".to_string(), "12".to_string()),
            ("KEBAB_INGEST_EXPANSION_MODEL".to_string(), "gemma4:e4b".to_string()),
            ("KEBAB_INGEST_EXPANSION_PROMPT_VERSION".to_string(), "expansion-v2".to_string()),
        ]
        .into_iter()
        .collect();
        cfg.apply_env(&env);
        assert!(cfg.ingest.expansion.enabled);
        assert_eq!(cfg.ingest.expansion.max_aliases_per_chunk, 12);
        assert_eq!(cfg.ingest.expansion.model, "gemma4:e4b");
        assert_eq!(cfg.ingest.expansion.prompt_version, "expansion-v2");
    }
```

- [ ] **Step 2: 실패 확인**

Run: `CARGO_TARGET_DIR=/build/out/cargo-target/target cargo test -p kebab-config expansion_ -j 4 > /tmp/t3.log 2>&1; echo "EXIT=$?"`
Expected: 컴파일 실패 — `ingest.expansion` 필드 없음.

- [ ] **Step 3: 구조체 + default + env 추가**

(3a) `IngestCfg`(라인 ~596) 에 필드 추가:

```rust
pub struct IngestCfg {
    pub code: IngestCodeCfg,
    #[serde(default)]
    pub expansion: IngestExpansionCfg,
}
```

(3b) `IngestCodeCfg` 정의 아래에 신규 구조체:

```rust
/// Phase 2 doc-side expansion: 색인시 LLM 으로 청크당 "검색용 별칭"
/// (같은언어 paraphrase + 한↔영 번역) 1회 생성. 별도 chunk_aliases_fts
/// 채널에 저장, lexical 검색이 본문+별칭 병합. default off (additive).
/// 설계 spec 2026-05-30-doc-side-expansion-design.md §3.2.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct IngestExpansionCfg {
    /// 색인시 별칭 생성 활성화. off 면 chunks.aliases=NULL (기존 동작).
    pub enabled: bool,
    /// 별칭 생성에 쓸 LLM 모델. 빈 문자열이면 `models.llm.model` 로 폴백.
    pub model: String,
    /// 청크당 별칭 최대 개수(상한). 초과분 drop.
    pub max_aliases_per_chunk: usize,
    /// 프롬프트 버전(추적용). 변경 시 재생성 대상 식별.
    pub prompt_version: String,
}

impl Default for IngestExpansionCfg {
    fn default() -> Self {
        Self {
            enabled: false,
            model: String::new(),
            max_aliases_per_chunk: 8,
            prompt_version: "expansion-v1".to_string(),
        }
    }
}
```

(3c) `Config::defaults()` 의 `ingest: IngestCfg::default(),` 는 이미 `IngestCfg::default()` 를 쓰므로(라인 716) — `IngestCfg` 가 `Default` 파생인지 확인. 만약 `IngestCfg` 가 수동 default 면 `expansion: IngestExpansionCfg::default()` 추가. (확인: `grep -n "impl Default for IngestCfg\|derive.*Default.*\n.*struct IngestCfg" crates/kebab-config/src/lib.rs`)

(3d) `apply_env`(라인 ~861-1090) 에 env 키 추가. 기존 `parse_bool` 헬퍼 사용:

```rust
            "KEBAB_INGEST_EXPANSION_ENABLED" => self.ingest.expansion.enabled = parse_bool(v),
            "KEBAB_INGEST_EXPANSION_MODEL" => self.ingest.expansion.model = v.clone(),
            "KEBAB_INGEST_EXPANSION_MAX_ALIASES" => {
                if let Ok(n) = v.parse::<usize>() {
                    self.ingest.expansion.max_aliases_per_chunk = n;
                }
            }
            "KEBAB_INGEST_EXPANSION_PROMPT_VERSION" => {
                self.ingest.expansion.prompt_version = v.clone()
            }
```

- [ ] **Step 4: 통과 확인**

Run: `CARGO_TARGET_DIR=/build/out/cargo-target/target cargo test -p kebab-config -j 4 > /tmp/t3.log 2>&1; echo "EXIT=$?"`
Expected: EXIT=0, `expansion_defaults_off` + `expansion_env_override` PASS, 기존 config 테스트 전부 PASS.

- [ ] **Step 5: 커밋**

```bash
git add crates/kebab-config
git commit -m "feat(config): [ingest.expansion] flag (default off)"
```

---

## Task 4: `ExpansionGenerator`

**Files:**
- Create: `crates/kebab-app/src/expansion.rs`
- Modify: `crates/kebab-app/src/lib.rs` (`mod expansion;` 선언)
- Modify: `crates/kebab-app/Cargo.toml` ([dev-dependencies] 에 `kebab-llm` 의 `mock` feature)
- Test: `crates/kebab-app/src/expansion.rs` 인라인

`LanguageModel::generate_stream(req) -> Iterator<Result<TokenChunk>>` 를 모아 문자열로 합치고, 줄 단위 파싱 → trim → 빈 줄/과길이(>120 chars) drop → 상한 N → 개행 join. LLM 호출 실패/빈 결과 시 `None`(fail-soft).

- [ ] **Step 1: 실패 테스트 작성**

`crates/kebab-app/src/expansion.rs` 생성:

```rust
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
            .trim_start_matches(|c: char| c.is_ascii_digit() || c == '.' || c == ')' || c == '-' || c == '*')
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
            canned_usage: TokenUsage { prompt_tokens: 0, completion_tokens: 0 },
        }
    }

    #[test]
    fn parses_lines_strips_bullets_and_caps() {
        let llm = mock("- 메모리 안전성\n1. who owns the value\nborrow checker\n\n* 소유권");
        let gen = ExpansionGenerator::new(&llm, 2);
        let out = gen.generate(&mk_chunk("Rust ownership")).unwrap();
        // 상한 2 → 앞 2개만, 접두 제거됨.
        assert_eq!(out, "메모리 안전성\nwho owns the value");
    }

    #[test]
    fn drops_overlong_lines() {
        let long = "x".repeat(200);
        let llm = mock(&format!("{long}\n짧은 별칭"));
        let gen = ExpansionGenerator::new(&llm, 8);
        let out = gen.generate(&mk_chunk("t")).unwrap();
        assert_eq!(out, "짧은 별칭", "120자 초과 줄은 drop");
    }

    #[test]
    fn empty_output_returns_none() {
        let llm = mock("   \n\n");
        let gen = ExpansionGenerator::new(&llm, 8);
        assert_eq!(gen.generate(&mk_chunk("t")), None);
    }
}
```

- [ ] **Step 2: 모듈 선언 + dev-dep**

`crates/kebab-app/src/lib.rs` 상단 모듈 선언부에 `mod expansion;` 추가(필요 시 `pub mod`).
`crates/kebab-app/Cargo.toml` 의 `[dev-dependencies]` 에 mock feature 활성화(이미 kebab-llm 의존 시):

```toml
[dev-dependencies]
kebab-llm = { workspace = true, features = ["mock"] }
```

(확인: `grep -n "kebab-llm" crates/kebab-app/Cargo.toml`. 이미 `[dependencies]` 에 있으면 dev-dep 에서 features 만 추가하거나, `[dev-dependencies]` 줄 신설.)

- [ ] **Step 3: 실패 확인**

Run: `CARGO_TARGET_DIR=/build/out/cargo-target/target cargo test -p kebab-app expansion:: -j 4 > /tmp/t4.log 2>&1; echo "EXIT=$?"`
Expected: 위 구현이 이미 들어 있으면 PASS 할 수도 있으나, mock feature/모듈 선언 누락 시 컴파일 실패. 파일에서 사유 확인 후 Step 2 보완.

- [ ] **Step 4: 통과 확인**

Run: `CARGO_TARGET_DIR=/build/out/cargo-target/target cargo test -p kebab-app expansion:: -j 4 > /tmp/t4.log 2>&1; echo "EXIT=$?"`
Expected: EXIT=0, 3개 테스트 PASS.

- [ ] **Step 5: 커밋**

```bash
git add crates/kebab-app/src/expansion.rs crates/kebab-app/src/lib.rs crates/kebab-app/Cargo.toml
git commit -m "feat(app): ExpansionGenerator — 청크당 별칭 생성 (fail-soft)"
```

---

## Task 5: ingest hook (flag 게이트)

**Files:**
- Modify: `crates/kebab-app/src/lib.rs` (ingest 진입부에 expansion LLM 빌드 ~388-400 근방; `ingest_one_asset` chunk 직후 ~1253)

`OllamaLanguageModel` 은 `kebab-llm-local` 의 타입. caption_llm 패턴(라인 394-400) 을 그대로 따른다. expansion LLM 은 `ingest_one_asset` 까지 전달돼야 하므로, caption 처럼 ingest 함수 시그니처/호출 체인을 따라 내려보낸다(`ingest_one_asset` 가 `app` 을 받으므로 `app.config.ingest.expansion` 으로 분기하고 LLM 을 함수 내에서 빌드하는 게 가장 단순 — per-asset 빌드 비용은 무시 가능하지만, 더 깔끔히 하려면 ingest 루프 밖에서 1회 빌드해 `&dyn LanguageModel` 로 전달).

> **구현 노트(executor 판단):** 우선 가장 단순한 형태 — `ingest_one_asset` 내부에서 `app.config.ingest.expansion.enabled` 이면 LLM 1회 빌드 후 청크 루프. caption_llm 처럼 ingest 루프 밖 1회 빌드가 가능하면 그쪽이 낫다(LLM 핸들 재사용). 단 **테스트 가능성**을 위해 별칭 부여 로직은 Task 4 의 `ExpansionGenerator` 에 이미 격리돼 있으므로, 여기선 "flag 분기 + 청크 루프 + chunk.aliases 세팅"만 한다.

- [ ] **Step 1: hook 코드 작성**

`crates/kebab-app/src/lib.rs` 의 `ingest_one_asset` 에서 chunk 생성 직후(라인 1253-1255 의 `let chunks = ...?;` 다음, 버전 스탬핑 전후), `chunks` 를 `mut` 로 바꾸고 추가:

```rust
    let mut chunks = MdHeadingV1Chunker
        .chunk(&canonical, chunk_policy)
        .context("kb-chunk::MdHeadingV1Chunker::chunk")?;

    // Phase 2 doc-side expansion: flag on 이면 청크당 별칭 생성 (fail-soft).
    // 설계 spec 2026-05-30-doc-side-expansion-design.md §3.1.
    if app.config.ingest.expansion.enabled {
        let exp = &app.config.ingest.expansion;
        let model = if exp.model.is_empty() {
            app.config.models.llm.model.clone()
        } else {
            exp.model.clone()
        };
        match kebab_llm_local::OllamaLanguageModel::with_model(&app.config, &model) {
            Ok(llm) => {
                let generator =
                    crate::expansion::ExpansionGenerator::new(&llm, exp.max_aliases_per_chunk);
                for chunk in &mut chunks {
                    chunk.aliases = generator.generate(chunk);
                }
            }
            Err(e) => {
                // fail-soft: 별칭 없이 색인 진행 (본문 검색은 정상).
                tracing::warn!(
                    target: "kebab-app",
                    error = %e,
                    "kb-app::ingest: expansion LLM 빌드 실패 — 별칭 없이 진행"
                );
            }
        }
    }
```

> `OllamaLanguageModel::with_model(&config, &model)` 가 없으면 — `OllamaLanguageModel::new(&config)`(config.models.llm.model 사용) 로 폴백하고, model override 가 필요하면 `kebab-llm-local` 에 `with_model` 생성자를 추가한다. 확인: `grep -n "impl OllamaLanguageModel\|pub fn new\|pub fn with" crates/kebab-llm-local/src/ollama.rs`. override 가 과하면 1차는 `new(&app.config)` 만 쓰고 `exp.model` 은 무시(spec §3.2 의 model 폴백 동작은 Task 7 에서 README 에 "현재 models.llm 사용"으로 명시) — **executor 가 실제 생성자 확인 후 결정**.

- [ ] **Step 2: 컴파일 + 회귀 확인**

Run: `CARGO_TARGET_DIR=/build/out/cargo-target/target cargo build -p kebab-app -j 4 > /tmp/t5.log 2>&1; echo "EXIT=$?"`
Expected: EXIT=0. 실패 시 생성자 시그니처(위 노트) 보정.

Run: `CARGO_TARGET_DIR=/build/out/cargo-target/target cargo test -p kebab-app -j 4 > /tmp/t5b.log 2>&1; echo "EXIT=$?"`
Expected: EXIT=0 (flag default off 라 기존 ingest 테스트 무영향).

- [ ] **Step 3: 통합 테스트 (flag on, mock 불가 시 생략 가능)**

실제 Ollama 가 필요하므로 단위 테스트로는 검증이 어렵다. 대신 flag off 회귀만 단위로 보장하고, flag on end-to-end 는 Task 7 의 dogfood 측정에서 검증한다. (이 Step 은 "flag off 시 chunk.aliases 가 None 으로 유지됨"을 보장하는 기존 테스트로 충분 — 추가 테스트 불필요.)

- [ ] **Step 4: 커밋**

```bash
git add crates/kebab-app/src/lib.rs
git commit -m "feat(app): ingest 별칭 생성 hook (flag off 기본, fail-soft)"
```

---

## Task 6: `LexicalRetriever` body+alias 병합 검색

**Files:**
- Modify: `crates/kebab-search/src/lexical.rs` (`build_match_string` 컬럼 파라미터화, `run_alias_query` 추가, `search()` 병합)
- Test: `crates/kebab-search/tests/` (기존 lexical 통합 테스트 패턴) 또는 lexical.rs 인라인

핵심: `build_match_string` 은 현재 `text : (...)` 컬럼 필터를 반환. alias 검색은 `aliases : (...)` 가 필요하므로 컬럼명을 파라미터화한다. `search()` 는 body 결과(`run_query`) + alias 결과(`run_alias_query`)를 병합 — **body 우선, alias-only 를 뒤에 append**, `chunk_aliases_fts` 가 비면 alias 결과 0 → 기존과 동일.

- [ ] **Step 1: 실패 테스트 작성**

`crates/kebab-search/tests/` 의 기존 lexical 테스트가 store 를 어떻게 채우는지 확인:
Run: `cd /home/altair823/kebab && ls crates/kebab-search/tests/ && grep -rln "LexicalRetriever" crates/kebab-search/tests/`

그 패턴으로, **본문에 없고 별칭에만 있는 term** 으로 검색 시 해당 청크가 회수되는 테스트 작성(핵심 pool-rescue 회귀):

```rust
// 헬퍼(store 오픈 + put_chunks)는 기존 테스트 패턴 재사용.
#[test]
fn alias_only_term_recalls_chunk() {
    let store = /* temp store + 1 document */;
    // 본문엔 "backpropagation" 만, 별칭에 "역전파" 추가.
    let chunk = Chunk {
        /* ... */
        text: "backpropagation computes gradients".into(),
        aliases: Some("역전파\n신경망 오차 역전달".into()),
        /* ... */
    };
    store.put_chunks(&doc, &[chunk]).unwrap();

    let retr = LexicalRetriever::with_settings(store.clone(), IndexVersion("v1".into()), 220);
    // 본문에 없는 한국어로 검색 → 별칭 덕에 회수돼야 한다.
    let q = SearchQuery { text: "역전파".into(), mode: SearchMode::Lexical, k: 10, filters: Default::default() };
    let hits = retr.search(&q).unwrap();
    assert!(hits.iter().any(|h| h.chunk_id.0 == "c1"),
        "별칭에만 있는 term 으로도 청크가 회수돼야 한다 (pool-rescue)");
}

#[test]
fn empty_aliases_table_matches_baseline() {
    // aliases 전부 None → chunk_aliases_fts 빈 상태 → 본문 검색 결과가
    // 별칭 도입 전과 동일해야 한다 (회귀 안전).
    let store = /* temp store, aliases=None 청크들 */;
    let retr = LexicalRetriever::with_settings(store, IndexVersion("v1".into()), 220);
    let q = SearchQuery { text: "ownership".into(), mode: SearchMode::Lexical, k: 10, filters: Default::default() };
    let hits = retr.search(&q).unwrap();
    // 본문 매칭 청크가 정상 회수 (별칭 경로가 결과를 바꾸지 않음).
    assert!(hits.iter().any(|h| h.chunk_id.0 == "c1"));
}
```

- [ ] **Step 2: 실패 확인**

Run: `CARGO_TARGET_DIR=/build/out/cargo-target/target cargo test -p kebab-search alias_only_term_recalls -j 4 > /tmp/t6.log 2>&1; echo "EXIT=$?"`
Expected: 실패 — 현재 `search()` 는 본문(`chunks_fts`)만 보므로 별칭-only term 회수 0.

- [ ] **Step 3: `build_match_string` 컬럼 파라미터화**

`build_match_string` 의 마지막 줄 `Some(format!("text : ({expression})"))` 을 컬럼 인자로:

```rust
fn build_match_string(text: &str) -> Option<String> {
    build_match_string_for_column(text, "text")
}

/// `column` 은 FTS5 컬럼 필터 prefix ("text" 또는 "aliases").
fn build_match_string_for_column(text: &str, column: &str) -> Option<String> {
    // ... 기존 본문 (whole_candidate / token_and_candidate / expression) 그대로 ...
    Some(format!("{column} : ({expression})"))
}
```

(기존 `build_match_string("rust cargo")` 테스트는 `text : (...)` 를 기대하므로 그대로 통과.)

- [ ] **Step 4: `run_alias_query` 추가**

`run_query` 아래에 별칭 전용 쿼리. 필터는 1차에선 미적용(별칭 회수가 목적; 측정 후 필요 시 공유)하되, snippet 은 `chunks.text` 앞부분으로 대체:

```rust
/// chunk_aliases_fts 를 검색해 RawRow 를 만든다. snippet 은 별칭이 아닌
/// 본문(c.text) 앞부분으로 채워 UI 일관성 유지. chunk_aliases_fts 가 비면
/// 0행 반환(회귀 안전). 1차는 filters 미적용 — body 쪽에서 필터가 적용되고,
/// 별칭 경로는 pool 진입이 목적(측정 후 필요 시 filters 공유).
fn run_alias_query(
    conn: &Connection,
    match_str: &str,
    snippet_chars: usize,
    fetch_limit: usize,
) -> Result<Vec<RawRow>> {
    let sql = "SELECT \
            af.chunk_id, af.doc_id, \
            bm25(chunk_aliases_fts) AS score, \
            substr(c.text, 1, ?) AS snippet, \
            c.heading_path_json, c.section_label, c.source_spans_json, \
            c.chunker_version, \
            d.workspace_path, d.updated_at \
         FROM chunk_aliases_fts af \
         JOIN chunks c    ON c.chunk_id = af.chunk_id \
         JOIN documents d ON d.doc_id = af.doc_id \
         WHERE chunk_aliases_fts MATCH ? \
         ORDER BY score, af.chunk_id LIMIT ?";
    let mut stmt = conn
        .prepare(sql)
        .context("kb-search lexical: prepare alias FTS5 statement")?;
    let rows = stmt
        .query_map(
            params_from_iter(vec![
                Box::new(snippet_chars as i64) as Box<dyn ToSql>,
                Box::new(match_str.to_owned()),
                Box::new(i64::try_from(fetch_limit).unwrap_or(i64::MAX)),
            ]
            .iter()
            .map(std::convert::AsRef::as_ref)),
            row_from_sql,
        )
        .context("kb-search lexical: execute alias FTS5 query")?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.context("kb-search lexical: read alias row")?);
    }
    Ok(out)
}
```

- [ ] **Step 5: `search()` 에서 병합**

`LexicalRetriever::search` 에서 `run_query` 호출 직후, body+alias 병합. 기존:

```rust
        let raw_rows = run_query(&conn, &match_str, self.snippet_words, filters, fetch_limit)?;
```

를 다음으로 교체:

```rust
        let body_rows = run_query(&conn, &match_str, self.snippet_words, filters, fetch_limit)?;
        // 별칭 채널: 같은 query 를 aliases 컬럼 필터로 다시 매칭. 테이블이
        // 비면 0행 → body_rows 그대로(회귀 안전). body 우선, alias-only append.
        let alias_rows = match build_match_string_for_column(&query.text, "aliases") {
            Some(am) => run_alias_query(&conn, &am, self.snippet_chars, fetch_limit)?,
            None => Vec::new(),
        };
        let raw_rows = merge_body_alias(body_rows, alias_rows, fetch_limit);
```

병합 헬퍼 추가(`run_alias_query` 아래):

```rust
/// body 결과 우선, body 에 없는 alias-only 청크를 뒤에 append. fetch_limit
/// 로 절단. body_rows 는 이미 bm25 오름차순; alias_rows 도 그러하므로
/// alias-only 부분도 별칭 적합도 순으로 들어간다.
fn merge_body_alias(body: Vec<RawRow>, alias: Vec<RawRow>, limit: usize) -> Vec<RawRow> {
    use std::collections::HashSet;
    let mut seen: HashSet<String> = body.iter().map(|r| r.chunk_id.clone()).collect();
    let mut out = body;
    for r in alias {
        if out.len() >= limit {
            break;
        }
        if seen.insert(r.chunk_id.clone()) {
            out.push(r);
        }
    }
    out.truncate(limit);
    out
}
```

> `query.text` 가 `search()` 스코프에 있는지 확인(있음 — `match_opt = build_match_string(&query.text)`). `self.snippet_chars` 필드도 존재(LexicalRetriever 구조체).

- [ ] **Step 6: 통과 + 전체 회귀 확인**

Run: `CARGO_TARGET_DIR=/build/out/cargo-target/target cargo test -p kebab-search -j 4 > /tmp/t6.log 2>&1; echo "EXIT=$?"`
Expected: EXIT=0, `alias_only_term_recalls_chunk` + `empty_aliases_table_matches_baseline` PASS, 기존 lexical/hybrid 테스트 전부 PASS(`build_match_string_default_emits_or_of_phrase_and_and` 포함 — `text : (...)` 유지).

- [ ] **Step 7: 커밋**

```bash
git add crates/kebab-search/src/lexical.rs crates/kebab-search/tests
git commit -m "feat(search): lexical body+alias 병합 검색 (pool-rescue)"
```

---

## Task 7: 측정 + 문서 동기화

**Files:**
- 측정: dogfood KB (`/build/dogfood`)
- Modify: `README.md`, `HANDOFF.md`, `docs/ARCHITECTURE.md`, `tasks/HOTFIXES.md`, `docs/release-notes/v<X.Y.Z>-draft.md`

- [ ] **Step 1: 전체 빌드 + clippy 게이트**

Run: `CARGO_TARGET_DIR=/build/out/cargo-target/target cargo build --release -j 4 > /tmp/t7build.log 2>&1; echo "EXIT=$?"`
Run: `CARGO_TARGET_DIR=/build/out/cargo-target/target cargo clippy --workspace --all-targets -j 4 -- -D warnings > /tmp/t7clippy.log 2>&1; echo "EXIT=$?"`
Expected: 둘 다 EXIT=0. 파일에서 확인.

- [ ] **Step 2: baseline (flag off) 측정**

`/build/dogfood/config.toml` 의 `[ingest.expansion]` 미설정(=off) 상태. dogfood KB 가 V010 migration 을 받도록 한 번 ingest(또는 reset+reingest — pristine 필요 시):

```
KEBAB_EVAL_GOLDEN=/build/dogfood/golden_queries.yaml \
  /build/out/cargo-target/target/release/kebab eval run --config /build/dogfood/config.toml --mode hybrid --k 50 > /tmp/t7-off-run.log 2>&1; echo "EXIT=$?"
# run_id 추출 (Read 로 확인 — 추측 금지)
KEBAB_EVAL_GOLDEN=/build/dogfood/golden_queries.yaml \
  /build/out/cargo-target/target/release/kebab eval variants <run_id> --config /build/dogfood/config.toml > /tmp/t7-off-var.log 2>&1; echo "EXIT=$?"
```

`/tmp/t7-off-var.log` 를 **Read 로 열어** `groups / fully_consistent / A_dominant / B_dominant / spread@10` 값을 그대로 기록. (Phase 1 baseline: `groups=8 fully_consistent=2 A_dominant=2 B_dominant=4 spread@10=0.750` 와 대조.)

- [ ] **Step 3: 처방 (flag on) 측정**

`/build/dogfood/config.toml` 에 추가:

```toml
[ingest.expansion]
enabled = true
max_aliases_per_chunk = 8
```

reset + reingest (별칭 생성 — Ollama gemma 필요, 시간 소요. 진행은 `kebab ingest` ndjson 으로 확인):

```
/build/out/cargo-target/target/release/kebab reset --config /build/dogfood/config.toml --yes > /tmp/t7-reset.log 2>&1; echo "EXIT=$?"
/build/out/cargo-target/target/release/kebab ingest --config /build/dogfood/config.toml > /tmp/t7-ingest.log 2>&1; echo "EXIT=$?"
KEBAB_EVAL_GOLDEN=/build/dogfood/golden_queries.yaml \
  /build/out/cargo-target/target/release/kebab eval run --config /build/dogfood/config.toml --mode hybrid --k 50 > /tmp/t7-on-run.log 2>&1; echo "EXIT=$?"
KEBAB_EVAL_GOLDEN=/build/dogfood/golden_queries.yaml \
  /build/out/cargo-target/target/release/kebab eval variants <run_id> --config /build/dogfood/config.toml > /tmp/t7-on-var.log 2>&1; echo "EXIT=$?"
```

`/tmp/t7-on-var.log` 를 Read 로 열어 값 기록. **성공 기준**: B_dominant↓ / fully_consistent↑ / spread@10↓ (off 대비). 회귀: 기존 Ok 그룹이 깨지지 않는지.

> ⚠️ 측정값 추측 금지([[feedback_search_quality_dogfood]]). grep clean 추출 + Read 확인값만 기록. 효과가 없거나 음수면 — spec §2 의 가설(KO↔EN 별칭이 우리 corpus 에서 recall 회복) 반증으로 보고, HOTFIXES 에 기록 후 사용자와 다음 단계 상의(default off 유지, 또는 프롬프트/단위 조정 재측정).

- [ ] **Step 4: 문서 동기화**

- `README.md`: **Configuration** 에 `[ingest.expansion]`(off 기본) 한 줄 + "별칭 생성은 색인 시간을 늘리며 Ollama LLM 필요" 포인터. flag 망라는 config 예제/`--help` 위임.
- `docs/ARCHITECTURE.md`: ingest 파이프라인에 expansion hook + `chunk_aliases_fts` 채널 1~2줄. lexical 병합 검색 언급.
- `HANDOFF.md`: "머지 후 발견된 버그/결정" 에 Phase 2 doc-side expansion 한 줄(측정 결과 요약).
- `tasks/HOTFIXES.md`: dated entry(2026-05-30 이후) — V010, 측정 표(off vs on), known limitation(필터 미적용 등).
- `docs/release-notes/v<X.Y.Z>-draft.md`: V010 breaking schema → 4단락(변경/trade-off/mitigation/upgrade). 측정 evidence link.

- [ ] **Step 5: 커밋**

```bash
git add README.md docs/ARCHITECTURE.md HANDOFF.md tasks/HOTFIXES.md docs/release-notes
git commit -m "docs: doc-side expansion 측정 결과 + 문서 동기화 (V010)"
```

---

## Self-Review (작성자 체크 — plan 검토)

- **Spec 커버리지:** §2 결정(D1~D4)→Task 4·5(청크당, 내용)·Task 1·2·5(additive)·Task 4(단순 품질). §3 아키텍처→Task 2(별도 테이블)·Task 6(lexical 병합). §4 스키마→Task 2. §5 프롬프트→Task 4. §6 versioning(try_skip 미변경)→Task 5 가 별칭 부재를 skip 판단에 안 넣음(기존 try_skip_unchanged 무수정). §7 측정→Task 7. §8 YAGNI(3채널/sparse/필터 제외)→plan 에 미포함(의도적). §9 테스트→각 Task TDD. §10 PR/문서→Task 7. ✅
- **Placeholder 스캔:** Task 5 의 `OllamaLanguageModel::with_model` / dev-dep 줄은 "executor 가 실제 시그니처 확인 후 결정" 노트로 명시(미정이 아니라 분기 지시). Task 1 Step 4 / Task 6 Step 1 의 `grep` 은 주변 코드 확인 지시(완성 코드 자체는 제시). ✅
- **타입 일관성:** `Chunk.aliases`(Task1) ↔ put_chunks(Task2) ↔ ExpansionGenerator.generate→Option<String>(Task4) ↔ ingest hook `chunk.aliases = generator.generate(chunk)`(Task5). `build_match_string_for_column`(Task6 Step3) ↔ search() 호출(Step5). `RawRow`/`row_from_sql`/`build_hit` 재사용(Task6). ✅
- **알려진 리스크:** Task 6 의 body/alias bm25 스케일 차이로 lexical 내부 순서가 근사 — hybrid 가 rank 변환하므로 pool 진입(핵심)은 보장, 정밀 순위는 측정 후. Task 5 end-to-end 는 Ollama 필요라 단위 테스트 불가 → Task 7 dogfood 로 검증.

---

## Execution Handoff

이 plan 은 핸드오프 §4.2 의 **OMC teammate(sequential single-team)** 로 task 별 구현 → code-reviewer 리뷰 → 독립 검증한다. Task 1~6 은 코드(executor), Task 7 은 측정+문서. 모델 라우팅(§4.3): Task 2·4·6(핵심 로직)=opus, Task 1·3·5(작은 변경)=sonnet, 리뷰는 핵심=opus.
