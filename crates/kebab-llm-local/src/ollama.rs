//! [`OllamaLanguageModel`] — `reqwest::blocking` adapter for Ollama's
//! `POST /api/generate` streaming endpoint.
//!
//! ## Wire shape
//!
//! Request body (design §11.2 / §6.4):
//!
//! ```json
//! {
//!   "model": "<config.models.llm.model>",
//!   "prompt": "<system + '\n\n' + user>",
//!   "stream": true,
//!   "options": {
//!     "temperature": <float>,
//!     "seed":        <u64>,
//!     "num_ctx":     <usize>,
//!     "stop":        ["<str>", ...]
//!   }
//! }
//! ```
//!
//! Response is line-delimited JSON; each non-empty line is one
//! [`OllamaLine`]. Non-final lines carry `response: "..."` plus
//! `done: false`; the final line carries `done: true` plus aggregate
//! counters (`prompt_eval_count`, `eval_count`, `total_duration`).
//!
//! ## Streaming model
//!
//! [`generate_stream`] returns a stateful [`OllamaStream`] that owns the
//! `BufReader<reqwest::blocking::Response>` and yields one `TokenChunk` per
//! `Iterator::next` call — true streaming, no `collect` into a buffer. This
//! matters because cold-loading a 14B model can take >1 minute and we want
//! tokens to appear as the server emits them.
//!
//! ## Send-safety
//!
//! `reqwest::blocking::Response: Send`, so `BufReader<Response>: Send`, so
//! the boxed iterator satisfies the trait's `+ Send` bound without any
//! `Mutex` ceremony.

use std::io::{BufRead, BufReader};
use std::time::Duration;

use kebab_core::{
    FinishReason, GenerateRequest, LanguageModel, ModelRef, TokenChunk, TokenUsage,
};
use serde::{Deserialize, Serialize};

use crate::error::LlmError;

// v0.17.0 post-dogfood: the per-request ceiling now lives in
// `kebab_config::LlmCfg::request_timeout_secs` (default 300s) so users
// running larger models on CPU-only hosts can extend it without a
// rebuild. Cold-loading an 8B+ model on first call routinely takes
// 60-90 s plus multi-minute inference; 300s was the legacy hard
// ceiling and remains the default for back-compat.
//
// Edge case: `request_timeout_secs = 0` becomes
// `Duration::from_secs(0)` which is reqwest's "fail immediately", NOT
// "disable". The field doc explains the workaround (use u64::MAX or a
// large finite value).

/// `reqwest::blocking` adapter implementing [`LanguageModel`] over Ollama's
/// local HTTP API. Construction is cheap and offline; the first network
/// call happens inside [`generate_stream`].
pub struct OllamaLanguageModel {
    client: reqwest::blocking::Client,
    /// Already-validated endpoint URL string (e.g. `"http://127.0.0.1:11434"`).
    /// Stored as `String` rather than `url::Url` to keep the dep set minimal.
    endpoint: String,
    model_id: String,
    context_tokens: usize,
    default_temperature: f32,
    default_seed: u64,
}

impl OllamaLanguageModel {
    /// Build an adapter from a workspace [`kebab_config::Config`]. Reads
    /// `config.models.llm.{provider, model, endpoint, context_tokens,
    /// temperature, seed}`.
    ///
    /// Does NOT touch the network — see module docs. The caller is
    /// expected to have validated `provider == "ollama"`; this constructor
    /// trusts the config and would happily build for an unknown provider.
    /// (Provider routing is the App layer's job, not the adapter's.)
    pub fn new(config: &kebab_config::Config) -> anyhow::Result<Self> {
        let llm = &config.models.llm;
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(llm.request_timeout_secs))
            .build()?;
        Ok(Self {
            client,
            endpoint: llm.endpoint.clone(),
            model_id: llm.model.clone(),
            context_tokens: llm.context_tokens,
            default_temperature: llm.temperature,
            default_seed: llm.seed,
        })
    }
}

impl LanguageModel for OllamaLanguageModel {
    fn model_ref(&self) -> ModelRef {
        ModelRef {
            id: self.model_id.clone(),
            // Per design §3.8 / §6.4 — adapters that route through Ollama
            // report `provider = "ollama"` regardless of which model id
            // they carry, so downstream `Answer.model` displays consistently.
            provider: "ollama".to_string(),
            // Chat models have no embedding dimension — see §3.8.
            dimensions: None,
        }
    }

