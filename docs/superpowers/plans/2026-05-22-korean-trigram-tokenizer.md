# 한국어 trigram FTS tokenizer + dogfood 버그픽스 구현 Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** kebab 의 FTS5 tokenizer 를 `unicode61` → `trigram` 으로 교체해 한국어 lexical 검색을 가능하게 하고, 같은 도그푸딩 라운드의 작은 버그 둘(C typedef struct 미노출, code_lang_breakdown 집계 단위)을 함께 닫는다.

**Architecture:** 3개 독립 변경을 별도 PR(A/B/C)로 진행. PR-A 는 V007 migration 으로 `chunks_fts` shadow 테이블만 재구축(원본 `chunks`·embedding 불변) + `lexical.rs::build_match_string()` trigram 대응 재설계 + CLI/TUI 짧은 query 안내. PR-B 는 C extractor 에 typedef alias unit 방출 추가 + **`parser_version` `code-c-v1`→`code-c-v2` bump + same-workspace_path orphan purge** (Codex round 2 검증으로 추가). PR-C 는 wire additive 필드 + 기존 stats 필드 설명 정정. 셋 머지 후 v0.17.0 release cut.

**Tech Stack:** Rust 2024, SQLite FTS5, refinery migrations, tree-sitter-c, cargo test.

**작업 방식:** 코드는 Claude(`executor` agent), 각 PR diff 는 Codex + Gemini 가 리뷰(`/ask codex`·`/ask gemini`), PR 은 gitea-ops. design: `docs/superpowers/specs/2026-05-22-korean-trigram-tokenizer-design.md`.

---

## File Structure

**생성:**
- `migrations/V007__fts_trigram.sql` — chunks_fts 를 trigram tokenizer 로 재구축 + backfill

**수정:**
- `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` — §5.5 verbatim SQL 블록 + contentless 표현 정정
- `crates/kebab-store-sqlite/tests/fts.rs` — CI diff-check 테스트 + 한국어 trigram (3자 이상) + 2자 query 0-hit 핀 + 영어 substring 테스트
- `crates/kebab-search/src/lexical.rs` — `build_match_string()` 재설계 (필수) + BM25 snapshot 갱신
- `crates/kebab-cli/src/main.rs` (또는 search wrapper) — 2자 미만 query + 0 결과 시 안내 메시지
- `crates/kebab-tui/src/search.rs` — 동일 안내
- `crates/kebab-parse-code/src/c.rs` — typedef-wrapped struct → synthetic unit + `PARSER_VERSION` bump
- `crates/kebab-app/src/lib.rs` — ingest 경로의 same-workspace_path orphan purge (parser_version mismatch + asset 동일 케이스)
- `crates/kebab-store-sqlite/src/fts.rs` — 모듈 헤더 주석의 "contentless FTS5" 표현 정정 (실제는 일반 FTS5 shadow)
- `crates/kebab-store-sqlite/src/store.rs` — `code_lang_chunk_breakdown()` (JOIN documents)
- `crates/kebab-app/src/schema.rs` — `Stats.code_lang_chunk_breakdown`
- `docs/wire-schema/v1/schema.schema.json` — `code_lang_chunk_breakdown` additive 필드
- `docs/SMOKE.md` — 한국어 검색 시나리오 추가
- `README.md`, `HANDOFF.md`, `tasks/HOTFIXES.md`, `tasks/p10/p10-1d-c-cpp-ast-chunker.md`
- `Cargo.toml` — workspace `version`

(`crates/kebab-cli/src/wire.rs` 는 수정하지 않음 — `wire_schema()` 가 `SchemaV1` 을 serde 로 통째 직렬화하므로 변경 2 의 새 필드가 자동 포함됨.)

---

## PR-A — FTS5 trigram tokenizer

브랜치: `feat/korean-trigram-tokenizer`. design doc 도 이 PR 에 포함(아직 main 에 commit 안 됨).

### Task A1: 현재 query builder 동작 파악 + SQLite 버전 확인

Codex 리뷰로 현재 `build_match_string()` (lexical.rs:177) 이 trigram 비호환이라는 점은 이미 확정 (whitespace split → `"..."` AND 결합 → 한국어 multi-token 0-hit). 본 task 는 builder 의 정확한 동작 기록과 SQLite 버전 확인이 목적이며, 재설계 자체는 Task A5 (필수).

**Files:**
- Read: `crates/kebab-search/src/lexical.rs` (`build_match_string()` 본문, MATCH query 빌드 라인 260-290, lexical snapshot 라인 506 부근)

- [x] **Step 1: builder 동작 기록** — `build_match_string()` (lexical.rs:177-200) baseline:
  1. `text.trim()` → trimmed. 빈 → `None` 반환.
  2. `strip_single_quotes(trimmed)` 매치 시 (= `'...'` 전체 감싸기, closing quote 가 trimmed 의 마지막 char) → inner.trim() 빈 아니면 `Some(inner.to_string())` (raw FTS5 verbatim mode).
  3. 그 외 → `trimmed.split_whitespace().map(escape_fts5_token).collect()` → 빈이면 `None`, 아니면 ` ` join (FTS5 default implicit AND).
  - `escape_fts5_token` (lexical.rs:218): 토큰을 `"..."` 으로 wrap, inner `"` 은 doubling.
  - prefix `*` 별도 처리 없음 — 사용자가 raw mode 로 입력해야.
  - raw mode 진입 조건: 사용자가 single quote `'...'` 로 trimmed 전체를 감싼 경우 (`lexical.rs:167` 주석에 명시).
  - MATCH 호출: lexical.rs:281 `WHERE chunks_fts MATCH ?` (bound parameter).

