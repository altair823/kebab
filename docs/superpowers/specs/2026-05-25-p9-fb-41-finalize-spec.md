---
title: "p9-fb-41 finalize — multi-hop RAG post-dogfood safety hardening + v0.18.0 cut"
date: 2026-05-25
task_id: p9-fb-41-finalize
phase: P9
status: approved-by-team
target_version: 0.18.0
contract_source: ./2026-04-27-kebab-final-form-design.md
contract_sections: [§3.8 RAG, §7 RAG pipeline]
predecessor: ./2026-05-25-p9-fb-41-multi-hop-rag-design.md
review_round: 5
review_outcome: |
  All 4 OMC team reviewers APPROVE after 5-round convergence.
  - architect: APPROVE (round 2)
  - planner: APPROVE (round 2)
  - document-specialist: APPROVE (round 3)
  - critic: APPROVE (round 5)
  Δ-severity 5-round 단조감소: 1C+9M+3m → 0+0+1NIT.
  잔존 NIT (R5-NEW-NIT-1) closure 됨 (release notes wording).
---

# p9-fb-41 finalize — multi-hop RAG post-dogfood safety hardening

## 동기

predecessor spec (`2026-05-25-p9-fb-41-multi-hop-rag-design.md`) 가 정의한 multi-hop pipeline 의 PR-1 ~ PR-8 모두 머지 완료. v0.18 pre-cut 도그푸딩 (`/build/cache/dogfood-v018/results/SUMMARY.md`) 에서 발견된 *safety regression* 을 닫고 v0.18.0 cut 으로 가는 finalize spec.

predecessor 의 frozen contract 는 변경 없음 — 본 spec 는 *delta* 만:

1. dogfood 발견 (S7 hallucination) 의 진단 + fix path 정리.
2. PR-7 (probe gate) + PR-8 (pool 축소 + prompt rule) 의 부분 fix 결과.
3. PR-9 (NLI-based post-synthesis verification) 의 최종 fix 설계.
4. v0.18.0 cut steps.

## 1. 도그푸딩 진단 (S7)

**Query**: `What is the chemical formula of caffeine?` (KB 에 없는 fact).

| path | top_score | grounded | latency | answer |
|---|---|---|---|---|
| single-pass (hybrid) | 0.5 | false (LlmSelfJudge) | 30s | "근거가 부족하다" ✓ |
| multi-hop pre-fix | 0.5 | true ✗ | 614s | hallucination: "C₉H₁₅N₃O [#6]" (Adam optimizer 의 g_t 수식을 인용) |
| multi-hop PR-7 | 0.5 | true ✗ | 143s | hallucination (probe gate top_score 0.5 > 0.30 통과) |
| multi-hop PR-8 | 0.5 | true ✗ | **158s** (4× 개선) | hallucination (LLM 새 prompt rule 무시) |

### 1.1 진단 정합

1. **single-pass 의 LlmSelfJudge** = LLM 의 self-judgement 가 *uncorrelated chunks* 에 대해 "근거 부족" 인지. *probabilistic safety* — gemma3:4b 환경에서 우연히 정답 path. 다른 케이스 / 다른 LLM 에서 동일 reliability 보장 없음.
2. **multi-hop pre-fix 의 hallucination** = synthesize prompt 가 *5 sub-questions + 30 chunks* 의 large context 에서 LLM 의 self-judgement 잃음. `score_gate` 도 `hits[0].fusion_score` 만 검사 — multi-hop pool 의 union 이 한 sub-query 의 top score 가 gate 위면 통과.
3. **PR-7 probe gate** = single-pass 와 동일한 *원본 query* retrieve top_score 검사. 그러나 hybrid mode 의 RRF default score 가 0.5 (vector embedding 의 false positive — caffeine 와 Adam optimizer 수식 chunk 사이 semantic 유사도) → probe 도 통과.
4. **PR-8 prompt rule + pool 15** = synthesize prompt 강화 + size 축소 → latency 4× 개선. 그러나 gemma3:4b 의 prompt-following ceiling — strong rule 도 무시.

**근본 원인**: LLM-self-judge 기반 groundedness check 의 ceiling (gemma3:4b 한정 관측 — larger LLM 의 ceiling 도 unknown). *deterministic external verifier* 필요.

### 1.2 alternative root cause 검토 (왜 NLI path 인가)

다음 lighter alternatives 도 검토했으나 NLI path 채택:

| alternative | 효과 | 한계 / 거부 이유 |
|---|---|---|
| `[rag] vector_min_score = 0.4` knob (RRF *원본 vector cosine* threshold 추가) | caffeine ↔ Adam optimizer 의 vector 유사도 차단 가능 | RRF formula `score = sum(1/(60+rank))` 가 top-K 통과 시 *원본 cosine 낮아도* RRF 0.5 → vector_min_score 추가 = retrieval-side fix. Synthesis-side hallucination 의 *근본 원인 (LLM 의 prompt-following ceiling)* 미해결. 다른 query 패턴 (paraphrase chunk 가 retrieve) 의 hallucination 같은 path. |
| LLM 모델 업그레이드 (gemma2:9b / qwen2.5:7b) | larger LLM 의 prompt-following 능력 강화 → "근거 부족" rule 잘 따를 가능성 | CPU only 16 GB RAM 환경에서 9B+ Q4 모델은 RAM/latency 부담 ↑ (HOTFIXES 2026-05-25 v0.17.0 post-dogfood entry). 사용자 환경 의존성 ↑. *모델 무관 safety floor* 가 본 spec 의 목표. |
| LLM-as-judge (별 LLM call 으로 yes/no) | 모델 prompt-following 안 의존 — 별 call 의 binary judgement | 추가 LLM call → multi-hop latency 더 늘어남 (현재 158s + 10-30s). 그리고 *judge LLM 도 prompt-following ceiling* 가짐 — 같은 문제 재발 가능. |
| **NLI post-synthesis verification (선택)** | deterministic + lightweight + 학계 표준 | model dep + first-run download 부담. *그러나 단일 280 MB ONNX 가 모든 multi-hop ask 의 safety floor 제공*. |

NLI 가 *deterministic verifier* 의 약속 (LLM 의 stochastic self-judge 와 직교) + production proven (Auto-GDA, MedTrust-RAG) + multilingual 가능 (multilingual NLI model) 의 3 axis 모두 만족.

(향후 v0.19+ 의 ceiling 측정 / dogfood iteration 에서 LLM-as-judge 또는 cross-encoder reranker 도 보조 path 로 검토 가능 — `nli_threshold = 0.0` disable 옵션 항상 보존.)

### 1.3 LLM upgrade vs NLI 의 future 관계

§1.2 의 LLM 업그레이드 path 가 v0.19+ 의 *NLI 와 병행* 또는 *NLI 대체* 가능성:

- **병행**: larger LLM 도 hallucinate 가능 — NLI 가 safety floor 유지. *defense in depth*.
- **대체**: 만약 future LLM (예: gemma4:e4b 의 instruction-tuned variant) 가 prompt-following ceiling 가 사라지면 NLI cost 정당화 약화 — `[rag] nli_threshold = 0.0` disable 로 opt-out.

본 v0.18 spec 의 NLI 는 *opt-in default OFF* (§2.6) 이라 사용자가 환경에 맞춰 enable. v0.19+ 의 measurement 후 default ON / OFF 결정.

## 2. PR-9 — NLI-based post-synthesis verification

학계 / industry 표준 (Self-RAG / CRAG / Auto-GDA / MedTrust-RAG) 의 결론: *post-synthesis groundedness verification* 이 정답 path. **multilingual NLI ONNX model** (~280 MB) 이 `(premise = packed_chunks, hypothesis = answer)` entailment 검사 → score < threshold 면 refuse.

### 2.1 Model

- **HuggingFace repo (production default)**: `Xenova/mDeBERTa-v3-base-xnli-multilingual-nli-2mil7` — Xenova org 의 ONNX export.
- **원본 PyTorch weight**: `MoritzLaurer/mDeBERTa-v3-base-xnli-multilingual-nli-2mil7` (Apache-2.0 lic.). Xenova 의 ONNX export 는 *Optimum* 으로 생성된 변환본. config 의 default 는 ONNX 호스팅하는 `Xenova/...` 사용.
- 280 MB ONNX (FP32). Q8 양자화 variant 도 Xenova 에 별 file (`onnx/model_quantized.onnx`) 있음 — v0.19+ 에서 옵션 추가 검토.
- 3-way classifier: `[entailment, neutral, contradiction]` (XNLI `id2label` 표준).
- 100+ multilingual (Korean + English 필수).
- CPU inference: ~10-50 ms per (premise, hypothesis) pair (mDeBERTa-base 기준).

