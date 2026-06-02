# Spec: doc-side expansion(별칭) 기능 제거

**날짜**: 2026-06-03
**유형**: 기능 제거 (refactor/removal)
**근거**: `docs/superpowers/research/2026-06-03-expansion-cost-rethink-research.md` (Step 0/1 측정 + 딥리서치). 별칭 ROI 음수: cross-lingual 은 e5-large 단독으로 이미 완벽, 별칭 기여는 설명형 +2 그룹뿐인데 대가가 청크당 색인-시 LLM(살아있는 KB 에 지속 불가). 문헌(arXiv 2309.08541)도 "강한 검색기엔 expansion 해롭다" 확인.
**design contract 영향**: design §(Phase 2 doc-side expansion) 에서 도입된 기능 제거 → `tasks/HOTFIXES.md` dated entry + 원 spec(`docs/superpowers/specs/2026-05-30-doc-side-expansion-design.md`)의 Risks/notes 에 제거 cross-link 1줄. design 본문은 별도 spec PR 이 아닌 본 PR 에서 "deprecated/removed" 주석만.

## 목표
색인-시 청크당 LLM 별칭 생성 + 별칭 검색 경로를 **완전히 제거**한다. 기본 동작 불변(별칭은 이미 default-off)이라 일반 사용자 체감 0. 코드/스키마/wire 표면을 정리해 유지보수 부담을 없앤다.

## 제거 대상 (REMOVE)
- `crates/kebab-app/src/expansion.rs` — 모듈 전체 (ExpansionGenerator, is_nav_boilerplate, parse_aliases, strip_list_marker).
- `crates/kebab-app/src/lib.rs` — `pub mod expansion;`, ingest_one_asset 의 expansion 루프(별칭 생성·캐시 조회/저장·`alias_version_key`·`embed_aliases` 임베딩·alias sentinel 벡터 `{orig}#alias#N`), 관련 카운터(`alias_cache_hit/miss`, `alias_touch_keys`).
- `crates/kebab-config/src/lib.rs` — `ExpansionCfg` 구조체 + `IngestCfg.expansion` 필드 + 기본값.
- `crates/kebab-config/src/migrate.rs` — `[ingest.expansion]` 섹션 주석/마이그레이션 처리.
- `crates/kebab-core/src/chunk.rs` — `Chunk.aliases: Option<String>` 필드 (+ 관련 serde default 테스트). **주의: `crates/kebab-core/src/metadata.rs` 의 `Metadata.aliases: Vec<String>` 는 문서 메타데이터(§3.6)로 무관 — 유지.**
- `crates/kebab-search/src/lexical.rs` — `run_alias_query`, `merge_body_alias`, alias FTS 분기(`build_match_string_for_column(.., "aliases")`).
- `crates/kebab-store-sqlite` — `chunk_aliases_fts` 테이블 + 트리거 + `chunks.aliases` 컬럼: **신규 forward 마이그레이션(V0XX)으로 DROP**. INSERT/SELECT 경로(`documents.rs` 의 aliases 컬럼 쓰기/읽기) 제거.
- `crates/kebab-app/src/ingest_progress.rs` — `IngestEvent::ExpansionProgress` variant (+ 직렬화 테스트). **`AssetChunked`/`AssetTimings` 는 유지**(별칭과 무관, 청킹/타이밍 가시성).
- `crates/kebab-cli/src/progress.rs` + `crates/kebab-tui/src/ingest_progress.rs` — ExpansionProgress 렌더(`별칭 확장 N/chunks`).
- `crates/kebab-tui/src/inspect.rs` — chunk 별칭 표시(있으면).
- derivation_cache 의 `"alias"` kind: 쓰기 경로 제거. 기존 행은 무해(읽지 않음), `kebab reset` 시 정리. kind enum 에서 alias 제거는 선택(read 호환 위해 남겨도 무방).