- [x] **Step 2: SQLite 버전 확인** — `Cargo.toml`: `rusqlite = { version = "0.32", features = ["bundled"] }` + `Cargo.lock` `libsqlite3-sys = "0.30.1"` (system sqlite 무관, in-tree 빌드). libsqlite3-sys 0.30.1 의 번들 SQLite ~3.46.x — trigram (3.34+) 사용 가능. design 결정대로 `tokenize = 'trigram'` 단독 사용 (case-insensitive 기본). `remove_diacritics` 옵션 미사용.

- [x] **Step 3: lexical snapshot 위치 확인** — Codex round 1 의 "lexical.rs:506" 은 `fn normalize_bm25` (BM25 score → (0,1] mapping) 였음 — numerical transformation 이라 token stream 영향 없음. 진짜 snapshot 은:
  - `crates/kebab-search/tests/lexical.rs:1012` `lexical_snapshot_run_1` — fixture 기반, `KEBAB_UPDATE_SNAPSHOTS=1` env 로 regenerate, "baseline snapshot must exist; run with KEBAB_UPDATE_SNAPSHOTS=1 to seed".
  - `crates/kebab-search/tests/hybrid.rs:121` `hybrid_snapshot_run_1` — 동일 패턴 (`hybrid_snapshot drift`). 한국어 trigram 영향 받음 (token stream 변경).
  - inline `crates/kebab-search/src/lexical.rs:592` `normalize_bm25_top_score_in_unit_interval` — numerical, 영향 없음 (회귀 없음 확인만).
  Task A4 Step 5 에서 lexical_snapshot_run_1 + hybrid_snapshot_run_1 둘 다 regenerate.

### Task A2: V007 migration 작성

**Files:**
- Create: `migrations/V007__fts_trigram.sql`
- Read: `migrations/V002__fts.sql` (trigger 본문 verbatim 복사용)

- [x] **Step 1: V007 작성** — 아래 내용으로 생성. 컬럼 구성은 V002 와 동일, `tokenize` 만 교체. trigger 본문은 V002 와 동일.

```sql
-- V007__fts_trigram.sql
-- Replace the chunks_fts tokenizer: unicode61 -> trigram.
-- Korean is agglutinative; unicode61 tokenizes whole eojeol (with
-- particles attached) so substring matching fails. trigram indexes
-- 3-character grams, enabling Korean partial matches. See design §5.5
-- and tasks/HOTFIXES.md (2026-05-22).
--
-- chunks_fts is a shadow of chunks; this migration rebuilds it in
-- place and backfills from chunks, so no re-ingest is required.

DROP TRIGGER IF EXISTS chunks_au;
DROP TRIGGER IF EXISTS chunks_ad;
DROP TRIGGER IF EXISTS chunks_ai;
DROP TABLE IF EXISTS chunks_fts;

CREATE VIRTUAL TABLE chunks_fts USING fts5(
  chunk_id     UNINDEXED,
  doc_id       UNINDEXED,
  heading_path,
  text,
  tokenize = 'trigram'
);

CREATE TRIGGER chunks_ai AFTER INSERT ON chunks BEGIN
  INSERT INTO chunks_fts(chunk_id, doc_id, heading_path, text)
  VALUES (new.chunk_id, new.doc_id, new.heading_path_json, new.text);
END;
CREATE TRIGGER chunks_ad AFTER DELETE ON chunks BEGIN
  DELETE FROM chunks_fts WHERE chunk_id = old.chunk_id;
END;
CREATE TRIGGER chunks_au AFTER UPDATE ON chunks BEGIN
  DELETE FROM chunks_fts WHERE chunk_id = old.chunk_id;
  INSERT INTO chunks_fts(chunk_id, doc_id, heading_path, text)
  VALUES (new.chunk_id, new.doc_id, new.heading_path_json, new.text);
END;

INSERT INTO chunks_fts(chunk_id, doc_id, heading_path, text)
  SELECT chunk_id, doc_id, heading_path_json, text FROM chunks;
```

> Step 1 전에 `migrations/V002__fts.sql` 의 `CREATE VIRTUAL TABLE` 컬럼 목록과 trigger 본문을 실제로 대조해, 위 SQL 이 V002 와 trigger 본문·컬럼명(`heading_path_json` 등)에서 정확히 일치하는지 확인한다. 다르면 V002 를 source 로 맞춘다.

- [x] **Step 2: migration 적용 확인** — `cargo test -p kebab-store-sqlite` 통과 (10/10 fts tests + 모든 store test PASS). V007 backfill 도 정상 동작.

### Task A3: design §5.5 verbatim + CI diff-check 갱신

**Files:**
- Modify: `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` (§5.5, 라인 ~1024-1043)
- Modify: `crates/kebab-store-sqlite/tests/fts.rs` (`fts_v002_matches_design_section_5_5_verbatim`, 라인 ~408-435)

- [x] **Step 1: diff-check 테스트 baseline 확인** — A2 검증에서 `fts_v002_matches_design_section_5_5_verbatim` 는 PASS (V002 vs design 둘 다 unicode61 시점이라 match). V007 추가 자체는 기존 test 안 깨뜨림.

- [x] **Step 2: design §5.5 갱신** — `tokenize = 'unicode61 remove_diacritics 2'` → `'trigram'`. §5.5 본문 위에 한국어 trigram 채택 사유 + trade-off + "contentless 가 아님" 명시 prose 한 단락 추가.

- [x] **Step 3: diff-check 테스트를 V007 대상으로 갱신** — `extract_migration_5_5_verbatim_block()` 의 `include_str!` path 를 `V007__fts_trigram.sql` 로, 함수명 `fts_v002_matches_design_section_5_5_verbatim` → `fts_v007_matches_design_section_5_5_verbatim`, assertion msg 갱신.

