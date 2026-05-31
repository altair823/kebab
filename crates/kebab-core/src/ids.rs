//! Newtype IDs (§3.1) + ID generation recipe (§4.2).
//!
//! Every ID is `blake3(canonical_json(tuple))[..32]`. `Display` returns the
//! inner hex string; `FromStr` accepts 32 hex characters (mixed case) and
//! normalizes the stored representation to lowercase so equality and hashing
//! are canonical.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::asset::WorkspacePath;
use crate::document::SourceSpan;
use crate::errors::CoreError;
use crate::versions::{
    ChunkerVersion, EmbeddingModelId, EmbeddingVersion, IndexVersion, ParserVersion,
};

macro_rules! newtype_id {
    ($name:ident) => {
        #[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
        pub struct $name(pub String);

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl FromStr for $name {
            type Err = CoreError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                validate_hex32(s)?;
                Ok(Self(s.to_ascii_lowercase()))
            }
        }
    };
}

newtype_id!(AssetId);
newtype_id!(DocumentId);
newtype_id!(BlockId);
newtype_id!(ChunkId);
newtype_id!(EmbeddingId);
newtype_id!(IndexId);

fn validate_hex32(s: &str) -> Result<(), CoreError> {
    if s.len() != 32 {
        return Err(CoreError::InvalidId(format!(
            "expected 32 hex chars, got {}",
            s.len()
        )));
    }
    if !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(CoreError::InvalidId(format!("non-hex character in {s:?}")));
    }
    Ok(())
}

/// Suffix appended to a chunk's vector ID to mark an alias embedding row.
pub const ALIAS_SUFFIX: &str = "#alias";

/// Strip the alias marker from `id`, returning the bare chunk ID.
///
/// Returns everything before the first occurrence of `ALIAS_SUFFIX`. This
/// handles both the suffix form `{orig}#alias` and the per-alias form
/// `{orig}#alias#N`. A bare chunk ID is blake3 hex (32 chars, no `#`), so the
/// first `#alias` always marks the boundary. If `id` contains no `ALIAS_SUFFIX`,
/// returns `id` unchanged.
pub fn strip_alias_suffix(id: &str) -> &str {
    match id.find(ALIAS_SUFFIX) {
        Some(pos) => &id[..pos],
        None => id,
    }
}

/// Canonical-JSON + blake3 + hex prefix 32. Per design §4.2.
pub fn id_from<T: Serialize>(tuple: T) -> String {
    let bytes = serde_json_canonicalizer::to_vec(&tuple)
        .expect("canonical JSON serialization must not fail for kb-core inputs");
    // The crate exposes `to_vec` for `T: Serialize` returning `Vec<u8>`.
    let hex = blake3::hash(&bytes).to_hex().to_string();
    hex[..32].to_string()
}

#[derive(Serialize)]
struct AssetTuple<'a> {
    kind: &'static str,
    asset_blake3: &'a str,
}

#[derive(Serialize)]
struct DocTuple<'a> {
    kind: &'static str,
    workspace_path: &'a str,
    asset_id: &'a str,
    parser_version: &'a str,
}

#[derive(Serialize)]
struct BlockTuple<'a> {
    kind: &'static str,
    doc_id: &'a str,
    block_kind: &'a str,
    heading_path: &'a [String],
    ordinal: u32,
    source_span: &'a SourceSpan,
}

#[derive(Serialize)]
struct ChunkTuple<'a> {
    kind: &'static str,
    doc_id: &'a str,
    chunker_version: &'a str,
    block_ids: Vec<&'a str>,
    policy_hash: &'a str,
}

#[derive(Serialize)]
struct EmbeddingTuple<'a> {
    kind: &'static str,
    chunk_id: &'a str,
    model_id: &'a str,
    model_version: &'a str,
    dimensions: usize,
}

#[derive(Serialize)]
struct IndexTuple<'a> {
    kind: &'static str,
    collection: &'a str,
    embedding_model: &'a str,
    dimensions: usize,
    index_version: &'a str,
    index_kind: &'a str,
    index_params_hash: &'a str,
}

