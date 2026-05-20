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
///   1. Tier 1 basename exact match (e.g. `Dockerfile`, `Makefile`)
///   2. Tier 2 basename match (e.g. `Cargo.toml`, `package.json`, `build.gradle`)
///   3. Tier 2 `Dockerfile.*` prefix variant
///   4. Tier 1 + Tier 2 extension fallback (lowercase)
pub fn code_lang_for_path(path: &Path) -> Option<&'static str> {
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        // Tier 1 basename exact match
        match name {
            "Dockerfile" => return Some("dockerfile"),
            "Makefile" | "GNUmakefile" => return Some("make"),
            _ => {}
        }

        // Tier 2 basename match (configuration / manifest files)
        match name {
            "Cargo.toml" | "pyproject.toml" => return Some("toml"),
            "package.json" | "tsconfig.json" => return Some("json"),
            "go.mod" => return Some("go-mod"),
            "pom.xml" => return Some("xml"),
            "build.gradle" => return Some("groovy"),
            _ => {}
        }

        // Tier 2: `Dockerfile.*` prefix variant (e.g. `Dockerfile.dev`, `Dockerfile.prod`)
        if name.starts_with("Dockerfile.") && name.len() > "Dockerfile.".len() {
            return Some("dockerfile");
        }
    }

    // Extension fallback (Tier 1 + Tier 2)
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        // Tier 1 extensions
        "rs" => Some("rust"),
        "py" | "pyi" => Some("python"),
        "ts" | "tsx" | "mts" | "cts" => Some("typescript"),
        "js" | "mjs" | "cjs" | "jsx" => Some("javascript"),
        "go" => Some("go"),
        "java" => Some("java"),
        "kt" | "kts" => Some("kotlin"),
        "c" | "h" => Some("c"),
        "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" => Some("cpp"),
        "sh" | "bash" | "zsh" => Some("shell"),
        "mk" => Some("make"),
        // Tier 2 extensions
        "yaml" | "yml" => Some("yaml"),
        "toml" => Some("toml"),
        "json" => Some("json"),
        "xml" => Some("xml"),
        "dockerfile" => Some("dockerfile"),
        "gradle" => Some("groovy"),
        _ => None,
    }
}

/// p10-1B: workspace-relative Python file path → dotted module-path prefix.
/// See plan §Task C for the exact rules + tasks/p10/p10-1b for the §3.4
/// design contract.
///
/// Stripped source-roots: `src/`, `lib/`, and `crates/<crate>/src/`.
/// `tests/`, `examples/`, and `benches/` are intentionally NOT stripped —
/// they appear in test/example/bench namespaces and dropping them would
/// conflate identical symbol names across conventional Python directories
/// (e.g. `tests/test_foo.py` → `tests.test_foo`, not `test_foo`).
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
    for ext in [".tsx", ".mts", ".cts", ".ts", ".jsx", ".mjs", ".cjs", ".js"] {
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
        // `tests/` is NOT a stripped source-root — it is preserved as
        // part of the module path so test symbols stay namespaced.
        assert_eq!(module_path_for_python("tests/test_foo.py"),           "tests.test_foo");
    }

    #[test]
    fn module_path_for_tsjs_keeps_slashes_and_strips_ext() {
        for ext in ["ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs"] {
            let p = format!("src/search/retriever/Retriever.{ext}");
            assert_eq!(module_path_for_tsjs(&p), "src/search/retriever/Retriever");
        }
        assert_eq!(module_path_for_tsjs("foo.ts"),                 "foo");
        assert_eq!(module_path_for_tsjs("a/b/c.ts"),               "a/b/c");
        assert_eq!(module_path_for_tsjs("packages/x/src/Foo.ts"),  "packages/x/src/Foo");
    }

    #[test]
    fn tier2_basename_takes_precedence_over_extension() {
        assert_eq!(code_lang_for_path(Path::new("Dockerfile")),         Some("dockerfile"));
        assert_eq!(code_lang_for_path(Path::new("foo/Dockerfile.dev")), Some("dockerfile"));
        assert_eq!(code_lang_for_path(Path::new("myapp.dockerfile")),   Some("dockerfile"));
        assert_eq!(code_lang_for_path(Path::new("repo/Cargo.toml")),    Some("toml"));
        assert_eq!(code_lang_for_path(Path::new("pyproject.toml")),     Some("toml"));
        assert_eq!(code_lang_for_path(Path::new("repo/package.json")),  Some("json"));
        assert_eq!(code_lang_for_path(Path::new("tsconfig.json")),      Some("json"));
        assert_eq!(code_lang_for_path(Path::new("go.mod")),             Some("go-mod"));
        assert_eq!(code_lang_for_path(Path::new("pom.xml")),            Some("xml"));
        assert_eq!(code_lang_for_path(Path::new("build.gradle")),       Some("groovy"));
    }

    #[test]
    fn tier2_extension_fallback() {
        assert_eq!(code_lang_for_path(Path::new("k8s/deploy.yaml")),    Some("yaml"));
        assert_eq!(code_lang_for_path(Path::new("k8s/deploy.yml")),     Some("yaml"));
        assert_eq!(code_lang_for_path(Path::new("foo/bar.toml")),       Some("toml"));
        assert_eq!(code_lang_for_path(Path::new("foo/bar.json")),       Some("json"));
        assert_eq!(code_lang_for_path(Path::new("foo/bar.xml")),        Some("xml"));
        assert_eq!(code_lang_for_path(Path::new("foo/bar.gradle")),     Some("groovy"));
    }
}