**pre-flight check (PR-9a 시작 전 manual 확인)**:
```sh
curl -I https://huggingface.co/Xenova/mDeBERTa-v3-base-xnli-multilingual-nli-2mil7/resolve/main/onnx/model.onnx
curl -I https://huggingface.co/Xenova/mDeBERTa-v3-base-xnli-multilingual-nli-2mil7/resolve/main/tokenizer.json
```
두 HEAD 모두 `200 OK` 면 진행. 404 면 PR-9 의 design re-evaluation (다른 ONNX repo 또는 self-export via Optimum).

#### 2.1.1 대안 모델 trade-off (informational)

| 모델 | size | lang | quality 차이 | 적합도 |
|---|---|---|---|---|
| `xlm-roberta-large-xnli` | 1.5 GB | 100+ multilingual | ~3-5% 더 높음 | 16 GB RAM 환경에서 LLM + lance + NLI 동시 cold start 부담 (overflow risk). |
| **`Xenova/mDeBERTa-v3-base-xnli-multilingual-nli-2mil7` (선택)** | **280 MB** | **100+ multilingual** | **baseline** | **균형 — 본 spec 의 default** |
| `MiniLM-L12-mnli-xnli` | 110 MB | multilingual (좁음) | ~5-10% 낮음 | Korean 의 quality 약함 — kebab corpus 의 KR+EN mix 와 부적합. |

선택 사유: kebab 의 사용자 환경 (CPU only, 16 GB RAM, KR+EN mix) 에서 *유일한 균형점*. 모델 교체 시 본 표 + dogfood retest 측정값 함께 갱신.

### 2.2 Architecture

```
crates/kebab-nli/  (신규 crate, trait + impl 한 곳)
├── Cargo.toml
└── src/
    ├── lib.rs     — NliVerifier trait + NliScores struct + softmax helper
    └── onnx.rs    — OnnxNliVerifier (ort + tokenizers + hf-hub)
```

**Trait + impl 동일 crate 정당화** (vs `kebab-embed` + `kebab-embed-local` 패턴):

- v0.18 scope = ONNX adapter 하나만. trait split crate 의 *현재* 가치 0.
- v0.19+ 에서 candle / CUDA / remote adapter 등장 시 `kebab-nli-onnx` 분리 가능 — *그때 breaking change* 는 internal API only (kebab-app 만 영향, *wire 무관*). PR-9 단순화 우선.
- §8 self-review 에 향후 split 시 trigger 명시.

#### 2.2.1 Trait surface

```rust
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct NliScores {
    pub entailment: f32,    // production accept signal
    pub neutral: f32,
    pub contradiction: f32, // observability
}

impl NliScores {
    pub fn faithfulness(&self) -> f32 { self.entailment }
    pub fn from_xnli_logits(logits: [f32; 3]) -> Self { /* softmax + wrap */ }
}

pub trait NliVerifier: Send + Sync {
    fn score(&self, premise: &str, hypothesis: &str) -> Result<NliScores>;
}
```

`NliScores::faithfulness()` 가 `entailment` channel 반환 — production accept rule (`entailment >= threshold` = grounded).

#### 2.2.2 OnnxNliVerifier

- `ort::Session` (transitive download-binaries 으로 fastembed 와 같은 ONNX runtime).
- `tokenizers::Tokenizer` (SentencePiece via mDeBERTa tokenizer.json).
- `hf-hub::api::sync::Api` 가 first-run model + tokenizer download.
- **Lazy init**: 첫 `score` 호출 시 model + tokenizer load. 후속 호출 reuse (OnceLock 또는 OnceCell).
- **Cache dir**: `{config.storage.model_dir}/nli/<sanitized-model-id>/{model.onnx, tokenizer.json}`. fastembed 의 model cache 와 sibling. sanitization 은 `/` → `_` 로 (`Xenova/mDeBERTa-...` → `Xenova_mDeBERTa-...`).
- **Failure handling**: download 실패 (network / disk / corrupt) 시 `RefusalReason::NliModelUnavailable` (단일 ask) 또는 facade construction 시 verifier=None 으로 graceful — §2.6 참조.

#### 2.2.3 Input encoding + truncation

mDeBERTa-v3 의 `max_seq_len = 512` token. multi-hop 의 packed_chunks (15 chunks × ~300-500 token = 4500-7500 token) 가 무조건 초과 → **명시적 truncation 정책 필수**:

```rust
let mut encoding_params = tokenizers::EncodeInput::Dual(premise, hypothesis);
tokenizer
    .with_truncation(Some(tokenizers::TruncationParams {
        max_length: 512,
        strategy: tokenizers::TruncationStrategy::OnlyFirst, // premise (chunks) 만 truncate
        stride: 0,
        direction: tokenizers::TruncationDirection::Right,    // 끝부터 잘림
    }))?
    .encode(encoding_params, /*add_special_tokens=*/true)?
```

- **`OnlyFirst`**: hypothesis (answer) 는 보전, premise (chunks) 끝부터 truncate. answer 가 잘리면 entailment 가 *임의로 fail* — 절대 회피.
- **packed_text pre-budget in pipeline (옵션)**: `kebab-rag` 가 NLI 호출 전 packed_chunks 를 self-truncate. PR-9c-2 에서 helper `truncate_for_nli(premise: &str, hypothesis: &str) -> (String, bool)` 작성 — `max_seq_len = 512` 는 helper 내부 상수 `MAX_NLI_PREMISE_CHARS` 로 hardcode (v0.18 scope 단일 NLI model 가정). signature single source of truth = §3 PR-9c-2.

회귀 핀 (PR-9c unit test): `long_premise_truncation_preserves_hypothesis_score` — premise 가 10000-token 일 때 score 가 정상 (panic / NaN 없음). truncation indicator (`encoding.get_overflowing()`) 비어 있지 않음 검증.

#### 2.2.4 Inference

```
input_ids        : [1, seq_len] i64
attention_mask   : [1, seq_len] i64
→ Session::run
→ logits          : [1, 3]      f32
→ softmax(logits) → NliScores
```

mDeBERTa-v3 는 token_type_ids 없음 (single-segment encoding). ort input name 확정:
- input: `input_ids`, `attention_mask`
- output: `logits`

(PR-9a 의 pre-flight check 에서 ONNX 의 `onnx.SessionInfo::inputs()` / `outputs()` 출력으로 검증 후 lock — 다른 name 이면 spec 갱신.)

### 2.3 Pipeline integration

`RagPipeline::ask_multi_hop` 의 step 8.5 (synthesize 후, citation extract 전):

**Empty answer (stream abort / LM crash) 의 처리**: synthesize 가 empty `acc` 반환 시 step 8.5 *skip* — 이미 별 path 의 refusal 처리 (예: `RefusalReason::LlmStreamAborted` for fb-33 cancel) 가 이전 단계에서 결정. 본 step 8.5 의 NLI verify 는 *non-empty answer* 에 대해서만 호출 — empty hypothesis 가 NLI tokenizer 의 edge case 진입 회피. PR-9c-2 의 `ask_multi_hop` integration 시 `if !acc.trim().is_empty() { /* step 8.5 */ }` 가드 추가.

```rust
// 8.5 — NLI groundedness verification (multi-hop only in v0.18 scope)
// §2.7: single-pass `ask` 는 LlmSelfJudge 그대로. NLI 미적용.
let verification = if self.config.rag.nli_threshold > 0.0 {
    let v = self.verifier.as_ref().expect(
        "verifier must be Some when nli_threshold > 0.0 \
         (facade enforces this invariant in App::new)"
    );
    let (truncated_premise, _) = truncate_for_nli(&packed_text, &acc);
    match v.score(&truncated_premise, &acc) {
        Ok(scores) => {
            let passed = scores.entailment >= self.config.rag.nli_threshold;
            Some(VerificationSummary {
                nli_score: scores.entailment,
                nli_threshold: self.config.rag.nli_threshold,
                nli_passed: passed,
            })
        }
        Err(e) => {
            // model unavailable / inference error → refusal path
            tracing::warn!(target: "kebab-rag", error=%e, "NLI verifier failed");
            return self.refuse_nli_model_unavailable(query, &opts, hops, started);
        }
    }
} else {
    None
};
if let Some(v) = &verification {
    if !v.nli_passed {
        return self.refuse_nli_verification(query, &opts, hops, v.clone(), started);
    }
}
```

- `nli_threshold = 0.0` (config default) → verify skip (backwards-compat for environments without model). 명시적 *single source of truth* — `enabled` field 별도 안 둠 (§2.6 참조).
- `nli_threshold > 0.0` → verify ON. 권장 production 0.5 (multilingual NLI 의 한국어 보수). dogfood iteration 으로 tuning.
- Inference error (model download fail, ONNX runtime panic 등) → `RefusalReason::NliModelUnavailable` (fail-closed).