- [x] **Step 4: 테스트 통과 확인** — `cargo test -p kebab-store-sqlite --test fts` → 10/10 PASS (`fts_v007_matches_design_section_5_5_verbatim` 포함).

- [ ] **Step 5: Commit** — A2 + A3 한 묶음으로 commit.

### Task A4: 한국어/영어 trigram 매칭 테스트

**Files:**
- Create: `fixtures/search/korean/hash-table.md` (또는 동등) — 도그푸딩 한국어 문서 복사
- Modify: `crates/kebab-store-sqlite/tests/fts.rs`
- Modify: `crates/kebab-app/tests/search_korean.rs` (회귀 핀 + multi-token assert + fixture 통합)
- Update: lexical BM25 snapshot (A1 Step 3 위치)

- [ ] **Step 0: 한국어 fixture 도입 (Gemini round 3 medium)** — 도그푸딩에 사용한 `/build/cache/dogfood-p10b/` 한국어 위키 문서 중 대표적인 것 (예: `hash-table.md`) 을 `fixtures/search/korean/` 으로 복사 + git add. 위키 문서가 CC-BY 등 외부 라이선스라면 `fixtures/search/korean/LICENSE` 에 출처·라이선스 표기 같이 commit. 통합 테스트가 이 fixture 를 ingest 해 재현성 확보.

- [ ] **Step 1: 한국어 trigram 매칭 테스트 (실패 확인)** — fixture chunk text `"해시 충돌은 키와 값을 매핑할 때 발생한다"` (V007 적용 store). Codex sqlite 3.45.1 검증 기준 동작:
  - raw `MATCH '충돌은'` (공백 없는 3자 연속 substring) → hit. ✓
  - quoted `MATCH '"해시 충돌"'` (whole phrase) → hit. ✓
  - quoted `MATCH '"시 충"'` (phrase 2 chars + space + 1 char) → hit. ✓
  - raw `MATCH '해시충'` → 0-hit (원문에 "해시충" 3-gram 이 연속으로 없음 — "해시" 공백 "충돌").
  - raw `MATCH '시 충'` (공백 포함 unquoted) → 0-hit (FTS5 가 공백을 토큰 경계로 처리).
  위 5개 assert. Expected: V007 적용 store 에서 PASS. store 테스트가 migration 을 V006 까지만 적용한다면 V007 까지 적용되도록 수정.

- [ ] **Step 1b: 2자 query 0-hit 핀 (회귀 감지)** — `MATCH '충돌'` (2 Unicode chars) 이 반드시 0 결과를 반환. trigram 구조 변경 감지 회귀 테스트.

- [ ] **Step 1c: multi-token 한국어 query 테스트** — `crates/kebab-search` 또는 `crates/kebab-app` 통합 레벨. 사용자 query `해시 충돌` 이 `build_match_string()` 을 통해 hit 하는지. Expected: A4 시점 FAIL (현재 builder 가 `"해시" "충돌"` AND 로 trigram 0-hit), Task A5 builder 재설계 후 PASS.

- [ ] **Step 2: 영어 substring 동작 핀** — 영어 텍스트에 대해 trigram substring 매칭 (예: `tokenizer` 텍스트가 `MATCH 'token'` 에 hit) 을 명시적으로 문서화·고정.

- [ ] **Step 3: 통과 확인 (부분)** — `cargo test -p kebab-store-sqlite` → Step 1 / 1b / 2 PASS. Step 1c 는 A5 후.

- [ ] **Step 4: 통합 회귀 확인** — `cargo test -p kebab-app search_korean` (`러스트` 3자라 trigram 으로도 통과). `search_korean.rs` 에 `해시 충돌` multi-token assert 추가 (A5 후 통과).

- [ ] **Step 5: lexical BM25 snapshot 갱신** — A1 Step 3 에서 식별한 snapshot 파일을 trigram token stream 기준으로 갱신 (`cargo insta accept` 또는 수동). snippet token 단위가 trigram 으로 바뀌므로 word budget 관련 테스트 기대값도 함께 검토.

- [ ] **Step 6: Commit** — `git commit` (test: korean + english trigram matching + bm25 snapshot).

### Task A5: lexical.rs query builder 재설계 (필수)

Codex 검증: 현재 `build_match_string()` (lexical.rs:177) 은 whitespace split 후 각 토큰을 `"..."` 로 감싸 implicit AND 결합. 각 토큰이 2자 이하면 trigram MATCH 가 0-hit → `해시 충돌` 같은 multi-token 한국어 query 가 깨짐. 본 task 는 builder 를 trigram 대응으로 재설계.

**사용자 결정** (2자 이하 한국어 query 정책): lexical core 는 정상 0-hit (변경 없음), 안내 메시지는 CLI/TUI 레이어가 출력 ("3자 이상 키워드 권장").

**A1 baseline 노트** (Task A1 Step 1 에서 채움):

`build_match_string(text: &str) -> Option<String>` (lexical.rs:177-200) baseline:

1. `text.trim()` → trimmed. 빈 → `None`.
2. `strip_single_quotes(trimmed)` 매치 시 (single quote `'...'` 가 trimmed 전체 감쌈, closing quote 가 마지막 char — `'foo' bar` 는 raw 아님) → inner.trim() 빈 아니면 `Some(inner.to_string())` (raw FTS5 verbatim).
3. 그 외 → `trimmed.split_whitespace().map(escape_fts5_token).collect()` → 빈이면 `None`, 아니면 ` ` join (FTS5 default implicit AND).