    fn context_tokens(&self) -> usize {
        self.context_tokens
    }

    fn generate_stream(
        &self,
        req: GenerateRequest,
    ) -> anyhow::Result<Box<dyn Iterator<Item = anyhow::Result<TokenChunk>> + Send>> {
        // ── Resolve effective options ──────────────────────────────────
        //
        // Per design §6.4 the effective temperature/seed come from the
        // config defaults. `GenerateRequest` exposes a `temperature: f32`
        // (always present) and `seed: Option<u64>` so the request can
        // override on a per-call basis. Resolution order:
        //   - temperature: `req.temperature` always wins. The field is
        //     non-Optional, so the RAG layer always declares an intent
        //     (typically `0.0` for grounded answers); the workspace
        //     default merely seeds that field at the RAG construction
        //     site. NaN is rejected → fall back to the config default
        //     so a malformed RAG request can never reach Ollama.
        //   - seed: `req.seed.unwrap_or(default_seed)` — Optional in the
        //     request, so the config default is the natural fallback.
        let effective_temperature = if req.temperature.is_nan() {
            self.default_temperature
        } else {
            req.temperature
        };
        let effective_seed = req.seed.unwrap_or(self.default_seed);

        let prompt = if req.system.is_empty() {
            req.user.clone()
        } else {
            format!("{}\n\n{}", req.system, req.user)
        };

        // Vision inputs (P6-3) flow through the request via Ollama's
        // `images: [base64, ...]` field. Empty for the text-only RAG
        // path so older snapshots and JSON dumps stay byte-identical
        // (the field is `#[serde(default)]` here so it's omitted from
        // the wire when empty).
        let body = OllamaRequest {
            model: &self.model_id,
            prompt,
            images: &req.images,
            stream: true,
            options: OllamaOptions {
                temperature: effective_temperature,
                seed: effective_seed,
                num_ctx: self.context_tokens,
                stop: &req.stop,
            },
        };

        // ── Send (blocking) ────────────────────────────────────────────
        let url = format!("{}/api/generate", self.endpoint.trim_end_matches('/'));
        let response = match self.client.post(&url).json(&body).send() {
            Ok(r) => r,
            Err(e) => return Err(map_send_error(e, &self.endpoint).into()),
        };

        let status = response.status();
        if !status.is_success() {
            // Read the body so we can pattern-match on Ollama's "model not
            // found" envelope (§11.2). Body read is bounded by the server
            // — Ollama only sends a short JSON envelope on error, no
            // streaming body to drain.
            let body_text = response.text().unwrap_or_default();
            return Err(map_status_error(status, &body_text, &self.model_id).into());
        }

        // ── Hand off to the streaming iterator ─────────────────────────
        let stream = OllamaStream {
            reader: BufReader::new(response),
            line_buf: Vec::with_capacity(1024),
            done: false,
            has_emitted: false,
        };
        Ok(Box::new(stream))
    }
}

// ── Wire types ────────────────────────────────────────────────────────────

/// Ollama `/api/generate` request body. Borrowed model id + stop list keep
/// allocations to one (the prompt) per call.
#[derive(Serialize)]
struct OllamaRequest<'a> {
    model: &'a str,
    prompt: String,
    /// Skipped from the JSON when empty so the text-only path keeps
    /// the same on-the-wire shape it had pre-P6-3 (`{"model": ...,
    /// "prompt": ..., "stream": ..., "options": ...}` — no `images`
    /// key). Vision-capable callers populate this with one or more
    /// base64-encoded images.
    #[serde(skip_serializing_if = "<[String]>::is_empty")]
    images: &'a [String],
    stream: bool,
    options: OllamaOptions<'a>,
}

#[derive(Serialize)]
struct OllamaOptions<'a> {
    temperature: f32,
    seed: u64,
    num_ctx: usize,
    stop: &'a [String],
}

