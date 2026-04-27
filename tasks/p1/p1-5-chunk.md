---
phase: P1
component: kb-chunk
task_id: p1-5
title: "Markdown heading-aware chunker (md-heading-v1)"
status: planned
depends_on: [p1-4]
unblocks: [p1-6, p2-2, p3-2]
contract_source: ../../docs/superpowers/specs/2026-04-27-kb-final-form-design.md
contract_sections: [§3.5 Chunk, §4.2 chunk_id recipe, §7.2 Chunker, §0 Q3 citation]
---

# p1-5 — Markdown heading-aware chunker

## Goal

Implement `Chunker` trait emitting `chunker_version = "md-heading-v1"`. Block-aware: respect heading boundaries, never split code/table, propagate `heading_path` and merged `source_spans`.

## Why now / why this size

The first concrete `Chunker`. Establishes how subsequent chunkers (PDF page chunker, audio segment chunker) are scoped: per-medium chunker version label. Independent of any store/embed.

## Allowed dependencies

- `kb-core`
- `kb-config`
- `serde`
- `blake3` (policy_hash)
- `serde-json-canonicalizer`
- `thiserror`

## Forbidden dependencies

- `kb-source-fs`, `kb-parse-md`, `kb-normalize` (consumes `CanonicalDocument` only via `kb-core`), `kb-store-*`, `kb-embed*`, `kb-search`, `kb-llm*`, `kb-rag`, `kb-tui`, `kb-desktop`

## Inputs

| input | type | source |
|-------|------|--------|
| `CanonicalDocument` | `kb_core::CanonicalDocument` | p1-4 |
| `ChunkPolicy` | `kb_core::ChunkPolicy` | `kb-app` from config |

## Outputs

| output | type | downstream |
|--------|------|------------|
| `Vec<Chunk>` | `kb_core::Chunk` | `kb-store-sqlite` (p1-6), `kb-embed*` (P3) |

## Public surface (signatures only — no new types)

```rust
pub struct MdHeadingV1Chunker;

impl kb_core::Chunker for MdHeadingV1Chunker {
    fn chunker_version(&self) -> kb_core::ChunkerVersion;
    fn policy_hash(&self, policy: &kb_core::ChunkPolicy) -> String;
    fn chunk(&self, doc: &kb_core::CanonicalDocument, policy: &kb_core::ChunkPolicy) -> anyhow::Result<Vec<kb_core::Chunk>>;
}
```

`policy_hash` = `blake3(canonical_json(policy))` hex truncated to 16 chars.

## Behavior contract

- Priority order (per design §0 / report §14):
  1. heading boundary first
  2. never split a code block
  3. table stays in a single chunk if possible
  4. long sections split by paragraph
  5. propagate `heading_path` from blocks
  6. carry merged `source_spans` (each chunk lists every contributing block's span)
  7. record `chunker_version = "md-heading-v1"` and `policy_hash`
- `target_tokens` and `overlap_tokens` from `ChunkPolicy`. Token estimate is byte-based proxy until a real tokenizer is introduced (note in `Chunk.token_estimate`).
- `chunk_id` per design §4.2: tagged tuple of `(doc_id, chunker_version, block_ids, policy_hash)`.
- `block_ids` listed in document order (significant — affects ID).
- ImageRef / AudioRef blocks are emitted as their own chunks (text portion = alt + caption preview if present, else empty string with `token_estimate=0`). They still receive `chunk_id` so future image/audio search can locate them.

## Storage / wire effects

- None directly. Outputs feed p1-6.

## Test plan

| kind | description | fixture / data |
|------|-------------|----------------|
| unit | heading boundary respected (no chunk crosses H2 → H2) | inline |
| unit | code block of 800 tokens stays in one chunk even when target=500 | inline |
| unit | table block stays single chunk if size < 2× target | inline |
| unit | long paragraph split with overlap_tokens applied | inline |
| unit | ImageRefBlock produces a chunk with token_estimate=0 | inline |
| determinism | identical input + identical policy → identical chunk_ids | inline |
| snapshot | `fixtures/markdown/long-section.md` → Vec<Chunk> JSON stable | fixture |

All tests under `cargo test -p kb-chunk`.

## Definition of Done

- [ ] `cargo check -p kb-chunk` passes
- [ ] `cargo test -p kb-chunk` passes
- [ ] Snapshot stable across two runs
- [ ] No imports outside Allowed dependencies
- [ ] PR links design §3.5, §4.2

## Out of scope

- DB persistence (p1-6).
- Embedding (P3).
- Reranking / hybrid (P3).

## Risks / notes

- Token estimate proxy: a real tokenizer (e.g., sentencepiece for the embedding model) replaces this in P3. The proxy must err toward overestimation so chunks fit in real tokenizer budget.
- Changing `chunker_version` invalidates all downstream embedding records. Bump only with PR documenting the migration plan (design §9).