pub fn id_for_asset(asset_blake3_full_hex: &str) -> AssetId {
    AssetId(id_from(AssetTuple {
        kind: "asset",
        asset_blake3: asset_blake3_full_hex,
    }))
}

pub fn id_for_doc(
    workspace_path: &WorkspacePath,
    asset: &AssetId,
    parser_version: &ParserVersion,
) -> DocumentId {
    DocumentId(id_from(DocTuple {
        kind: "doc",
        workspace_path: &workspace_path.0,
        asset_id: &asset.0,
        parser_version: &parser_version.0,
    }))
}

pub fn id_for_block(
    doc: &DocumentId,
    block_kind: &str,
    heading_path: &[String],
    ordinal: u32,
    span: &SourceSpan,
) -> BlockId {
    BlockId(id_from(BlockTuple {
        kind: "block",
        doc_id: &doc.0,
        block_kind,
        heading_path,
        ordinal,
        source_span: span,
    }))
}

pub fn id_for_chunk(
    doc: &DocumentId,
    chunker_version: &ChunkerVersion,
    block_ids: &[BlockId],
    policy_hash: &str,
) -> ChunkId {
    ChunkId(id_from(ChunkTuple {
        kind: "chunk",
        doc_id: &doc.0,
        chunker_version: &chunker_version.0,
        block_ids: block_ids.iter().map(|b| b.0.as_str()).collect(),
        policy_hash,
    }))
}

pub fn id_for_embedding(
    chunk: &ChunkId,
    model: &EmbeddingModelId,
    version: &EmbeddingVersion,
    dims: usize,
) -> EmbeddingId {
    EmbeddingId(id_from(EmbeddingTuple {
        kind: "embedding",
        chunk_id: &chunk.0,
        model_id: &model.0,
        model_version: &version.0,
        dimensions: dims,
    }))
}