/// One line of the streaming response. All counter fields are optional
/// because older Ollama builds omit them on the final frame; see §10
/// "actionable errors" — we degrade gracefully with `0` + a `tracing::warn`
/// span rather than failing the stream.
#[derive(Deserialize, Default, Debug)]
struct OllamaLine {
    /// Token text. Absent / empty on the final `done: true` line.
    #[serde(default)]
    response: String,
    /// Terminal frame marker.
    #[serde(default)]
    done: bool,
    /// `"stop"` | `"length"` | `"abort"` | (older builds: missing).
    #[serde(default)]
    done_reason: Option<String>,
    /// Tokens consumed by the prompt. Older Ollama: absent → defaulted to 0.
    #[serde(default)]
    prompt_eval_count: Option<u64>,
    /// Tokens generated. Older Ollama: absent → defaulted to 0.
    #[serde(default)]
    eval_count: Option<u64>,
    /// Total wall-clock in nanoseconds. Older Ollama: absent → 0.
    #[serde(default)]
    total_duration: Option<u64>,
    /// Server-side error message (Ollama uses this on some 200-with-error
    /// frames). When present we surface it instead of treating the line as
    /// a token.
    #[serde(default)]
    error: Option<String>,
}

// ── Streaming iterator ────────────────────────────────────────────────────

/// Stateful iterator over Ollama's NDJSON stream.
///
/// Owns the `BufReader<Response>` so reading is incremental — `next()`
/// blocks only as long as it takes Ollama to flush the next line.
///
/// Iterator semantics:
/// - `Some(Ok(TokenChunk::Token(_)))` for each non-terminal frame.
/// - One terminal `Some(Ok(TokenChunk::Done { .. }))` on the `done: true`
///   line, after which `done == true` and subsequent calls return `None`.
/// - `Some(Err(_))` on parse / I/O failure; the iterator does **not** yield
///   `Done` after an error. Callers that need a guaranteed terminal frame
///   should adapt with their own wrapper (the trait contract for streams
///   ending in `Done` is enforced by the RAG pipeline, not the adapter).
///
/// Timeout invariant: the iterator has no inherent stop condition for an
/// indefinitely-stalled server — only the underlying
/// `reqwest::blocking::Client`'s read timeout (configured via
/// `kebab_config::LlmCfg::request_timeout_secs`, default 300 s) breaks
/// the hang. Callers needing tighter / looser bounds should set
/// `[models.llm] request_timeout_secs = N` (or
/// `KEBAB_MODELS_LLM_REQUEST_TIMEOUT_SECS=N`) before building.
struct OllamaStream {
    reader: BufReader<reqwest::blocking::Response>,
    line_buf: Vec<u8>,
    done: bool,
    /// Tracks whether we have parsed at least one valid NDJSON line. Used
    /// to discriminate "server never spoke NDJSON" (→ `LlmError::Stream`)
    /// from "mid-stream corruption" (→ `LlmError::Malformed`); see §10
    /// error taxonomy split.
    has_emitted: bool,
}

