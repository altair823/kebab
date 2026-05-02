//! Golden-set YAML loader.
//!
//! Two entry points:
//!
//! - [`load_golden_set`] — pure YAML parse + uniqueness check. Used by
//!   tests that don't have a SQLite store handy.
//! - [`load_golden_set_validated`] — additionally verifies every
//!   `expected_doc_id` / `expected_chunk_id` exists in the SQLite DB
//!   the supplied [`kebab_config::Config`] points at. Used by
//!   [`crate::run_eval`] in production so a stale golden set fails
//!   fast at run start.

use std::collections::{BTreeSet, HashSet};
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use kebab_store_sqlite::SqliteStore;

use crate::types::GoldenQuery;

/// Parse the YAML at `path` into a `Vec<GoldenQuery>` and check that
/// every `id` is unique.
///
/// The YAML is expected to be a top-level list of mappings. Required
/// fields per entry: `id`, `query`. All other fields default to empty /
/// `None` per [`GoldenQuery`]'s `serde(default)` annotations.
pub fn load_golden_set(path: &Path) -> Result<Vec<GoldenQuery>> {
    let bytes =
        std::fs::read(path).with_context(|| format!("read golden YAML from {}", path.display()))?;
    let queries: Vec<GoldenQuery> = serde_yaml::from_slice(&bytes)
        .with_context(|| format!("parse golden YAML at {}", path.display()))?;
    check_unique_ids(&queries)?;
    Ok(queries)
}

/// Same as [`load_golden_set`] but additionally validates that every
/// `expected_doc_id` and `expected_chunk_id` referenced by the loaded
/// entries actually exists in the SQLite database `cfg` resolves to.
///
/// Missing IDs are surfaced as a single sorted error listing every
/// offender, so curators can fix the whole set in one pass.
///
/// Currently used only by the in-module tests below; production code
/// inlines `load_golden_set` + `validate_against_db` in
/// [`crate::run_eval_with_config`] so the validation can run against
/// an already-opened [`kebab_config::Config`] without re-parsing YAML.
#[cfg(test)]
pub(crate) fn load_golden_set_validated(
    yaml_path: &Path,
    cfg: &kebab_config::Config,
) -> Result<Vec<GoldenQuery>> {
    let queries = load_golden_set(yaml_path)?;
    validate_against_db(&queries, cfg)?;
    Ok(queries)
}

fn check_unique_ids(queries: &[GoldenQuery]) -> Result<()> {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut dups: BTreeSet<String> = BTreeSet::new();
    for q in queries {
        if !seen.insert(q.id.as_str()) {
            dups.insert(q.id.clone());
        }
    }
    if dups.is_empty() {
        Ok(())
    } else {
        let list: Vec<String> = dups.into_iter().collect();
        Err(anyhow!("duplicate query id(s): {}", list.join(", ")))
    }
}

