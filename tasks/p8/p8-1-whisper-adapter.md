---
phase: P8
component: kebab-parse-audio (whisper adapter)
task_id: p8-1
title: "Audio Extractor + Transcriber trait + whisper.cpp adapter"
status: planned
depends_on: [p0-1, p1-6]
unblocks: [p8-2]
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§3.4 Block::AudioRef + AudioRefBlock, §3.7a Transcript + TranscriptSegment, §9.3 audio policy, §9 versioning]
---

# p8-1 — Whisper adapter

## Goal

Implement `Extractor` for `MediaType::Audio(_)` plus a `Transcriber` trait + whisper.cpp Rust binding adapter (`whisper-rs`). Produces a `CanonicalDocument` whose body is one `AudioRefBlock` populated with `Transcript { segments, language, engine, engine_version }`.

## Why now / why this size

Audio stays a single, replaceable engine boundary (Transcriber trait). Extractor + adapter together because the extractor is essentially a thin shell over the transcriber.

## Allowed dependencies

- `kebab-core`
- `kebab-config`
- `whisper-rs = "0.13"` (or current stable)
- `symphonia = { version = "0.5", features = ["all"] }` — decode `.m4a/.mp3/.wav/.flac/.ogg` to interleaved f32 PCM at the source's native sample rate / channel layout. Symphonia does NOT resample; that is rubato's job.
- `rubato = "0.15"` — sample-rate conversion to 16 kHz mono f32 (the input shape whisper.cpp expects). Use `rubato::FftFixedIn::new(input_sample_rate, 16_000, frames_per_chunk, sub_chunks, 1 /* channels after downmix */)` for fixed-input streaming; pre-mix multi-channel to mono via simple averaging before the resampler.
- `serde`, `serde_json`
- `time`
- `tracing`
- `thiserror`

## Forbidden dependencies

- `kebab-source-fs`, `kebab-parse-md`, `kebab-parse-pdf`, `kebab-parse-image`, `kebab-normalize`, `kebab-chunk`, `kebab-store-*`, `kebab-embed*`, `kebab-search`, `kebab-llm*`, `kebab-rag`, `kebab-tui`, `kebab-desktop`

## Inputs

| input | type | source |
|-------|------|--------|
| `RawAsset` | `kebab_core::RawAsset` | `kebab-source-fs` |
| audio bytes | `&[u8]` | filesystem |
| `kebab-config.audio` | `{ model_path, language, chunk_seconds, n_threads, gpu }` | runtime |

## Outputs

| output | type | downstream |
|--------|------|------------|
| `CanonicalDocument` | `kebab_core::CanonicalDocument` | `kebab-chunk` (`audio-segment-v1` chunker in p8-2) |

## Public surface (signatures only — no new types)

```rust
pub trait Transcriber: Send + Sync {
    fn engine(&self) -> &'static str;
    fn engine_version(&self) -> String;
    fn transcribe(&self, pcm_f32_16khz: &[f32], language_hint: Option<&kebab_core::Lang>) -> anyhow::Result<kebab_core::Transcript>;
}

pub struct WhisperCppTranscriber { /* internal: whisper_rs::WhisperContext */ }
impl WhisperCppTranscriber { pub fn new(config: &kebab_config::Config) -> anyhow::Result<Self>; }
impl Transcriber for WhisperCppTranscriber { /* per trait */ }

pub struct AudioExtractor { transcriber: std::sync::Arc<dyn Transcriber> }
impl AudioExtractor { pub fn new(transcriber: std::sync::Arc<dyn Transcriber>) -> Self; }
impl kebab_core::Extractor for AudioExtractor {
    fn supports(&self, m: &kebab_core::MediaType) -> bool { matches!(m, kebab_core::MediaType::Audio(_)) }
    fn parser_version(&self) -> kebab_core::ParserVersion { kebab_core::ParserVersion("audio-whisper-v1".into()) }
    fn extract(&self, ctx: &kebab_core::ExtractContext, bytes: &[u8]) -> anyhow::Result<kebab_core::CanonicalDocument>;
}
```

## Behavior contract

- Decode pipeline (in `extract`):
  1. `symphonia` opens the audio bytes, picks the best track, decodes to f32 PCM mono.
  2. Down-mixes to mono (mean of channels) and resamples to 16 kHz f32 via `rubato::FftFixedIn` (input rate from `SymphoniaTrack::codec_params.sample_rate`).
  3. Produces a single `Vec<f32>` for the entire audio.