## 유지 (KEEP — 제거 금지)
- `Metadata.aliases` (문서 메타데이터, metadata.rs).
- `AssetChunked`, `AssetTimings` wire 이벤트 + 렌더.
- derivation_cache 의 `embedding` kind (V012 임베딩 캐시 — 별칭과 독립, 성능 핵심).
- `chunks_fts`(본문 FTS) 전부.
- `Chunk` 구조체를 생성하는 모든 곳(kebab-chunk/*, kebab-parse-*/*): `aliases: None` 리터럴은 필드 제거에 맞춰 **삭제만**(기능 변경 아님).

## 결정 사항
- **마이그레이션**: 신규 forward-only 마이그레이션으로 `chunk_aliases_fts`(+ 트리거)와 `chunks.aliases` 컬럼 DROP. SQLite 3.35+ `DROP COLUMN` 사용(번들 sqlite 확인). down 마이그레이션 불필요(refinery forward-only 관행 따름). 기존 KB: 별칭 default-off 라 대부분 빈 데이터 → 손실 없음. corpus_revision cascade 불필요(별칭은 검색 보조였을 뿐, 본문/임베딩 불변).
- **wire schema**: `ingest_progress.v1` 에서 `expansion_progress` kind 제거. v0.24.0 에서 막 추가된 additive variant 라 소비자(agent/CLI)는 부재 허용 → major bump 불요. `docs/wire-schema/v1/ingest_progress.schema.json` 에서 해당 kind 정의 삭제 + 주석.
- **버전**: workspace `version` patch/minor bump(별칭 제거 = surface 정리, breaking schema 아님 — 단 chunk_aliases_fts DROP 마이그레이션 포함이라 이전 binary 가 새 DB 열 때 영향 없음(컬럼 제거는 구 binary 의 SELECT 깨뜨릴 수 있으나 단일 사용자·forward-only 전제). minor bump 권장.
- **config**: `[ingest.expansion]` 제거 후 기존 사용자 config.toml 에 해당 섹션이 있어도 serde forward-compat(unknown field ignore)로 무해. `kebab config migrate` 가 섹션 제거하도록 갱신(선택).

## 문서 동기화 (같은 PR)
- `tasks/HOTFIXES.md`: dated entry — 제거 근거(연구 링크) + 마이그레이션 + wire 변경.
- `docs/superpowers/specs/2026-05-30-doc-side-expansion-design.md`: Risks/notes 에 "2026-06-03 제거됨, 본 spec 참조" 1줄.
- `README.md` / `HANDOFF.md`: 별칭이 README 에 노출돼 있으면 제거(default-off 라 노출 없을 가능성). HANDOFF 한 줄.
- `docs/wire-schema/v1/ingest_progress.schema.json`: expansion_progress 제거.
- design 본문(frozen contract)에 Phase 2 별칭 기술이 있으면 "removed (HOTFIXES 2026-06-03)" 주석.

## 검증 기준 (Acceptance)
- `cargo clippy --workspace --all-targets -j 4 -- -D warnings` 통과.
- `cargo test --workspace --no-fail-fast -j 1` 통과 — 별칭 전용 테스트(`tests/chunk_aliases.rs`, expansion.rs 테스트, lexical alias 테스트)는 삭제, 그 외 회귀 0.
- 신규 마이그레이션 적용된 fresh KB 에 `chunk_aliases_fts`/`chunks.aliases` 부재 확인(`.schema`).
- `kebab ingest`(별칭 config 없이) 정상 — AssetChunked/AssetTimings 진행 표시 유지, expansion_progress 미출력.
- 기존 별칭 데이터가 있던 KB 도 마이그레이션 후 search/ask 정상(별칭 벡터는 무시/정리).
- grep 잔존 0: `expansion::|ExpansionCfg|chunk_aliases|run_alias_query|merge_body_alias|ExpansionProgress|embed_aliases|is_nav_boilerplate|Chunk.*aliases`.

## 비범위 (out of scope)
- 별칭 대체 방법(heading enrichment / arctic-ko 임베더 / reranker / query-side) — 후속 별 작업(연구문서 §7 Layer A~D).
- `Metadata.aliases`(문서 메타) 변경.
- derivation_cache GC wiring.