impl Iterator for OllamaStream {
    type Item = anyhow::Result<TokenChunk>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        loop {
            self.line_buf.clear();
            // `read_until(b'\n', ...)` accumulates bytes across HTTP chunk
            // boundaries until it hits a newline (or EOF). UTF-8 multibyte
            // sequences inside a JSON `response` field are therefore
            // always whole by the time we attempt to decode — the line
            // boundary IS the safe re-sync point.
            let read = match self.reader.read_until(b'\n', &mut self.line_buf) {
                Ok(n) => n,
                Err(e) => {
                    self.done = true;
                    return Some(Err(anyhow::anyhow!(e).context(
                        "failed to read next line from ollama /api/generate stream",
                    )));
                }
            };
            if read == 0 {
                // EOF without a `done: true` line. Treat as a stream
                // anomaly — synthesize an Aborted Done so downstream
                // pipelines that expect a terminal frame still terminate.
                self.done = true;
                tracing::warn!(
                    target: "kebab_llm_local",
                    "ollama stream ended without a `done: true` frame; synthesizing Aborted",
                );
                return Some(Ok(TokenChunk::Done {
                    finish_reason: FinishReason::Aborted,
                    usage: TokenUsage {
                        prompt_tokens: 0,
                        completion_tokens: 0,
                        latency_ms: 0,
                    },
                }));
            }

            // Strip trailing `\n` / `\r\n`. Empty lines (keep-alive
            // heartbeats, blank separators) are skipped silently.
            let trimmed = trim_trailing_newline(&self.line_buf);
            if trimmed.is_empty() {
                continue;
            }

            let line: OllamaLine = match serde_json::from_slice(trimmed) {
                Ok(l) => l,
                Err(e) => {
                    self.done = true;
                    let preview = String::from_utf8_lossy(trimmed).into_owned();
                    if !self.has_emitted {
                        // First line of the body did not parse as NDJSON
                        // at all — the server clearly didn't speak the
                        // protocol (e.g. an HTML 500 page from a
                        // misrouted reverse proxy returning 200). Per §10
                        // error taxonomy this is `Stream`, not
                        // `Malformed`.
                        return Some(Err(anyhow::Error::from(LlmError::Stream(
                            truncate_body(&preview, 512),
                        ))));
                    }
                    // Mid-stream corruption — earlier lines parsed, this
                    // one didn't. That's `Malformed`.
                    return Some(Err(anyhow::Error::from(LlmError::Malformed(format!(
                        "{e}: line={preview}"
                    )))));
                }
            };
            // We've now parsed at least one structurally-valid NDJSON
            // line; subsequent parse failures count as mid-stream.
            self.has_emitted = true;

            // Server-side error envelope on a 200 stream.
            if let Some(err) = line.error {
                self.done = true;
                return Some(Err(anyhow::Error::from(LlmError::Stream(
                    truncate_body(&err, 512),
                ))));
            }

            if line.done {
                self.done = true;
                let finish_reason = match line.done_reason.as_deref() {
                    Some("length") => FinishReason::Length,
                    Some("abort") => FinishReason::Aborted,
                    // Per §11.2 missing or unknown done_reason → Stop.
                    // We treat unrecognised reasons as Stop rather than
                    // surfacing them as `Error(_)` because Ollama
                    // historically used "stop" as the only terminal value
                    // and forward-compatible parsing should be lenient.
                    _ => FinishReason::Stop,
                };
                let prompt_tokens = line.prompt_eval_count.unwrap_or_else(|| {
                    tracing::warn!(
                        target: "kebab_llm_local",
                        "ollama done frame missing prompt_eval_count; defaulting to 0",
                    );
                    0
                });
                let completion_tokens = line.eval_count.unwrap_or_else(|| {
                    tracing::warn!(
                        target: "kebab_llm_local",
                        "ollama done frame missing eval_count; defaulting to 0",
                    );
                    0
                });
                let total_duration_ns = line.total_duration.unwrap_or(0);
                let usage = TokenUsage {
                    // u32 saturation: even ~4G tokens is implausible for a
                    // single chat turn; we still saturate rather than
                    // panic on the unlikely case.
                    prompt_tokens: prompt_tokens.min(u32::MAX as u64) as u32,
                    completion_tokens: completion_tokens.min(u32::MAX as u64) as u32,
                    latency_ms: (total_duration_ns / 1_000_000).min(u32::MAX as u64) as u32,
                };
                return Some(Ok(TokenChunk::Done {
                    finish_reason,
                    usage,
                }));
            }

            // Non-terminal frame. Older Ollama versions occasionally emit
            // empty `response` strings as keep-alive — don't surface
            // those as zero-length tokens.
            if line.response.is_empty() {
                continue;
            }
            return Some(Ok(TokenChunk::Token(line.response)));
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────

fn trim_trailing_newline(bytes: &[u8]) -> &[u8] {
    let mut end = bytes.len();
    while end > 0 && (bytes[end - 1] == b'\n' || bytes[end - 1] == b'\r') {
        end -= 1;
    }
    &bytes[..end]
}

/// Map a `reqwest::Error` from the initial `.send()` to an [`LlmError`]
/// (returned to the caller as `anyhow::Error`).
fn map_send_error(err: reqwest::Error, endpoint: &str) -> LlmError {
    if err.is_timeout() {
        return LlmError::Timeout(err);
    }
    if err.is_connect() {
        return LlmError::Unreachable {
            endpoint: endpoint.to_string(),
            source: err,
        };
    }
    // Other transport errors (DNS, body builder, etc.) — surface verbatim
    // (truncated; see `truncate_body`).
    LlmError::Stream(truncate_body(&err.to_string(), 512))
}

/// Map a non-2xx HTTP response to an [`LlmError`]. Pattern-matches on the
/// 404 + "model" / "not found" body envelope to surface the actionable
/// `ollama pull <model>` hint.
fn map_status_error(
    status: reqwest::StatusCode,
    body: &str,
    model_id: &str,
) -> LlmError {
    if status == reqwest::StatusCode::NOT_FOUND {
        let lower = body.to_ascii_lowercase();
        // Heuristic: Ollama's "model not pulled" envelope is roughly
        // `{"error":"model 'qwen2.5:7b-instruct' not found, try pulling
        // it first"}`.
        //
        // Primary signal: the body mentions our exact model id —
        // language-agnostic, so a localized Ollama (e.g. Korean error
        // strings) still routes here. Fallback: the original English
        // "model" + "not found" substring check, kept for the case where
        // Ollama returns a generic envelope without echoing the model id.
        if lower.contains(&model_id.to_ascii_lowercase())
            || (lower.contains("model") && lower.contains("not found"))
        {
            return LlmError::ModelNotPulled(model_id.to_string());
        }
    }
    LlmError::Stream(truncate_body(
        &format!("status={status} body={body}"),
        512,
    ))
}

/// Truncate a body / error string to `n` characters, appending an
/// "(truncated, original N chars)" marker if the cap was hit. Counted in
/// `chars()` rather than bytes so multibyte UTF-8 (Korean / Japanese /
/// emoji) cannot land mid-codepoint.
///
/// Used at every `LlmError::Stream` construction site so a megabyte-sized
/// nginx 500 page or Ollama panic dump cannot blow up `Display` / logs.
fn truncate_body(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let mut out: String = s.chars().take(n).collect();
    out.push_str(&format!("... (truncated, original {} chars)", s.chars().count()));
    out
}

// ── Unit tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trim_trailing_newline_removes_lf_and_crlf() {
        assert_eq!(trim_trailing_newline(b"hi\n"), b"hi");
        assert_eq!(trim_trailing_newline(b"hi\r\n"), b"hi");
        assert_eq!(trim_trailing_newline(b"hi"), b"hi");
        assert_eq!(trim_trailing_newline(b""), b"");
    }

