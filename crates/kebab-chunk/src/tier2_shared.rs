//! p10-2: Tier 2 chunker shared helpers (oversize fallback + Chunk build).
//!
//! Mirrors `code_rust_ast_v1`'s Chunk-construction pattern exactly so that
//! id / hashes / token-count / ChunkPolicy semantics stay identical across
//! Tier 1 (AST) and Tier 2 (resource-aware) chunkers.

use anyhow::Result;
use kebab_core::{
    BlockId, CanonicalDocument, Chunk, ChunkPolicy, ChunkerVersion, DocumentId, SourceSpan,
    id_for_chunk,
};

pub(crate) const AST_CHUNK_MAX_LINES: u32 = 200;
const BYTES_PER_TOKEN: usize = 3;
const POLICY_HASH_HEX_LEN: usize = 16;

/// Compute the policy hash the same way `code_rust_ast_v1` does.
pub(crate) fn policy_hash(policy: &ChunkPolicy) -> String {
    let bytes = serde_json_canonicalizer::to_vec(policy)
        .expect("canonical JSON serialization of ChunkPolicy must not fail");
    let hex = blake3::hash(&bytes).to_hex().to_string();
    hex[..POLICY_HASH_HEX_LEN].to_string()
}

/// Emit one chunk for `(text, line_start..=line_end, symbol, lang)`, splitting
/// into line-windows of at most `AST_CHUNK_MAX_LINES` if the slice is oversize.
/// Mirrors the oversize path in `code_rust_ast_v1`'s `chunk` impl.
///
/// `base_split_key` is used as the `split_key` for the non-oversize single-chunk
/// case. Callers that emit multiple chunks from the same document (e.g.
/// `K8sManifestResourceV1Chunker` — one call per k8s resource) MUST pass
/// `Some(line_start)` so that each call produces a distinct `chunk_id`.
/// Single-chunk callers (dockerfile-file-v1, manifest-file-v1) pass `None` to
/// keep chunk_ids stable (no sibling can collide when there's only one chunk).
#[allow(clippy::too_many_arguments)]
pub(crate) fn push_chunks_with_oversize(
    out: &mut Vec<Chunk>,
    doc: &CanonicalDocument,
    policy: &ChunkPolicy,
    text: &str,
    line_start: u32,
    line_end: u32,
    symbol: &str,
    lang: &str,
    chunker_version: &str,
    base_split_key: Option<u32>,
) -> Result<()> {
    let n_lines = (line_end - line_start + 1).max(1);
    let cv = ChunkerVersion(chunker_version.to_string());
    let base_policy_hash = policy_hash(policy);

    if n_lines <= AST_CHUNK_MAX_LINES {
        out.push(build_chunk(
            doc,
            &cv,
            &base_policy_hash,
            text,
            line_start,
            line_end,
            symbol,
            lang,
            base_split_key,
        ));
        return Ok(());
    }

    let lines: Vec<&str> = text.lines().collect();
    let total = lines.len();
    let mut window_start = line_start;
    let mut i = 0usize;
    while i < total {
        let take = (AST_CHUNK_MAX_LINES as usize).min(total - i);
        let window_text = lines[i..i + take].join("\n");
        let window_end = window_start + take as u32 - 1;
        out.push(build_chunk(
            doc,
            &cv,
            &base_policy_hash,
            &window_text,
            window_start,
            window_end,
            symbol,
            lang,
            Some(window_start),
        ));
        i += take;
        window_start = window_end + 1;
    }
    Ok(())
}

/// Build a single `Chunk`, mirroring `make_chunk` in `code_rust_ast_v1.rs`
/// exactly (same id recipe, same token estimate, same field set).
///
/// `split_key` is `Some(line_start_of_window)` for oversize splits, `None`
/// for normal single-chunk emission.  Mirrors the `Some(part_ls)` / `None`
/// split_key pattern in 1A-2.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_chunk(
    doc: &CanonicalDocument,
    chunker_version: &ChunkerVersion,
    base_policy_hash: &str,
    text: &str,
    line_start: u32,
    line_end: u32,
    symbol: &str,
    lang: &str,
    split_key: Option<u32>,
) -> Chunk {
    let span = SourceSpan::Code {
        line_start,
        line_end,
        symbol: Some(symbol.to_string()),
        lang: Some(lang.to_string()),
    };
    build_chunk_from_span(doc, chunker_version, base_policy_hash, text, span, split_key)
}

/// Like `build_chunk` but emits `symbol: None`. Used by Tier 3 (per spec §9.3).
///
/// Accepts `policy: &ChunkPolicy` and `chunker_version: &str` (string slice)
/// so callers don't need to pre-compute the hash and version wrapper.
/// `split_key` is `Some(window_start)` for oversize line-window splits.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_chunk_no_symbol(
    doc: &CanonicalDocument,
    policy: &ChunkPolicy,
    text: &str,
    line_start: u32,
    line_end: u32,
    lang: &str,
    chunker_version: &str,
    split_key: Option<u32>,
) -> Chunk {
    let cv = ChunkerVersion(chunker_version.to_string());
    let base_policy_hash = policy_hash(policy);
    let span = SourceSpan::Code {
        line_start,
        line_end,
        symbol: None,
        lang: Some(lang.to_string()),
    };
    build_chunk_from_span(doc, &cv, &base_policy_hash, text, span, split_key)
}

/// Core chunk-building logic shared by `build_chunk` and `build_chunk_no_symbol`.
///
/// Takes a pre-built `SourceSpan` so the only difference between the two
/// public helpers is whether `symbol` is `Some` or `None`.  All id/hash/
/// token mechanics are identical.
fn build_chunk_from_span(
    doc: &CanonicalDocument,
    chunker_version: &ChunkerVersion,
    base_policy_hash: &str,
    text: &str,
    span: SourceSpan,
    split_key: Option<u32>,
) -> Chunk {
    // id_hash mirrors code_rust_ast_v1's make_chunk logic:
    //   split_key Some(k) => "{base_policy_hash}#L{k}"
    //   split_key None    => base_policy_hash
    let id_hash = match split_key {
        Some(k) => format!("{base_policy_hash}#L{k}"),
        None => base_policy_hash.to_string(),
    };

    // block_ids: Tier 2/3 chunkers have no per-block structure (the whole file
    // is one Block::Code), so we pass an empty slice — same as using the doc-
    // level slice without explicit block granularity.
    let block_ids: Vec<BlockId> = vec![];

    let chunk_id = id_for_chunk(
        &DocumentId(doc.doc_id.0.clone()),
        chunker_version,
        &block_ids,
        &id_hash,
    );

    let token_estimate = text.len().div_ceil(BYTES_PER_TOKEN);

    Chunk {
        chunk_id,
        doc_id: DocumentId(doc.doc_id.0.clone()),
        block_ids,
        text: text.to_string(),
        heading_path: Vec::new(),
        source_spans: vec![span],
        token_estimate,
        chunker_version: chunker_version.clone(),
        policy_hash: base_policy_hash.to_string(),
    }
}
