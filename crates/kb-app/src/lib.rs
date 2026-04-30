//! `kb-app` — facade that downstream `kb-cli` / `kb-tui` / `kb-desktop`
//! depend on (§7, §8).
//!
//! P0 implementations stub out — the signatures are frozen so that later
//! phases swap in real bodies without breaking call sites.

use std::path::PathBuf;

use anyhow::bail;
use serde::{Deserialize, Serialize};

use kb_core::{
    Answer, CanonicalDocument, Chunk, ChunkId, DocFilter, DocSummary, DocumentId,
    IngestReport, SearchHit, SearchMode, SearchQuery, SourceScope,
};

pub mod doctor_signal;
pub mod logging;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AskOpts {
    pub k: usize,
    pub explain: bool,
    pub mode: SearchMode,
    pub temperature: Option<f32>,
    pub seed: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DoctorReport {
    /// Wire schema version label (`"doctor.v1"`).
    pub schema_version: String,
    pub ok: bool,
    pub checks: Vec<DoctorCheck>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DoctorCheck {
    pub name: String,
    pub ok: bool,
    pub detail: String,
    pub hint: Option<String>,
}

/// Create XDG dirs and write a starter `config.toml`. Idempotent unless
/// `force=true` (which overwrites an existing config).
pub fn init_workspace(force: bool) -> anyhow::Result<()> {
    let cfg_path = kb_config::Config::xdg_config_path();
    let data_dir = kb_config::Config::xdg_data_dir();
    let cache_dir = kb_config::Config::xdg_cache_dir();
    let state_dir = kb_config::Config::xdg_state_dir();

    for d in [
        cfg_path.parent().map(PathBuf::from).unwrap_or_default(),
        data_dir.clone(),
        cache_dir,
        state_dir.clone(),
        state_dir.join("logs"),
    ] {
        if !d.as_os_str().is_empty() {
            std::fs::create_dir_all(&d)?;
        }
    }

    let workspace_root = expand_tilde(&kb_config::Config::defaults().workspace.root);
    std::fs::create_dir_all(&workspace_root)?;

    if !cfg_path.exists() || force {
        let cfg = kb_config::Config::defaults();
        let toml_text = toml::to_string_pretty(&cfg)?;
        std::fs::write(&cfg_path, toml_text)?;
    }

    Ok(())
}

fn expand_tilde(s: &str) -> PathBuf {
    if let Some(rest) = s.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    if s == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    }
    PathBuf::from(s)
}

pub fn ingest(_scope: SourceScope, _summary_only: bool) -> anyhow::Result<IngestReport> {
    bail!("not yet wired (P1-2)")
}

pub fn list_docs(_filter: DocFilter) -> anyhow::Result<Vec<DocSummary>> {
    bail!("not yet wired (P1-5)")
}

pub fn inspect_doc(_id: &DocumentId) -> anyhow::Result<CanonicalDocument> {
    bail!("not yet wired (P1-5)")
}

pub fn inspect_chunk(_id: &ChunkId) -> anyhow::Result<Chunk> {
    bail!("not yet wired (P1-5)")
}

pub fn search(_query: SearchQuery) -> anyhow::Result<Vec<SearchHit>> {
    bail!("not yet wired (P3-1/P4-1)")
}

pub fn ask(_query: &str, _opts: AskOpts) -> anyhow::Result<Answer> {
    bail!("not yet wired (P5-1)")
}

/// Run the doctor checks. P0 emits `config_loaded` + `data_dir_writable`
/// (downstream checks land in later phases).
pub fn doctor() -> anyhow::Result<DoctorReport> {
    tracing::debug!("doctor() invoked");
    let mut checks = Vec::new();

    // config_loaded — defaults always load; from-file is best-effort.
    let cfg_path = kb_config::Config::xdg_config_path();
    let (config_ok, config_detail) = if cfg_path.exists() {
        match kb_config::Config::from_file(&cfg_path) {
            Ok(_) => (true, cfg_path.display().to_string()),
            Err(e) => (false, format!("{} ({e})", cfg_path.display())),
        }
    } else {
        // Defaults are always loadable; report the path that would be read.
        (true, format!("{} (defaults)", cfg_path.display()))
    };
    checks.push(DoctorCheck {
        name: "config_loaded".to_string(),
        ok: config_ok,
        detail: config_detail,
        hint: if config_ok {
            None
        } else {
            Some("run `kb init` to seed config".to_string())
        },
    });

    // data_dir_writable — try to create the dir and write a probe file.
    let data_dir = kb_config::Config::xdg_data_dir();
    let writable = (|| -> anyhow::Result<()> {
        std::fs::create_dir_all(&data_dir)?;
        let probe = data_dir.join(".kb-doctor-probe");
        std::fs::write(&probe, b"ok")?;
        std::fs::remove_file(&probe).ok();
        Ok(())
    })();
    let (data_ok, data_detail, data_hint) = match writable {
        Ok(()) => (true, data_dir.display().to_string(), None),
        Err(e) => (
            false,
            format!("{} ({e})", data_dir.display()),
            Some("ensure XDG_DATA_HOME is writable".to_string()),
        ),
    };
    checks.push(DoctorCheck {
        name: "data_dir_writable".to_string(),
        ok: data_ok,
        detail: data_detail,
        hint: data_hint,
    });

    let ok = checks.iter().all(|c| c.ok);
    Ok(DoctorReport {
        schema_version: "doctor.v1".to_string(),
        ok,
        checks,
    })
}
