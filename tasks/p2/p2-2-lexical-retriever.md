---
phase: P2
component: kb-search (lexical mode)
task_id: p2-2
title: "Lexical Retriever via SQLite FTS5 + bm25 + citation"
status: planned
depends_on: [p2-1]
unblocks: [p3-4, p4-3]
contract_source: ../../docs/superpowers/specs/2026-04-27-kb-final-form-design.md
contract_sections: [§3.7 SearchQuery/Hit, §0 Q3 citation (URI fragment), §1.5/1.6 search output, §2.2 wire schema, §6.4 search settings]
---

# p2-2 — Lexical Retriever (FTS5 + bm25)

## Goal

Implement `kb_core::Retriever` for `SearchMode::Lexical` using SQLite FTS5. Returns `SearchHit` with `bm25` ranking, `snippet()`-derived preview, and proper W3C-fragment citation.

## Why now / why this size

First concrete `Retriever`. Lets `kb search --mode lexical` work without any embedding/LLM infrastructure. Establishes the SearchHit construction contract that hybrid (p3-4) reuses.

## Allowed dependencies

- `kb-core`
- `kb-config`
- `kb-store-sqlite` (read access to `chunks_fts` + `chunks` + `documents`)
- `rusqlite`
- `tracing`
- `thiserror`

## Forbidden dependencies

- `kb-source-fs`, `kb-parse-md`, `kb-normalize`, `kb-chunk`, `kb-store-vector`, `kb-embed*`, `kb-llm*`, `kb-rag`, `kb-tui`, `kb-desktop`

## Inputs

| input | type | source |
|-------|------|--------|
| `SearchQuery` (mode=Lexical) | `kb_core::SearchQuery` | `kb-app::search` |
| `kb-config::search` settings (`default_k`, `snippet_chars`) | `kb_config::Config` | runtime |
| SQLite connection (read) | `rusqlite::Connection` | `kb-store-sqlite` |

## Outputs

| output | type | downstream |
|--------|------|------------|
| `Vec<SearchHit>` | `kb_core::SearchHit` | `kb-cli` printer, `kb-rag` packer (P4), hybrid (p3-4) |

## Public surface (signatures only — no new types)

```rust
pub struct LexicalRetriever { /* internal: holds an Arc<rusqlite::Connection> + IndexVersion */ }

impl LexicalRetriever {
    pub fn new(store: std::sync::Arc<kb_store_sqlite::SqliteStore>, index_version: kb_core::IndexVersion) -> Self;
}

impl kb_core::Retriever for LexicalRetriever {
    fn search(&self, query: &kb_core::SearchQuery) -> anyhow::Result<Vec<kb_core::SearchHit>>;
    fn index_version(&self) -> kb_core::IndexVersion;
}
```

## Behavior contract

- SQL pattern (read-only):
  ```sql
  SELECT
    f.chunk_id, f.doc_id,
    bm25(chunks_fts) AS score,
    snippet(chunks_fts, 3, '', '', '…', :snippet_words) AS snippet,
    c.heading_path_json, c.section_label, c.source_spans_json, c.chunker_version,
    d.workspace_path, d.title
  FROM chunks_fts f
  JOIN chunks c   ON c.chunk_id = f.chunk_id
  JOIN documents d ON d.doc_id = f.doc_id
  WHERE chunks_fts MATCH :match
  ORDER BY score
  LIMIT :k
  ```
  with `score` ASC because SQLite FTS5 returns negative bm25 (lower = better). Convert to a positive normalized score for `SearchHit.retrieval.fusion_score`: `score = -bm25_raw / (1 + abs(bm25_raw))` (bounded ~[0,1]).
- `:match` building: tokenize the query string conservatively (split on whitespace, escape FTS5 special chars, default to AND of terms; if the user supplied an explicit FTS5 expression, pass it through when wrapped in single quotes).
- `:snippet_words` derived from `config.search.snippet_chars / 4` (~chars-per-token estimate). Snippet length must not exceed `snippet_chars` characters.
- `SearchHit.citation` constructed from `chunks.source_spans_json` first span:
  - `Line` → `Citation::Line { path, start, end, section: section_label }`
  - `Page` → `Citation::Page { path, page, section: section_label }`
  - other variants → forwarded as-is.
- `SearchHit.retrieval` = `RetrievalDetail { method: SearchMode::Lexical, lexical_score: Some(normalized), vector_score: None, fusion_score: normalized, lexical_rank: Some(rank), vector_rank: None }`.
- `index_version()` returns the `IndexVersion` configured at construction (e.g., `"v1.0"`).
- Filters (`SearchFilters`):
  - `tags_any` → join `document_tags` and add `IN (:tags)` condition
  - `lang` → `documents.lang = :lang`
  - `path_glob` → SQL `LIKE` with glob translated via `globset`
  - `trust_min` → ordered enum compare
- Empty match string returns `Ok(vec![])` (no error).
- Determinism: same DB + same query → same `Vec<SearchHit>` order.

## Storage / wire effects

- Reads only. Never mutates `kb.sqlite`.
- Wire: `Vec<SearchHit>` serialized via wire schema `search_hit.v1` when `kb-cli --json` is used.

## Test plan

| kind | description | fixture / data |
|------|-------------|----------------|
| unit | empty corpus → empty `Vec<SearchHit>` | tmp DB |
| unit | single-doc corpus matches keyword and returns 1 hit with citation | tmp DB seeded from `fixtures/markdown/code-and-table.md` |
| unit | snippet length ≤ `snippet_chars` | tmp DB |
| unit | filter `tags_any=["rust"]` excludes docs without that tag | tmp DB |
| unit | citation line range round-trip equals chunk's `source_spans` first span | tmp DB |
| unit | bm25 normalization keeps top-1 score in (0, 1] | tmp DB with 3 ranked chunks |
| determinism | identical query twice produces identical hit order and scores | tmp DB |
| snapshot | `Vec<SearchHit>` JSON for fixed corpus stable | `fixtures/search/lexical/run-1.json` |

All tests under `cargo test -p kb-search lexical`.

## Definition of Done

- [ ] `cargo check -p kb-search` passes
- [ ] `cargo test -p kb-search lexical` passes
- [ ] No imports outside Allowed dependencies (`cargo tree -p kb-search` audit)
- [ ] Output JSON conforms to `docs/wire-schema/v1/search_hit.schema.json`
- [ ] PR links design §3.7, §0 Q3, §2.2

## Out of scope

- Vector search (p3-3).
- Hybrid fusion (p3-4).
- Reranker (P+).
- Korean morphological tokenizer (P+).

## Risks / notes

- bm25 raw scores depend on FTS5 internals; the normalization formula chosen here is for display + RRF input. Avoid leaking raw bm25 to wire schema.
- `globset` translation of `path_glob`: ensure `*` does not match `/` to avoid surprising matches.
- SQLite FTS5 query string is sensitive to special characters (`"`, `^`, `*`, `:`, `(`, `)`); always escape unless the caller explicitly opted into FTS5 syntax.