### 2.4 RefusalReason

`kebab_core::RefusalReason` 에 신규 2 variant + wire mapping:

| Rust variant | answer.v1 `refusal_reason` (snake) | error.v1 `code` (snake) | identical? |
|---|---|---|---|
| `NliVerificationFailed` | `"nli_verification_failed"` | `"nli_verification_failed"` | ✓ (predecessor `MultiHopDecomposeFailed` 패턴 정합 — noun + verb + state 순서) |
| `NliModelUnavailable` | `"nli_model_unavailable"` | `"nli_model_unavailable"` | ✓ |

두 wire string 모두 RefusalReason 과 error.v1.code 가 *동일* — consumer agent translation table 불요. predecessor `MultiHopDecomposeFailed` / `"multi_hop_decompose_failed"` 패턴 일관.

**구현 시 결정**:
- `RefusalReason::NliVerificationFailed` (Rust variant) → `#[serde(rename_all="snake_case")]` 가 자동으로 `"nli_verification_failed"` emit.
- `answer.schema.json` 의 `refusal_reason.anyOf[0].enum` 에 두 값 추가.
- `error.v1.code` enum 에 두 reservation 추가.
- `error.v1.details.description` 의 per-code section 추가:
  - `nli_verification_failed: { score, threshold }` (forward-looking, reserved).
  - `nli_model_unavailable: { source }` (download / inference 실패 chain).

### 2.5 Wire schema

`answer.v1` 에 `verification` optional field:

```json
{
  "schema_version": "answer.v1",
  ...
  "verification": {
    "nli_score": 0.12,
    "nli_threshold": 0.5,
    "nli_passed": false
  }
}
```

- field naming: **`nli_score`** (단일 entire-answer NLI). future v0.19+ 의 atomic claim split 도입 시 `nli_min_score` / `nli_mean_score` 추가 가능 — 그때 별 wire bump.
- `#[serde(default, skip_serializing_if = "Option::is_none")]` — additive minor. pre-v0.18 reader 무영향.
- `$defs.VerificationSummary` 인라인 정의 (기존 `$defs.HopRecord` 패턴):
  ```json
  "$defs": {
    "VerificationSummary": {
      "type": "object",
      "required": ["nli_score", "nli_threshold", "nli_passed"],
      "properties": {
        "nli_score":     { "type": "number" },
        "nli_threshold": { "type": "number" },
        "nli_passed":    { "type": "boolean" }
      }
    }
  }
  ```
  `required` array 가 3 field 모두 present-when-non-null 명시 — strict consumer 정합 (HopRecord 패턴 답습).

`refusal_reason.enum` 갱신 (`answer.schema.json` 의 `anyOf[0].enum` 에 추가):
- `"nli_verification_failed"`
- `"nli_model_unavailable"`

### 2.6 Config knobs

```toml
[models.nli]
# Production default = Xenova 의 ONNX export. 원본 PyTorch weight 는
# MoritzLaurer/mDeBERTa-v3-base-xnli-multilingual-nli-2mil7.
model = "Xenova/mDeBERTa-v3-base-xnli-multilingual-nli-2mil7"
provider = "onnx"            # only one supported in v0.18

[rag]
# 0.0 = NLI disabled (v0.18 default). > 0.0 = enable.
# 권장 production 0.5 (multilingual NLI 의 한국어 confidence 보수).
# strict 환경 0.9 (Auto-GDA / MedTrust-RAG paper 의 production threshold).
nli_threshold = 0.0
```

**default 결정 — `nli_threshold = 0.0` (disabled)**:

- backward-compat: 옛 config / 새 사용자 모두 *NLI off* 로 시작 → 280 MB first-run download 강제 없음.
- opt-in flag: 사용자가 `[rag] nli_threshold = 0.5` 설정 시 NLI active.
- single source of truth: code path `if self.config.rag.nli_threshold > 0.0 { verify } else { skip }`. `enabled` flag 별도 안 둠 — 두 gate 의 모순 회피.
- **edge case — `nli_threshold = 0.0` 의 entailment=0.0 비교**: §2.3 코드의 outer guard `if self.config.rag.nli_threshold > 0.0 { ... }` 가 disabled path 를 *short-circuit* — `>=` 비교 (`entailment >= threshold`) 는 *active 분기 (threshold > 0.0) 에서만* 도달. 즉 `entailment=0.0` + `threshold=0.0` 시나리오는 guard 가 verify 자체 skip → `>= 0.0 = true` 통과 path 절대 발생 안 함. doc reader 헷갈림 회피.

**default 결정 — `enabled` field 제거**:

round-1 review 의 D3 / A3 발견: `[models.nli].enabled` + `[rag].nli_threshold` 두 gate 모순 위험. **`enabled` field 미도입** — single gate `nli_threshold` 만:
- `nli_threshold = 0.0` → verify skip + model never loaded.
- `nli_threshold > 0.0` → verify on + model lazy-loaded on first multi-hop ask.

env override: `KEBAB_MODELS_NLI_MODEL`, `KEBAB_RAG_NLI_THRESHOLD`. legacy config 의 `#[serde(default)]` backward-compat — 옛 config.toml 그대로 parse + `nli_threshold = 0.0` (skip).

**model download 실패 fallback**:

- `nli_threshold > 0.0` + first-run model download 실패 (network / disk full / corrupt) → 모든 multi-hop ask 가 `RefusalReason::NliModelUnavailable` (fail-closed). stderr warn 명시. 사용자가 (a) `nli_threshold = 0.0` 으로 임시 disable 또는 (b) network / disk 복구 후 재시도.
- 사유: silent skip (verify 우회) 은 *S7 hallucination 재발* — 보안 측면에서 fail-closed 가 안전.

**download progress indicator**:

- first-run `score` 호출 시 hf-hub download — stderr 에 simple progress (예: `kebab-nli: downloading model.onnx (280 MB)...`).
- non-`--json` mode 만 progress emit. `--json` mode 는 quiet (wire output 의 노이즈 회피).

### 2.7 Single-pass NLI 도 적용?

학계 표준은 single-pass + multi-hop 양쪽. 그러나 single-pass 의 LlmSelfJudge 가 *gemma3:4b 환경에서* 작동 (S7 single-pass 가 grounded=false). 본 spec 의 v0.18 scope:

- **multi-hop 만 NLI 적용** — large prompt + pool union 의 hallucination risk 가 single-pass 보다 압도적.
- single-pass NLI 는 *v0.18.1 priority candidate* — §1.1 의 "LlmSelfJudge probabilistic safety" 인정 위에 *defense in depth*. config knob `[rag] nli_single_pass_enabled = false` (default) 별 PR 에서 추가.

(round-1 wording "redundant safety" → "v0.18 scope priority" 로 조정 — §1.4 의 ceiling 주장과 일관.)

## 3. PR-9 단계별 sub-PRs

### PR-9a — kebab-nli crate skeleton

**Goal**: trait surface + scaffolding + workspace dep chain 도입. implementation 없이도 build 가능.

**Files**:
- `Cargo.toml` (workspace):
  - `members` 에 `"crates/kebab-nli"` 추가.
  - `workspace.dependencies` 에 추가 (fastembed 의 transitive 와 *정확히 일치*):
    - `ort = { version = "=2.0.0-rc.9", default-features = false, features = ["ndarray"] }` (download-binaries 는 fastembed 의 transitive 활성화 의존 — features union).
    - `tokenizers = { version = "0.21", default-features = false, features = ["onig"] }`.
    - `hf-hub = { version = "0.4", default-features = false, features = ["ureq", "rustls-tls"] }` (fastembed 의 `hf-hub-native-tls` 와 cargo features union 처리 — `rustls-tls` 둘 다 활성화는 build OK).
    - `ndarray = "0.16"`.
- `crates/kebab-nli/Cargo.toml` 신규 (skeleton 만, PR-9b 가 추가):
  - `dependencies`: `kebab-config`, `anyhow`, `serde`.
  - `dev-dependencies`: `tempfile`.
- `crates/kebab-nli/src/lib.rs`:
  - `NliScores` struct + `faithfulness()` + `from_xnli_logits()`.
  - `NliVerifier` trait.
  - private `softmax3` helper.
- `crates/kebab-nli/src/onnx.rs`:
  - `OnnxNliVerifier` placeholder struct.
  - `OnnxNliVerifier::new(&Config) -> Result<Self>` placeholder.
  - `impl NliVerifier::score → bail!("PR-9a stub")`.