`escape_fts5_token(tok)` (lexical.rs:218): `"..."` wrap + inner `"` doubling.

재설계 시 회귀 방지 — raw mode (single quote `'...'`) 진입 조건은 그대로 유지. escape_fts5_token 도 그대로 (trigram 도 FTS5 special char escape 필요). 변경은 비-raw 경로의 토큰 합성만.

SQLite: rusqlite 0.32 + libsqlite3-sys 0.30.1 **bundled** (in-tree). SQLite ~3.46.x → trigram 사용 가능.

Snapshot: `crates/kebab-search/tests/lexical.rs::lexical_snapshot_run_1` + `crates/kebab-search/tests/hybrid.rs::hybrid_snapshot_run_1` (둘 다 `KEBAB_UPDATE_SNAPSHOTS=1` 로 regenerate). inline `normalize_bm25_top_score_in_unit_interval` 는 numerical 영향 없음.

**Files:**
- Modify: `crates/kebab-search/src/lexical.rs` (`build_match_string()`)
- Modify: `crates/kebab-cli/src/main.rs` 또는 search 결과 처리 wrapper — 안내 메시지
- Modify: `crates/kebab-tui/src/search.rs` 또는 결과 렌더 — 안내 메시지

- [ ] **Step 1: builder 재설계 테스트 작성 (실패 확인)** — `해시 충돌` multi-token 한국어 query + 한영 혼합 query (`Rust 충돌은`) 가 hit 하는 테스트. raw FTS mode 진입 (사용자가 single quote `'...'` 로 감싼 경우, `lexical.rs:167`) 회귀 테스트. Expected: FAIL.

- [ ] **Step 2: `build_match_string()` 재설계** — Codex round 2 권장안 (검증된 알고리즘):
  1. raw single-quote mode (사용자가 single quote `'...'` 로 감싼 경우, `lexical.rs:167`) 는 기존 유지.
  2. `whole = escape_fts5_phrase(trimmed)` 를 항상 첫 후보로 — 단 `trimmed.chars().count() >= 3` 일 때만.
  3. whitespace 로 분리된 토큰 중 `chars().count() >= 3` 만 escaped token AND 후보 생성.
  4. 후보가 둘 다 있으면 `(<whole>) OR (<token_and>)`, 하나만 있으면 그대로.
  5. **후보가 하나도 없으면 `None` 반환 (빈 MATCH 금지 — FTS5 syntax error).** 호출자는 None 시 SQL 실행 자체를 회피하고 빈 결과를 반환.
  이러면 `해시 충돌` (각 토큰 2자, whole 5자) → whole phrase 후보로 hit, `충돌` (whole 2자, token 0개) → None → 0-hit, `Rust 충돌은` (token 2개 모두 ≥3) → AND + whole 모두 후보 → OR hit. escape 는 trigram 도 `"`, `*` 처리 필요 — 기존 로직 보강.

- [ ] **Step 3: 테스트 통과 확인** — Step 1 신규 + Task A4 Step 1c·4 (`해시 충돌`) PASS.

- [ ] **Step 4: 안내 메시지 — CLI** — `crates/kebab-cli/src/main.rs` 의 `kebab search` 결과 처리에서, 결과가 비어 있고 **`query.trim().chars().count() < 3`** (trimmed 전체 기준) 일 때 stderr 에 "3자 이상 키워드 권장 (trigram tokenizer 제약)" 한 줄. **"모든 토큰이 3자 미만" 조건은 사용 금지** (Codex round 3 medium) — `해시 충돌` 같은 valid whole-phrase query 에 false trigger 회피. `--json` 모드에서는 stderr 안내 미출력 (wire hint 는 Step 4b 에서 별도 전달).

- [ ] **Step 4b: wire `search_response.v1` 에 `hint` 필드 추가 (MCP 가시성, Gemini round 3 high)** — `--json` 모드와 MCP 가 사용하는 search response 에도 hint 가 전달돼야 LLM/agent 가 "0 결과 + 3자 미만" 케이스를 이해함. 변경:
  - `crates/kebab-app/src/schema.rs` (또는 search 응답 type 정의 위치) 의 `SearchResponse` 에 `hint: Option<String>` additive 필드 추가.
  - search 실행 결과가 비어 있고 query trimmed.chars().count() < 3 일 때 `hint = Some("3자 이상 키워드 권장 (trigram tokenizer 제약)")`, 그 외 None.
  - `crates/kebab-mcp` 의 `search` tool 결과 직렬화에 hint 포함 (serde 자동이면 OK, 확인).
  - `docs/wire-schema/v1/search_response.schema.json` (또는 search 응답 스키마 파일) 에 `hint: { type: ["string", "null"] }` additive 필드 명세.
  - CLI 의 Step 4 stderr 안내는 사람 가시성, wire hint 는 agent 가시성 — 둘은 보완적, 같은 조건 사용.

- [ ] **Step 5: 안내 메시지 — TUI** — Codex round 2/3 권장 구현 (`search.rs`/`app.rs`/`run.rs` 실제 구조 기반):
  - `SearchState` (`crates/kebab-tui/src/app.rs:116` 근처) 에 `short_query_hint: Option<String>` 필드 추가.
  - **Stale hint 방지 (Codex round 3 high)**: 현재 generation 은 `fire_search` 때만 증가하고 input mutation 때는 증가 안 함 — `poll_worker` 가 worker 결과 수신 시 `last_query == 현재 SearchState.input.content && last_mode == 현재 mode` 일치 시만 hint 를 세팅한다. 불일치 시 (사용자가 새 query 입력 중) hint 세팅 skip — stale worker 결과로 새 input 화면이 덮이지 않게.
  - 추가로 input 이 변경되면 (`set_input` 등) `short_query_hint = None` reset.
  - hint 세팅 조건: `last_query.trim().chars().count() < 3` (trimmed 전체 기준, Codex round 3 medium 으로 통일 — 토큰 기반 분기 사용 금지) + hits 비어 있음 + raw mode 아님.
  - 표시: `dynamic_status` (`crates/kebab-tui/src/run.rs:389` 근처) 또는 Search pane 의 결과 영역 empty render 분기에서 `short_query_hint` 가 Some 일 때 한 줄 표시.

