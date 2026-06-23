-- V014: [[workspace.sources]] multi-source support.
--
-- Adds `documents.source_id`: the id of the `[[workspace.sources]]` entry a
-- document was ingested from. Single-root workspaces (and every pre-existing
-- row) get the implicit `default` id via the column DEFAULT — so this is a
-- backward-compatible additive migration (no data rewrite, no corpus_revision
-- bump required for existing chunks/embeddings).
--
-- The DEFAULT 'default' literal is kept in sync with
-- `kebab_config::DEFAULT_SOURCE_ID`. The index backs the `--source <id>`
-- search filter (SearchFilters.source_id → `d.source_id IN (...)`).

ALTER TABLE documents ADD COLUMN source_id TEXT NOT NULL DEFAULT 'default';

CREATE INDEX idx_docs_source_id ON documents(source_id);
