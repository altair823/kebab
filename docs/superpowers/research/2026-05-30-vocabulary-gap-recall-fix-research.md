---
title: 어휘격차(vocabulary-gap) pool-miss 해결 — 딥리서치 레퍼런스
date: 2026-05-30
type: research-reference
provenance:
  - "deep-research 워크플로 (wf_e76011c6-de8): 5 angle, 22 sources fetch, 103 claims 추출, 25 claim 3-vote 적대적 검증, 22 confirmed / 3 killed. 104 agent, ~3.5M subagent tokens."
related:
  - docs/superpowers/handoffs/2026-05-29-query-paraphrase-robustness-phase1-handoff.md (B 어휘격차 우세 진단)
  - docs/superpowers/research/2026-05-29-crossscript-synonym-retrieval-research.md (선행 — rerank 결정 중심)
  - memory: project_paraphrase_robustness, project_crossscript_diagnosis
---

# 어휘격차 pool-miss 해결 — 딥리서치 레퍼런스

> Phase 1 진단(변형 일관성 측정)에서 **B(어휘격차) 우세** 확인 — 같은 의미를 다른 단어로 물으면
> 정답이 top-50 pool 에도 안 들어옴(recall@50=0), rerank 불가. 그 실패 모드 전용 처방을 조사.

## 0. 질문 (요약)

CPU-only 로컬 RAG(FTS5/BM25 + LanceDB dense e5-large + RRF, Rust/fastembed-rs/LanceDB) 에서
"같은 의미 다른 표현(동의어·풀어쓴 문장·한/영)이 top-50 pool 에도 못 들어오는" 실패를 가장
좋은 비용대비로 고치는 법. 제약: per-query LLM 확장 거부("밑 빠진 독"), e5-large 유지(bge-m3
dense 는 실측 더 나빴음), 코드 식별자 정확매칭 보존.

## 1. 적대적 검증 통과 결론 (confirmed)

### 1.1 색인시 doc-side expansion(doc2query/docTTTTTquery)이 pool-miss 의 최선책 (3-0)
- **유일하게 lexical pool 자체를 키운다**(rerank 아님): docTTTTTquery README — MS MARCO doc
  Recall@1000 0.9180→0.9490(rerank 없는 1차 BM25 지표 → pool 진입 증가 실증). passage MRR@10
  18.6→27.2, **per-query +9ms 만**(55→64ms, T5 추론은 색인시 1회·query 무영향).
- **메커니즘이 어휘격차 정조준**: SIGIR 2024 — N개 예측 query append 가 없던 term 주입(TF↑),
  Doc2Query model card "generated queries contain synonyms → close the lexical gap".
- **Doc2Query++(2510.09557, 2025-10)**: 5개 BEIR 에서 sparse·dense 둘 다 **Recall@100** 개선
  (SCIDOCS 0.3323→0.3749, FiQA 0.5864→0.6197). Recall@100=pool-membership → pool 확장 확인.
- 출처: github.com/castorini/docTTTTTquery, doc2query/msmarco-14langs-mt5-base-v1, mzzm24-sigir, 2510.09557.

### 1.2 ⚠️ vanilla 다국어 doc2query(mt5)는 한/영을 못 잇는다 (3-0)
- model card: "input passage 의 **같은 언어**로 query 생성". mMARCO 14개 언어에 **한국어 없음**.
- 귀결: doc2query 단독은 영어 paraphrase·동의어 pool-miss(raft "how nodes agree…")는 고치나,
  **KO↔EN 갭(역전파→영어 backprop doc)은 못 고침** → 색인시 *교차언어* 대체 query 생성이 추가로 필요.

### 1.3 MILCO(교차언어 learned-sparse)가 한/영 갭 직접 해결, 단 배포 경로 없음 (3-0)
- 2510.00671(2025-10): query·doc 를 "공유 English lexical space"로 매핑. MKQA(한국어 포함)
  zero-shot R@100 **76.6**(BGE-M3-Sparse +69%, BM25 +92%). **그러나 560M 연구모델, ONNX/
  fastembed-rs 체크포인트 미확인** → research signal, turnkey 아님. (0.61ms 는 index lookup 만, 인코딩 제외.)

### 1.4 BGE-M3 sparse 채널은 CPU-native 추가 가능, 단 한/영은 못 고침 (3-0)
- 1 forward 로 dense+sparse+ColBERT 산출, fastembed-rs `BGEM3Q`(CPU 양자화, CUDA 주면 오히려 실패).
- 단일언어 향상: MIRACL Dense+Sparse 68.9 > Dense 67.8 → **3rd RRF 채널 후보**.
- **교차언어 약함**: BGE-M3 논문도 sparse cross-lingual MKQA 45.3 vs dense 67.8 "다른 언어라 공존 term
  거의 없음". → KO↔EN 갭엔 무용. ("sparse 가 모든 언어서 BM25 압도" 주장은 **0-3 기각**.)