**Pre-flight check (PR-9a 시작 전, manual)**:

1. **Model + tokenizer file 존재 검증** — §2.1 의 `curl -I` 두 commands → `200 OK` 확인. 실패 시 PR-9 design re-evaluation.
2. **`tokenizers` features 검증** — mDeBERTa-v3 tokenizer.json 이 `Tokenizer::from_file` 로 *어떤 feature set* 필요한지 standalone repro 로 확인:
   ```sh
   cargo new --bin /tmp/nli-tok-probe
   cd /tmp/nli-tok-probe
   cargo add tokenizers --no-default-features -F onig
   wget https://huggingface.co/Xenova/mDeBERTa-v3-base-xnli-multilingual-nli-2mil7/resolve/main/tokenizer.json
   # main.rs: tokenizers::Tokenizer::from_file("tokenizer.json").expect("load");
   cargo run --release
   ```
   성공 시 PR-9a 의 `tokenizers = { ..., features = ["onig"] }` lock. 실패 시 *진단*:
   - `unstable_wasm` 또는 다른 feature 가 SentencePiece 모듈 활성화에 필요한지 확인 (tokenizers 0.21 docs 참조).
   - `default-features = true` 가 가장 안전 path — features 결정에 confidence 부족 시.

**Cargo features 의 결정 trace**: 본 pre-flight 결과는 PR-9a 의 PR description 의 `## Cargo features 결정 trace` 절에 첨부 (`cargo run` 출력 + 최종 features set). spec lock value.

**Tests** (6 unit):
- `softmax3_normalises_to_unit` — sum = 1, monotonic.
- `softmax3_is_invariant_to_constant_shift` — log-sum-exp 안전성.
- `nli_scores_from_xnli_logits_orders_correctly` — high entailment → entailment 최대.
- `faithfulness_returns_entailment_channel`.
- `new_succeeds_on_default_config`.
- `score_returns_err_in_skeleton` — stub 의 명시적 err 메시지.

**검증**:
- `cargo test -p kebab-nli -j 1` — 6 통과.
- `cargo clippy -p kebab-nli --all-targets -j 1 -- -D warnings` clean.

**Wire 영향**: 없음 (crate 만 도입).

**Risks**:
- workspace 의 ort / tokenizers / hf-hub 추가 → 전체 build 재 link (큰 변화 없음, fastembed 가 이미 transitive).
- features union 위험 — `download-binaries` (fastembed) + `ndarray` (kebab-nli) 동시 활성화는 build OK 검증 필수.

**시간**: 2-3h.

### PR-9b — OnnxNliVerifier 의 ONNX inference + model download

**Goal**: `OnnxNliVerifier::score` 의 진짜 implementation. model + tokenizer download / cache / inference 완성.

**Dependency**: PR-9a 머지 완료.

**Files**:
- `crates/kebab-nli/Cargo.toml`:
  - `ort`, `tokenizers`, `hf-hub`, `ndarray`, `tracing` 추가 (workspace.dependencies 에서).
- `crates/kebab-nli/src/onnx.rs`:
  - `OnnxNliVerifier` 의 fields:
    - `model_id: String`.
    - `cache_dir: PathBuf` (`config.storage.model_dir.join("nli").join(sanitize(model_id))`).
    - `session: OnceLock<ort::Session>`.
    - `tokenizer: OnceLock<tokenizers::Tokenizer>`.
  - `OnnxNliVerifier::new(&Config) -> Result<Self>`:
    - `model_id`, `cache_dir` stamp. actual session/tokenizer load *deferred*.
  - `ensure_loaded(&self) -> Result<(&Session, &Tokenizer)>`:
    - hf-hub download (cache hit 시 skip + warn 에서 hit/miss 명시).
    - tokenizer.json 로드 → `Tokenizer::from_file`.
    - model.onnx 로드 → `Session::builder().commit_from_file`.
    - truncation params 설정 (§2.2.3).
    - 두 OnceLock 에 store.
  - `score(premise, hypothesis)`:
    - `ensure_loaded()` 호출.
    - `tokenizer.encode((premise, hypothesis), add_special_tokens=true)`.
    - input_ids + attention_mask ndarray `[1, seq_len]` i64.
    - `session.run(ort::inputs![...])`.
    - `outputs["logits"].try_extract_tensor::<f32>()` → shape `[1, 3]`.
    - `NliScores::from_xnli_logits([l0, l1, l2])`.
  - `sanitize_model_id(s: &str) -> String` helper — `/` → `_`.
- `crates/kebab-nli/tests/inference.rs` 신규:
  - `#[ignore]` integration test — real model download + 5 forward pass cases:
    1. `premise = "Caffeine is a stimulant.", hypothesis = "Caffeine is a stimulant."` → entailment 매우 높음 (>0.8).
    2. `premise = "Caffeine is a stimulant.", hypothesis = "The chemical formula of caffeine is C8H10N4O2."` → entailment 낮음 (<0.3) — neutral/contradiction.
    3. Korean: `premise = "사과는 빨갛다.", hypothesis = "사과는 색이 있다."` → entailment 높음.
    4. Long premise (10000 char) → truncation 적용 후 정상 score (panic 없음).
    5. Empty hypothesis → graceful error (panic 없음, err 반환).

**Manual smoke protocol (PR-9b PR description 강제)**:

PR description 의 `## 검증` 절에 다음 *manual run* 결과 첨부:
```sh
cargo test -p kebab-nli -j 1 --test inference -- --ignored 2>&1 | tail -20
```
- 5 test 모두 PASS 확인.
- 첫 case (entailment 높음) 의 NliScores dump (예: `entailment=0.92, neutral=0.05, contradiction=0.03`).

CI 부담 회피 위해 unit test (no `--ignored`) 만 CI 실행. ignored test 는 PR 작업자 manual.

**검증**:
- unit test 통과 + clippy clean.
- `--ignored` integration test 의 manual run (PR 작업자 책임, PR body 첨부).

**Wire 영향**: 없음 (crate-internal).

**Risks**:
- `ort` 2.0-rc.9 의 API stability — rc 라 minor 사이 incompat 가능. *=mitigation*: workspace `ort = "=2.0.0-rc.9"` pin (fastembed 와 정확히 일치).
- mDeBERTa-v3 의 ONNX export 가 Xenova HF Hub 에 존재 — §2.1 의 pre-flight check 가 PR-9a 시작 전 검증. 없으면 PR-9 design re-evaluation (다른 ONNX repo 또는 Optimum self-export).
- `tokenizers` 0.21 의 SentencePiece 지원 — fastembed 가 BERT tokenizer 사용 (multilingual-e5-small), kebab-nli 가 mDeBERTa SentencePiece 사용 (다른 patterns). 첫 통합 위험.
- `hf-hub` 0.4 의 `ureq + rustls-tls` features 가 workspace 의 다른 deps 와 incompat 없는지 — fastembed 의 `hf-hub-native-tls` 와 cargo features union 시 build OK 가정 (rustls-tls + native-tls 동시 활성화는 hf-hub crate features 가 mutually compatible 검증 필요).

**시간**: **8-12h** (round-1 planner 의 6-8h underestimated 지적 반영). 첫 시도 실패 시 fallback (Optimum self-export) 까지 포함.

### PR-9c — Pipeline integration (split: 9c-1 core + 9c-2 pipeline)

**Goal**: kebab-rag pipeline 의 `ask_multi_hop` 에 NLI verify 통합. core types + wire + config 추가.

**Dependency**: PR-9b 머지 완료.

**round-1 review (P1 / M2) 분할 권장 반영** — 9c 를 **별 PR 2개로 분할** (9c-1 → 9c-2 sequential 머지). review 부담 분산 + git bisect 시 surface (wire/types) vs behavior (pipeline integration) 분리. 한 PR 의 commit 2개 방식보다 *별 PR* 채택 — round-1 P1 의 목적 (review 부담 ↓) 정합.

#### PR-9c-1 — Core types + wire scaffolding (breaking surface)

**Files**:
- `crates/kebab-core/src/answer.rs`:
  - `RefusalReason::NliVerificationFailed` + `RefusalReason::NliModelUnavailable` 신규.
  - `Answer.verification: Option<VerificationSummary>` field.
  - `VerificationSummary { nli_score: f32, nli_threshold: f32, nli_passed: bool }` 신규 struct.
- `crates/kebab-config/src/lib.rs`:
  - `NliCfg` 신규 struct + `[models.nli]`:
    - `model: String` (default `"Xenova/mDeBERTa-v3-base-xnli-multilingual-nli-2mil7"`).
    - `provider: String` (default `"onnx"`).
  - `RagCfg.nli_threshold: f32` (default `0.0`).
  - env override + legacy parse 단위 test.
