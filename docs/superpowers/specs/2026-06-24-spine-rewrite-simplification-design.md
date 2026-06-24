---
title: "Spine rewrite + simplification — kebab 척추 재작성 / 단순화 설계"
created: 2026-06-24
status: draft
supersedes_partially: docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: ["§1 workspace", "§2 RAG", "§6 parse/chunk", "§8 deps", "§9 versioning"]
---

# Spine rewrite + simplification

kebab 은 기능을 하나씩 더하며 자랐고, **consolidation pass 없이** 누적되어
표면·동작·내부 세 층위가 모두 산만해졌다. 이 설계는 단일 사용자·pre-1.0 라는
이점을 살려 **능력은 (거의) 유지하되 조작 표면과 코드 구조를 강하게 단순화**한다.

## 동기 (실측)

| 층위 | 증거 |
|---|---|
| 표면 | config **109 필드/19 섹션**, env **97개**(OCR만 41), `search` 한 명령에 **플래그 20개**, wire 스키마 **20개**, OCR 설정이 image(13)+pdf(17) **중복** |
| 내부 | `kebab-app` lib.rs **4193줄**(facade sprawl), `kebab-config` **17 crate** 가 통째 import, `kebab-chunk` 청커 **10+버전 동거**, `kebab-rag` pipeline.rs **2633줄 단일파일** |
| 스코프 | 24 crate / ~100K LOC. core 는 작고 주변부가 비대 — v0.5.0 이후 추가분이 정리 없이 쌓임 |

## 목표 / 비목표

**목표**
- 조작 표면 절반 이하: 노출 config 109→~30, env 97→~25, `search` 플래그 20→~6.
- 스파인 4 crate(app/config/chunk/rag)의 모놀리스 해체 → 작은·타입·독립테스트 stage.
- 안 쓰는 기능 제거(아래 Cuts).
- **출력 계약(wire) 안정** + **ingest 출력 불변**(재색인 불필요) 유지.

**비목표**
- 새 기능 추가. (이번은 순수 단순화)
- crate 토폴로지 병합. (사용자 결정: "통합 안 함, 스파인 내부만" — 삭제는 예외)
- 검색/RAG **결과**의 의도적 변경. (refactor 는 동작 보존; 표면·기본값만 정리)

## 결정 (brainstorm Q&A)

1. 범위 = **전면**(기능 cut 포함).
2. 실사용 = code/image/PDF ingest · MCP · multi-hop · eval · arctic · NLI · 멀티소스 **유지**.
   안 씀 = **TUI · multi-turn 세션** → 삭제.
3. 통증 = 표면·동작·내부 **셋 다 비슷** → 전면 정리.
4. 전략 = **C. 척추 재작성**(핵심 4 crate 근본 재구조화 후 기능 재장착).
5. 추가 삭제 = **legacy RAG 템플릿 v1/v2 · search LRU 캐시 · candle 임베더 provider** 전부.
6. crate 통합 = **안 함**(스파인 내부만; 삭제되는 crate 제외).
7. 표면 정리 = **공격적**(search→`--filter`, ask 정리, inspect+fetch→`get`, 출력 wire 안정).

## Scope: Cuts / Keeps / Restructures

### Cuts (삭제)
- **`kebab-tui` 전체 crate**(6.5K LOC) + `kebab tui` 서브커맨드.
- **multi-turn 세션**: `chat_sessions`/`chat_turns` 테이블, `ask --session`, `AskOpts` 의
  history/conversation 필드, 관련 wire. (list-sessions 도 없던 dead CRUD)
- **legacy RAG 템플릿 v1/v2**: `rag-v3`(기본)·`rag-v4`(provenance)만 유지.
- **search LRU 캐시**(p9-fb-19): `search_cache` 필드, `cache_capacity` config, 무효화 로직.
- **candle 임베더 provider**: `kebab-embed-candle` crate + provider enum 의 `candle` arm.
  (fastembed 기본 + ollama remote 로 충분; Mac Metal 미사용 확인)
