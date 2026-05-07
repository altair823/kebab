//! Long-lived server state — holds Config so per-request handlers don't
//! reload from disk. Future: cache opened SqliteStore / Lance handles
//! here so first tool call pays the cost, subsequent calls hit warm
//! state.

use std::sync::Arc;

use kebab_config::Config;

#[derive(Clone)]
pub struct KebabAppState {
    pub config: Arc<Config>,
}

impl KebabAppState {
    pub fn new(config: Config) -> Self {
        Self {
            config: Arc::new(config),
        }
    }
}
