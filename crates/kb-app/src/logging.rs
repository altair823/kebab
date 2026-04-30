//! Tracing initialization helper for `kb-cli`.
//!
//! Daily-rolling file appender at `~/.local/state/kb/logs/` per task spec.
//! Returns a `WorkerGuard` that the caller must keep alive until program
//! exit (so buffered log lines flush).

use anyhow::Result;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LogLevel {
    Default,
    Verbose,
    Debug,
}

/// Initialize tracing. Returns a guard to keep alive until exit. Idempotent
/// — a second call is a no-op (the second `try_init` is dropped silently
/// but the guard is still returned so the caller can keep it alive).
pub fn init(level: LogLevel) -> Result<WorkerGuard> {
    let log_dir = kb_config::Config::xdg_state_dir().join("logs");
    std::fs::create_dir_all(&log_dir)?;

    let file_appender = tracing_appender::rolling::daily(&log_dir, "kb.log");
    let (nb, guard) = tracing_appender::non_blocking(file_appender);

    let env_filter = match level {
        LogLevel::Default => EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
        LogLevel::Verbose => EnvFilter::new("info"),
        LogLevel::Debug => EnvFilter::new("debug"),
    };

    let registry = tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt::layer().with_writer(nb).with_ansi(false));

    // `try_init` rather than `init` so a second call (e.g. in tests) is a
    // no-op.
    let _ = registry.try_init();

    Ok(guard)
}