pub fn id_for_index(
    collection: &str,
    model: &EmbeddingModelId,
    dims: usize,
    version: &IndexVersion,
    kind: &str,
    params_hash: &str,
) -> IndexId {
    IndexId(id_from(IndexTuple {
        kind: "index",
        collection,
        embedding_model: &model.0,
        dimensions: dims,
        index_version: &version.0,
        index_kind: kind,
        index_params_hash: params_hash,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newtype_display_roundtrip() {
        let s = "0123456789abcdef0123456789abcdef";
        let id: AssetId = s.parse().unwrap();
        assert_eq!(id.to_string(), s);
    }

    #[test]
    fn newtype_rejects_short() {
        let r: Result<AssetId, _> = "abc".parse();
        assert!(r.is_err());
    }

    #[test]
    fn newtype_rejects_non_hex() {
        let r: Result<AssetId, _> = "ZZZ456789abcdef0123456789abcdef0".parse();
        assert!(r.is_err());
    }

    #[test]
    fn newtype_accepts_uppercase_normalizes_to_lowercase() {
        let r: Result<AssetId, _> = "0123456789ABCDEF0123456789ABCDEF".parse();
        let id = r.expect("uppercase hex must be accepted");
        assert_eq!(id.0, "0123456789abcdef0123456789abcdef");
        assert_eq!(id.to_string(), "0123456789abcdef0123456789abcdef");
    }

    #[test]
    fn newtype_rejects_invalid_chars_after_uppercase_pass() {
        // Mix of upper-hex (would pass) and non-hex `XYZ` (must reject).
        let r: Result<AssetId, _> = "DEADBEEFCAFEBAB1XYZ23456789ABCD0".parse();
        assert!(r.is_err());
    }

    /// Determinism: 1000 runs of `id_from` over the same input yield the same
    /// hex.
    #[test]
    fn id_from_deterministic_1000() {
        #[derive(Serialize)]
        struct T<'a> {
            a: u32,
            b: &'a str,
        }
        let input = T { a: 7, b: "hello" };
        let first = id_from(&input);
        for _ in 0..1000 {
            assert_eq!(id_from(&input), first);
        }
        assert_eq!(first.len(), 32);
    }

    /// Key order in the source struct does not affect hash (canonical JSON
    /// sorts keys alphabetically).
    #[test]
    fn id_from_key_order_invariant() {
        #[derive(Serialize)]
        struct A {
            a: u32,
            b: u32,
        }
        #[derive(Serialize)]
        struct B {
            b: u32,
            a: u32,
        }
        assert_eq!(id_from(A { a: 1, b: 2 }), id_from(B { b: 2, a: 1 }));
    }

    /// The expected hex below is hand-computed via design §4.2:
    ///   tuple = { "kind": "asset", "asset_blake3": "deadbeef" }
    ///   canonical JSON (key-sorted, no whitespace, both keys are pure ASCII):
    ///       {"asset_blake3":"deadbeef","kind":"asset"}
    ///   blake3 of those bytes → hex → first 32 chars.
    /// Pinned via an independent tool (b3sum, computed once outside the code
    /// under test) so a regression in our JCS or hash pipeline is caught.
    #[test]
    fn id_for_asset_pinned() {
        // printf '{"asset_blake3":"deadbeef","kind":"asset"}' | b3sum
        //   → cec9353553efb238a7919d38d3e148f1...
        let id = id_for_asset("deadbeef");
        assert_eq!(id.0, "cec9353553efb238a7919d38d3e148f1");
    }

    /// Independent pin for id_for_doc.
    /// canonical JSON:
    ///   {"asset_id":"6cb0ef0eb89c63b8b6e76ec53dca6e7d",
    ///    "kind":"doc",
    ///    "parser_version":"pulldown-cmark-0.x",
    ///    "workspace_path":"notes/test.md"}
    /// (concatenated, no whitespace).
    #[test]
    fn id_for_doc_pinned() {
        let asset = AssetId("6cb0ef0eb89c63b8b6e76ec53dca6e7d".to_string());
        let path = WorkspacePath::new("notes/test.md".to_string()).unwrap();
        let pv = ParserVersion("pulldown-cmark-0.x".to_string());
        let id = id_for_doc(&path, &asset, &pv);
        assert_eq!(id.0, "8547fe58cb42d593fd761d77242401db");
    }

    /// Independent pin for id_for_block.
    /// inputs:
    ///   doc=DocumentId("aabbccdd00112233445566778899aabb"),
    ///   block_kind="paragraph", heading_path=["Intro"], ordinal=3,
    ///   span=SourceSpan::Line { start: 10, end: 20 }
    /// canonical JSON (key-sorted, compact, no whitespace):
    ///   {"block_kind":"paragraph",
    ///    "doc_id":"aabbccdd00112233445566778899aabb",
    ///    "heading_path":["Intro"],
    ///    "kind":"block",
    ///    "ordinal":3,
    ///    "source_span":{"end":20,"kind":"line","start":10}}
    /// computed via:
    ///   printf '{"block_kind":"paragraph","doc_id":"aabbccdd00112233445566778899aabb","heading_path":["Intro"],"kind":"block","ordinal":3,"source_span":{"end":20,"kind":"line","start":10}}' \
    ///     | ~/.cargo/bin/b3sum --no-names | cut -c1-32
    ///   → 8a7bf22de7ec3293a792028c829b3812
    #[test]
    fn id_for_block_pinned() {
        let doc = DocumentId("aabbccdd00112233445566778899aabb".to_string());
        let heading = vec!["Intro".to_string()];
        let span = SourceSpan::Line { start: 10, end: 20 };

        // Sanity check: confirm that the canonical JSON our code produces
        // matches the literal we hashed externally. If a future field-order
        // change (or rename) silently shifts the hash, this assertion fails
        // before the hex comparison and points at the JSON layer directly.
        let expected_json = b"{\"block_kind\":\"paragraph\",\"doc_id\":\"aabbccdd00112233445566778899aabb\",\"heading_path\":[\"Intro\"],\"kind\":\"block\",\"ordinal\":3,\"source_span\":{\"end\":20,\"kind\":\"line\",\"start\":10}}";
        let tuple = BlockTuple {
            kind: "block",
            doc_id: &doc.0,
            block_kind: "paragraph",
            heading_path: &heading,
            ordinal: 3,
            source_span: &span,
        };
        assert_eq!(
            serde_json_canonicalizer::to_vec(&tuple).unwrap(),
            expected_json
        );

        let id = id_for_block(&doc, "paragraph", &heading, 3, &span);
        assert_eq!(id.0, "8a7bf22de7ec3293a792028c829b3812");
    }

    /// Independent pin for id_for_chunk.
    /// inputs:
    ///   doc=DocumentId("aabbccdd00112233445566778899aabb"),
    ///   chunker_version=ChunkerVersion("greedy-1.0"),
    ///   block_ids=[BlockId("a1b2c3d4e5f6789012345678abcdef00")],
    ///   policy_hash="abc123"
    /// canonical JSON (key-sorted, compact, no whitespace):
    ///   {"block_ids":["a1b2c3d4e5f6789012345678abcdef00"],
    ///    "chunker_version":"greedy-1.0",
    ///    "doc_id":"aabbccdd00112233445566778899aabb",
    ///    "kind":"chunk",
    ///    "policy_hash":"abc123"}
    /// computed via:
    ///   printf '{"block_ids":["a1b2c3d4e5f6789012345678abcdef00"],"chunker_version":"greedy-1.0","doc_id":"aabbccdd00112233445566778899aabb","kind":"chunk","policy_hash":"abc123"}' \
    ///     | ~/.cargo/bin/b3sum --no-names | cut -c1-32
    ///   → 8809f627777fe7ca5c4433b97dd88ce9
    #[test]
    fn id_for_chunk_pinned() {
        let doc = DocumentId("aabbccdd00112233445566778899aabb".to_string());
        let cv = ChunkerVersion("greedy-1.0".to_string());
        let blocks = vec![BlockId("a1b2c3d4e5f6789012345678abcdef00".to_string())];

        let expected_json = b"{\"block_ids\":[\"a1b2c3d4e5f6789012345678abcdef00\"],\"chunker_version\":\"greedy-1.0\",\"doc_id\":\"aabbccdd00112233445566778899aabb\",\"kind\":\"chunk\",\"policy_hash\":\"abc123\"}";
        let tuple = ChunkTuple {
            kind: "chunk",
            doc_id: &doc.0,
            chunker_version: &cv.0,
            block_ids: blocks.iter().map(|b| b.0.as_str()).collect(),
            policy_hash: "abc123",
        };
        assert_eq!(
            serde_json_canonicalizer::to_vec(&tuple).unwrap(),
            expected_json
        );

        let id = id_for_chunk(&doc, &cv, &blocks, "abc123");
        assert_eq!(id.0, "8809f627777fe7ca5c4433b97dd88ce9");
    }

    /// Independent pin for id_for_embedding.
    /// inputs:
    ///   chunk=ChunkId("d1e2f3a4b5c6789012345678aabbccdd"),
    ///   model_id=EmbeddingModelId("BAAI/bge-small-en"),
    ///   model_version=EmbeddingVersion("v1"), dimensions=384
    /// canonical JSON (key-sorted, compact, no whitespace):
    ///   {"chunk_id":"d1e2f3a4b5c6789012345678aabbccdd",
    ///    "dimensions":384,
    ///    "kind":"embedding",
    ///    "model_id":"BAAI/bge-small-en",
    ///    "model_version":"v1"}
    /// computed via:
    ///   printf '{"chunk_id":"d1e2f3a4b5c6789012345678aabbccdd","dimensions":384,"kind":"embedding","model_id":"BAAI/bge-small-en","model_version":"v1"}' \
    ///     | ~/.cargo/bin/b3sum --no-names | cut -c1-32
    ///   → 71992c457a5da39880a6d17d646ed0fd
    #[test]
    fn id_for_embedding_pinned() {
        let chunk = ChunkId("d1e2f3a4b5c6789012345678aabbccdd".to_string());
        let model = EmbeddingModelId("BAAI/bge-small-en".to_string());
        let version = EmbeddingVersion("v1".to_string());

        let expected_json = b"{\"chunk_id\":\"d1e2f3a4b5c6789012345678aabbccdd\",\"dimensions\":384,\"kind\":\"embedding\",\"model_id\":\"BAAI/bge-small-en\",\"model_version\":\"v1\"}";
        let tuple = EmbeddingTuple {
            kind: "embedding",
            chunk_id: &chunk.0,
            model_id: &model.0,
            model_version: &version.0,
            dimensions: 384,
        };
        assert_eq!(
            serde_json_canonicalizer::to_vec(&tuple).unwrap(),
            expected_json
        );

        let id = id_for_embedding(&chunk, &model, &version, 384);
        assert_eq!(id.0, "71992c457a5da39880a6d17d646ed0fd");
    }

    #[test]
    fn strip_alias_suffix_roundtrip() {
        let bare = "0123456789abcdef0123456789abcdef";
        let with_suffix = format!("{bare}{ALIAS_SUFFIX}");
        assert_eq!(strip_alias_suffix(&with_suffix), bare);
        assert_eq!(strip_alias_suffix(bare), bare);
        assert_eq!(strip_alias_suffix(""), "");
        assert_eq!(strip_alias_suffix("#alias"), "");
        // Per-alias form `{orig}#alias#N` strips to the bare chunk ID.
        assert_eq!(strip_alias_suffix(&format!("{bare}{ALIAS_SUFFIX}#3")), bare);
        assert_eq!(strip_alias_suffix(&format!("{bare}{ALIAS_SUFFIX}#0")), bare);
        assert_eq!(strip_alias_suffix("#alias#3"), "");
    }

    /// Independent pin for id_for_index.
    /// inputs:
    ///   collection="default",
    ///   embedding_model=EmbeddingModelId("BAAI/bge-small-en"),
    ///   dimensions=384, version=IndexVersion("v1"),
    ///   kind="hnsw", params_hash="xyz"
    /// canonical JSON (key-sorted, compact, no whitespace):
    ///   {"collection":"default",
    ///    "dimensions":384,
    ///    "embedding_model":"BAAI/bge-small-en",
    ///    "index_kind":"hnsw",
    ///    "index_params_hash":"xyz",
    ///    "index_version":"v1",
    ///    "kind":"index"}
    /// computed via:
    ///   printf '{"collection":"default","dimensions":384,"embedding_model":"BAAI/bge-small-en","index_kind":"hnsw","index_params_hash":"xyz","index_version":"v1","kind":"index"}' \
    ///     | ~/.cargo/bin/b3sum --no-names | cut -c1-32
    ///   → e733ee2f9936f0e1ac5143cdbf0f2b54
    #[test]
    fn id_for_index_pinned() {
        let model = EmbeddingModelId("BAAI/bge-small-en".to_string());
        let version = IndexVersion("v1".to_string());

        let expected_json = b"{\"collection\":\"default\",\"dimensions\":384,\"embedding_model\":\"BAAI/bge-small-en\",\"index_kind\":\"hnsw\",\"index_params_hash\":\"xyz\",\"index_version\":\"v1\",\"kind\":\"index\"}";
        let tuple = IndexTuple {
            kind: "index",
            collection: "default",
            embedding_model: &model.0,
            dimensions: 384,
            index_version: &version.0,
            index_kind: "hnsw",
            index_params_hash: "xyz",
        };
        assert_eq!(
            serde_json_canonicalizer::to_vec(&tuple).unwrap(),
            expected_json
        );

        let id = id_for_index("default", &model, 384, &version, "hnsw", "xyz");
        assert_eq!(id.0, "e733ee2f9936f0e1ac5143cdbf0f2b54");
    }
}
