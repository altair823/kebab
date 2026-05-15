//! Canonical extension → language identifier mapping (spec §3.5).
//!
//! Lowercase canonical identifiers, matching tree-sitter parser conventions:
//! `rust`, `python`, `typescript`, `javascript`, `go`, `java`, `kotlin`, `c`,
//! `cpp`, `yaml`, `toml`, `json`, `shell`, `make`, `dockerfile`.

use std::path::Path;

/// Returns the canonical language identifier for a given file path, or
/// `None` if the extension / filename is not recognized.
///
/// Matching priority:
///   1. exact filename match (e.g. `Dockerfile`, `Makefile`)
///   2. lowercase extension match
pub fn code_lang_for_path(path: &Path) -> Option<&'static str> {
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        match name {
            "Dockerfile" => return Some("dockerfile"),
            "Makefile" | "GNUmakefile" => return Some("make"),
            _ => {}
        }
    }
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "rs" => Some("rust"),
        "py" | "pyi" => Some("python"),
        "ts" | "tsx" => Some("typescript"),
        "js" | "mjs" | "cjs" | "jsx" => Some("javascript"),
        "go" => Some("go"),
        "java" => Some("java"),
        "kt" | "kts" => Some("kotlin"),
        "c" | "h" => Some("c"),
        "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" => Some("cpp"),
        "yaml" | "yml" => Some("yaml"),
        "toml" => Some("toml"),
        "json" => Some("json"),
        "sh" | "bash" | "zsh" => Some("shell"),
        "mk" => Some("make"),
        "dockerfile" => Some("dockerfile"),
        _ => None,
    }
}