- [ ] **Step 6: 안내 메시지 테스트** — CLI stderr 캡처 + 미출력 케이스 (`--json`, 3자 이상 query, 결과 ≥ 1) 각각 테스트. TUI 안내 표시 unit 테스트.

- [ ] **Step 7: 전체 검증** — `cargo test -p kebab-search -p kebab-cli -p kebab-tui` → 신규 + 기존 PASS.

- [ ] **Step 8: Commit** — `git commit` (feat: trigram-aware query builder + short-query guidance).

### Task A6: 사용자 문서 동기화

**Files:**
- Modify: `README.md`, `HANDOFF.md`, `tasks/HOTFIXES.md`, `docs/SMOKE.md`

- [ ] **Step 1: README** — 검색/Configuration 절에 한 줄: 한국어 포함 KB 의 `--mode lexical`/`hybrid` 가 trigram 3-gram substring 으로 동작 (3자 이상 query 권장). SQLite 파일 (`kebab.sqlite`) 크기가 trigram 인덱스 비대화로 증가 (도그푸딩 KB 기준 ~2-5배 또는 수백 MB 단위, Gemini round 3 low) 한 줄.

- [ ] **Step 2: HANDOFF** — "머지 후 발견된 버그/결정" 절의 2026-05-22 한국어 lexical 항목을 "v0.17.0 trigram 으로 해소" 로 갱신. "P10 dogfooding 백로그" 의 한국어 tokenizer 항목 상태 갱신.

- [ ] **Step 3: HOTFIXES** — 2026-05-22 한국어 lexical 항목의 "Next step (미진행)" 을 v0.17.0 / V007 으로 closure 처리. trigram 채택, 영어 동작 변경, 디스크 용량 증가, `heading_path` JSON 노이즈 후속을 dated 항목으로 기록.

- [ ] **Step 4: SMOKE.md** — 한국어 검색 시나리오 추가 (Codex round 3 high: hit query 가 자기 단언과 모순되지 않게):
  - fixture: A4 Step 0 에서 commit 한 `fixtures/search/korean/hash-table.md` (또는 동등) 를 ingest.
  - `kebab search --mode lexical '충돌은'` (원문에 공백 없이 3자 연속 substring) → hit 확인.
  - `kebab search '해시 충돌'` (multi-token, builder 가 whole phrase 후보로 hit) → hit 확인.
  - `kebab search --mode lexical '충돌'` (2자) → 0-hit + "3자 이상 키워드 권장" stderr 안내 확인.
  - `kebab search --mode lexical '충돌' --json` → 결과 hits 빈 배열 + `hint` 필드 (Step 4b) 포함 확인.
  - V007 자동 backfill (re-ingest 불필요) + SQLite 파일 크기 증가 안내 (도그푸딩 KB 기준 ~2-5배 또는 수백 MB).

- [ ] **Step 4b: SKILL.md (Gemini round 3 medium)** — `integrations/claude-code/kebab/SKILL.md` 의 `mcp__kebab__search` 섹션 또는 Don't 섹션에 한 줄 추가: "한국어 lexical 검색 시 3자 이상의 키워드를 사용하는 것이 검색 품질·recall 측면에서 유리. 2자 이하 한국어 query (예: '값', '키', '충돌') 는 trigram tokenizer 구조상 lexical 0-hit — search_response 의 `hint` 필드 확인 권장."

- [ ] **Step 5: Commit** — `git commit` (docs: trigram tokenizer — README/HANDOFF/HOTFIXES/SMOKE/SKILL).

### Task A7: PR-A 생성 + 리뷰 루프

- [ ] **Step 1: 전체 검증** — `cargo test --workspace --no-fail-fast -j 1` + `cargo clippy --workspace --all-targets -- -D warnings`. 둘 다 통과 확인.
- [ ] **Step 2: PR 생성** — gitea-ops 로 `feat/korean-trigram-tokenizer` → main PR. 본문에 design doc 링크 + V007 자동 backfill(re-ingest 불필요) 명시.
- [ ] **Step 3: 리뷰** — PR diff 를 `/ask codex` + `/ask gemini` 로 리뷰. 두 리뷰 종합 후 반영 — 반영 시 같은 브랜치에 commit, 재검증.
- [ ] **Step 4: 머지** — 리뷰 반영 완료 + CI green 후 머지.

---

## PR-B — C typedef-wrapped struct fix

브랜치: `feat/c-typedef-struct-unit`.

### Task B1: typedef extractor fix (TDD)

**Files:**
- Modify: `crates/kebab-parse-code/src/c.rs` (extractor 라인 ~254-262, `PARSER_VERSION` 라인 34, 테스트 라인 ~492-505)

- [ ] **Step 1: 기존 테스트 재작성(실패 확인)** — `c_extractor_typedef_struct_falls_into_glue` 를 `c_extractor_typedef_struct_emits_unit` 으로 바꾼다. `typedef struct { int x; int y; } Point;` 입력에서 `Point` 라는 이름의 unit 이 방출되는지 assert. Expected: FAIL (현재는 glue 로 빠짐).