- Transcribe via `transcriber.transcribe(&pcm, lang_hint)`. The trait returns `Transcript { segments, language: detected_lang, engine, engine_version }`.
- Build `AudioRefBlock { common, asset_id: asset.asset_id, duration_ms: ((pcm.len() as u64 * 1000) / 16_000), transcript: Some(transcript) }`.
- `common.source_span = SourceSpan::Time { start_ms: 0, end_ms: duration_ms }`.
- `title` = filename without extension; `lang` = detected language from transcript (fallback `Lang("und")`).
- `metadata.user["audio"] = { "duration_ms": ..., "sample_rate": 16000, "channels": 1, "engine": "whisper.cpp", "engine_version": "..." }`.
- `metadata.source_type = SourceType::Reference`; `trust_level = TrustLevel::Primary` (transcripts are observed text, not generated narration).
- `provenance` events: `Discovered`, `Parsed`, `Transcribed`.
- `block_id` per design §4.2 with `block_kind = "audio_ref"`, `heading_path = []`, `ordinal = 0`, `source_span = SourceSpan::Time { start_ms: 0, end_ms: duration_ms }`.
- `WhisperCppTranscriber`:
  - Loads model from `config.audio.model_path` (e.g., `~/.local/share/kebab/models/whisper/ggml-large-v3.bin`).
  - Runs with `WhisperFullParams::new(SamplingStrategy::Greedy { best_of: 1 })` — deterministic.
  - Streams in chunks of `config.audio.chunk_seconds` (default 30) to bound memory; aggregates segments.
  - `Transcript.segments` populated with `start_ms`, `end_ms`, `text`, `confidence: Some(p)` from whisper's per-token probabilities (averaged), `speaker: None` (diarization is P+).
  - `engine = "whisper.cpp"`, `engine_version = whisper_rs::version()`.
- Determinism: greedy sampling + fixed model + identical PCM → identical transcript text and segment timestamps. Tests use `base.en` (small fast model) for speed.
- Failure modes:
  - Decode failure (unsupported codec) → `anyhow::Error`.
  - Model file missing → `anyhow::Error` with hint `download whisper.cpp model and set audio.model_path`.

## Storage / wire effects

- Reads: `config.audio.model_path` (model file).
- Otherwise none directly.

## Test plan

| kind | description | fixture / data |
|------|-------------|----------------|
| unit | 3-second WAV containing "hello world" → segments[0].text contains "hello world" (using `base.en` model, downloaded once for CI) | `fixtures/audio/hello.wav` |
| unit | duration_ms matches actual audio length within ±50 ms | inline |
| unit | corrupt audio → error | `fixtures/audio/corrupt.wav` |
| unit | model file missing → error with helpful hint | inline |
| unit | language hint passed to whisper changes detected language | inline |
| determinism | identical input → identical Transcript twice | inline |
| `#[ignore]` integration | 30-second Korean audio → segments_count > 1, language = "ko" | requires `large-v3` model |
| snapshot | CanonicalDocument JSON stable for short fixture | `fixtures/audio/hello.wav` |

All tests under `cargo test -p kebab-parse-audio`. Mark slow/large-model tests `#[ignore]`.

## Definition of Done

- [ ] `cargo check -p kebab-parse-audio` passes
- [ ] `cargo test -p kebab-parse-audio` passes (excluding `#[ignore]`)
- [ ] No imports outside Allowed dependencies (resampler crate may be added — record in PR)
- [ ] First-run model download path documented (NOT performed by code; user responsibility)
- [ ] PR links design §3.4, §3.7a, §9.3

## Out of scope

- Diarization (P+).
- Real-time / streaming transcription (P+).
- Voice activity detection beyond what whisper.cpp offers internally.
- Lossless re-encoding of source audio.

## Risks / notes

- whisper.cpp model files are large (1+ GB for large-v3). Tests must default to `base.en` (~150 MB) and ship a 3-second fixture.
- macOS Metal acceleration: ensure `whisper-rs` feature flags align with M-series builds; document any required env vars.
- Decoding errors for variable-bitrate `.m4a` are common; symphonia is the most reliable Rust option but expect occasional unsupported codec; fail clean rather than panic.
- Resampling: `rubato::FftFixedIn` is the v1 default — high enough quality that whisper.cpp recognition is not the bottleneck, fast enough that decode + resample stays under real-time on M-series. If a regression appears, switch to `SincFixedIn` with PR; record the change in `engine_version` since transcript stability depends on the resampler.
