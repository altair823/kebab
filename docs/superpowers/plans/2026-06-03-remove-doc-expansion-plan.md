# Plan: doc-side expansion(별칭) 제거 구현

spec: `docs/superpowers/specs/2026-06-03-remove-doc-expansion-spec.md`. 브랜치 `refactor/remove-doc-expansion`. 빌드 `CARGO_TARGET_DIR=/build/out/cargo-target`, 직렬 `-j 4`(전체 테스트는 `-j 1`).

원칙: 작은 단위로 컴파일 가능 상태 유지. 각 단계 후 `cargo build -p <crate> -j 4`. 최종 clippy+test.

## Task 1 — kebab-core: Chunk.aliases 필드 제거
- `chunk.rs`: `pub aliases: Option<String>` + serde default + `aliases_defaults_to_none_on_deserialize` 테스트 제거.
- **금지**: `metadata.rs` `Metadata.aliases`(Vec) 는 손대지 않음.
- 컴파일 깨짐 → Task 2~ 에서 Chunk 리터럴 정리하며 해소.

## Task 2 — Chunk 리터럴 정리 (kebab-chunk/*, kebab-parse-*/*, store-sqlite, app)
- `grep -rn "aliases: None" crates/*/src` 로 Chunk 생성부 전수 → `aliases: None,` 줄 삭제.
- store-sqlite `documents.rs`: chunks INSERT 컬럼리스트/바인딩에서 `aliases` 제거(line ~126/156), SELECT 매핑의 aliases 제거, `aliases: None`(271) 제거.

## Task 3 — kebab-app: expansion 모듈 + 루프 제거
- `lib.rs`: `pub mod expansion;` 삭제, `expansion.rs` 파일 삭제.
- ingest_one_asset: expansion 블록 전체(`if app.config.ingest.expansion.enabled { … }` + `alias_version_key`/`alias_cache_*`/`alias_touch_keys`/embed_aliases 임베딩/sentinel 벡터 생성) 제거. `expansion_ms` 타이밍은 0 고정 또는 AssetTimings 에서 필드 유지하되 항상 0 — **AssetTimings 의 expansion_ms 필드는 유지(wire 호환)**, 값 0.
- alias sentinel 벡터 upsert 경로 제거, purge_vector_orphans 는 본문 벡터 정리로 유지.

## Task 4 — kebab-config: ExpansionCfg 제거
- `lib.rs`: `ExpansionCfg` struct + `IngestCfg.expansion` 필드 + Default 제거.
- `migrate.rs`: `[ingest.expansion]` 처리/주석 제거.
- config 직렬화 테스트에서 expansion 기대 제거.

## Task 5 — kebab-search: alias lexical arm 제거
- `lexical.rs`: `run_alias_query`, `merge_body_alias`, alias 분기 제거. body_rows 직접 사용으로 단순화. alias 관련 테스트 제거/갱신.

## Task 6 — wire/progress 정리
- `kebab-app/ingest_progress.rs`: `IngestEvent::ExpansionProgress` variant + 직렬화 테스트 제거. AssetChunked/AssetTimings 유지.
- `kebab-cli/progress.rs`, `kebab-tui/ingest_progress.rs`: ExpansionProgress 매치/렌더 제거.
- `kebab-tui/inspect.rs`: 별칭 표시 제거.
- `docs/wire-schema/v1/ingest_progress.schema.json`: expansion_progress kind 제거.

## Task 7 — sqlite 마이그레이션: DROP chunk_aliases_fts + chunks.aliases
- `schema.rs`(refinery 마이그레이션 등록부) 확인 → 신규 forward 마이그레이션 추가: `DROP TABLE IF EXISTS chunk_aliases_fts` (+ 관련 트리거/shadow 테이블 chunk_aliases_fts_*), `ALTER TABLE chunks DROP COLUMN aliases`.
- chunk_aliases_fts 를 만들던 기존 마이그레이션은 **수정 금지**(과거 마이그레이션 freeze) — 새 마이그레이션으로 덮어 제거.
- `tests/chunk_aliases.rs` 삭제. `tests/migration.rs` 신규 마이그레이션 반영.
- 번들 sqlite DROP COLUMN 지원 확인(3.35+); 미지원이면 테이블 재생성 패턴.

## Task 8 — 검증 + 문서
- `cargo clippy --workspace --all-targets -j 4 -- -D warnings`.
- `cargo test --workspace --no-fail-fast -j 1`.
- fresh KB `.schema` 로 chunk_aliases_fts/aliases 부재 확인. `kebab ingest` 스모크(별칭 config 없이).
- grep 잔존 0 (spec Acceptance 의 정규식).
- 문서: HOTFIXES dated entry, 2026-05-30 doc-expansion-design spec Risks/notes cross-link, HANDOFF 1줄, wire schema, design 본문 removed 주석, Cargo.toml version bump.

## 리뷰 루프
구현 완료 → `gitea-pr`(title `refactor(app): doc-side expansion(별칭) 제거`, body 요약/검증) → gitea-pr 리뷰 루프(actionable 해소까지) → 사용자 머지.