- [ ] **Step 2: extractor 수정** — top-level `type_definition` 노드 처리: 내부에 anonymous `struct_specifier`/`enum_specifier`/`union_specifier`(name 필드 없음)가 있으면, `type_definition` 의 `declarator`(typedef alias)에서 이름을 추출해 그 이름으로 unit 을 방출한다. named struct 경로는 그대로 둔다. 코드 변경 전 `c.rs` 의 현재 노드 분기(`struct_specifier | enum_specifier | union_specifier` arm)와 tree-sitter-c 의 `type_definition` 자식 구조를 읽고 맞춘다.

- [ ] **Step 3: 테스트 통과 확인** — `cargo test -p kebab-parse-code c_extractor_typedef` → PASS.

- [ ] **Step 4: named struct 회귀 확인** — `cargo test -p kebab-parse-code` 전체 → 기존 C extractor 테스트(named struct, glue 등) 모두 PASS.

- [ ] **Step 5: parser_version bump** — `crates/kebab-parse-code/src/c.rs:34` 의 `PARSER_VERSION = "code-c-v1"` 을 `"code-c-v2"` 로 bump. **chunker (`crates/kebab-chunk/src/code_c_ast_v1.rs` 의 `code-c-ast-v1`) 는 건드리지 않는다** — extractor output 만 바뀌고 chunker 로직 동일. C extractor 스냅샷/통합 테스트가 `parser_version` 문자열을 assert 하면 `code-c-v2` 로 갱신.

- [ ] **Step 5b: same-workspace_path orphan purge (Codex round 2 critical)** — parser_version bump 만으로 doc_id 가 갱신되지만, **파일 bytes 동일 (asset_id 동일) 케이스에서 기존 ingest 의 `stale_chunk_ids_at` (asset_id 변경 기반) 가 발동하지 않아 옛 doc_id row + 옛 chunk row + Lance vector 가 orphan 으로 남고 `idx_docs_workspace_path` UNIQUE 충돌이 날 수 있다**. 보강:
  - **신규 helper 도입 (Codex round 3 medium)**: P7-3 의 `stale_chunk_ids_at` (`store.rs:440`) / `purge_orphan_at_workspace_path` (`store.rs:497`) 는 `asset_id != new_asset_id` 전용이라 parser-only bump 케이스에 no-op. 기존 helper 그대로 호출/확장보다 새 helper 두 개를 `crates/kebab-store-sqlite/src/store.rs` 에 추가:
    - `stale_chunk_ids_for_workspace_path_except_doc_id(workspace_path, new_doc_id) -> Vec<ChunkId>` — 같은 workspace_path 의 다른 doc_id 가 가진 chunk_ids 수집.
    - `purge_document_at_workspace_path_except_doc_id(workspace_path, new_doc_id)` — 같은 workspace_path 의 다른 doc_id row 와 그 chunks 제거.
  - `crates/kebab-app/src/lib.rs` 의 code asset ingest 분기 (parser mismatch 판정 직후, `lib.rs:812`/`882` 근처) 에서 위 두 helper 순차 호출: chunk_ids 수집 → `VectorStore::delete_by_chunk_ids` (P7-3 hotfix helper, 이건 chunk_id 기반이라 재사용 가능) → document/chunks row delete → 새 doc_id 로 정상 ingest 계속.
  - 테스트: fixture C 파일을 `code-c-v1` 로 한 번 ingest → `PARSER_VERSION` 을 `v2` 로 모의 변경 후 같은 fixture 재 ingest → 옛 doc_id row 사라지고 새 doc_id 만 남음 + Lance vector 도 새 chunk_ids 만 존재 + UNIQUE 충돌 없음 확인.

- [ ] **Step 5c: 회귀 테스트 — 다른 asset 시 기존 purge 동작 유지** — bytes 가 실제로 바뀐 케이스 (asset_id 변경) 에서 `stale_chunk_ids_at` 가 기존대로 정리하는지 확인 (Step 5b 변경이 기존 경로 안 깨뜨리는지).

- [ ] **Step 6: 테스트 통과 확인** — `cargo test -p kebab-parse-code` 전체 → PASS.

- [ ] **Step 7: Commit** — `git commit` (fix: C typedef-wrapped struct emits named unit, parser_version code-c-v2).

### Task B2: HOTFIXES + spec 갱신, PR-B

**Files:**
- Modify: `tasks/HOTFIXES.md`, `tasks/p10/p10-1d-c-cpp-ast-chunker.md`

- [ ] **Step 1: HOTFIXES** — 2026-05-21 "typedef-wrapped struct/enum in C falls into glue" 항목의 Status/Next step 을 v0.17.0 closure 로 갱신.
- [ ] **Step 2: spec Risks** — `p10-1d-c-cpp-ast-chunker.md` 의 Risks/notes 에 typedef alias unit 방출(top-level 한정, nested 익명 struct 는 여전히 glue) 을 한 줄로 갱신. frozen spec 본문은 건드리지 않고 Risks 절만.
- [ ] **Step 3: Commit + PR** — `git commit` (docs) → gitea-ops 로 PR-B 생성.
- [ ] **Step 4: 리뷰 루프** — `/ask codex` + `/ask gemini` 리뷰 → 반영 → 머지.

---

## PR-C — code_lang_chunk_breakdown

브랜치: `feat/code-lang-chunk-breakdown`.

### Task C1: store 함수 추가 (TDD)

**Files:**
- Modify: `crates/kebab-store-sqlite/src/store.rs` (`code_lang_breakdown` 인접, 라인 ~801-825)

