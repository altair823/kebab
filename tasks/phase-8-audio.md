---
phase: P8
title: "음성 transcription + timestamp citation"
status: planned
depends_on: [P5]
source: kb_local_rust_report.md §9.3, §17 Phase 8
---

# P8 — 음성 ingestion

## 목표

audio 파일 → transcript (timestamped segment) → CanonicalDocument → 동일 검색/RAG 파이프라인. citation 은 `meeting.m4a:00:13:42-00:14:10`.

## 산출 crate

- `kb-parse-audio` — `Extractor` 구현.
- `kb-asr-whisper` (또는 `kb-parse-audio` 내부 모듈) — whisper.cpp adapter.

## 파이프라인 (§9.3)

```text
audio file
  -> (선택) decode/resample
  -> whisper.cpp transcription
  -> timestamped segments
  -> (선택) speaker diarization
  -> CanonicalDocument
```

## ASR 엔진

- 1차: whisper.cpp. Apple Silicon (Metal/Core ML/Accelerate) 가속 지원, M4 MacBook 적합.
- Rust binding 또는 sidecar binary. abstract trait `Transcriber` 로 둘 다 수용.
- 모델 선택: `large-v3` 정확도 우선, `medium`/`small` 속도 우선. config.

```rust
pub trait Transcriber {
    fn model_id(&self) -> &str;
    fn transcribe(&self, audio: &AudioInput) -> anyhow::Result<Transcript>;
}

pub struct Transcript {
    pub segments: Vec<TranscriptSegment>,
    pub language: Lang,
    pub model_id: String,
    pub model_version: String,
}

pub struct TranscriptSegment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    pub speaker: Option<String>,
    pub confidence: Option<f32>,
}
```

## Diarization (선택, 후순위)

- 화자 분리 (pyannote 등) → `speaker = "S1" | "S2" | ...`.
- 1차 구현에서는 single speaker 가정. trait 만 마련.

## CanonicalDocument 매핑

오디오 1개 = 1 document. blocks:

```rust
Block::AudioRef(AudioRefBlock {
    asset_id,
    duration_ms: u64,
    transcript_segments: Vec<TranscriptSegment>,
    transcript_engine: String,
    transcript_engine_version: String,
})
```

전체 transcript 를 한 덩어리 텍스트로도 보관 (검색 편의).

## Chunking

- segment 인접 그룹핑 → target_tokens 도달까지 합침.
- 합칠 때 첫 segment 의 `start_ms`, 마지막 segment 의 `end_ms` 가 chunk 의 `source_span`.
- 발화자 전환 시점에서 우선 분할 (있을 경우).
- chunker version: `audio-segment-v1`.

## Citation 형식

```text
meeting-2026-04-27.m4a:00:13:42-00:14:10
meeting-2026-04-27.m4a:00:13:42-00:14:10:speaker=S1
```

## CLI

```text
kb ingest ./meeting.m4a
kb ingest ./recordings/
kb search "회의에서 언급한 결정사항"
kb inspect doc <audio_doc_id>   # transcript + segment timestamp 표시
kb play <chunk_id>              # (선택) 해당 구간 재생 — 후순위
```

## 테스트

- fixture: 짧은 한국어 오디오, 영문 오디오, 한영 코드 스위칭, 잡음 포함.
- transcript timestamp 단조 증가.
- chunk 의 `source_span` 이 실제 segment 시간과 일치.
- 동일 오디오 재수집 idempotent (asset_id = blake3).
- 큰 파일 streaming 처리 (RAM 폭주 방지).

## 의존성 경계

- `kb-parse-audio` 는 `kb-core` + `Transcriber` adapter 만.
- LLM 호출 금지. RAG 단계는 transcript text 기반으로 동일 파이프라인.

## 완료 조건

- [ ] `kb ingest <audio>` 동작
- [ ] transcript 가 segment timestamp 와 함께 저장
- [ ] 검색 결과에 `00:hh:mm:ss-` citation 포함
- [ ] 동일 오디오 재수집 idempotent
- [ ] 모델 변경 시 transcript_version 추적 (재처리 대상 식별)

## 리스크 / 주의

- 모델 크기/정확도 trade-off 큼. 회의 1시간 = `large-v3` 로 분 단위 처리 시간.
- 한영 혼합/전문용어/고유명사 정확도 낮음. transcript 만으로는 RAG 답변 신뢰도 떨어질 수 있음 → citation 으로 사용자 확인 가능하게.
- diarization 도입 시 segment 경계와 speaker turn 경계 reconcile 필요. 신중.
- 저작권/프라이버시 민감. 로컬에서만 처리되는 점 명시.
