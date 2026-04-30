-- V001__init.sql — schema bootstrap.
-- Per design §5.1 + §5.9. Only the meta + migrations tables land here;
-- data tables (assets, documents, blocks, chunks, fts5, …) ship in later
-- phase-specific migrations (P1-6 / P2-1 / P3-3).

CREATE TABLE schema_meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE TABLE migrations (
  id          INTEGER PRIMARY KEY,
  applied_at  TEXT NOT NULL,
  description TEXT NOT NULL
);
