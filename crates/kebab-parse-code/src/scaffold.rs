//! `kebab-parse-code::scaffold` — shared pure helpers used by all
//! per-language extractor modules.
//!
//! These are `pub(crate)` utilities extracted from the four extractor
//! modules (rust / python / typescript / javascript) where identical
//! copies existed. Keeping them here is the single source of truth.

/// Extract the last path component (filename) from a `/`-separated
/// workspace path string.
/// For a path like `crates/x/src/foo.rs` this returns `foo.rs`.
pub(crate) fn filename_from_workspace_path(p: &str) -> String {
    p.rsplit('/').next().unwrap_or(p).to_string()
}

/// Strip the last dot-extension from a filename string.
/// A leading dot (hidden-file convention) is preserved as-is.
/// `foo.rs` → `foo`, `.hidden` → `.hidden`, `noext` → `noext`.
pub(crate) fn strip_extension(filename: &str) -> String {
    match filename.rfind('.') {
        Some(0) => filename.to_string(),
        Some(idx) => filename[..idx].to_string(),
        None => filename.to_string(),
    }
}

/// Join `(mod_prefix, mod_path, name)` into a dotted symbol string.
///
/// Used by Python / TypeScript / JavaScript extractors. Rust uses
/// `::` separators instead and builds symbols inline; this helper
/// covers the `.`-joined languages.
///
/// Empty `mod_prefix` (e.g. file is `__init__.py` at workspace root)
/// drops the leading prefix segment; empty `mod_path` (file top-level)
/// drops the class-nesting middle segment.
pub(crate) fn join_symbol(mod_prefix: &str, mod_path: &[String], name: &str) -> String {
    let mut parts: Vec<&str> = Vec::with_capacity(mod_path.len() + 2);
    if !mod_prefix.is_empty() {
        parts.push(mod_prefix);
    }
    for p in mod_path {
        parts.push(p.as_str());
    }
    parts.push(name);
    parts.join(".")
}