- **ingest API 5변종** → 1개로.

→ crate 24 → **22**(tui, embed-candle 삭제).

### Keeps (유지 — 능력 보존)
code/image/PDF ingest, code AST 9언어, MCP 서버, multi-hop RAG, NLI 검증, eval 하네스,
arctic 임베더(via ollama; e5 기본은 fastembed), 멀티소스/provenance/trust 필터, 증분 ingest,
auto-reingest, 하이브리드 검색, single-hop RAG + citation, 모든 출력 wire 스키마.

### Restructures (스파인 재작성) — 아래 아키텍처

## 아키텍처

지도 원칙: ①Config = 타입 슬라이스 ②명시적 stage 파이프라인 ③축마다 디스패치 1곳
④얇은 facade(유스케이스당 진입점 1개) ⑤순수 RAG(`query→Answer`, 영속화는 호출자)
⑥출력 계약 안정.

### A. Config: god struct → 타입 슬라이스 (`kebab-config`)

- 다운스트림이 통째 `&Config` 가 아니라 자기 슬라이스만 받는다:
  `&IngestConfig` / `&SearchConfig` / `&RagConfig` / `&StorageConfig` / `&ModelsConfig`.
  최상위 `Config` 는 이들을 조립. 예: `RagPipeline::new(rag, models, …)`,
  `SqliteStore::open(&storage)`, `LanceVectorStore::new(&storage)`. → god-struct 결합(매듭 1) 소멸.
- 로딩 표면(`Config::load`/`from_file`/`validate`) 유지.
- **표면 정리**:
  - OCR 중복 제거: image·pdf 가 공유하는 paddle 손잡이(det/rec/dict/score_thresh/
    unclip/max_boxes)를 공유 `[ingest.ocr]` 엔진 블록으로 추출; image/pdf 는 고유분만.
    30+키 → ~12.
  - 거의 안 만지는 손잡이는 파싱은 유지하되 문서 표면에서 숨김(README/예시엔 핵심 ~30키).
  - env 자동 미러 중단: 런타임 override 가 실제 필요한 것(endpoint·path·thread)만 ~25.
  - 삭제 기능 키 제거(cache_capacity, 세션 관련).
- **config schema v4 → v5** + 무손실 자동 마이그레이션(OCR 통합·키 제거).

### B. Ingest 스파인: 4193줄 모놀리스 → stage 파이프라인 (`kebab-app` 내부 모듈)

- 내부 `ingest` 모듈의 선형 파이프라인: `scan(sources) → 자산마다 [fingerprint/skip]
  → extract → chunk → embed → store`.
- **축마다 디스패치 1곳**: `extractor_for(media)`, `chunker_for(media, &IngestConfig)` —
  흩어진 미디어 분기 + `*_chunker_from_config` 를 한 곳으로(매듭 2/3).
- **단일 fingerprint 타입**: `AssetFingerprint { parser_v, chunker_v, embedding_v,
  config_sig }` 가 `should_reprocess()` 결정을 소유. 각 미디어가 자기 `config_sig`
  기여분을 명시 선언(매듭 5, seam 4/6 해소).
- **ingest API 5변종 → 1**: `ingest(cfg, scope, IngestOpts{ progress, cancel,
  force_reingest, summary_only })`.
- `App` 은 리소스/수명 홀더(sqlite/embedder/vector/llm/extractors)로 남고 per-run
  오케스트레이션만 모듈로 이동.

### C. Chunk: trait + impls + selector 1개 (`kebab-chunk`)

- 중앙 `chunker_for(media, &IngestConfig) -> Box<dyn Chunker>` 추가.
- legacy `md-heading-v1` 제거(v2 유지), pdf/code/manifest 청커 유지.
- 공유 primitive(`oversize`, korean morphological tokenizer) 유지.
- **불변식**: 청커 출력은 byte-identical 보존 → `chunker_version` 불변 → 재청크 없음.

