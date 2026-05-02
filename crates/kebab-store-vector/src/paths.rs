//! Path expansion + table-name sanitization.
//!
//! `expand_path` lives in `kb-config` so `kb-store-vector`,
//! `kb-store-sqlite`, `kb-embed-local`, and `kb-eval` all resolve
//! `${XDG_DATA_HOME:-…}` / leading `~` / `{data_dir}` identically. This
//! module re-exports nothing; consumers within the crate `use
//! kebab_config::expand_path` directly.

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
}
