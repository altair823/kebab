---
title: 색인시 doc-side expansion (검색용 별칭 생성) — 설계 spec
date: 2026-05-30
status: 설계 확정 (brainstorm 완료) — plan 대기
phase: Phase 2 (query-paraphrase robustness 처방)
related:
  - docs/superpowers/handoffs/2026-05-30-phase2-doc-expansion-kickoff.md
  - docs/superpowers/research/2026-05-30-vocabulary-gap-recall-fix-research.md
  - docs/superpowers/specs/2026-05-29-query-paraphrase-robustness-eval-design.md
  - memory: project_paraphrase_robustness, project_crossscript_diagnosis, feedback_search_quality_dogfood
contract_sections:
  - "design §6 (retrieval / hybrid fusion)"
  - "design §9 (versioning cascade)"
---

# 색인시 doc-side expansion — 설계 spec

> **⚠️ 제거됨 (2026-06-03).** 본 spec 이 도입한 doc-side expansion(별칭) 기능은
> 2026-06-03 완전히 제거되었다. 근거: 별칭 ROI 음수(cross-lingual 은 e5-large
> 단독으로 충분, 기여는 설명형 +2 그룹뿐인데 대가가 살아있는 KB 에 지속 불가한
> 청크당 색인-시 LLM). 제거 spec: `docs/superpowers/specs/2026-06-03-remove-doc-expansion-spec.md`,
> HOTFIXES dated entry 2026-06-03. 이 문서는 역사적 contract 로 freeze 유지.

## 0. 한 줄 요약

문서를 색인할 때(ingest) 각 청크마다 로컬 LLM(gemma)에게 "이 내용을 찾을 사람이 던질 법한 다른
표현·질문"(같은언어 paraphrase + 한↔영 번역 별칭)을 **1회** 생성하게 해, **별도 FTS5 채널**에
저장한다. 검색 시 RRF 가 `{body-BM25, aliases-BM25, e5-dense}` 3채널을 융합한다. 어휘격차(B)로
정답이 top-50 pool 에도 안 들어오던 실패(`recall@50=0`)를 lexical pool 자체를 키워 해결하는 게 목표.
**flag off 기본**, on/off 를 `kebab eval variants` 로 정량 비교한다.

## 1. 배경 / 문제 (압축)

- Phase 1 진단: 같은 의미를 다른 단어로 물으면 정답이 top-50 pool 에도 안 들어옴(`recall@50=0`).
  rerank 는 pool 안 순서만 바꿔 무력(`[[project_rerank_experiment]]` 가설 반증).
- 딥리서치(104 agent, 적대검증): pool-miss 의 최선책 = **색인시 doc-side expansion**. query-side
  (HyDE=거부된 per-query LLM, Vector-PRF=recall 주장 기각) 부적합. learned-sparse(SPLADE/MILCO)
  CPU/Rust turnkey 경로 없음.
- 핵심 함정: vanilla mt5 doc2query 는 *같은 언어* query 만 생성 → 한/영 갭 못 메움. 따라서 색인시
  **KO↔EN 번역 별칭**을 함께 생성해야 함 (research §1.2). 이 교차언어 부분은 직접 벤치 논문 없는
  **합성 권고** → 우리 corpus 측정 필수.

## 2. 설계 결정 (brainstorm 확정)

| # | 결정 | 선택 | 근거 |
|---|------|------|------|
| D1 | 별칭 생성 단위 | **청크당 1회** | 각 조각의 세부 내용에 맞는 정밀 별칭. ingest 느려지나 효과 측정이 1순위(§4.6 측정 규율). |
| D2 | 별칭 내용 | **같은언어 paraphrase + 한↔영 번역**, 1 LLM 호출 | 진단상 영어 paraphrase 도 miss(어휘 거리), 한/영 갭은 번역 별칭으로만 메움. 한 호출로 둘 다 → 추가 호출비용 0. |
| D3 | 기존 문서 처리 | **additive + 수동 재색인** | 별칭은 "있으면 쓰고 없으면 본문만". flag on 이 전체 자동 재색인을 트리거하지 않음. `--force` 로 원할 때 재생성. 측정은 dogfood reset→reingest 로 통제. |
| D4 | 품질 제어 (1차) | **단순**: 개수 상한 + 형식 검증만 | 정교한 환각 필터(임베딩 유사도, Doc2Query--)는 research openQuestion 3 = 측정 대상. 1차는 단순히 만들고 환각·팽창이 실제 문제인지 측정 후 결정. |

