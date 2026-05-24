---
title: "v0.17.0 설계 — 한국어 trigram FTS tokenizer + P10 round-2 dogfood 버그픽스"
date: 2026-05-22
status: draft
contract_sections: ["§5.5", "§9"]
---

# v0.17.0 설계 — 한국어 trigram FTS tokenizer + P10 round-2 dogfood 버그픽스

## 1. 배경

P10 종합 도그푸딩 round 2 (2026-05-22, `tasks/HOTFIXES.md`) 에서 세 가지가 드러났다:

- 한국어 `kebab search --mode lexical` 이 FTS5 `unicode61` 토크나이저에서 거의 0 hit. unicode61 은 공백·구두점 경계로만 토큰을 끊어, 한국어 어절(조사·어미 포함)이 통째로 한 토큰이 되고 부분 매칭이 안 된다.
- `code_lang_breakdown` 이 chunk 가 아닌 doc 수를 집계 — 코드가 많은 KB 에서 언어별 chunk 분포 granularity 가 떨어진다.
- C `typedef struct {...} Foo;` 의 alias 가 검색 symbol 로 노출되지 않는다.

이 설계는 셋을 v0.17.0 한 release 사이클에 묶어 처리한다. 본체는 한국어 tokenizer (변경 1), 나머지 둘은 같은 도그푸딩 라운드의 작은 버그픽스 (변경 2·3).

## 2. 범위

| # | 변경 | crate | cascade |
|---|------|-------|---------|
| 1 | FTS5 `unicode61` → `trigram` tokenizer | kebab-store-sqlite, migrations | V007 migration, design §5.5 갱신, release cut |
| 2 | `code_lang_chunk_breakdown` wire 필드 | kebab-store-sqlite, kebab-app, kebab-cli | wire additive (release 트리거 아님) |
| 3 | C typedef-wrapped struct → synthetic unit | kebab-parse-code, kebab-app(ingest), kebab-store-sqlite(purge) | **`parser_version`** bump (`code-c-v1`→`code-c-v2`) + same-workspace_path orphan purge |

3개는 서로 독립적인 코드 경로다. 각각 별도 PR 로, 한 작업 세션에서 연속 진행하고, 셋 다 머지된 뒤 v0.17.0 release 를 한 번 cut 한다.

## 3. 변경 1 — FTS5 trigram tokenizer (본체)

### 3.1 현재 상태

`migrations/V002__fts.sql` 의 `chunks_fts` 는 FTS5 가상 테이블 (V002 DDL 에 `content=''` 가 없어 contentless 가 아닌 일반 FTS5 shadow table) 이고 `tokenize = 'unicode61 remove_diacritics 2'` 로 생성된다. `chunks` 테이블의 INSERT/UPDATE/DELETE 가 trigger (`chunks_ai` / `chunks_ad` / `chunks_au`) 로 `chunks_fts` 와 동기화된다. 즉 `chunks` 가 source-of-truth, `chunks_fts` 는 검색용 shadow 다.

design §5.5 (`docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` 라인 1024-1043) 에 동일한 SQL 이 verbatim 으로 박혀 있고, 테스트 `fts_v002_matches_design_section_5_5_verbatim` (`crates/kebab-store-sqlite/tests/fts.rs`) 이 둘을 whitespace-normalized 로 대조하는 CI diff-check 다.

### 3.2 변경 내용

새 마이그레이션 `migrations/V007__fts_trigram.sql`:

1. `DROP TRIGGER` (`chunks_ai`/`chunks_ad`/`chunks_au`) + `DROP TABLE chunks_fts;` — 가상 테이블과 연결 trigger 를 명시적으로 제거.
2. `CREATE VIRTUAL TABLE chunks_fts USING fts5(..., tokenize = 'trigram');` — 컬럼 구성(`chunk_id`/`doc_id` UNINDEXED, `heading_path`, `text`)은 V002 와 동일, tokenizer 만 교체.
3. `chunks_ai`/`chunks_ad`/`chunks_au` trigger 재생성 — V002 와 동일 본문.
4. `INSERT INTO chunks_fts(chunk_id, doc_id, heading_path, text) SELECT chunk_id, doc_id, heading_path_json, text FROM chunks;` — 기존 chunk 전부 재색인 (V002 backfill 과 동일 패턴).

