-- p9-fb-23: incremental ingest needs to know which chunker / embedding
-- versions were used to populate this document so a re-ingest can
-- decide whether to skip (versions match) or re-process (any mismatch).
-- parser_version is already on documents from V001.
ALTER TABLE documents ADD COLUMN last_chunker_version TEXT;
ALTER TABLE documents ADD COLUMN last_embedding_version TEXT;