- `crates/kebab-rag/src/pipeline.rs`:
  - `RagPipeline` 의 새 field: `verifier: Option<Arc<dyn NliVerifier>>` (None = verify off).
  - **시그니처 widening 결정 = Option B (builder pattern)**:
    - 기존 `RagPipeline::new(config, retriever, llm, sqlite)` 시그니처 *유지* (backward-compat for 18+ existing call sites).
    - 신규 `pub fn with_verifier(self, v: Arc<dyn NliVerifier>) -> Self` builder.
    - `kebab-app` facade 만 `with_verifier` 호출. 다른 caller (cli/tui/mcp tests) 무영향.
    - Cargo.toml: `kebab-rag` 가 `kebab-nli` 의존 추가.
- `docs/wire-schema/v1/answer.schema.json`:
  - `verification` field 추가 (anyOf [object, null]) + `$defs.VerificationSummary` 인라인.
  - `refusal_reason.enum` 에 `"nli_verification_failed"`, `"nli_model_unavailable"` 추가.
- `docs/wire-schema/v1/error.schema.json`:
  - `code` enum 에 `nli_verification_failed`, `nli_model_unavailable` 추가.
  - `details.description` 에 두 항목 추가 (`multi_hop_decompose_failed: {}` 패턴 그대로).

**Tests**:
- `crates/kebab-config/src/lib.rs::tests`:
  - `default_nli_threshold_is_zero`.
  - `default_nli_model_is_xenova_mdeberta`.
  - `legacy_config_without_nli_uses_defaults`.
  - `env_override_nli_threshold`.
- `crates/kebab-cli/tests/wire_ask_multi_hop.rs`:
  - `answer_schema_declares_verification_field_and_defs`.
  - `answer_schema_refusal_reason_enum_includes_nli_verification_failed` (+ `nli_model_unavailable`).
  - `error_schema_code_enum_includes_nli_verification_failed` (+ `nli_model_unavailable`).

**검증**:
- `cargo test --workspace -j 1` — 회귀 0 (기존 multi-hop tests pass, RagPipeline::new 시그니처 unchanged).
- `cargo clippy --workspace --all-targets -j 1 -- -D warnings` clean.

**시간**: 2-3h.

#### PR-9c-2 — Pipeline integration + mock test

**Dependency**: PR-9c-1 머지 완료 (core types: `RefusalReason::Nli*` variants + `Answer.verification` field + `RagPipeline.verifier` field + `kebab-nli` 의 trait + config knobs 가 9c-1 에서 도입). 9c-2 가 그 위에 *behavior* 통합.

**Files**:
- `crates/kebab-rag/src/pipeline.rs`:
  - `ask_multi_hop` 의 step 8.5 NLI hook (§2.3 코드).
  - `refuse_nli_verification` helper (`refuse_*` 패턴) — `verification: Some(...)` 채움.
  - `refuse_nli_model_unavailable` helper — `verification: None`.
  - `pub fn truncate_for_nli(premise: &str, hypothesis: &str) -> (String, bool)` helper (§2.2.3 packed_text pre-budget). signature: 첫 return = truncated premise (max char count = `MAX_NLI_PREMISE_CHARS = 4 * 400` ≈ 1600 chars, hypothesis 길이 빼고 special tokens 32 char budget 적용 후 자연 보존). 둘째 return = was_truncated boolean (caller 가 tracing log 또는 wire 의 `verification` extension 에서 사용 가능 — v0.18 wire 추가 안 함, future v0.19+ candidate).
  - **`MAX_NLI_PREMISE_CHARS` 의 token ratio 가정**: 4 char ≈ 1 token (영어 BPE 기준, mDeBERTa-v3 의 default). 한국어 SentencePiece 는 1-2 char/token (한 음절 = 1 token 통상) — 1600 chars 한국어 = 800-1600 tokens, max_seq_len 512 초과 가능. 이때 tokenizer 의 `OnlyFirst` truncation 가 backup 으로 작동 (premise 끝부터 잘림, hypothesis 보전). dogfood retest 의 S10 (KR) NLI score 측정 후 가능하면 *token-count 기반 budget* 으로 v0.18.1 갱신 — char-based budget 의 EN-biased 보정.
- `crates/kebab-app`:
  - `App::new` 또는 `pipeline_from_config` 가 NliVerifier 생성:
    - `config.rag.nli_threshold > 0.0` → `OnnxNliVerifier::new(config)` 호출 + `Arc::new` wrap.
    - `config.rag.nli_threshold == 0.0` → verifier = None.
  - **facade invariant 결정 — `Result<App, anyhow::Error>` (construction-time error)**: `App::new` 가 `Result<Self, anyhow::Error>` 반환. `config.rag.nli_threshold > 0.0` + `OnnxNliVerifier::new` 실패 시 `bail!()` — user-facing crash 회피. `RagPipeline.verifier == None` + `config.rag.nli_threshold > 0.0` 의 *unreachable* 조합은 `expect("App::new enforces invariant")` safety net 만 — 정상 path 도달 불가능. round-2 critic NEW-M2 closure.
- `crates/kebab-rag/tests/multi_hop.rs`:
  - `common/mod.rs` 에 `MockNliVerifier { scores: NliScores }` helper.
  - `multi_hop_nli_pass_keeps_grounded` — entailment 0.9 → grounded=true, verification.nli_passed=true.
  - `multi_hop_nli_fail_refuses` — entailment 0.1 → refusal=NliVerificationFailed.
  - `multi_hop_nli_disabled_skip_verify` — threshold = 0.0 → verify skip, verification=None.
  - `multi_hop_nli_model_unavailable_refuses` — verifier Err → refusal=NliModelUnavailable.
  - `multi_hop_truncate_for_nli_preserves_hypothesis` — long premise + 짧은 hypothesis → hypothesis 그대로.
- `integrations/claude-code/kebab/SKILL.md`:
  - `mcp__kebab__ask` 절에 NLI 안내 한 줄 — `answer.v1.verification.nli_passed` 의미 + threshold tuning 가이드 + `nli_verification_failed` / `nli_model_unavailable` refusal 처리.

**Tests**: 5 신규 multi-hop tests (위 list) + 기존 tests 회귀 0.

**검증**:
- `cargo test --workspace -j 1` — 모든 test 통과 + 신규 5 multi-hop pass.
- `cargo clippy --workspace --all-targets -j 1 -- -D warnings` clean.

**Wire 영향**: PR-9c-1 의 wire schema 변경에 *behavior wiring* — `verification` field 가 multi-hop ask 의 happy path / refuse path 양쪽에서 채움.

**시간**: 3-4h.

**Total PR-9c (1+2)**: 5-7h (round-1 4-6h underestimated 반영 → 5-7h).

### PR-9d — Dogfood retest + HOTFIXES closure

**Goal**: PR-9c 머지 후 같은 dogfood corpus 에서 S7 + S1 + S3 + S10 retest. PR-9 의 진짜 작동 확인.

**Dependency**: PR-9c 머지 완료.

**Scope**: 본 *PR* 가 아닌 *별 commit* 로 가능성 ↑:
- repo 변경 = `tasks/HOTFIXES.md` 의 "PR-9 closure" sub-section 추가 + (선택) `docs/dogfood/v0.18.0/` 의 dogfood result snapshot.
- `/build/cache/dogfood-v018/results/post-pr9/` 는 repo 외 (gitignore 처럼).
- **결정**: PR (gitea-pr) 또는 main 직접 commit 둘 다 가능. 작업자 선택 — review 부담 ↓ 우선이면 commit, audit trail 우선이면 PR. *본 spec 의 default = PR* (다른 PR 패턴과 일관).

**Files**:
- `tasks/HOTFIXES.md`:
  - "PR-9 closure (post-v0.18 dogfood retest)" sub-section 추가 — pre/post 결과 비교 표.
- `docs/dogfood/v0.18.0/` (신규 디렉토리):
  - `SUMMARY.md` — sanitized dogfood 보고서 (원본 `/build/cache/dogfood-v018/results/SUMMARY.md` 의 repo 포함 가능 부분).
  - `s7-multihop-post-pr9.json` — S7 multi-hop NLI 결과 sample (refuse + nli_score).
  - `s1-multihop-post-pr9.json` — S1 multi-hop NLI 결과 sample (grounded + nli_score).
- `/build/cache/dogfood-v018/results/post-pr9/` (작업 디렉토리, repo 외):
  - 시나리오별 JSON dump + findings.md.

**Tests**: 자동화 없음. 사용자 환경 (release binary + Ollama gemma3:4b + NLI model first-run) 에서 manual run:

