//! p9-fb-17: `ChatSessionRepo` impl for `SqliteStore`.
//!
//! `chat_sessions` + `chat_turns` tables (V005 migration) back the
//! multi-turn conversation primitive (p9-fb-15 facade, p9-fb-16 TUI,
//! p9-fb-18 CLI `--session`). The trait + row types live in
//! `kebab-core::traits` so other store backends (postgres, …) can
//! plug in without depending on this crate.

use anyhow::{Context, Result};
use kebab_core::traits::{ChatSessionRepo, ChatSessionRow, ChatTurnRow};
use rusqlite::{OptionalExtension, params};

use crate::error::StoreError;
use crate::store::SqliteStore;

impl ChatSessionRepo for SqliteStore {
    fn create_session(&self, row: &ChatSessionRow) -> Result<()> {
        let conn = self.lock_conn();
        conn.execute(
            "INSERT INTO chat_sessions
             (session_id, created_at, updated_at, title, config_snapshot_json)
             VALUES (?, ?, ?, ?, ?)",
            params![
                row.session_id,
                row.created_at,
                row.updated_at,
                row.title,
                row.config_snapshot_json,
            ],
        )
        .map_err(StoreError::from)
        .context("create_session")?;
        Ok(())
    }

    fn get_session(&self, session_id: &str) -> Result<Option<ChatSessionRow>> {
        let conn = self.read_conn();
        let row = conn
            .query_row(
                "SELECT session_id, created_at, updated_at, title, config_snapshot_json
                 FROM chat_sessions WHERE session_id = ?",
                params![session_id],
                |r| {
                    Ok(ChatSessionRow {
                        session_id: r.get(0)?,
                        created_at: r.get(1)?,
                        updated_at: r.get(2)?,
                        title: r.get(3)?,
                        config_snapshot_json: r.get(4)?,
                    })
                },
            )
            .optional()
            .map_err(StoreError::from)
            .context("get_session")?;
        Ok(row)
    }

    fn list_sessions(&self, limit: usize) -> Result<Vec<ChatSessionRow>> {
        let conn = self.read_conn();
        let mut stmt = conn
            .prepare(
                "SELECT session_id, created_at, updated_at, title, config_snapshot_json
                 FROM chat_sessions
                 ORDER BY updated_at DESC
                 LIMIT ?",
            )
            .map_err(StoreError::from)
            .context("list_sessions: prepare")?;
        let limit_i64 = i64::try_from(limit).unwrap_or(i64::MAX);
        let rows = stmt
            .query_map(params![limit_i64], |r| {
                Ok(ChatSessionRow {
                    session_id: r.get(0)?,
                    created_at: r.get(1)?,
                    updated_at: r.get(2)?,
                    title: r.get(3)?,
                    config_snapshot_json: r.get(4)?,
                })
            })
            .map_err(StoreError::from)
            .context("list_sessions: query")?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(StoreError::from).context("list_sessions: row")?);
        }
        Ok(out)
    }

    fn delete_session(&self, session_id: &str) -> Result<()> {
        let conn = self.lock_conn();
        // ON DELETE CASCADE in V005 migration sweeps `chat_turns`.
        conn.execute(
            "DELETE FROM chat_sessions WHERE session_id = ?",
            params![session_id],
        )
        .map_err(StoreError::from)
        .context("delete_session")?;
        Ok(())
    }

    fn append_turn(&self, turn: &ChatTurnRow) -> Result<()> {
        let conn = self.lock_conn();
        // Wrap insert + parent updated_at in one transaction so a
        // crash between the two never leaves a turn under a stale
        // `updated_at`.
        let tx_result: Result<()> = (|| {
            conn.execute(
                "INSERT INTO chat_turns
                 (turn_id, session_id, turn_index, question, answer,
                  citations_json, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
                params![
                    turn.turn_id,
                    turn.session_id,
                    turn.turn_index,
                    turn.question,
                    turn.answer,
                    turn.citations_json,
                    turn.created_at,
                ],
            )
            .map_err(StoreError::from)
            .context("append_turn: insert")?;
            conn.execute(
                "UPDATE chat_sessions SET updated_at = ? WHERE session_id = ?",
                params![turn.created_at, turn.session_id],
            )
            .map_err(StoreError::from)
            .context("append_turn: bump updated_at")?;
            Ok(())
        })();
        tx_result
    }

    fn list_turns(&self, session_id: &str) -> Result<Vec<ChatTurnRow>> {
        let conn = self.read_conn();
        let mut stmt = conn
            .prepare(
                "SELECT turn_id, session_id, turn_index, question, answer,
                        citations_json, created_at
                 FROM chat_turns
                 WHERE session_id = ?
                 ORDER BY turn_index ASC",
            )
            .map_err(StoreError::from)
            .context("list_turns: prepare")?;
        let rows = stmt
            .query_map(params![session_id], |r| {
                Ok(ChatTurnRow {
                    turn_id: r.get(0)?,
                    session_id: r.get(1)?,
                    turn_index: r.get(2)?,
                    question: r.get(3)?,
                    answer: r.get(4)?,
                    citations_json: r.get(5)?,
                    created_at: r.get(6)?,
                })
            })
            .map_err(StoreError::from)
            .context("list_turns: query")?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(StoreError::from).context("list_turns: row")?);
        }
        Ok(out)
    }
}
