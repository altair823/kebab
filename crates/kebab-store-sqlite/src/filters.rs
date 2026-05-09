//! Chunk-level filter helpers shared between retrievers.
//!
//! `kb-store-vector::search` post-filters its Lance candidate set
//! against the SQLite-side metadata (committed-status / lang / tags /
//! trust / path_glob). Rather than open a private SQL surface in
//! `kb-store-vector`, the JOIN logic lives here so:
//!
//! - The schema (and CHECK / FK invariants) stays owned by the crate
//!   that ships the migrations.
//! - `kb-store-vector` doesn't need its own `rusqlite` / `globset`
//!   direct deps — both are forbidden by the P3-3 spec's allowed-dep
//!   list.
//! - Future retrievers (e.g. a hybrid blender) can reuse the same
//!   helper without re-deriving the SQL.
//!
//! `kb-search::lexical` already has a similar `tags / lang / trust /
//! path_glob` filter pass for FTS5 results; we deliberately do *not*
//! refactor that one in this PR — its SQL is interleaved with the
//! `bm25 + snippet()` SELECT, so sharing would force an awkward
//! trait split. P3-3 spec line 27 only mandates the move for
//! `kb-store-vector`'s usage.

use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use rusqlite::{params_from_iter, ToSql};

use crate::store::SqliteStore;

