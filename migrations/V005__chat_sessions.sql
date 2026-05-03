-- V005__chat_sessions.sql — multi-turn conversation persistence.
--
-- p9-fb-17 introduces session-level storage for the multi-turn `Ask`
-- conversation primitive (p9-fb-15 facade, p9-fb-16 TUI). Each session
-- groups N consecutive Q/A turns under one `session_id`; the TUI
-- "이전 대화 이어가기" + the future `kebab ask --session foo` flag
-- (p9-fb-18) read+append against these tables.
--
-- Schema notes:
--
-- * `session_id` is user-supplied (`--session foo`) or auto-derived
--   from `blake3(first_question || first_ts)` as a 32-hex string. No
--   foreign-key into another table — sessions are sovereign.
--
-- * `chat_turns.turn_index` is monotonic per session (0-based). The
--   `UNIQUE(session_id, turn_index)` pair enforces the invariant on
--   the storage side so a buggy caller cannot double-append turn 3.
--
-- * `ON DELETE CASCADE` so `kebab reset --data-only` (p9-fb-06)
--   wipes both tables together — orphan turns can never outlive
--   their session.
--
-- * `config_snapshot_json` mirrors `eval_runs.config_snapshot_json`
--   (P5-1) — captures the prompt_template_version, llm.model, and
--   max_context_tokens that produced the session so a retroactive
--   answer-quality regression can be re-traced.
--
-- * `citations_json` carries `Vec<AnswerCitation>` (per p9-fb-18) —
--   each AnswerCitation holds a `Citation` plus `marker`, so the
--   answer can be redisplayed with the same citation markers a
--   future session sees on resume.
--
-- * `INTEGER` timestamps (unix epoch seconds) — same convention the
--   rest of the schema uses (P1-7 baselines this).

CREATE TABLE chat_sessions (
  session_id           TEXT    PRIMARY KEY NOT NULL,
  created_at           INTEGER NOT NULL,
  updated_at           INTEGER NOT NULL,
  title                TEXT,
  config_snapshot_json TEXT    NOT NULL
) STRICT;

CREATE TABLE chat_turns (
  turn_id        TEXT    PRIMARY KEY NOT NULL,
  session_id     TEXT    NOT NULL REFERENCES chat_sessions(session_id) ON DELETE CASCADE,
  turn_index     INTEGER NOT NULL,
  question       TEXT    NOT NULL,
  answer         TEXT    NOT NULL,
  citations_json TEXT    NOT NULL,
  created_at     INTEGER NOT NULL,
  UNIQUE(session_id, turn_index)
) STRICT;

CREATE INDEX idx_chat_turns_session ON chat_turns(session_id, turn_index);