## 3. 아키텍처

### 3.1 데이터 흐름

```
ingest_one_asset (kebab-app/src/lib.rs:~1253)
  chunks = MdHeadingV1Chunker.chunk(&canonical, policy)?
    │
    ├─ [NEW] if config.ingest.expansion.enabled:
    │     for chunk in &mut chunks:
    │       aliases = ExpansionGenerator.generate(chunk)?         # gemma 1회/청크
    │       chunk.aliases = Some(aliases)                         # 상한·형식검증 적용
    │
  app.sqlite.put_chunks(doc_id, &chunks)?    # chunks.aliases 컬럼 저장
    │
  (V010 chunk_aliases_au/ai trigger) → chunk_aliases_fts 별도 테이블 색인
    │
  embedder.embed(...) → vec_store.upsert(...)  # dense는 body text 기준 (변경 없음)

검색 (kebab-search/src/lexical.rs — body·alias 두 쿼리 + Rust merge):
  body  = run_query(chunks_fts MATCH 'text : (..)')          (bm25 asc)  ┐ merge_body_alias:
  alias = run_alias_query(chunk_aliases_fts MATCH 'aliases : (..)')      ┘ body 우선 + alias-only append
    │                                                          → 단일 lexical 결과 (rank 부여)
  HybridRetriever.fuse: RRF(lexical, vector)   # 2채널 그대로 — RetrievalDetail/wire 무변경
```

**왜 lexical 내부 병합인가 (3채널 RRF 대신):** `RetrievalDetail` 은 `lexical_score`/
`vector_score`/`*_rank` 만 보유하고 wire schema `search_hit.v1` 가 이를 그대로 노출한다. 정통
3채널 RRF 는 `RetrievalDetail` + wire schema + `HybridRetriever` 시그니처 + 다수 테스트를 침습
변경한다. alias-only 청크가 lexical 결과(→ hybrid pool)에 진입하기만 하면 pool-rescue 목적은
동일하게 달성되므로, **`LexicalRetriever` 내부에서 body+alias 를 병합**해 단일 lexical 결과로
내보낸다. `chunk_aliases_fts` 가 비면(flag off / 미생성) alias 쿼리가 0행 → merge no-op → 기존
동작과 동일 → **search-side 는 flag 게이트 불필요, ingest-side 만 게이트**.

> **구현 메커니즘 (shipped):** 단일 `UNION ALL + GROUP BY` SQL 이 아니라 **두 쿼리(`run_query` +
> `run_alias_query`) + Rust `merge_body_alias`(body 우선, body 에 없는 alias-only 만 append,
> `fetch_limit` 절단)**. 서로 다른 FTS 테이블의 bm25 절대값을 `GROUP BY MIN` 으로 비교하는 것은
> 무의미하므로 body-우선 Rust 병합이 의미상 더 깨끗하다(§3.3 body 보존과도 일치). raw 모드
> (작은따옴표 식)는 body-only 컬럼 참조 가능성 때문에 alias 채널에서 제외한다(방어 가드).

### 3.2 컴포넌트 (단위별 책임)

- **`ExpansionGenerator`** (kebab-app, `kebab_llm::LanguageModel` trait 경계로 mock 가능)
  - 입력: 청크(본문 + heading_path 컨텍스트), config(model, max_aliases, prompt_version).
  - 출력: 검증된 별칭 문자열(개행 join). 빈 출력·과길이 drop, 개수 상한 적용.
  - 의존: `LanguageModel::generate_stream`(스트림을 모아 문자열). 기존 `OllamaLanguageModel`
    재사용. LLM 호출 실패/빈 결과 시 해당 청크는 별칭 없이 진행(ingest 비중단 — **fail-soft**).
- **V010 migration** — `chunks.aliases TEXT` 컬럼 + **별도 `chunk_aliases_fts` virtual table**
  + 별도 trigger 3종(`chunk_aliases_ai/ad/au`). 기존 `chunks_fts` / `chunks_ai/ad/au`(§5.5
    verbatim CI 대상)는 **무수정**. tokenizer `unicode61`(V009 동일).