    #[test]
    fn map_status_error_404_with_model_not_found_returns_not_pulled() {
        let body = r#"{"error":"model 'qwen2.5:7b-instruct' not found, try pulling it first"}"#;
        let err = map_status_error(
            reqwest::StatusCode::NOT_FOUND,
            body,
            "qwen2.5:7b-instruct",
        );
        match err {
            LlmError::ModelNotPulled(m) => assert_eq!(m, "qwen2.5:7b-instruct"),
            other => panic!("expected ModelNotPulled, got {other:?}"),
        }
    }

    #[test]
    fn map_status_error_500_returns_stream() {
        let err = map_status_error(
            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            "boom",
            "qwen2.5:7b-instruct",
        );
        assert!(matches!(err, LlmError::Stream(_)));
    }

    #[test]
    fn map_status_error_404_with_model_id_in_localized_body_is_not_pulled() {
        // Localized Ollama: imagine a Korean build returning
        // `{"error":"모델 'qwen2.5:7b-instruct' 을(를) 찾을 수 없습니다"}`.
        // The English "not found" substring is absent, but the model id
        // is echoed — heuristic should still route to ModelNotPulled.
        let body = r#"{"error":"모델 'qwen2.5:7b-instruct' 을(를) 찾을 수 없습니다"}"#;
        let err = map_status_error(
            reqwest::StatusCode::NOT_FOUND,
            body,
            "qwen2.5:7b-instruct",
        );
        assert!(
            matches!(err, LlmError::ModelNotPulled(ref m) if m == "qwen2.5:7b-instruct"),
            "expected ModelNotPulled for localized 404 body, got {err:?}",
        );
    }

    #[test]
    fn truncate_body_under_cap_returns_input_unchanged() {
        assert_eq!(truncate_body("short", 512), "short");
        assert_eq!(truncate_body("", 512), "");
        // Boundary: exactly at the cap.
        let exact = "x".repeat(10);
        assert_eq!(truncate_body(&exact, 10), exact);
    }

    #[test]
    fn truncate_body_over_cap_appends_marker_and_caps_chars() {
        let big = "y".repeat(1000);
        let out = truncate_body(&big, 512);
        // 512 chars of payload + the truncation marker.
        assert!(out.starts_with(&"y".repeat(512)));
        assert!(out.contains("(truncated, original 1000 chars)"));
    }

    #[test]
    fn truncate_body_counts_chars_not_bytes_for_multibyte() {
        // 600 Korean chars (each ~3 UTF-8 bytes). Slicing by bytes would
        // land mid-codepoint; chars() iteration is safe.
        let big: String = "한".repeat(600);
        let out = truncate_body(&big, 512);
        // Make sure the prefix is exactly 512 Korean chars, valid UTF-8.
        let prefix: String = out.chars().take(512).collect();
        assert_eq!(prefix.chars().count(), 512);
        assert!(prefix.chars().all(|c| c == '한'));
        assert!(out.contains("(truncated, original 600 chars)"));
    }
}
