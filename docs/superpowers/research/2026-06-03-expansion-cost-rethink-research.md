# Expansion 비용 재고 — 별칭(doc-side LLM expansion)을 대체할 방법 조사

**날짜**: 2026-06-03
**상태**: 조사 완료, 검증(측정) 대기
**선행**: [[2026-05-30-vocabulary-gap-recall-fix-research]] (당시 결론 = doc-side expansion), v0.21.0 별칭 구현(#195/#196)
**계기**: 도그푸딩에서 expansion 이 ingest 임계경로의 압도적 병목으로 확정(청크당 gemma4:e4b ~1.3s, 5150 청크 ≈ 1.9h). 동시성(A)·모델스왑(D) 실측 소진. 사용자가 두 구조적 반론 제기.

---

## 1. 문제 재정의 — 왜 "반창고"가 다 실패하는가

별칭은 **청크마다 LLM 1회 호출**(`kebab-app/src/lib.rs` expansion 루프). 비용이 **코퍼스 크기에 비례**하고, KB 가 살아있으므로(문서 수정·추가) **갱신 청크를 영원히 따라가야 함**.

소진된 레버 (전부 *같은 총량을 언제/어떻게 나눌지*만 바꿈, 총량 불변):

| 레버 | 실측(2026-06-02~03, Mac M4 Pro Metal) | 판정 |
|------|------|------|
| A. `OLLAMA_NUM_PARALLEL` + 클라 동시요청 | 슬롯 2/4, 동시 2/4/8 → **최대 1.28×** (GPU compute 포화) | ✖ 불충분 |
| D. 모델 스왑 | gemma4:e4b 1.22s/건(품질 합격선) · qwen2.5vl:3b 더 느림+무한반복 · **qwen3.5:2b-mlx 0.24s(~5배)지만 중국어(所有权系统)+47줄 degeneration** · 0.8b 입력에코 | ✖ gemma 품질 못 이김 |
| B. 백그라운드/별도명령 | 총량·팬·리소스 불변, 유지보수 treadmill 잔존 | ✖ 사용자 반론으로 기각 |

**사용자 반론(정확)**: ① 별도 명령이어도 맥 팬·리소스 총량 동일 ② 청크당 이렇게 비싸면 갱신을 못 따라감. → "청크마다 미리 LLM" 구조 자체가 부적합. **아키텍처를 의심해야 하는 지점.**

---

## 2. 학계/웹 조사 핵심 발견

### 2.1 결정타 — Expansion 은 강한 검색기에 오히려 해롭다
*"When do Generative Query and Document Expansions Fail?"* (arXiv 2309.08541): **검색기 성능과 expansion 이득 사이 강한 음의 상관**. 11개 expansion 기법 × 12개 데이터셋 × 24개 검색 모델에서 일관. 약한 모델엔 도움, **강한 모델엔 손해**(추가 noise 가 relevance 신호를 흐림, false positive 유발). 권고: *"target 이 학습 코퍼스와 크게 다르거나 약한 모델일 때만 expansion, 아니면 피하라."*

→ 함의: 별칭의 v0.21.0 이득이 **14/18→16/18(미미)** 였던 건 우연이 아님. e5-large 는 이미 준수한 다국어 검색기 → 별칭은 *목발*에 가깝고 ROI 가 0~음수 구간일 수 있음. **측정으로 즉시 확인 가능**(별칭 on/off 골든 비교).

### 2.2 어휘·교차언어 격차는 본질적으로 *임베딩* 문제
별칭은 "역전파↔backpropagation 이 벡터공간에서 안 가깝다"를 색인-시 텍스트로 우회한 것. 정공법 = **교차언어가 강한 임베더로 벡터공간 자체에서 정렬**. 비용 = LLM 0, **색인 1회 재계산**(살아있는 KB 에서도 신규/수정 청크 임베딩은 어차피 하는 일 — treadmill 없음).

### 2.3 임베더 후보 (로컬·오픈, 2026)
- **BGE-M3** (사용자 Mac 에 이미 pull 됨): XLM-RoBERTa-large 기반(= `kebab-embed-candle` 의 `XLMRobertaModel` 과 **동일 아키텍처**), dense **1024-dim**(= e5-large, 벡터스토어 그대로), **prefix 불필요**(e5 의 `query:`/`passage:` 와 달리), 단 **CLS pooling**(e5 는 mean pooling — 통합 시 분기 필요). **dense+sparse(lexical)+multi-vector(ColBERT)** 3-헤드.
  - 한↔영 실측(Belebele, 2507.08480): base bge-m3 ≈ base e5-large (EN→KO 는 e5 92.0 > m3 90.4, KO→EN 은 m3 88.4 > e5 86.5). **dense 단독 교체만으론 대박 아님**. 차별점은 sparse/multi-vector 헤드.
- **Qwen3-Embedding** (2026 초, MTEB v2 오픈웨이트 1위; 8B 는 무거움, 0.6B/4B 변형 존재): 다국어 최상위. 소형 변형이 로컬 가용하면 dense 업그레이드 후보.
- 다국어 일반 권고(2026 가이드들): "BGE-M3 또는 Nomic". e5-large 도 여전히 경쟁력.

### 2.4 Multi-vector(ColBERT)는 색인-전체가 아니라 *질의-시 rerank* 로
ColBERT/multi-vector 는 토큰당 벡터 1개 → 저장 폭증(10M doc ≈ 6TB vs bi-encoder 30GB). **전체 코퍼스 색인 금지.** 실용 패턴 = dense 1차 검색 → **top-50/100 만 multi-vector late-interaction rerank(질의-시, O(질의))**. 진단된 "near-tie 벡터 불안정"([[project_crossscript_diagnosis]])을 정조준하면서 색인 비용 0.

### 2.5 굳이 expansion 한다면 — query-side, single-pass
CTQE(2509.02377): LLM 한 번의 decoding 패스에서 candidate token 재활용 → **추가 inference 없이** query expansion. 비용 O(질의), 캐시 가능. doc-side 의 O(코퍼스)·treadmill 과 정반대.

---

## 3. 권고 아키텍처 — 청크당 LLM 0, 측정-우선 단계별

원칙: 비용을 **O(코퍼스 LLM)** 에서 **O(코퍼스 임베딩, 이미 수용중) + O(질의)** 로 이동. 각 단계는 기존 골든/variant eval 로 검증 후 다음 진행(사용자 "측정 먼저" 방법론).

- **Step 0 — 별칭 ROI 실측 (LLM 0, 코드 0)**: 현재 e5-large 에서 별칭 **on vs off** 골든/variant 비교. 2.1 예측대로 차이 미미/음수면 → **별칭 기능 통째 제거**(즉시 최대 승리: 청크당 LLM 영구 소멸). 차이가 유의미할 때만 Step 1+.
- **Step 1 — 강한 dense 임베더 (LLM 0, 색인 1회)**: BGE-M3 를 `kebab-embed-candle` 로 dense 임베더 교체 검증(같은 XLM-R, CLS pooling + prefix 제거, 1024-dim 동일). 소형 Qwen3-Embedding 가용 시 병행. `embedding_version` cascade = 전체 1회 재임베딩(0.48s/asset 관측, 수용 범위, treadmill 아님).
- **Step 2 — BGE-M3 sparse 헤드를 lexical arm 으로 (LLM 0)**: 학습된 sparse lexical 이 FTS5 보다 교차언어 우수. 같은 임베드 패스 산출물 → 추가 색인 비용 ≈ 0. RRF 의 lexical 항 보강/대체.
- **Step 3 — (선택) 질의-시 multi-vector rerank**: 잔존 near-tie 순위 출렁이면 top-50 만 BGE-M3 multi-vector late-interaction rerank(O(질의), 색인 bloat 0).

**통합 이점**: 사용자가 이미 NUMA 대응으로 만든 `kebab-embed-candle`(XLM-RoBERTa)가 BGE-M3 와 동일 아키텍처 → 가중치/풀링/헤드 추가 위주로 재사용. fastembed 도 bge-m3 지원(단 NUMA double-free 회피 위해 candle 경로 선호).

**리스크/주의**: ① dense 단독 교체 이득은 한↔영 데이터상 작을 수 있음 → sparse/multi-vector 가 실질 차별점, Step 1 단독 성패로 판단 말 것. ② CLS vs mean pooling, prefix 차이 → 정확히 구현 안 하면 품질 급락(검증 필수). ③ `embedding_version` bump = breaking, 재임베딩 필요(versioning cascade). ④ Mono-IR 소폭 저하 가능(2507.08480) — 골든의 한국어-단일 케이스도 같이 측정.

---

## 4. Step 0 측정 결과 (2026-06-03, v0.24.0 fresh)

namu corpus(997 docs / 23151 chunks, e5-large) + `namu_golden_step0.yaml`(doc_id 재매핑, 18그룹×4변형+10대조) hybrid k=50.

| arm | fully_consistent | recall@10 | recall@50 | mean_spread@10 | 색인 LLM 비용 |
|------|------|------|------|------|------|
| **OFF (별칭 없음, fresh v0.24.0)** | **14/18** | **68/72 (0.944)** | 70/72 (0.972) | **0.222** | **0** |
| ON (별칭, v0.21.0 prior, 동일 corpus/golden) | 16/18 | ~70/72 | ~72/72 | 0.111 | 별칭 LLM (정답 18문서만 **2.5h**, 전 corpus 수 시간) |

fresh OFF 가 이전 baseline(14/18, A2/B2, spread 0.222)을 **정확히 재현** → 이전 ON(16/18, A1/B1, spread 0.111, handoff 2026-05-31)과 직접 비교 유효. ON 재측정은 시드 캐시의 alias 행이 7개뿐이라 전 corpus cold 별칭생성=수 시간(= 사용자가 못 견디는 그 비용) → 비실시.

**변형 종류별 OFF recall@10 (별칭 0):**
`en 18/18 · ko 18/18 · syn 11/11 · abbr 7/7 · para 14/18` — **교차언어(en↔ko)·동의어·약어는 별칭 없이 이미 완벽.** 유일한 약점 = 설명형(paraphrase) 4쿼리.

**결론 (Step 0)**:
- 별칭이 정조준한 **cross-lingual 격차는 e5-large 단독으로 이미 top-10 완벽 해결**(역전파↔backprop 우려는 기우였음). 별칭의 실제 기여 = **paraphrase 그룹 +2(14→16)** 뿐, 그것도 stack/svm 설명형 잔존.
- 그 +2 를 위해 **색인-시 수 시간 LLM + 살아있는 KB treadmill**(사용자 2대 반론) 을 지불 = ROI 음수 구간. §2.1 "강한 검색기엔 expansion 이 해롭다" 와 정합.
- **권고**: 별칭 default-off 유지하다 **제거 후보**로 격하. 단 paraphrase 잔존(4쿼리)을 §3 Step 1(BGE-M3 dense, LLM 0/색인 1회)이 닫는지 먼저 측정 → 닫으면 별칭 완전 삭제, 못 닫으면 query-side single-pass(§2.5) 소폭 보강. **어느 쪽도 청크당 LLM 0.**

산출물: `/build/dogfood/_archive/step0/`(config-off/on, kb-off, namu_golden_step0.yaml, fill_docids_step0.py), OFF run `run_019e89c524ca76a1befae126f0c77336`, `/tmp/step0_off_variants.json`.

## 5. Step 1 측정 결과 (2026-06-03) — bge-m3 dense = lateral, 업그레이드 아님

kebab(fastembed 4.9.1)은 bge-m3 dense 미지원(reranker V2M3 / BGE EN·ZH v1.5 / e5 만). candle 은 e5 전용(mean pool+prefix). → **standalone 측정**: kb-off 청크 23151개 + 변형 72쿼리를 Ollama `bge-m3:latest`(Mac GPU, /api/embed, prefix 없음)로 임베딩, exact cosine top-k recall. e5 baseline = kebab `--mode vector`(run_019e89d0...). 청크 임베딩 911s(~25/s), npz 캐시 `bge_m3_chunks.npz`.

| 변형 | e5-large dense | bge-m3 dense | Δ |
|------|------|------|------|
| en (영→한 cross-lingual) | 18/18 | 17/18 | −1 |
| ko | 18/18 | 18/18 | = |
| syn (동의어) | 11/11 | 10/11 | −1 |
| abbr (약어) | 7/7 | 6/7 | −1 |
| **para (설명형)** | 14/18 | **17/18** | **+3** |
| **recall@10 합계** | **68/72 (0.944)** | **68/72 (0.944)** | **0** |
| recall@50 합계 | 70/72 | 71/72 | +1 |

bge-m3 미스(recall@10): nn_syn(뉴럴 네트워크 모델), dp_abbr(DP 알고리즘), **stk_para**(stack 설명형 — 양쪽 공통 잔존), re_en(regular expression).

**결론(Step 1)**: bge-m3 dense 는 **맞교환** — 설명형 +3, 용어/약어/영어 −3, 합계 동률. §2.3 KO-EN 연구("base bge-m3 ≈ base e5-large, 케이스별 한쪽씩")의 정량 재현. **dense 단독 임베더 교체는 정당화 안 됨**(이득 0, embedding_version cascade 재임베딩 비용만 발생). bge-m3 의 미검증 레버 = sparse+multivector hybrid(용어 손실을 sparse 가 회복하며 para 이득 유지 가능) — 단 별도/대형 작업(kebab 에 bge-m3 3-head 통합 필요).

## 6. 종합 결론 & 권고
- **별칭(doc-side expansion) = 제거 확정 후보.** Step 0: cross-lingual 은 e5 단독으로 이미 완벽, 별칭 기여는 para +2 뿐, 대가는 청크당 LLM(살아있는 KB 에 지속 불가). §2.1 문헌과 정합. **권고 = 별칭 기능 제거(또는 영구 default-off + 문서화), e5-large 유지.** → 사용자 2대 반론(총량·treadmill) 즉시 해소.
- **임베더 교체(e5→bge-m3 dense) = 보류.** Step 1: 이득 0(lateral). 추진 시에도 dense 단독 말고 **bge-m3 hybrid(sparse 포함)** 를 먼저 측정해야 의미 — 별도 조사 트랙.
- **잔존 약점(설명형 ~4쿼리, 특히 stack)**: 별칭으로도 bge-m3 로도 안 닫힘 → 별도 소형 과제(query-side single-pass §2.5 또는 bge-m3 sparse)로 분리, 우선순위 낮음.

**다음 행동**: 별칭 제거 spec → plan → 구현(gitea-pr 리뷰루프). bge-m3 hybrid 는 후속 조사 항목으로 파킹. 관련 메모리 [[project_expansion_perf]] · [[project_paraphrase_robustness]] · [[project_embedding_numa_backends]].

## 7. 딥리서치 — 별칭 대체 방법 (2026-06-03, 4-agent 병렬)

별칭이 정조준했던 목적(설명형/풀어쓴 질의 recall)을 사용자 제약(로컬, 비용∝질의 또는 색인-1회, 청크당 LLM 금지)에서 달성하는 방법을 4갈래 병렬 조사. 출처는 각 절 말미.

### 7.1 핵심 재프레이밍 (agent 1·4 수렴)
잔존 실패("마지막에 넣은 것을 먼저 꺼내는 자료구조"→스택)는 **reverse-dictionary / describe-to-term** 과제 — 설명에서 이름(용어)을 찾는, 본질적으로 **생성·추론** 문제. dense cosine(e5든 bge-m3든) 단독이 약한 이유. **함의: 빠진 표면 용어("스택/stack/LIFO")를 materialize 하는 방법 > 벡터만 평활화하는 방법(dense PRF 등).** 측정된 실패 분해(OFF): recall@10 68/72, recall@50 70/72 → **MisRanked ~2(top-50 안, top-10 밖) + Missing ~2(top-50 밖)**.

### 7.2 후보 shortlist (제약 적합)
| # | 방법 | 비용모델 | 설명형 효과 | 로컬 경로 | 통합 난이도 | 한계 |
|---|------|---------|-----------|----------|-----------|------|
| **A** | **heading/title chunk enrichment** (제목+가장 가까운 heading 을 청크 임베드 텍스트/FTS5 필드에 주입) | **per-doc, LLM 0**(heading 추출만, kebab `heading_path` 이미 존재) | terse doc 에 "손잡이" 부여 → Missing 완화. MC-indexing +16~43% recall(무학습) | 색인 1회 재임베딩 | 낮음 | 순수 paraphrase(용어가 doc 본문에도 없음)엔 부분적 |
| **B** | **임베더 교체 → `dragonkue/snowflake-arctic-embed-l-v2.0-ko`** | 색인 1회 재임베딩, LLM 0 | **e5 대비 Korean 전 벤치 우위**(Ko-StrategyQA·AutoRAGRetrieval·Belebele 설명형 + XPQA 용어 둘 다) — bge-m3 와 달리 **회귀 없는 업그레이드** | XLM-R-large·1024-dim·`query:` prefix = candle crate 거의 드롭인 | 낮음 | chunk >~1300토큰서 품질↓(긴 청크면 KURE-v1 고려) |
| **C** | **query-time rerank `bge-reranker-v2-m3`** (RRF top-50 재정렬) | **per-query**(O질의) | MisRanked 설명형의 정석 해결(cross-encoder 토큰 상호작용); 긴/설명형서 이득 최대 | **fastembed 4.9.1 `BGERerankerV2M3` 이미 보유**(ONNX int8 CPU/ M4 GPU) | 가장 낮음(신규 인덱스 0) | **Missing 못 고침**(top-50 밖). CPU FP32 느림→int8 필수. `dragonkue/...-ko` 파인튠 +3.5% |
| **D** | **term-style query expansion**(설명→핵심용어 ≤32토큰, 캐시, RRF 별 리스트 융합) | per-query, **캐시→amortized ~0** | reverse-dictionary 직접 공략(용어 materialize). pseudo-doc(HyDE) 아님 | 기존 Ollama 재사용 | 중간 | 드리프트 위험→원쿼리 융합 유지·하드질의만 게이트 |
| **E** | **bge-m3 sparse 헤드**를 RRF lexical 항으로 | 색인 1회(dense+sparse 1패스), LLM 0 | dense 의 용어/약어 손실 회복 가능(MLDR서 sparse>dense +10NDCG) | fastembed `SparseTextEmbedding`/ bge-m3 ONNX | 높음(3rd 인덱스+가중치) | FTS5 와 중복, dense 항은 Korean서 arctic-ko 보다 약함 |

### 7.3 수렴한 경고 (피할 것)
- **always-on HyDE / pseudo-document LLM**: 로컬 1~4B 에서 13s+/질의, 살아있는(long-tail) KB 서 환각·baseline 하회. → 쓰면 **term 변형 + 하드질의 게이트만**.
- **dense PRF 를 주해법으로**: 이미 recalled facet 만 강화, Missing 못 살림, 드리프트. (싼 add-on 이상 금물.)
- **학습형 QPP 를 트리거로**: 비신뢰·미일반화(2504.01101). → 대신 사용자가 이미 측정한 **near-tie Δcosine(0.003~0.005, [[project_crossscript_diagnosis]])** 를 corpus-보정 게이트로 사용.
- **SPLADE 전면 도입 / ColBERT 전체 색인**: Korean 약함·저장 폭증. (multi-vector 는 top-k rerank 로만.)
- **reranking 으로 Missing 기대**: 불가(1차에 없으면 못 살림).

### 7.4 권고 — 측정-우선 계층 (싼 것부터)
0. **무료 점검**: e5 `query:`/`passage:` prefix 정확 적용 확인(불일치 시 verbose↔terse 격차 악화). 코드 0.
1. **Layer A (heading enrichment)** — 가장 싸고 제약 완벽 적합, kebab 이 `heading_path` 보유. 재임베딩 1회 후 골든 측정. Missing 완화 기대.
2. **Layer B (arctic-ko 임베더)** — bge-m3 와 달리 회귀 없는 Korean 업그레이드 가설. candle 드롭인. A 와 직교·중첩 가능. 측정.
3. **Layer C (bge-reranker-v2-m3 top-50)** — MisRanked 해결 + **MisRanked:Missing 비율 진단 도구**(한 실험으로 둘 다). 이미 보유.
4. **Layer D (near-tie 게이트 term-expansion/triggered HyDE)** — A~C 후에도 순수 paraphrase 잔존 시에만, 하드질의에만.

각 계층은 기존 golden/variant eval 로 검증, 회귀(잘 되던 질의) 감시.

### 7.5 반대 의견 (agent 4, 정직히 기록)
단일 사용자 KB(본인이 쓴 corpus, 존재를 기억)에선 놓친 paraphrase 질의는 **용어로 재입력하면 초 단위 복구** — 멀티테넌트엔 없는 무료 fallback. 공격적 expansion 의 false-positive 비용 > 가끔의 miss 비용일 수 있음. reverse-dictionary 는 본질적으로 어려워(생성 추론) 무리한 추격은 16/18 잘 되는 걸 회귀시킬 위험([[project_ranking_deferred]]). **현실적 결론: 싼 A(+B) 로 80% 잡고, 잔존 paraphrase 꼬리는 "알려진 한계"로 문서화** — eval 이 그게 실사용의 큰 반복 비중임을 보이지 않는 한.

### 7.6 출처
- reverse-dictionary: [GEAR 2412.06654](https://arxiv.org/pdf/2412.06654) · [unified RD 2205.04602](https://arxiv.org/abs/2205.04602) · [RD probe 2402.14404](https://arxiv.org/pdf/2402.14404)
- query expansion: [CTQE 2509.02377](https://arxiv.org/abs/2509.02377) · [QE survey 2509.07794](https://arxiv.org/html/2509.07794) · [knowledge-leakage 2504.14175](https://arxiv.org/html/2504.14175v1) · [HyDE on local 1B/4B 2506.21568](https://arxiv.org/html/2506.21568v1)
- PRF: [LLM-VPRF 2504.01448](https://arxiv.org/abs/2504.01448) · [PRF pitfalls TOIS 3570724](https://dl.acm.org/doi/10.1145/3570724)
- rerank: [bge-reranker-v2-m3](https://huggingface.co/BAAI/bge-reranker-v2-m3) · [dragonkue ko 파인튠](https://huggingface.co/dragonkue/bge-reranker-v2-m3-ko) · [onnx-community ONNX](https://huggingface.co/onnx-community/bge-reranker-v2-m3-ONNX) · [FlashRank](https://github.com/PrithivirajDamodaran/FlashRank) · [Scaling Laws for Reranking 2603.04816](https://arxiv.org/pdf/2603.04816) · [SlideGar 2501.09186](https://arxiv.org/html/2501.09186v1)
- 임베더: [arctic-embed-l-v2.0-ko](https://huggingface.co/dragonkue/snowflake-arctic-embed-l-v2.0-ko) · [Arctic-Embed 2.0 2412.04506](https://arxiv.org/html/2412.04506v1) · [ko-embedding-leaderboard](https://github.com/OnAnd0n/ko-embedding-leaderboard) · [KURE](https://github.com/nlpai-lab/KURE) · [bge-m3 2402.03216](https://arxiv.org/html/2402.03216v3) · [bge-m3-onnx dense+sparse](https://github.com/yuniko-software/bge-m3-onnx)
- verbose-query/asymmetry: [Key Concepts in Verbose Queries (SIGIR'08)](https://dl.acm.org/doi/abs/10.1145/1390334.1390419) · [Collapse of Dense Retrievers 2503.05037](https://arxiv.org/pdf/2503.05037) · [MC-indexing 2404.15103](https://arxiv.org/pdf/2404.15103) · [Elastic title-into-chunk](https://www.elastic.co/search-labs/blog/multi-vector-documents) · [Adaptive-RAG 2403.14403](https://arxiv.org/pdf/2403.14403) · [QPP limits 2504.01101](https://arxiv.org/abs/2504.01101)

## 출처
- [When do Generative Query and Document Expansions Fail? (2309.08541)](https://arxiv.org/pdf/2309.08541)
- [Korean-English Cross-Lingual Retrieval data-centric study (2507.08480)](https://arxiv.org/html/2507.08480)
- [BGE M3-Embedding (2402.03216)](https://arxiv.org/html/2402.03216v3) · [HF BAAI/bge-m3](https://huggingface.co/BAAI/bge-m3) · [Ollama bge-m3](https://ollama.com/library/bge-m3)
- [Doc2Query++ (2510.09557)](https://arxiv.org/abs/2510.09557) · [Doc2Query-- When Less is More](https://www.semanticscholar.org/paper/7b2e78d4e7986914ae633fa6b30e73bad8a2b2c1)
- [CTQE — Upcycling Candidate Tokens for Query Expansion (2509.02377)](https://arxiv.org/pdf/2509.02377)
- [Query Expansion in the Age of LLMs: A Survey (2509.07794)](https://arxiv.org/abs/2509.07794)
- [Best Open-Source Embedding Models 2026 (BentoML)](https://www.bentoml.com/blog/a-guide-to-open-source-embedding-models)
- [ColBERT / late interaction storage tradeoff (Weaviate)](https://weaviate.io/blog/late-interaction-overview) · [PLAID (2205.09707)](https://arxiv.org/pdf/2205.09707)
- [MILCO — multilingual learned sparse (2510.00671)](https://www.arxiv.org/pdf/2510.00671)