### 1.5 turnkey SPLADE 는 새 corpus 에서 자동 해결 못 함 (3-0)
- LSR 은 term expansion 으로 어휘격차 겨냥하나, SOTA(Echo-Mistral-SPLADE BEIR 55.07)는 **Mistral-7B
  로 학습**(무겁다). AACL 2022: "SPLADE 는 저빈도 단어 exact match 에 약함 + 어휘/빈도 domain shift
  시 성능 저하". → 개인 혼합 KO/EN corpus 에 drop-in 기대 금물, 코드 식별자 정확매칭도 약점.

### 1.6 query-side(HyDE, Vector-PRF)는 이 제약에 부적합 (confirmed)
- **HyDE**: query 마다 LLM 이 가설답변 생성(1~5s/query) = 사용자가 거부한 "밑 빠진 독" 바로 그것.
- **Vector-PRF**: per-query 생성은 피하나 2-pass 필요 + **recall 개선 주장 0-3 기각**(1.6/6.2/7.7%
  Recall@100 gain 전부 refute). → 이 실패 모드 해결 증거 없음.

## 2. 기각 (killed — 믿지 말 것)
- "BGE-M3 sparse 가 모든 언어서 BM25 압도" (0-3).
- "Vector-PRF 가 Recall@100 을 1.6~7.7% 올린다(pool 확장)" (0-3).
- "Vector-PRF 가 여러 데이터셋서 dense 효과 개선" (0-3).

## 3. 권고 (minimal combination, medium conf — 합성/추론)

**(1) 색인시 doc2query-style 확장 → 별도 FTS5 lexical 필드** (원문 body 필드는 그대로 verbatim
index → 코드 식별자 정확매칭 보존, append-not-replace). RRF 가 {body-BM25, expansion-BM25, e5-dense} 융합.
**(2) 문서당 같은언어 query + 소수의 교차언어(KO↔EN) 대체 표현/번역**을 색인시 1회 생성(로컬 LLM
= gemma, **per-query 아님 → 사용자 제약 충족**). 역전파→backprop doc 의 직접 해법.
**(3) (선택) BGE-M3 sparse(fastembed-rs BGEM3Q)를 4th RRF 채널**로 단일언어 lift, e5-large dense 유지.
필터(Doc2Query--/++ topic-coverage)로 환각·index 팽창 제어.

- 사용자 제약 충족: 색인시 1회(per-query LLM 아님) + e5-large 유지 + 정확매칭 보존.
- 엄격 no-LLM 원하면 (2) 대신 seq2seq mt5 doc2query 로 폴백(단 한국어 미커버 → 한/영 갭 부분만 해결).

## 4. 미해결 질문 (= Phase 2 실험 설계, **기존 variant eval 로 측정 가능**)
1. **색인시 KO↔EN 대체 query 생성이 우리 corpus 에서 recall@50 을 0→양수로 올리나?** — 핵심 미검증
   고리. `/build/dogfood` golden + `kebab eval variants` 로 직접 측정(또 프록시 금지).
2. ONNX/fastembed 호환 교차언어 learned-sparse(MILCO 또는 distill) 체크포인트가 있나, 아니면
   교차언어는 전적으로 색인시 doc expansion 으로만 풀어야 하나.
3. doc2query 가 개인 KB(수천 doc/수만 chunk)의 FTS5 index 를 얼마나 부풀리나. Doc2Query--/++ 필터
   가치 있나 vs plain mt5.
4. e5 dense 유지하고 BGE-M3 **sparse 만** 추가 시 paraphrase/동의어 recall@50 순이득인가, 약한
   다국어 sparse 가 RRF 에 노이즈만 더하나.

## 5. 핵심 caveat (시점 민감)
- 최강 교차언어 근거(MILCO, Doc2Query++)는 2025-10 단일 논문·저자 보고 벤치 — research signal.
- **교차언어 권고(색인시 KO↔EN 생성)는 합성/추론** — "index-time LLM translation 이 한/영 recall@50
  갭을 닫는다"를 직접 벤치한 논문 없음. confirmed fact 들의 논리적 조합. → **우리 corpus 측정 필수**.
- docTTTTTquery 의 MS MARCO recall 증가는 modest(+3.4% rel) — 순수 pool-rescue 크기는 우리 corpus 미검증.
- 정확매칭 보존은 architectural 논증(별도 필드), 코드 corpus 직접 측정 아님.

## 6. 출처
docTTTTTquery — github.com/castorini/docTTTTTquery · Doc2Query++ — arxiv 2510.09557 ·
mt5 14-lang doc2query — hf.co/doc2query/msmarco-14langs-mt5-base-v1 · SIGIR2024 doc-exp — jmmackenzie.io/pdf/mzzm24-sigir.pdf ·
MILCO — arxiv 2510.00671 · BGE-M3 — arxiv 2402.03216 · bge-m3-onnx — github.com/yuniko-software/bge-m3-onnx ·
Mistral-SPLADE — arxiv 2408.11119 · SPLADE domain-shift — arxiv 2211.03988 · HyDE/PRF — arxiv 2511.19349, 2504.01448, 2108.11044 ·
fastembed-rs — github.com/Anush008/fastembed-rs · KURE — github.com/nlpai-lab/KURE · arctic-embed-ko — hf.co/dragonkue/snowflake-arctic-embed-l-v2.0-ko