- **`LexicalRetriever` body+alias 병합** (kebab-search/lexical.rs) — 기존 `run_query`(body) +
  신규 `run_alias_query`(`chunk_aliases_fts` MATCH, `chunks`/`documents` JOIN, snippet 은 본문
  `substr(c.text,1,?)`) 를 각각 실행하고 `merge_body_alias`(body 우선, body 에 없는 alias-only 만
  append, `fetch_limit` 절단)로 합친다. `build_match_string` 은 컬럼 파라미터화(`text :` / `aliases :`).
  alias-only 청크가 결과에 진입. `chunk_aliases_fts` 가 비면 alias 쿼리 0행 → 기존과 동일(회귀 안전).
- **config `[ingest.expansion]`** — `IngestExpansionCfg`:
  - `enabled: bool` (default **false**)
  - `model: String` (default = `models.llm.model`)
  - `max_aliases_per_chunk: usize` (default 8)
  - `prompt_version: String` (default `expansion-v1`)
  - env override: `KEBAB_INGEST_EXPANSION_ENABLED`, `KEBAB_INGEST_EXPANSION_MODEL`,
    `KEBAB_INGEST_EXPANSION_MAX_ALIASES`, `KEBAB_INGEST_EXPANSION_PROMPT_VERSION`.

### 3.3 격리 / 코드 식별자 보존 (load-bearing)

- `chunks_fts.text`(body) 는 **verbatim 유지**, 별칭은 **별도 테이블** `chunk_aliases_fts`.
  body-우선 merge 라 body 매칭이 항상 alias-only 보다 앞서 보존되어, 코드 식별자(`Vec::with_capacity`)
  정확매칭이 별칭 노이즈에 희석되지 않음.
- dense(e5) 임베딩은 **body text 기준 그대로** — 별칭을 임베딩에 넣지 않음(research: e5 dense
  유지, bge-m3 dense 는 실측 더 나빴음). 별칭은 lexical 채널에만 기여.

## 4. 스키마 / migration (V010)

현재 최신 = V009. 신규 = **V010__chunk_aliases.sql**. 기존 `chunks_fts` / `chunks_ai/ad/au`
(§5.5 verbatim CI `fts_v009_matches_design_section_5_5_verbatim` 대상)는 **건드리지 않는다.**

```sql
-- 1) chunks 테이블에 별칭 컬럼 (nullable; 미생성/flag off = NULL)
ALTER TABLE chunks ADD COLUMN aliases TEXT;

-- 2) 별도 FTS5 가상 테이블 (body 와 분리된 lexical 채널)
CREATE VIRTUAL TABLE chunk_aliases_fts USING fts5(
  chunk_id  UNINDEXED,
  doc_id    UNINDEXED,
  aliases,
  tokenize = 'unicode61'                   -- V009 본문과 동일 tokenizer
);

-- 3) 별도 sync trigger 3종 (aliases NULL 이면 색인 안 함)
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

-- 4) backfill 불필요: 기존 행 aliases 전부 NULL → chunk_aliases_fts 빈 채로 시작.

-- 5) corpus_revision bump (in-process search cache 무효화; V009 와 동일 패턴)
UPDATE kv SET value = CAST(CAST(value AS INTEGER) + 1 AS TEXT) WHERE key = 'corpus_revision';
```

- `put_chunks` 의 DELETE-then-INSERT(documents.rs:101) 는 `chunk_aliases_ad`(DELETE) +
  `chunk_aliases_ai`(INSERT) 를 발화 → 별칭 동기화 자동. INSERT 문에 `aliases` 컬럼만 추가.
- migration 은 refinery 자동 embed/apply. **migration = breaking schema change** → CLAUDE.md
  §Release / Dogfood trigger 발동(V010, dogfood + release notes).
- `kebab_core::Chunk` 에 `aliases: Option<String>` 필드 추가(`#[serde(default)]`).

## 5. gemma 프롬프트 (expansion-v1)

청크 본문 + heading_path 를 주고, **검색 별칭만** 줄 단위로 출력하게 한다(설명·번호 금지).
같은언어 표현 + 반대언어(한↔영) 번역을 섞어 최대 `max_aliases_per_chunk` 개.

요지(plan 단계에서 정확한 문구·few-shot 확정):
- "다음 문단을 검색할 사용자가 쓸 법한 짧은 질의/표현을 생성하라. 동의어·풀어쓴 표현 포함.
  문단이 한국어면 영어 표현도, 영어면 한국어 표현도 섞어라. 한 줄에 하나, 설명 없이."
