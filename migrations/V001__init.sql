-- V001__init.sql — full P1 schema bootstrap.
-- Per design §5.1 (meta), §5.2 (assets), §5.3 (documents/document_tags),
-- §5.4 (blocks), §5.5 (chunks — FTS5 virtual table + triggers DEFERRED to
-- V002 in P2-1), §5.6 (embedding_records), §5.7 (jobs / ingest_runs /
-- answers / eval_runs / eval_query_results).

-- §5.1 Migrations meta -------------------------------------------------------

CREATE TABLE schema_meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE TABLE migrations (
  id          INTEGER PRIMARY KEY,
  applied_at  TEXT NOT NULL,
  description TEXT NOT NULL
);

-- §5.2 Assets ----------------------------------------------------------------

CREATE TABLE assets (
  asset_id        TEXT PRIMARY KEY,
  source_uri      TEXT NOT NULL,
  workspace_path  TEXT NOT NULL,
  media_type      TEXT NOT NULL,
  byte_len        INTEGER NOT NULL,
  checksum        TEXT NOT NULL,
  storage_kind    TEXT NOT NULL CHECK (storage_kind IN ('copied','reference')),
  storage_path    TEXT NOT NULL,
  discovered_at   TEXT NOT NULL
);
CREATE UNIQUE INDEX idx_assets_workspace_path  ON assets(workspace_path);
CREATE INDEX        idx_assets_media_type      ON assets(media_type);

-- §5.3 Documents -------------------------------------------------------------

CREATE TABLE documents (
  doc_id          TEXT PRIMARY KEY,
  asset_id        TEXT NOT NULL REFERENCES assets(asset_id) ON DELETE RESTRICT,
  workspace_path  TEXT NOT NULL,
  title           TEXT,
  lang            TEXT,
  source_type     TEXT NOT NULL,
  trust_level     TEXT NOT NULL,
  parser_version  TEXT NOT NULL,
  doc_version     INTEGER NOT NULL,
  schema_version  INTEGER NOT NULL,
  metadata_json   TEXT NOT NULL,
  provenance_json TEXT NOT NULL,
  created_at      TEXT NOT NULL,
  updated_at      TEXT NOT NULL
);
CREATE UNIQUE INDEX idx_docs_workspace_path ON documents(workspace_path);
CREATE INDEX        idx_docs_lang           ON documents(lang);
CREATE INDEX        idx_docs_source_type    ON documents(source_type);

CREATE TABLE document_tags (
  doc_id TEXT NOT NULL REFERENCES documents(doc_id) ON DELETE CASCADE,
  tag    TEXT NOT NULL,
  PRIMARY KEY (doc_id, tag)
);
CREATE INDEX idx_document_tags_tag ON document_tags(tag);

-- §5.4 Blocks ----------------------------------------------------------------

CREATE TABLE blocks (
  block_id          TEXT PRIMARY KEY,
  doc_id            TEXT NOT NULL REFERENCES documents(doc_id) ON DELETE CASCADE,
  kind              TEXT NOT NULL,
  heading_path_json TEXT NOT NULL,
  ordinal           INTEGER NOT NULL,
  source_span_json  TEXT NOT NULL,
  payload_json      TEXT NOT NULL
);
CREATE INDEX idx_blocks_doc_id ON blocks(doc_id);

-- §5.5 Chunks (FTS5 virtual table + triggers deferred to V002 / P2-1) -------

CREATE TABLE chunks (
  chunk_id          TEXT PRIMARY KEY,
  doc_id            TEXT NOT NULL REFERENCES documents(doc_id) ON DELETE CASCADE,
  text              TEXT NOT NULL,
  heading_path_json TEXT NOT NULL,
  section_label     TEXT,
  source_spans_json TEXT NOT NULL,
  token_estimate    INTEGER NOT NULL,
  chunker_version   TEXT NOT NULL,
  policy_hash       TEXT NOT NULL,
  block_ids_json    TEXT NOT NULL,
  created_at        TEXT NOT NULL
);
CREATE INDEX idx_chunks_doc_id          ON chunks(doc_id);
CREATE INDEX idx_chunks_chunker_version ON chunks(chunker_version);

