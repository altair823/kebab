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

## 2026-06-24 — pdf-page-v1.2: PDF 페이지 oversize 분할 + 공유 `crate::oversize` 모듈

**무엇을 바꿨나.** md-heading-v2 의 oversize 분할 primitive(`text_pieces`/
`char_pieces`/`BYTES_PER_TOKEN`)를 `crates/kebab-chunk/src/oversize.rs` 공유 모듈로
추출하고, **PDF 청커를 `pdf-page-v1.1` → `pdf-page-v1.2`** 로 올려 같은 분할을
적용했다. v1.1 의 `chunk_page` 는 문장/문단 경계로만 잘라서, 경계 없는 거대
페이지(빽빽한 scanned page 가 한 줄로 OCR 된 경우)가 통째로 한 청크 → strict
임베더에서 실패하는 hole 이 PDF 에 잔존했다(md 와 동형). v1.2 는 **2-tier**:
tier-1(문장/문단 greedy + overlap) 후, segment 가 `max_chunk_tokens`(공유 config,
default 4000) 초과면 tier-2 가 `text_pieces` 로 재분할 → 모든 PDF 청크 ≤ 예산.

**구현.** `PdfPageV1Chunker { max_chunk_tokens }`(이전 unit struct), `policy_hash`
budget fold(md 와 동일), `pdf_chunker_from_config`(kebab-app)로 config 주입(신규
config 키 없음). 분할 조각 chunk_id 는 `#c{segment_start}s{i}`(미분할 단일 segment
는 bare `#c{segment_start}` 유지 → 공통 경우 hash 컴포넌트 v1.1 동일). md-heading-v2
는 공유 모듈을 호출만 하고 **출력 byte-identical**(md 라벨·동작 불변, parity 테스트
전부 통과).

**span 버그 발견·수정(코드 리뷰).** 최초 구현은 분할 조각 Page span 의
char_start/char_end 를 per-piece 로 정밀 narrow 하려 했으나, `text_pieces` 가 줄
분할 시 piece 사이 `'\n'` 구분자를 소실시켜 running offset 이 줄 경계마다 1씩
drift(`"aaaa\nbbbb\ncccc"` → drift 2). `Vec<String>` 만으로 복구 불가 →
**md 와 동일하게 부모 segment span(`char_start..seg_char_end`)을 모든 sub-piece 에
적용**(segment-granular, 절대 drift 없음, never wrong). 회귀 테스트
`oversize_pdf_page_with_newlines_splits_without_span_drift` 로 잠금.

**cascade.** `chunker_version` v1.1→v1.2 → 다음 plain `kebab ingest` 에서 PDF 자산
1회 자동 재청크(markdown/code 무영향). wire/CLI/포맷 불변(검색 hit 의 거대 PDF
페이지가 여러 hit 로 나뉠 수 있음).

**도그푸딩 evidence**(실험 KB, scanned PDF + paddle-onnx OCR + arctic@Lemonade,
budget 200 으로 tier-2 강제). 625/625 errors=0. scanned_page1.pdf 1→**3 청크**
(max 190 ≤200), scanned_page2.pdf 3→**7 청크**(max 197 ≤200), 둘 다
`chunker_version=pdf-page-v1.2` 스탬프. 전 코퍼스(markdown+이미지OCR+PDF) **10215
청크 전부 ≤200, 초과 0**. 단위: kebab-chunk lib 93 pass(span 회귀 테스트 포함),
kebab-app green, clippy `-D warnings` 0. 설계:
`docs/superpowers/plans/2026-06-24-pdf-page-v1.2-oversize-split.md`.

## 2026-06-24 — config: `[ingest.chunking]` budget floor 검증

**무엇을 바꿨나.** `Config::from_file` 에 `validate_chunking()` 을 추가해
청킹 budget 의 명백히 깨진 조합을 **load 시점에 reject**(`validate_sources`
와 동일 패턴, `ConfigInvalid`). 규칙: `target_tokens ≥ 16`(`MIN_CHUNK_TOKENS`),
`overlap_tokens < target_tokens`, `max_chunk_tokens ≥ target_tokens`.