/// Read every doc_id / chunk_id referenced by `queries` and confirm
/// SQLite has rows for them. Builds a sorted, deduplicated error
/// message listing every missing ID.
pub(crate) fn validate_against_db(queries: &[GoldenQuery], cfg: &kebab_config::Config) -> Result<()> {
    // Short-circuit when there is nothing to validate — saves opening
    // SQLite for golden sets that omit expected_*_ids entirely.
    let needs_check = queries
        .iter()
        .any(|q| !q.expected_doc_ids.is_empty() || !q.expected_chunk_ids.is_empty());
    if !needs_check {
        return Ok(());
    }

    let store = SqliteStore::open(cfg).context("open SqliteStore for golden validation")?;
    store
        .run_migrations()
        .context("run migrations for golden validation")?;

    let mut missing_docs: BTreeSet<String> = BTreeSet::new();
    let mut missing_chunks: BTreeSet<String> = BTreeSet::new();

    for q in queries {
        for did in &q.expected_doc_ids {
            let exists = store
                .document_exists(&did.0)
                .with_context(|| format!("probe document {}", did.0))?;
            if !exists {
                missing_docs.insert(did.0.clone());
            }
        }
        for cid in &q.expected_chunk_ids {
            let exists = store
                .chunk_exists(&cid.0)
                .with_context(|| format!("probe chunk {}", cid.0))?;
            if !exists {
                missing_chunks.insert(cid.0.clone());
            }
        }
    }

    if missing_docs.is_empty() && missing_chunks.is_empty() {
        return Ok(());
    }

    let mut parts: Vec<String> = Vec::new();
    if !missing_docs.is_empty() {
        parts.push(format!(
            "missing doc_ids: {}",
            missing_docs.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }
    if !missing_chunks.is_empty() {
        parts.push(format!(
            "missing chunk_ids: {}",
            missing_chunks.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }
    Err(anyhow!(
        "golden set references unknown IDs — {}",
        parts.join("; ")
    ))
}

#[cfg(test)]
mod tests {
    //! Tests that exercise the crate-private
    //! [`load_golden_set_validated`]. The pure-parser cases live in
    //! `tests/loader.rs`; only the validated-variant cases need to sit
    //! next to the function so they can see the `pub(crate)` symbol.
    use super::*;
    use kebab_config::Config;
    use kebab_store_sqlite::SqliteStore;
    use rusqlite::params;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn rejects_unknown_expected_chunk_id() {
        let tmp = tempdir().unwrap();
        let mut config = Config::defaults();
        config.storage.data_dir = tmp.path().to_string_lossy().into_owned();

        let store = SqliteStore::open(&config).unwrap();
        store.run_migrations().unwrap();
        seed_one_chunk(&store, "doc_present", "chunk_present");

        let yaml_path = tmp.path().join("golden.yaml");
        fs::write(
            &yaml_path,
            "- id: g1\n  query: hello\n  expected_chunk_ids: [\"chunk_present\", \"chunk_missing\"]\n",
        )
        .unwrap();

        let err = load_golden_set_validated(&yaml_path, &config).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("missing chunk_ids"), "msg: {msg}");
        assert!(msg.contains("chunk_missing"), "msg: {msg}");
        assert!(!msg.contains("chunk_present"), "msg: {msg}");
    }

    #[test]
    fn accepts_resolved_ids() {
        let tmp = tempdir().unwrap();
        let mut config = Config::defaults();
        config.storage.data_dir = tmp.path().to_string_lossy().into_owned();

        let store = SqliteStore::open(&config).unwrap();
        store.run_migrations().unwrap();
        seed_one_chunk(&store, "doc_present", "chunk_present");

        let yaml_path = tmp.path().join("golden.yaml");
        fs::write(
            &yaml_path,
            "- id: g1\n  query: hello\n  expected_doc_ids: [\"doc_present\"]\n  expected_chunk_ids: [\"chunk_present\"]\n",
        )
        .unwrap();

        let qs = load_golden_set_validated(&yaml_path, &config).unwrap();
        assert_eq!(qs.len(), 1);
    }

    fn seed_one_chunk(store: &SqliteStore, doc_id: &str, chunk_id: &str) {
        let conn = store.read_conn();
        let asset_id = format!("a_{doc_id}");
        conn.execute(
            "INSERT OR IGNORE INTO assets (
                asset_id, source_uri, workspace_path, media_type, byte_len,
                checksum, storage_kind, storage_path, discovered_at
             ) VALUES (?, ?, ?, '\"markdown\"', 0,
                       'deadbeefdeadbeefdeadbeefdeadbeef',
                       'reference', ?, '1970-01-01T00:00:00Z')",
            params![asset_id, "file:///tmp/x.md", "x.md", "x.md"],
        )
        .unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO documents (
                doc_id, asset_id, workspace_path, title, lang, source_type,
                trust_level, parser_version, doc_version, schema_version,
                metadata_json, provenance_json, created_at, updated_at
             ) VALUES (?, ?, ?, NULL, 'en', 'markdown', 'primary', 'v1', 1, 1,
                       '{}', '{}', '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z')",
            params![doc_id, asset_id, "x.md"],
        )
        .unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO chunks (
                chunk_id, doc_id, text, heading_path_json, section_label,
                source_spans_json, token_estimate, chunker_version,
                policy_hash, block_ids_json, created_at
             ) VALUES (?, ?, 'hi', '[]', NULL,
                       '[{\"kind\":\"line\",\"start\":1,\"end\":3}]',
                       1, 'v1', 'h', '[]', '1970-01-01T00:00:00Z')",
            params![chunk_id, doc_id],
        )
        .unwrap();
    }
}