- 출력 파싱: 줄 단위 split → trim → 빈 줄/번호접두/과길이(예: >120자) drop → 상한 N개.
- 결정성: `temperature` 낮게, `seed` 고정(config 의 llm seed 재사용) → 재색인 재현성.

## 6. versioning cascade (design §9)

- 별칭은 **additive** → `try_skip_unchanged`(kebab-app:~886) 의 기존 5버전(parser/chunker/
  embedding…) 판단에 **넣지 않는다**. 즉 flag 토글이 전체 문서를 stale 로 만들지 않음(D3).
- `expansion_version`(= `prompt_version`)을 documents 레코드에 기록(추적용). 프롬프트가 바뀌면
  추후 재생성 대상 식별 가능. 단 자동 cascade 는 걸지 않음(수동 `--force`).
- 측정/실사용에서 별칭을 새로 입히려면: `kebab ingest --force`(전체 재처리) 또는 dogfood
  `kebab reset` + reingest.

## 7. 측정 (§4.6 측정 규율 — 프록시 금지, 추측 금지)

```
# baseline (flag off, 또는 Phase 1 기록): groups=8 fully_consistent=2 A=2 B=4 spread@10=0.750
KEBAB_EVAL_GOLDEN=/build/dogfood/golden_queries.yaml \
  kebab eval run --config /build/dogfood/config.toml --mode hybrid --k 50
KEBAB_EVAL_GOLDEN=/build/dogfood/golden_queries.yaml \
  kebab eval variants <run_id> --config /build/dogfood/config.toml

# 처방 on: expansion enabled 로 reset+reingest 후 동일 측정
```

- 성공 기준: **B_dominant↓, fully_consistent↑, spread@10↓** (on vs off). 전체 golden 회귀 확인
  (기존 Ok 그룹이 깨지지 않는지).
- 측정값은 grep clean 추출 → Read 확인값만 기록(추측 금지). HOTFIXES + release notes-draft 에 cascade.

## 8. 범위 밖 (YAGNI)

- **BGE-M3 sparse 4th RRF 채널** — research §1.4: 교차언어 약함(우리 핵심은 KO↔EN 갭). 측정 후
  단일언어 lift 가 필요하다 판단되면 별도 작업.
- **임베딩 유사도 환각 필터 / Doc2Query--/++** — D4. 측정에서 환각·팽창이 실제 문제일 때.
- **문서/혼합 단위 생성** — D1 에서 청크당으로 확정.
- **별칭의 dense 임베딩** — body 기준 유지(§3.3).

## 9. 테스트 전략 (TDD — plan 에서 task 분해)

- migration: V010 적용 후 `chunks.aliases` + `chunks_fts.aliases` 존재, 기존 행 본문 색인 동일.
- `put_chunks`/`get` round-trip: `aliases=Some(..)` 저장·조회.
- FTS5 alias 검색: `chunk_aliases_fts` 의 term 으로 MATCH 시 해당 chunk 회수.
- lexical UNION: body 에 없고 alias 에만 있는 term 으로 검색 시 alias-only 청크가 `LexicalRetriever`
  결과(→ hybrid pool)에 진입(pool-rescue 핵심 회귀). 양쪽 매칭 청크는 중복 없이 1개.
- `ExpansionGenerator`(LLM mock): 프롬프트→파싱, 상한 N 적용, 빈/과길이 drop, LLM 실패 시 fail-soft.
- 회귀: `chunk_aliases_fts` 빈 상태에서 lexical 결과가 V009 와 동일(alias 쿼리 0행 → merge no-op).

## 10. PR / 문서 동기화

- gitea-pr 리뷰 루프(`[[feedback_pr_workflow]]`). flag off 기본.
- user-facing surface(신규 config `[ingest.expansion]`, `KEBAB_INGEST_EXPANSION_*` env, V010
  migration) → 같은 PR 에서 README(좁게: flag 존재+포인터) + HANDOFF + ARCHITECTURE 동기화
  (`[[feedback_readme_sync_rule]]`). flag 망라는 `--help`/config 예제에 위임.
- V010 = breaking schema → dogfood evidence(HOTFIXES dated entry) + release notes-draft 4단락.