`chunks` 원본·embedding·vector index 는 전혀 건드리지 않는다. 마이그레이션이 FTS shadow 만 재구축하므로 **사용자는 `kebab ingest` 를 다시 돌릴 필요가 없다** — 0.17.0 바이너리가 기존 DB 를 열면 V007 이 자동 적용되며 backfill 까지 끝난다. 비싼 fastembed 재계산이 없다.

### 3.3 동반 갱신

- design §5.5 verbatim 블록을 V007 의 SQL 로 갱신한다. frozen design 변경이므로 release 트리거 중 하나다. design 본문 어디든 "contentless" 표현이 있으면 함께 "shadow / non-contentless" 로 정정.
- CI diff-check 테스트: 함수명에 `v002` 가 박혀 있으므로 `fts_v007_matches_design_section_5_5_verbatim` 으로 갱신하고, 대조 대상을 V007 파일로 바꾼다.
- `crates/kebab-store-sqlite/src/fts.rs` 의 `rebuild_chunks_fts` 는 컬럼 구성이 동일하므로 코드 변경이 불필요하다 (tokenizer 는 테이블 DDL 에만 존재). 동작만 확인.
- `crates/kebab-search/src/lexical.rs:177` 의 `build_match_string()` **재설계가 본 PR 의 본체다**. Codex 리뷰 검증 결과: 현재 builder 는 whitespace split 후 각 토큰을 `"..."` 로 감싸 implicit AND 결합 → trigram 에서 2자 이하 토큰 (예: `해시`, `충돌`) 은 매칭 불가 → `해시 충돌` 같은 multi-token 한국어 query 가 0-hit. trigram 대응 재설계 필요 — 권장: 3자 미만 토큰을 drop 또는 raw 처리, 전체 query 가 3자 이상이면 전체 query phrase 도 OR 후보로 추가.
- **2자 이하 한국어 query 정책 (사용자 결정)**: lexical core 는 정상 0-hit (변경 없음), CLI/TUI 레이어가 결과 0 + query 3자 미만일 때 "3자 이상 키워드 권장 (trigram tokenizer 제약)" 한 줄 안내. `--json` 모드는 wire 무결성 위해 안내 미출력. hybrid 모드는 vector 가 결과를 받쳐 안내가 안 나오는 케이스가 많다.
- `crates/kebab-search/src/lexical.rs:506` 부근의 lexical BM25 snapshot 테스트 갱신 — token stream 이 word → trigram 으로 바뀌어 raw score 분포·`snippet()` token 단위가 달라진다.
- `docs/wire-schema/v1/schema.schema.json` 에 변경 2 의 `code_lang_chunk_breakdown` 추가 (PR-C 에서 처리).
- `docs/SMOKE.md` 에 한국어 검색 시나리오 추가 (PR-A 에서 처리).

### 3.4 trade-off

- trigram 은 3자 (Unicode chars) 이상 substring 만 색인한다 (Codex 가 sqlite 3.45.1 로 검증). 3자 미만 query (`값`/`키`/`충돌`) 는 lexical 0-hit — unicode61 에서도 어절 단위 토큰화라 단일 토큰 부분 매칭은 안 됐으므로 단일 토큰 측면은 회귀가 아니다.
- 단 multi-token 한국어 query (`해시 충돌`) 는 §3.3 의 query builder 재설계가 동반돼야 hit 한다. builder 재설계가 본 PR 의 본체.
- 2자 이하 query 0-hit 시 CLI/TUI 가 안내 출력 (§3.3, 사용자 결정).
- 영어 lexical 검색도 substring 매칭으로 바뀐다: recall 상승, 단어 경계 정밀도 하락 가능. lexical-only KB 의 영어 검색 동작이 변경된다 — 의도된 동작 변경, 테스트로 핀.
- **BM25 score 분포 변경**: 알고리즘은 유지되지만 token stream 이 word → overlapping trigram 으로 바뀌어 raw score, term frequency, document length 모두 달라진다. lexical snapshot 갱신 (§3.3). `snippet()` 의 token 도 trigram 기준이라 word budget 의미가 달라진다. hybrid (RRF) 는 rank 기반이라 ranking 자체 영향은 미미, 단 `retrieval.lexical_score` 노출값은 변동.
- **DB 디스크 용량 증가**: trigram 인덱스는 unicode61 대비 통상 2-10배 크다 (chunk 본문 + heading_path 모두 trigram 색인). 기존 KB 가 V007 적용 후 `kebab.sqlite` 파일 크기 증가. release notes 명시.
- **`heading_path_json` JSON 노이즈**: trigram 이 JSON 표기 (`[`, `"`, `,`) 와 그 안의 단어 (예: `app`, `src`) 까지 3-gram 색인 → query 가 우연히 JSON 구문이나 흔한 경로 단어와 겹쳐 false positive 가능. v0.17.0 에서는 컬럼 구성 유지 (column filter / 평문 heading 변환 결정은 도그푸딩 후), Risks 등재.
- `remove_diacritics` 는 trigram tokenizer 에서 SQLite 버전 의존 (3.45.0+). 호환성 위해 `tokenize = 'trigram'` 단독 사용 (case-insensitive 기본). 빌드 환경 SQLite 버전은 plan 단계에서 확인.

