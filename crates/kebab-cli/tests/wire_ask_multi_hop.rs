//! p9-fb-41 PR-4: CLI `--multi-hop` flag wiring + answer.v1 / error.v1
//! schema additivity.
//!
//! Four Ollama-free pins:
//!
//! 1. `--multi-hop` is exposed on `kebab ask --help` so users can
//!    discover the flag at the CLI surface (clap-level smoke).
//! 2. `answer.schema.json` parses as valid JSON and declares a
//!    `hops` property with a `HopRecord` `$defs` entry — guards
//!    against accidental schema deletion / typo in future edits.
//! 3. `answer.schema.json`'s `refusal_reason` enum lists
//!    `multi_hop_decompose_failed` — agents validating against
//!    the schema accept the new variant on refusal answers.
//! 4. `error.schema.json`'s `code` enum lists
//!    `multi_hop_decompose_failed` — forward-looking enum extension
//!    documented in PR-4.
//!
//! End-to-end multi-hop ask against a live Ollama lands in a
//! follow-up `#[ignore]` test (same pattern as `wire_ask_stale.rs`).

use std::path::PathBuf;
use std::process::Command;

fn schema_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("docs")
        .join("wire-schema")
        .join("v1")
        .join(name)
}

fn parse_schema(name: &str) -> serde_json::Value {
    let text = std::fs::read_to_string(schema_path(name))
        .unwrap_or_else(|e| panic!("read {name}: {e}"));
    serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("{name} must parse as valid JSON: {e}"))
}

#[test]
fn cli_ask_help_advertises_multi_hop_flag() {
    let bin = env!("CARGO_BIN_EXE_kebab");
    let out = Command::new(bin).args(["ask", "--help"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("--multi-hop"),
        "`kebab ask --help` must advertise --multi-hop so users can discover it:\n{stdout}"
    );
}

#[test]
fn answer_schema_declares_hops_property_with_hop_record_defs() {
    let schema = parse_schema("answer.schema.json");
    assert!(
        schema["properties"]["hops"].is_object(),
        "`hops` property must be declared on answer.v1"
    );
    // `hops` allows array-or-null (single-pass omits the field;
    // multi-hop emits a non-empty array).
    let hops_any_of = schema["properties"]["hops"]["anyOf"]
        .as_array()
        .expect("hops must declare anyOf (array | null)");
    assert!(
        hops_any_of.iter().any(|v| v["type"] == "array"),
        "hops anyOf must include array shape"
    );
    assert!(
        hops_any_of.iter().any(|v| v["type"] == "null"),
        "hops anyOf must include null (single-pass omits the field)"
    );

    // HopRecord $defs entry — guards against accidental deletion or
    // structural drift in future schema edits.
    let hop_record = &schema["$defs"]["HopRecord"];
    assert!(
        hop_record.is_object(),
        "$defs.HopRecord must be declared so `hops.items` can $ref it"
    );
    let kind_enum = hop_record["properties"]["kind"]["enum"]
        .as_array()
        .expect("HopRecord.kind must be an enum");
    let kinds: Vec<&str> = kind_enum.iter().filter_map(|v| v.as_str()).collect();
    for needed in ["decompose", "decide", "synthesize"] {
        assert!(
            kinds.contains(&needed),
            "HopRecord.kind enum must include {needed:?}, got {kinds:?}"
        );
    }
}

#[test]
fn answer_schema_refusal_reason_enum_includes_multi_hop_decompose_failed() {
    let schema = parse_schema("answer.schema.json");
    let refusal_any_of = schema["properties"]["refusal_reason"]["anyOf"]
        .as_array()
        .expect("refusal_reason must declare anyOf");
    let enum_arr = refusal_any_of
        .iter()
        .find_map(|v| v["enum"].as_array())
        .expect("one of refusal_reason.anyOf entries must declare an enum");
    let values: Vec<&str> = enum_arr.iter().filter_map(|v| v.as_str()).collect();
    assert!(
        values.contains(&"multi_hop_decompose_failed"),
        "refusal_reason enum must include `multi_hop_decompose_failed`, got {values:?}"
    );
    // All earlier RefusalReason wire values remain on the enum —
    // guards against an accidental rewrite dropping old variants.
    for needed in [
        "score_gate",
        "llm_self_judge",
        "no_index",
        "no_chunks",
        "llm_stream_aborted",
    ] {
        assert!(
            values.contains(&needed),
            "refusal_reason enum must keep prior variant {needed:?}, got {values:?}"
        );
    }
}

#[test]
fn error_schema_code_enum_includes_multi_hop_decompose_failed() {
    let schema = parse_schema("error.schema.json");
    let code_enum = schema["properties"]["code"]["enum"]
        .as_array()
        .expect("error.v1 must declare code.enum");
    let values: Vec<&str> = code_enum.iter().filter_map(|v| v.as_str()).collect();
    assert!(
        values.contains(&"multi_hop_decompose_failed"),
        "error.v1 code enum must include forward-looking `multi_hop_decompose_failed`, got {values:?}"
    );
    // Existing codes remain — guards against accidental deletion.
    for needed in [
        "config_invalid",
        "not_indexed",
        "model_unreachable",
        "generic",
    ] {
        assert!(
            values.contains(&needed),
            "error.v1 code enum must keep prior code {needed:?}, got {values:?}"
        );
    }
}
