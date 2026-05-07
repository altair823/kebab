//! Map `anyhow::Error` (returned by `kebab-app` facade calls) to the
//! `error.v1` wire shape. The classifier downcasts to known typed errors
//! re-exported via `kebab_app::error_signal` (LlmError, ConfigInvalid,
//! NotIndexed) and falls back to `code: "generic"` for everything else.
//!
//! Refusal / no-hit / doctor-unhealthy are NOT routed here — they remain
//! exit-code-only signals (see main.rs `exit_code()`).

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use kebab_app::error_signal::{ConfigInvalid, LlmError, NotIndexed};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorV1 {
    pub code: String,
    pub message: String,
    pub details: Value,
    pub hint: Option<String>,
}

pub fn classify(err: &anyhow::Error, verbose: bool) -> ErrorV1 {
    if let Some(s) = err.downcast_ref::<ConfigInvalid>() {
        return ErrorV1 {
            code: "config_invalid".to_string(),
            message: s.to_string(),
            details: json!({
                "path": s.path.to_string_lossy(),
                "cause": s.cause,
            }),
            hint: Some("check `--config <path>` and TOML syntax".to_string()),
        };
    }
    if let Some(s) = err.downcast_ref::<NotIndexed>() {
        return ErrorV1 {
            code: "not_indexed".to_string(),
            message: s.to_string(),
            details: json!({
                "expected": s.expected,
                "found": s.found,
            }),
            hint: Some("run `kebab init` then `kebab ingest`".to_string()),
        };
    }
    if let Some(s) = err.downcast_ref::<LlmError>() {
        return classify_llm(s);
    }
    if let Some(io) = err.downcast_ref::<std::io::Error>() {
        return ErrorV1 {
            code: "io_error".to_string(),
            message: io.to_string(),
            details: json!({"kind": format!("{:?}", io.kind())}),
            hint: None,
        };
    }
    let mut details = json!({});
    if verbose {
        let chain: Vec<String> = err.chain().map(|c| c.to_string()).collect();
        details = json!({"chain": chain});
    }
    ErrorV1 {
        code: "generic".to_string(),
        message: err.to_string(),
        details,
        hint: None,
    }
}

fn classify_llm(s: &LlmError) -> ErrorV1 {
    match s {
        LlmError::Unreachable { endpoint, source } => ErrorV1 {
            code: "model_unreachable".to_string(),
            message: format!("ollama unreachable at {endpoint}"),
            details: json!({
                "endpoint": endpoint,
                "source": source.to_string(),
            }),
            hint: Some(format!("ensure `ollama serve` is reachable at {endpoint}")),
        },
        LlmError::ModelNotPulled(model) => ErrorV1 {
            code: "model_not_pulled".to_string(),
            message: format!("ollama model `{model}` is not pulled"),
            details: json!({"model": model}),
            hint: Some(format!("run `ollama pull {model}`")),
        },
        LlmError::Timeout(e) => ErrorV1 {
            code: "timeout".to_string(),
            message: format!("ollama timeout: {e}"),
            details: json!({"source": e.to_string()}),
            hint: Some("increase timeout or check Ollama load".to_string()),
        },
        LlmError::Stream(body) => ErrorV1 {
            code: "generic".to_string(),
            message: format!("ollama HTTP error: {body}"),
            details: json!({"body": body}),
            hint: None,
        },
        LlmError::Malformed(line) => ErrorV1 {
            code: "generic".to_string(),
            message: format!("malformed response line: {line}"),
            details: json!({"line": line}),
            hint: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_invalid_classifies_to_config_invalid_code() {
        let err = anyhow::Error::new(ConfigInvalid {
            path: std::path::PathBuf::from("/tmp/x.toml"),
            cause: "missing".to_string(),
        });
        let v1 = classify(&err, false);
        assert_eq!(v1.code, "config_invalid");
        assert_eq!(v1.details.get("path").and_then(|p| p.as_str()), Some("/tmp/x.toml"));
        assert!(v1.hint.is_some());
    }

    #[test]
    fn not_indexed_classifies_correctly() {
        let err = anyhow::Error::new(NotIndexed {
            expected: "/data/k.sqlite".to_string(),
            found: None,
        });
        let v1 = classify(&err, false);
        assert_eq!(v1.code, "not_indexed");
    }

    #[test]
    fn llm_unreachable_classifies_to_model_unreachable() {
        // We cannot construct a reqwest::Error from scratch (private constructor).
        // Use a real network call with a guaranteed-unroutable endpoint:
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_millis(50))
            .build().unwrap();
        let err = client.get("http://127.0.0.1:1").send().unwrap_err();
        let llm = LlmError::Unreachable {
            endpoint: "http://127.0.0.1:1".to_string(),
            source: err,
        };
        let anyhow_err = anyhow::Error::new(llm);
        let v1 = classify(&anyhow_err, false);
        assert_eq!(v1.code, "model_unreachable");
    }

    #[test]
    fn model_not_pulled_classifies_correctly() {
        let llm = LlmError::ModelNotPulled("gemma4:e4b".to_string());
        let v1 = classify(&anyhow::Error::new(llm), false);
        assert_eq!(v1.code, "model_not_pulled");
        assert_eq!(v1.details.get("model").and_then(|p| p.as_str()), Some("gemma4:e4b"));
    }

    #[test]
    fn unknown_error_classifies_to_generic() {
        let err = anyhow::anyhow!("something else");
        let v1 = classify(&err, false);
        assert_eq!(v1.code, "generic");
        assert!(v1.hint.is_none());
    }

    #[test]
    fn generic_with_verbose_includes_chain() {
        let err = anyhow::anyhow!("root").context("middle").context("leaf");
        let v1 = classify(&err, true);
        assert_eq!(v1.code, "generic");
        let chain = v1.details.get("chain").and_then(|c| c.as_array()).unwrap();
        assert_eq!(chain.len(), 3);
    }

    #[test]
    fn io_error_classifies_correctly() {
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "no such file");
        let err = anyhow::Error::new(io);
        let v1 = classify(&err, false);
        assert_eq!(v1.code, "io_error");
    }
}