- [ ] **Step 1: 테스트 작성(실패 확인)** — `code_lang_chunk_breakdown()` 이 `chunks` 테이블 기준 언어별 chunk 수를 반환하는지 보는 store 테스트 추가. 한 doc 에 여러 chunk 인 fixture 로 doc 집계와 다른 값이 나옴을 확인. Expected: FAIL (함수 미존재).

- [ ] **Step 2: 함수 구현** — 기존 `code_lang_breakdown()` 패턴을 그대로 따르되 source 를 `chunks` 로: 언어 식별 컬럼을 `chunks` 에서 끌어온다. `chunks` 에 code_lang 이 직접 없으면 `chunks JOIN documents` 로 `documents` 의 code_lang 을 끌어 `COUNT(chunks)`. Step 2 전에 `chunks` 와 `documents` 스키마에서 code_lang 이 어디에 있는지 확인한다. 반환 타입은 `code_lang_breakdown` 과 동일한 `BTreeMap<String, u32>`.

- [ ] **Step 3: 테스트 통과 확인** — `cargo test -p kebab-store-sqlite code_lang_chunk` → PASS.

- [ ] **Step 4: Commit** — `git commit` (feat: code_lang_chunk_breakdown store query).

### Task C2: wire 필드 추가 (TDD)

**Files:**
- Modify: `crates/kebab-app/src/schema.rs` (`Stats`, 라인 ~69·170·202-219)
- Modify: `docs/wire-schema/v1/schema.schema.json` (`code_lang_chunk_breakdown` 필드)

- [ ] **Step 1: stats 테스트 확장 (실패 확인)** — `schema.rs` 의 `stats_includes_code_lang_and_repo_breakdown_fields` 테스트에 `code_lang_chunk_breakdown` 필드 존재·값 검증 추가. fixture 는 한 doc 에 여러 chunks (doc count 와 chunk count 가 다른 값으로 채워지는지 확인). Expected: FAIL (필드 미존재).

- [ ] **Step 2: Stats 필드 추가** — `Stats` 에 `code_lang_chunk_breakdown: BTreeMap<String, u32>` 추가, stats 빌드 지점에서 Task C1 의 `code_lang_chunk_breakdown()` 호출로 채운다. 기존 `code_lang_breakdown` 필드는 유지 (제거 시 wire breaking).

- [ ] **Step 3: wire.rs 자동 직렬화 확인** — `crates/kebab-cli/src/wire.rs::wire_schema()` 는 `SchemaV1` 을 serde 로 통째 직렬화하므로 별도 코드 수정 불필요. 신규 필드가 wire JSON 출력에 자동 포함됨을 `cargo test -p kebab-cli wire` 의 기존 schema wrapper 테스트가 확인 (또는 신규 assertion 추가).

- [ ] **Step 4: 테스트 통과 확인** — `cargo test -p kebab-app schema` + `cargo test -p kebab-cli` → PASS.

- [ ] **Step 5: wire schema JSON 갱신 (필수) + 기존 필드 설명 정정** — `docs/wire-schema/v1/schema.schema.json` 의 `Stats` 정의에:
  - `code_lang_chunk_breakdown` 을 기존 `code_lang_breakdown` 과 동일한 형태 (`{"type": "object", "additionalProperties": {"type": "integer", "minimum": 0}}`) 로 additive 추가.
  - Gemini round 2 발견: 기존 `code_lang_breakdown`·`repo_breakdown` 의 description 이 "chunk count" 로 잘못 적혀 있으면 (실제 구현은 doc count) "doc count" 로 정정. 추가 필드 `code_lang_chunk_breakdown` description 은 "chunk count" 로 명시.
  CI 가 schema-vs-impl 대조를 한다면 함께 통과 확인.

- [ ] **Step 6: Commit + PR** — `git commit` (feat: code_lang_chunk_breakdown wire field) → gitea-ops 로 PR-C 생성.

- [ ] **Step 7: 리뷰 루프** — `/ask codex` + `/ask gemini` 리뷰 → 반영 → 머지.

---

## Release — v0.17.0

### Task R1: version bump + release cut

- [ ] **Step 1: 선행 확인** — PR-A·B·C 셋 다 main 에 머지됐는지 확인. `git pull` 후 `cargo test --workspace --no-fail-fast -j 1` green.
- [ ] **Step 2: version bump** — `Cargo.toml` workspace `version` `0.16.1` → `0.17.0`. `cargo build` 로 `Cargo.lock` 자동 갱신.
- [ ] **Step 3: Commit** — `git commit` (`chore: bump version 0.16.1 → 0.17.0`).
- [ ] **Step 4: release** — gitea-ops 의 `gitea-release v0.17.0`. release notes: 한국어 lexical 검색 trigram 동작, 영어 lexical substring 동작 변경, C typedef symbol 노출, `schema.v1.stats.code_lang_chunk_breakdown` 신규 필드, V007 자동 마이그레이션(re-ingest 불필요).
- [ ] **Step 5: HANDOFF/INDEX** — `HANDOFF.md` 한 줄 요약의 version (`v0.17.0`)·Phase 표 갱신. `tasks/INDEX.md` 의 P10 섹션 하단에 "P10 Dogfooding Feedback" 섹션을 만들어 v0.17.0 작업 (한국어 trigram + C typedef + code_lang_chunk_breakdown) 을 listup (P9 의 fb-01~42 형식 참고, Gemini round 2 권장).

---

## Self-Review (Codex+Gemini 리뷰 반영 후)

**Spec coverage:** design §3(변경 1)→PR-A Task A1-A7, §4(변경 2)→PR-C, §5(변경 3)→PR-B, §6(PR 구성/release)→Task R1, §8(테스트)→각 task 의 test step + A4 의 2자/multi-token/snippet, §9 Risks→A5(builder 재설계)·A4(영어 동작/heading_path 노이즈)·B1(nested typedef). §10 버전 cascade→B1 Step 5 (parser_version), R1 (workspace version). 누락 없음.

