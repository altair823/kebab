---
title: "Post-merge hotfixes log"
date: 2026-05-01
---

# Post-merge hotfixes log

Bugs discovered AFTER a phase task was merged, and the small follow-up
PRs that close them. Each entry: what broke, how it surfaced, what the
fix touched, and which task spec it amends.

The original task specs in `tasks/p<N>/p<N>-<M>-*.md` stay frozen as the
historical contract that was implemented; this file accumulates the
deltas so phase 5+ readers can find the live behavior without diffing
git history.

## 2026-05-25 — v0.17.0 post-dogfood: `[models.llm] request_timeout_secs` 노브 + 권장 모델 가이드

v0.17.0 후속 도그푸딩에서 발견: 사용자가 default `gemma4:e4b` (8B Q4, 9.6 GB) 를 CPU only / 16 GB RAM 환경에서 시도 시 첫 RAG 답변이 5 분 (hard-coded 300 s) 한도를 항상 넘겨 `error: kb-rag: llm.generate_stream` 으로 떨어졌다. 메모리도 ollama RSS 10.7 GB / free 2 GB 까지 압박. 후속 도그푸딩 32 분 / 199 mem-monitor sample 결과는 `tasks/HOTFIXES.md` 의 본 entry 와 conversation 의 도그푸딩 보고 참조.

**변경**:
- `crates/kebab-config/src/lib.rs::LlmCfg` 에 `request_timeout_secs: u64` additive 필드 (`#[serde(default = "default_llm_request_timeout_secs")]`, default `300`). 옛 config 가 필드 누락해도 그대로 파싱 + 동일 동작 (3 신규 unit test 가 default / env override / legacy parse 핀).
- env override `KEBAB_MODELS_LLM_REQUEST_TIMEOUT_SECS`.
- `crates/kebab-llm-local/src/ollama.rs` 의 `REQUEST_TIMEOUT` 상수 제거. `OllamaLanguageModel::new` 가 `Duration::from_secs(llm.request_timeout_secs)` 로 reqwest blocking client 빌드. doc comment 도 동일하게 갱신.
- `README.md` 사전 요구 절 + `docs/SMOKE.md` 의 ollama 안내에 권장 모델 (≤ 4B Q4 — `gemma3:4b` / `qwen2.5:3b` / `phi3:mini`) + timeout 노브 anchor 한 줄. 8B+ 시도 시 timeout 패턴 사전 안내.
- `crates/kebab-config/src/lib.rs::Config::defaults` 의 LlmCfg literal 에 `request_timeout_secs: default_llm_request_timeout_secs()` + comment 한 줄로 CPU only 권장 안내.

**미진행 (scope 밖) — closure 갱신**:
- ~~`crates/kebab-parse-image/src/ocr.rs::REQUEST_TIMEOUT` 도 동일한 hard-coded 300 s — OCR 이 보통 짧아 LLM 만큼 부담 안 되지만, 일관성 측면에서 다음 round 에 같은 노브 (또는 별 노브) 로 재검토.~~ → **closure**: 아래 2026-05-25 v0.17.2 OCR timeout entry 참조 (별 노브 `[image.ocr] request_timeout_secs` 신설).
- ~~`kebab ask --stream` (fb-33) 권장 강조: 5분 cold-start 동안 첫 token 빠르게 surface — UX 개선. README/SKILL.md 추가 한 줄 후속.~~ → **closure**: PR #163 (v0.17.1 cut) 에서 이미 README + SMOKE + SKILL.md 세 곳 모두 추가됨 (`README.md:22` cold start 권장 단락, `docs/SMOKE.md:45/209` 예제, `SKILL.md:114/119` 사용 가이드). 본 entry 의 미진행 표기가 outdated 였음.

**후속 도그푸딩 baseline 보존**: `/build/cache/dogfood-v017/` (466 MB workspace + DB + memory.log), `/build/cache/ollama/` (21 GB binary + gemma3:4b/gemma4:e4b 모델). 다음 round 회귀 비교용.

Cross-link: `crates/kebab-config/src/lib.rs::LlmCfg::request_timeout_secs`, `crates/kebab-llm-local/src/ollama.rs::OllamaLanguageModel::new`.

## 2026-05-25 — post-v0.17.1 dogfood: `[image.ocr] request_timeout_secs` 노브 (closure of v0.17.1 미진행)

v0.17.1 entry 의 첫 번째 미진행 항목 closure. LLM 쪽이 v0.17.1 에서 `[models.llm] request_timeout_secs` 로 풀려난 패턴을 OCR 어댑터에 동일 적용. 별 노브로 분리한 이유 (사용자 결정): OCR 은 통상 LLM 대비 짧고 cold start 패턴도 다름 — 두 노브를 독립 조절할 수 있어야 16 GB / CPU only 환경에서 vision 모델만 다른 timeout 을 쓰기 편함. release tag 는 본 entry 시점 미결정 — cut 합의 시점에 동일 entry 가 v0.17.2 / v0.18.0 등으로 anchor 갱신.

**변경**:
- `crates/kebab-config/src/lib.rs::OcrCfg` 에 `request_timeout_secs: u64` additive 필드 (`#[serde(default = "default_ocr_request_timeout_secs")]`, default `300`). 옛 config 가 필드 누락해도 그대로 파싱 + 동일 동작 (3 신규 unit test 가 default / env override / legacy parse 핀).
- env override `KEBAB_IMAGE_OCR_REQUEST_TIMEOUT_SECS`.
- `crates/kebab-parse-image/src/ocr.rs` 의 `REQUEST_TIMEOUT` 상수 제거. `OllamaVisionOcr::build` 시그니처가 `request_timeout_secs: u64` 추가, `new(&Config)` 는 `config.image.ocr.request_timeout_secs` 전달. `from_parts` (테스트 전용 surface) 도 동일하게 시그니처 확장 — caller 9 call site (`crates/kebab-parse-image/src/ocr.rs::tests` 5 test / 6 call site, `crates/kebab-parse-image/tests/ocr.rs::from_parts_clamps_max_pixels_into_legal_range` 1 test / 3 call site) 모두 `300` 명시 갱신.
- `OcrCfg::defaults()` 에 `request_timeout_secs: default_ocr_request_timeout_secs()` 추가. `Config::defaults()` 는 `ImageCfg::defaults()` 경유라 cascade.

**Edge case 동일**: `0` 은 disable 아닌 "즉시 timeout" (`Duration::from_secs(0)` 의 reqwest 의미). LlmCfg 의 doc comment 와 같은 안내가 OcrCfg field doc 에 명시.

**사용자 영향**: 기존 v0.17.x KB / config 는 변경 불필요 — 새 필드는 serde default 로 채워지고 동작도 동일 (300s). vision 모델 cold start 가 길면 `KEBAB_IMAGE_OCR_REQUEST_TIMEOUT_SECS=600` 또는 config 에서 `[image.ocr] request_timeout_secs = 600` 설정.

Cross-link: `crates/kebab-config/src/lib.rs::OcrCfg::request_timeout_secs`, `crates/kebab-parse-image/src/ocr.rs::OllamaVisionOcr::build`.

## 2026-05-25 — post-v0.17.1 dogfood: `heading_path` FTS5 column filter (text-only matching, closure of 2026-05-24 `heading_path_json` 노이즈)

v0.17.0 의 한국어 trigram tokenizer 채택 entry (2026-05-24 위) 가 미수정으로 남겨둔 `heading_path_json` JSON 노이즈 closure. trigram 이 `chunks_fts.heading_path` 컬럼 (V002/V007 트리거가 `chunks.heading_path_json` 을 그대로 INSERT) 의 JSON 표기 (`[`, `"`, `,`) + 안의 path 세그먼트 (`app`, `src`) 까지 3-gram 색인해서 query 가 우연히 false positive hit 하는 문제. 사용자 결정 (column filter vs 평문 heading 변환): **column filter** — `heading_path` 색인은 V007 verbatim 그대로 유지, 매칭 대상만 `text` 컬럼으로 한정. V008 migration / design §5.5 verbatim 블록 변경 불필요.

**변경**:
- `crates/kebab-search/src/lexical.rs::build_match_string` 가 non-raw 분기에서 combined expression 을 `text : (<expr>)` 로 wrap. FTS5 column filter syntax (`column:expr`) 가 OR/AND sub-expression 허용 — 한국어 trigram 빌더의 `(whole) OR (token_and)` 형태가 그대로 들어감.
- Raw mode (`'...'`) 는 변경 없음 — 사용자가 명시 의도로 `'heading_path : agent'` 같은 explicit column filter opt-in 가능 (escape hatch).
- 9 unit test (8 갱신 + 1 신규) + 2 신규 통합 test (`crates/kebab-search/tests/lexical.rs`) = 11 total:
  - `build_match_string_*` 8 expected string 갱신 (column filter prefix 추가)
  - `build_match_string_raw_mode_preserves_heading_filter` 신규 unit — raw mode 가 `heading_path : ...` 보존
  - `lexical_heading_only_token_does_not_hit_default_mode` 신규 통합 — heading-only unique token 이 default mode 에서 0 hit
  - `lexical_raw_mode_can_opt_into_heading_path_filter` 신규 통합 — 같은 fixture 가 raw mode 로 hit 확인
- `integrations/claude-code/kebab/SKILL.md` 의 search 절에 column scoping + heading_path raw-mode escape hatch 안내 한 bullet 추가 (회차 1 follow-up suggestion 반영, 본 PR 에 포함).

**사용자 영향**:
- 기본 lexical / hybrid 검색에서 heading 만 매칭되던 false positive 차단. 한국어 / 영어 substring 매칭의 recall 은 그대로 (text 본문에 있는 token 은 변함없이 hit). 본문 검색의 precision 가 올라감.
- heading 으로 일부러 검색하던 사용자는 `'heading_path : <token>'` 형태로 raw mode 진입. CLI / TUI / MCP 모든 surface 동일.
- `kebab.sqlite` 크기 변화 없음 (색인 column 그대로 유지). re-ingest 불필요 (FTS query 시점의 매칭 범위만 변경).
- BM25 score 영향: `lexical_snapshot_run_1` + `hybrid_snapshot_run_1` 둘 다 column filter 적용 후에도 점수 동일 (text 본문에만 매칭되던 query 라 column filter 가 점수 분포에 영향 안 줌). fixture regenerate 불필요.

**MCP / agent 가시성**: `search_response.v1` 의 wire shape 변경 없음. 사용자가 heading 검색을 명시 의도하던 케이스는 raw mode 안내 — `integrations/claude-code/kebab/SKILL.md` 의 search 절은 v0.17.0 의 raw mode 안내 (`'foo OR bar*'`) 가 그대로 적용. 별도 SKILL.md 갱신 불필요 (raw mode 가 이미 documented escape hatch).

Cross-link: `crates/kebab-search/src/lexical.rs::build_match_string`, `migrations/V007__fts_trigram.sql` (verbatim 유지), design §5.5 (verbatim 유지, query-time 동작만 변경).

## 2026-05-24 — v0.17.0: 한국어 trigram FTS5 tokenizer 채택 (closure of 2026-05-22 한국어 lexical)

V007 migration 으로 `chunks_fts` 의 tokenizer 를 `unicode61` → `trigram` 으로 교체. `chunks` 원본 + embedding + vector index 는 그대로, FTS shadow 만 재구축 + 자동 backfill — 사용자는 `kebab ingest` 재실행 불필요 (binary 만 교체하면 다음 open 시 V007 가 즉시 적용). 같은 라운드의 다른 두 follow-up (`code_lang_chunk_breakdown`, C typedef) 은 별 PR (PR-C / PR-B).

**한국어 lexical 동작**: 3자 이상 substring 매칭. `해시 충돌` 같은 2자 토큰 multi-token query 는 `crates/kebab-search/src/lexical.rs::build_match_string` 의 trigram-aware 재설계로 `("해시 충돌") OR ("해시" "충돌")` 형태가 되어 whole-phrase 후보로 hit (각 토큰 2자라 token-AND 후보는 trigram 에서 0-hit, 자동 drop). 한영 혼합 `Rust 충돌은` (둘 다 ≥3자) 도 OR-combined. 2자 이하 query (`충돌` / `키`) 는 정상 0 hit + CLI stderr `[hint] 3자 이상 키워드 권장 (trigram tokenizer 제약)` + `search_response.v1.hint` additive 필드 + TUI status bar 동일 안내. raw FTS5 single-quote mode (`'...'`) 는 사용자 명시 의도이므로 hint 안 나옴. 회귀 핀: `lexical_multi_token_korean_query_hits` + `lexical_mixed_korean_english_multi_token_query_hits` (`crates/kebab-app/tests/search_korean.rs`).

**영어 lexical 동작 변경**: substring 매칭으로 바뀜. `token` query 가 `tokenizer` 도 hit (recall ↑, 단어 경계 정밀도 ↓). 의도된 변경, 회귀 핀 = `fts_trigram_english_substring_hits` (`crates/kebab-store-sqlite/tests/fts.rs`).

**lexical BM25 score 분포**: 알고리즘 동일하지만 token stream 이 word → overlapping trigram 으로 바뀌어 raw score / TF / doc-length 모두 달라짐. `crates/kebab-search/tests/lexical.rs::lexical_snapshot_run_1` + `crates/kebab-search/tests/hybrid.rs::hybrid_snapshot_run_1` 둘 다 trigram baseline 으로 regenerate. hybrid (RRF) 는 rank 기반이라 ranking 영향 미미하나 `retrieval.lexical_score` 노출값은 변동.

**디스크 용량**: trigram 인덱스는 unicode61 대비 통상 2-10배. V007 자동 backfill 후 `kebab.sqlite` 파일 크기 증가 (도그푸딩 KB 기준 ~2-5배 또는 수백 MB). release notes 명시.

**`heading_path_json` JSON 노이즈 (관찰, 미수정)**: trigram 이 JSON 표기 (`[`, `"`, `,`) 와 그 안의 단어 (`app`, `src`) 까지 3-gram 색인 → query 가 우연히 JSON 구문 / 흔한 경로 단어와 겹쳐 false positive 가능. v0.17.0 에서는 컬럼 구성 유지, 도그푸딩 후 column filter (`{text} : <q>` 한정) 또는 평문 heading 변환 결정. 후속 도그푸딩 entry 로 등재 예정. → **closure**: 아래 2026-05-25 v0.17.1 post-dogfood heading text column filter entry 참조 (column filter 방식 채택, V008 migration 불필요).

