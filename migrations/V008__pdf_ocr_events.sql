-- v0.20.x r2 Enhancement 2: PDF OCR events SQLite mirror.
-- Stores per-page OCR samples for corpus-wide latency / failure analysis.
CREATE TABLE pdf_ocr_events (
  id               INTEGER PRIMARY KEY,
  run_id           TEXT    NOT NULL,
  ts               TEXT    NOT NULL,   -- ISO 8601 UTC (RFC 3339)
  doc_id           TEXT,               -- nullable (detect-skip path)
  doc_path         TEXT    NOT NULL,
  page             INTEGER NOT NULL,
  image_byte_size  INTEGER,
  image_width      INTEGER,
  image_height     INTEGER,
  ms               INTEGER NOT NULL,
  chars            INTEGER NOT NULL,
  success          INTEGER NOT NULL,   -- 0 = fail, 1 = success
  reason           TEXT,               -- "timeout" / "ocr_error" / NULL
  ocr_engine       TEXT    NOT NULL
);
CREATE INDEX idx_pdf_ocr_events_doc_id ON pdf_ocr_events(doc_id);
CREATE INDEX idx_pdf_ocr_events_run_id ON pdf_ocr_events(run_id);
CREATE INDEX idx_pdf_ocr_events_ts     ON pdf_ocr_events(ts);
