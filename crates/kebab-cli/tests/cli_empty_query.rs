//! Integration tests for Bug #14: empty or whitespace-only query must emit
//! error.v1 code=invalid_input and exit nonzero (not silent 0-hit return).

use std::process::Command;
use serde_json::Value;

fn kebab_bin() -> String {
    env!("CARGO_BIN_EXE_kebab").to_string()
}

fn parse_error_v1(stderr: &str) -> Value {
    let last = stderr.lines().last().expect("expected error.v1 ndjson on stderr");
    serde_json::from_str(last)
        .unwrap_or_else(|e| panic!("expected ndjson on stderr: {e}\nstderr={stderr}"))
}

#[test]
fn search_empty_query_emits_invalid_input() {
    for q in ["", "   "] {
        let out = Command::new(kebab_bin())
            .args(["search", q, "--json"])
            .output()
            .expect("spawn kebab");
        assert_ne!(
            out.status.code(),
            Some(0),
            "empty/whitespace query must fail (q={q:?})"
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        let v = parse_error_v1(&stderr);
        assert_eq!(v["schema_version"], "error.v1", "stderr={stderr}");
        assert_eq!(v["code"], "invalid_input", "stderr={stderr}");
    }
}

#[test]
fn ask_empty_query_emits_invalid_input() {
    let out = Command::new(kebab_bin())
        .args(["ask", "", "--json"])
        .output()
        .expect("spawn kebab");
    assert_ne!(out.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&out.stderr);
    let v = parse_error_v1(&stderr);
    assert_eq!(v["schema_version"], "error.v1");
    assert_eq!(v["code"], "invalid_input");
}