**Placeholder scan:** Task A5 의 "A1 baseline 노트" 는 의도적 plan-내 동적 슬롯 — A1 Step 1 이 채워 A5 가 참조. 그 외 "TBD/TODO" 없음. V007 SQL 전문 박음. 정확한 코드 (build_match_string 재설계, c.rs typedef 노드 분기, chunks JOIN documents 위치) 는 "해당 파일을 읽어 구현" 으로 명시 — placeholder 가 아닌 실행 지시.

**Type consistency:** `code_lang_chunk_breakdown` 명칭이 store 함수(C1)·Stats 필드(C2 Step 2)·wire JSON schema(C2 Step 5) 전체 동일. `BTreeMap<String, u32>` 반환 타입이 기존 `code_lang_breakdown` 과 일치. `chunks_fts` 컬럼명이 V007·design §5.5·diff-check 테스트 동일. `parser_version = "code-c-v2"` 문자열이 B1 Step 5·테스트 갱신·design §5·§10 일치.

**리뷰 반영 변경 (round 1):**
- 변경 1 본체에 `lexical.rs::build_match_string()` 재설계 추가 (A5 필수화).
- 2자 이하 한국어 query 정책 = 0-hit + CLI/TUI 안내 (사용자 결정).
- C typedef cascade 를 chunker_version → **parser_version** 으로 정정 (`code-c-v1` → `code-c-v2`).
- design §3.1 의 "contentless" 표현 정정 (V002 는 일반 FTS5 shadow).
- heading_path JSON 노이즈, 디스크 용량 증가, BM25 snapshot drift 를 Risks 등재.
- 누락 task 추가: SMOKE.md 갱신 (A6 Step 4), `docs/wire-schema/v1/schema.schema.json` 갱신 (C2 Step 5).
- 잘못된 task 제거: `wire.rs` 수정 (serde 자동 직렬화이므로 불필요).

**리뷰 반영 변경 (round 2):**
- **[Critical]** PR-B 에 same-workspace_path orphan purge step 추가 (B1 Step 5b/5c) — parser_version bump 만으로는 같은-asset 케이스에서 옛 doc_id/chunk/vector 가 orphan, UNIQUE 충돌 위험. design §5 본문에 실제 cascade 동작 명시.
- **[High]** design §2 표 + plan Architecture 의 잔존 "code-c-ast-v2 chunker bump" → "code-c-v2 parser_version bump" 로 정정.
- **[High]** A4 Step 1 의 trigram 테스트 예시를 Codex sqlite 3.45.1 검증 동작으로 정정 — quoted phrase 와 공백 없는 연속 substring 으로 (`'해시충'`/`'시 충'` 는 0-hit 가 맞음).
- **[High]** A5 Step 2 의 builder 알고리즘을 Codex 권장안으로 — whole phrase 후보 + 3자 이상 토큰 AND → OR 결합, 후보 없음 시 `None` 반환 (빈 MATCH 금지).
- **[Medium]** A5 Step 5 의 TUI 안내 구현을 `SearchState.short_query_hint` 필드 + `poll_worker` 세팅 + `dynamic_status` 표시로 구체화.
- **[Low]** File Structure 에 `crates/kebab-store-sqlite/src/fts.rs` (코드 주석의 contentless 정정) 추가.
- **[Low]** C2 Step 5 에 기존 stats 필드 (`code_lang_breakdown`·`repo_breakdown`) description 정정 추가 (실제는 doc count).
- **[Low]** R1 Step 5 의 INDEX.md 갱신 위치를 "P10 Dogfooding Feedback" 섹션으로 구체화.

**리뷰 반영 변경 (round 3):**
- **[Codex High]** SMOKE.md 시나리오의 hit query 를 `해시충` (원문 미존재) → `충돌은` (3자 연속) + `해시 충돌` (whole phrase) 로 정정. JSON 모드 hint 필드 검증도 시나리오에 포함.
- **[Codex High]** TUI short_query_hint 의 stale 방지 — `poll_worker` 가 `last_query == 현재 input + mode` 일치 시만 hint 세팅, input 변경 시 reset.
- **[Gemini High]** `search_response.v1` 에 `hint: Option<String>` additive 필드 추가 (A5 Step 4b) — `--json`/MCP 가시성 보강. CLI stderr 안내와 보완적.
- **[Codex Medium]** PR-B helper 이름 명시 — `stale_chunk_ids_for_workspace_path_except_doc_id` + `purge_document_at_workspace_path_except_doc_id` 새 helper. P7-3 helper 의 asset_id 조건 우회.
- **[Codex Medium]** raw FTS mode 표기 single quote `'...'` 로 통일 (A1 Step 1, A5 Step 1, A5 Step 2 권장안 1) — 실제 코드 `lexical.rs:167` 기준.
- **[Codex Medium]** short-query CLI 조건을 `query.trim().chars().count() < 3` 으로 고정 — "모든 토큰 < 3" 분기 제거 (valid whole-phrase query false trigger 회피). TUI 도 동일.
- **[Gemini Medium]** A4 Step 0 — `fixtures/search/korean/` 으로 한국어 도그푸딩 fixture 복사·commit, LICENSE 표기.
- **[Gemini Medium]** A6 Step 4b — `integrations/claude-code/kebab/SKILL.md` 에 3자 권장 + hint 필드 안내 한 줄.
- **[Gemini Low]** README 디스크 용량 수치화 (~2-5배 또는 수백 MB 단위).
