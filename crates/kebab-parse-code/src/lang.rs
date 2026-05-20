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

/// p10-1B: workspace-relative Python file path → dotted module-path prefix.
/// See plan §Task C for the exact rules + tasks/p10/p10-1b for the §3.4
/// design contract.
pub fn module_path_for_python(workspace_path: &str) -> String {
    let mut p: &str = workspace_path;
    if let Some(rest) = p.strip_prefix("crates/") {
        if let Some(slash) = rest.find('/') {
            let after = &rest[slash + 1..];
            if let Some(stripped) = after.strip_prefix("src/") {
                p = stripped;
            }
        }
    } else if let Some(stripped) = p.strip_prefix("src/") {
        p = stripped;
    } else if let Some(stripped) = p.strip_prefix("lib/") {
        p = stripped;
    }
    let p = match p.strip_suffix(".py") {
        Some(s) => s,
        None => p.strip_suffix(".pyi").unwrap_or(p),
    };
    let p = if let Some(parent) = p.strip_suffix("/__init__") {
        parent
    } else if p == "__init__" {
        ""
    } else {
        p
    };
    p.replace('/', ".")
}

/// p10-1B: workspace-relative TS/JS file path → path-style prefix
/// (no slash replacement, no source-root strip). See plan §Task C.
pub fn module_path_for_tsjs(workspace_path: &str) -> String {
    let p = workspace_path;
    for ext in [".tsx", ".ts", ".jsx", ".mjs", ".cjs", ".js"] {
        if let Some(stripped) = p.strip_suffix(ext) {
            return stripped.to_string();
        }
    }
    p.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_path_for_python_strips_src_roots_and_extensions() {
        assert_eq!(module_path_for_python("kebab_eval/metrics.py"),       "kebab_eval.metrics");
        assert_eq!(module_path_for_python("kebab_eval/__init__.py"),      "kebab_eval");
        assert_eq!(module_path_for_python("src/foo/bar.py"),              "foo.bar");
        assert_eq!(module_path_for_python("crates/x/src/foo/bar.py"),     "foo.bar");
        assert_eq!(module_path_for_python("a/b/c.pyi"),                   "a.b.c");
        assert_eq!(module_path_for_python("standalone.py"),               "standalone");
        assert_eq!(module_path_for_python("src/__init__.py"),             "");
    }

    #[test]
    fn module_path_for_tsjs_keeps_slashes_and_strips_ext() {
        for ext in ["ts", "tsx", "js", "jsx", "mjs", "cjs"] {
            let p = format!("src/search/retriever/Retriever.{ext}");
            assert_eq!(module_path_for_tsjs(&p), "src/search/retriever/Retriever");
        }
        assert_eq!(module_path_for_tsjs("foo.ts"),                 "foo");
        assert_eq!(module_path_for_tsjs("a/b/c.ts"),               "a/b/c");
        assert_eq!(module_path_for_tsjs("packages/x/src/Foo.ts"),  "packages/x/src/Foo");
    }
}
