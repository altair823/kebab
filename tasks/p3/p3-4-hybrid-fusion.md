---
phase: P3
component: kb-search (hybrid)
task_id: p3-4
title: "Hybrid Retriever (RRF) over lexical + vector"
status: completed
depends_on: [p2-2, p3-3]
unblocks: [p4-3]
contract_source: ../../docs/superpowers/specs/2026-04-27-kb-final-form-design.md
contract_sections: [§3.7 RetrievalDetail, §0 Q3, §1.6 search --explain, §6.4 [search] rrf settings]
---

# p3-4 — Hybrid Retriever (RRF)

## Goal

Compose `LexicalRetriever` (p2-2) and a vector retriever wrapper around `LanceVectorStore` (p3-3) into a single `Retriever` that dispatches by `SearchMode`. For `Hybrid`, fuse via Reciprocal Rank Fusion (RRF) and populate full `RetrievalDetail` per `SearchHit`.

## Why now / why this size

Single mediator. Keeps the lexical and vector retrievers focused; only this task knows how to fuse. RAG (p4-3) consumes hybrid output without caring about the underlying retrievers.

## Allowed dependencies

- `kb-core`
- `kb-config`
- `kb-store-sqlite` (for `LexicalRetriever`)
- `kb-store-vector` (for `LanceVectorStore`)
- `kb-embed` (trait only — for query embedding via `Embedder`)
- `tracing`
- `thiserror`

## Forbidden dependencies

- `kb-source-fs`, `kb-parse-md`, `kb-normalize`, `kb-chunk`, `kb-llm*`, `kb-rag`, `kb-tui`, `kb-desktop`. (`kb-embed-local` is a runtime-injected `dyn Embedder`; this crate must not depend on the concrete adapter directly.)

## Inputs

| input | type | source |
|-------|------|--------|
| `LexicalRetriever` | trait object | constructed elsewhere |
| `LanceVectorStore` | trait object | constructed elsewhere |
| `Box<dyn Embedder>` | for query embedding | runtime-injected |
| `kb-config::Config.search` | `default_k`, `hybrid_fusion`, `rrf_k` | runtime |
| `SearchQuery` | `kb_core::SearchQuery` | `kb-app::search` |

## Outputs

| output | type | downstream |
|--------|------|------------|
| `Vec<SearchHit>` (with full `RetrievalDetail`) | `kb_core::SearchHit` | `kb-cli` printer, `kb-rag` packer |

## Public surface (signatures only — no new types)

```rust
pub struct HybridRetriever {
    lexical: std::sync::Arc<dyn kb_core::Retriever>,
    vector:  std::sync::Arc<dyn kb_core::Retriever>,   // wrapper over LanceVectorStore + Embedder
    fusion:  FusionPolicy,
    k:       usize,
}

pub enum FusionPolicy { Rrf { k_rrf: u32 } }

impl HybridRetriever {
    pub fn new(
        config: &kb_config::Config,
        lexical: std::sync::Arc<dyn kb_core::Retriever>,
        vector:  std::sync::Arc<dyn kb_core::Retriever>,
    ) -> Self;
}

impl kb_core::Retriever for HybridRetriever {
    fn search(&self, query: &kb_core::SearchQuery) -> anyhow::Result<Vec<kb_core::SearchHit>>;
    fn index_version(&self) -> kb_core::IndexVersion;
}

/// Wrapper that turns a VectorStore + Embedder into a Retriever.
pub struct VectorRetriever {
    store:   std::sync::Arc<dyn kb_core::VectorStore>,
    embed:   std::sync::Arc<dyn kb_core::Embedder>,
    /* heading_path/snippet enrichment hits SQLite via kb-store-sqlite read accessor */
}
impl VectorRetriever {
    pub fn new(store: std::sync::Arc<dyn kb_core::VectorStore>, embed: std::sync::Arc<dyn kb_core::Embedder>, sqlite: std::sync::Arc<kb_store_sqlite::SqliteStore>) -> Self;
}
impl kb_core::Retriever for VectorRetriever { /* per §7.2 */ }
```

## Behavior contract