### 3.5 사용자 영향

- 옛 binary (≤0.16.x) 는 V007 적용 DB 와 비호환 → v0.17.0 release cut 이 필요하다 (CLAUDE.md release cascade: V00X migration 트리거).
- 한국어 문서 KB 에서 `--mode lexical` / `--mode hybrid` 가 정상 동작한다 (3자 이상 substring). 도그푸딩에서 확인된 "한국어 hybrid 의 lexical 기여가 0" 문제가 해소된다.
- `kebab.sqlite` 파일 크기가 trigram 인덱스 비대화로 증가한다 (V007 자동 backfill 후). release notes 에 안내.
- 2자 이하 query 검색 시 lexical 0-hit + CLI/TUI 안내 메시지 표시 (§3.3).

## 4. 변경 2 — code_lang_chunk_breakdown

`crates/kebab-store-sqlite/src/store.rs` 의 기존 `code_lang_breakdown()` (doc 수, `documents` GROUP BY) 는 그대로 두고, `code_lang_chunk_breakdown()` 을 추가한다. `chunks` 테이블에는 `code_lang` 컬럼이 직접 없으므로 `chunks JOIN documents ON chunks.doc_id = documents.doc_id` 로 `documents.metadata_json` 의 `code_lang` 을 끌어와 `COUNT(chunks.chunk_id)` GROUP BY. 반환 타입은 기존과 동일 `BTreeMap<String, u32>`.

`crates/kebab-app/src/schema.rs` 의 `Stats` 에 `code_lang_chunk_breakdown: BTreeMap<String, u32>` 필드를 추가하고, stats 빌드 지점에서 신규 함수 호출로 채운다. `crates/kebab-cli/src/wire.rs::wire_schema()` 는 `SchemaV1` 을 serde 로 통째 직렬화하므로 **별도 수정 불필요** — 신규 필드가 자동으로 wire 출력에 포함된다. 단 `docs/wire-schema/v1/schema.schema.json` 에 `code_lang_chunk_breakdown` 을 additive 로 추가 (필수).

기존 `code_lang_breakdown` 필드는 유지 (제거 시 wire breaking). additive 추가 → migration·`schema_version` bump 불필요, release 트리거 아님.

## 5. 변경 3 — C typedef-wrapped struct fix

`crates/kebab-parse-code/src/c.rs` 의 extractor 가 top-level `type_definition` 노드를 만나면, 그 내부의 anonymous `struct_specifier`/`enum_specifier`/`union_specifier` 를 탐지해 **typedef alias 이름** (`type_definition` 의 `declarator` 에서 추출) 으로 synthetic unit 을 방출한다. named struct 는 기존 경로를 그대로 유지한다.

**`parser_version` bump** (`crates/kebab-parse-code/src/c.rs:34` 의 `PARSER_VERSION = "code-c-v1"` → `"code-c-v2"`) 가 본 변경의 cascade 키다 — extractor output 이 바뀌기 때문이다. design §9 cascade: `doc_id` 는 `(workspace_path, asset_id, parser_version)` 기반이라 parser_version bump 만으로 doc_id 가 갱신된다. chunker (`crates/kebab-chunk/src/code_c_ast_v1.rs` 의 `code-c-ast-v1`) 는 **건드리지 않는다** — chunker 로직 동일.

