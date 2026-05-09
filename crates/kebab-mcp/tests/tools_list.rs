//! Integration: `build_tools_vec` returns 7 tools with correct names and
//! inputSchema. Uses the extracted `pub fn build_tools_vec()` helper — no
//! transport or RequestContext needed.

use kebab_mcp::build_tools_vec;

#[test]
fn tools_list_returns_seven_tools() {
    let tools = build_tools_vec();
    assert_eq!(tools.len(), 7, "expected exactly 7 tools, got {}", tools.len());

    let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    assert!(names.contains(&"schema"), "missing 'schema' tool");
    assert!(names.contains(&"doctor"), "missing 'doctor' tool");
    assert!(names.contains(&"search"), "missing 'search' tool");
    assert!(names.contains(&"ask"), "missing 'ask' tool");
    assert!(names.contains(&"ingest_file"), "missing 'ingest_file' tool");
    assert!(names.contains(&"ingest_stdin"), "missing 'ingest_stdin' tool");
    assert!(names.contains(&"fetch"), "missing 'fetch' tool");
}

#[test]
fn search_tool_input_schema_has_required_query() {
    let tools = build_tools_vec();
    let search = tools
        .iter()
        .find(|t| t.name.as_ref() == "search")
        .expect("search tool must be present");

    // input_schema is Arc<JsonObject> (serde_json::Map<String, Value>).
    let schema_val = serde_json::Value::Object(search.input_schema.as_ref().clone());

    let required = schema_val
        .get("required")
        .and_then(|v| v.as_array())
        .expect("search inputSchema must have a 'required' array");

    assert!(
        required.iter().any(|v| v.as_str() == Some("query")),
        "search inputSchema 'required' must contain 'query', got: {required:?}"
    );
}

#[test]
fn schema_and_doctor_tools_accept_empty_input() {
    let tools = build_tools_vec();

    for name in &["schema", "doctor"] {
        let tool = tools
            .iter()
            .find(|t| t.name.as_ref() == *name)
            .unwrap_or_else(|| panic!("{name} tool must be present"));

        let schema_val = serde_json::Value::Object(tool.input_schema.as_ref().clone());
        // An empty-input schema has type "object" and no required fields
        // (or no 'required' key at all).
        let ty = schema_val.get("type").and_then(|v| v.as_str());
        assert_eq!(
            ty,
            Some("object"),
            "{name} inputSchema 'type' must be 'object', got {ty:?}"
        );

        if let Some(required) = schema_val.get("required").and_then(|v| v.as_array()) {
            assert!(
                required.is_empty(),
                "{name} inputSchema 'required' must be empty, got: {required:?}"
            );
        }
    }
}