**MCP / agent 가시성**: `search_response.v1` 에 `hint: Option<String>` additive 필드. 결과가 비어 있고 query trimmed.chars().count() < 3 + raw mode 아닐 때만 set (helper `kebab_app::short_query_hint`). `integrations/claude-code/kebab/SKILL.md` 의 search 절에 "한국어 lexical 은 3자 이상 권장, `hint` 필드 확인" 안내 추가.

Cross-link: `migrations/V007__fts_trigram.sql`, `crates/kebab-search/src/lexical.rs::build_match_string`, design §5.5, `docs/superpowers/specs/2026-05-22-korean-trigram-tokenizer-design.md`.

## 2026-05-22 — p10 종합 도그푸딩 (round 2): 한국어 lexical 검색 한계 + code_lang_breakdown

**Origin**: P10 종합 도그푸딩 round 2 (`/build/cache/dogfood-p10b/`). 다양한 OSS 코드베이스 8 repo (rust / python / go / ts / js / java / c / cpp) + 한국어 위키 기술 문서 10편 (pandoc HTML→gfm 변환). `multilingual-e5-small` embedding 활성화 후 ingest — `scanned=2663 updated=2080 errors=0` (k8s multi-resource chunk_id collision 은 같은 라운드에서 발견·수정 — 아래 2026-05-21 항목).

### 한국어 lexical 검색이 FTS5 unicode61 토크나이저에서 무용 (vector/hybrid 가 우회)

**Symptom**: `kebab search --mode lexical` 의 한국어 query 가 거의 0 hit. "충돌" 은 hash-table.md 본문에 37회(21회 단독 어절) 등장하나 lexical 0 hit. 4개 한국어 query 측정 — lexical: `충돌` 0 / `해시 충돌` 0 / `컴파일러 최적화` 0 / `트리 순회 방법` 1.

**원인**: `chunks_fts` 의 `tokenize = 'unicode61 remove_diacritics 2'` (`migrations/V002__fts.sql:24`, design §5.5 verbatim 블록). unicode61 은 공백·구두점 경계로만 토큰을 끊는다 — 한국어는 어절 전체가 한 토큰이 되고 조사·어미가 붙은 채라 부분 매칭이 안 된다. V002 헤더 주석이 이미 "Korean morphological tokenizer is a P+ note" 로 예고한 사항.

**검증 (vector/hybrid 우회 확인)**: 동일 4 query 를 `--mode vector` / `--mode hybrid` 로 측정 — 전부 10 hit. `multilingual-e5-small` semantic 검색이 한국어를 정상 처리. 즉 embedding 켠 KB 는 **기본 hybrid 모드에서 한국어 검색이 동작**한다. 단 hybrid 는 RRF(lexical+vector) fusion 이라 한국어 query 는 lexical 기여가 0 → 사실상 vector-only 로 reduced (score 증거: lexical 도 hit 한 `트리 순회 방법` 만 hybrid score 1.000, 나머지 한국어 query 는 0.500).

**Status**: ✅ closed — v0.17.0 (2026-05-24) 에서 V007 trigram migration + `lexical.rs::build_match_string` trigram-aware 재설계로 해소. 영향은 위 2026-05-24 절 참조. 이하는 closure 전 원래 round-2 관찰 기록 (frozen).

**Workaround (pre-v0.17.0)**: 한국어 문서 KB 는 embedding 활성화 (`[models.embedding] provider = "fastembed"`) 가 사실상 필수였다 — vector / hybrid 가 한국어를 carry.

**Resolution (v0.17.0)**: FTS5 builtin `trigram` tokenizer 채택. `chunks_fts` 재생성 = V007 migration (`chunks` 원본 / embedding / vector 불변, FTS shadow 만 자동 backfill — re-ingest 불필요). design §5.5 verbatim 블록 + CI diff-check (`fts_v007_matches_design_section_5_5_verbatim`) 동반 갱신.

### code_lang_breakdown 이 chunk 수가 아닌 doc 수를 집계

**Symptom**: `schema.v1.stats.code_lang_breakdown` 이 언어별 *문서* 수를 보고. 코드가 많은 KB 에서 언어별 chunk 분포를 보려 할 때 granularity 가 doc 단위라 덜 유용.

**Status**: LOW. `code_lang_breakdown` 은 p10-1A-2 가 의도적으로 doc count 로 구현 (`store.rs::code_lang_breakdown` doc 주석 + `COUNT(*) FROM documents GROUP BY code_lang`). design §3.5 의 "언어별 분포" 의도와 엄밀히는 어긋나나 통계 표시 한정 — 검색/ingest 동작 무관.

**Next step**: chunk 단위 집계를 추가/교체하는 소규모 follow-up. wire schema 영향 시 additive 필드 (`code_lang_chunk_breakdown`) 로 처리 검토.

### ranking — glue chunk 이 top hit (deferred 유지)

multi-root 도그푸딩(2026-05-20)에서 관찰한 본문 vs 테스트 / glue chunk ranking 편향이 round 2 에서도 재확인됨. 자동 heuristic 은 user intent misalignment 위험 → 사용자 명시 요청 전까지 surface 변경 0 으로 유지 (project memory `project_ranking_deferred` 결정 그대로).

Cross-link: `tasks/p10/INDEX.md`, `migrations/V002__fts.sql`, design §5.5 / §3.5.

## 2026-05-24 — v0.17.0 PR-B: C typedef-wrapped struct/enum/union 이 typedef alias unit 으로 방출 (closure of 2026-05-21)

`crates/kebab-parse-code/src/c.rs::extract_blocks` 에 `type_definition` 분기 추가. 내부 anonymous `struct_specifier` / `enum_specifier` / `union_specifier` (name field 없음) 인 typedef 일 때 declarator 의 typedef alias identifier 를 추출해 synthetic unit 방출. named inner aggregate (`typedef struct Pt { ... } P;`) 와 plain alias (`typedef int MyInt;`) 는 기존대로 glue (top-level typedef-wrapped anonymous aggregate 만 v2 의 1차 범위).

**parser_version cascade**: `PARSER_VERSION` `code-c-v1` → `code-c-v2` bump. design §9 — `doc_id = (workspace_path, asset_id, parser_version)`. 같은 file (asset_id 불변) + 새 parser_version → 새 doc_id. 즉 같은 workspace_path 에 옛 doc_id 와 새 doc_id 가 동시 INSERT 시도 → `idx_docs_workspace_path` UNIQUE 충돌.

**Same-workspace_path orphan purge (B1 Step 5b)**: `crates/kebab-store-sqlite/src/store.rs` 에 두 helper 신규 — `stale_chunk_ids_for_workspace_path_except_doc_id(workspace_path, keep_doc_id)` (chunk_ids 수집) + `purge_document_at_workspace_path_except_doc_id(workspace_path, keep_doc_id)` (CASCADE document/chunks 제거). `crates/kebab-app/src/lib.rs::try_skip_unchanged` 의 parser_mismatch 분기에서 `purge_workspace_path_for_parser_bump` wrapper 호출 → 옛 chunk_ids 의 LanceDB orphan 도 `delete_by_chunk_ids` 로 정리 후 SQLite document row 제거 → 이후 `Ok(None)` 반환 → caller 가 새 doc_id 로 INSERT. 기존 `purge_orphan_at_workspace_path` (asset_id 변경 케이스) 는 그대로 — bytes 변경 경로 회귀 없음.

**사용자 영향**: 기존 v0.16.x KB 의 C 파일은 v0.17.0 binary 로 다음 ingest 시 자동 재처리 (parser_version mismatch → cleanup → 새 doc). 명시적 re-ingest 명령 불필요 (다음 `kebab ingest` 가 자연스럽게 처리). `typedef struct {...} Foo;` 가 `Citation::Code.symbol = "Foo"` 로 search 에 노출.

**미해결 (Risks)**: nested typedef (`typedef struct { struct {...} inner; } Outer;`) 의 inner 익명 struct 는 여전히 glue — v2 의 1차 범위는 top-level typedef alias 만.

Cross-link: `crates/kebab-parse-code/src/c.rs::recover_typedef_alias`, `tasks/p10/p10-1d-c-cpp-ast-chunker.md` Risks/notes section.

## 2026-05-24 — v0.17.0 PR-C: `code_lang_chunk_breakdown` additive wire 필드 (closure of 2026-05-22 LOW)

`schema.v1.stats` 에 `code_lang_chunk_breakdown: { <lang>: <chunk_count> }` additive 필드 추가. 기존 `code_lang_breakdown` (doc 수) 와 sister — chunk 수 집계로 indexing 압력 granularity 노출. 한 PDF spec → 200 chunks vs 한 Rust file → 5 chunks 가 동일한 `1 doc` 으로 보이던 한계 closure.

**구현**: `crates/kebab-store-sqlite/src/store.rs::code_lang_chunk_breakdown()` — `chunks INNER JOIN documents` 후 `json_extract(d.metadata_json, '$.code_lang')` GROUP BY, `COUNT(c.chunk_id)`. `BTreeMap<String, u32>` 반환 (기존 helper 와 동일 shape). `crates/kebab-app/src/schema.rs::Stats` 에 동일 이름 필드 추가 + `collect_stats` builder 에서 호출. `docs/wire-schema/v1/schema.schema.json` 에 additive 필드 명세. **additive 변경 — wire breaking 아님, `schema_version` bump 불필요.**

**Gemini round 2 권고 반영**: 기존 `code_lang_breakdown` / `repo_breakdown` 의 JSON schema description 이 "code chunk count" 로 잘못 적혀 있던 (실제는 doc count) 부분을 "doc count" 로 정정. 신규 필드만 "chunk count" 로 명시. 사용자가 두 metric 의 의미 차이를 schema 만 보고도 구분 가능.

**사용자 영향**: `kebab schema --json` 출력에 신규 키 등장. MCP `schema` tool 도 동일. 옛 v0.16.x 가 보낸 호출은 그대로 동작 (additive).

Cross-link: `crates/kebab-store-sqlite/src/store.rs::code_lang_chunk_breakdown`, `docs/wire-schema/v1/schema.schema.json`.

## 2026-05-21 — p10-2: k8s multi-resource YAML chunk_id collision

**Origin**: P10 종합 도그푸딩 (`/tmp/kebab-p10-dogfood/`, 16 파일). 한 파일에 2+ k8s document (Deployment + Service, `---` 구분) 인 YAML 이 ingest 실패.

**Symptom**: `DocumentStore::put_chunks (code): UNIQUE constraint failed: chunks.chunk_id`. document row 는 생성되나 chunk 0개 → 검색 불가. p10-2 의 통합 테스트 `tier2_k8s_yaml_ingest_searchable` 가 single-Deployment fixture 만 써서 미발견.

**원인**: `tier2_shared::push_chunks_with_oversize` 의 non-oversize 분기가 `split_key = None` 하드코딩. `K8sManifestResourceV1Chunker` 가 resource 마다 호출 — 같은 document 의 모든 resource 가 `doc_id` + `chunker_version` + `base_policy_hash` 공유 + `split_key = None` → 동일 `id_hash` → 동일 `chunk_id`. p10-3 의 `code_text_paragraph_v1` 가 같은 버그였고 `df3c5b8` 에서 fix 됐지만 그건 `build_chunk_no_symbol` 직접 호출 경로, `push_chunks_with_oversize` 경로는 미수정.

