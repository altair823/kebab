//! Shared test scaffolding for `kb-app` integration tests.
//!
//! Each test gets a fresh `TempDir` and a `Config` whose storage paths
//! all point inside it, so the user's real `data_dir` / `model_dir`
//! is never touched. The fixture workspace at
//! `tests/fixtures/workspace/` is *copied* into the temp dir for each
//! test so a write-side ingest can't trip on a read-only fixture
//! tree. The default lane (no `--ignored`) opts out of embeddings via
//! `provider = "none"` so AVX is not required.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use kebab_config::Config;
use tempfile::TempDir;

/// Test environment: owns a `TempDir` and exposes a `Config` whose
/// storage paths live inside it.
pub struct TestEnv {
    pub temp: TempDir,
    pub workspace_root: PathBuf,
    pub config: Config,
}

impl TestEnv {
    /// Build an env with embeddings disabled (lexical-only). Default
    /// lane — no AVX, no fastembed download.
    pub fn lexical_only() -> Self {
        let env = Self::new_inner();
        let mut e = env;
        e.config.models.embedding.provider = "none".to_string();
        e.config.models.embedding.dimensions = 0;
        e
    }

    /// Build an env with the default fastembed embedding provider.
    /// Used by AVX-gated `#[ignore]` tests.
    pub fn with_embeddings() -> Self {
        Self::new_inner()
    }

    fn new_inner() -> Self {
        let temp = tempfile::tempdir().expect("tempdir");
        let workspace_root = temp.path().join("workspace");
        copy_fixture_workspace(&workspace_root);

        let data_dir = temp.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        let model_dir = temp.path().join("models");
        std::fs::create_dir_all(&model_dir).unwrap();

        let mut config = Config::defaults();
        config.workspace.root = workspace_root.to_string_lossy().into_owned();
        // Drop the ".obsidian" / "node_modules" excludes — they bring
        // in nothing useful for fixtures and just hide debugging.
        config.workspace.exclude.clear();
        config.storage.data_dir = data_dir.to_string_lossy().into_owned();
        // Pin model_dir to the TempDir so a future fastembed-touching
        // test can't accidentally write to the user's `~/.local/share`.
        config.storage.model_dir = model_dir.to_string_lossy().into_owned();
        // Drop in a small chunk policy so the fixture's small files
        // emit at least a couple of chunks even with overlap_tokens
        // honored.
        config.chunking.target_tokens = 80;
        config.chunking.overlap_tokens = 20;

        Self {
            temp,
            workspace_root,
            config,
        }
    }

    pub fn scope(&self) -> kebab_core::SourceScope {
        kebab_core::SourceScope {
            root: self.workspace_root.clone(),
            exclude: self.config.workspace.exclude.clone(),
            ..Default::default()
        }
    }

    /// p9-fb-34 alias — tests added in fb-34 invoke `TestEnv::new()`
    /// per the plan; route to the existing lexical-only constructor
    /// so the lane stays AVX-free without churning all the existing
    /// callers.
    pub fn new() -> Self {
        Self::lexical_only()
    }

    /// p9-fb-34: open a fresh `App` against this env's config. Used
    /// by integration tests that need to call `App::search_with_opts`
    /// directly. Caller can invoke this multiple times to simulate
    /// re-opening the binary after a corpus revision bump.
    pub fn app(&self) -> kebab_app::App {
        kebab_app::App::open_with_config(self.config.clone())
            .expect("App::open_with_config")
    }
}

/// p9-fb-34: write `content` into the env's workspace at
/// `relative_path`, then run a full ingest so the document is
/// searchable. Mirrors the convenience helpers used by other
/// `TestEnv`-driven crates.
pub fn ingest_md(env: &TestEnv, relative_path: &str, content: &str) {
    let path = env.workspace_root.join(relative_path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create parent dirs");
    }
    std::fs::write(&path, content).expect("write workspace file");
    kebab_app::ingest_with_config(env.config.clone(), env.scope(), true)
        .expect("ingest_with_config");
}

/// Test helper: build a `SearchQuery` for lexical mode at k=10. Used
/// by every kebab-app integration test that calls
/// `kebab_app::search_with_config`. Centralized here so a future
/// `SearchQuery` field bump only edits one site.
pub fn lexical_query(text: &str) -> kebab_core::SearchQuery {
    kebab_core::SearchQuery {
        text: text.to_string(),
        mode: kebab_core::SearchMode::Lexical,
        k: 10,
        filters: kebab_core::SearchFilters::default(),
    }
}

/// p9-fb-32: rewrite `documents.updated_at` for one workspace path
/// to `now - days_ago` (RFC3339 UTC). Used by staleness integration
/// tests to simulate aged-out docs without faking system time. Caller
/// is responsible for ingesting the doc *before* calling this — the
/// row must already exist.
pub fn backdate_document_updated_at(env: &TestEnv, workspace_path: &str, days_ago: i64) {
    let backdated = (time::OffsetDateTime::now_utc() - time::Duration::days(days_ago))
        .format(&time::format_description::well_known::Rfc3339)
        .expect("format backdated updated_at");
    let db_path = PathBuf::from(&env.config.storage.data_dir).join("kebab.sqlite");
    let conn = rusqlite::Connection::open(&db_path).expect("open kebab.sqlite");
    let updated = conn
        .execute(
            "UPDATE documents SET updated_at = ?1 WHERE workspace_path = ?2",
            rusqlite::params![backdated, workspace_path],
        )
        .expect("UPDATE documents.updated_at");
    assert_eq!(
        updated, 1,
        "backdate_document_updated_at: expected to update exactly 1 row for {workspace_path}, got {updated}"
    );
}

fn copy_fixture_workspace(dest: &Path) {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("workspace");
    copy_dir_recursive(&src, dest);
}

fn copy_dir_recursive(src: &Path, dest: &Path) {
    std::fs::create_dir_all(dest).unwrap();
    for entry in std::fs::read_dir(src).expect("read fixture dir") {
        let entry = entry.unwrap();
        let path = entry.path();
        let target = dest.join(entry.file_name());
        if path.is_dir() {
            copy_dir_recursive(&path, &target);
        } else {
            std::fs::copy(&path, &target).expect("copy fixture file");
        }
    }
}