- `[rag] nli_threshold = 0.5` config (production 권장값).
- S7 / S1 / S3 / S10 query → 각각 NLI score 측정 + grounded/refuse 확인.
- **RAM peak 측정 protocol** (round-2 critic gap 반영) — 시작 전 `ps -o rss=,vsz=,comm= -p $(pgrep -f 'ollama|kebab')` baseline. multi-hop ask 진행 중 1초 간격 sampling (5분 cap) — `while sleep 1; do ps ... ; done > /tmp/ram-S<N>.log`. peak RSS = `awk '{sum+=$1} END {print max}'` (Ollama + kebab + NLI model 합산). 16 GB 환경 OOM 없는지 + peak < 10 GB 확인. release notes 의 권장 RAM (peak + 4 GB headroom) 한 줄 명시.

**Pre-run prereq (manual + subagent 양쪽 적용)**: PR-9d 시작 전 환경 검증 — manual run 작업자 또는 subagent dispatch 모두 동일 prereq:

- Ollama service running (`curl -s 127.0.0.1:11434/api/tags`).
- dogfood corpus 디렉토리 존재 (`/build/cache/dogfood-v018/queries/*.txt`).
- network reachable (hf-hub 의 280 MB NLI model first-run download 가능).
- free RAM ≥ 6 GB (peak headroom).
- release binary path: `/build/out/cargo-target/release/kebab` (CARGO_TARGET_DIR 활용 environment) 또는 `./target/release/kebab` (default in-tree).

prereq 실패 시 subagent 가 *조기 abort* + 사용자 보고. *partial* dogfood 결과 commit 회피.

**Expected (PASS criteria)**: §7 verification plan 의 acceptance criteria 표 단일 source of truth. 본 절에서는 *워크플로우 설명* 만 — measurement value 와 threshold 결정은 §7 표에서. duplication 회피 (round-4 R4-NEW-M1 + R4-NEW-N1 closure).

dogfood iteration 결과에 따른 default 조정 trigger:
- S1 의 entailment 가 0.6 미만이면 *legitimate answer 가 reject* 의 false positive — threshold 조정 (`nli_threshold = 0.3` 등) 또는 NLI model 교체 (xlm-roberta-large) 검토.
- S3/S10 의 acceptable degraded outcome 이 50% 이상이면 multilingual NLI 의 한국어 confidence 약함 — model 교체 또는 token-count budget 갱신 (R3-NEW-N1 의 v0.18.1 candidate).

**Wire 영향**: 없음 (docs only).

**시간**: 4-6h (round-1 P3 PR vs commit 결정 + RAM 측정 + dogfood corpus 보존 추가).

## 4. 이미 머지된 PR-1 ~ PR-8 의 결과

| PR | 변경 | 상태 |
|---|---|---|
| #166 PR-1 | multi-hop eval golden set | ✅ |
| #167 PR-2 | `ask_multi_hop` skeleton (fixed depth=2) | ✅ |
| #168 PR-3a | HopRecord wire + RagCfg knobs | ✅ |
| #169 PR-3b-i | dynamic decide loop + helpers | ✅ |
| #170 PR-3b-ii | ScriptedLm + 7 multi-hop tests + refusal hop trace | ✅ |
| #171 PR-4 | CLI `--multi-hop` flag + wire schema | ✅ |
| #172 PR-5 | MCP `multi_hop: bool` arg + SKILL.md | ✅ |
| #173 PR-6 | TUI F2 toggle + badge + hops summary | ✅ |
| #174 PR-7 | pre-decompose probe gate (S7 1차 fix) | ✅ |
| #175 PR-8 | synthesize prompt rule + pool 30→15 (S7 2차 partial mitigation) | ✅ |

frozen design contract (`2026-05-25-p9-fb-41-multi-hop-rag-design.md`) 의 PR-3 분할 (3a/3b-i/3b-ii) + PR-7 / PR-8 추가는 *post-merge deviation*. HOTFIXES 에 기록 (이미 dated entries 존재).

## 5. v0.18.0 cut PR (PR-9d 머지 후, 별 PR `chore: cut v0.18.0`)

**바람직한 patterns** (round-1 M7 / D2 / M6 모두 반영):
- `v0.18.0` bump + tag = **같은 commit** (CLAUDE.md "Release / binary version bump" rule).
- frozen design §3.8 갱신은 *본 cut PR 안* 에서 (PR-9c 가 design contract 변경 안 함, 머지 후 한꺼번에).
- `gitea-release` tag 는 본 PR 머지 commit 위에 즉시.

**한 commit 내용 (또는 짧은 PR scope)**:
1. **Workspace `Cargo.toml` version** 0.17.2 → 0.18.0 (minor bump).
   - surface 확장: CLI `--multi-hop`, MCP `multi_hop`, TUI F2, answer.v1 `hops` + `verification`.
   - prompt_template_version: `rag-multi-hop-v1` (PR-2 이후, 변경 없음).
   - safety fix: PR-7 + PR-8 + PR-9.
   - `Cargo.lock` 자동 cascade.
2. **HANDOFF.md**:
   - 한 줄 요약 (P0~P9 + P10 + v0.18.0 fb-41 multi-hop ship).
   - 머지 후 결정 절에 fb-41 entry 단락 (PR-1~PR-9 + dogfood + NLI 한 문단).
3. **HOTFIXES.md**:
   - PR-9 closure sub-section anchor 정리 (`post-v0.18`).
   - 기존 fb-41 entry 들 `post-v0.18` anchor.
4. **INDEX.md**:
   - fb-41 status `open` → `completed`.
   - v0.18.0 release subheader (fb-41 multi-hop + NLI verification).
5. **frozen design** (`docs/superpowers/specs/2026-04-27-kebab-final-form-design.md`):
   - §3.8 RAG 의 multi-hop sub-section 추가 — 본 finalize spec 의 §1-§3 요약을 verbatim 형식으로 inline.
   - §9 versioning cascade 표에 (선택) `nli_model_version` row — *config knob 변경만 cascade 영향, embedding 처럼 chunks 재-index 불요* 명시.
6. **integrations/claude-code/kebab/SKILL.md**:
   - PR-9c-2 에서 *비활성* 상태 NLI 안내 추가됨. cut PR 에서 v0.18.0 release notes link 한 줄.
7. **README**:
   - `kebab ask --multi-hop` + NLI 옵션 안내 한 단락 (model first-run download cost, RAM 권장).
   - binary path confusion (round-1 N1 / dogfood SUMMARY §부수 발견) 한 줄 — `CARGO_TARGET_DIR` 활용 시 `/build/out/cargo-target/release/kebab` 명시.
8. **`docs/SMOKE.md`**:
   - NLI 옵션 활성화 절차 ([rag] nli_threshold = 0.5).
   - first-run model download 안내 (~280 MB to `{data_dir}/models/nli/`).
   - RAM 권장 (NLI active + Ollama **gemma3:4b** (권장 모델) 동시 — peak RSS ~5-6 GB; 16 GB 머신에서 OK). **8B+ Q4 모델** (gemma4:e4b 8B / gemma2:9b 등) 사용 시 *추정* peak ~10 GB — 16 GB 환경 경계, OOM risk 별 안내 한 줄.

**같은 commit 의 PR title + tag**:
- Commit msg: `chore: bump version 0.17.2 → 0.18.0 + cut fb-41 multi-hop`.
- gitea-release: `v0.18.0` tag *본 commit* 위.
- Release notes (자동 `--auto-notes`):
  ```
  # v0.18.0 — fb-41 multi-hop RAG ship + NLI verification
  
  ## 새 surface
  - CLI: `kebab ask --multi-hop <query>` — multi-hop reasoning.
  - MCP: `ask` tool `multi_hop: true` argument.
  - TUI: Ask 패널 F2 toggle + multi-hop badge + hops summary.
  
  ## 새 wire
  - `answer.v1.hops` — multi-hop per-iter trace (decompose/decide/synthesize).
  - `answer.v1.verification` — NLI groundedness score (`nli_threshold > 0.0` 일 때).
  - `error.v1.code` enum 확장: `multi_hop_decompose_failed`, `nli_verification_failed`, `nli_model_unavailable`.
  
  ## 새 config
  - `[rag] multi_hop_max_depth` (default 3), `multi_hop_max_sub_queries_per_iter` (5), `multi_hop_max_pool_chunks` (15).
  - `[rag] nli_threshold` (default 0.0 — disabled; 권장 production 0.5).
  - `[models.nli] model` (default `Xenova/mDeBERTa-v3-base-xnli-multilingual-nli-2mil7`).
  
  ## 새 RefusalReason
  - `multi_hop_decompose_failed`, `nli_verification_failed`, `nli_model_unavailable`.
  
  ## 권장 환경
  - LLM: gemma3:4b (CPU only, 16 GB RAM 권장).
  - NLI 활성화 시: ~280 MB first-run download to `{data_dir}/models/nli/`.
  - RAM peak (NLI active + Ollama 동시, **gemma3:4b 기준**): ~5-6 GB (16 GB 환경 OK). 8B+ Q4 모델 (gemma4:e4b 8B / gemma2:9b 등) 은 *추정* peak ~10 GB — 16 GB 경계.
  
  ## Known limitations
  - single-pass NLI 미적용 (v0.18.1 priority).
  - atomic claim split 미적용 (entire answer = 1 NLI call).
  - GPU acceleration 미지원 (CPU ONNX runtime).
  
  ## 도그푸딩
  - dogfood corpus snapshot: `docs/dogfood/v0.18.0/`.
  - HOTFIXES dated entries 의 PR-9 closure 절 참조.
  ```

