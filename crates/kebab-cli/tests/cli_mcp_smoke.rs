//! Spawn `target/debug/kebab mcp` and exercise initialize → tools/list.
//!
//! rmcp 1.6 has no public in-memory test transport, so this is the only
//! end-to-end MCP assertion in the suite. The binary is located via
//! `CARGO_BIN_EXE_kebab` which cargo injects at test compile time.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

#[test]
fn cli_mcp_initialize_then_tools_list() {
    let bin = env!("CARGO_BIN_EXE_kebab");
    let mut child = Command::new(bin)
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout);

    // rmcp 1.6 defaults to protocol version "2025-03-26" (confirmed by
    // manual smoke in Task 10). The server echoes whatever version the
    // client sends during the handshake, so this literal must match.
    let init_req = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"0"}}}"#;
    writeln!(stdin, "{init_req}").unwrap();
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","method":"notifications/initialized"}}"#
    )
    .unwrap();
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{{}}}}"#
    )
    .unwrap();

    // Read initialize response.
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let init: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
    assert_eq!(
        init.get("id").and_then(serde_json::Value::as_i64),
        Some(1),
        "unexpected id in initialize response: {init}"
    );
    assert!(
        init.get("result").is_some(),
        "initialize result missing: {init}"
    );

    // Read tools/list response.
    line.clear();
    reader.read_line(&mut line).unwrap();
    let list: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
    assert_eq!(
        list.get("id").and_then(serde_json::Value::as_i64),
        Some(2),
        "unexpected id in tools/list response: {list}"
    );
    let tools = list["result"]["tools"]
        .as_array()
        .expect("tools/list result.tools must be an array");
    assert_eq!(
        tools.len(),
        8,
        "expected 8 tools (schema, doctor, search, bulk_search, ask, fetch, ingest_file, ingest_stdin), got {}: {list}",
        tools.len()
    );

    // Gracefully close stdin so the server shuts down cleanly.
    drop(stdin);
    let _ = child.wait().unwrap();
}
