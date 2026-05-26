# v0.18.0 dogfood retest (PR-9d closure)

Post-PR-9c-2 dogfood retest 결과. PR-1~PR-8 머지 후 발견된 S7 caffeine hallucination (multi-hop synthesize 가 chunks 와 무관한 Adam optimizer gradient 식을 답변으로 emit) 의 NLI-based post-synthesis verification 효과 측정.

- 환경: v0.18.0 candidate binary, Ollama gemma3:4b, fastembed multilingual-e5-small, mDeBERTa-v3-base-xnli-multilingual ONNX
- Config: `[rag] nli_threshold = 0.5`, `score_gate = 0.3`
- Corpus: `/build/cache/dogfood-v018/` (PR-1~PR-8 와 동일)
- Date: 2026-05-26

## 결과 비교

| case | query | PR-8 baseline | PR-9 retest | 판정 |
|---|---|---|---|---|
| **S7** | "What is the chemical formula of caffeine?" | `grounded=true, refusal_reason=null`, **답변=Adam gradient 공식 (hallucination)** | `refusal_reason=nli_verification_failed`, `nli_score=0.0035`, `nli_threshold=0.5` | ✅ **HALLUCINATION FIXED** |
| S1 | "컴파일러 파이프라인 ... 출력 데이터 의존성" | `refusal_reason=llm_self_judge` | `refusal_reason=nli_verification_failed`, `nli_score=0.058` | ✅ 둘 다 reject, NLI 가 더 deterministic |
| S3 | "Why does kebab combine multilingual-e5, LanceDB, and RRF together?" | `refusal_reason=llm_self_judge` | `refusal_reason=nli_model_unavailable`, latency 313s | ⚠ **consistent fail — follow-up 필요** |
| S10 | "Why did the dinosaurs go extinct?" (KB outside) | `refusal_reason=llm_self_judge` | `refusal_reason=nli_verification_failed`, `nli_score=0.0028` | ✅ 둘 다 reject, NLI 가 더 deterministic |

## S7 hallucination root cause 해결 확정

PR-8 까지 multi-hop synthesize 가 chunks 와 entail 안 되는 답변을 *silent emit* 했음 — LLM-self-judge ceiling (synthesize prompt 의 "self-check rule" 가 caffeine 같은 single-fact 부재 case 를 못 잡음). PR-9c-2 의 step 8.5 NLI hook 가 entailment 0.0035 (0.35%) 로 검출 → graceful refusal.

PR-9 의 *deterministic external verifier* (mDeBERTa-v3 XNLI) 가 LLM-self-judge 의 *probabilistic ceiling* 을 극복.

## S3 의 nli_model_unavailable (follow-up)

S3 만 `nli_model_unavailable` 로 fail (S1/S7/S10 의 entailment 측정은 정상). 잠재 원인:
- mDeBERTa session inference 가 *특정 input 에 대해* panic / err 변환 (`tokenizers::encode` 실패, `Session::run` shape 검증 fail 등)
- 또는 *eager session 재 load* 가 process 단위 보다 invocation 단위에서 race
- `KEBAB_LOG=info,kebab_rag=debug,kebab_nli=debug` 로 retry 시 debug log emit 안 됨 (env 이름 ignored 또는 tracing subscriber init 안 됨) — 진단 어려움

본 closure 의 scope 외. `tasks/HOTFIXES.md` 에 follow-up entry 등록 (HOTFIX candidate #15 와 별개 — kebab-nli 의 *간헐 / 특정 input dependent* issue).

## 비교 측정값

| metric | PR-8 baseline | PR-9 retest |
|---|---|---|
| S7 latency | 158s | 241s (NLI inference 추가 + first-run model download — 첫 호출만) |
| S1 latency | (post-pr8 시점 비교 baseline 부재 — `results/s1-multihop.json` 는 더 이전 시점, 같은 quality 단순 비교 불가) | 224s |
| S10 latency | (동상) | 79s |
| RAM peak | ~5-6 GB (gemma3:4b) | ~7-8 GB (gemma3:4b + ONNX session ~600 MB) |
| Disk (NLI model) | 0 | 1.1 GB (model 280 MB + tokenizer 16 MB + blobs/locks/snapshots overhead) |

S1/S10 의 *동일 시점 baseline* 가 `results/` 하나에만 있어 timeline 비교가 부정확. S7 만 `results/post-pr8/` 에 retest 보존되어 latency 비교 의미 있음 (158s baseline → 241s with NLI first-run; 두번째 호출은 240s - 30s download = ~210s 추정).

NLI inference latency 자체는 ~10-50 ms per call (spec §2.1 명세 일치). 첫 호출 시 model load (~30-60s) + multi-hop synthesize latency 가 dominant.

## Sample wire outputs

본 디렉토리의 `s{1,3,7,10}-multihop-post-pr9.json` 4 sample.

Schema 정합:
- `answer.v1` 의 신규 `verification: { nli_score, nli_threshold, nli_passed }` field 확인.
- `refusal_reason` 의 `"nli_verification_failed"` / `"nli_model_unavailable"` 두 신규 값.
- pre-v0.18 reader 는 `verification` field 가 `skip_serializing_if = None` 으로 omit 되므로 backward-compat (PR-9c-1 의 additive minor wire).

## NLI threshold tuning iteration trigger?

현재 결과로는 *없음*:
- 모든 PASS 케이스 (S7/S1/S10) 가 *명백히 ungrounded* 답변에서 entailment < 0.1 — 0.5 threshold 가 *과도하게 엄격* 하지 않음.
- 모든 RETEST 가 PR-8 baseline 의 `llm_self_judge` refuse 와 일치 (false positive 없음).
- v0.18.1 candidate: S3 issue 진단 + 만약 happy-path (실 grounded 답변) 가 false positive 로 reject 되는 케이스 측정 시 threshold tuning.

## 한계

- happy-path (NLI 통과하는 실 grounded 답변) 직접 측정 부재 — 모든 retest 가 refuse path. dogfood corpus 가 *부정 / 부재 사실 위주* 라 happy path 의 sample 부족. v0.18.1 candidate: corpus 보강.
- gemma3:4b 의 synthesize quality 가 baseline — 더 큰 모델 (gemma4:e4b 8B Q4) 에서는 happy path 확률 ↑ 가능. release notes 의 RAM 권장 가이드 의 의미.
- S3 의 follow-up.
