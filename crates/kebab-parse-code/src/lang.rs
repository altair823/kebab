//! Workspace-relative path → module-path conversion for P10-1B AST extractors
//! (Python dotted form / TS+JS slash form). 본 module 의 `code_lang_for_path`
//! 는 v0.18.0+ 부터 `kebab-source-fs::code_meta` 로 이동.

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
}