-- §5.6 Embedding records (P3 — table empty in P1, present for forward compat) -

CREATE TABLE embedding_records (
  embedding_id   TEXT PRIMARY KEY,
  chunk_id       TEXT NOT NULL REFERENCES chunks(chunk_id) ON DELETE CASCADE,
  model_id       TEXT NOT NULL,
  model_version  TEXT NOT NULL,
  dimensions     INTEGER NOT NULL,
  lance_table    TEXT NOT NULL,
  created_at     TEXT NOT NULL,
  UNIQUE(chunk_id, model_id, model_version, dimensions)
);
CREATE INDEX idx_embed_chunk ON embedding_records(chunk_id);
CREATE INDEX idx_embed_model ON embedding_records(model_id, model_version, dimensions);

-- §5.7 Jobs / IngestRuns / Answers / EvalRuns -------------------------------

CREATE TABLE jobs (
  job_id        TEXT PRIMARY KEY,
  kind          TEXT NOT NULL,
  status        TEXT NOT NULL CHECK (status IN ('pending','running','succeeded','failed','canceled')),
  payload_json  TEXT NOT NULL,
  progress_json TEXT,
  error_json    TEXT,
  created_at    TEXT NOT NULL,
  updated_at    TEXT NOT NULL,
  finished_at   TEXT
);
CREATE INDEX idx_jobs_status ON jobs(status);
CREATE INDEX idx_jobs_kind   ON jobs(kind);

CREATE TABLE ingest_runs (
  run_id        TEXT PRIMARY KEY,
  scope_json    TEXT NOT NULL,
  scanned       INTEGER NOT NULL,
  new_count     INTEGER NOT NULL,
  updated_count INTEGER NOT NULL,
  skipped_count INTEGER NOT NULL,
  error_count   INTEGER NOT NULL,
  duration_ms   INTEGER NOT NULL,
  started_at    TEXT NOT NULL,
  finished_at   TEXT NOT NULL,
  items_json    TEXT
);

CREATE TABLE answers (
  trace_id                TEXT PRIMARY KEY,
  query                   TEXT NOT NULL,
  answer                  TEXT NOT NULL,
  grounded                INTEGER NOT NULL,
  refusal_reason          TEXT,
  model_id                TEXT NOT NULL,
  model_provider          TEXT NOT NULL,
  embedding_model_id      TEXT,
  embedding_dimensions    INTEGER,
  prompt_template_version TEXT NOT NULL,
  retrieval_mode          TEXT NOT NULL,
  retrieval_k             INTEGER NOT NULL,
  score_gate              REAL NOT NULL,
  top_score               REAL NOT NULL,
  chunks_returned         INTEGER NOT NULL,
  chunks_used             INTEGER NOT NULL,
  citations_json          TEXT NOT NULL,
  packed_chunks_json      TEXT,
  prompt_tokens           INTEGER,
  completion_tokens       INTEGER,
  latency_ms              INTEGER,
  created_at              TEXT NOT NULL
);
CREATE INDEX idx_answers_created_at ON answers(created_at);
CREATE INDEX idx_answers_grounded   ON answers(grounded);

CREATE TABLE eval_runs (
  run_id              TEXT PRIMARY KEY,
  suite               TEXT NOT NULL,
  config_snapshot_json TEXT NOT NULL,
  aggregate_json      TEXT NOT NULL,
  commit_hash         TEXT,
  created_at          TEXT NOT NULL
);

CREATE TABLE eval_query_results (
  run_id   TEXT NOT NULL REFERENCES eval_runs(run_id) ON DELETE CASCADE,
  query_id TEXT NOT NULL,
  result_json TEXT NOT NULL,
  PRIMARY KEY (run_id, query_id)
);
