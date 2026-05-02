//! [`LlmError`] — adapter-side error taxonomy mapping Ollama failure modes
//! onto the variants downstream RAG / CLI code pattern-matches against.
//!
//! Living in this crate (rather than `kb-core` or `kb-llm`) is deliberate:
//! the variants are LLM-adapter specific (e.g. "model not pulled" is an
//! Ollama-ism), and surfacing them as `anyhow::Error` source values lets
//! callers `downcast_ref::<LlmError>()` only if they actually care. Trait
//! consumers stay generic over the error.
//!
//! Display strings follow design §10 — every variant is **actionable**: it
//! tells the user the next command to run (`ollama serve`, `ollama pull`)
//! when the cause is operational rather than programmatic.

/// Errors specific to the Ollama HTTP adapter.
///
/// Wrapped into `anyhow::Error` at API boundaries; downstream code that
/// needs to render hints (e.g. `kb doctor`) can `downcast_ref::<LlmError>()`.
#[derive(thiserror::Error, Debug)]
pub enum LlmError {
    /// Ollama not running at the configured endpoint, or the host is
    /// unreachable. Detected via `reqwest::Error::is_connect()`.
    #[error(
        "ollama unreachable at {endpoint}: {source}\n\
         hint: ensure `ollama serve` is running and reachable at {endpoint}"
    )]
    Unreachable {
        endpoint: String,
        #[source]
        source: reqwest::Error,
    },

    /// Server returned 404 with a body indicating the requested model is not
    /// pulled. Carries the model id so the hint is copy-pasteable.
    #[error(
        "ollama model `{0}` is not pulled\n\
         hint: run `ollama pull {0}`"
    )]
    ModelNotPulled(String),

    /// Network read/write timed out. `reqwest::blocking::Client` is built
    /// with a 5-minute ceiling — cold-loading a 14B model can legitimately
    /// take >1 minute on first call.
    #[error("ollama timeout: {0}")]
    Timeout(#[source] reqwest::Error),

    /// HTTP-level / server-shape error: a non-404 4xx/5xx response, or a
    /// 200 response whose body is not NDJSON at all (e.g. an HTML 500 page
    /// from a misrouted reverse proxy, or a `{"error":...}` envelope on a
    /// streaming frame). Carries the response body, **truncated to 512
    /// chars** at the construction site so a megabyte-sized nginx error
    /// page or Ollama panic dump cannot blow up logs / `Display`.
    #[error("ollama HTTP error: {0}")]
    Stream(String),

    /// Mid-stream JSON parse failure on a line that should have been
    /// NDJSON: i.e. earlier lines in the same response parsed cleanly,
    /// then a later line was corrupt. Distinct from `Stream` (which covers
    /// "the server never spoke NDJSON to begin with") so callers can
    /// choose to skip vs. abort. Carries the offending line for
    /// `kb doctor`-style diagnostics.
    #[error("malformed response line: {0}")]
    Malformed(String),
}