**Cascade 실제 동작 (Codex round 2 검증)**: parser_version 만 바뀌고 파일 bytes 가 동일하면 `asset_id` 가 같아 기존 ingest 경로의 `stale_chunk_ids_at` (asset_id 변경 기반) 가 발동하지 않는다. 새 doc_id 로 `documents` INSERT 시 `idx_docs_workspace_path` UNIQUE 가 충돌하거나, 옛 doc_id row 와 옛 chunk/vector row 가 orphan 으로 잔존한다. 따라서 본 PR 은 **same-workspace_path orphan purge** 를 동반해야 한다 — ingest 의 parser-mismatch 분기에서 `(workspace_path, 다른 doc_id)` 옛 row 의 chunk_id 를 수집해 `VectorStore::delete_by_chunk_ids` (P7-3 hotfix helper) 호출 + `documents` row 교체. plan B1 에 별도 step.

현재는 dogfood 단계라 prod KB 가 없다.

기존 테스트 `c_extractor_typedef_struct_falls_into_glue` 는 동작이 반대로 바뀌므로 `c_extractor_typedef_struct_emits_unit` 으로 재작성한다. HOTFIXES 2026-05-21 항목을 closure 로 갱신하고, spec `tasks/p10/p10-1d-c-cpp-ast-chunker.md` 의 Risks/notes 를 갱신한다.

## 6. PR 구성 / release

- **PR-A**: 변경 1 (trigram tokenizer). `feat/*` 브랜치 — 코드 + V007 migration + design §5.5 + task spec 을 한 PR 에 (design 변경과 그것을 참조하는 task spec 은 같은 PR 규칙).
- **PR-B**: 변경 3 (C typedef). `feat/*` 브랜치.
- **PR-C**: 변경 2 (code_lang_chunk_breakdown). `feat/*` 브랜치.
- 셋 머지 후 `chore: bump version 0.16.1 → 0.17.0` 같은 commit 직후 같은 commit 에 `gitea-release v0.17.0`. release notes 는 도그푸딩 영향 surface 위주 — 한국어 lexical 검색 동작, C symbol 노출, `schema.v1.stats` 신규 필드.

PR-A 가 design 변경을 포함하므로 README/HANDOFF/ARCHITECTURE sync 규칙이 적용된다 — 한국어 검색 동작을 README 검색/Configuration 절에 한 줄, HANDOFF "머지 후 발견된 버그/결정" 절, HOTFIXES round-2 항목 status 갱신.

## 7. 작업 방식 (team)

- **코드 작성**: Claude Code — OMC `executor` agent, migration·extractor 같은 복잡 부분은 `model=opus`.
- **리뷰**: Codex + Gemini 가 각 PR 의 diff 를 리뷰한다 (`/ask codex`, `/ask gemini` — OMC ask 라우팅). Claude 가 두 리뷰를 종합해 반영한다.
- **PR 생성·머지**: gitea-ops skill (Gitea REST API).
- 각 PR = 구현 → codex+gemini 리뷰 → 반영 → 머지 루프.

## 8. 테스트 전략