impl SqliteStore {
    /// Filter `chunk_ids` down to those whose owning document passes
    /// `filters` AND whose embedding row is at `status='committed'`.
    ///
    /// The result preserves the input order so the caller can feed it
    /// back to a Lance distance-asc result list and `take(k)` directly.
    ///
    /// `filters` semantics mirror `kebab_core::SearchFilters`:
    ///
    /// - `tags_any`: doc must own at least one of the listed tags
    ///   (empty vec ⇒ no tag constraint).
    /// - `lang`: exact match against `documents.lang`.
    /// - `trust_min`: doc trust ≥ the supplied level (Generated <
    ///   Secondary < Primary, mirroring `list_documents` and
    ///   `kb-search::lexical`).
    /// - `path_glob`: shell-style glob (`*` does **not** cross `/`)
    ///   against `documents.workspace_path`. Compiled in Rust via
    ///   `globset` rather than translated to SQLite GLOB so the
    ///   semantics match `kb-search::lexical` exactly.
    ///
    /// The `embedding_records.status='committed'` predicate is always
    /// applied: tombstoned and pending rows must never surface to
    /// search callers (spec §5.6).
    pub fn filter_chunks(
        &self,
        chunk_ids: &[kebab_core::ChunkId],
        filters: &kebab_core::SearchFilters,
    ) -> Result<Vec<kebab_core::ChunkId>> {
        if chunk_ids.is_empty() {
            return Ok(Vec::new());
        }

        // Deduplicate the IN-list so a pathological caller passing
        // `[c1, c1, c1]` doesn't blow the SQL placeholder count.
        let unique_ids: Vec<String> = {
            let mut seen = HashSet::new();
            chunk_ids
                .iter()
                .filter_map(|c| {
                    if seen.insert(c.0.as_str()) {
                        Some(c.0.clone())
                    } else {
                        None
                    }
                })
                .collect()
        };

        let placeholders = std::iter::repeat_n("?", unique_ids.len())
            .collect::<Vec<_>>()
            .join(",");
        let mut sql = format!(
            "SELECT er.chunk_id, d.workspace_path
               FROM embedding_records er
               JOIN chunks c    ON c.chunk_id = er.chunk_id
               JOIN documents d ON d.doc_id  = c.doc_id
              WHERE er.status = 'committed'
                AND er.chunk_id IN ({placeholders})"
        );

        let mut bind: Vec<Box<dyn ToSql>> = unique_ids
            .iter()
            .map(|s| {
                let b: Box<dyn ToSql> = Box::new(s.clone());
                b
            })
            .collect();

        if let Some(lang) = &filters.lang {
            sql.push_str(" AND d.lang = ?");
            bind.push(Box::new(lang.0.clone()));
        }
        if let Some(min) = &filters.trust_min {
            // Mirror `list_documents` / `kb-search::lexical`: rank
            // Generated=1 < Secondary=2 < Primary=3.
            sql.push_str(
                " AND CASE d.trust_level
                       WHEN 'primary'   THEN 3
                       WHEN 'secondary' THEN 2
                       WHEN 'generated' THEN 1
                       ELSE 0 END >= ?",
            );
            let rank: i64 = match min {
                kebab_core::TrustLevel::Primary => 3,
                kebab_core::TrustLevel::Secondary => 2,
                kebab_core::TrustLevel::Generated => 1,
            };
            bind.push(Box::new(rank));
        }
        if !filters.tags_any.is_empty() {
            let tag_ph = std::iter::repeat_n("?", filters.tags_any.len())
                .collect::<Vec<_>>()
                .join(",");
            sql.push_str(&format!(
                " AND EXISTS (SELECT 1 FROM document_tags t \
                   WHERE t.doc_id = d.doc_id AND t.tag IN ({tag_ph}))"
            ));
            for tag in &filters.tags_any {
                bind.push(Box::new(tag.clone()));
            }
        }

        // p9-fb-36: media_type filter (IN-list).
        // `assets.media_type` JSON has two shapes:
        //   - unit variant (Markdown / Pdf / …): JSON text, e.g. `"markdown"`
        //   - tuple variant (Image(Png) / Audio(Mp3) / Other(s)): JSON object,
        //     e.g. `{"image": "png"}`
        // Extract a unified "kind" string for both shapes; mirrors lexical.
        if !filters.media.is_empty() {
            let media_ph = std::iter::repeat_n("?", filters.media.len())
                .collect::<Vec<_>>()
                .join(",");
            sql.push_str(&format!(
                " AND d.doc_id IN (\
                   SELECT d2.doc_id FROM documents d2 \
                   JOIN assets a ON a.asset_id = d2.asset_id \
                   WHERE CASE \
                     WHEN json_type(a.media_type) = 'text' THEN json_extract(a.media_type, '$') \
                     ELSE (SELECT key FROM json_each(a.media_type) LIMIT 1) \
                   END IN ({media_ph}))"
            ));
            for kind in &filters.media {
                bind.push(Box::new(kind.clone()));
            }
        }

        // p9-fb-36: ingested_after filter.
        // `documents.updated_at` is RFC3339 TEXT (UTC `Z` per fb-32);
        // lexicographic >= compare is correct.
        if let Some(after) = &filters.ingested_after {
            let formatted = after
                .format(&time::format_description::well_known::Rfc3339)
                .expect("OffsetDateTime formats to RFC3339");
            sql.push_str(" AND d.updated_at >= ?");
            bind.push(Box::new(formatted));
        }

        // p9-fb-36: doc_id filter — single-doc scoping.
        if let Some(id) = &filters.doc_id {
            sql.push_str(" AND d.doc_id = ?");
            bind.push(Box::new(id.0.clone()));
        }

        // Optional path_glob: applied in Rust on the rows we get back,
        // not in SQL — matching `kb-search::lexical`'s post-filter so
        // the glob semantics are byte-identical between retrievers.
        let path_matcher = match filters.path_glob.as_deref() {
            Some(pat) => Some(
                globset::GlobBuilder::new(pat)
                    .literal_separator(true)
                    .build()
                    .with_context(|| {
                        format!("kb-store-sqlite::filter_chunks: invalid path_glob {pat:?}")
                    })?
                    .compile_matcher(),
            ),
            None => None,
        };

        let conn = self.read_conn();
        let mut stmt = conn
            .prepare(&sql)
            .context("kb-store-sqlite::filter_chunks: prepare SQL")?;
        let rows = stmt
            .query_map(
                params_from_iter(bind.iter().map(|b| b.as_ref())),
                |row| {
                    let chunk_id: String = row.get(0)?;
                    let workspace_path: String = row.get(1)?;
                    Ok((chunk_id, workspace_path))
                },
            )
            .context("kb-store-sqlite::filter_chunks: execute SQL")?;

        let mut allowed: HashMap<String, String> = HashMap::new();
        for r in rows {
            let (chunk_id, workspace_path) =
                r.context("kb-store-sqlite::filter_chunks: read row")?;
            allowed.insert(chunk_id, workspace_path);
        }

        let mut out = Vec::with_capacity(chunk_ids.len());
        for cand in chunk_ids {
            let workspace_path = match allowed.get(&cand.0) {
                Some(p) => p,
                None => continue,
            };
            if let Some(m) = &path_matcher {
                if !m.is_match(workspace_path) {
                    continue;
                }
            }
            out.push(cand.clone());
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kebab_config::Config;
    use kebab_core::{ChunkId, Lang, SearchFilters, TrustLevel};
    use rusqlite::params;
    use tempfile::TempDir;
    use time::OffsetDateTime;

    use crate::EmbeddingRecordRow;

    fn open_store(tmp: &TempDir) -> SqliteStore {
        let mut c = Config::defaults();
        c.storage.data_dir = tmp.path().to_string_lossy().into_owned();
        let store = SqliteStore::open(&c).unwrap();
        store.run_migrations().unwrap();
        store
    }

    /// Seed (asset, document, document_tags, chunk) rows + a
    /// committed embedding_records row for a single chunk_id. Mirrors
    /// the shape `kb-store-vector` builds in production.
    fn seed_committed(
        store: &SqliteStore,
        chunk_id: &str,
        doc_id: &str,
        workspace_path: &str,
        lang: &str,
        tags: &[&str],
        trust: &str,
    ) {
        let asset_id = format!("a{}", &doc_id[..31]);
        {
            let conn = store.lock_conn();
            conn.execute(
                "INSERT INTO assets (
                    asset_id, source_uri, workspace_path, media_type, byte_len,
                    checksum, storage_kind, storage_path, discovered_at
                 ) VALUES (?, ?, ?, '{}', 0, 'deadbeefdeadbeefdeadbeefdeadbeef',
                           'reference', ?, '1970-01-01T00:00:00Z')",
                params![
                    asset_id,
                    format!("file://{workspace_path}"),
                    workspace_path,
                    workspace_path,
                ],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO documents (
                    doc_id, asset_id, workspace_path, title, lang, source_type,
                    trust_level, parser_version, doc_version, schema_version,
                    metadata_json, provenance_json, created_at, updated_at
                 ) VALUES (?, ?, ?, NULL, ?, 'markdown', ?, 'v1', 1, 1,
                           '{}', '{}', '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z')",
                params![doc_id, asset_id, workspace_path, lang, trust],
            )
            .unwrap();
            for t in tags {
                conn.execute(
                    "INSERT INTO document_tags (doc_id, tag) VALUES (?, ?)",
                    params![doc_id, t],
                )
                .unwrap();
            }
            conn.execute(
                "INSERT INTO chunks (
                    chunk_id, doc_id, text, heading_path_json, section_label,
                    source_spans_json, token_estimate, chunker_version,
                    policy_hash, block_ids_json, created_at
                 ) VALUES (?, ?, 'hi', '[]', NULL, '[]', 1, 'v1', 'h', '[]',
                           '1970-01-01T00:00:00Z')",
                params![chunk_id, doc_id],
            )
            .unwrap();
        }

        let embed_row = EmbeddingRecordRow {
            embedding_id: format!("e{}", &chunk_id[..31]),
            chunk_id: chunk_id.to_string(),
            model_id: "m".to_string(),
            model_version: "v1".to_string(),
            dimensions: 4,
            lance_table: "t".to_string(),
            created_at: OffsetDateTime::UNIX_EPOCH,
        };
        store
            .put_embedding_records_pending(std::slice::from_ref(&embed_row))
            .unwrap();
        store
            .mark_embedding_records_committed(std::slice::from_ref(
                &embed_row.embedding_id,
            ))
            .unwrap();
    }

