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
    /// Original config file path passed via `--config <path>`, if any.
    /// Forwarded to `kebab_app::doctor_with_config_path` so the doctor
    /// report reflects the same config file the server was started with.
    /// Plan Task 10 (Cmd::Mcp wiring) will pass the actual path; all
    /// existing callers pass `None` which falls back to the XDG default.
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