## 6. 한계 / 미해결 (v0.18.1+ 또는 P+)

- **NLI single-pass 적용** — v0.18 scope 외. `[rag] nli_single_pass_enabled = false` (default) 별 PR.
- **NLI threshold tuning** — production 표준 0.9, kebab 권장 enable 값 0.5 (config default 는 0.0 disabled, §2.6 참조; multilingual NLI 의 한국어 confidence 보수). PR-9d dogfood 후 적정값 결정 — *measured value* 가 default 갱신 또는 doc 권장값 갱신.
- **Atomic claim split** — 현재 entire answer 1 claim. LLM-based claim extraction (별 LLM call) 은 v0.19+. wire field `nli_score` 가 *single* 인 이유.
- **NLI false negative** — strong paraphrase → reject. dogfood 측정 후 threshold 조정 또는 model 교체 (xlm-roberta-large 1.5 GB).
- **GPU acceleration** — ort 의 CUDA execution provider 가능. v0.19+ 사용자 환경 의존.
- **release binary path confusion** — `target/release/kebab` (in-tree) vs `/build/cache/...` (CARGO_TARGET_DIR). v0.18.0 cut PR 의 README 한 줄 (§5 의 step 7 포함) — *deferred 아닌 closure*.
- **Future LLM 의 ceiling 측정** — gemma4:e4b / qwen2.5:7b / larger 의 prompt-following 측정. NLI vs LLM-upgrade 의 ROI 재평가. v0.19+ dogfood agenda.
- **NLI model 양자화 (Q8 INT8)** — 280 MB FP32 → ~70 MB INT8 (`Xenova/.../onnx/model_quantized.onnx`). accuracy 미세 저하. v0.19+ config knob `[models.nli] quantization = "fp32" | "q8"`.

## 7. 검증 plan (PR-9d acceptance criteria)

각 sub-PR 가 자체 회귀 핀. PR-9d 의 dogfood retest 가 *integration-level* 검증.

**측정 환경 (전체 표 공통)**: `[rag] nli_threshold = 0.5` (production 권장값). *NLI score 자체* 가 expected range 안인지가 PASS — `nli_passed` boolean 은 threshold 함수라 redundant. dogfood 작업자가 다른 threshold 로 측정 시 (예: 0.3) 결과 해석 다를 수 있어 spec 가 *threshold lock*.

| 시나리오 | path | primary expected | acceptable degraded | nli_score range | latency expected |
|---|---|---|---|---|---|
| S7 (caffeine, KB outside, EN) | multi-hop NLI | grounded=false, refusal=`nli_verification_failed` | (없음 — 반드시 NLI refuse) | **< 0.3** | 158s + NLI ~50ms (PR-7 probe gate 가 RRF top_score 0.5 > 0.30 통과시키므로 multi-hop pipeline 전체 진행 후 step 8.5 NLI refuse) |
| S1 (compiler compound, KR) | multi-hop NLI | grounded=true, refusal=None | (없음 — 반드시 grounded) | **≥ 0.6** | 158-200s + NLI |
| S3 (retrieval stack, **EN**) | multi-hop NLI | grounded=true, refusal=None | grounded=false + LlmSelfJudge (paraphrase 강한 EN→KR sub-queries 의 entailment 약함) — *citation marker 누락 잔존 issue, NLI 자체는 통과* | **≥ 0.5** | 같은 range |
| S10 (dinosaur, KB outside, KR) | multi-hop NLI | grounded=false, refusal=`nli_verification_failed` | grounded=false + LlmSelfJudge (NLI 의 한국어 confidence 낮으면 LLM self-judge 가 reject path) | **< 0.4** | 590s |
| S7 single-pass | single-pass (NLI 미적용) | grounded=false, LlmSelfJudge | (없음) | n/a (verification field 없음) | 30s |

**Primary vs degraded acceptable** (round-2 critic P-M5 closure):
- S7: NLI refuse 가 본 PR-9 의 *core 검증* — degraded outcome 허용 안 함. NLI 가 안 refuse 면 *PR-9 가 작동 안 함*.
- S1: legitimate compound query — NLI 가 reject 시 *false positive*. degraded outcome 허용 안 함.
- S3 / S10: NLI 의 한국어 confidence / paraphrase 강도가 multilingual NLI 의 known weakness. primary 우선 기대지만 degraded LlmSelfJudge 도 안전한 fail-closed path 라 acceptable. *그러나 degraded 가 50% 이상* 시 NLI 효과 약함 — threshold 조정 또는 model 교체 (xlm-roberta-large) 검토.

PR-9d 의 PASS: S7 + S1 primary expectation 모두 충족 + S3/S10 의 primary 또는 acceptable degraded. range 밖 시 threshold 또는 model 재검토.

**RAM peak 측정** (protocol 은 §3 PR-9d 참조):
- Ollama RSS + kebab-cli RSS + NLI model RSS = peak 약 ~5-6 GB.
- 16 GB 환경에서 OOM 없는지 확인. release notes 의 권장 RAM 명시.

## 8. self-review notes

- **PR-9 의 ONNX integration** 가 *새 dep chain* (ort + tokenizers + hf-hub) 도입 — 첫 사용 안정화 필요. PR-9b 의 `#[ignore]` test 의 manual smoke protocol (PR description 강제 첨부) 이 *production binary 의 실제 동작 검증* path.
- **multi-hop NLI 의 latency 추가** — current multi-hop synthesize 158s + NLI ~50ms ≈ 158s. negligible.
- **Model first-run download (~280 MB)** — 사용자 도그푸딩 환경 (CPU only) 의 disk + download bandwidth 1회 비용. README + SMOKE 안내. fail-closed download failure 정책.
- **`RagPipeline::new` 시그니처 widening — Option B (builder) 결정** (round-1 A2 반영). 기존 시그니처 유지 + `with_verifier` builder. 18+ existing call sites 무영향.
- **frozen design contract §3.8 갱신 timing — v0.18.0 cut PR 안** (round-1 M6 / D2 반영). PR-9c 가 contract 변경 안 함 — implementation 만. cut PR 에서 contract + implementation 결과 동시 갱신.
- **kebab-nli 의 trait + impl 동일 crate** (round-1 A4 deferred 결정 명시) — v0.18 scope = adapter 1개. v0.19+ 에 multi-adapter 등장 시 `kebab-nli-onnx` 분리 (그 시점에 internal API breaking, wire 무관).
- **single-pass NLI deferred wording** "v0.18 scope priority — multi-hop hallucination risk 가 single-pass 보다 큼" 으로 round-1 wording 조정 (M9 반영).
- **alternative root cause 검토** (M1 반영) — §1.2 의 4 alternatives 비교 표. NLI 채택 ROI justification 강화.
- **PR-9c 분할** (M2 / P1 반영) — 9c-1 (core types) + 9c-2 (pipeline integration).
- **PR-9d PR vs commit** (P3 반영) — PR default, 작업자 선택 가능.
- **dogfood corpus 보존** (P5 반영) — `docs/dogfood/v0.18.0/` 신규 dir + sanitized SUMMARY + sample JSON.
- **RAM cold-start 측정** (P6 반영) — PR-9d 의 PASS criteria 에 포함.
- **ort version pin** (P7 반영) — `workspace.dependencies.ort = "=2.0.0-rc.9"` (fastembed transitive 와 정확히 일치).
- **integrations/claude-code/kebab/SKILL.md NLI 안내** (D6 반영) — PR-9c-2 에서 추가.

## 9. round-1 review 의 issue closure 매트릭스