- `SearchMode::Lexical` dispatches solely to `lexical`. `RetrievalDetail.method = Lexical`, `vector_*` fields are `None`.
- `SearchMode::Vector` dispatches solely to `vector`. `RetrievalDetail.method = Vector`, `lexical_*` fields are `None`.
- `SearchMode::Hybrid`:
  - run `lexical.search(query)` and `vector.search(query)` in sequence (fan-out is fine; not required).
  - fuse with RRF: `raw(c) = Σ_{m ∈ {lex, vec}} 1 / (k_rrf + rank_m(c))` where `k_rrf` from config (default 60). `rank_m` is 1-based; chunks not appearing in retriever `m` contribute 0.
  - **normalize fusion_score to [0, 1]** (post-merge fix, 2026-05): divide by `num_retrievers / (k_rrf + 1)` so the top-1-everywhere case maps to `1.0` and single-retriever chunks cap around `0.5`. Without this, raw RRF tops out at `≈ 0.033` and is incomparable with the `[0, 1]` lexical / vector `fusion_score` (and incompatible with the `config.rag.score_gate` default `0.05` — every hybrid query refused). RRF's rank ordering is preserved (we divide every score by the same positive constant). See [HOTFIXES.md](../HOTFIXES.md).
  - sort by fused score DESC, take top `query.k`.
  - populate every `SearchHit.retrieval`: `method = Hybrid`, `lexical_score` / `lexical_rank` / `vector_score` / `vector_rank` from each retriever's hit (or `None` if absent), `fusion_score` = normalized fused score.
  - if a chunk appears in only one retriever, its `RetrievalDetail` still gets populated with `Some(...)` from that side and `None` for the other.
  - tie-break by `lexical_rank` ascending, then `chunk_id` ascending (deterministic).
- `VectorRetriever`:
  - embeds the query via `embed.embed(&[EmbeddingInput { text: query.text, kind: Query }])`.
  - calls `VectorStore::search(query_vec, query.k * 2, query.filters)` (over-fetch for filter losses), trims to `k`.
  - hydrates `doc_path` / `heading_path` / `section_label` / `chunker_version` / `embedding_model` from SQLite by joining on `chunk_id`.
  - builds `Citation` from chunk's first source span (same logic as p2-2).
- `index_version()` returns the lexical index version when in pure lexical mode, else the vector index version, else "hybrid:<lex_iv>+<vec_iv>".

## Storage / wire effects

- Reads only. No mutations.
- Output JSON conforms to `search_hit.v1`.

## Test plan

| kind | description | fixture / data |
|------|-------------|----------------|
| unit | pure lexical mode delegates 1:1 to `lexical.search` | mock retrievers |
| unit | pure vector mode delegates 1:1 to `vector.search` | mock retrievers |
| unit | hybrid: chunk only in lexical receives `vector_*: None`, but still has a fused score | mock retrievers |
| unit | RRF formula matches expected with `k_rrf=60` | inline math test |
| unit | tie-break deterministic (same fused score → stable order) | inline |
| unit | hybrid recall ≥ max(lexical recall, vector recall) on a tiny corpus where each mode finds disjoint hits | tmp DB + Lance + MockEmbedder |
| determinism | identical query twice → byte-identical `Vec<SearchHit>` | tmp DB |
| snapshot | hybrid output JSON stable | `fixtures/search/hybrid/run-1.json` |

All tests under `cargo test -p kb-search hybrid`.

## Definition of Done

- [ ] `cargo check -p kb-search` passes
- [ ] `cargo test -p kb-search hybrid` passes
- [ ] No imports outside Allowed dependencies
- [ ] PR links design §3.7, §6.4 search, §0 Q3

## Out of scope

- Reranker (P+).
- Multimodal retrieval (image/audio) — P6+.
- Score calibration across modes (RRF makes scores rank-comparable; absolute calibration is P+).

## Risks / notes

- Mismatched `index_version` between lexical and vector should be flagged at construction so users notice stale indexes.
- Over-fetching at the vector retriever (`2 * k`) is conservative; if filters reject everything, the hybrid `k` may shrink. Document this in CLI `--explain`.
- RRF is rank-based, so absolute lexical bm25 normalization (p2-2) doesn't affect fused order; still keep normalization for `--explain` readability.
