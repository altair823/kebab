//! Path expansion + table-name sanitization.
//!
//! Mirrors `kb-store-sqlite::store::expand_data_dir` and
//! `kb-embed-local::expand_path` so the three crates resolve
//! `${XDG_DATA_HOME:-…}` / leading `~` / `{data_dir}` identically. A
//! shared helper would live in `kb-config`, but the task spec forbids
//! adding new types to `kb-config`, so we keep a private clone.

use std::path::PathBuf;

/// Expand `{data_dir}` → `data_dir`, `${XDG_DATA_HOME:-…}` → env or
/// default, leading `~` → `$HOME`. Pass an empty `data_dir` when
/// resolving `data_dir` itself (the `{data_dir}` substitution is a
/// no-op in that case).
pub(crate) fn expand_path(raw: &str, data_dir: &str) -> PathBuf {
    let mut s = raw.to_string();

    if !data_dir.is_empty() {
        s = s.replace("{data_dir}", data_dir);
    }

    // ${XDG_DATA_HOME:-~/.local/share}: env override, else default after `:-`.
    if let Some(start) = s.find("${XDG_DATA_HOME") {
        if let Some(rel_end) = s[start..].find('}') {
            let end = start + rel_end + 1;
            let inner = &s[start + 2..end - 1];
            let replacement = match std::env::var("XDG_DATA_HOME") {
                Ok(v) if !v.is_empty() => v,
                _ => {
                    if let Some((_, default)) = inner.split_once(":-") {
                        default.to_string()
                    } else {
                        String::new()
                    }
                }
            };
            s.replace_range(start..end, &replacement);
        }
    }

    if let Some(rest) = s.strip_prefix('~') {
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            return home.join(rest.trim_start_matches('/'));
        }
    }

    PathBuf::from(s)
}

/// Build the per-model Lance table name. Per design §6.3:
/// `chunk_embeddings_<model>_<dim>.lance`. Model IDs may contain
/// characters that are illegal in directory names on some filesystems
/// (Windows reserved chars, `/`, …) — squash anything outside
/// `[A-Za-z0-9-]` to `_` so the name is portable.
///
/// LanceDB's `connect(uri).open_table(name)` resolves `name` against
/// the connection root; the trailing `.lance` is part of the directory
/// LanceDB itself appends when it materializes the table, so we pass
/// the bare logical name (`chunk_embeddings_<model>_<dim>`) and let
/// Lance manage the suffix. Spec text uses the suffixed form for the
/// on-disk path; both are present.
pub(crate) fn lance_table_name(model_id: &str, dim: usize) -> String {
    let sanitized = sanitize_model_id(model_id);
    format!("chunk_embeddings_{sanitized}_{dim}")
}

/// Replace anything outside `[A-Za-z0-9-]` with `_`. Idempotent.
pub(crate) fn sanitize_model_id(model_id: &str) -> String {
    model_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_replaces_path_separators() {
        assert_eq!(sanitize_model_id("BAAI/bge-small-en"), "BAAI_bge-small-en");
    }

    #[test]
    fn sanitize_keeps_dash_and_alpha_num() {
        assert_eq!(sanitize_model_id("e5-small-v2"), "e5-small-v2");
    }

    #[test]
    fn sanitize_squashes_dot_and_colon() {
        assert_eq!(sanitize_model_id("model.v1:fast"), "model_v1_fast");
    }

    #[test]
    fn lance_table_name_format() {
        assert_eq!(
            lance_table_name("BAAI/bge-small-en", 384),
            "chunk_embeddings_BAAI_bge-small-en_384"
        );
    }

    #[test]
    fn expand_path_substitutes_data_dir() {
        let p = expand_path("{data_dir}/lancedb", "/tmp/kbtest");
        assert_eq!(p, PathBuf::from("/tmp/kbtest/lancedb"));
    }

    #[test]
    fn expand_path_passthrough_absolute() {
        let p = expand_path("/abs/dir", "/ignored");
        assert_eq!(p, PathBuf::from("/abs/dir"));
    }
}