- 변경 1:
  - `crates/kebab-store-sqlite/tests/fts.rs`: V007 ↔ design §5.5 diff-check (테스트명 `fts_v007_matches_design_section_5_5_verbatim` 으로 rename).
  - 한국어 trigram 매칭 테스트 — **3자 이상 연속 substring 만 hit**. fixture `"해시 충돌은 키와 값을 매핑할 때 발생한다"` 기준 (Codex sqlite 3.45.1 검증): raw `MATCH '충돌은'` hit (공백 없는 3자 연속), `MATCH '"해시 충돌"'` quoted phrase hit, `MATCH '"시 충"'` quoted phrase hit; 반면 raw `MATCH '해시충'`/`MATCH '시 충'` 은 0-hit (전자는 원문에 해당 trigram 없음, 후자는 FTS5 가 raw 입력의 공백을 토큰 경계로 처리). quoted phrase 또는 공백 없는 연속 substring 으로 테스트.
  - **2자 query 0-hit 핀 테스트** — `MATCH '충돌'` 같은 2자 query 가 반드시 0 결과 (trigram 구조 회귀 감지).
  - **multi-token 한국어 query 테스트** (kebab-search / kebab-app 통합) — 사용자 query `해시 충돌` 이 재설계된 `build_match_string()` 을 거쳐 hit (whole phrase 후보 `"해시 충돌"` 경로). A4 작성 시점 FAIL, A5 후 PASS.
  - 영어 substring 동작 핀 (`token` query 가 `tokenizer`/`testbed` 등 hit).
  - lexical BM25 snapshot (`crates/kebab-search/src/lexical.rs:506` 근처 또는 `crates/kebab-search/tests/`) 갱신.
  - 기존 `crates/kebab-app/tests/search_korean.rs` 회귀 핀 (`러스트` 3자) + `해시 충돌` multi-token assert 추가.
  - CLI/TUI 안내 메시지 (3자 미만 query + 0 결과) 테스트 — `kebab-cli` stderr 검증, `kebab-tui` Search pane 단위 테스트.
- 변경 2: `crates/kebab-app/src/schema.rs` stats 테스트에 `code_lang_chunk_breakdown` 필드 검증 (한 doc 다중 chunks fixture 로 doc count 와 다른 값). `docs/wire-schema/v1/schema.schema.json` JSON 검증.
- 변경 3: `c.rs` typedef 테스트 재작성 (`Point` alias 가 unit 방출), `parser_version = "code-c-v2"` 확인, named struct 회귀 없음.
- 전체: `cargo test --workspace --no-fail-fast -j 1`, `cargo clippy --workspace --all-targets -- -D warnings`.

## 9. Risks / notes

- `lexical.rs::build_match_string()` 재설계가 본 PR 의 본체 — multi-token 한국어 query, 3자 미만 토큰 정책, lexical snapshot drift. Codex 검증으로 현재 builder 가 trigram 비호환임이 확정됨 (`해시 충돌` 0-hit). 빈 MATCH 는 FTS5 syntax error 이므로 후보 없음 시 `None` 반환 (SQL 미실행).
- PR-B 의 parser_version cascade — 같은 bytes + parser bump 케이스 (orphan vector/document row) 가 ingest 의 기존 asset_id 기반 purge 로 정리 안 됨 (Codex round 2 검증). same-workspace_path 명시 purge 가 PR-B 의 구성 요소. (미래의 모든 parser_version bump 에도 같은 보강이 필요할 수 있는 일반 케이스.)
- `heading_path_json` JSON 노이즈 — v0.17.0 에서는 컬럼 구성 유지, 도그푸딩 후 column filter (lexical query 를 `{text} : <q>` 한정) 또는 평문 heading 변환 재검토. HOTFIXES 후속 entry 로 등재.
- SQLite 파일 크기 증가 (trigram 인덱스) — release notes 명시. 검색 정확도와 무관.
- 영어 lexical 동작 변경 (substring 매칭) — release notes 명시.
- lexical BM25 raw score 분포 변경 — hybrid (RRF) 는 rank 기반이라 ranking 영향 미미, 단 `retrieval.lexical_score` 노출값 변동. wire schema 는 그대로지만 score 값 비교 기반 외부 도구가 있다면 영향.
- C typedef fix synthetic unit naming: nested typedef (`typedef struct { struct {...} inner; } Outer;`) 의 inner 익명 struct 는 여전히 glue. 1차 범위는 top-level typedef alias 만. spec Risks 명시.

## 10. contract_sections / 버전 cascade

- design §5.5 (Chunks + FTS5) — 변경 1 이 갱신 (tokenize 값 + "shadow / non-contentless" 표현).
- design §9 (versioning cascade) — 변경 3 의 **`parser_version` bump** (`code-c-v1` → `code-c-v2`) 가 cascade 사례. doc_id 가 `(workspace_path, asset_id, parser_version)` 기반이라 parser bump 만으로 다음 ingest 가 전체 재처리. chunker_version 은 chunk_id 에만 영향이라 본 fix 에는 불필요.
- 버전: workspace `Cargo.toml` 의 `version` 을 0.16.1 → 0.17.0 (minor bump, pre-1.0 단계 surface 변경 누적).
