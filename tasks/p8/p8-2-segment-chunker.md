---
phase: P8
component: kb-chunk (audio-segment-v1)
task_id: p8-2
title: "Audio segment chunker (audio-segment-v1)"
status: planned
depends_on: [p8-1]
unblocks: []
contract_source: ../../docs/superpowers/specs/2026-04-27-kb-final-form-design.md
contract_sections: [§3.5 Chunk, §3.4 SourceSpan::Time, §4.2 chunk_id recipe, §0 Q3 citation, §9 versioning]
---

# p8-2 — Audio segment chunker

## Goal

Implement `Chunker` with `chunker_version = "audio-segment-v1"`. Groups consecutive transcript segments into chunks that approach `target_tokens` while respecting speaker-turn boundaries (when present).

## Why now / why this size

Per-medium chunker. Tiny but versioned — `chunk_id` depends on `chunker_version` so labeling matters.

## Allowed dependencies

- `kb-core`
- `kb-config`
- `serde`, `serde_json`
- `blake3` (policy_hash)
- `serde-json-canonicalizer`
- `thiserror`

## Forbidden dependencies

- `kb-source-fs`, `kb-parse-md`, `kb-parse-pdf`, `kb-parse-image`, `kb-parse-audio` (consumes via `kb-core` only), `kb-normalize`, `kb-store-*`, `kb-embed*`, `kb-search`, `kb-llm*`, `kb-rag`, `kb-tui`, `kb-desktop`

## Inputs

| input | type | source |
|-------|------|--------|
| `CanonicalDocument` containing one `AudioRefBlock` with `Transcript` | `kb_core::CanonicalDocument` | p8-1 |
| `ChunkPolicy` | `kb_core::ChunkPolicy` | `kb-app` |

## Outputs

| output | type | downstream |
|--------|------|------------|
| `Vec<Chunk>` | `kb_core::Chunk` | `kb-store-sqlite`, embedders |

## Public surface (signatures only — no new types)

```rust
pub struct AudioSegmentV1Chunker;

impl kb_core::Chunker for AudioSegmentV1Chunker {
    fn chunker_version(&self) -> kb_core::ChunkerVersion { kb_core::ChunkerVersion("audio-segment-v1".into()) }
    fn policy_hash(&self, policy: &kb_core::ChunkPolicy) -> String;
    fn chunk(&self, doc: &kb_core::CanonicalDocument, policy: &kb_core::ChunkPolicy) -> anyhow::Result<Vec<kb_core::Chunk>>;
}
```

`policy_hash` = `blake3(canonical_json(policy))` truncated to 16 hex chars.

## Behavior contract

- Operates only on documents whose first block is `Block::AudioRef` with `Some(transcript)`. Other documents → `anyhow::Error("AudioSegmentV1Chunker only handles audio docs")`.
- Iterate `transcript.segments` (already in chronological order):
  - Greedily group adjacent segments until estimated token budget approaches `policy.target_tokens` (`bytes / 4` proxy on segment text).
  - Force a split when `segment[i].speaker != segment[i-1].speaker` (only if speaker info present), even if budget not met.
  - No overlap across chunks (audio chunk overlap is rarely useful for retrieval).
- For each emitted chunk:
  - `text` = `segments.iter().map(|s| s.text).join(" ")`.
  - `source_spans = vec![SourceSpan::Time { start_ms: first.start_ms, end_ms: last.end_ms }]` (single span covering the whole chunk).
  - `heading_path = vec![]`.
  - `block_ids = [audio_ref_block.block_id]` (always one block per chunk).
  - `token_estimate = byte_len / 4`.
- Empty transcript (`segments.is_empty()`) → `Vec::new()` (no chunks).
- Speaker label for citation: if all segments in a chunk share a speaker, the chunk's `Citation::Time { speaker: Some(...) }` (constructed downstream by retrieval) preserves it. This task's responsibility ends at populating `source_spans`; retrieval-side citation construction reads `transcript.segments` from DB to attach speaker (or this chunker can serialize speaker into a small extension JSON in `chunk.heading_path` — chosen approach: leave the speaker propagation to the retriever, NOT the chunker, because including it in `chunk_id` would couple speakers into `chunk_id`).
- Determinism: identical `Transcript.segments` + identical policy → identical chunk_ids and text.

## Storage / wire effects

- None.

## Test plan

| kind | description | fixture / data |
|------|-------------|----------------|
| unit | 5 segments under target → 1 chunk; total span = first.start_ms..last.end_ms | inline |
| unit | 20 segments well above target → multiple chunks, none cross speaker change | inline (with synthetic speakers) |
| unit | empty transcript → empty Vec | inline |
| unit | non-audio doc returns error | inline (Markdown-like doc) |
| determinism | same input → same chunk_ids twice | inline |
| snapshot | `Vec<Chunk>` JSON for fixture transcript stable | `fixtures/audio/transcript-1.json` (constructed) |

All tests under `cargo test -p kb-chunk audio`.

## Definition of Done

- [ ] `cargo check -p kb-chunk` passes (md-heading-v1 + pdf-page-v1 + audio-segment-v1 all coexist)
- [ ] `cargo test -p kb-chunk audio` passes
- [ ] Snapshot stable across two runs
- [ ] No imports outside Allowed dependencies
- [ ] PR links design §3.5, §3.4 SourceSpan::Time, §4.2

## Out of scope

- Diarization-aware chunking beyond honoring existing speaker boundaries.
- Time-overlap chunks (intentionally not supported in v1).
- Real tokenizer integration (P+ replaces byte proxy across all chunkers).

## Risks / notes

- Speaker boundary forcing can create very small chunks if speakers alternate fast (e.g., interview Q/A). Document a `policy.min_segments_per_chunk` knob (default 1) to optionally suppress force-splits below the floor — implementer's call to add a config knob if metric pressure demands.
- Citation speaker inference at retrieval time needs DB lookup of `transcript_segments` (or a `transcript_segments` table — none exists yet). For v1, surface speaker info via the wire `Citation::Time.speaker` only when the retriever can confidently attach it; otherwise leave `None`. This task does not block on that decision.
- Bumping `chunker_version` invalidates downstream embeddings; treat as a versioning event per §9.
