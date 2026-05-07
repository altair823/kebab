//! Long-lived server state — holds Config so per-request handlers don't
//! reload from disk. Future: cache opened SqliteStore / Lance handles
//! here so first tool call pays the cost, subsequent calls hit warm
//! state.

use std::path::PathBuf;
use std::sync::Arc;

use kebab_config::Config;

#[derive(Clone)]
pub struct KebabAppState {
    pub config: Arc<Config>,
    /// `--config <path>` from CLI when present, else `None` (XDG default
    /// fallback applies in `doctor_with_config_path`).
    pub config_path: Option<PathBuf>,
}

impl KebabAppState {
    pub fn new(config: Config, config_path: Option<PathBuf>) -> Self {
        Self {
            config: Arc::new(config),
            config_path,
        }
    }
}