**왜.** md-heading-v2(PR #209) 의 `max_chunk_tokens` 는 검증이 없어, `0` 같은
오설정이 청커 내부 `budget.max(1)` 클램프에 흡수돼 **3-byte 청크 폭주**(인덱스
bloat, 에러 없음)를 냈다. 기존 `target_tokens`/`overlap_tokens` 도 동일하게
미검증이었다(reviewer 지적). 세 필드를 한 번에 floor + 상호 제약으로 막는다.

**영향.** valid config 는 무영향(동작·결과 불변). 깨진 config(예: `max_chunk
< target`, `overlap ≥ target`)만 명확한 메시지로 load 실패. 새 config 키·migration
없음, 동작 변경 없음 → patch-level. 테스트: `defaults_pass_chunking_validation`
+ reject 4종 + `from_file_rejects_invalid_chunking`(e2e).


## 2026-06-24 — md-heading-v2: 예산 초과 청크 일반 분할 (oversize-chunk split) (v0.30.0)

**무엇을 바꿨나.** markdown 청커에 새 변종 `md-heading-v2` 를 추가하고
기본값으로 승격했다. v1 의 규칙 2("코드/테이블 블록은 `target_tokens` 를
넘어도 절대 분할하지 않는다")는 **모든** 블록 종류로 일반화된 한계였다 — 하나의
거대 블록(코드뿐 아니라 list·table·paragraph 도)이 통째로 한 청크가 되어
임베더 컨텍스트를 초과할 수 있었다. v2 는 v1 과 **모든 출력이 동일**하되,
마지막에 청크의 **실제 임베드 text 크기**(`text.len()/3`)가 `max_chunk_tokens`
를 넘는 청크만 줄(`\n`) 경계로, 단일 거대 줄은 UTF-8 char 경계로 잘라 **각 조각이
예산 이하**가 되도록 분할한다. (판정 기준이 저장 `token_estimate` 가 **아니라**
실제 text 길이인 이유: ImageRef/AudioRef 청크는 image-only 규약으로
`token_estimate=0` 인데 그 text(alt+OCR+caption)는 거대할 수 있다 — 빽빽한
스크린샷 OCR 이 대표 사례. 도그푸딩 일치 재테스트에서 발견·수정.)
신규 config `[ingest.chunking] max_chunk_tokens` (byte/3 토큰, default **4000**).
분할 조각의 chunk_id 는 동일 `block_ids` 를 공유하므로 id-input 해시에
`#seg{i}` 접미사를 붙여 충돌을 막는다(저장 `policy_hash` 는 bare — pdf-page-v1
의 `#L` 레시피와 동형). `max_chunk_tokens` 는 v2 의 `policy_hash` 에
folding 되어(공유 `ChunkPolicy` 는 미변경 → 코드/PDF 청커 cascade 무영향)
값을 바꾸면 markdown 만 재청크된다.

**왜 — strict 임베더는 oversize 입력을 truncate 가 아니라 거부한다.** 도그푸딩
도중 jira 이슈 일부가 임베드에 실패했다. 원인은 긴 MongoDB 로그/스택트레이스가
md 변환 시 **하나의 거대 `list` 블록**(예: SERVER-22906 = 76189 토큰 / 소스
60–1303 줄)으로 렌더된 것. 기존 ollama(`/api/embed`)는 이런 입력을 조용히
서버측 8192 로 **truncate** 해서 0 errors 였지만(=사실상 잘려 색인됨), AMD
Lemonade 같은 strict 백엔드는 `500 "input (N tokens) is too large ... increase
the physical batch size"` 로 **거부**한다. 임베더-무관하게 견고하려면 청커가
애초에 예산 초과 청크를 만들지 않는 게 옳다. (전역 truncate 를 임베더 측에
넣는 대안은 silently-잘림이라 reject.)

**cascade / 업그레이드.** `chunker_version` 라벨이 `md-heading-v1` → `md-heading-v2`
로 바뀌므로, **다음 plain `kebab ingest` 에서 markdown 자산이 1회 자동
재청크**된다(`--force-reingest` 불필요 — skip 비교가 mismatch). 코드/PDF 자산은
각자 chunker_version 이 unchanged 라 영향 없음. wire / CLI / `--json` 포맷
불변(검색 hit 의 텍스트·citation 모양 동일, 단 거대 블록이 여러 hit 로 나뉠 수
있음). **알아둘 wrinkle**: markdown 청커 dispatch 는 하드코딩이고 config
`chunker_version` 문자열은 impl 선택에 쓰이지 않는다 → 기존 config 가
`chunker_version = "md-heading-v1"` 로 핀돼 있어도 실제로는 v2 가 돈다(문자열은
정보성). 새 default config 와 `kebab config migrate` 는 `max_chunk_tokens` 를
additive 로 주입한다.

**citation 정밀도(known limitation).** 분할 조각은 원 블록의 `source_spans`
(블록 전체 범위)를 그대로 갖는다 — 즉 거대 블록을 쪼갠 조각의 인용은
**블록 단위**(sub-line 정밀 아님)다. fenced 코드의 `SourceSpan::Line` 이 fence
줄을 포함하는 비대칭 때문에 조각별 줄 범위를 정확히 좁히는 건 (span,code) 만으로
일반적으로 불가능 → "절대 틀리지 않되 블록 단위" 를 택했다. 일반 청크(미분할)는
v1 과 byte-identical 이라 영향 없음.

**도그푸딩 evidence** (실험 KB `/home/user/large_data/out/kebab-ab/xdg_sources`,
arctic-embed-l-v2 @ Lemonade `.243`). v2 전: 620 중 2 doc(SERVER-22906/23097)
임베드 실패. v2 후: **620/620, errors=0**, 7114 청크 전부 ≤ 4000(초과 0).
SERVER-22906 → 14 청크(max 3897), SERVER-23097 → 5 청크(max 3850). 검색 payoff:
"WiredTiger excessive memory cache size" 질의에 SERVER-22906 가 **1위(0.977)** —
"임베드 불가" 에서 "최상위 검색 결과" 로. `--trust-min primary` 등 출처 필터
정상 유지.

**일치 재테스트 (사용자 실 config 재현 — 이미지 OCR + PDF OCR ON, paddle-onnx)**.
초기 검증은 markdown-only 였으나 사용자 실 config 가 image/pdf OCR ON 임을 반영해
미디어(이미지 2 + scanned PDF 2)를 같은 paddle-onnx + arctic@Lemonade 로 재인덱싱.
**여기서 image-OCR 구멍 발견·수정**: 분할 판정을 `token_estimate` 로 하면
ImageRef 청크는 `token_estimate=0`(image-only) 이라 OCR text 가 거대해도 분할이
안 됨 → 빽빽한 스크린샷 OCR 이 임베더 ctx 초과 시 strict 백엔드에서 실패 가능.
판정을 실제 text 길이로 교정 + 회귀 테스트(`oversize_image_ocr_chunk_splits`).
검증: scanned_page1/2.pdf OCR→청크(pdf-page-v1.1)→arctic 임베드 정상(max tok
443/672), 624 자산 errors=0. **known limitation**: PDF 는 별도 청커 `pdf-page-v1.1`
이라 이 oversize-split 미적용 — 초고밀도 scanned page 가 한 청크로 budget 초과 시
잔존(현 fixture 는 무관). md 청커(이미지 OCR text 포함)만 v2 가 커버. 후속 후보:
pdf-page 청커에도 동일 oversize-split.

p1-5(md-heading-v1) 를 확장하며 frozen 설계 doc / frozen p1-5 spec 은
미변경(설계 §9 가 `md-heading-v2` 라벨 bump 를 이미 변경 메커니즘으로 명시 —
pdf-page-v1→v1.1 선례 동형). 설계: `docs/superpowers/plans/2026-06-24-md-heading-v2-oversize-split.md`.

## 2026-06-21 — provenance 출처 필터: `[[workspace.sources]]` 멀티소스 + `--source` / `--source-type` (v0.29.0)

**무엇을 추가했나.** 혼합 출처 KB(예: 위키 문서 + jira 이슈)에서 "출처별로
검색을 좁히는" provenance 레버를 두 층으로 붙였다. 검색에 `--source-type`
(Phase-2) 와 `--source <id>` (Phase-3) 필터, config 에 `[[workspace.sources]]`
명명 멀티소스 선언, 저장소에 `documents.source_id` 컬럼(V014)을 추가했다.
기존 단일-root 사용자는 **아무것도 손대지 않아도 된다** — load 시 단일 `root`
가 implicit `default` source 로 정규화되고, V014 는 additive(컬럼 DEFAULT
`'default'`)라 재색인이 발생하지 않는다.

**왜 필터인가 — 전역 trust 가중(weighted-RRF)은 반증됨.** Phase-1 통제 실험
(`/home/user/large_data/out/kebab-ab/`, MongoDB 도메인 정합 corpus)에서 jira 를
docs KB 에 섞으면 **개념 질의는 약하게 오염**(concept ΔMRR −0.072,
CI[−0.159,−0.004]; top-3 정답은 유지, rank1→2 강등)되지만 **운영/이슈 질의는
크게 개선**(incident ΔMRR +0.972, jira_only hit@10 0/10 → 10/10)됨을 측정했다.
Phase-2 에서 골든셋을 142 쿼리로 확장(`golden_v2.json`: 워크플로 12토픽 병렬생성
96 + 원본 46)해 재현(concept 70 −0.03 유의, incident 66 +0.92~0.97)한 뒤, θ
sweep 시뮬(`eval_phase2.py`)로 **전역 trust 곱셈가중을 반증**했다 — jira 에
θ=0.85 만 곱해도 RAG 점수 압축 때문에 incident MRR 0.918→0.340 으로 절벽 하락.
작은 오염을 잡으려다 큰 개선을 버리는 see-saw 라 **빌드하지 않았다**. 올바른
레버는 see-saw 없는 **출처 필터링**: 색인은 전부 하되 질의 시 출처로 좁힌다.

**구현 표면.**

- **config 스키마** (`kebab-config`): `WorkspaceCfg.root` 가 `Option<String>` 으로,
  신규 `WorkspaceCfg.sources: Vec<SourceCfg>` 추가. `SourceCfg { id, root,
  exclude?, trust_level?, source_type? }`. `Config::resolved_sources()` 가
  단일 entry point — `sources` 가 비면 `workspace.root` 를 implicit `default`
  source 로 합성, 있으면 각 entry 의 root 확장 + `workspace.exclude` ∪ per-source
  exclude. `validate_sources()` 가 id 비어있음/중복을 `ConfigInvalid` 로 거절.
- **config v3→v4 migration** (`migrate.rs::step_3_to_4`): 단일 `workspace.root`
  를 `[[workspace.sources]]` id=default 로 **미러**(기존 root 키는 보존 — 둘 다
  default 를 가리켜 무해). `[[workspace.sources]]` 가 이미 있으면 no-op. 멱등.
  `CURRENT_SCHEMA_VERSION` 3→4. load 시 메모리 자동 변환 + `kebab config migrate`
  로 디스크 갱신(값·주석 보존) — v0.28.0 v2→v3 패턴 동일.
- **저장소** (V014 `documents_source_id.sql`): `documents.source_id TEXT NOT NULL
  DEFAULT 'default'` + `idx_docs_source_id`. additive — 기존 row 는 DEFAULT 로
  `'default'`, 재색인/`corpus_revision` bump 불요. DEFAULT 리터럴은
  `kebab_config::DEFAULT_SOURCE_ID` 와 동기.
- **도메인/파서**: `Metadata.source_id: Option<String>` 추가(`skip_serializing_if
  = Option::is_none`). `BodyHints` 에 `source_id` + `fallback_trust_level` 추가 —
  markdown derive 의 trust precedence 가 **frontmatter > per-source 기본값 >
  하드코딩 Primary**. source_id 는 frontmatter 가 덮지 않는 ingest-time
  provenance stamp.
- **ingest** (`kebab-app`): `--root` 미지정 시 `resolved_sources()` 를 순회하며
  각 source 를 own root+exclude 로 스캔하고 asset→source_id 매핑을 만든 뒤 doc
  마다 source_id + source 기본 trust 를 stamp. `--root`/single-file/include 지정
  시는 ad-hoc `default` source 한 개(기존 동작 보존). `FsScanSkips::merge` 로
  멀티소스 스킵 집계.
- **검색 필터**: `SearchFilters` 에 `source_type: Vec<String>` + `source_id:
  Vec<String>`(빈 vec = 무필터, multi-value = OR). lexical(FTS5
  `kebab-search/lexical.rs`)·vector(`kebab-store-sqlite/filters.rs`) **두 site
  모두** `d.source_type IN (...)` / `d.source_id IN (...)` 직접 인덱스 컬럼 필터.
  CLI `kebab search --source-type <TYPE>` + `--source <ID>`(repeatable/comma-sep).

**검증(도그푸딩, v0.29.0 release 빌드, 실험 corpus xdg_sources KB).** ingest
620 문서 / 0 error, `source_id = {jira: 400, wiki: 220}`. **trust precedence
실측**: jira source 기본값 secondary(frontmatter 없어도) → `--trust-min primary`
시 jira 0/6 노출, wiki primary 유지. **출처 필터 실측**: `--source wiki` → 개념
질의 MRR 0.780→0.810 (KB_wiki 수준 오염 회복), `--source jira` → incident
0.918→0.975. Phase-2 `--source-type reference`/`markdown` 도 동일 효과(concept
0.810, incident 0.975). weighted-RRF 절벽과 대비해 필터는 see-saw 없음.

**Known limitations / follow-up.**

- **MCP search 도구 미노출**: `kebab-mcp/tools/search.rs` 는 `source_type` /
  `source_id` 를 빈 vec 로 채워 컴파일만 맞춤 — agent 가 MCP 로 출처 필터를 못
  건다. `SearchInput` 에 두 필드 추가가 다음 additive 후보.
- **`kebab list` / `doc_summary.v1` 에 source_id 미노출**: `documents.source_id`
  는 stamp/필터되지만 list 출력(`DocSummary`)에는 안 실린다 — 사용자가 "이 문서가
  어느 source 냐" 를 list 로 못 본다. additive 후보.
- **RAG provenance 라벨 미구현**: `kebab ask` citation 에 source 라벨 없음. 검색은
  필터 가능하나 답변 근거의 출처 표기는 다음 단계.
- **구현 교훈**: `Metadata`(Default 없음 · 60+ 곳에서 literal 구성)에 필수 필드
  추가는 churn 이 큼(이 PR 의 90+ 파일 대부분이 `source_id: None` 추가). 차라리
  store 레이어에서 stamp 하면 저-churn — 다음 리팩터 후보.

관련 메모리: jira-contamination-ab-experiment(Phase-1/2/3 측정), jira-wiki-dogfood-kb.

## 2026-06-04 — config 스키마 v2→v3 재편: 미디어 ingest 통합 (v0.28.0)

**무엇을 바꿨나.** `config.toml` 의 미디어 형식 설정을 `[ingest.*]` 우산 아래로 통합했다. 첫 non-additive rename 마이그레이션.

rename 매핑:

| v2 (top-level) | v3 (`[ingest.*]`) |
|---|---|
| `[indexing]` (스칼라 키) | `[ingest]` 스칼라 (`max_parallel_extractors`/`max_parallel_embeddings`/`watch_filesystem`) |
| `[chunking]` | `[ingest.chunking]` |
| `[image.ocr]` | `[ingest.image.ocr]` |
| `[image.caption]` | `[ingest.image.caption]` |
| `[pdf.ocr]` | `[ingest.pdf.ocr]` |
| `[ingest.code]` | `[ingest.code]` (불변) |

**보장한 3가지 불변식.**

1. **signature 바이트 불변** — `ingest_config_signature` 출력은 값 기반이라 struct 경로 재편 후에도 v2 와 바이트 동일. 업그레이드 시 전체 재색인 발생 안 함. paddle 경로(det/rec/dict)는 미디어별로 호출자가 넘기도록 인자화(`ocr_engine_version_for_sig` + `engine_version_for_paths`); v2 의 "pdf 가 image paddle 을 빌려쓰던" 비대칭은 `step_2_to_3` 의 값 복사(`copy_image_paddle_to_pdf`)로 보존.
2. **env override 이름 100% 보존** — `apply_env` whitelist 의 키 문자열(LHS, 예 `KEBAB_CHUNKING_TARGET_TOKENS`/`KEBAB_INDEXING_MAX_PARALLEL_EXTRACTORS`)은 불변, 대입 대상(RHS)만 `self.ingest.*` 로. 기존 `KEBAB_*` 스크립트 무파손. 신규 `KEBAB_PDF_OCR_{DET_MODEL,REC_MODEL,DICT,SCORE_THRESH,UNCLIP_RATIO,MAX_BOXES}` 6키 추가(image.ocr paddle 대칭).
3. **load 시 메모리 내 자동 변환** — `Config::from_file` 이 `schema_version < 3` 파일을 디스크 미변경으로 메모리에서 v3 변환(`migrate_document` 경유). 미변환 v2 파일도 설정 유실 0. 파일 갱신은 `kebab config migrate` (값·주석·대안 줄 보존, 멱등).

`PdfOcrCfg` 에 paddle 대칭 6키 추가. `ser_f32_clean` 으로 f32 직렬화 정리(`0.30000001192092896`→`0.3`). per-option 인라인 주석(`key_comment`)을 init/migrate 산출 config 에 부착.

**도그푸딩 (v0.28.0 release 빌드).**

1. **사용자 실제 v2 config 변환** (`/build/dogfood/config-v3-test/`): `kebab config migrate` → `v2 → v3 (11 changes)`, `.bak` 백업. schema_version 2→3, 섹션 헤더만 [ingest.*] 로 이동(`[indexing]`/`[chunking]`/`[image.ocr]`/`[image.caption]`/`[pdf.ocr]` → `[ingest.*]`). 사용자 값 보존(`root`, `model = "snowflake-arctic-embed2"`, `endpoint = "http://192.168.0.2:11943"`, `score_gate = 0.30000001192092896` 그대로) + 대안 주석 보존(`# engine = "ollama-vision"`, `# provider = "candle"`). 재실행 멱등(`config 이미 최신입니다`).

2. **재색인 0 실증** (`/build/dogfood/config-v3-reindex/`, lexical-only `provider = "none"`): v2 config(디스크 schema_version=2)로 first ingest `new=2`. (a) 동일 v2 config 재ingest(메모리 자동변환) → `new=0 updated=0 unchanged=2`. (b) 디스크 파일을 v3 로 `config migrate` 후 재ingest → `new=0 updated=0 unchanged=2`. signature 바이트 불변이 실제 업그레이드 경로(v2 자동변환 ↔ v3 디스크)에서 재색인 0 으로 확인됨 — 불변식 #1 검증.

## 2026-06-04 — PP-OCRv5 ONNX Rust 네이티브 OCR 엔진 (v0.27.0)

**무엇을 추가했나.** 이미지 OCR 에 두 번째 백엔드 `paddle-onnx` 를 붙였다. 기존 `ollama-vision`
(원격 vision LM, 이미지당 ~50초)은 default 로 유지하고, `[image.ocr] engine = "paddle-onnx"` 로
PP-OCRv5(검출 DBNet + 인식 CTC) ONNX 모델을 `ort`(=2.0.0-rc.9) 로 **in-process** 실행한다 —
Python 런타임/원격 호출 없이 큰 페이지 CPU <4초. `OcrEngine` trait 의 두 번째 구현
`OnnxPaddleOcr`(`crates/kebab-parse-image/src/paddle_onnx.rs`), 팩토리는
`kebab-app::build_image_ocr_engine`/`build_pdf_ocr_engine` (`match engine`). 검출 후처리
(min-area rect = rotating calipers, unclip = polygon offset)는 clipper2/OpenCV 없이 pure-Rust.

**T11 e2e 에서 발견·수정한 핵심 버그 (unclip).** 첫 실측 CER 이 0.26(게이트 0.05) 으로 크게
초과. 단계 골든(`crates/kebab-parse-image/tests/golden/`) 와 prediction dump 로 국소화한 결과
`unclip_rect` 가 corner 를 centroid 기준 **방사(radial) 확장**하고 있었다. 텍스트 박스는
wide/short(예 586×15)라 대각선이 거의 수평 → 방사 확장 시 corner 가 수평으로만 ~11px 움직이고
**세로로는 거의 안 커져** 글자 윗/아랫부분이 잘렸다(ㄷ→ㄴ 로 `다`→`나`, ascender 손실).
PaddleOCR pyclipper 처럼 **edge 별로 바깥으로 offset**(width·height 각각 2·distance 증가) 하도록
rect 자체 (u,v) 축 기준 확장으로 재작성. 결과: mean gate CER **0.2585 → 0.0049**
(clean_paragraph/korean_heavy/numbers_table/tech_terms = 0.0), PoC 0.024 baseline 보다 우수.
큰 페이지 3.9초 < 5초 게이트. **교훈**: 회전 사각형 unclip 은 방사 확장이 아니라 polygon edge
offset 이어야 한다.

**Config / 서명 cascade.** `[image.ocr]` 에 `det_model`/`rec_model`/`dict`(Option, override) +
`score_thresh`(0.3)/`unclip_ratio`(1.5)/`max_boxes`(1000) serde-default 필드 + `KEBAB_IMAGE_OCR_*`
env 추가(기존 config 무수정 로드 — forward-compat). `ingest_config_signature` 의 image/pdf 브랜치를
`|ocr:1:{model}` → `|ocr:1:{engine}:{engine_version}` 로 바꿔 engine 전환(ollama↔paddle) 또는
모델 변경 시 영향 자산 자동 재색인. paddle engine_version 은 모델 3-asset blake3 를 **per-process
1회만** 계산(triple 키 memo) — 자산마다 17MB 재해시 회피.

**모델 배포.** ONNX 2개(det 4.7MB / rec 13MB) + dict + NOTICE 를 `crates/kebab-parse-image/
assets/paddleocr-onnx/` 에 둔다(Git LFS). 테스트는 `KEBAB_IMAGE_OCR_MODEL_DIR`(기본 = 번들 dir)
에서 로드, e2e(`tests/paddle_e2e.rs`)는 모델/fixture 부재 시 깨끗이 skip(CI green). 자세한 설계:
spec/plan `docs/superpowers/{specs,plans}/2026-06-04-rust-native-ocr-*.md`.

## 2026-06-03 — ingest 출력 영향 설정 변경 시 영향 자산 자동 재색인 (v0.26.2)

**무엇이 깨졌나.** `[image.ocr]` / `[image.caption]` 를 off→색인→on 으로 바꿔도 증분
skip(`try_skip_unchanged`, `kebab-app/src/lib.rs`)이 그 이미지를 "Unchanged" 로 건너뛰어
재색인이 안 됐다. 더 일반적으로, skip 판정은 자산 내용(blake3) + `parser_version` +
`chunker_version` + `embedding_version` 만 비교하는데, **ingest 산출물을 바꾸는 다른 설정들**
(청킹 파라미터, OCR/caption, pdf.ocr, `[ingest.code]` 옵션)이 이 셋 중 어디에도 반영되지
않아, 변경해도 재색인이 트리거되지 않았다. 사용자 요구: OCR/caption 뿐 아니라 **ingest 출력에
영향 주는 모든 설정**이 변경되면 영향 자산이 자동 재색인.

**무엇이 바뀌었나 (내부 skip 판정 정정 — 결과 포맷·CLI·wire 불변, patch).**

- 신규 헬퍼 `ingest_config_signature(config, media_type) -> String` — 그 자산 타입의
  **ingest 산출물에 영향 주는 설정만** 결정적으로 직렬화. 공통(전 타입): `[chunking]`
  target_tokens/overlap_tokens/respect_markdown_headings/chunker_version. image: + ocr(enabled,
  +model) + caption(enabled, +prompt_template_version). pdf: + pdf.ocr(enabled||always_on 이면
  enabled/always_on/model). code: + `[ingest.code]` 7개 필드. markdown: 공통만.
- 각 ingest 경로(md/image/pdf/code)의 effective parser_version 을
  `format!("{base}|{signature}")` composite 로 만들어 (a) `try_skip_unchanged` 비교값,
  (b) **persist 전 `canonical.parser_version` override** — 두 값이 같은 함수에서 나오므로
  설정 변경 시 다음 run 비교가 mismatch → 영향 자산만 자동 재색인.
- **doc_id 는 손대지 않음**: base parser_version(extractor 상수)으로 계속 파생 →
  설정 변경에도 doc_id 안정(orphan churn 회피). composite 는 비교/저장 필드에만.
- **제외(재색인 트리거 X)**: search/rag/nli/ui/logging/storage/workspace + 산출 무관
  런타임 파라미터(max_pixels/languages/*_timeout_secs). "그 값이 바뀌면 색인되는
  chunk/embedding 내용이 달라지는가" 기준. 과도 무효화 회피.
- code 의 Tier-3 fallback 문서는 의도적으로 bare `"none-v1"` sentinel 유지(skip 의
  `stored_is_tier3_fallback` bypass 가 정확히 그 문자열에 의존) — composite 는 정상 outcome 에만.

**업그레이드 1회 효과.** 기존 doc 의 저장 parser_version(상수)이 새 composite 와 달라,
업그레이드 후 첫 `kebab ingest` 에서 **전 자산이 현재 설정대로 1회 재색인**된다(force 불필요).
마크다운/코드도 1회 재청킹되나 embedding 은 V012 derived-cache 히트라 재임베딩 비용은 작다.
`--force-reingest` 는 전체 강제용으로 그대로.

**도그푸딩 evidence (release 바이너리, Ollama down — OCR 호출은 Lenient 실패).**
이미지 1장, `[image.ocr] enabled=false` 색인 → New=1. config 에서 `enabled=true` 로 변경 후
`kebab ingest`(force 없이) → **Updated=1**(재색인, errors=0). 동일 config 재실행 → **Unchanged=1**
(불필요 재색인 0). 저장된 parser_version =
`image-meta-v1|chunk:500:80:true:md-heading-v1|ocr:1:gemma4:e4b|cap:0`(base 보존 + OCR on 반영).

**테스트.** `kebab-app/src/lib.rs::ingest_config_signature_tests`(8 단위: 결정성, 청킹=전타입,
이미지 ocr/caption 토글=이미지만, pdf.ocr=pdf만, code 옵션=코드만, search/rag/ui·런타임 파라미터
불변 회귀가드) + `kebab-app/tests/config_invalidation.rs`(4 end-to-end: 동일 config=전 skip,
청킹 변경=md+code 재색인, `[ingest.code]` 변경=코드만, search 변경=재색인 0). 기존 skip 테스트
회귀 0(parser_version exact assert 는 base 접두사 비교로 갱신 — code_ingest_smoke/pdf_pipeline).

spec/plan: `docs/superpowers/specs/2026-06-03-ocr-toggle-invalidation-spec.md` /
`…/plans/2026-06-03-config-invalidation-plan.md`.

## 2026-06-03 — ingest 진행 로그 개선: 파일명·phase·heartbeat·slowest 요약 (v0.26.1)

**무엇을 왜 추가했나.** arctic 도그푸딩 중 이미지/PDF 혼재 + OCR/caption on 볼트에서
ingest 가 중간부터 느려졌는데, TTY 진행바가 **파일명·현재 phase·모델·경과시간**을 안 보여
"멈춘 것처럼" 보였다. 원인(비전 모델 스와핑)을 진행 표시만으로 파악 불가. v0.24.0 상세
진행 로깅의 후속으로 느린 phase 와 병목 파일을 가시화했다. spec/plan:
`docs/superpowers/specs/2026-06-03-ingest-log-improve-spec.md` / `…/plans/2026-06-03-ingest-log-improve-plan.md`.

**무엇이 바뀌었나 (additive, `ingest_progress.v1` 유지 — major bump 없음).**

- 신규 wire 이벤트 `asset_phase { idx, total, phase, model }` — asset 이 느린 phase
  (`ocr` / `caption` / `embed`) 진입 시 1회 emit. `model` 은 그 phase 의 모델 id
  (ocr/caption = 비전 LLM, embed = 임베더 model_id), 없으면 `null`. 짧은 phase
  (parse/chunk/store) 는 노이즈 방지로 미emit.
- `asset_timings` 에 `ocr_ms` / `caption_ms` 필드 추가 (serde `default` 0 → 구 소비자
  호환). 이미지·PDF 경로도 이제 `asset_timings` 를 emit (이전엔 markdown 만) — slowest
  요약이 비전-모델 병목을 정확히 집계.
- CLI 렌더(`kebab-cli/src/progress.rs`): AssetStarted 시 진행바 메시지에 파일명(긴 path 는
  말미 축약), AssetPhase 시 `{path} · {phase}({model})…`, steady-tick 1s 커스텀 키로
  경과초 `(Ns)` 라이브 갱신, `Completed` 시 stderr 에 `⏱ 최장 소요 top-5` 표.
  `--quiet` 여도 요약은 출력, `--json` 은 ndjson 만(사람텍스트 미혼입).

**emit 지점.** `kebab-app/src/lib.rs` — 이미지 경로 `apply_ocr`/`apply_caption` 직전
+ ocr/caption 시간 측정, markdown/이미지/PDF 임베딩 루프 직전 `embed` phase, 각 경로
`asset_timings` 에 측정값 채움. PDF `ocr_ms` 는 기존 page-OCR 총합 재사용.

**알려진 한계.** code asset 경로는 진행 이벤트(AssetChunked/Timings) 무emit 이라 slowest
요약에 미포함(기존 동작 유지, 비범위). top-N 의 N=5 상수(config 화 비범위). PDF 페이지
OCR 진행은 기존 `pdf_ocr_started/finished` 가 담당(본 작업 비범위).

**도그푸딩 (별도).** 사용자 Obsidian 볼트(이미지/PDF + OCR on) 재현 — 느린 구간의
파일·phase·모델 즉시 가시 + 종료 요약이 병목 파일을 짚는지. release notes + 본 entry 갱신.

## 2026-06-03 — arctic-embed-l-v2.0 임베더 통합 (candle + Ollama) (v0.26.0)

**무엇을 왜 추가했나.** 별칭(doc-side expansion) 제거(v0.25.0) 후 설명형 query 의
recall 보강책으로 `snowflake-arctic-embed-l-v2.0` 임베더를 두 백엔드로 통합했다.
근거는 방법별 측정(`/build/dogfood/logs/2026-06-03-method-measurements.md`):
arctic = recall@10 **130/132**, recall@50 **132/132**, **용어 무손실**(syn/abbr/en
유지). e5-large 대비 +7, 색인 1회·per-query 0·LLM 0 = 살아있는 KB 에 지속 가능.
별칭이 청크당 색인-시 LLM(나무위키 18문서 cold 2.5h)을 요구한 것과 대조.

**무엇을 건드렸나.**
- `kebab-embed-candle`: e5 하드코딩(`HF_MODEL`/`SUPPORTED_MODEL`/mean/`query:`+`passage:`)을
  **모델 레지스트리**(`MODEL_REGISTRY`: `EmbedModelSpec { name, hf_repo, pooling, query_prefix, doc_prefix, dim, version_tag }`)로
  일반화. e5(mean, `query:`/`passage:`) + arctic(**CLS**, `query:`/무접두어). pooling
  은 모델별 분기(mean=attention-mask-weighted / CLS=`hidden[:,0,:]`), tokenize/forward/L2
  공유. arctic pooling=CLS 는 HF `1_Pooling/config.json`(`pooling_mode_cls_token:true`)로
  확인. `model_version` 은 arctic 일 때 `+arctic-cls` 태그(switch 시 embedding_version
  cascade 트리거); e5 는 fastembed-e5 와의 호환(NUMA 드롭인) 위해 plain `config.version` 유지.
- `kebab-embed-ollama` (신규 크레이트): `Embedder` 구현, `reqwest::blocking` POST
  `/api/embed` `{model, input:[...]}` → `embeddings`. batch 48 + fail-soft 재시도 3,
  결과 **L2 정규화**(Ollama raw 반환), dim 검증, query/doc prefix 모델 태그로 추론
  (`arctic-embed`→`query:`/무접두어, `e5`→`query:`/`passage:`). `model_version=ollama:{model}`.
  endpoint = `models.embedding.endpoint` ?? `models.llm.endpoint`.
- `kebab-config`: `EmbeddingModelCfg.endpoint: Option<String>`(serde default, ollama용) +
  `provider` 문서에 `ollama` 추가 + env `KEBAB_MODELS_EMBEDDING_ENDPOINT`.
- `kebab-app::embedder()`: provider match 에 `ollama` 분기 추가(facade 경유).
- workspace member += `kebab-embed-ollama`, version 0.25.0 → **0.26.0**(minor).

**correctness 게이트.** candle arctic 임베딩이 측정에 쓴 Ollama `snowflake-arctic-embed2`
임베딩과 일치해야 pooling/prefix 정확성(=recall 130 재현)이 보장된다. 검증:
`kebab-embed-candle/tests/arctic_ollama_parity.rs`(`#[ignore]`, live Ollama 의존) 가
candle arctic vs 우리 Ollama 어댑터로 같은 문장(설명형/약어/영문 포함, doc+query
양 경로)을 임베딩해 per-sentence **코사인 > 0.99** 를 assert. 수동 실행 결과(코사인값)는
릴리스 전 본 entry 에 기록.

**수동 검증 결과** (2026-06-03 worker 실측, Ollama @192.168.0.47:11434
`snowflake-arctic-embed2`): 8문장 × (doc+query) 16벡터 per-sentence 코사인
**0.999984 ~ 0.999995**, `cosine_min = 0.999984` (게이트 0.99 대비 대폭 상회).
설명형("후입선출 방식으로 동작하는 자료구조")·약어("SVM 은 support vector machine")·
영문·한글 모두 일치. → candle arctic 의 CLS pooling + `query: ` prefix 가 Ollama 측정
경로와 정확히 동일 = recall@10 130 재현 보장. Ollama raw 도 이미 L2-정규화(norm 1.0)라
어댑터의 L2 정규화는 idempotent no-op. 로그: `/build/dogfood/logs/arctic-parity.log`,
요약: `/tmp/arctic-result.md`.

**종단 도그푸딩** (2026-06-03, kebab **v0.26.0** 바이너리, provider=ollama
`snowflake-arctic-embed2` @192.168.0.47). Python 하니스가 아닌 **실제 kebab
ingest→store→search 파이프라인**으로 검증: namu 코퍼스 997 docs / 23151 chunks
fresh 색인(`config-arctic.toml`, kb-arctic, errors=0) → 확장 골든
(`namu_golden_expanded.yaml`, 24그룹/132변형) hybrid k=50 eval
(run_019e8c5788a374e098d85d84eb900e23). 결과: **recall@10 130/132 (0.985)**,
**recall@50 132/132 (완벽)**, fully_consistent **22/24**(baseline e5 19/24 대비 +3),
MisRanked 2 / Missing 0, mean_spread@10 0.083(e5 0.208 대비 대폭 개선). 종류별
recall@10: abbr 7/7 · en 24/24 · ko 24/24 · syn 17/17 · para 23/24 · para2 18/18 ·
para3 17/18 = **용어 무손실 + 설명형 거의 완벽**. e5 baseline(123/132) + 측정 하니스
arctic(130) 와 종단 일치 — 측정→구현→실파이프라인 재현 삼중 확인. 잔존 MisRanked
2개는 D(query-side) 후속 보강 대상. 결과 `/tmp/arctic_e2e_variants.json`,
baseline 비교 `/build/dogfood/logs/2026-06-03-new-baseline-v025.md`.

**호환성.** 기본 provider=fastembed e5 동작/벡터 불변(arctic 은 opt-in). dim 1024
동일이나 LanceDB 테이블명에 모델명 포함(`chunk_embeddings_{model}_{dim}`)이라 충돌
없음. e5 → arctic 전환 = `embedding_version` cascade(모델별 벡터 상이) → **재색인 필요**
(기존 e5 KB 와 혼용 불가, 명확). A(heading enrichment)는 측정상 arctic 에서 악화 →
미적용. spec: `docs/superpowers/specs/2026-06-03-arctic-embedder-spec.md`, plan: 동일
디렉토리 `2026-06-03-arctic-embedder-plan.md`.

## 2026-06-03 — doc-side expansion(별칭) 기능 완전 제거 (v0.25.0)

**무엇을 왜 제거했나.** v0.21.0 (PR #195/#196) 에서 도입한 색인-시 청크당 LLM
별칭 생성 + 별칭 검색 채널을 **완전히 제거**했다. 근거는 비용 재고 연구
(`docs/superpowers/research/2026-06-03-expansion-cost-rethink-research.md`, Step 0/1
측정 + 딥리서치): 별칭 ROI 가 음수였다 — cross-lingual 검색은 e5-large 임베더
단독으로 이미 충분하고, 별칭의 실측 기여는 설명형 query +2 그룹(14/18→16/18)뿐인데,
그 대가가 **청크당 색인-시 LLM 호출**(살아있는 KB 에 지속 불가능한 비용; 나무위키
18문서 cold 2.5h)이었다. 문헌(arXiv 2309.08541)도 "강한 검색기에는 query/doc
expansion 이 오히려 해롭다"를 확인. 별칭은 default-off 였으므로 일반 사용자 체감 0.

**무엇이 제거됐나 (코드/스키마/wire).**
- 코드: `kebab-app/src/expansion.rs` 모듈 전체, `ingest_one_asset` 의 별칭 생성·캐시·
  임베딩 루프, `Chunk.aliases` 필드, `kebab-config` 의 `IngestExpansionCfg`
  (`[ingest.expansion]` 섹션 + `KEBAB_INGEST_EXPANSION_*` env), `kebab-search` 의
  `run_alias_query`/`merge_body_alias` alias lexical arm, alias sentinel 벡터 upsert
  경로 + `alias_sentinel_ids_to_delete`.
- wire: `ingest_progress.v1` 의 `expansion_progress` kind 제거 (v0.24.0 에서 막
  추가된 additive variant 라 소비자는 부재 허용 → major bump 불요).
  `asset_timings.expansion_ms` 필드는 **wire 호환 위해 유지하되 값 항상 0**.
- 스키마: 신규 forward-only 마이그레이션 **V013** 이 `chunk_aliases_fts`(+ 트리거)
  와 `chunks.aliases` 컬럼을 DROP. 과거 V010 은 freeze 무수정. 별칭 default-off 라
  기존 KB 대부분 빈 데이터 → 손실 없음. corpus_revision bump (검색 캐시 무효화).

**무엇을 유지했나 (제거 금지).** `Metadata.aliases`(문서 메타데이터 Vec, expansion
과 무관), `AssetChunked`/`AssetTimings` wire 이벤트, derivation_cache 의 `embedding`
kind(V012 임베딩 캐시 — 성능 핵심), `chunks_fts`(본문 FTS) 전부, `ALIAS_SUFFIX`/
`strip_alias_suffix`(검색 시 기존 KB 의 잔존 별칭 벡터를 본문 chunk 로 graceful 매핑하는
read-side 하위호환).

**기존 KB 영향.** 별칭 벡터가 있던 KB 도 마이그레이션 후 search/ask 정상 — 잔존 별칭
sentinel 벡터(`{chunk}#alias#N`)는 검색 시 `strip_alias_suffix` 로 본문 chunk 에
매핑되거나 `kebab reset` 으로 정리된다. 본문/임베딩 불변이라 재색인 불요.

**spec/plan.** `docs/superpowers/specs/2026-06-03-remove-doc-expansion-spec.md` +
`docs/superpowers/plans/2026-06-03-remove-doc-expansion-plan.md`. 원 도입 spec
`2026-05-30-doc-side-expansion-design.md` 에 제거 banner 추가.

## 2026-06-02 — 상세 ingest 진행 로깅 (asset 내부 phase 가시화, v0.24.0)

**무엇이 문제였나.** ingest 진행 이벤트가 asset(문서) 단위(`asset_started` /
`asset_finished`)뿐이라 한 문서 내부의 parse / chunk / **expansion(별칭 LLM,
청크당 순차 호출)** / embed / store 가 깜깜했다. expansion 은 청크당 ~1~4s
(원격 GPU Ollama)이고 큰 문서는 청크 수백~천 개 → 그 한 문서에서 수십 분이
걸리는데, 진행바는 `1/5150` 에 멈춘 듯 보여 사용자가 병목을 못 봤다.

**무엇을 추가했나 (wire `ingest_progress.v1` additive, 호환 유지).**
`IngestEvent` 에 세 변이 추가 — `#[serde(tag="kind")]` 라 신규 `kind` 추가는
wire v1 호환:

- `asset_chunked { idx, total, chunks }` — 청킹 직후(expansion/embed 전) 즉시
  "이 문서가 N청크" 노출. markdown / image / pdf 세 경로 모두 emit.
- `expansion_progress { idx, total, done, chunks }` — expansion 루프 중
  **스로틀** 발신(매 25청크 또는 ≥1s, 종료 시 `done == chunks` 1프레임 더).
  캐시 히트 청크도 `done` 에 포함(warm 재색인 fast-forward 가시화). 채널 폭주
  방지 — 매 청크 emit 금지.
- `asset_timings { idx, total, parse_ms, chunk_ms, expansion_ms, embed_ms,
  store_ms }` — asset 처리 phase 별 소요시간. **markdown 경로만** emit
  (image/pdf 는 phase shape 가 달라 생략; AssetChunked 만 emit).

**설계 결정 — AssetTimings 이벤트 vs AssetFinished 필드.** IMPL_BRIEF §1 은
`AssetFinished` 에 optional phase-timing 필드를, §2 는 대안으로 신규
`AssetTimings` 이벤트를 제시(권장). 후자를 택함 — `AssetFinished` 는 호출부
(`ingest_with_config_progress` 루프)에서 만들어지는데 timing 데이터는
`ingest_one_asset` 내부에만 있어, 필드를 채우려면 `kebab_core::IngestItem`
(wire-stable struct) 변경 또는 별도 plumbing 이 필요. `ingest_one_asset` 가
`progress` 핸들을 이미 들고 있으므로 새 이벤트를 직접 emit 하는 쪽이 crate
경계(kebab-core 불변)도 지키고 더 깔끔. `AssetFinished` 는 손대지 않음.

**CLI 렌더(`kebab-cli` progress.rs).** `asset_chunked` → 진행바 message `→ N
chunks`. `expansion_progress` → message `별칭 확장 {done}/{chunks}` (라이브).
`asset_timings` → asset 종료 시 `⏱ parse Xs · chunk Ys · expand Zs · embed Ws
· store Vs` 한 줄(`fmt_ms`: <1s 는 ms, ≥1s 는 1-decimal 초). `--json` 은
`emit_json` 이 임의 이벤트를 직렬화하므로 자동 처리. `--quiet` 억제, 비-TTY
expansion_progress 는 로그 폭주 방지로 기본 억제(진행바 message 로 커버).

**검증.** `cargo clippy --workspace --all-targets -- -D warnings` exit 0,
`cargo test -p kebab-app -p kebab-cli` exit 0. 단위 테스트: ingest_progress.rs
(3 신규 변이 직렬화 `kind` 판별 + 순서 불변식 재작성), progress.rs(`fmt_ms` 단위
전환), 통합(`--json`/human stderr 에 새 이벤트 흐름). 실동작 smoke: 2-문서 ingest
의 `--json` 에 `asset_chunked`/`asset_timings` 출현 + human `⏱ parse…·store…` 라인
확인. expansion 라이브 카운터는 원격 LLM 필요라 단위/통합으로 커버.

**리뷰 반영.** (1) `store_ms` 경계 정정 — stale-vector orphan purge(LanceDB I/O)를
`store_ms`(SQLite persist 전용)에서 빼 `embed_ms`(vector phase)로 이동. 진단
정확도: store_ms 가 이제 SQLite put_* 만 의미(편집 재색인 시 920ms 가 실은 벡터
삭제였던 오귀속 제거). purge 는 여전히 unconditional + 새 upsert 이전 실행 —
기능 동등. (2) 최종 `expansion_progress` 프레임을 `done != last_done` 로 가드 —
chunks 가 throttle 배수일 때의 중복 프레임 + chunks==0 시 0/0 프레임 제거.

**알려진 한계.** image/pdf 경로는 phase timing 없음(AssetChunked 만).
expansion_progress 비-TTY 억제는 의도적(필요 시 `--json` 으로 전량 관측).

## 2026-06-02 — ingest 백엔드/디바이스 표시 + KB 이전 문서 (v0.23.1)

**동기.** Metal 빌드가 실제로 GPU 를 쓰는지 사용자가 터미널에서 못 봐서 Activity
Monitor 로 확인해야 했다(`select_device()` 의 device 로그는 kb.log 파일로만, 기본
EnvFilter=warn 이라 `--verbose` 필요). 또 "어떤 DB 파일을 옮기나" 가 README 에
구체적이지 않았다.

**무엇.** (1) `kebab-cli` ingest 시작 시 임베딩 백엔드/모델/차원을 stderr 한 줄로
표시(`임베딩 백엔드: candle (Metal/GPU 빌드) · 모델 …`), `--json`/`--quiet` 에선
억제. Metal 표기는 `cfg!(feature="embed_metal")` 기반(빌드 사실); 확정 런타임
디바이스는 여전히 kb.log(`candle device = …`). (2) README "외부 계산 + 로컬 검색"
절에 복사 대상 2개(`kebab.sqlite`/`sqlite`, `lancedb/`/`vector_dir`)와 `[storage]`
config 키·`models/`·`assets/` 복사 불필요·동일 버전/모델 조건·rsync 예시 추가.

**범위.** CLI 출력 + 문서만. 동작·wire·schema·벡터 변경 없음. 버전 0.23.0 → 0.23.1.

## 2026-06-02 — candle Metal(Apple Silicon GPU) opt-in build feature

**동기.** candle CPU 임베딩은 e5-large/512-tok 에서 ~1.5~1.9 s/chunk 로 느리고,
코어를 더 줘도(rayon/MKL) 안 빨라진다(병목=커널 효율). 대용량 코퍼스(수만 청크)는
CPU 로는 수 시간. 사용자 워크플로: **M4 Pro 맥에서 GPU 로 빠르게 색인 → sqlite +
lancedb 만 Linux NUMA 서버로 복사 → 서버는 CPU candle 로 질의** (벡터 동일 모델이라
호환, KB 이식성은 06-01 항목 + workspace_path 상대경로 + chunks.text 저장으로 확인).

**무엇.** `kebab-embed-candle` 에 `metal` feature 추가 →
`candle-core/-nn/-transformers` 의 metal 백엔드 활성. `select_device()` 가 metal
빌드 시 `Device::new_metal(0)` 선택(실패 시 CPU fallback), 비-metal 빌드는 기존
`Device::Cpu` 그대로. host 복사 전 `.contiguous()` 추가(Metal 의 strided view 가
`to_vec2` 거부 — CPU 는 허용). feature passthrough: `kebab-app/embed_metal` →
`kebab-cli/embed_metal`. 빌드: `cargo build --release --features embed_metal`(macOS).

**제약 / 검증 분담.** metal 은 **macOS 전용 컴파일** — Linux CPU 머신(개발/서버)은
비-metal 경로만 빌드(검증: clippy 0 + candle 단위 6 + thread_cap + parity, exit 0).
**Metal 실행·속도·벡터 패리티(GPU vs CPU)는 M4 Pro 에서 사용자 검증** (Claude 의
Linux 환경에서 불가). 로그 `candle device = Metal (GPU)` 로 GPU 사용 확인.

**호환성.** default(비-metal) 동작·벡터 불변. wire/schema 변경 없음. 버전 0.22.0 →
**0.23.0** (신규 opt-in build feature surface).

amends: `docs/superpowers/specs/2026-06-01-embed-candle-track-spec.md` (§10 후속 — GPU 가속).

## 2026-06-01 — candle 임베딩 provider (NUMA double-free 회피, opt-in)

**무엇이 문제였나.** 듀얼소켓 NUMA 서버에서 `provider=fastembed`(onnxruntime)로
대규모 ingest(5150-doc)를 돌리면 onnxruntime 가 intra-op 스레드를 48개로
하드코딩해 NUMA 힙을 손상시키고 double-free 로 프로세스가 죽었다. 스레드 수를
config 로 줄일 surface 가 없었고, fastembed 4.9 의 ORT 바인딩은 이를 노출하지
않는다.

**진단 / 결정 (사용자 승인 2026-06-01).** 같은 모델
`intfloat/multilingual-e5-large` 를 **candle(순수 Rust)** 로 돌리는 임베딩
provider 를 추가하기로 결정. candle 의 CPU 백엔드는 글로벌 rayon 풀 크기로
스레드를 정하므로, 한 번의 `rayon::ThreadPoolBuilder::build_global` 캡으로
스레드를 NUMA-안전한 수로 묶을 수 있다. **재색인 0 목표**(`embedding_version`
유지) — Phase 0 스파이크(커밋 76841af)가 candle vs onnxruntime **코사인
1.000000** 패리티를 입증했고, 본 Track 1 구현의 패리티 테스트로 차원별 max
절대오차를 재실측해 확정.

**무엇을 건드렸나.**
- 신규 crate `crates/kebab-embed-candle` — `kebab_core::Embedder` 구현
  (`CandleEmbedder`). 스파이크 파이프라인(safetensors via hf-hub → XLM-RoBERTa
  forward → attention-mask mean pooling → L2 → e5 prefix)을 production 으로
  흡수. deps 는 candle 트리를 이 crate 에 격리 (core/config 외 다른 kebab-*
  의존 0 — design §8 경계). 모델 캐시 `{model_dir}/candle/`.
- 스레드 캡: `[models.embedding].num_threads`(u32, default 0=auto) + env
  `KEBAB_EMBED_THREADS`(우선). `CandleEmbedder::new` 에서 n>0 이면 글로벌 rayon
  풀 1회 캡(이미 init 시 no-op).
- 주입 분기: `kebab-app::App::embedder()` 가 `config.models.embedding.provider`
  분기 — `fastembed`/`onnx`/(빈값) → 기존 `FastembedEmbedder`(동작 불변),
  `candle` → `CandleEmbedder`, 미지값 → 에러. `none` 은 기존 lexical-only 유지.
- 스파이크 crate `crates/spike-embed-candle` 제거(학습은 production 으로 흡수됨).
- 버전 0.21.1 → **0.22.0** (신규 config surface — pre-1.0 minor bump).

**패리티 증거.** candle vs `FastembedEmbedder`(onnxruntime), 동일 10문장
(한/영 혼합, e5 `passage:`/`query:` prefix): **cosine_min = 1.000000,
차원별 max 절대오차 = 2.01e-7** (f32 커널 반올림 수준 — 랭킹 영향 임계보다
약 50배 작음). 재현: `cargo test -p kebab-embed-candle --release -- --ignored
--nocapture` (`crates/kebab-embed-candle/tests/parity.rs`, 모델 ~2GB 필요라
CI 기본 제외). 이 수치가 `embedding_version` 유지(재색인 0) 결정의 근거.

**호환성.** fastembed default 경로의 동작/벡터 불변. `embedding_version`
유지 → 기존 색인 재사용(재색인 0). wire schema 변경 없음. 옛 config.toml 은
`num_threads` 가 serde default(0)로 채워져 그대로 파싱.

**잔여 게이트 (사용자 실행, Claude 불가).** 그 듀얼소켓 NUMA 서버에서
`provider=candle` 로 ingest 가 double-free 없이 EXIT=0 완주하는지 — 사용자
배포·실사용이 곧 이 검증을 겸한다 (meta-spec §4.3).

**도그푸딩 (2026-06-02, 단일소켓 12-thread VM).** `provider=candle` +
`config-candle.toml`(expansion off — 임베더 격리) 로 `/build/dogfood/corpus`
전체 재색인: **scanned=998, new=997, errors=0, stderr=0, KB 997 docs /
23,151 chunks**, duration ≈ 34,329 s (9.5 h). candle 가 23k+ 청크를 메모리
오류 0 으로 완주 — onnxruntime 이 서버에서 6/5150 에 죽던 것과 정반대.
(이 VM 은 비-NUMA 라 NUMA 자체 재현은 아니나, candle 은 onnxruntime 을
호출하지 않으므로 동일 크래시 종류가 구조적으로 불가.)

**A1(taskset/numactl) 워크어라운드 실서버 반증 (2026-06-02).** 사용자가 NUMA
서버에서 `taskset -c 0-3 kebab ingest`(fastembed/onnx 바이너리) 실행 → 4코어로
제한했는데도 6/5150 에서 `세그멘테이션 오류 (core dumped)`. 스레드 축소가
onnxruntime 힙 손상을 제거하지 못함(크래시 위치만 3→6 이동). 결론: 이 크래시는
스레드 *수* 문제가 아니라 onnxruntime 네이티브 코드의 메모리 안전 결함 →
**A1 은 신뢰 불가 우회책. candle(onnxruntime-free)이 유일한 실 해법.**

**MKL 가속 부정 결과 (2026-06-02).** "candle 이 코어를 더 쓰게" 하려고 candle
`mkl` feature(Intel MKL) 를 벤치 (e5-large, 512-tok 청크, N=32):
pure-Rust 1857 ms/chunk(381% CPU) vs MKL 2574 ms/chunk(896% CPU, rayon12+mkl12)
/ 2792 ms/chunk(817% CPU, rayon1+mkl12). **MKL 은 코어를 더 쓰지만 모든 설정에서
38~50% 더 느림** (MKL 2020.1 sgemm + 스레드 오버헤드/과다구독; candle 0.10.2 는
f16 `hgemm_` 미해결로 링크도 실패 — 벤치는 호출 안 되는 스텁으로 우회). 또
pure-Rust 는 rayon 8↔12 간 throughput 불변(~1.86 s/chunk) — 병목은 코어 수가
아니라 candle e5-large/512tok 커널 효율. **결론: MKL 미채택, 순수-Rust 유지(안전
최상 + CPU 에서 더 빠름). 속도 레버는 코어가 아니라 청크 길이/모델 크기/GPU.**

amends: `docs/superpowers/specs/2026-06-01-embed-candle-track-spec.md`.

## 2026-05-31 — config 마이그레이션 (`kebab config migrate`)

**Trigger**: config.toml 스키마가 진화해도(v0.21.0 의 `[ingest.expansion]` 등) 기존 사용자 파일은 serde default 로 *동작*만 호환될 뿐 새 섹션이 파일에 안 써져 사용자가 노브의 존재를 알 수 없었다. DB 의 V00X refinery 와 달리 config 엔 마이그레이션 메커니즘이 없어 추가. 설계 `docs/superpowers/specs/2026-05-31-config-migration-design.md`, 계획 `docs/superpowers/plans/2026-05-31-config-migration.md`, PR #198.

### 메커니즘

`kebab config migrate` 가 (1) **reconciliation** — `Config::defaults()` 구조에 있고 사용자 파일에 없는 섹션/키를 주석과 함께 `toml_edit` 으로 추가(버전 무관·멱등) + (2) **step 체인** — `schema_version` 기반 non-additive 변환(첫 step v1→v2 = `workspace.include` 제거, p9-fb-25). `init` 과 migrate 가 `annotated_default_document()` 로 주석·헤더 단일 원천 공유 → init config 도 섹션 주석 보유. `schema_version` default 1→2(sync 마커+step 축). 안전 3축=멱등·백업(`.bak`, 원본 byte-identical)·dry-run + tmp atomic rename(round-trip 검증). 순수변환=`kebab-config/migrate.rs`, I/O facade=`kebab-app`.

### 도그푸딩 evidence (v0.21.0 release 바이너리)

옛 스키마 흉내(`schema_version=1`, `[workspace]`+`[search]`+`[rag]`, `workspace.include` 보유, 사용자가 `default_k=25`/`score_gate=0.8`+인라인 주석 손봄):

| 시나리오 | 결과 |
|----------|------|
| `migrate --dry-run` | 22 changes 나열, **파일 미수정** |
| `migrate` | 적용 v1→v2, `.bak` **원본과 byte-identical**(diff 0) |
| 값·주석 보존 | `root="~/MyNotes" # 내가 직접 바꾼…`, `default_k=25`, `score_gate=0.8` 유지 |
| deprecated 정리 | `workspace.include` 제거(grep 0) |
| 가시화 | `[ingest.expansion]`·`[logging]`·`[pdf.ocr]` 등장 |
| 멱등 | 재실행 → `config 이미 최신입니다 (schema v2)` |
| doctor | `✓ config_migration  config up to date (schema v2)` |
| `--json` | `config_migration.v1` (kind=added_section/removed_deprecated) |

### 알려진 한계 / 결정

- 누락 섹션은 테이블 끝 append(순서 미보존, 값·주석·기존순서는 보존).
- 통째 누락 부모는 부모 경로 1건 기록, 부분 존재 부모는 leaf 경로 기록(재귀 깊이 차이).
- doctor 의 `config_migration` ok=false 가 전체 `DoctorReport.ok` 를 false 로 만듦(의도; hint 가 교정 명령 제시, warn 상태 미도입).
- `schema_version` bump(1→2)은 additive(데이터 무효화 아님, 읽기 호환 유지) → DB/wire breaking release 트리거 아님. 신규 CLI 서브커맨드+doctor 체크+init 출력 변경은 user-visible surface.

## 2026-05-31 — doc-side expansion 별칭 개선 + 파생물 캐시(V012)

**Trigger**: Phase 2 doc-side expansion(별칭) 효과를 실사용 규모(한국어 나무위키 ~1000 문서 CS corpus)로 검증하고, 그 과정에서 드러난 별칭 생성 비용을 "내용 해시 기반 파생물 캐시"로 해소. v0.21.0 cut. 측정 상세: `docs/superpowers/handoffs/2026-05-31-namu-wiki-alias-cache-study.md`, 설계: `docs/superpowers/specs/2026-05-31-derivation-cache-design.md`.

### (a) 별칭 개별 dense 벡터 + boilerplate skip

초기 별도-벡터(청크당 별칭 8개를 줄바꿈으로 묶어 한 벡터로 임베딩) 방식은 평균화로 특정 표현이 **희석**되고 나무위키 메뉴(boilerplate) 청크에도 별칭이 생성돼 **오히려 회귀**(13/18). 개선판은 별칭을 줄별 **개별 sentinel 벡터**(`{chunk}#alias#N`)로 색인하고 본문 벡터는 그대로 두며, boilerplate 청크는 별칭 생성을 skip 한다. `kebab-core::strip_alias_suffix` 가 suffix 형(`{orig}#alias`)과 per-alias 형(`{orig}#alias#N`) 둘 다 처리(bare chunk_id 는 `#` 없는 blake3 32-hex 라 첫 `#alias` 가 경계).

| 구성 | fully_consistent | mean_spread@10 |
|------|------------------|----------------|
| baseline (별칭 off) | 14/18 | 0.222 |
| 별도-벡터 (별칭 묶음 1벡터) | 13/18 | 0.278 (악화) |
| **개선 (별칭 개별 벡터 + boilerplate skip)** | **16/18** | **0.111** |

baseline 약점은 전부 "설명형" 변형(용어·약어·영어는 18그룹 완벽) = 자연어 설명과 문서 전문용어의 "어휘 격차". 개선판이 linked_list·sorting 회복 + tcp 회귀 복구. 파일: `crates/kebab-core/src/ids.rs` (`strip_alias_suffix` find 기반), `crates/kebab-app/src/lib.rs`, `crates/kebab-app/src/expansion.rs`. `[ingest.expansion]` default off (opt-in).

### (b) 대조군 false-positive — 별칭 무죄

대조군(정답 없는 질문) 10개 RAG run 에서 refusal 0.6 (4개 grounded). false-positive 4개(graphql·oauth·react·grpc)의 인용 출처는 **전부 노이즈 본문**(GitHub_Mobile·API·Svelte 등), **별칭 sentinel 인용 0** → 별칭이 false-positive 를 유발하지 않음(별칭 무죄, default-on 안전성 근거).

### (c) 파생물 캐시 145배 + 외부 계산 이식 워크플로

별칭 18문서 재생성 2.5시간이 근본 병목. `chunk_id` 가 위치(`ordinal+span`) 기반이라 chunk_id 캐싱은 중간 수정 시 무력 → 청크 text **내용 해시**를 키로 한 범용 캐시(V012). `cache_key = blake3(kind ‖ text_blake3 ‖ version_key)[:32]`, version_key 에 model/prompt/dimensions 포함 → §9 cascade 와 자동 정합(버전 bump 시 자동 miss). embedding(본문 + 별칭 벡터 양쪽) + 별칭 LLM 결과 캐싱. **측정: 정답 3개 cold 1879s → warm 13s ≈ 145배**(18문서 환산 2.5h → ~80s). `corpus_revision` 은 bump 안 함(순수 가산). 파일: `migrations/V012__derivation_cache.sql`, `crates/kebab-core/src/derivation.rs`, `crates/kebab-store-sqlite/src/derivation_cache.rs`, `crates/kebab-app/src/derivation_payload.rs`.

**이식**: search/ask 는 `kebab.sqlite` + `lancedb` 만으로 동작(`storage_path` asset 은 search/ask 경로에서 사용처 0). 비싼 색인(별칭 LLM + embedding)을 외부 CPU ollama 서버에서 돌린 뒤 sqlite(+derivation_cache) + lancedb 만 로컬로 복사하면 동일 동작 + 증분 캐시 히트가 머신 독립적으로 적용.

### Known limitation

- **stack·svm 설명형 잔존**: 개선 후에도 2개 설명형 변형은 별칭으로 못 메움(추가 개선 보류).
- **grounded/refusal 오분류**: answer 가 "근거에서 찾을 수 없다"고 정직히 거부했는데도 부분 언급 인용이 있으면 grounded 로 오분류 → 실제 refusal 은 0.6 보다 높음. kebab grounded/refusal 판정의 별도 개선 여지(후속 후보).
- **korean_tokens 캐시 / export-import 명령 / 별칭 default-on**: 보류.

## 2026-05-29 — v0.20.2 dogfood findings + 검색 품질 baseline

**Trigger**: v0.20.2 release 준비 8-finding dogfood 라운드 (2026-05-29). 구현 + eval + 도그푸딩 전부 완료.

### 8 findings 요약

| # | Finding | 구현 범위 | Dogfood 결과 |
|---|---------|----------|-------------|
| 1 | Ask 응답언어 (rag-v3) | `SYSTEM_PROMPT_RAG_V3` 신설, config default rag-v2→rag-v3 | 영어 query→영어 응답 ✅ |
| O-2 | Refusal 언어중립화 | 한국어 리터럴 → 언어중립 문구 | 중립 문구 확인 ✅ |
| 2 | Bulk input schema | `bulk_search_input.schema.json` 15필드 + error shape hint | bulk ndjson 검증 ✅ |
| 3 | List docs title 중복 | human-readable `doc_id \t title \t doc_path` | Registry java/kt 구분 ✅ |
| 4 | doc.lang semantic | schema/README 에 `lang="und"` = code 정상 명시 (docs only) | docs 정합 확인 ✅ |
| 5/6 | fusion_score/score_kind | README + `search_hit.schema.json` description 보강 | wire 정합 ✅ |
| 7 | index_version 구분 | vector(LanceDB) vs FTS5 구분 README + schema 명시 | `schema --json` 확인 ✅ |
| 8 | Ollama endpoint hint | `kebab init` 에 endpoint config 주석 힌트 추가 | init 출력 확인 ✅ |
| - | eval `--config` facade | eval run/aggregate/compare 가 `--config` honor | dogfood KB eval 가능 ✅ |

**Finding O-2 known limitation**: gemma4:e4b 같은 소형 모델은 refusal 메시지(근거 부족 시)의 언어가 query 언어와 불일치할 수 있음 (영어 query → 한국어 refusal 가능). refusal 판정 자체는 답변의 citation marker(`[#번호]`) 유무 기반(유효 marker 없으면 `LlmSelfJudge` 로 refuse 판정; pipeline.rs:463-486 — `근거가 부족` 정규식은 판정에 no-op, tracing 관찰용)이라 정확도 영향 없음. v0.20.2 known limitation 명시.

### 검색 품질 baseline (golden suite, 2026-05-29)

**Golden suite**: `/build/dogfood/golden_queries.yaml` (10 query, 다중 토픽 한국어+영어+코드). eval `--config /build/dogfood/config.toml` 로 dogfood KB 직접 평가 가능 (eval `--config` facade 패치 enabler).

**Metric baseline** (v0.20.2 dogfood KB, 2026-05-29):

| Mode | hit@1 | hit@3 | hit@10 | MRR | recall@10 | empty |
|------|-------|-------|--------|-----|-----------|-------|
| hybrid | 0.7 | **1.0** | 1.0 | **0.833** | 1.0 | 0 |
| lexical | 0.4 | 1.0 | 1.0 | 0.7 | 1.0 | 0 |

**인사이트**:
- hybrid 가 vector 덕분에 top-1 정확도 우위 (0.7 vs lexical 0.4). hit@3 이후는 두 모드 모두 완벽.
- lexical (V009 형태소) 이 짧은 한국어 토큰을 top-3 에 정확히 배치.
- empty_result_rate = 0 — 10개 query 전부 ≥ 1 hit.
- ranking 조정 없이 현재 hybrid RRF 가 baseline 달성 (`[[project_ranking_deferred]]` 결정 유효).

**Golden 큐레이션 교훈 (v0.20.2)**: 초기 dispatch.py 정답을 note 로만 한정한 것이 오류였음. eval 분해 시 vector 가 영어 docstring dispatch.py 를 정상 top-1 으로 반환함을 발견, 정답에 `dispatch.py` 추가 정정 → hit@3 0.9→1.0 개선. **교훈**: golden answer 는 "note 의 intent" 뿐 아니라 "합리적으로 관련된 모든 doc" 을 포함해야 하며, 코드와 note 가 동시에 정답일 수 있다.

Eval logs: `/build/dogfood/logs/eval-hybrid-v0.20.2.json` + `/build/dogfood/logs/eval-lexical-v0.20.2.json`.

Cross-link: `docs/release-notes/v0.20.2-draft.md`, `docs/DOGFOOD.md` §3.6 + §10.2.

## 2026-05-28 — Bug #8 한국어 2자 query 해소 (V009 morphological tokenizer)

**Discovered**: 도그푸딩 round 3/4 (2026-05-28). '한국' / '서울' 0-hit 반복.

**Symptom**: V007 trigram tokenizer 의 ≥3-char minimum 한계.

**Root cause**: trigram 의 bucket 미존재. unicode61 기반 단순 3-gram 분해로는 2-char 한국어 단어를 충분히 커버 못함.

**Fix**: V009 migration + lindera-ko-dic 형태소분석기 + tokenized_korean_text column + first-boot eager backfill. branch `feat/korean-morphological-tokenizer` (17 commit).
- `migrations/V009__fts_korean_morphological.sql` — `tokenized_korean_text` column ADD + chunks_fts (trigram → unicode61) + CASE expression triggers + corpus_revision bump.
- `crates/kebab-chunk/src/lib.rs::tokenize_korean_morphological` — lindera ko-dic 형태소 분석 helper (OnceLock 캐시 + None fallback).
- `crates/kebab-store-sqlite/src/store.rs::backfill_tokenized_korean_text` — 1000-row batch transaction + idempotent backfill (tokenize closure 주입으로 dependency-inversion).
- `crates/kebab-app/src/app.rs::App::open_with_config` — first-boot hook 에서 backfill 호출 (실패 시 warn log + App open 계속).
- `crates/kebab-search/src/lexical.rs::build_match_string` — `MIN_QUERY_CHARS` 3 → 2 로 낮춰 2자 한국어 query 통과 허용 (V007 시절 doc-comment 의 trigram 가정 갱신).

**Amends**: design §5.5 (FTS5 한국어 지원으로 갱신), §9 (index_version cascade — `fts5-v009-korean-morphological` suffix), HOTFIXES 2026-05-24 trigram entry (한국어 2자 query 미해결 footnote 해소).

**Deviation from spec**: spec 의 lindera crate 이름 예상값과 실제 crates.io 등록명 불일치:
- spec §6.1 예상: `lindera-dict-ko-dic`
- 실제 v3.x: `lindera-ko-dic` (crates.io 표준 이름, 한국 형태소분석 dictionary).

**Deferred**: `cargo-deny` 정식 도입 (workspace deny.toml 스캔 + CI gate) 은 별 P9 follow-up 으로 분리. 본 PR 은 `cargo tree --depth 2` 의 SPDX 수동 검증 (lindera/ko-dic 모두 MIT/Apache-2.0 compatible).

**Path A regression noted**: V007 trigram 의 영어 substring 매칭 (token → tokenizer hit) 은 V009 lindera 전환으로 (lindera-ko-dic 은 한국어 only) 영어는 V002 (whole-token only) 로 회귀. Hybrid/vector 검색이 영어 carry, user impact 미미. spec §3 Non-Goals 의 설계 선택 확인 (lexical-only 기능 제약 허용).

**User impact**: 
- `kebab search "한국"` / `kebab search "서울"` 등 2-char 한국어 단어가 이제 hit.
- hybrid/vector 모드에서 한국어 검색은 이미 정상 (embedding 의존), lexical 개선으로 RRF 점수 향상.
- `kebab.sqlite` 크기 증가 (형태소 tokenizer 비용, 도그푸딩 KB 기준 +5-10% 또는 수십 MB).

**Dogfood verification (2026-05-28)** — 2-file Korean wiki fixture (`korea-overview.md` + `korea-compound.md`, DOGFOOD.md §2.1bis reference corpus 참조) 로 fresh KB 색인 + 검증:

| Scenario | Query | Hits | Status |
|---|---|---|---|
| §2.1.a Korean 2-char | `'한국'` | 4 | ✅ |
| §2.1.a Korean 2-char | `'서울'` | 2 | ✅ |
| §2.1.b Korean 3-char | `'지하철'` | 2 | ✅ |
| §2.1.b Compound noun | `'한국어'` | 1 | ✅ |
| §2.1.b Compound noun | `'한국문화'` | 1 | ✅ |
| §2.1.b Compound noun | `'서울특별시'` | 1 | ✅ (ko-dic morpheme decomposition evidence — `서울특별시` → `[서울, 특별시]`) |
| §2.1.d 1-char filter | `'키'` | 0 | ✅ (MIN_QUERY_CHARS=2) |
| §2.1.f raw FTS5 mode | `"'한국'"` | 4 | ✅ |
| §2.1.g FTS5 phrase | `'"서울 의"'` | 2 | ✅ |
| §2.2 Vector | `'한국 문화 와 전통'` (k=3) | 3 | ✅ |
| §2.3 Hybrid | `'한국'` (k=3) | 3 | ✅ |
| §1 Ingest idempotent | re-ingest | 0 new/updated | ✅ |
| §6 Wire schema | `kebab schema --json` | `kebab_version=0.20.1`, `schema_version=schema.v1` | ✅ |
| §9 Doctor | `kebab doctor --json` | `ok=true` | ✅ |

**Snippet evidence** (lindera 분해 확인):
- `'한국'` query → "한국 은 동아시아 의 반 도 국가 다 . 한국 어 는 한반도 의 주요 언어 다" (조사 `은`, `의` 분리).
- `'서울'` query → "서울특별시 와 부산광역시" — ko-dic 의 compound `서울특별시` → `[서울, 특별시]` 자동 분해.

**Known limitation (Option α acceptance)**:
- 사용자 KnowledgeBase 같은 영어/code 위주 KB 에서는 한국어 token 자체 부재로 lexical 0-hit 자연 (vector/hybrid mode 로 우회).
- ko-dic 이 compound noun (`한국정부`, `대한민국` 등) 을 단일 token 으로 저장하는 경우 그 chunk 의 `'한국'` 단독 query 는 hit X.
- N-gram supplement (Option β, sub-token 추가 emit) 은 v0.21.x P9 follow-up.

**V007 → V009 upgrade simulation (2026-05-28)** — whitespace-less Korean fixture (`/build/cache/tmp/v0.20.1-v007strict/corpus/no-space.md` 의 `한국문화는오래되었다한국문화의역사는깊다...`) 로 backfill mechanism 검증:

1. v0.20.1 ingest → chunks 의 tokenized_korean_text 자동 populated.
2. python sqlite3 으로 V007-like state 시뮬레이션 (`UPDATE chunks SET tokenized_korean_text = NULL` + chunks_fts 재구성 raw text only).
3. `App::open_with_config` 재호출 → first-boot hook 의 `backfill_tokenized_korean_text` 자동 발화 → lindera 분해 결과 UPDATE → chunks_au trigger 로 chunks_fts 자동 재-index.
4. Verify post-backfill: tokenized_korean_text 의 populated 값이 `한국 문화 는 오래 되 었 다 한국 문화 의 역사 는 깊 다 . 서울 특별시 는 한국 의 수도 이 며 지하철...` (lindera morpheme + 조사 boundary 분리).

**의외 발견**: FTS5 의 default `unicode61` tokenizer 가 CJK 문자 시퀀스를 별 codepoint 단위로 처리해, raw text 만 indexed 된 상태에서도 일부 한국어 query (예: `'한국'`) 가 hit. lindera 의 marginal benefit 은 corpus 의 morpheme 경계 정확도에 따라 변화. 자세한 unicode61 의 CJK tokenization 정책 = SQLite docs 의 `categories=L*` default + ICU optional extension 참고. spec §4 design choice 의 추가 evidence — V009 의 영어 회귀가 사용자 가치 가장 큰 user-facing 변화로 남고, 한국어 측 benefit 은 corpus 와 ko-dic 정책 의존이라 case-by-case.

**N-gram supplement (Option β) 도입 (2026-05-28, post-PR review enhancement)**:

spec §6.2 의 Option β (sub-token 추가 emit) 가 follow-up 으로 deferred 였지만, dogfood 의 ko-dic compound noun 정책 (`대한민국`, `한국정부` 등 단일 token) limitation 을 즉시 해소하기 위해 v0.20.1 의 implementation 에 포함:

- `kebab-chunk::tokenize_korean_morphological` 에 한글 morpheme (`is_hangul` filter) 의 sliding window 2-gram 추가 emit. 길이 ≥ 3자 morpheme 만 대상 (이미 ≤ 2자 morpheme 은 그대로 사용).
- 영어 / 숫자 / 혼합 token 은 supplement X (`is_hangul` 의 `chars().all()` filter — false positive 회피).

**Verification (fresh dogfood corpus + re-ingest)** — `/build/cache/tmp/v0.20.1-ngram/corpus/extra.md` (대한민국, 한국정부, 주민등록번호 포함):

| Query | Hits | Mechanism |
|---|---|---|
| `'대한'` | 1 | `대한민국` morpheme 의 window `[대한, 한민, 민국]` |
| `'한민'` | 1 | 동일 |
| `'민국'` | 1 | 동일 |
| `'특별'` | 2 | `서울특별시` → `[서울, 특별시]` + `특별시` 의 window `[특별, 별시]` |
| `'주민'` | 1 | `주민등록번호` morpheme window |
| `'등록'` | 1 | 동일 |
| `'tokenizer'` (영어) | 0 | corpus 에 없음, 영어는 supplement 안 함 |

**Trade-off**:
- DB size: 한국어 compound noun 비례 +20-30% (`tokenized_korean_text` column 의 token 수 증가).
- Ingest latency: marginal (sliding window 는 단순 vector loop, lindera tokenize 의 ~5-10% overhead).
- False positive risk: 일부 (예: `'한민'` query 가 `'대한민국'` 도 hit). 작은 risk, user 가 raw FTS5 mode 또는 longer query 로 우회 가능.

**Released as part of v0.20.1**. spec Appendix B 의 prior-knowledge limitation 이 supplement 으로 해소. spec §6.2 의 Option β 결정을 v0.21.x 에서 v0.20.1 implementation 으로 promote (HOTFIXES → spec 갱신 cascade — design §5.5 변경 외에 §6.2 본문은 보존, supplement 동작 만 implementation detail 로 추가).

**Large-scale dogfood verification (2026-05-28, KnowledgeBase + N-gram)** — 사용자 실제 `/home/altair823/KnowledgeBase/` (1781 markdown, 9050 chunk) 를 N-gram supplement 포함 binary 로 backfill 재실행:

- **Backfill duration**: 9050 chunk × lindera tokenize + N-gram + UPDATE + chunks_au trigger = **26.6 초** (real-time wall clock, OnceLock 캐시 + 1000-row batch transaction). ~3 ms/chunk amortized.
- **Storage delta**: `kebab.sqlite` 크기 변화는 영어/code 위주 corpus 라 minimal (N-gram supplement 가 한글 morpheme 만 emit).
- **Query evidence (KnowledgeBase, post-backfill)**:

| Query | Pre-backfill hits | Post-backfill hits | Mechanism |
|---|---|---|---|
| `'한국'` | 0 | **10** ✅ | N-gram supplement 의 `'한국어'` → `[한국, 국어]` window. KB 의 `testdata/coding-md-corpus/*/...md` 의 "문서를 한국어로 다시 정리하기" pattern 에서 hit. |
| `'한국어'` | 5 | 10 | morpheme + N-gram 양쪽 매칭으로 hit count 증가 (raw `한국어` token + N-gram supplement) |
| `'서울'` | 0 | 0 | KB corpus 에 단어 자체 부재 (data limitation, V009 limitation X) |
| `'지하철'` | 0 | 0 | 동일 |
| `'token'` (영어) | 10 | 10 | KB 의 OAuth/JWT docs — whole-token 매칭. supplement 미적용. |
| `'tokenizer'` | 0 | 0 | KB 에 부재 |
| `'pipeline'` | 10 | 10 | data ingest pipeline docs |
| `'config'` | 10 | 10 | config-related docs |

**핵심 결론**: Bug #8 의 **functional closure 검증** — V007 trigram 의 `'한국'` 0 hit limitation 이 V009 + N-gram supplement 로 **10 hit** 으로 개선. 다른 한국어 query 의 0-hit 는 corpus 의 단어 자체 부재 (KB 가 React/Cargo/MD docs 위주). 실제 한국어 content 가 더 많은 KB (예: 한국 정부 docs, K-wiki) 에서는 더 큰 benefit 기대.

**Snippet evidence (ko-dic 분해 + N-gram window)**:
```
testdata/coding-md-corpus/security/security-310-item.md
  → "¶ 문서 를 한국어 한국 국어 로 다시 정리 하 기"
testdata/coding-md-corpus/rust/rust-020-functions.md
  → "Functions 문서 를 한국어 한국 국어 로 다시 정리 하 기"
```

`한국어` morpheme + sliding window `한국` + `국어` 가 동시에 chunks_fts 에 indexed — `'한국'` query 가 morpheme 분해 결과의 부분 token 으로 hit.
- README + SKILL.md + HANDOFF.md 세 문서 반영.

Cross-link: `migrations/V009__fts_korean_morphological.sql`, `crates/kebab-search/src/lexical.rs`, design §5.5 / §9, `docs/superpowers/specs/2026-05-28-v0.20.x-korean-morphological-tokenizer-spec.md`.

## 2026-05-28 — PDF OCR `request_timeout_secs` default 60s → 180s (Bug #11 follow-up)

**Discovered**: v0.20.0 final dogfood (2026-05-28), round 3 fresh KB ingest.

**Symptom**: 60s default 가 metro-korea.pdf 의 page 8/9/13 (dense Korean text + Identity-H CID font) 의 OCR 을 강제 timeout. round 2 (600s default) 의 page 8/13 (2 page) → round 3 (60s) 의 page 8/9/13 (3 page) — **1 page 더 timeout** + 본문 indexed 손실 증가. 사용자 perspective: cost 절약 vs coverage trade-off 가 60s 에선 coverage 쪽으로 너무 깎임.

**Decision**: **conservative starting point 180s 로 재조정** + dogfood evidence 기반 sweet spot 점진적 축소 정책. 180s 가 600s 의 1/3 cost cap (page 당 max 3분) 보장 + dense page coverage 회복.

**Fix** (HOTFIXES 2026-05-28 follow-up, branch `feat/pdf-scanned-ocr`):
- `crates/kebab-config/src/lib.rs::default_pdf_ocr_request_timeout_secs() = 180`.
- Doc-comment 보강 — "180s starting point + sweet spot 점진적 축소" 명시.
- Unit test rename `pdf_ocr_request_timeout_default_is_60s` → `_is_180s` + assertion 180.
- `crates/kebab-config/tests/pdf_ocr.rs` 의 `assert_eq!(...request_timeout_secs, 60)` → `180`.
- User override path 보존 — `config.toml [pdf.ocr] request_timeout_secs = N` 로 늘리/줄이기 가능 (이미 `#[serde(default)]` field, 변경 0).

**Future tuning policy**: 향후 dogfood 마다 OCR 평균 ms 분포 측정 후 (예: 90th percentile + buffer) default 점진적 축소. 60s 같은 짧은 default 로 직접 jump 안 함.

## 2026-05-27 — PDF OCR `request_timeout_secs` default 600s → 60s (Bug #11, superseded by 2026-05-28 follow-up)

**Discovered**: v0.20.0 final dogfood (2026-05-27), metro-korea.pdf 의 page 8 + 13.

**Symptom**: 두 page 모두 `kebab ingest` 가 600s 까지 완전 timeout (`ms: 600000, chars: 0, skipped: true`). 본문 indexed 안 됨, page 당 20분 cost 낭비, user 가 ingest 완료 signal 못 받음.

**Root cause**: `default_pdf_ocr_request_timeout_secs() = 600` (spec `2026-05-27-pdf-scanned-ocr-spec.md` line 1000 + OQ-1 line 1628 의 "CPU 환경 105s 의 5x 여유" 가정). 실측 cloud GPU Ollama 의 per-page throughput 는 6-32s — 600s 까지 가야 timeout 이라면 Ollama 다운 상태가 사실상 확실. 600s 가 fail-fast 신호로 작동 안 함.

**Fix** (v0.20.0 bugfix3 round 3, branch `feat/pdf-scanned-ocr`):
- `crates/kebab-config/src/lib.rs` `default_pdf_ocr_request_timeout_secs() = 60` (2026-05-28 entry 에서 180 으로 재조정).
- Doc-comment 보강 — 6-32s 정상 throughput, 60s 초과는 Ollama 다운 / 매우 dense·고해상도 page 신호.
- User override path 보존 — `config.toml [pdf.ocr] request_timeout_secs = N` 로 늘릴 수 있음.

**Amends**: `docs/superpowers/specs/2026-05-27-pdf-scanned-ocr-spec.md` line 1000 / OQ-1 line 1628 (frozen — text 변경 없음, inline HTML 주석 cross-link 1 줄만 추가). 본 entry 가 live source of truth.

## 2026-05-27 — Identity-H mojibake marker bypassed OCR fallback (Bug #6)

- **Symptom**: `metro-korea.pdf` (Identity-H CID font without ToUnicode CMap) 의 ingest 가 `pdf_ocr_pages=0` 으로 종료. text 전체가 `?Identity-H Unimplemented?` marker 1154회 반복 (lopdf 0.32.0 emit). text-detect ratio = 1.0 → OCR fallback threshold 0.5 bypass.
- **Root cause**: `crates/kebab-parse-pdf/src/text_quality.rs::compute_valid_char_ratio()` 의 `is_valid_text_char()` 가 ASCII printable range (0x0020..=0x007E) 를 unconditional valid 처리. marker (28 ASCII char) 는 valid 로 count.
- **Fix**: `MOJIBAKE_MARKERS` const 도입 + marker strip after-strip 의 trim-empty → 0.0 + dominance heuristic (strip > 잔여 일 때 cap 0.3). spec ACCEPT: `docs/superpowers/specs/2026-05-27-v0.20-sub1-bugfix2-spec.md` §4.1. parser_version/wire schema 영향 0.
- **User action**: 이미 `metro-korea.pdf` class 의 mojibake-heavy PDF 를 v0.20.0 pre-bugfix2 binary 로 indexed 한 경우, `kebab ingest --force-reingest <workspace>` 로 cached skip 무효화 필요 (release notes 동등 안내).

## 2026-05-27 — v0.20.0 sub-item 1: chunk_id `#c{char_start}` workaround collapses under aggressive overlap (Bug #3 second-iteration patch)

**Symptom**: F2 (1580 chars OCR, scanned_page2.pdf) ingest 시
`DocumentStore::put_chunks (pdf): sqlite error: UNIQUE constraint
failed: chunks.chunk_id: ... Error code 1555: A PRIMARY KEY constraint
failed`. `kebab v0.20.0` (commit `b4d9e60`) dogfood (qwen2.5vl:3b 의
`192.168.0.47:11434` Ollama endpoint, `/build/cache/tmp/v0.20-dogfood`
isolated KB) `--force-reingest` 마다 reproducible.

**Root cause**: `crates/kebab-chunk/src/pdf_page_v1.rs:170` 의
`per_chunk_hash = format!("{base_policy_hash}#c{char_start}")` 에서
`char_start` = post-overlap `actual_start`. line 266-281 의 overlap
walk 가 `prev_min` floor 까지만 back-walk 하므로 aggressive overlap
+ 첫 segment 가 작은 page (F2 의 한국어 OCR text: 첫 ~10 char 안
sentence-end → segment_1 = [0, 30], segment_2 = [30, n], overlap_bytes
240 / chars=80 → segment_2 의 actual_start 가 prev_min=0 으로
collapse) → 두 chunk 의 `#c0` suffix identical → identical chunk_id →
`chunks` PRIMARY KEY violation.

**Fix** (spec §4.4): `chunk_page` return tuple 에 `segment_start`
추가 (3-tuple → 4-tuple `(segment_start, actual_start, chunk_end,
slice)`), caller `per_chunk_hash` 의 suffix 를 `segment_start` 로
변경. `segment_start` 는 `bounds[seg_idx]` (dedup 후 strictly
increasing) — overlap walk 와 무관하게 모든 chunk distinct. citation
locality 의 `SourceSpan::Page.char_start` 는 여전히 post-overlap
`actual_start` 유지.

**chunker_version cascade**: `pdf-page-v1` → `pdf-page-v1.1` bump
(spec §4.4.1 round 1c M-1 결정, design §9 cascade rule 의 직접 적용).
multi-chunk PDF page (pre-OCR 시점 `metro-korea.pdf` 의 21 block /
34 chunk 같은 정상 path) 의 chunk_id 가 변경 — explicit user-facing
audit trail 확보, store layer 의 자동 invalidation report. v0.20.0
force-update path 라 사용자 cost zero (어차피 fresh ingest).

**Amends**: spec `docs/superpowers/specs/2026-05-27-v0.20-sub1-bugfix-spec.md`
§4.4. parent design §4.2 chunk_id recipe 자체 unchanged (workaround
layer 의 internal computation 만 변경). parent PR #189
(`feat/pdf-scanned-ocr`, force-update path).

## 2026-05-26 — design deviation — kebab-normalize + kebab-parse-types 흡수 (24 → 22 crates)

**Symptom**: design deviation — post-PR9 audit (system-architect, `tasks/INDEX.md` L169) identified 두 crate (`kebab-normalize` + `kebab-parse-types`) 가 dead abstraction. design §3.7b 의 "thin layer" raison d'être ((a) `kebab-core` namespace 폭발 방지, (b) normalize 의 parser non-dependence) 가 4 parser 중 1개 (markdown) 만 lift 를 경유하는 현실에서 fan-in/fan-out 모두 1 → layer 의미 잃음. `kebab-parse-types` 의 production caller 가 2개 (`kebab-parse-md` + `kebab-normalize`) 이고 `kebab-normalize` 자체 caller 가 1개 (`kebab-app`) — 모두 markdown 의 lift 경로 안에서 단일 fan-in 경계 가능.

**Root cause**: P1~P10 머지를 거치며 `kebab-parse-pdf` (P7) / `kebab-parse-image` (P6) / `kebab-parse-code` (P10) 가 `CanonicalDocument` 직접 emit 패턴으로 정착. `kebab-normalize::build_canonical_document` 는 markdown-specific `Vec<ParsedBlock>` → `CanonicalDocument` lift 만 책임. design §3.7b 가 가정한 "ParsedBlock 류는 모든 parser 가 emit → normalize 가 일괄 lift" 의 fan-in ≥ 2 시나리오가 미도래 — 그러나 layer 비용 (24 crate workspace, 두 crate 의 lib.rs only structure) 은 계속 지불.

**Action**: `kebab-normalize` (1097 LOC) + `kebab-parse-types` (98 LOC) 를 `kebab-parse-md` 에 흡수 — 22 crate workspace.

- `crates/kebab-parse-md/src/types.rs` (신규): `kebab-parse-types/src/lib.rs` 의 98 LOC 1:1 이식 (5 사용 type + 3 forward-declared struct 보존).
- `crates/kebab-parse-md/src/normalize.rs` (신규): `kebab-normalize/src/lib.rs` 의 production fn body (`build_canonical_document`, `derive_title`, `warning_agent`) 이식. `warning_agent` 의 return string ("kb-normalize") 보존 — SQLite `documents.provenance_json` 의 audit log 일관성 (wire-invisible, see spec §1.9).
- 3 dead struct (`ParsedImageRegion` / `ParsedPdfPage` / `ParsedAudioSegment`) 는 보존 — v0.20+ image/pdf normalize integration 의 future surface (spec §11 참조).
- `crates/kebab-parse-md/src/lib.rs`: `pub use crate::types::{...}; pub use crate::normalize::{build_canonical_document, derive_title};` re-export 추가.
- `crates/kebab-parse-md/src/{blocks,frontmatter}.rs`: `use kebab_parse_types::*` → `use crate::types::*`.
- `crates/kebab-app/src/lib.rs:51`: `use kebab_normalize::build_canonical_document` → `use kebab_parse_md::build_canonical_document` (line 55 의 기존 use list 와 통합). line 1119 context string `kb-normalize::build_canonical_document` → `kb-parse-md::build_canonical_document`.
- `crates/kebab-app/Cargo.toml`: `kebab-normalize` regular dep 제거 + `kebab-parse-types` regular dep 제거 (후자는 dead dep — `cargo tree -p kebab-app | grep kebab_parse_types` 0줄 검증으로 incidental cleanup).
- `crates/kebab-chunk/Cargo.toml` + `crates/kebab-store-sqlite/Cargo.toml`: `[dev-dependencies] kebab-normalize` 제거. 통합 test source (`tests/long_section_snapshot.rs:21` + `tests/contract_roundtrip.rs:16`) 의 `use kebab_normalize::build_canonical_document` → `kebab-parse-md` use list 통합.
- `crates/kebab-normalize/tests/normalize_snapshot.rs` → `crates/kebab-parse-md/tests/normalize_snapshot.rs` (mechanical move + use shift).
- `Cargo.toml` workspace.members: `kebab-normalize` + `kebab-parse-types` entries 제거. `workspace.package.version` 0.18.0 → **0.19.0** (frozen design contract 변경 trigger — CLAUDE.md "Release / binary version bump").
- `crates/kebab-normalize/` + `crates/kebab-parse-types/` 디렉토리 전체 삭제 (`git rm -r`).
- `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` §3.7b 재작성 (보존 + future re-extraction trigger 명시) + §8 graph 갱신 (3 edge 제거 + 2 forbidden bullet 의미 갱신).
- `docs/ARCHITECTURE.md` crate graph + 디렉토리 tree mechanical 갱신.
- `tasks/INDEX.md` L169 의 "kebab-normalize 흡수" defer mention 해소 + "Future work / deferred" 섹션 신설 (image/pdf normalize integration entry).

**Amends**: spec `docs/superpowers/specs/2026-05-26-normalize-absorption-spec.md` cross-link. design `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` §3.7b + §8 동시 갱신 (CLAUDE.md "Changing the design doc requires updating every referencing task spec in the same PR" — 본 PR 의 design 갱신은 ~25 referencing task spec 의 raison d'être 인용을 stale 화하지만, frozen 원칙에 따라 mechanical update 없음. live source of truth = 본 HOTFIXES entry). 영향받는 task spec 의 `Forbidden dependencies` 또는 `contract_sections: ["§3.7b"]` 인용은 historical contract 로 보존됨 — `tasks/p1/p1-2-parser-types.md`, `tasks/p1/p1-3-markdown-parser.md`, `tasks/p1/p1-4-normalize.md`, `tasks/p9/p9-fb-07-md-title-fallback.md` 등. (Wire / surface impact: 0건 — CLI / TUI / MCP / `--json` 출력 / config / XDG path / parser_version 모두 unchanged. wire-invisible `provenance.events[].agent` 의 stage label "kb-normalize" 도 보존 — old DB row 와 new DB row 의 audit log 일관성.)

## 2026-05-26 — S3 NLI unavailable — hypothesis truncate + token-count fallback

**Symptom**: S3 dogfood query (`"Why does kebab combine multilingual-e5, LanceDB, and RRF together?"`) 가 NLI 활성 (`rag.nli_threshold > 0`) 시 `nli_model_unavailable` 일관 fail. `~/.local/state/kebab/logs/kb.log.2026-05-26` 의 5 회 WARN 라인 `tokenizer.encode failed: Truncation error: Sequence to truncate too short to respect the provided max_length`.

**Root cause**: `crates/kebab-nli/src/onnx.rs::load_tokenizer` 가 mDeBERTa-v3 tokenizer 의 truncation strategy 를 `OnlyFirst` 로 설정 — *2-sequence input* 에서 첫 번째 sequence (premise) 만 truncate. LLM 의 949-token (4564-char) 장문 답변 = hypothesis 단독이 512-token cap 초과 → premise 를 0 까지 잘라도 fit 시킬 방법 없음 → tokenizer `SequenceTooShortToTruncate` raise. `pipeline.rs::ask_multi_hop` 의 step 8.5 hook 이 premise-side `truncate_for_nli` 만 적용, hypothesis-side 무방비.

**Action**: 4 production files + 2 test files + 1 doc entry — Option A (KR-safe + graceful fallback + symmetric layering).

- `crates/kebab-nli/src/lib.rs`: `NliVerifier::hypothesis_token_count(&self, &str) -> anyhow::Result<usize>` trait method 추가 (default `Ok(0)` backward-compat).
- `crates/kebab-nli/src/onnx.rs`: `OnnxNliVerifier::HYPOTHESIS_TOKEN_BUDGET = 256` inherent const + `impl NliVerifier for OnnxNliVerifier {}` block **안** 에서 `hypothesis_token_count` override (real `tokenizer.encode` probe). **CRITICAL**: trait impl block 안 위치 필수 — inherent `impl OnnxNliVerifier {}` 안에 두면 vtable 미등록 → trait dispatch 시 default `Ok(0)` → retry loop 즉시 통과 → production silent NO-OP.
- `crates/kebab-rag/src/pipeline.rs`: `MAX_NLI_HYPOTHESIS_CHARS_INITIAL = 1200` + `MAX_NLI_HYPOTHESIS_CHARS_MIN = 150` consts + `pub(crate) fn truncate_chars` pure-fn (codepoint-aware) + `pub fn truncate_hypothesis_for_nli_with_budget` retry helper (char budget 1200 → 600 → 300 → 150 절반화 retry + min floor 미달 시 `anyhow::bail!`) + step 8.5 hook 의 callsite explicit `match { Ok => x, Err => return refuse_nli_model_unavailable(...) }` (—`?` propagation 금지: wire `answer.v1 + NliModelUnavailable refusal` 유지). `v.score(&truncated_premise, &acc)` → `v.score(&truncated_premise, &truncated_hypothesis)`.
- `crates/kebab-rag/tests/common/mod.rs`: `SpyNliVerifier` closure-based helper (2-arg constructor: `score_fn` + `token_count_fn`, `Mutex<Vec<String>>` capture). 기존 `MockNliVerifier` (고정 mode) 와 sibling.
- `crates/kebab-nli/tests/inference.rs`: 2 신규 `#[ignore]` tests — `score_long_en_hypothesis_returns_err_without_pipeline_truncation` (raw nli crate 의 OnlyFirst dead-end pin) + `hypothesis_token_count_dispatches_correctly_via_dyn_trait` (vtable dispatch pin — `&dyn NliVerifier` 통해 호출, EN/KR bounded range assertion, RC1-residual silent NO-OP regression pin).
- `crates/kebab-rag/tests/multi_hop_nli_truncate.rs` (신규): 3 mock multi-hop tests — `long_en_synth_answer_truncated_before_nli_call` (EN happy + Right direction pin) + `long_kr_synth_answer_retries_with_smaller_budget` (KR retry path + ≤ 300 chars 최종) + `unrelenting_token_overflow_falls_through_to_unavailable` (graceful unavailable fallback pin, score_fn 은 `unreachable!()`).
- `crates/kebab-rag/src/pipeline.rs::#[cfg(test)] mod tests`: 4 `truncate_chars` boundary tests (identity / truncate / empty / KR codepoint).

**Amends**: spec `docs/superpowers/specs/2026-05-26-s3-nli-model-unavailable-diagnose-spec.md` (4 round APPROVE) cross-link. task spec `tasks/p9/p9-fb-41-multi-hop-finalize.md` 의 NLI 동작 추가 보강 (hypothesis-side budget 신규 — frozen task spec 자체는 변경 안 함, 본 entry 가 live source of truth). HOTFIX 번호 미부여 — sibling fb-41 PR-9 closure layer 의 production behavior follow-up (HOTFIX #15 의 fixture-issue 와 다른 layer).

## 2026-05-26 — HOTFIX #15 — MCP ask multi_hop dispatch-divergence assertion stale (fixture 보강)

**Symptom**: PR-7 (multi-hop probe-first dogfood fix) 머지 후 `kebab-mcp::tools_call_ask_multi_hop::ask_tool_routes_multi_hop_true_to_decompose_first` 가 모든 workspace test 에서 deterministic fail (no_chunks short-circuit 으로 `is_error=Some(false)`).

**Root cause**: PR-5 의 test 가 *empty KB → multi-hop 은 decompose first → LLM 도달* 의 stale contract 에 assert. PR-7 의 pre-decompose probe 가 빈 KB → refuse_no_chunks short-circuit.

**Action**: test fixture 보강 — `minimal_config.score_gate = 0.0` + workspace_root 에 `note.md` ("This note is about a compound containing X and Y in detail.") ingest → probe 통과 → decompose → unreachable LLM → `error.v1` 의 원래 dispatch divergence 회복. + 신규 `_multi_hop_short_circuits_when_probe_empty` test 1개 (probe-empty short-circuit 의 MCP-layer wire pin 안전망). + module doc rewrite.

**Amends**: spec `docs/superpowers/specs/2026-05-26-hotfix-15-mcp-ask-multi-hop-flaky-spec.md` cross-link. production code 0 touch (PR-7 의 probe-first 는 의도된 동작 유지).

## 2026-05-25 — fb-41 pre-v0.18 dogfood: multi-hop score-gate 우회 (S7 hallucination 회귀 핀)

v0.18.0 cut 전 fb-41 multi-hop RAG 도그푸딩 (`/build/cache/dogfood-v018/`, 33 assets / 205 chunks corpus — 16 신규 markdown 5 클러스터 + v017 carryover, gemma3:4b CPU only / 16 GB RAM) 에서 발견된 **score_gate 우회 + hallucination 케이스**.

### Symptom (S7)

Query: `"What is the chemical formula of caffeine?"` (KB 에 없는 fact).
- **Single-pass** `kebab ask`: retrieve 의 top score 가 default `rag.score_gate = 0.30` 미만 → `refuse_score_gate` → 안전한 refusal.
- **Multi-hop** `kebab ask --multi-hop`: `grounded = true`, 본문 `"카페인의 화학식은 C₉H₁₅N₃O 입니다 [#6]"` (**hallucination** — 실제 C₈H₁₀N₄O₂) + `[#6]` 가 *Adam optimizer chunk* 의 `g_t = ∂L/∂θ_i` 본문을 인용 (시각적으로 화학식 비슷한 short structured token 매칭 trigger).

### Root cause

`ask_multi_hop` (`crates/kebab-rag/src/pipeline.rs`) 의 score-gate 검사가 *pool 의 top_score* 만 봄. multi-hop 의 pool 은 *5 sub-queries 의 union* — 한 sub-query 의 top score 가 gate 위면 pool 의 다른 chunks 가 원본 query 와 무관해도 gate 통과 + synthesize 단계 진입 → LLM 이 chunks 위에서 hallucinate.

같은 query 에 대해 single-pass 는 *원본 query 의 retrieve top score* 검사 → reject. multi-hop 만 hole.

### Fix (PR-7)

`ask_multi_hop` entry 에 **pre-decompose probe** 추가:
1. *원본 query* 로 retrieve 한 번 (LLM call 0회, ~ms).
2. probe empty → `refuse_no_chunks(None)` (decompose 안 함, hop trace 도 없음).
3. probe top_score < gate → `refuse_score_gate(None)` (decompose 안 함).
4. probe pass → 기존 decompose / decide / synthesize flow 그대로.

Multi-hop 의 safety floor 가 single-pass 와 정확히 일치 — multi-hop 은 *원본 query 가 이미 KB 범위 내* 일 때만 cross-doc reasoning 추가.

### Test 갱신

- 신규 3 회귀 핀 (`crates/kebab-rag/tests/multi_hop.rs`):
  - `multi_hop_below_probe_gate_refuses_before_any_llm_call` — S7 회귀 직접 핀. low-score chunk + empty LM script → score_gate refusal, LM calls 0회, hops=None.
  - `multi_hop_empty_probe_pool_refuses_before_any_llm_call` — empty retrieve 시 NoChunks refusal, LM calls 0회.
  - `multi_hop_above_probe_gate_proceeds_to_decompose` — probe pass 시 full multi-hop flow (decompose + decide + synth) 정상 동작.
- 기존 7 multi-hop test 의 `ScriptedRetriever` 에 probe entry prepend + `retriever_handle.calls()` expectation +1.
- `multi_hop_refuse_no_chunks_preserves_hops_trace` / `multi_hop_refuse_score_gate_preserves_hops_trace` 의 의미 좁힘 — 이제 *decompose-driven* refusal (probe pass 후 sub-query retrieve 가 empty 또는 below-gate) 만 검증. *probe-driven* refusal 은 hops=None (decompose 안 함) — 신규 test 가 그 path 핀.

### 다른 도그푸딩 발견 (별 fix 보류)

같은 도그푸딩에서 발견된 다른 항목들 — PR-7 본 fix 의 scope 밖, v0.18.1 또는 후속 PR 대상:

- **synthesize citation marker 일관성 부족** (S1/S2/S3, P1) — 30-chunk pool 의 large prompt 에서 gemma3:4b 가 `[#N]` citation rule 잃음 → 답변 본문 정상이나 `grounded=false (LlmSelfJudge)` 로 노출. **권장 mitigation**: `MULTI_HOP_SYNTHESIZE_SYSTEM_PROMPT` 의 citation rule 강화 또는 `multi_hop_max_pool_chunks` default 30 → 15. → **closure (부분)**: 아래 *post-PR-7 dogfood retest + PR-8* 절. pool 30 → 15 ship.
- **latency 20-25× cost** — single-pass 30s vs multi-hop 590-685s (synthesize 단계가 cost dominant). spec 의 "2-5× LLM cost" 보다 큼. pool size 축소가 mitigation. → **closure**: PR-8 의 pool 15 로 614s → 158s (4× 개선) 확인.
- **release binary path confusion** — `/home/altair823/kebab/target/release/kebab` (v0.17.1 stale) vs `/build/out/cargo-target/release/kebab` (v0.17.2 latest). CARGO_TARGET_DIR env 의 영향. docs 한 줄 권장.

### post-PR-7 dogfood retest + PR-8 partial mitigation

PR-7 머지 후 같은 dogfood S7 (`What is the chemical formula of caffeine?`) 시나리오 재검증:

| metric | single-pass | multi-hop pre-fix | multi-hop PR-7 | multi-hop PR-8 |
|---|---|---|---|---|
| grounded | false (LlmSelfJudge) | true ✗ | true ✗ | true ✗ |
| latency | ~30s | 141s | 143s | **158s** (4× 개선) |
| top_score / gate | 0.5 / 0.30 | 0.5 / 0.30 | 0.5 / 0.30 (probe pass) | 0.5 / 0.30 (probe pass) |
| pool size | n/a | 30 | 30 | **15** |
| answer | "근거가 부족하다" ✓ | hallucination | hallucination | hallucination (LLM 새 rule 무시) |

**PR-7 의 probe gate not enough**: hybrid mode 의 RRF top_score = 0.5 (gate 0.30 위) — caffeine vector 유사도가 *어떤* chunk 와 매칭 (Adam optimizer 의 `g_t = ∂L/∂θ` 시각적 short structured token) → probe gate 통과 → synthesize 진입 → hallucination.

**PR-8 의 변경** (`crates/kebab-config/src/lib.rs` + `crates/kebab-rag/src/pipeline.rs`):
- `multi_hop_max_pool_chunks` default **30 → 15**. synthesize prompt size 축소 → latency 4× 개선.
- `MULTI_HOP_SYNTHESIZE_SYSTEM_PROMPT` 에 **답하기 전 self-check** rule 추가 — 원본 question 의 핵심 entity 가 [근거] 본문에 literal 없으면 즉시 "근거가 부족하다".

**PR-8 의 한계**: gemma3:4b 가 prompt rule 무시. strong rule + small pool 도 hallucination 차단 못함. **LLM-self-judge 기반 safety 의 ceiling** 명확.

### PR-9 — NLI-based post-synthesis verification (완료, 2026-05-26)

학계 / industry 표준 (Self-RAG, CRAG, Auto-GDA, MedTrust-RAG) 결론: deterministic post-synthesis verification 이 정답. **mDeBERTa-v3-base-xnli-multilingual ONNX model (280 MB)** 가 `(premise = packed_chunks, hypothesis = answer)` entailment 검사 → score < threshold 면 refuse. PR-8 위에 layered defense. design note: `/build/cache/dogfood-v018/results/PR-9-DESIGN.md`. 단계적 PR (9a / 9b / 9c-1 / 9c-2 / 9d) 모두 머지.

**Sub-PR 시퀀스 (모두 머지)**:
- PR #176 (PR-9a): `kebab-nli` crate skeleton — trait surface + workspace deps.
- PR #177 (PR-9b): `OnnxNliVerifier` ONNX inference + model download (lazy `OnceLock`, OnlyFirst truncation).
- PR #178 (PR-9c-1): wire surface — `RefusalReason::Nli{Verification,Model}Failed/Unavailable`, `Answer.verification`, `RagPipeline.verifier` field + builder, `[rag] nli_threshold` + `[models.nli]` config.
- PR #179 (PR-9c-2): `ask_multi_hop` step 8.5 NLI hook 활성화 + `App::open_with_config` 의 NliVerifier construction + 5 mock multi-hop tests.
- PR-9d: dogfood retest + 본 closure (별 PR — fb-41 multi-hop NLI 검증).

**Dogfood retest 결과** (2026-05-26, `/build/cache/dogfood-v018/results/post-pr9/`, repo 보존 = `docs/dogfood/v0.18.0/`):

| case | PR-8 baseline | PR-9 retest | 판정 |
|---|---|---|---|
| **S7** (caffeine) | `grounded=true, refusal_reason=null`, **답변=Adam gradient 공식 (hallucination)** | `refusal_reason=nli_verification_failed`, `nli_score=0.0035` | ✅ **HALLUCINATION FIXED** |
| S1 (compiler) | `refusal_reason=llm_self_judge` | `refusal_reason=nli_verification_failed`, `nli_score=0.058` | ✅ 둘 다 reject, NLI 더 deterministic |
| S3 (kebab EN) | `refusal_reason=llm_self_judge` | `refusal_reason=nli_model_unavailable` (consistent) | ⚠ follow-up entry (다음 sub-section) |
| S10 (dinosaur) | `refusal_reason=llm_self_judge` | `refusal_reason=nli_verification_failed`, `nli_score=0.0028` | ✅ 둘 다 reject, NLI 더 deterministic |

PR-9 의 핵심 목표 (S7 silent hallucination root cause 해결) ✅ **달성**. LLM-self-judge 의 *probabilistic ceiling* 을 NLI 의 *deterministic external verifier* 가 극복.

**RAM peak**: PR-8 ~5-6 GB → PR-9 ~7-8 GB (gemma3:4b + ONNX session ~600 MB). 16 GB 환경 안전.

**Disk**: NLI model cache 1.1 GB (model 280 MB + tokenizer 16 MB + hf-hub blobs/locks/snapshots overhead). user XDG (`~/.local/share/kebab/models/nli/`) 또는 config 의 `storage.model_dir`.

### PR-9d 의 S3 follow-up (kebab-nli `nli_model_unavailable` consistent fail)

**Symptom**: S3 query ("Why does kebab combine multilingual-e5, LanceDB, and RRF together?") 가 *consistent* (재시도 2회 모두) `nli_model_unavailable` 로 fail. 다른 case (S1/S7/S10) 의 entailment 측정은 정상 — NLI infrastructure 자체는 작동. S3 만 특정 input 의존 fail.

**Diagnosis 시도**: `KEBAB_LOG=info,kebab_rag=debug,kebab_nli=debug` 로 retry — *debug log emit 안 됨* (env 이름 ignored 또는 tracing subscriber init 안 됨). stderr 비어 있어 graceful refuse path 만 확인.

**Hypothesis** (확정 안 됨):
- mDeBERTa session inference 가 *S3 의 특정 packed_text shape* 에 대해 err (encode 단계 또는 ort Session::run shape 검증).
- 또는 *eager session reload* 가 process invocation 단위 의 race.

**임시 대응**: 사용자가 `[rag] nli_threshold = 0` 로 disable 가능. release notes 의 known limitations 명시.

**Next step**: v0.18.1 candidate — tracing 의 env 이름 검증 (`RUST_LOG` 또는 `KEBAB_TRACING_LEVEL` 등) + S3 packed_text shape 분석 (chunks 개수, char count, language mix). HOTFIX 진단 후 별 PR.

**Amends**: 없음 (PR-9 의 known limitation, v0.18.1 candidate).

### PR-9 NLI refusal: terminal Synthesize hop omitted from hops trace

**Symptom**: multi-hop `ask` 가 step 8.5 NLI gate 에서 refuse 시 (`RefusalReason::NliVerificationFailed` / `NliModelUnavailable`) `Answer.hops` 는 decompose+decide chain 까지만 담겨 있고, synthesize 가 실제로 LLM 호출 + 토큰 누적까지 끝났음에도 terminal `Synthesize` `HopRecord` 가 append 되지 않는다. happy path (refuse 안 함) 의 `hops` trace 는 `Synthesize` 항을 포함하므로 두 surface 사이 trace shape 가 비대칭.

**Action**: 본 entry 가 tracker. `crates/kebab-rag/src/pipeline.rs` 의 `refuse_nli_verification` / `refuse_nli_model_unavailable` 진입 직전 (step 8.5) 에서 `Synthesize` `HopRecord` (with `forced_stop = false`, synth `usage` / `elapsed_ms` 동봉) 를 append 후 refuse 호출하도록 정리.

**Next step**: v0.18.1 candidate — wire 비대칭이 explain / TUI hops 표시에 사용자 노출되는지 도그푸딩에서 확인 후 fix. 시급도 낮음 (`hops` 는 trace/debug 용, grounded/refusal 결정에는 무관).

**Amends**: 없음 (v0.18.1 candidate). Cross-link: `crates/kebab-rag/src/pipeline.rs` (refuse_nli_* call sites 직전).

### 사용자 영향

PR-7 + PR-8 머지 후 (v0.18.0 cut 직전):
- multi-hop 의 safety floor 는 single-pass 와 동일 (PR-7 probe gate). probe gate 가 reject 하는 query 는 hallucination 못 받음.
- 그러나 hybrid RRF 가 *weak match* 라도 gate 통과 시 (vector embedding 의 false positive) hallucination 가능 — PR-9 NLI 가 진짜 close.
- Multi-hop 정상 use case (compound / cross-doc reasoning) 영향 없음.
- Wire 변경 없음 (PR-8 의 pool default 만 변경, additive config 영향).

Cross-link: `/build/cache/dogfood-v018/results/SUMMARY.md` (전체 dogfood 보고서), `/build/cache/dogfood-v018/results/PR-9-DESIGN.md` (NLI design note), spec `docs/superpowers/specs/2026-05-25-p9-fb-41-multi-hop-rag-design.md`.

## 2026-05-25 — v0.17.0 post-dogfood: `[models.llm] request_timeout_secs` 노브 + 권장 모델 가이드

v0.17.0 후속 도그푸딩에서 발견: 사용자가 default `gemma4:e4b` (8B Q4, 9.6 GB) 를 CPU only / 16 GB RAM 환경에서 시도 시 첫 RAG 답변이 5 분 (hard-coded 300 s) 한도를 항상 넘겨 `error: kb-rag: llm.generate_stream` 으로 떨어졌다. 메모리도 ollama RSS 10.7 GB / free 2 GB 까지 압박. 후속 도그푸딩 32 분 / 199 mem-monitor sample 결과는 `tasks/HOTFIXES.md` 의 본 entry 와 conversation 의 도그푸딩 보고 참조.

**변경**:
- `crates/kebab-config/src/lib.rs::LlmCfg` 에 `request_timeout_secs: u64` additive 필드 (`#[serde(default = "default_llm_request_timeout_secs")]`, default `300`). 옛 config 가 필드 누락해도 그대로 파싱 + 동일 동작 (3 신규 unit test 가 default / env override / legacy parse 핀).
- env override `KEBAB_MODELS_LLM_REQUEST_TIMEOUT_SECS`.
- `crates/kebab-llm-local/src/ollama.rs` 의 `REQUEST_TIMEOUT` 상수 제거. `OllamaLanguageModel::new` 가 `Duration::from_secs(llm.request_timeout_secs)` 로 reqwest blocking client 빌드. doc comment 도 동일하게 갱신.
- `README.md` 사전 요구 절 + `docs/SMOKE.md` 의 ollama 안내에 권장 모델 (≤ 4B Q4 — `gemma3:4b` / `qwen2.5:3b` / `phi3:mini`) + timeout 노브 anchor 한 줄. 8B+ 시도 시 timeout 패턴 사전 안내.
- `crates/kebab-config/src/lib.rs::Config::defaults` 의 LlmCfg literal 에 `request_timeout_secs: default_llm_request_timeout_secs()` + comment 한 줄로 CPU only 권장 안내.

**미진행 (scope 밖) — closure 갱신**:
- ~~`crates/kebab-parse-image/src/ocr.rs::REQUEST_TIMEOUT` 도 동일한 hard-coded 300 s — OCR 이 보통 짧아 LLM 만큼 부담 안 되지만, 일관성 측면에서 다음 round 에 같은 노브 (또는 별 노브) 로 재검토.~~ → **closure**: 아래 2026-05-25 v0.17.2 OCR timeout entry 참조 (별 노브 `[image.ocr] request_timeout_secs` 신설, PR #164).
- ~~`kebab ask --stream` (fb-33) 권장 강조: 5분 cold-start 동안 첫 token 빠르게 surface — UX 개선. README/SKILL.md 추가 한 줄 후속.~~ → **closure**: PR #163 (v0.17.1 cut) 에서 이미 README + SMOKE + SKILL.md 세 곳 모두 추가됨 (`README.md:22` cold start 권장 단락, `docs/SMOKE.md:45/209` 예제, `SKILL.md:114/119` 사용 가이드). 본 entry 의 미진행 표기가 outdated 였음.

**후속 도그푸딩 baseline 보존**: `/build/cache/dogfood-v017/` (466 MB workspace + DB + memory.log), `/build/cache/ollama/` (21 GB binary + gemma3:4b/gemma4:e4b 모델). 다음 round 회귀 비교용.

Cross-link: `crates/kebab-config/src/lib.rs::LlmCfg::request_timeout_secs`, `crates/kebab-llm-local/src/ollama.rs::OllamaLanguageModel::new`.

## 2026-05-25 — v0.17.2: `[image.ocr] request_timeout_secs` 노브 (closure of v0.17.1 미진행, PR #164)

v0.17.1 entry 의 첫 번째 미진행 항목 closure. LLM 쪽이 v0.17.1 에서 `[models.llm] request_timeout_secs` 로 풀려난 패턴을 OCR 어댑터에 동일 적용. 별 노브로 분리한 이유 (사용자 결정): OCR 은 통상 LLM 대비 짧고 cold start 패턴도 다름 — 두 노브를 독립 조절할 수 있어야 16 GB / CPU only 환경에서 vision 모델만 다른 timeout 을 쓰기 편함.

**변경**:
- `crates/kebab-config/src/lib.rs::OcrCfg` 에 `request_timeout_secs: u64` additive 필드 (`#[serde(default = "default_ocr_request_timeout_secs")]`, default `300`). 옛 config 가 필드 누락해도 그대로 파싱 + 동일 동작 (3 신규 unit test 가 default / env override / legacy parse 핀).
- env override `KEBAB_IMAGE_OCR_REQUEST_TIMEOUT_SECS`.
- `crates/kebab-parse-image/src/ocr.rs` 의 `REQUEST_TIMEOUT` 상수 제거. `OllamaVisionOcr::build` 시그니처가 `request_timeout_secs: u64` 추가, `new(&Config)` 는 `config.image.ocr.request_timeout_secs` 전달. `from_parts` (테스트 전용 surface) 도 동일하게 시그니처 확장 — caller 9 call site (`crates/kebab-parse-image/src/ocr.rs::tests` 5 test / 6 call site, `crates/kebab-parse-image/tests/ocr.rs::from_parts_clamps_max_pixels_into_legal_range` 1 test / 3 call site) 모두 `300` 명시 갱신.
- `OcrCfg::defaults()` 에 `request_timeout_secs: default_ocr_request_timeout_secs()` 추가. `Config::defaults()` 는 `ImageCfg::defaults()` 경유라 cascade.

**Edge case 동일**: `0` 은 disable 아닌 "즉시 timeout" (`Duration::from_secs(0)` 의 reqwest 의미). LlmCfg 의 doc comment 와 같은 안내가 OcrCfg field doc 에 명시.

**사용자 영향**: 기존 v0.17.x KB / config 는 변경 불필요 — 새 필드는 serde default 로 채워지고 동작도 동일 (300s). vision 모델 cold start 가 길면 `KEBAB_IMAGE_OCR_REQUEST_TIMEOUT_SECS=600` 또는 config 에서 `[image.ocr] request_timeout_secs = 600` 설정.

Cross-link: `crates/kebab-config/src/lib.rs::OcrCfg::request_timeout_secs`, `crates/kebab-parse-image/src/ocr.rs::OllamaVisionOcr::build`.

## 2026-05-25 — v0.17.2: `heading_path` FTS5 column filter (text-only matching, closure of 2026-05-24 `heading_path_json` 노이즈, PR #165)

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

**MCP / agent 가시성**: `search_response.v1` 의 wire shape 변경 없음. heading 검색 의도 사용자 / agent 를 위해 `integrations/claude-code/kebab/SKILL.md` 의 search 절에 column scoping + heading_path raw-mode escape hatch 안내 한 bullet 추가 (회차 1 follow-up 반영). 새 escape hatch (`'heading_path : <token>'`) 는 v0.17.0 의 raw mode (`'foo OR bar*'`) 와 같은 single-quote opt-out 패턴 위에 build — 새 surface 가 아닌 documented column-filter 활용.

Cross-link: `crates/kebab-search/src/lexical.rs::build_match_string`, `migrations/V007__fts_trigram.sql` (verbatim 유지), design §5.5 (verbatim 유지, query-time 동작만 변경).

## 2026-05-24 — v0.17.0: 한국어 trigram FTS5 tokenizer 채택 (closure of 2026-05-22 한국어 lexical)

V007 migration 으로 `chunks_fts` 의 tokenizer 를 `unicode61` → `trigram` 으로 교체. `chunks` 원본 + embedding + vector index 는 그대로, FTS shadow 만 재구축 + 자동 backfill — 사용자는 `kebab ingest` 재실행 불필요 (binary 만 교체하면 다음 open 시 V007 가 즉시 적용). 같은 라운드의 다른 두 follow-up (`code_lang_chunk_breakdown`, C typedef) 은 별 PR (PR-C / PR-B).

**한국어 lexical 동작**: 3자 이상 substring 매칭. `해시 충돌` 같은 2자 토큰 multi-token query 는 `crates/kebab-search/src/lexical.rs::build_match_string` 의 trigram-aware 재설계로 `("해시 충돌") OR ("해시" "충돌")` 형태가 되어 whole-phrase 후보로 hit (각 토큰 2자라 token-AND 후보는 trigram 에서 0-hit, 자동 drop). 한영 혼합 `Rust 충돌은` (둘 다 ≥3자) 도 OR-combined. 2자 이하 query (`충돌` / `키`) 는 정상 0 hit + CLI stderr `[hint] 3자 이상 키워드 권장 (trigram tokenizer 제약)` + `search_response.v1.hint` additive 필드 + TUI status bar 동일 안내. raw FTS5 single-quote mode (`'...'`) 는 사용자 명시 의도이므로 hint 안 나옴. 회귀 핀: `lexical_multi_token_korean_query_hits` + `lexical_mixed_korean_english_multi_token_query_hits` (`crates/kebab-app/tests/search_korean.rs`).

**영어 lexical 동작 변경**: substring 매칭으로 바뀜. `token` query 가 `tokenizer` 도 hit (recall ↑, 단어 경계 정밀도 ↓). 의도된 변경, 회귀 핀 = `fts_trigram_english_substring_hits` (`crates/kebab-store-sqlite/tests/fts.rs`).

**lexical BM25 score 분포**: 알고리즘 동일하지만 token stream 이 word → overlapping trigram 으로 바뀌어 raw score / TF / doc-length 모두 달라짐. `crates/kebab-search/tests/lexical.rs::lexical_snapshot_run_1` + `crates/kebab-search/tests/hybrid.rs::hybrid_snapshot_run_1` 둘 다 trigram baseline 으로 regenerate. hybrid (RRF) 는 rank 기반이라 ranking 영향 미미하나 `retrieval.lexical_score` 노출값은 변동.

**디스크 용량**: trigram 인덱스는 unicode61 대비 통상 2-10배. V007 자동 backfill 후 `kebab.sqlite` 파일 크기 증가 (도그푸딩 KB 기준 ~2-5배 또는 수백 MB). release notes 명시.

**`heading_path_json` JSON 노이즈 (관찰, 미수정)**: trigram 이 JSON 표기 (`[`, `"`, `,`) 와 그 안의 단어 (`app`, `src`) 까지 3-gram 색인 → query 가 우연히 JSON 구문 / 흔한 경로 단어와 겹쳐 false positive 가능. v0.17.0 에서는 컬럼 구성 유지, 도그푸딩 후 column filter (`{text} : <q>` 한정) 또는 평문 heading 변환 결정. 후속 도그푸딩 entry 로 등재 예정. → **closure**: 위 2026-05-25 v0.17.2 heading text column filter entry 참조 (column filter 방식 채택, V008 migration 불필요, PR #165).

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
