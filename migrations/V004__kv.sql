-- V004__kv.sql — single-row key/value table for monotonic counters.
--
-- p9-fb-19 introduces an in-process LRU search cache; cache keys carry
-- a `corpus_revision` snapshot so a successful `kebab ingest` (which
-- bumps the counter) automatically invalidates every prior entry.
-- Persisting the counter in SQLite (rather than holding it in memory)
-- means a fresh process picks up the latest value, so a CLI invocation
-- after an ingest in another process correctly skips the stale cache.
--
-- Schema is a generic `key/value` so future scalars (last_compaction,
-- last_doctor_run, ...) can land here without another migration. The
-- value column is TEXT because SQLite has no opinion on integer width
-- and downstream code can parse `u64` / `i64` / strings as needed.
--
-- Seed `corpus_revision = 0` so the first cache miss after a fresh
-- install gets a defined snapshot; ingest's `bump_corpus_revision`
-- moves it to 1 on the first successful commit.

CREATE TABLE kv (
  key   TEXT PRIMARY KEY NOT NULL,
  value TEXT NOT NULL
) STRICT;

INSERT INTO kv (key, value) VALUES ('corpus_revision', '0');