**Fix** (PR #158, v0.16.1): `push_chunks_with_oversize` 에 `base_split_key: Option<u32>` 추가. k8s chunker 가 `Some(resource.line_start)` 전달 → resource 별 distinct chunk_id. dockerfile / manifest 는 `None` (파일당 1 chunk, 충돌 없음, chunk_id 불변).

**Deviation note**: single-resource k8s YAML 의 chunk_id 도 `None → Some(1)` 으로 바뀜 (`id_hash` 가 `base_policy_hash` → `base_policy_hash#L1`). `chunker_version` (`k8s-manifest-resource-v1`) 은 의도적으로 bump 안 함 — p10-2 가 v0.14.0 (~1주 전) 머지된 dogfood 단계라 prod KB 없음. v0.14.0~v0.16.0 사이 single-resource k8s 를 색인한 KB 는 re-ingest 시 old chunk 가 orphan 될 수 있으나 (UNIQUE 충돌 아님 — 다른 id), `kebab reset` 또는 re-ingest sweep 으로 정리됨. dogfood-only 단계라 chunker_version bump (전체 re-process) 보다 가벼운 선택.

Cross-link: `tasks/p10/p10-2-tier2-resource-aware.md` Risks/notes section.

## 2026-05-21 — p10-1D: typedef-wrapped struct/enum in C falls into glue

**Origin**: PR #156 (p10-1d) code-reviewer review. Verified during dogfood.

**Symptom**: `typedef struct { ... } Foo;` in a `.c` file does NOT emit a struct-level unit. tree-sitter-c classifies the construct as a top-level `type_definition` with an *anonymous* inner `struct_specifier` (no `name` field), so the extractor's `struct_specifier` arm doesn't fire — the whole declaration falls into `<top-level>` glue. The named typedef alias `Foo` is therefore not searchable as a symbol.

**Status**: ✅ closed — v0.17.0 (2026-05-24) PR-B 에서 extractor 의 `type_definition` 분기 추가로 해소. 영향은 위 2026-05-24 PR-B 절 참조. 이하는 closure 전 round-2 dogfood 관찰 기록 (frozen).

**Workaround (pre-v0.17.0)**: search the struct by its field/function names, or use `--code-lang c` to broaden scope. Typedef-aliased struct names won't surface as `Citation::Code.symbol`.

**Resolution (v0.17.0)**: extractor 가 top-level `type_definition` 노드를 만나 내부 anonymous `struct_specifier` / `enum_specifier` / `union_specifier` 가 있으면 `declarator` field 의 typedef alias 이름으로 synthetic unit 방출. `PARSER_VERSION` `code-c-v1` → `code-c-v2` bump. design §9 cascade 동작 — 같은 `(workspace_path, asset_id)` 의 `doc_id` 가 새 parser_version 으로 다르게 계산됨. 옛 doc/chunks row + LanceDB orphan 회피용 same-workspace_path orphan purge helper 동반 (`stale_chunk_ids_for_workspace_path_except_doc_id` + `purge_document_at_workspace_path_except_doc_id`).

Cross-link: `tasks/p10/p10-1d-c-cpp-ast-chunker.md` Risks/notes section.

## 2026-05-20 — p10-1B: Rust 1A-2 symbol path is file-scope-only; 1B+ uses workspace path → module prefix

**무엇이 바뀌었나**: P10-1A-2 의 Rust `code-rust-ast-v1` chunker 가 생성하는 symbol 은 file-scope mod-path nesting 만 사용한다 (예: `Foo::double`). P10-1B 이후 Python / TypeScript / JavaScript 의 symbol 은 workspace 경로 → module path prefix 를 포함한다 (예: `kebab_eval.metrics.compute_mrr`, `src/Foo.Foo.search`).

**원인**: 1A-2 는 symbol path 컨벤션이 확정되기 전에 구현됐고, 1B spec 에서 workspace path → module prefix 를 명시적 결정으로 확정했다 (p10-1b-py-ts-js-ast-chunkers.md §동결된 설계 결정). 1A-2 retrofit = `chunker_version` bump + Rust corpus 전체 re-ingest 비용이 수반됨.

**사용자 가시적 영향**: Rust 코드 검색 시 symbol 이 `<ClassName>::<method>` 형태 (workspace prefix 없음). Python/TypeScript/JavaScript 는 `<module.path>.<symbol>` / `<module/path>.<symbol>` 형태. 비일관이지만 각각은 일관되게 동작.

**proper fix**: Rust AST chunker 에 `module_path_for_rust(workspace_path)` helper 추가 + `chunker_version = "code-rust-ast-v2"` bump → 사용자가 명시 요청할 때까지 보류.

**cross-link**: `tasks/p10/p10-1b-py-ts-js-ast-chunkers.md` Risks / notes 섹션, design §3.4.

## 2026-05-20 — p10-1B: module_path_for_python / _tsjs do not sanitize non-ASCII / 공백 / 특수문자 in workspace path

**동작**: `module_path_for_python` 와 `module_path_for_tsjs` 가 workspace path 의 비-ASCII / 공백 / 따옴표 / 백슬래시 같은 특수문자를 그대로 prefix 에 통과시킨다. 예: `kebab eval/metrics.py` (공백 포함) → module prefix `kebab eval.metrics` — 라이브러리 코드는 동작하지만 symbol 텍스트에 공백이 들어간다.

**이유**: 1B 1차 단순화. 대다수 코드 베이스가 ASCII identifier + `/` 구분자만 사용하므로 사용자 경험상 영향 미미.

**해결**: 후속 phase 에서 path-sanitize 추가 검토. NFKC normalize 후 `[^A-Za-z0-9_.\-/]` → `_` 변환 식. 적용 시 chunker_version bump 트리거 (re-ingest cascade 필요).

**cross-link**: `tasks/p10/p10-1b-py-ts-js-ast-chunkers.md` Risks / notes 섹션 line 55.

## 2026-05-20 — p10-1B: expression-level functions (arrow fn, function expression assigned to const) NOT emitted as units in 1B 1차

**무엇이 바뀌었나**: TypeScript / JavaScript 의 `const foo = () => {...}` 또는 `const bar = function() {...}` 같은 expression-level 함수 할당은 `code-ts-ast-v1` / `code-js-ast-v1` 에서 독립 unit 으로 방출되지 않는다. 해당 코드는 가장 가까운 surrounding declaration-level unit (또는 `<top-level>` glue) 에 흡수된다.

**원인**: `function_declaration` / `class_declaration` / `method_definition` / `interface_declaration` 같은 declaration-level 노드만 unit 으로 선택. `lexical_declaration` (= `const / let / var`) 안의 function / arrow expression 은 별도 unwrap 없이 pass-through. 1B 1차 단순화.

**사용자 가시적 영향**: expression-level 함수 이름으로 검색 시 함수 body 를 포함하는 glue chunk 가 반환되지만, symbol 이 함수 이름 자체를 가리키지는 않는다. 함수명이 함수 본문 텍스트에 등장하므로 lexical / hybrid 검색으로 일반적으로 찾을 수 있다.

**proper fix**: `lexical_declaration` visitor 에서 binding value 가 `arrow_function` / `function` expression 인 경우 해당 identifier name 을 symbol 로 사용하는 unwrap 추가. 후속 phase 에서 검토.

**cross-link**: `tasks/p10/p10-1b-py-ts-js-ast-chunkers.md` Risks / notes 섹션.

## 2026-05-19 — p10-1A-2: AST_CHUNK_MAX_LINES constant vs config deviation

**무엇이 바뀌었나**: `kebab-chunk/src/code_rust_ast_v1.rs` 가 `IngestCodeCfg.ast_chunk_max_lines` config 값을 읽지 않고 모듈 상수 `AST_CHUNK_MAX_LINES = 200` 으로 고정함.

**원인**: 현행 `Chunker` trait 이 per-medium config 를 인자로 받지 않는다. PDF 선례 (`pdf-page-v1` 의 pinned `chunker_version`) 와 같은 패턴 — chunker 가 config 를 bolt-on 으로 받을 수 있는 per-medium chunker registry 는 P+ task.

**사용자 가시적 영향**: 없음 (상수 200 이 `IngestCodeCfg::default().ast_chunk_max_lines` 와 동일). 사용자가 config 에서 `ast_chunk_max_lines` 를 변경해도 Rust AST chunker 에는 반영 안 됨.

**proper fix**: per-medium chunker registry 도입 시 `RustAstV1Chunker` 가 `IngestCodeCfg` 를 주입받도록 변경. 별도 P+ task.

**cross-link**: `tasks/p10/p10-1a-2-rust-ast-chunker.md` Risks / notes 섹션 참조.

## 2026-05-19 — p10-1A-2: SourceType::Code deferred — code files classified SourceType::Note

**무엇이 바뀌었나**: `kebab-core` 의 `SourceType` enum 에 `Code` variant 가 없어 `kebab-parse-code::RustAstExtractor` 가 `SourceType::Note` 로 fallback 함.

**원인**: `SourceType::Code` 추가는 additive (소규모) 변경이지만, 1A-2 PR 스코프를 넓히지 않기 위해 명시적으로 deferred. Plan 이 이 fallback 을 예상했음 — 기능 회귀 아님.

**사용자 가시적 영향**: 없음. `--media code` / `--code-lang rust` filter 는 `MediaType::Code("rust")` 기반으로 동작 (SourceType 과 독립). 현재 code 파일에 source_type 기반 필터링 표면 없음.

**proper fix**: `kebab-core::SourceType` 에 `Code` variant 추가 + `citation_helper` + `store-sqlite` 의 exhaustive match 갱신. 별도 소규모 task (P10-1A-2 follow-up).

**cross-link**: `tasks/p10/p10-1a-2-rust-ast-chunker.md` Risks / notes 섹션 참조.

## 2026-05-10 — p9-fb-39b: embedding upgrade UX

**무엇이 바뀌었나**: default embedding 이 `multilingual-e5-small` (384 dim) 에서 `multilingual-e5-large` (1024 dim) 로 변경. LanceDB 테이블은 `(model, dim)` 으로 네임스페이스되어 새 모델은 fresh 테이블에 쓰고, 옛 `chunk_embeddings_multilingual-e5-small_384` 테이블은 orphan 상태 됨.

**user TOML 에 small 명시한 경우**: backwards-compat 유지. 사용자가 `[models.embedding] model = "multilingual-e5-small"` 로 명시했으면 그대로 small 사용 (새 default 무시).

**idempotent re-embed**: fb-23 incremental ingest 가 embedding_version mismatch 감지하면 자동으로 이전 chunk 를 새 모델로 re-embed. 다음 `kebab ingest` 호출 시 기존 chunk 의 embedding 을 새 테이블에 재작성.

**disk 절약**: 이전 모델의 orphan 테이블을 먼저 정리하려면 `kebab reset --vector-only` 실행 (LanceDB + SQLite `embedding_records` 모두 wipe). 이후 `kebab ingest` 가 모든 chunk 를 새 모델로 re-embed 해 새 테이블 채움.

**search/ask 결과**: re-embed 전까지는 empty hit (새 모델에 데이터가 없음). `kebab ingest` 후 정상 검색 가능.

**Spec contract 와의 관계**: design §5 (storage) + §9 (versioning cascade) 의 embedding_model.id / dimensions 변경. wire 의 `embedding_version` 필드 (kebab-app schema.v1.models.embedding_version 가 config.models.embedding.model 값을 그대로 emit) 변경 — CLAUDE.md cascade rule 의 release 트리거. 본 PR 머지 후 `chore: bump version 0.5 → 0.6` + tag 필요.

**Spec deviation**: design `2026-05-10-p9-fb-39b-embedding-upgrade-design.md` 의 §Migration policy + §Public surface delta 가 `LanceVectorStore::open` 안 신규 `error.v1.code = "embedding_dim_mismatch"` 명시했으나 구현 제외. 이유: LanceDB tables 가 `(model, dim)` namespaced — silent orphan + empty-hit 으로 surface (hard error 아님). 명시 error 필요 시 별도 startup health check 작업 필요 (fb-39c 후보 또는 doctor 확장).

## 2026-05-09 — p9-fb-34: search wire wrapped in search_response.v1

**무엇이 바뀌었나**: `kebab search --json` stdout 이 기존 `search_hit.v1[]` 배열에서 신규 `search_response.v1` object 로 교체. wrapper 가 `hits`, `next_cursor`, `truncated` 세 필드를 가짐.

**Spec contract 와의 관계**: 명시적 wire breaking change. spec `docs/superpowers/specs/2026-05-09-p9-fb-34-output-budget-controls-design.md` 의 §Wire shape 절에 단일 출처 결정.

**의식적 결정**:
- pagination + truncation metadata 를 `search_hit` 자체에 흡수하면 단일 hit 의 도메인 의미가 오염됨 (모든 hit 가 `next_cursor` 필드 보유 등). top-level wrapper 가 분리도 깨끗.
- 외부 consumer 영향: 단일 사용자 환경 + Claude Code skill 한 곳. skill 은 fb-34 와 동시 갱신.
- 이 변경은 search_hit.v1 자체 schema 는 손대지 않음 — 도메인 stable.

**영향 받는 consumer**: kebab-tui (Search 패널 — 변경 불필요, App::search 시그니처 보존), kebab-mcp (search tool — 같은 PR 에서 갱신), Claude Code skill (같은 PR 에서 갱신). 외부 producer/consumer 없음.

**`--no-cache` 의미 변화**: fb-34 이전 `--no-cache` 는 `search_uncached_with_config` 로 cache 자체를 우회. fb-34 는 cached path 위에 `clear_search_cache()` 호출 후 search 실행 — long-lived process (TUI / MCP) 에서는 clear 와 fetch 사이 race window 가 있음. CLI (fresh App per call) 에서는 무영향. 후속 fb-3X 에서 `search_with_opts_uncached` 추가로 격리.

## 2026-05-09 — p9-fb-33: AskOpts.stream_sink type widened to StreamEvent

**무엇이 바뀌었나**: `kebab_rag::AskOpts.stream_sink` 의 타입이 `Option<mpsc::Sender<String>>` 에서 `Option<mpsc::Sender<StreamEvent>>` 로 변경됨. `kebab_app::StreamEvent` 가 새 re-export.

**Spec contract 와의 관계**: `answer_event.v1` (신규 wire schema) 가 단일 sink 로 3 stage (retrieval_done / token / final) 를 운반하도록 강제하면서 자연스럽게 in-process sink 의 type 폭이 넓어진 부산물. spec `docs/superpowers/specs/2026-05-09-p9-fb-33-streaming-ask-design.md` 의 "Domain API change" 절에서 미리 명시. consumer = TUI worker 한 곳 (이번 PR 에서 같이 갱신). 외부 consumer 없음.

**의식적 결정**:
- single sink 로 retrieval / token / final 세 stage 를 모두 운반하기 위한 필수 타입 변경.
- 기존 `Sender<String>` 으로는 retrieval / final 단계를 표현할 방법이 없음.
- internal API 라 wire schema 와 다름 — `answer_event.v1` 는 신규 schema (additive minor at wire layer).

**영향 받는 consumer**: `kebab-tui::ask::spawn_ask_worker` (PR #124 에서 동시 갱신). 외부 통합 없음.

## 2026-05-09 — p9-fb-32: search_hit.v1 / citation.v1 required-field expansion

**무엇이 바뀌었나**: `search_hit.v1` 과 `citation.v1` 의 `required` 배열에 `indexed_at` (RFC3339) + `stale` (bool) 두 필드가 추가됨. `schema_version` 은 그대로 (`search_hit.v1` / `citation.v1`).

**Spec contract 와의 관계**: 본 PR 에서는 additive minor 로 분류했으나 strict JSON Schema validator 입장에서는 pre-fb-32 payload 가 invalid 가 됨. CLAUDE.md `Wire schema v1` 절의 "breaking it requires a *.v2 major bump" 와 엄밀히는 충돌.

**의식적 결정**:

- single-user / single-producer 환경 (kebab CLI + MCP server 가 동일 binary) 에서는 producer 가 항상 새 필드를 채우므로 실용적 호환성 영향 없음.
- v2 cascade 로 가면 schema 파일 + 모든 consumer 코드 + integration 테스트가 `.v2` 로 동시 bump 가 필요한데, 두 필드 추가만으로 그 비용은 과함.
- producer-controlled 환경의 minor bump 로 처리. 향후 외부 third-party producer 가 등장하면 그 시점에 v2 cascade 검토.

**영향 받는 consumer**: 없음 (현재 모든 consumer 가 동일 repo 내 — `kebab-cli`, `kebab-tui`, `kebab-mcp`, `integrations/claude-code/kebab/`).

## 2026-05-07 (2)

### macOS XDG path collision: `data_dir` == `config_dir` → DataOnly reset deletes config

- **File**: `crates/kebab-config/src/lib.rs`
- **Root cause**: `dirs` crate 가 macOS 에서 `config_dir()` 과 `data_dir()` 모두 `~/Library/Application Support/` 반환. `ResetScope::DataOnly` 가 `data_dir` 을 삭제하면 config 파일까지 함께 삭제됨.
- **Fix**: `xdg_config_path`, `xdg_data_dir`, `xdg_cache_dir` 의 `dirs` fallback 제거 → `$HOME/.config`, `$HOME/.local/share`, `$HOME/.cache` 직접 사용 (XDG 표준, 플랫폼 무관).
- **Migration**: `Config::load(None)` 에서 새 경로 없고 macOS legacy (`~/Library/Application Support/kebab/config.toml`) 있으면 자동 copy + stderr 안내.
- **New paths** (macOS):
  - config: `~/.config/kebab/config.toml` (was `~/Library/Application Support/kebab/config.toml`)
  - data: `~/.local/share/kebab/` (was `~/Library/Application Support/kebab/`)
  - cache: `~/.cache/kebab/` (was `~/Library/Caches/kebab/`)
  - state: `~/.local/state/kebab/` (unchanged)

## 2026-05-07

### fb-26: ingest 로그 `Aborted` 무조건 writeln + `Completed` TTY 요약 없음

- **File**: `crates/kebab-cli/src/progress.rs`
- `Aborted` 핸들러가 TTY 모드에서도 무조건 `writeln!` 하여 `bar.abandon_with_message` 아래에 중복 출력 발생. Fixed: `if !tty && !quiet` 로 가드.
- `Completed` TTY 경로가 `bar.finish_and_clear()` 호출 후 요약 라인 없음. Fixed: `!quiet` 일 때 항상 `ingest: complete (...)` writeln 출력.
- `KEBAB_PROGRESS=plain` env override 추가 — CI pty wrapper 에서 TTY 감지 강제 제거.
- `ProgressMode::Human` 에 `quiet: bool` 필드 추가; `--quiet` flag 전체 progress stderr 억제.

### fb-28: `--readonly` / `--quiet` 전역 flag + `readonly_mode` error code

- **File**: `crates/kebab-cli/src/main.rs`
- `--readonly` (또는 `KEBAB_READONLY=1`) — mutating subcommand (`ingest`, `ingest-file`, `ingest-stdin`, `reset`) 차단. exit code 1.
- `--json --readonly` — stderr 로 `error.v1` 신규 code: `"readonly_mode"` emit.
- `--quiet` — 모든 human-readable stderr (progress, hint) 억제; error 는 여전히 stderr 도달.
- `--json` 자동 quiet 함축 (명시적 현재).
- `error.v1` code: `"readonly_mode"` main() guard block 에서 직접 construction (classify() 경로 아님).

## 2026-05-07 — p9-fb-31 (post-dogfooding): single-file / stdin ingest

**Source feedback**: 사용자 도그푸딩 2026-05-06 — agent (Claude Code via MCP, fb-30) 가 web fetch 한 markdown / 단일 외부 file 을 KB 에 저장하려면 `kebab ingest` 전체 walk 재실행 비효율. agent 메모리상 string contents 도 stdin ingest 가능해야.

**Live binding 변경**:

- 신규 subcommand `kebab ingest-file <path>` — 단일 file ingest, workspace 외부 path 가능.
- 신규 subcommand `kebab ingest-stdin --title <T> [--source-uri <URI>]` — stdin 의 markdown 본문 ingest, v1 markdown only.
- 신규 MCP tool `ingest_file` + `ingest_stdin` — fb-30 v1 read-only 정책 변경, 첫 mutation surface 도입 (의도된 진화). tools/list 4 → 6.
- 외부 file 저장 정책: `<workspace.root>/_external/<blake3-12>.<ext>` 로 copy. deterministic 명명 → idempotent. `_external/` 첫 생성 시 `.kebabignore` 자동 append (walk 무한 루프 방지).
- `.kebabignore` 매치 시 stderr warn (`warn: <path> matches .kebabignore patterns; proceeding (explicit ingest bypasses ignore)`) 후 진행. `--force-ignore` flag 불필요 — explicit ingest 가 default bypass intent.
- stdin frontmatter 처리: 본문이 `---` 으로 시작하면 error (`use kebab ingest-file`); 그 외 frontmatter block prepend (title + 옵션 source_uri, YAML 더블쿼트 escape).
- `kebab-app::external` 신규 모듈 — `ensure_external_dir`, `ensure_kebabignore_entry`, `copy_to_external`, `inject_frontmatter` helper. kebab-cli + kebab-mcp 둘 다 facade 통해 호출.
- `kebab-app::ingest_file_with_config` + `ingest_stdin_with_config` 신규 facade fn.

**Spec contract impact**: design §6 에 `_external/` subdirectory 절 추가 (실제 §6.7 — 기존 §6 sub-section 이 6.6 까지 채워져 있어 §6.7 로 부착됨; spec stub 의 §6.3 명시는 deviation).

**Tests added**: kebab-app external::tests (14: dir / kebabignore append / copy / inject_frontmatter / yaml_quote), kebab-app integration (3 + 3: ingest_file + ingest_stdin), kebab-cli integration (2: cli_ingest_file + cli_ingest_stdin spawn-based), kebab-mcp integration (1 + 2: tools_call_ingest_file + tools_call_ingest_stdin), tools_list assertion update (4 → 6).

**Known limitation (deferred)**:

- PDF / image stdin — binary stream + base64 처리 v2.
- `--title` + `--source-uri` 외 metadata field (tags, language, custom kv) — v2.
- 자동 dedup by source_uri — content hash 기반 dedup 만 (incremental ingest). URI lookup 별 task.
- Storage quota / TTL — agent 무한 ingest 시 KB 비대 우려. monitor + 별 task.
- frontmatter merge (stdin 이 이미 frontmatter 보유 시 머지) — v1 은 error.
- MCP `ingest_file` 의 multi-file batch 입력 — v1 single path. 여러 file 호출은 agent 가 N 회.

**Amends**:
- design §6 (`_external/` subdirectory subsection 추가, §6.7 위치).
- spec `tasks/p9/p9-fb-31-single-file-stdin-ingest.md` (status `open` → `completed`).
- spec stub 의 §6.3 명시 → 실제 §6.7 (기존 §6 구조 우선).

## 2026-05-07 — p9-fb-30 (post-dogfooding): MCP server (stdio) — agent integration MVP

**Source feedback**: 사용자 도그푸딩 2026-05-06 — Claude Code 같은 AI agent 가 kebab CLI 를 사용하는 것이 궁극 목표. 현재 surface 는 Claude Code 전용 skill (subprocess wrapper) 만 — host 무관 표준 통신 없음. fb-29 HTTP daemon 은 single-user local-first 환경 대비 비대로 deferred (2026-05-07), fb-30 stdio MCP 가 동일 사용자 가치 (agent integration + session 동안 hot cache) 를 daemon 복잡도 없이 제공.

**Live binding 변경**:

- 신규 subcommand `kebab mcp` — stdio JSON-RPC server, `--config <path>` honor.
- 신규 crate `kebab-mcp` (lib only) — `serve_stdio(Config, Option<PathBuf>)` entry. UI crate 카테고리 (kebab-cli + kebab-tui + kebab-mcp 가 facade 룰 동일 적용 — `kebab-app` facade 만 import).
- Tool surface v1 (read-only 4): `search` (lexical/vector/hybrid 검색, default Hybrid), `ask` (RAG 답변, default mode Hybrid, optional `session_id` for multi-turn + optional `mode` override), `schema` (introspection), `doctor` (health check). `ingest_*` / `fetch` / `list_docs` / `inspect_chunk` 는 fb-31 / fb-35 / 후속 task 머지 시 추가.
- Resources / Prompts / Sampling — 모두 미선언 (tools-only v1).
- Output: 모든 tool 이 wire schema v1 JSON 을 MCP `text` content block 으로 직렬화. CLI `--json` 모드와 동일 wire — single source.
- Error mapping: tool dispatch `Err(e)` 만 `isError: true` + error.v1 content. Refusal (`grounded: false`) / no-hit (empty array) / unhealthy (`ok: false`) 는 모두 정상 응답 — agent 가 wire payload semantic flag 으로 분기.
- `kebab-app::error_wire` 신규 — fb-27 의 `kebab-cli::error_classify` 코드 그대로 promotion (struct + classify + classify_llm + 7 unit test). kebab-cli + kebab-mcp 둘 다 동일 모듈 사용. reqwest dev-dep 도 함께 이동. 부수 변경: `ErrorV1` 에 `schema_version: String` 필드 추가 — kebab-mcp 의 직접 serialize 경로에서도 wire 정합 (kebab-cli 의 `wire_error_v1` 의 `tag_object` 는 idempotent 로 작동, 동작 무영향).
- `kebab-app::Capabilities::mcp_server`: `false` → `true`. `schema_report` 통합 테스트 + `cli_schema` 통합 테스트 assertion 갱신.
- Initialize handshake: `protocolVersion = "2025-03-26"` (rmcp 1.6 default), `capabilities.tools = { listChanged: false }`, `serverInfo = { name: "kebab", version: <CARGO_PKG_VERSION> }`.
- `KebabAppState` 가 `(Config, Option<PathBuf>)` carry — `kebab_app::doctor_with_config_path` 는 `Option<&Path>` 만 받기 때문 (`doctor_with_config(&Config)` 미존재). path 없으면 `None` (XDG default 동작).
- `tokio::task::spawn_blocking` wrap on `call_tool` arms for `ask` + `search` — `OllamaLanguageModel` 의 `reqwest::blocking::Client::build()` 가 내부적으로 tokio runtime create+drop 하므로 async 안에서 panic. spawn_blocking 으로 우회. schema / doctor 는 cheap reads 라 wrap 불필요.
- `tools/list` 의 list construction 을 `pub fn build_tools_vec()` 로 추출 — rmcp 1.6 가 in-memory test transport 미노출이라 spawn 없이 unit-level 검증 위함.

**Spec contract impact**: design §10 에 §10.2 MCP transport 절 추가.

**Tests added**: kebab-mcp integration (5: tools_call_search / tools_call_ask / tools_call_schema / tools_call_doctor / tools_list / error_mapping + initialize), kebab-cli integration (1: cli_mcp_smoke spawn + initialize + tools/list round-trip). 약 8 신규 테스트.

**Known limitation (deferred)**:

- HTTP-SSE transport — fb-29 P+ deferral 따라 stdio 단일. browser agent / remote 시나리오 등장 시 재개.
- Resources (`kebab://chunk/<id>` URI) — fb-35 verbatim fetch 와 함께 v2.
- Prompts — RAG 자체 prompt template 내장으로 사용자 가치 약함, defer.
- Streaming `ask` — fb-33 streaming ask 와 함께.
- `ingest_*` / `fetch` / `list_docs` / `inspect_chunk` tools — 후속 task 별로 추가.
- Server-scope state caching — 현재 매 tool call 마다 store open. 첫 call 시 `KebabAppState` 에 `OnceLock<SqliteStore>` 도입 검토 (post-merge 후속 PR).
- rmcp SDK API 호환성 — 1.6 채택, 미래 major bump 시 별 task.
- Manual `tools/list` + `tools/call` dispatch 채택 — rmcp 1.6 의 `#[tool_router]` 매크로보다 명시적, 디버깅 쉬움. 하지만 새 tool 추가 시 두 곳 (list_tools 의 vec + call_tool 의 match) 동시 갱신 필요. 후속 task 가 5개 이상 tool 추가하면 매크로 도입 재검토.
- `AskOpts` 가 `Default` 미도입 — kebab-cli + kebab-tui + kebab-mcp 의 모든 호출 site 가 9 field 를 명시적으로 초기화. 새 field 추가 시 모든 site 동시 갱신 필요. `impl Default for AskOpts` 또는 builder 패턴 도입은 별 PR.

**Amends**:
- design §10 (MCP transport subsection 추가).
- spec `tasks/p9/p9-fb-30-mcp-server.md` (status `open` → `completed`).
- spec stub 의 `transport: stdio default + http (fb-29 daemon) 위에 SSE 옵션` → 실제 채택 stdio 단일 (fb-29 deferral 결과, 2026-05-07 commit `2e8de14` 의 spec 갱신과 일관).

## 2026-05-07 — p9-fb-27 (post-dogfooding): introspection (`kebab schema`) + structured error wire

**Source feedback**: 사용자 도그푸딩 2026-05-06 — agent 가 kebab 인스턴스의 wire 버전 / 기능 / 모델 / 인덱스 통계 introspect 못 함; error 가 stderr text 라 substring 분기 필요.

**Live binding 변경**:

- 신규 명령 `kebab schema [--json]` — text / `schema.v1` JSON. `--config <path>` honor.
- 신규 wire `schema.v1` — `kebab_version` (`env!("CARGO_PKG_VERSION")`) / `wire.schemas` / `capabilities` (10 bool, 4 미래 surface 포함) / `models` (parser/chunker/embedding/prompt_template/index/corpus_revision 6축) / `stats` (doc/chunk/asset count + last_ingest_at). `SchemaV1` 가 자체 `schema_version: "schema.v1"` 필드 carry — `wire_doctor` 와 동일 idempotent re-tag pattern.
- 신규 wire `error.v1` — `--json` 모드에서 fatal error 가 stderr ndjson 으로 emit. 비 `--json` 은 기존 stderr text 유지.
- error code 7개 initial set: `config_invalid` (`ConfigInvalid` signal in kebab-config, `cause` prefix `read_failed:` / `parse_failed:` underscore-slugged for stable agent matching) / `not_indexed` (`NotIndexed` in kebab-store-sqlite, `SqliteStore::open_existing` API 신규 — `OpenFlags::SQLITE_OPEN_READ_WRITE | SQLITE_OPEN_URI` 로 silent CREATE 방지) / `model_unreachable` (`LlmError::Unreachable`) / `model_not_pulled` (`LlmError::ModelNotPulled`) / `timeout` (`LlmError::Timeout`) / `io_error` (`std::io::Error` chain detection) / `generic` (catch-all, verbose 시 `details.chain` 채움).
- exit code 0/1/2/3 unchanged — `RefusalSignal` / `NoHitSignal` / `DoctorUnhealthy` 만 보고 1/1/3 결정. 신규 5 typed signal 모두 fall-through → 2.
- `kebab-app::error_signal` 모듈 신규 — `doctor_signal` 의 3 signal 과 신규 typed error 들 한 곳에서 re-export.
- `kebab-store-sqlite::SqliteStore::count_summary` 메서드 신규 — `schema.v1.stats` block backing.
- `kebab_parse_md::PARSER_VERSION` + `kebab_store_vector::INDEX_VERSION_STR` `pub const` 노출 — kebab-app 의 `Models` block 이 single source of truth (cascade 규약 충족).

**Spec contract impact**: design §10 에 §10.1 capability matrix subsection 추가 — `schema.v1` / `error.v1` wire 명시.

**Tests added**: kebab-config fb27_tests (2: ConfigInvalid downcast / malformed TOML), kebab-store-sqlite (3: NotIndexed signal + open_existing no-create regression + count_summary zero state), kebab-cli error_classify::tests (7: 7 code 분류 + verbose chain), kebab-cli wire::tests (2: schema.v1 / error.v1 round-trip), kebab-app schema_report integration (2: ingested KB stats + empty KB), kebab-cli cli_schema integration (2: --json + text), kebab-cli cli_error_wire integration (2: --json error.v1 + legacy text). 약 20 신규 테스트.

**Known limitation (deferred — interim wire shape)**:

- `error.v1.details` shape per code 가 frozen design literal 과 일부 일탈 — 신규 typed signal 도입 deferred 라 발생:
  - `io_error.details` = `{ "kind": "<ErrorKind debug string>" }` (spec literal 의 `{ path, op }` 아님 — `IoFailure` typed signal 추가 시 정정).
  - `timeout.details` = `{ "source": "<error display>" }` (spec literal 의 `{ operation, elapsed_ms, deadline_ms }` 아님 — `OpTimeout` typed signal + per-callsite stamping 추가 시 정정).
  - `model_unreachable.details` = `{ endpoint, source }` (spec literal 의 `{ endpoint, operation }` — `LlmError::Unreachable` 가 `operation` field 없음).
  - `model_not_pulled.details` = `{ model }` (spec literal 의 `{ model, endpoint, operation }` — `LlmError::ModelNotPulled` 가 model id 만 carry).
  - JSON Schema literal `docs/wire-schema/v1/error.schema.json` 의 `details` block 은 `additionalProperties: true` + `required: []` 로 permissive — 실제 emit shape 반영. 후속 task 가 typed signal 추가 시 schema 의 description 갱신.
- `Config::load(Some(/nonexistent))` 가 silent default fallback — agent 가 `--config /wrong` 으로 호출 시 `config_invalid` 가 아닌 default config 적용 + 후속 명령이 default 동작. fb-28 (`--readonly`/`--quiet`) 또는 별 follow-up 에서 `--config` strict mode 도입 검토 필요.
- `Config::from_file` 의 schema-mismatch (DB 마이그레이션 버전 안 맞음) 는 `NotIndexed.found = None` 으로만 보고 — `_refinery_schema_history` 의 max version 을 read 하는 후속 PR 에서 `found: Some("V005")` 같은 정확한 값 채움.
- `LlmError::Stream` / `Malformed` 가 `code: "generic"` fallback — 후속 task 에서 `stream_aborted` / `malformed_response` 같은 dedicated code 도입 검토 (design §10.1 future-extensions 절 참조).
- `not_indexed.details` 가 `{ expected, found }` 만 emit (spec literal 의 `{ data_dir, expected, found }` 아님 — `expected` 가 full DB path 라 data_dir 은 caller 에서 derive 해야 함, NotIndexed signal 자체는 path 한 개만 carry).
- README 의 wire schema 목록과 CLAUDE.md 의 wire schema 목록이 fb-27 머지 시점에 약간 일치 안 함 (CLAUDE.md 가 `eval_run.v1`/`eval_compare.v1`/`list_docs.v1` 포함, 실제 docs/wire-schema/v1/ 에 해당 파일 없음). 별 follow-up 에서 doc / 실제 wire 동기화 sweep 진행.
- `SqliteStore::open_existing` 가 `SQLITE_OPEN_READ_WRITE` 로 열고 doc 으로만 "callers should not issue mutations" 명시 — 컴파일러 enforcement 없음. 후속 PR 에서 `apply_pragmas` 의 WAL 라인을 분리한 `apply_read_pragmas` + `SQLITE_OPEN_READ_ONLY` 변형 도입 검토 (WAL mode 는 DB 헤더에 영속이라 RO 도 동작 가능).

**Amends**:
- design §10 (capability matrix subsection 추가).
- spec `tasks/p9/p9-fb-27-introspection-and-error-wire.md` (status `open` → `completed`).
- spec stub 의 `Goal (skeleton)` 의 6 exit code (`0/1/2/3/4/5`) 제안 → 실제 채택 0/1/2/3 only.

## 2026-05-05 — p9-fb-25 (post-dogfooding): config workspace.include 제거 + 지원 형식 가시성

**Source feedback**: 사용자 도그푸딩 2026-05-05 — config 의 `workspace.include` + `workspace.exclude` 동시 존재가 case 4 (둘 다 매치 안 함) 의미 모호 + 어차피 처리 가능 형식 (md / png / jpg / pdf) 이 정해져 있으니 사용자에게 명시 필요.

**Live binding 변경**:

- `kebab-config::WorkspaceCfg.include: Vec<String>` 제거. denylist-only 모델. 옛 config 의 `include = [...]` 은 serde 가 silently 무시 + `Config::from_file` 가 단발 `tracing::warn!` 으로 deprecation 안내 (`std::sync::OnceLock` — 같은 process 안에서 한 번만).
- `kebab-core::IngestItem.warnings` 가 Skipped 시 사유 채움: `"unsupported media type: .{ext}"` (ext 없으면 `"unsupported media type: <no-ext>"`) / `"kb:// URI not yet supported"`.
- `kebab-core::IngestReport.skipped_by_extension: BTreeMap<String, u32>` + `kebab-app::AggregateCounts.skipped_by_extension` 신규. key = lowercase ext (`docx`, `txt`), no-ext sentinel = `<no-ext>`. wire schema `ingest_report.v1` 에 additive 추가 (v1 호환 유지 — release 트리거 안 됨 per CLAUDE.md release 규약).
- CLI summary + TUI status_line final / aborted: `5 skipped: 3 docx, 1 txt, 1 epub` 형식. desc 정렬 (count) + ties by key alphabetic + 모두 표시.
- `kebab-app::init_workspace` 헤더 주석에 지원 형식 명시 (Markdown / 이미지 / PDF + 각 확장자).
- README `kebab ingest` 설명에 지원 형식 + skip 사유 + breakdown 표시 명시.

**Spec contract impact**: design §6.2 의 `workspace.include` 항목 invalidate (frozen 그대로 두고 본 항목 + spec `tasks/p9/p9-fb-25-config-include-removal.md` 가 source of truth). design §3.x `IngestReport` + §2.4a `IngestEvent` 에 새 필드 / 새 warning 의미 추가 (additive).

**Tests added**: 5 신규 (kebab-config 단위 2: legacy include 무시 + WorkspaceCfg 필드 destructure / kebab-app 통합 1: skip_reason / kebab-app 통합 1: init_template 헤더 / kebab-tui 단위 2: status_line breakdown 완료/abort) + 1 unit (kebab-app 의 render_skipped_breakdown). 기존 fixture 6 개 mechanical adapter 수정 (`tests/common/mod.rs` SourceScope, `tests/image_pipeline.rs` × 2 + `tests/pdf_pipeline.rs` 의 dead `include.push` 제거, `tests/ingest_report_snapshot.rs` + `kebab-cli/src/wire.rs` literal 에 `BTreeMap::new()` 추가, snapshot JSON 의 `skipped_by_extension` 필드). assertion 의미 변경 없음.

**Known limitation (deferred)**:

- `SourceScope.include` (`kebab-core::traits`) 는 그대로 — design §7.1 abstraction 이라 별 spec 으로 다룰 수 있음. 본 PR 은 config 단의 `WorkspaceCfg.include` 만 정리.
- 새 extractor (txt / docx / epub 등) 도입은 별 spec.
- `kebab doctor` 가 unsupported 파일 카운트 분석은 후속 task.

## 2026-05-04 — p9-fb-23 (post-dogfooding): Incremental ingest

**Source feedback**: 사용자 도그푸딩 2026-05-04 — "새 문서들이 폴더에 추가되면 ingest 시 변하지 않은 문서는 다시 ingest 하지 않고 변하거나 새로 추가된 문서만 처리하고 싶어."

**Live binding 변경**:

- SQLite V006 migration — `documents` 에 `last_chunker_version` + `last_embedding_version` TEXT (nullable) 추가. 기존 row 는 NULL → 첫 번째 ingest 시 항상 mismatch → 강제 재처리 (안전 default).
- `kebab-core::IngestItemKind::Unchanged` variant 신규 (기존 `Skipped` 와 의미 분리: `Skipped` = media-type 필터, `Unchanged` = 모든 versions match).
- `IngestReport.unchanged: u32` + `AggregateCounts.unchanged: u32` 신규. wire schema `ingest_report.v1` 에 `unchanged` 필드 additive (v1 호환 유지).
- `kebab-app::IngestOpts { progress, cancel, force_reingest }` struct 신규 — `AskOpts` 패턴. 기존 `ingest_with_config_cancellable` 등 wrapper 보존, 신규 `ingest_with_config_opts` 가 IngestOpts 받음.
- `kebab-app::ingest_with_config_opts` asset 루프에 early-skip 블록: `force_reingest=false` + 4 조건 (asset_blake3 일치 + doc_id 존재 + last_chunker_version 일치 + last_embedding_version 일치) 모두 성립 시 `IngestEvent::AssetFinished{result: Unchanged}` emit + `aggregate.unchanged += 1` + `continue` (parse/chunk/embed/vector upsert 모두 회피). 세 flow (md / image / pdf) 모두 적용.
- 정상 path 끝에서 `CanonicalDocument.last_chunker_version` + `last_embedding_version` 을 현 active version 으로 stamp.
- `kebab-cli` 에 `--force-reingest` flag 추가 (skip 우회 강제 재처리).
- `kebab-tui::ingest_progress::status_line` final / aborted 라인 모두 `unchanged=N` 노출.

**Spec contract impact**: design §9 versioning cascade 의 명시적 동작 추가 — parser/chunker/embedder version bump 시 다음 ingest 가 자동으로 모든 doc 을 `updated` 로 처리. 기존엔 silently 새 version 으로 overwrite (idempotent UPSERT) 였으나 본 변경으로 explicit refresh + 비용 회피 모두 보장. design §3.x IngestReport / §2.4a IngestEvent 에 `Unchanged` variant 추가 (additive, wire v1 호환).

**Tests added**: 8 신규 (`crates/kebab-app/tests/incremental_ingest.rs` 2 + `crates/kebab-app/tests/ingest_lexical.rs` 2 + `crates/kebab-store-sqlite/tests/incremental_ingest.rs` 4) + 3 기존 갱신 (`image_pipeline.rs` / `pdf_pipeline.rs` / `ingest_lexical.rs::ingest_idempotent_on_second_run` 의 assertion 이 Updated → Unchanged 로 변경). 기존 ~720 워크스페이스 테스트 무수정 통과.

**Known limitation (deferred)**:

- Mtime-based pre-hash skip 미구현 — blake3 streaming 은 매 scan 마다 무조건 발생.
- Watch-mode (실시간 file change detection) 후속 task.
- Stale skip risk: 사용자가 외부에서 embedder 모델 swap 후 config 의 `models.embedding.id` 갱신 안 하면 last_embedding_version 매치 → silently skip. doctor 명령이 mismatch 감지 → 권고하는 후속 task 가능.

## 2026-05-04 — p9-fb-24 (post-dogfooding): TUI status bar + Library 헤더 + page scroll

**Source feedback**: 사용자 도그푸딩 2026-05-04 — (1) Library 컬럼이 무엇을 뜻하는지 헤더 부재, (2) Ask 트랜스크립트 / Inspect 둘 다 페이지 단위 스크롤 키 필요, (3) 모든 모드에서 항상 떠 있는 상태바 + 키 안내바 (버전 정보 포함) 가 있으면 좋겠다.

**Live binding 변경**:

- bottom 영역을 2 row 로 분할. 윗줄 = status bar (`kebab v<version> │ <pane> │ <docs> docs │ <state>`), 아랫줄 = key hint bar (기존 `footer_hints` 그대로). p9-fb-13 follow-up 의 single-row footer 와 충돌 — frozen spec 텍스트 보존, 본 항목이 live source of truth.
- ingest progress 의 dedicated row (p9-fb-03) 는 status bar 의 dynamic slot 으로 흡수. priority cascade: streaming → searching → indexing → idle. 시각적 위치 변경, 콘텐츠 동등.
- `Paragraph::line_count` 등 unstable feature 추가 없음.
- `crates/kebab-tui/src/pager.rs::PAGE_STEP = 10` 신규. Ask 의 PgUp/PgDn 추가 (mode 무관, `follow_tail = false` flip), Inspect 의 기존 +/-10 hardcode 가 같은 상수 참조로 일원화.
- `format_doc_header(area_width)` 신규 (kebab-tui/src/library.rs). Library 의 doc list 위에 1-row 헤더 (TITLE / TAGS / UPDATED / CHUNKS, display-width 정렬). Block 의 inner area 를 `Layout` 으로 header (Length 1) + list (Min 0) 로 분할.
- cheatsheet popup Ask section 에 `PgUp / PgDn` row 추가 (Inspect 는 이미 명시).

**Spec contract impact**: p9-fb-13 follow-up (footer 단행 row) + p9-fb-03 (ingest dedicated row) frozen spec 들과 layout 충돌. frozen 텍스트 보존, 본 HOTFIXES 항목 + spec `tasks/p9/p9-fb-24-tui-affordances.md` + design `docs/superpowers/specs/2026-05-04-p9-fb-24-tui-affordances-design.md` 가 live source of truth.

**Tests added**: 약 21 신규 (status_bar 통합 10 + library 헤더 1 + Ask PgUp/PgDn 3 + Inspect PgUp/PgDn 회귀 2 + format_doc_header 단위 1, 잔여는 cascade branch 별). 기존 695개 워크스페이스 테스트 무수정 통과 (`cargo test --workspace -j 1` 기준 716 passed).

**Known limitation (deferred)**: `PAGE_STEP = 10` 은 viewport-aware 가 아님 — 24 row 작은 터미널에서 한 페이지 > viewport, 80 row 큰 터미널에서 한 페이지 < viewport. 후속 task 에서 viewport-aware 로 업그레이드 가능.

## 2026-05-04 — p9-fb-22 (post-dogfooding): mid-string cursor editing + Ask follow-tail auto-scroll

**Issues**: Gitea #94 (커서 이슈) — 텍스트 입력 후 커서 이동 불가. Gitea #95 (새 응답 이슈) — 새 응답이 viewport 아래로 추가돼도 자동으로 스크롤이 따라가지 않음. 두 건 모두 사용자 도그푸딩 중 발견.

**Root cause**:

- p9-fb-10 의 `InputBuffer` 가 의도적으로 append-only (cursor invariant: `cursor_col == display_width(content)`). 화살표 / Home / End / Delete 가 어떤 pane 에서도 wired 되어 있지 않아 입력한 텍스트의 중간을 편집할 수 없었다.
- p9-3 의 Ask 트랜스크립트는 `Paragraph::scroll((s.scroll, 0))` 의 offset 을 위에서부터 카운트한다. 새 답변 도착 시 `s.scroll = 0` 으로 리셋하면 viewport 가 *위쪽* 에 고정되어, 트랜스크립트가 길어지면 새 응답이 시야 밖으로 밀려 사용자가 직접 `j` 로 스크롤해야 했다.

**Live binding 변경**:

- `InputBuffer` cursor 모델을 byte position 기반으로 재구성. `cursor_col` 은 prefix slice 의 `unicode-width` 합으로 derive. 새 메서드: `move_left / move_right / move_home / move_end / delete_after`. `push_char` / `pop_char` 는 cursor 위치에서 동작하도록 의미 변경 (cursor 가 끝에 있을 때 기존 append 동작과 동일 — 호환).
- Ask / Search / Library filter overlay 세 곳에 `←` / `→` / `Home` / `End` / `Delete` key handler 추가. Search 는 cursor 이동만으로는 input_dirty_at 을 바꾸지 않고, `Delete` 로 실제로 char 가 사라질 때만 debounce 타이머를 reset (커서 이동 ≠ 쿼리 변경).
- `AskState` 에 `follow_tail: bool` 필드 추가 (default `true`). `render_answer` 가 `follow_tail` 인 동안 매 프레임마다 `Paragraph::line_count(width)` 로 wrapped row 수를 재계산해 스크롤을 `line_count - inner_height` 로 pin. 사용자가 `j` / `k` 누르면 `follow_tail = false` 로 freeze, `Shift-G` 로 다시 활성화. 새 submission 과 `Ctrl-L` 도 follow-tail 을 재활성화.
- `kebab-tui` 의 `ratatui` dep 에 `unstable-rendered-line-info` feature 활성화 — `Paragraph::line_count` 가 ratatui 0.28 에서 unstable. ratatui 버전 bump 시 본 feature 의 안정 여부 재확인 필요 (현재는 0.28.1 에 pin).
- cheatsheet popup 의 Search / Ask section 에 화살표 + Home/End + Delete row 추가, Ask section 에 `Shift-G` row 추가.

**Spec contract impact**: p9-fb-10 frozen spec 의 "v1 is append-only; mid-string editing... is out of scope" 문구와 충돌. p9-fb-10 의 frozen 텍스트는 그대로 두고 본 HOTFIXES 항목이 InputBuffer 의 live cursor 모델 source of truth. p9-3 frozen spec 에는 follow-tail 동작이 명시되지 않았음 — 본 항목이 추가 동작 기록.

**Tests added**: 11 신규 InputBuffer unit (move_left/right ASCII/Hangul, home/end, mid-string insert, backspace at cursor + at home no-op, delete_after at cursor + at end no-op, mixed-width cursor invariant, take 후 cursor reset), 10 신규 Ask integration (left/right/home/end/Delete on Ask input, Hangul left arrow, follow_tail default, k disengages, Shift-G re-engages, Ctrl-L resets, follow-tail rendering bottom of long transcript). 기존 39 개 InputBuffer + Ask 테스트 (input.rs unit 18 + tests/ask.rs 21) 는 backwards-compat 으로 그대로 통과 (cursor 가 끝에 있을 때 push_char/pop_char 의미 동일).

**Known limitation (deferred)**: cheatsheet popup body 가 Search +3 row, Ask +4 row 로 늘어나 75% height 한계가 더 빡빡해짐. p9-fb-21 의 deferred 한계와 같은 후속 task (popup scroll 또는 multi-column layout) 가 점점 더 필요함.

## 2026-05-03 — p9-fb-21 (post-dogfooding): `i` universal Insert toggle + Search `i`→`o` rebind + F1 prefix

**Spec added**: `tasks/p9/p9-fb-21-tui-insert-key-discoverability.md` (status `completed` 직접). 이전 도그푸딩 사이클 (p9-fb-01..20) 닫은 후 사용자가 다시 TUI 돌려보며 발견:

- Ask Insert→Esc→Normal 후 Insert 로 돌아가는 키 모름 (p9-fb-12 의 mode_intercept 가 Search/Ask 의 `i` 를 fall-through 시킴 — 자동 INSERT 가정).
- 전반적 키바인딩 안내 부족 (F1 cheatsheet 가 invisible).

**Live binding 변경**:

- `mode_intercept` 의 `(Char('i'), Mode::Normal, _)` arm 이 pane 무관 모두 INSERT flip + intercept consume. 사용자가 어느 pane 에서든 Esc 후 `i` 로 즉시 복귀 가능.
- Search 의 chunk inspect 키 `i` → `o` (vim "open") rebind. `i` 가 universal Insert toggle 로 자유로워졌기 때문. Inspect 진입 명령은 `o` (대상 hit 의 chunk 를 Inspect pane 에서 "open").
- 모든 `footer_hints` 항목 (10 개 (pane, mode, filter) 조합) 첫 fragment = `F1 도움말`. F1 cheatsheet binding 의 discoverability 보장.
- Search/Ask Normal hint 에 `i 입력모드` fragment 추가 — Insert 복귀 경로 명시.
- cheatsheet popup 의 Global / Search / Ask section 갱신: Global `i` = "every pane", Search 에 `o` row + `i` row 분리, Ask 에 `i` row 추가.

**Spec contract impact**: Search 의 `i` → `o` rebind 은 frozen spec p9-fb-12 의 "Search 의 `j/k/i/g`" 표현과 충돌. p9-fb-12 의 frozen 텍스트는 그대로 두고 본 HOTFIXES 항목이 live binding 의 source of truth. p9-fb-13 footer hint 갱신 + p9-fb-21 의 footer hint 갱신은 동일 fn 에 누적.

**Tests added**: 6 신규 unit (mode intercept Normal/Insert × Search/Ask, Search `o` 명령 3 case, footer F1 prefix exhaustive, Search/Ask Normal `i 입력모드` 명시). 기존 footer hint 테스트 3 건 갱신 (F1 prefix 반영).

**Known limitation (deferred)**: cheatsheet popup body 가 Search + Ask 가 각 +1 row 늘어나면서 Inspect section (마지막) 이 75% height 안에 안 들어갈 수 있음 (TestBackend 120×40 환경 기준). 사용자는 Library/Inspect pane 에서 F1 누르면 Inspect 절 정보 일부 보임. 후속 task: popup scroll 또는 multi-column layout. 현재 스킵 — 도그푸딩 직접 신호 받은 후 우선순위 결정.

## 2026-05-03 — p9-fb-10 partial: helpers shipped, InputBuffer struct deferred

**Spec amended**: `tasks/p9/p9-fb-10-tui-cjk-input.md` (status flipped
planned → in_progress).

**Live state**: 본 PR 은 `kebab-tui::input::{display_width,
truncate_to_display_width}` helper 모듈 + Korean / Japanese fixture
render audit + 9 unit tests + library.rs 의 중복 truncate 제거 (단일
source) 만 머지. spec 의 `InputBuffer` struct (cursor 가 column 단위
wide-char width 를 추적) 도입은 follow-up.

**Why split**: Ask / Search / Editor pane 의 String + cursor 를
일괄 마이그레이션하면 회귀 표면이 커서 위 helper 만 먼저 머지. 백스페이스
경로는 모든 pane 이 이미 `String::pop()` 사용 — pop 은 `Option<char>`
반환 + UTF-8 sequence mid-byte split 안 함 (Rust std 가 char-aware).
즉 byte-boundary 안전성은 helper 없이도 이미 확보된 상태였고, 본 PR 의
helper 는 **rendering width** 만 정정.

**IME composing**: crossterm 0.28 이 native IME composing surface 를
노출 안 함 — finalized jamo / composed glyph 가 `KeyCode::Char(c)`
로만 도달. macOS / Windows / Linux (ibus/fcitx) 모두 동일. preedit
handling 은 out-of-scope (spec 도 "not in scope" 로 명시).

**Follow-up shipped 2026-05-03 in PR #88 — InputBuffer struct + Search/Ask/FilterEdit pane migrations + display-column-aware cursor placement + Korean FTS5 smoke pin. spec status flipped `in_progress` → `completed`.**

**후속 PR 체크리스트** (별 PR 에서 cover, 본 HOTFIXES 항목이 owner —
새 spec 파일을 만들지 않고 기존 `tasks/p9/p9-fb-10-tui-cjk-input.md`
의 status `in_progress` 가 유지되는 동안 본 체크리스트를 참조):

- [x] `kebab-tui::input::InputBuffer { content: String, cursor_col: usize }` struct
- [x] Ask / Search / Editor pane 의 String + cursor 를 InputBuffer 로 교체
- [x] cursor render 가 wide-char 위에서 column 단위로 정렬 (현재 char-count 기반)
- [x] 한글 query → SQLite FTS5 검색 fixture 추가 (이미 NFC 정규화 됨, 단순 smoke pin)
- [x] DoD 체크박스 3 개 모두 채우고 spec status `in_progress` → `completed`

## 2026-05-03 — p9-fb-13 cheatsheet: `?` → `F1` rebind

**Spec amended**: `tasks/p9/p9-fb-13-tui-cheatsheet.md` (frozen —
original contract uses `?` as the cheatsheet trigger).

**Why rebind**: Library 가 이미 `Char('?')` 를 quick-Ask binding 으로
사용 중 (`Pane::Library::handle_key_library` line ~305: `?` →
`SwitchPane(Pane::Ask)`). spec 의 `?` 도입은 이 기존 binding 을 깨거나
mode-aware override 가 필요한데, 후자는 mode machine 의 추가 special
casing.

**Live binding**: `F1` (universal help key, no collision). modifier-
bearing 변종 (Ctrl-F1 등) 은 미발동. cheatsheet 가 visible 인 동안
`Esc` 도 닫기 (cheatsheet_intercept 가 mode_intercept 보다 먼저
처리).

**Per-pane hint line redesign**: 별도 spec 항목 (verb-form hint
재구성) 은 본 PR 에서 deferral. 기존 `render_footer` 의 pane-별
힌트 문자열이 동일 역할을 하므로 사용자 경험상 누락 없음. 후속 PR
가 mode-aware verb fragments 로 split 가능.

**Follow-up shipped 2026-05-03 — verb-form hint line redesign.** `pub fn footer_hints(focus: Pane, mode: Mode, filter_open: bool) -> &'static str` 신규 (run.rs). 한국어 동사구 (`"위로"` / `"아래로"` / `"필터"` / `"타이핑 검색어"` / `"Esc 로 NORMAL 모드"`) + mode-aware (NORMAL = navigation, INSERT = typing + Esc reminder) + filter overlay 분기. 8 unit tests pin (Library Normal/Insert/filter, Search Normal/Insert, Ask Normal/Insert, Inspect Normal/Insert + 모든 (pane, mode, filter) 조합 non-empty exhaustive). spec status `in_progress` → `completed`.

## 2026-05-03 — p9-fb-12 partial: mode machine without dispatch removal

**Spec amended**: `tasks/p9/p9-fb-12-tui-mode-machine.md` (status stays
`in_progress`, NOT `completed`). Original contract: introduce vim
NORMAL/INSERT modes globally AND remove `is_typing_mod` (search) +
input-empty heuristic (ask) so the per-pane key dispatch becomes
mode-authoritative.

**What shipped**: Mode enum + `App.mode` field + global `i`/`Esc`
interception in run loop + auto mode flip on pane switch
(`Mode::auto_for(pane)`) + status-bar mode label (color-graded via
`Role::Success` for Insert, `Role::Heading` for Normal). Status bar
literals (`-- NORMAL --` / `-- INSERT --`) pinned.

**Deferred to follow-up PR**: removal of the existing input-empty
heuristics in `search::handle_key_search` and `ask::handle_key_ask`.
These continue to gate j/k vs typing based on input buffer state.
Tests rely on those heuristics, so the removal warrants its own
focused PR (separate review, separate test sweep).

**Why partial-ship**: the user-visible signal (mode label + auto
flip + i/Esc) is the most load-bearing part of the spec; the
heuristic removal is cleanup that doesn't change behavior anyone
currently observes. Splitting keeps the PR review surface small.

## 2026-05-03 — p9-fb-17 migration number V004 → V005

**Spec amended**: `tasks/p9/p9-fb-17-chat-session-storage.md` (frozen —
original contract calls the migration `V004__chat_sessions.sql`).

**Why renamed**: `V004__kv.sql` was already taken by p9-fb-19's `kv`
table for the `corpus_revision` counter (merged earlier the same day,
PR #78). Refinery numbers must be globally unique + monotonically
increasing, so chat-session storage shifts to `V005__chat_sessions.sql`.

**Behavior unchanged**: identical schema to the spec (chat_sessions +
chat_turns + idx_chat_turns_session); only the file name moved.

## 2026-05-03 — p9-fb-19 spec `index_version` → impl `corpus_revision` rename

**Spec amended**: `tasks/p9/p9-fb-19-search-cache.md` (frozen — original
contract uses `index_version` for the monotonic counter that ingest
bumps and `App::search` snapshots into its cache key).

**Why renamed**: design §9 already has an `index_version` identifier
(`IndexVersion` newtype, used in the §4.2 `index_id` recipe and on
`SearchHit`) — a *string label* for embedding-index identity. Reusing
the name for the monotonic u64 counter would collide silently on every
grep / type-search.

**Live name**: `corpus_revision` (added as a new row in design §9
versioning table). `SqliteStore::corpus_revision()` /
`bump_corpus_revision()` methods + `kv['corpus_revision']` row.
`SearchCacheKey.corpus_revision` field on `App`.

**Behavior unchanged**: every other detail (monotonic, ingest-commit
bump, in-key snapshot, no-bump on no-op reingest) matches the spec.

## 2026-05-02 — Config defaults: LLM = gemma4:e4b + workspace.root tilde expansion

**Discovered**: 사용자가 도그푸딩 환경에 `kebab init` 으로 생성된 `~/.config/kebab/config.toml` 검토하던 중.

**Symptom 1 (default 변경)**: `Config::defaults().models.llm.model` 가 `qwen2.5:14b-instruct`. OCR (P6-2) / caption (P6-3) 어댑터는 이미 `gemma4:e4b` 기본 사용 — 사용자가 OCR / caption / ask 모두 쓰려면 두 family 모델 (`qwen2.5` + `gemma4`) 을 모두 pull 해야 했음. 사용자 결정 (2026-05-02): **텍스트 LLM 기본도 gemma4 계열로 통일**.

**Symptom 2 (load-bearing)**: `workspace.root = "~/KnowledgeBase"` 같은 `~` 시작 경로가 코드 path 별로 다르게 처리:
- ✅ `kebab-source-fs::connector` 가 `expand_tilde` 사용 → walk 정상.
- ❌ `kebab-app::ingest_one_image_asset` 이 `PathBuf::from(&workspace.root)` 직접 → `~` 미확장 → ExtractContext 에 `~/KnowledgeBase` 그대로.
- ❌ `kebab-app::ingest_one_pdf_asset` 동일.
- ❌ `kebab-tui::search::handle_key_search` editor jump 도 동일 → `vim +12 ~/KnowledgeBase/foo.md` 의미 없는 경로 spawn.

**Fix**:
- `Config::defaults().models.llm.model` → `"gemma4:e4b"`. 코멘트가 OCR / caption family 통일 명시.
- kebab-app 의 image / pdf 분기 두 곳 모두 `expand_tilde(&app.config.workspace.root)` 호출 (markdown path 가 이미 쓰는 self-contained helper).
- kebab-tui::search jump 호출 site 가 `kebab_config::expand_path(&state.config.workspace.root, "")` 사용 — `expand_path` 가 `~` / `${XDG_DATA_HOME}` / `{data_dir}` 모두 처리하는 정식 helper.
- README / docs/SMOKE.md / docs/ARCHITECTURE.md 의 LLM 모델 예시 모두 `qwen2.5` → `gemma4` 갱신 (sync rule).

**Caveat (남은 inconsistency)**: kebab-app 자체 helper `expand_tilde` 와 kebab-config `expand_path` 가 별도 정의. 후자가 superset (env var + `{data_dir}` templating 추가). 통합은 P+ task — 본 PR scope 밖.

**Amends**:
- `Config::defaults` 의 `qwen2.5:14b-instruct` → `gemma4:e4b`.
- README 사전 요구 절 / docs/ARCHITECTURE 핵심 결정 표 / docs/SMOKE 의 ollama pull 예시 갱신.

## 2026-05-02 — P9-4 TUI Inspect: render_inspect generic + Search `i` entry + collapse simplification

**Discovered**: P9-4 implementation start.

**Symptom 1 (cosmetic)**: Same shape as P9-1/2/3 — `tasks/p9/p9-4-tui-inspect.md` § Public surface declares `render_inspect<B: ratatui::backend::Backend>(...)`. ratatui 0.28's `Frame` is backend-agnostic; the generic is unused.

**Symptom 2 (load-bearing)**: Spec § Behavior contract names `Search pressing 'i' (new key on Search pane) passes Chunk(selected_hit.chunk_id)` — but P9-2 (already merged) didn't include `i`. The Inspect entry from Search has to be wired retroactively.

**Symptom 3 (simplification)**: Spec § Behavior contract section on collapse: "focus is implicit by current scroll position; v1 may simplify by toggling all sections". Implementation takes the v1 path — `c` toggles all six sections (metadata / provenance / blocks / spans / text / embeddings) at once. Per-section focus is a P+ enhancement.

**Fix**:
- `render_inspect(f: &mut Frame, area: Rect, state: &App)` — no generic.
- New helper `kebab_tui::enter_inspect(state, target, return_to)` lifted out of pane handlers so both Library `Enter` and Search `i` use the same code path.
- Search pane gains `i` keybinding (pre-pass like `g`, plain modifier only — typing `i` in queries still reaches input). Esc returns the user to the originating pane stored in `return_to`.
- `InspectState.collapsed: HashSet<&'static str>` records collapsed section names. `c` flips all-collapsed ↔ all-expanded based on whether any are currently collapsed.
- `q` joins `Esc` as the back key (Inspect is the only read-only terminal pane in v1, so `q` is unambiguous).

**Trust note**: Embedding inspection is intentionally left as "(not loaded — out of v1 scope)" per spec § Out of scope. The full embedding-record fetch would require an extra facade method (`kebab-app::inspect_embedding`) that is not in the P5/P6/P7 facade surface. P+ task.

**Amends**:
- tasks/p9/p9-4-tui-inspect.md (`render_inspect` non-generic; collapse simplification; entry helper).
- tasks/p9/p9-2-tui-search.md (Search pane gains `i` for chunk inspect — was not in original p9-2 spec).

## 2026-05-02 — P9-3 TUI Ask: render_ask generic + command-vs-insert key disambiguation

**Discovered**: P9-3 implementation start.

**Symptom 1 (cosmetic)**: Same shape as P9-1 / P9-2 — `tasks/p9/p9-3-tui-ask.md` § Public surface declares `render_ask<B: ratatui::backend::Backend>(...)`. ratatui 0.28's `Frame` is backend-agnostic; the generic is unused and clippy `-D warnings` rejects it.

**Symptom 2 (load-bearing)**: Spec key bindings list `e` (toggle explain), `j` / `k` (scroll). All three collide with typing — a user asking "explain javascript" would have the leading `e` toggle explain mode, then `j` scroll, etc. The Library / Search panes don't hit this because their input is either filter-overlay-gated (Library) or the whole pane *is* an input (Search). Ask has both an always-visible input bar AND scrollable answer area.

**Fix**:
- `render_ask(f: &mut Frame, area: Rect, state: &App)` — no generic.
- `e` / `j` / `k` use the **input-empty heuristic**: when `state.ask.input.is_empty()`, they act as command keys (toggle explain / scroll up/down). When the input has content, they reach the input buffer as ordinary characters. Vim's "command vs insert mode" applied at the keystroke level — the user starts typing, the keys behave as text; clears the input (Backspace to empty), the keys behave as commands again.
- `Enter` always submits (when input non-empty AND not already streaming). `Esc` always returns to Library + clears `streaming/rx/thread` (best-effort cancel — worker keeps running but its result is dropped, per spec § Risks "fire and forget").

**Trust note**: The worker thread holds the `mpsc::Sender<String>`; the pane keeps `rx` and drains via `try_iter` once per render frame (no blocking). On Esc we `take()` the `JoinHandle` without `join` so quit is instant; the kernel reaps the orphan when its `ask_with_config` returns.

**Amends**:
- tasks/p9/p9-3-tui-ask.md (`render_ask` non-generic; `e`/`j`/`k` empty-input gating).

## 2026-05-02 — P9-2 TUI Search: render_search generic + jump_to_citation workspace_root

**Discovered**: P9-2 implementation start.

**Symptom 1 (cosmetic)**: Same shape as the P9-1 entry — `tasks/p9/p9-2-tui-search.md` § Public surface declares `render_search<B: ratatui::backend::Backend>(...)`. ratatui 0.28's `Frame` is backend-agnostic; the generic is unused and clippy `-D warnings` rejects it.

**Symptom 2 (load-bearing)**: Spec literal `jump_to_citation(citation: &Citation, editor_env: &str) -> Result<()>`. `Citation.path()` returns a `WorkspacePath` (workspace-relative), but the editor child needs an absolute path — `editor_env` does NOT carry the workspace root. The signature is unimplementable as written.

**Fix**:
- `render_search(f: &mut Frame, area: Rect, state: &App)` — no generic.
- `jump_to_citation(citation: &Citation, editor_env: &str, workspace_root: &Path) -> Result<()>` — added `workspace_root` arg. The run-loop call site reads `state.config.workspace.root`.
- `build_jump_command` extracted as a pure helper so unit tests can assert the `(program, args)` shape without spawning a child process. Lives next to `jump_to_citation` in `kebab-tui::search`.

**Trust note**: The `g` keybinding suspends the TUI (drops raw mode + LeaveAlternateScreen), runs the editor synchronously, then RAII-restores raw mode + AltScreen on return — even on panic in the child. Same shape as `kebab-tui::terminal::TuiTerminal::Drop` from P9-1.

**Amends**:
- tasks/p9/p9-2-tui-search.md (`render_search` non-generic; `jump_to_citation` adds `workspace_root`).

## 2026-05-02 — P9-1 TUI Library: render_library generic + test seam

**Discovered**: P9-1 implementation start.

**Symptom 1 (cosmetic)**: `tasks/p9/p9-1-tui-library.md` § Public surface declares `pub fn render_library<B: ratatui::backend::Backend>(f: &mut ratatui::Frame, area: Rect, state: &App)`. ratatui 0.28 dropped the backend generic from `Frame` (it's bound at `Terminal` initialisation, not at the render call site). The `<B: Backend>` parameter would be unused on the function and clippy `-D warnings` rejects unused generic parameters.

**Fix 1**: `render_library(f: &mut Frame, area: Rect, state: &App)` — no generic parameter. The function still works against any backend the `Terminal` was opened with (CrosstermBackend in production, TestBackend in snapshot tests). No call-site impact.

**Symptom 2 (test seam)**: `LibraryState.inner` is `pub(crate)` per the spec's parallel-safety contract — p9-2/3/4 must not mutate `LibraryState` directly. Snapshot tests in `tests/library.rs` (an integration test, NOT a unit test in the same module) cannot reach `pub(crate)` fields, so they cannot inject docs without going through `kebab-app::list_docs_with_config` (which would stand up a TempDir SQLite KB just to populate three rows).

**Fix 2**: new `App::populate_library_for_testing(&mut self, Vec<DocSummary>)` marked `#[doc(hidden)]`. Lets snapshot tests inject docs hermetically while keeping the parallel-safety boundary intact for normal callers (the helper is officially "test seam, not part of the UI API"). Same shape as `kebab-app::*_with_config` test seams from P3-5.

**Amends**:
- tasks/p9/p9-1-tui-library.md (`render_library` no longer generic; `populate_library_for_testing` test seam added).

## 2026-05-02 — P7-3 PDF ingest wiring: chunker_version deviation + storage UNIQUE bug

**Discovered**: P7-3 implementation start.

**Symptom 1 (deviation, intentional)**: `tasks/p7/p7-3-pdf-ingest-wiring.md` § Chunker selection notes that `config.chunking.chunker_version` is single-valued and serves the markdown path only. PDF ingest hard-codes `pdf-page-v1` regardless of the config value. A user who reads `config.toml` and sees `chunker_version = "md-heading-v1"` reasonably assumes PDFs use the same — they don't.

**Fix 1**: `ingest_one_pdf_asset` (in `kebab-app::lib.rs`) instantiates `PdfPageV1Chunker` directly. The `Chunk.chunker_version` field on emitted PDF chunks records `pdf-page-v1` truthfully. A future P+ task (chunker registry) either splits `Config::chunking.chunker_version` per medium or replaces the dispatch with a runtime registry. No HOTFIX entry needed once that happens — this entry is the cross-reference.

**Symptom 2 (storage-layer bug, fixed in same PR)**: P7-3's edited-bytes re-ingest test (`re_ingest_edited_pdf_produces_new_doc_id`) tripped on `sqlite error: UNIQUE constraint failed: assets.workspace_path: Error code 2067`. The assets table has a UNIQUE constraint on `workspace_path`, but `upsert_asset_row` (in `kebab-store-sqlite::store.rs`) only handles `ON CONFLICT(asset_id)`. When a file's bytes change, the new BLAKE3 produces a new `asset_id` while the `workspace_path` stays the same — INSERT picks the new asset_id branch, then trips the secondary UNIQUE on `workspace_path`.

**Why it didn't surface earlier**: No existing test (markdown / image) exercised edited-bytes re-ingest. The image path's `re_ingest_image_produces_updated_with_same_doc_id` uses identical bytes (same asset_id → `ON CONFLICT(asset_id)` catches it). Real-world editing of a tracked file would hit the same bug across all media types.

**Fix 2** (P7-3 implementation PR): new `purge_orphan_at_workspace_path` helper in `kebab-store-sqlite::store.rs`. Runs immediately before each `upsert_asset_row` call (both `put_asset_with_bytes` paths AND `DocumentStore::put_asset`). It:
1. SELECTs the stale row at `workspace_path` whose `asset_id` differs from the incoming one (none → no-op return).
2. DELETEs from `documents WHERE asset_id = stale` — `documents.asset_id ON DELETE RESTRICT` requires the documents go first; CASCADE on documents → `blocks` / `chunks` / `embedding_records` sweeps the dependent rows in the same statement.
3. DELETEs the stale `assets` row, freeing the `workspace_path` slot.
4. If the stale storage was `copied`, best-effort removes the byte file at `storage_path` so `data_dir/assets/` does not accumulate orphans across edits.

**Vector store cleanup (closed by follow-up PR)**: `embedding_records.chunk_id` CASCADE clears the SQLite side, but LanceDB lives in a separate store. The follow-up PR adds:
- `VectorStore::delete_by_chunk_ids` trait method (default impl no-op for older fakes).
- `LanceVectorStore::delete_by_chunk_ids` iterates every `chunk_embeddings_*` table in the connection and runs `Table::delete("chunk_id IN (...)")` in batches of 200.
- `SqliteStore::stale_chunk_ids_at(workspace_path, new_asset_id)` SELECT helper (read-only) that fetches the stale chunk_ids before they get cascade-deleted.
- `kebab-app::purge_vector_orphans_for_workspace_path` orchestrator. Each per-medium ingest helper (`ingest_one_asset` markdown branch, `ingest_one_image_asset`, `ingest_one_pdf_asset`) calls it immediately before `put_asset_with_bytes` so the stale Lance rows go away in lockstep with the SQLite cascade.

Verified end-to-end via the SMOKE runbook: edit a tracked PDF → re-ingest → vector search for the old body text returns the *new* chunks (semantic nearest-neighbour) and the old chunk_ids are not present in the vector store.

The previously-`#[ignore]`d `re_ingest_edited_pdf_produces_new_doc_id` integration test runs by default after this fix, plus a dedicated unit test `put_asset_with_bytes_sweeps_workspace_path_orphan` in `kebab-store-sqlite::tests::asset_writer` that exercises the no-documents flavour. Verified end-to-end via the SMOKE runbook: `kebab ingest` → edit a tracked PDF → `kebab ingest` reports `new=1` for that asset (rest `updated`) and the prior doc/chunks are gone from `inspect` / `list docs`.

**Amends**:
- tasks/p7/p7-3-pdf-ingest-wiring.md (chunker_version deviation; edited-bytes test runs).
- crates/kebab-store-sqlite (new `purge_orphan_at_workspace_path` helper called from both `put_asset_with_bytes` branches and `DocumentStore::put_asset`).
- crates/kebab-store-sqlite/tests/asset_writer.rs (`put_asset_with_bytes_sweeps_workspace_path_orphan` replaces the prior orphan-cleanup-on-failure test, since the failure path no longer exists).
- docs/SMOKE.md (note that edited-PDF re-ingest produces `new=1` rather than an error).

## 2026-05-02 — P7-2 pdf-page-v1: chunk_id collision + BYTES_PER_TOKEN

**Discovered**: P7-2 implementation start.

**Symptom 1 (load-bearing)**: `tasks/p7/p7-2-pdf-page-chunker.md` § Behavior contract literally says `chunk_id` per design §4.2 with `(doc_id, "pdf-page-v1", block_ids, policy_hash)`. But unlike `md-heading-v1` (which always emits at most one chunk per atomic block), `pdf-page-v1` splits one page-block into multiple chunks when page text exceeds the byte budget. All sub-chunks of the same page have identical `block_ids` → identical `chunk_id` collisions, breaking the §3.5 invariant that `chunk_id` is a primary key.

**Symptom 2 (cosmetic)**: Spec text says `token_estimate = byte_len / 4` and "matches `md-heading-v1` proxy". Looking at the actual md-heading-v1 source (`crates/kebab-chunk/src/md_heading_v1.rs:17`), the constant is `BYTES_PER_TOKEN = 3` (chosen to cover Korean ≈ 3 b/tok and over-estimate English ≈ 4 b/tok). Spec's "/4" claim is inconsistent with the implementation it claims to match.

**Root cause**: §4.2 chunk_id recipe was designed assuming one-chunk-per-block-set. Page-aware chunking violates that assumption.

**Fix** (PR #38, feat/p7-2-pdf-page-chunker):

- **Per-chunk policy_hash variant**: feed `format!("{base_policy_hash}#c{char_start}")` into `id_for_chunk`'s `policy_hash` slot so chunks within the same page get distinct `chunk_id`s. The §4.2 recipe itself stays unchanged — only the *input* to one of its slots differs per chunk. The unmodified `base_policy_hash` is still stored in `Chunk.policy_hash` so the field still answers "what policy was active" (workspace-wide policy invalidation lookups continue to work).
- **`BYTES_PER_TOKEN = 3`** (matches md-heading-v1 actual code, not spec literal). Cross-chunker policy fingerprint identity is verified by a unit test: `policy_hash_matches_md_heading_v1_for_identical_policy`.

**Trust note**: The per-chunk hash variant is opaque (`#c<n>` is just a marker, not interpretable as char_start by downstream tools — they read `Chunk.source_spans[0].char_start` for that). Downstream identifier comparisons on `chunk_id` continue to work as opaque blake3 hashes.

**Amends**:
- tasks/p7/p7-2-pdf-page-chunker.md (chunk_id recipe per-chunk variant; BYTES_PER_TOKEN = 3 not 4).

## 2026-05-02 — P6-3 caption: GenerateRequest.images + cargo feature dropped

**Discovered**: P6-3 implementation start.

**Symptom 1**: `tasks/p6/p6-3-caption-adapter.md` § Public surface declares `caption_image(llm: &dyn kebab_core::LanguageModel, ...)`, but the frozen `LanguageModel` trait + `GenerateRequest` from p4-1 carry no vision input. The spec's behavior contract ("the adapter is responsible for rendering the prompt to wire") implicitly relied on a trait extension that p4-1 never specced.

**Symptom 2**: Spec § Definition of Done asks for `cargo check -p kebab-parse-image --features caption` — i.e. a cargo feature gate. The captioning module's only extra deps are `base64` + `image` + the `kebab-llm` trait, all already pulled in by P6-2. A cargo feature would only complicate the build matrix without saving meaningful binary weight.

**Root cause**: Two small spec gaps that resolve cleanly together — extend the `LanguageModel` trait once for vision routing, and collapse compile-time + runtime gating into a single runtime gate.

**Fix** (PR #34, feat/p6-3-caption-adapter):
- `kebab-core::GenerateRequest` gains an `images: Vec<String>` field (`#[serde(default)]` for backward compat with pre-P6 wire payloads / snapshots). Empty for the text-only RAG path; populated with one or more base64 strings by vision-aware callers.
- `kebab-llm-local::OllamaLanguageModel` routes `req.images` onto the wire as `images: [base64, ...]` (Ollama's vision channel). The wire shape stays byte-identical for empty `images` because the field uses `#[serde(skip_serializing_if = "<[String]>::is_empty")]`.
- `kebab-parse-image::caption` module: `caption_image` / `apply_caption` build `GenerateRequest { images: vec![b64], temperature: 0.0, seed: 0, ... }` and accept any `&dyn LanguageModel`. Korean / English prompt branch picked from `lang_hint`.
- Cargo feature `caption` is **not** introduced — the runtime gate `config.image.caption.enabled = false` (default OFF) suffices.
- All existing `GenerateRequest { ... }` literals (kebab-rag, kebab-llm tests, kebab-llm-local tests) gained `images: Vec::new()` to satisfy the new field.

**Trust note**: Captions stay explicitly model-generated. `ModelCaption.model_version` carries `"<provider>/<prompt_template_version>"` (e.g. `"ollama/caption-v1"`) so a regression in either prompt or model is auditable from the wire.

**`model_version` shape deviation**: spec literal says `model_version: llm.model_ref().provider` (provider as a coarse version proxy). We extend to `<provider>/<prompt_template_version>` because prompt template churn is a real regression vector independent of the model — pinning both axes in one string lets `kebab-eval` (P5) detect either drift without a schema bump. Spec already left the door open ("if a vision model exposes a stable revision, prefer that"); the prompt template version is the closest stable revision we have today. Future PaddleOCR / Apple Vision adapters that expose a real model revision string can substitute it for `prompt_template_version` without breaking the wire shape.

**Amends**:
- tasks/p4/p4-1-llm-trait.md (`GenerateRequest` schema gained `images: Vec<String>`).
- tasks/p4/p4-2-ollama-adapter.md (request body now optionally includes `images: [...]`).
- tasks/p6/p6-3-caption-adapter.md ("Definition of Done" cargo feature `caption` dropped; runtime gate is the only feature gate).

## 2026-05-02 — P6-2 default OCR engine: Tesseract → Ollama-vision

**Discovered**: P6-2 implementation start.

**Symptom**: The original `tasks/p6/p6-2-ocr-adapter.md` spec lists Tesseract as the default OCR engine (`tesseract = "0.13"`, feature `tesseract`, default ON). Bringing Tesseract online requires installing `libtesseract-dev` (and `tesseract-ocr-kor` for the spec-default Korean languages set) on every dev / CI host. The kebab dev environment intentionally avoids system-package installs, so the Tesseract Rust bindings can't link.

**Root cause**: Spec was written assuming a Linux host with `apt install tesseract-ocr-*` available. The reality of single-developer local-first KB is that the same box also runs the Ollama vision endpoint already wired by P4-2 — using it for OCR adds zero new system dependencies.

**Fix** (PR #33, feat/p6-2-ocr-adapter):
- New `OllamaVisionOcr` adapter under `crates/kebab-parse-image/src/ocr.rs`. Implements the spec's `OcrEngine` trait by POSTing the image (base64) to `<endpoint>/api/generate` with a transcription prompt against `gemma4:e4b` (default) or any other vision-capable Ollama model.
- New `kebab-config::ImageCfg.ocr` block (`enabled`, `engine`, `model`, `endpoint`, `languages`, `max_pixels`). `enabled` defaults to `false` because OCR adds a model call per asset; `engine` defaults to `"ollama-vision"`. `endpoint` falls back to `models.llm.endpoint` when empty so the same Ollama host serves both LLM and OCR.
- The `OcrEngine` trait is unchanged from the spec — Tesseract / Apple Vision / PaddleOCR engines plug in as future feature-gated alternatives without touching the extractor or chunker. The trait abstraction is the part the spec actually demanded; only the choice of default implementation changes.
- Tests cover wiremock unit paths (200 happy / 5xx / 200 error envelope / empty response / downscale honours `max_pixels`), `apply_ocr` provenance + error handling, and an opt-in `KEBAB_OCR_INTEGRATION=1` integration test that hits a real Ollama endpoint with a generated `"Hello World 2026"` PNG. Tesseract feature-gated tests from the original spec are deferred to whenever someone is willing to bring `libtesseract` to CI.

**Trust note**: The original spec marked `OcrText` as "observed text (high trust)" to distinguish it from `ModelCaption`. With an LLM-driven default the line blurs — vision LMs can hallucinate. We kept `OcrText.engine = "ollama-vision"` so consumers can decide trust by engine identity. Future Tesseract / Apple Vision adapters write a different `engine` string and downstream code can branch.

**Amends**: tasks/p6/p6-2-ocr-adapter.md (default engine; "Allowed dependencies" list — `reqwest` + `base64` replace `tesseract`; "Apple Vision" feature gate deferred; `min_confidence` config field dropped because the LM doesn't expose per-region confidence).

## 2026-05-01 — `--config` flag silently ignored across all kebab-cli subcommands

**Discovered**: post-P3-5 manual smoke at `/tmp/kebab-smoke/`.

**Symptom**: `kebab --config /path/to/config.toml ingest|search|list|inspect|doctor` ignored the flag and fell back to `~/.config/kebab/config.toml` (XDG default). Users had to use `KEBAB_*` env vars to point at a non-default config.

**Root cause**: `kebab-cli` read `cli.config` only inside `Cmd::Ingest` to build `SourceScope`, then called bare `kebab_app::ingest(scope, summary_only)` which internally re-loaded `Config::load(None)` (XDG path). Same pattern in `Cmd::Search` / `List` / `Inspect` / `Doctor`. P3-5 introduced `*_with_config` test seams via `#[doc(hidden)] pub fn` but kebab-cli never used them.

**Fix** (PR #20, fix/cli-config-flag-and-search-output):
- `kebab-cli` now builds the Config once via `Config::load(cli.config.as_deref())` at the top of every subcommand and threads it into `kebab_app::*_with_config(cfg, ...)` instead of `kebab_app::*(...)`.
- `kebab_app::doctor()` rewritten as `doctor_with_config_path(Option<&Path>)` that reports the actual path probed and hard-fails when `--config <path>` doesn't exist (defaults would otherwise mask user intent).
- `kebab-app` module doc-comment updated: `#[doc(hidden)] pub fn *_with_config` is no longer "test-only seam" — it's the official "config-explicit" API consumed by CLI `--config`, integration tests, and TUI sessions.
- Same PR also improved `kebab search` printer: `{:.4}` score formatting (RRF range collapses on `{:.2}`) and `> heading_path` suffix so chunks from the same document are visually distinct.

**Amends**: tasks/p3/p3-5-app-wiring.md (the test seam was always meant to be the config-explicit API; only the doc-comment lied).

### 2026-05-01 — `--config` regression in `kebab ask` (P4-3 follow-up)

**Discovered**: post-P4-3 manual smoke against 192.168.0.47 Ollama with `gemma4:26b`.

**Symptom**: `kebab --config <path> ask` returned `model.id = qwen2.5:14b-instruct` (XDG default model) and `score_gate = 0.30` (XDG default), instead of `gemma4:26b` / `0.05` from the explicit config. P4-3 added the ask body but kebab-cli's `Cmd::Ask` arm still called bare `kebab_app::ask(query, opts)` — same regression class as the P3-5 fix above, just missed when ask was wired.

**Fix** (PR #24, fix/cli-ask-honor-config-flag):
- `kebab-cli` builds `Config::load(cli.config.as_deref())` once at the top of `Cmd::Ask` and calls `kebab_app::ask_with_config(cfg, query, opts)`.

**Amends**: tasks/p4/p4-3-rag-pipeline.md.

## 2026-05-01 — RRF `fusion_score` incompatible with `config.rag.score_gate` default

**Discovered**: post-P4-3 manual smoke. Top hybrid result returned `fusion_score = 0.0164` against `score_gate = 0.05` → ScoreGate refusal on every hybrid query.

**Root cause**: RRF formula `score(c) = Σ 1/(k_rrf + rank_m(c))` produces values bounded by `num_retrievers / (k_rrf + 1)`. With `num_retrievers = 2` and the default `k_rrf = 60`, the upper bound is `2/61 ≈ 0.0328`. The default `config.rag.score_gate = 0.05` was calibrated for vector / lexical scores already in `[0, 1]` and silently refused every hybrid query. `fusion_score` was also incomparable across modes — Lexical / Vector lived in `[0, 1]`, Hybrid lived in `(0, 0.033]`.

**Fix** (PR #25, fix/rrf-fusion-score-normalize-and-docs):
- `crates/kebab-search/src/hybrid.rs` divides every raw RRF score by `2 / (k_rrf + 1)` so `fusion_score` always lives in `[0, 1]` regardless of mode. Both retrievers contributing rank 1 normalises to `1.0`; chunks present in only one retriever cap around `0.5`. RRF's rank-ordering invariants are preserved (same constant divides every score), so sort + tiebreak behaviour is identical.
- One unit test (`rrf_formula_matches_known_value`) updated to expect the normalised value `(1/61 + 1/62) / (2/61) ≈ 0.9919`.
- The integration snapshot `crates/kebab-search/tests/fixtures/search/hybrid/run-1.json` already used presence checks (`fusion_score_positive: true`) rather than absolute values, so it didn't need regeneration.

**Why not a per-mode `score_gate` config**: separate `lexical_score_gate / vector_score_gate / hybrid_score_gate` would force every downstream consumer (CLI, eval, TUI) to know which mode picks which threshold. Normalising the score itself is a one-line change at the source and makes `Answer.retrieval.score_gate` semantically meaningful without per-mode bookkeeping.

**Amends**: tasks/p3/p3-4-hybrid-fusion.md (RRF formula now divides by `2/(k_rrf+1)` after summation), tasks/phase-3-vector-hybrid.md (RRF section).

**Verification**: post-fix smoke at `/tmp/kebab-smoke/` with default `score_gate = 0.05` succeeded across four scenarios — Korean→Korean, English→English, cross-language, and out-of-corpus refusal.

## How to add an entry

Each fix gets a dated subsection with five fields:

- **Discovered**: when / how the bug surfaced (smoke, integration test, user report).
- **Symptom**: what the user saw / what was wrong.
- **Root cause**: the actual code or design issue.
- **Fix**: PR number / branch + a one-paragraph summary of the change.
- **Amends**: which `tasks/p<N>/...` spec docs the fix retroactively contradicts. Spec text stays frozen; this log is the live source of truth for post-merge deltas.

If a fix is large enough that the original spec is no longer a useful reference, promote the entry into a new task spec (e.g., `p<N>-<M+1>-<topic>.md`) and link from here.
