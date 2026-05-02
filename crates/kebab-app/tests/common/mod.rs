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
            include: self.config.workspace.include.clone(),
            exclude: self.config.workspace.exclude.clone(),
        }
    }
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