    /// Variant of `seed_committed` that accepts an explicit `media_type`
    /// JSON string (e.g. `r#""markdown""#` or `r#""pdf""#`) and an
    /// explicit `updated_at` RFC3339 string so the fb-36 filter tests can
    /// exercise `media` and `ingested_after` without going through the full
    /// ingest pipeline.
    #[allow(clippy::too_many_arguments)]
    fn seed_committed_full(
        store: &SqliteStore,
        chunk_id: &str,
        doc_id: &str,
        workspace_path: &str,
        lang: &str,
        tags: &[&str],
        trust: &str,
        media_type_json: &str,
        updated_at: &str,
    ) {
        let asset_id = format!("a{}", &doc_id[..31]);
        {
            let conn = store.lock_conn();
            conn.execute(
                "INSERT INTO assets (
                    asset_id, source_uri, workspace_path, media_type, byte_len,
                    checksum, storage_kind, storage_path, discovered_at
                 ) VALUES (?, ?, ?, ?, 0, 'deadbeefdeadbeefdeadbeefdeadbeef',
                           'reference', ?, '1970-01-01T00:00:00Z')",
                params![
                    asset_id,
                    format!("file://{workspace_path}"),
                    workspace_path,
                    media_type_json,
                    workspace_path,
                ],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO documents (
                    doc_id, asset_id, workspace_path, title, lang, source_type,
                    trust_level, parser_version, doc_version, schema_version,
                    metadata_json, provenance_json, created_at, updated_at
                 ) VALUES (?, ?, ?, NULL, ?, 'markdown', ?, 'v1', 1, 1,
                           '{}', '{}', '1970-01-01T00:00:00Z', ?)",
                params![doc_id, asset_id, workspace_path, lang, trust, updated_at],
            )
            .unwrap();
            for t in tags {
                conn.execute(
                    "INSERT INTO document_tags (doc_id, tag) VALUES (?, ?)",
                    params![doc_id, t],
                )
                .unwrap();
            }
            conn.execute(
                "INSERT INTO chunks (
                    chunk_id, doc_id, text, heading_path_json, section_label,
                    source_spans_json, token_estimate, chunker_version,
                    policy_hash, block_ids_json, created_at
                 ) VALUES (?, ?, 'hi', '[]', NULL, '[]', 1, 'v1', 'h', '[]',
                           '1970-01-01T00:00:00Z')",
                params![chunk_id, doc_id],
            )
            .unwrap();
        }

        let embed_row = EmbeddingRecordRow {
            embedding_id: format!("e{}", &chunk_id[..31]),
            chunk_id: chunk_id.to_string(),
            model_id: "m".to_string(),
            model_version: "v1".to_string(),
            dimensions: 4,
            lance_table: "t".to_string(),
            created_at: OffsetDateTime::UNIX_EPOCH,
        };
        store
            .put_embedding_records_pending(std::slice::from_ref(&embed_row))
            .unwrap();
        store
            .mark_embedding_records_committed(std::slice::from_ref(
                &embed_row.embedding_id,
            ))
            .unwrap();
    }

    fn cid(s: &str) -> ChunkId {
        ChunkId(s.to_string())
    }

    #[test]
    fn filter_chunks_drops_uncommitted_rows() {
        let tmp = TempDir::new().unwrap();
        let store = open_store(&tmp);
        let c1 = "11111111111111111111111111111111";
        let c2 = "22222222222222222222222222222222";
        let d1 = "d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1";
        let d2 = "d2d2d2d2d2d2d2d2d2d2d2d2d2d2d2d2";
        seed_committed(&store, c1, d1, "a.md", "en", &[], "primary");

        // c2: chunk + doc but no committed embedding row.
        let asset_id = format!("a{}", &d2[..31]);
        let conn = store.lock_conn();
        conn.execute(
            "INSERT INTO assets (
                asset_id, source_uri, workspace_path, media_type, byte_len,
                checksum, storage_kind, storage_path, discovered_at
             ) VALUES (?, 'file://b.md', 'b.md', '{}', 0,
                       'deadbeefdeadbeefdeadbeefdeadbeef',
                       'reference', 'b.md', '1970-01-01T00:00:00Z')",
            params![asset_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO documents (
                doc_id, asset_id, workspace_path, title, lang, source_type,
                trust_level, parser_version, doc_version, schema_version,
                metadata_json, provenance_json, created_at, updated_at
             ) VALUES (?, ?, 'b.md', NULL, 'en', 'markdown', 'primary', 'v1',
                       1, 1, '{}', '{}',
                       '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z')",
            params![d2, asset_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO chunks (
                chunk_id, doc_id, text, heading_path_json, section_label,
                source_spans_json, token_estimate, chunker_version,
                policy_hash, block_ids_json, created_at
             ) VALUES (?, ?, 'hi', '[]', NULL, '[]', 1, 'v1', 'h', '[]',
                       '1970-01-01T00:00:00Z')",
            params![c2, d2],
        )
        .unwrap();
        drop(conn);

        let out = store
            .filter_chunks(&[cid(c1), cid(c2)], &SearchFilters::default())
            .unwrap();
        assert_eq!(out, vec![cid(c1)]);
    }

    #[test]
    fn filter_chunks_tags_any_lang_trust_path_glob() {
        let tmp = TempDir::new().unwrap();
        let store = open_store(&tmp);
        // c1: tags=[ko-style], lang=en, primary, notes/a.md
        // c2: tags=[other],    lang=en, primary, notes/b.md
        // c3: tags=[ko-style], lang=ko, secondary, notes/c.md
        // c4: tags=[ko-style], lang=en, generated, src/d.md
        let chunks = [
            ("11111111111111111111111111111111", "d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1", "notes/a.md", "en", "primary",   &["ko-style"][..]),
            ("22222222222222222222222222222222", "d2d2d2d2d2d2d2d2d2d2d2d2d2d2d2d2", "notes/b.md", "en", "primary",   &["other"][..]),
            ("33333333333333333333333333333333", "d3d3d3d3d3d3d3d3d3d3d3d3d3d3d3d3", "notes/c.md", "ko", "secondary", &["ko-style"][..]),
            ("44444444444444444444444444444444", "d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4", "src/d.md",   "en", "generated", &["ko-style"][..]),
        ];
        for (c, d, p, l, t, tags) in &chunks {
            seed_committed(&store, c, d, p, l, tags, t);
        }

        // tags_any=[ko-style] → c1, c3, c4 (drop c2).
        let f = SearchFilters {
            tags_any: vec!["ko-style".to_string()],
            ..Default::default()
        };
        let out = store
            .filter_chunks(
                &chunks.iter().map(|c| cid(c.0)).collect::<Vec<_>>(),
                &f,
            )
            .unwrap();
        let mut got: Vec<&str> = out.iter().map(|c| c.0.as_str()).collect();
        got.sort();
        assert_eq!(got, vec![chunks[0].0, chunks[2].0, chunks[3].0]);

        // + lang=en  → drops c3.
        let f = SearchFilters {
            tags_any: vec!["ko-style".to_string()],
            lang: Some(Lang("en".to_string())),
            ..Default::default()
        };
        let out = store
            .filter_chunks(
                &chunks.iter().map(|c| cid(c.0)).collect::<Vec<_>>(),
                &f,
            )
            .unwrap();
        let mut got: Vec<&str> = out.iter().map(|c| c.0.as_str()).collect();
        got.sort();
        assert_eq!(got, vec![chunks[0].0, chunks[3].0]);

        // + trust_min=Secondary → drops c4 (generated < secondary).
        let f = SearchFilters {
            tags_any: vec!["ko-style".to_string()],
            lang: Some(Lang("en".to_string())),
            trust_min: Some(TrustLevel::Secondary),
            ..Default::default()
        };
        let out = store
            .filter_chunks(
                &chunks.iter().map(|c| cid(c.0)).collect::<Vec<_>>(),
                &f,
            )
            .unwrap();
        let got: Vec<&str> = out.iter().map(|c| c.0.as_str()).collect();
        assert_eq!(got, vec![chunks[0].0]);

        // path_glob = "notes/*.md" with no other constraint → c1, c2, c3.
        let f = SearchFilters {
            path_glob: Some("notes/*.md".to_string()),
            ..Default::default()
        };
        let out = store
            .filter_chunks(
                &chunks.iter().map(|c| cid(c.0)).collect::<Vec<_>>(),
                &f,
            )
            .unwrap();
        let mut got: Vec<&str> = out.iter().map(|c| c.0.as_str()).collect();
        got.sort();
        assert_eq!(got, vec![chunks[0].0, chunks[1].0, chunks[2].0]);
    }

    #[test]
    fn filter_chunks_preserves_input_order_and_dedupes() {
        let tmp = TempDir::new().unwrap();
        let store = open_store(&tmp);
        let c1 = "11111111111111111111111111111111";
        let c2 = "22222222222222222222222222222222";
        let c3 = "33333333333333333333333333333333";
        seed_committed(&store, c1, "d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1", "a.md", "en", &[], "primary");
        seed_committed(&store, c2, "d2d2d2d2d2d2d2d2d2d2d2d2d2d2d2d2", "b.md", "en", &[], "primary");
        seed_committed(&store, c3, "d3d3d3d3d3d3d3d3d3d3d3d3d3d3d3d3", "c.md", "en", &[], "primary");

        // Ask in the order c3, c1, c2; result must preserve that order.
        let out = store
            .filter_chunks(&[cid(c3), cid(c1), cid(c2)], &SearchFilters::default())
            .unwrap();
        assert_eq!(out, vec![cid(c3), cid(c1), cid(c2)]);

        // Duplicates in the input survive in the output (dedup is for
        // the SQL IN-list only — caller may want repeats for ranking).
        let out = store
            .filter_chunks(&[cid(c1), cid(c1), cid(c2)], &SearchFilters::default())
            .unwrap();
        assert_eq!(out, vec![cid(c1), cid(c1), cid(c2)]);
    }

    #[test]
    fn filter_chunks_empty_input_short_circuits() {
        let tmp = TempDir::new().unwrap();
        let store = open_store(&tmp);
        let out = store.filter_chunks(&[], &SearchFilters::default()).unwrap();
        assert!(out.is_empty());
    }

    // ── p9-fb-36 new filter arms ─────────────────────────────────────────

    #[test]
    fn filter_chunks_media_type_keeps_matching_kind() {
        // c1 = markdown, c2 = pdf. Filter for pdf → only c2 survives.
        let tmp = TempDir::new().unwrap();
        let store = open_store(&tmp);
        let c1 = "11111111111111111111111111111111";
        let c2 = "22222222222222222222222222222222";
        seed_committed_full(
            &store, c1, "d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1",
            "notes/a.md", "en", &[], "primary",
            r#""markdown""#,
            "1970-01-01T00:00:00Z",
        );
        seed_committed_full(
            &store, c2, "d2d2d2d2d2d2d2d2d2d2d2d2d2d2d2d2",
            "notes/b.pdf", "en", &[], "primary",
            r#""pdf""#,
            "1970-01-01T00:00:00Z",
        );

        let f = SearchFilters {
            media: vec!["pdf".to_string()],
            ..Default::default()
        };
        let out = store
            .filter_chunks(&[cid(c1), cid(c2)], &f)
            .unwrap();
        assert_eq!(out, vec![cid(c2)], "only pdf chunk should survive media filter");
    }

    #[test]
    fn filter_chunks_ingested_after_excludes_old_docs() {
        // c1 ingested 2020, c2 ingested 2026.  filter ingested_after=2025 → only c2.
        let tmp = TempDir::new().unwrap();
        let store = open_store(&tmp);
        let c1 = "11111111111111111111111111111111";
        let c2 = "22222222222222222222222222222222";
        seed_committed_full(
            &store, c1, "d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1",
            "old.md", "en", &[], "primary",
            r#""markdown""#,
            "2020-01-01T00:00:00Z",
        );
        seed_committed_full(
            &store, c2, "d2d2d2d2d2d2d2d2d2d2d2d2d2d2d2d2",
            "new.md", "en", &[], "primary",
            r#""markdown""#,
            "2026-01-01T00:00:00Z",
        );

        let f = SearchFilters {
            ingested_after: Some(time::macros::datetime!(2025-01-01 00:00:00 UTC)),
            ..Default::default()
        };
        let out = store
            .filter_chunks(&[cid(c1), cid(c2)], &f)
            .unwrap();
        assert_eq!(out, vec![cid(c2)], "only post-2025 chunk should survive ingested_after filter");
    }

    #[test]
    fn filter_chunks_doc_id_scopes_to_single_doc() {
        // c1 belongs to d1, c2 belongs to d2. filter doc_id=d1 → only c1.
        let tmp = TempDir::new().unwrap();
        let store = open_store(&tmp);
        let c1 = "11111111111111111111111111111111";
        let c2 = "22222222222222222222222222222222";
        let d1 = "d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1";
        seed_committed_full(
            &store, c1, d1,
            "a.md", "en", &[], "primary",
            r#""markdown""#,
            "1970-01-01T00:00:00Z",
        );
        seed_committed_full(
            &store, c2, "d2d2d2d2d2d2d2d2d2d2d2d2d2d2d2d2",
            "b.md", "en", &[], "primary",
            r#""markdown""#,
            "1970-01-01T00:00:00Z",
        );

        let f = SearchFilters {
            doc_id: Some(kebab_core::DocumentId(d1.to_string())),
            ..Default::default()
        };
        let out = store
            .filter_chunks(&[cid(c1), cid(c2)], &f)
            .unwrap();
        assert_eq!(out, vec![cid(c1)], "doc_id filter must scope to the target doc only");
    }
}