### D. RAG 스파인: 2633줄 9단계 모놀리스 → 합성 stage (`kebab-rag` 내부 모듈)

- pipeline.rs 를 stage 모듈로 분해: `retrieve` · `gate`(score/no-chunks) · `pack`
  (컨텍스트 예산) · `prompt`(템플릿) · `generate`(LLM 스트림) · `cite`(추출+검증) ·
  `verify`(NLI). `ask` 는 이들의 얇은 합성. single-pass·multi-hop 이 같은 stage 공유
  (multi-hop 은 decompose/decide/synthesize 추가).
- **순수화**: `docs: SqliteStore` write 의존 제거 → `query → Answer` 반환, 영속화는
  호출자(kebab-app)(매듭 4, seam 8).
- **NLI**: verifier 항상 주입, 내부에서 threshold 판단(2곳 → 1곳, seam 7).
- **retriever**: `RagPipeline` 이 `mode` + `RetrieverFactory` 를 받아 선택 내부화(seam 5).
- 템플릿은 v3/v4 만 → `prompt` stage 단순.
- **불변식**: `prompt_template_version` 기본 rag-v4 유지 → 기존 답변/동작 보존.

### E. 표면 정리 (CLI · wire · env)

- **`search` 20플래그 → ~6**: 11 필터축(tag/lang/path-glob/trust-min/media/
  ingested-after/doc-id/repo/code-lang/source-type/source)을 단일 `--filter k:v,…` 식으로.
  남기는 것: `<query>`, `-k`, `--mode`, `--filter`, `--json`, `--explain`, `--cursor`.
  (max-tokens·snippet-chars → config 기본값 흡수, trace → `--explain`, bulk → 별도 입력 모드)
- **`ask` 10 → ~7**: session 제거, show/hide-citations → `--citations` 하나로.
- **서브커맨드**: `tui` 제거. `inspect{doc,chunk}` + `fetch{chunk,doc,span}` → 단일 `get`
  (레코드 vs 원문은 플래그로). 나머지 유지.
- **wire**: 출력 계약(`search_hit.v1`·`answer.v1` 등) **불변**(MCP/스킬 무영향).
  삭제 기능분 스키마만 정리.

## Target crate topology

24 → 22. 삭제: `kebab-tui`, `kebab-embed-candle`. 병합 없음. 스파인 4 crate
(app/config/chunk/rag)는 **내부 모듈 구조만** 재작성(공개 facade 는 얇아지되 crate 경계 유지).
parse-*, store-*, embed-{trait,fastembed,ollama}, llm-{trait,local}, nli, source-fs,
eval, mcp, cli, core 는 그대로.

## Surface targets (before → after)

| 지표 | before | after(목표) |
|---|---|---|
| crate 수 | 24 | 22 |
| `kebab-app` lib.rs 최대 파일 | 4193줄 | 단일 파일 ≤ ~800, 모듈 분리 |
| `kebab-rag` pipeline.rs | 2633줄 | stage 모듈, 단일 파일 ≤ ~600 |
| 노출 config 키 | 109 | ~30 |
| KEBAB_* env | 97 | ~25 |
| `search` 플래그 | 20 | ~6 |
| ingest API 변종 | 5 | 1 |
| RAG 템플릿 버전 | 4(v1~v4) | 2(v3/v4) |
| config 소비 결합 | 통째 `&Config` (17 crate) | 타입 슬라이스 |

## 불변식 (회귀 0 보증)

- **ingest 출력 byte-identical**: parser/chunker/embedding_version 불변 → 재색인·재임베딩 없음.
- **검색/RAG 결과 동등**: 동작 보존 refactor; `--filter` 는 입력 표면만, 출력 wire 불변.
- **wire 출력 계약 불변**: `search_hit.v1`·`answer.v1` shape 유지(MCP·Claude 스킬 무영향).
- config v4→v5 는 무손실 자동 마이그레이션.