| reviewer | issue | resolution |
|---|---|---|
| architect | A1 model ID 불일치 | §2.1 — Xenova/... config default + MoritzLaurer/... 원본 출처 명시 |
| architect | A2 widening path 미결정 | §8 / §3 PR-9c-1 — Option B (builder) 결정 |
| architect | A3 config default 모순 | §2.6 — `enabled` field 제거 + `nli_threshold` single gate |
| architect | A4 crate split | §2.2 + §8 — v0.18 단일 crate, future split trigger 명시 |
| architect | A5 ort version + feature | §3 PR-9a + §8 — `ort = "=2.0.0-rc.9"` pin, fastembed transitive 와 정확히 일치 |
| architect | A6 cache_dir → model_dir | §2.2.2 — `config.storage.model_dir.join("nli")` |
| architect | A7 §2.3 single-pass 주석 | §2.3 — 주석에서 single-pass 제거 + §2.7 cross-ref |
| critic | C1 truncation strategy | §2.2.3 — `OnlyFirst` + `truncate_for_nli` helper + 회귀 핀 |
| critic | M1 alternative root cause | §1.2 — 4 alternatives 비교 표 |
| critic | M2 PR-9c scope 과대 | §3 — 9c-1 + 9c-2 분할 |
| critic | M3 9b smoke protocol | §3 PR-9b — manual smoke + PR description 강제 첨부 |
| critic | M4 threshold default 모순 | §2.6 + §7 — default 0.0 (disabled), production 권장 0.5, dogfood 측정값 별 명시 |
| critic | M5 S1 acceptance criteria | §7 — measured value range 표 |
| critic | M6 frozen design timing | §5 + §8 — cut PR 안에 통합 |
| critic | M7 bump same-commit | §5 — 같은 commit 명시 + tag |
| critic | M8 download fallback | §2.6 — fail-closed + NliModelUnavailable + warn |
| critic | M9 single-pass deferred wording | §2.7 + §8 — wording 조정 |
| critic | N1 binary path | §5 step 7 — README 한 줄 |
| critic | N2 threshold 0.0 edge | §2.6 — doc comment 명시 |
| critic | N3 wire naming | §2.4 — `nli_verification_failed` + `nli_model_unavailable` (snake 통일) |
| planner | P1 9c scope | M2 와 같음 — 분할 |
| planner | P2 9b 시간 | §3 PR-9b — 8-12h 갱신 |
| planner | P3 9d PR vs commit | §3 PR-9d — PR default, 작업자 선택 |
| planner | P4 model pre-flight | §2.1 + §3 PR-9a — pre-flight curl check |
| planner | P5 dogfood 보존 | §3 PR-9d + §5 step 5 — `docs/dogfood/v0.18.0/` 신규 |
| planner | P6 RAM cold-start | §7 — PR-9d acceptance criteria + release notes |
| planner | P7 ort pin | §3 PR-9a — `"=2.0.0-rc.9"` |
| planner | P8 frozen design timing | M6 와 같음 — cut PR 안 |
| document | D1 schema refusal_reason.enum | §3 PR-9c-1 — `nli_verification_failed` + `nli_model_unavailable` |
| document | D2 frozen design timing | M6 와 같음 |
| document | D3 enabled/threshold | A3 와 같음 — `enabled` 제거 |
| document | D4 error.v1 description | §2.4 — per-code description 갱신 |
| document | D5 Xenova vs MoritzLaurer | A1 와 같음 — §2.1 명시 |
| document | D6 SKILL.md | §3 PR-9c-2 — multi-hop ask 절에 NLI 안내 |

### Round-2 issues (post-spec-v2 review)

| reviewer | round-2 issue | round-3 resolution |
|---|---|---|
| document | ISSUE-1 RefusalReason rename | §2.4 — `FailedNliVerification` → `NliVerificationFailed`. wire `"nli_verification_failed"` 가 RefusalReason + error.v1.code 양쪽 동일. mapping 표 §2.4 inline. |
| document | NIT-2 VerificationSummary required | §2.5 — `$defs.VerificationSummary` 의 `required: ["nli_score", "nli_threshold", "nli_passed"]` 명시. HopRecord 패턴. |
| critic | P-M4 threshold context | §7 — 측정 환경 명시 (`nli_threshold = 0.5` lock). |
| critic | P-M5 S3/S10 multi-outcome | §7 — primary expected + acceptable degraded 컬럼 + 50% 이상 degraded 시 model 재검토 명시. |
| critic | P-N2 entailment=0.0 edge | §2.6 — outer guard `> 0.0` 가 disabled path short-circuit + `>=` 비교는 active 분기 도달. doc comment 명시. |
| critic | P-N3 wire naming | §2.4 — RefusalReason wire 도 `nli_verification_failed` 통일. mapping 표 명시. document ISSUE-1 와 같음. |
| critic | NEW-M1 tokenizers features | §3 PR-9a — pre-flight 의 standalone repro (`cargo new --bin nli-tok-probe ...`). Cargo features 결정 trace 를 PR description 의 별 절에 첨부. |
| critic | NEW-M2 facade panic vs error | §3 PR-9c-2 — `App::new` 가 `Result<App, anyhow::Error>` 반환. `OnnxNliVerifier::new` 실패 시 `bail!`. unreachable safety net 만 `expect()`. |
| critic | NEW-N1 truncate_for_nli signature | §3 PR-9c-2 — `pub fn truncate_for_nli(premise: &str, hypothesis: &str) -> (String, bool)` 명시. second = was_truncated. |
| critic | NEW-N2 empty hypothesis | §2.3 — `if !acc.trim().is_empty()` guard. empty answer 는 step 8.5 skip — 다른 path (LlmStreamAborted 등) 가 처리. |
| critic | What's missing RAM protocol | §3 PR-9d — `ps -o rss` 1초 sampling. peak < 10 GB 검증. |
| critic | What's missing S3 EN | §7 — S3 표 row 의 language `(EN)` 명시. |
| planner | round-2 nit #1 9c-1/9c-2 별 PR | §3 PR-9c — "별 PR 2개로 분할 (9c-1 → 9c-2 sequential 머지)" 명시. |
| planner | round-2 nit #2 9c-2 dependency | §3 PR-9c-2 — "Dependency: PR-9c-1 머지 완료" 명시. |
| planner | round-2 nit #3 시간 합산 | spec self-review 의 시간 합산 19-28h (plan v2 갱신 시 정정 예정). |
| planner | round-2 nit #4 9d subagent prereq | §3 PR-9d — Ollama running + corpus 존재 + network reachable + free RAM 검증 prereq 명시. |

### Round-3 issues (post-spec-v3 review)

| reviewer | round-3 issue | round-4 resolution |
|---|---|---|
| critic | R3-NEW-M1 truncate_for_nli signature mismatch | §2.2.3 — *3-arg* recommendation 제거, `(premise, hypothesis) -> (String, bool)` 단일 source. signature lock = §3 PR-9c-2. |
| critic | R3-NEW-M2 S7 latency wrong baseline | §7 표 S7 row — `158s + NLI ~50ms` (multi-hop pipeline 전체 진행 후 step 8.5 refuse). probe gate pass 가 원인 설명 inline. |
| critic | R3-NEW-N1 MAX_NLI_PREMISE_CHARS 한국어 token ratio | §3 PR-9c-2 — 4 char ≈ 1 token (EN BPE), 한국어 SentencePiece 1-2 char/token. tokenizer OnlyFirst backup 명시 + dogfood S10 (KR) 측정 후 v0.18.1 token-count 기반 budget 갱신. |
| critic | R3-NEW-N2 LLM 모델 환경 모순 | §5 step 8 + release notes — RAM peak 의 모델 명시 (gemma3:4b 기준 ~5-6 GB, 9B+ 모델 *추정* ~10 GB / 16 GB 경계). |
| critic | R3-NEW-N3 prereq scope | §3 PR-9d — "Pre-run prereq (manual + subagent 양쪽)" wording 갱신. |

### Round-4 issues (post-spec-v4 review)

| reviewer | round-4 issue | round-5 resolution |
|---|---|---|
| critic | R4-NEW-M1 §3 PR-9d 표 vs §7 표 latency contradiction (S7) | §3 PR-9d — Expected 표 전체 제거, "§7 verification plan 표 단일 source of truth" cross-ref 로 대체. duplication 회피. |
| critic | R4-NEW-N1 §3 PR-9d 표 format inconsistency | R4-NEW-M1 와 동시 closure (cross-ref 가 양쪽 해결). |
| critic | R4-NEW-NIT-1 9B+ 모델 naming | §5 step 8 — "9B+ 모델" → "8B+ Q4 모델 (gemma4:e4b 8B / gemma2:9b 등)" |
| critic | R4-NEW-NIT-2 §6 default 0.5 wording | §6 — "kebab default 0.5" → "kebab 권장 enable 값 0.5 (config default 는 0.0 disabled, §2.6 참조)" |