## 코어 도그푸딩 품질 패리티 게이트 (per 수정 단위) — HARD GATE

**사용자 필수 제약**: 각 수정 단위(PR/phase)마다 **코어 기능의 도그푸딩 품질이 직전
baseline 과 반드시 비슷**해야 한다. 회귀 시 머지 금지. 이 게이트가 척추 재작성의
PR 분할 경계와 완료 정의를 지배한다.

- **코어 기능 정의**: markdown ingest · lexical/vector/hybrid 검색 · single-hop RAG +
  citation (= 사용자가 매일 받는 결과물). 보조로 multi-hop·OCR·code ingest spot-check.
- **baseline 동결**: 재작성 착수 *전*, 현재 main 바이너리로 표준 도그푸딩 코퍼스에
  golden query 실행 → 메트릭 스냅샷 동결(search hit ordering · MRR/hit@k · RAG
  citation 패턴 · grounded 율). `kebab eval run` 산출물 + 검색/ask `--json` 캡처.
- **per-unit 게이트**(매 단위 머지 전, 예외 없음):
  1. 동일 코퍼스·동일 query 재실행 → `kebab eval compare` 로 baseline 대비.
     허용 오차: hit ordering·citation 패턴 **동등**(MRR/hit@k 변동 ≤ 작은 ε, 순위 역전
     0 목표). 초과 시 원인 규명 → fix → 재게이트, 통과 전 머지 금지.
  2. **ingest 출력 byte-diff = 0**: 동일 코퍼스 재색인 산출(chunk text·chunk_id·
     embedding_version)이 baseline 과 byte-identical(refactor 동작 보존 증명).
  3. 자동화 불가한 UX/snippet 품질은 수동 도그푸딩 spot-check + HOTFIXES evidence.
- **도구**: eval 하네스(`kebab eval run|compare`)가 1차. 재작성으로 eval 자체가 바뀌는
  단위는 baseline 바이너리를 비교 기준으로 병행 보관.

## Relationship to frozen contract

`docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` 의 일부 조항(§1 단일 root/
멀티소스, §2 RAG 템플릿/세션, TUI 관련)을 **부분 supersede** 한다. 구현 PR 에서 frozen
contract 의 해당 섹션 + 참조 task spec 을 같은 PR 에 갱신(CLAUDE.md §Spec contract 규칙).
TUI·세션 제거는 frozen contract 에서 해당 컴포넌트를 "removed" 로 표시.

## 성공 기준

- Surface targets 표의 after 수치 달성.
- `cargo test --workspace` green, clippy 0, 기존 도그푸딩 시나리오 결과 동등(회귀 0).
- 동일 KB 에서 재색인 없이 새 바이너리 동작(ingest 출력 불변 증명).
- 단일 사용자 도그푸딩: 동일 query 의 검색 hit·RAG citation 패턴 불변.

## 위험 / 완화

- **대규모 refactor 회귀**: 척추 4 crate 동시 변경 → 단계별 PR + 각 단계 도그푸딩,
  ingest 출력 byte-diff 게이트로 불변식 강제.
- **config v4→v5 마이그레이션 버그**: v3→v4 와 동형의 무손실 자동 마이그레이션 + round-trip 테스트.
- **`--filter` 파서 회귀**: 기존 11 플래그 → filter 식 매핑 1:1 테스트, 잘못된 키 명확 에러.
- **frozen contract drift**: 구현 PR 에서 contract+task spec 동시 갱신 누락 위험 → CI diff-check.

## Open questions (구현 계획에서 확정)

- `--filter` 식 문법 정확한 형태(`k:v,k:v` vs 반복 `--filter k=v`).
- `get` 통합 서브커맨드의 정확한 플래그(`--verbatim`/`--record`).
- 스파인 재작성 PR 분할 경계(config 먼저 → ingest → rag → 표면, 각 독립 도그푸딩).
