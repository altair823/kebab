//! Snapshot test pinning the `Vec<Chunk>` JSON for a
//! representative C++ code `CanonicalDocument`.
//!
//! This is an integration test. `kebab-parse-code` is intentionally NOT
//! a dev-dep (design §6.3 / §8 boundary: AST extraction is parser-side).
//! The `CanonicalDocument` is built inline from hand-crafted `Block::Code`
//! units, which is the same pattern used in `code_c_ast_v1.rs`'s
//! internal `code_doc` test helper.
//!
//! Set `UPDATE_SNAPSHOTS=1` to re-bake the baseline.

use std::path::PathBuf;

use kebab_chunk::CodeCppAstV1Chunker;
use kebab_core::{
    AssetId, Block, CanonicalDocument, ChunkPolicy, Chunker, ChunkerVersion, CodeBlock, CommonBlock,
    Lang, Metadata, ParserVersion, Provenance, SourceSpan, SourceType, TrustLevel, WorkspacePath,
    id_for_block, id_for_doc,
};
use serde_json::Value;
use time::OffsetDateTime;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

fn fixed_doc() -> CanonicalDocument {
    let wp = WorkspacePath("projects/record.cpp".into());
    let aid = AssetId("c".repeat(64));
    // Pin parser_version so doc_id / block_ids are reproducible.
    let pv = ParserVersion("code-cpp-v1".into());
    let doc_id = id_for_doc(&wp, &aid, &pv);

    // Representative units (C++ specific):
    //  0. includes + namespace opening  (lines 1–4,   ≤200)
    //  1. class definition              (lines 6–20,  ≤200)
    //  2. template function             (lines 22–25, ≤200)
    //  3. namespace closing + free fn   (lines 27–29, ≤200)
    //  4. main fn                       (lines 31–34, ≤200)
    let raw_units: Vec<(&str, u32, u32, String)> = vec![
        (
            "<top-level>",
            1,
            4,
            "#include <string>\n#include <vector>\n\nnamespace kebab {".to_string(),
        ),
        (
            "kebab::chunk::MdHeadingV1Chunker",
            6,
            20,
            "class MdHeadingV1Chunker {\npublic:\n    MdHeadingV1Chunker() = default;\n    ~MdHeadingV1Chunker() = default;\n\n    std::string chunk_doc(const std::string& doc) {\n        return doc;\n    }\n\n    int operator()(int x) const {\n        return x * 2;\n    }\n\nprivate:\n    int counter_ = 0;\n};".to_string(),
        ),
        (
            "kebab::identity",
            22,
            25,
            "template <typename T>\nT identity(T value) {\n    return value;\n}".to_string(),
        ),
        (
            "kebab::global_helper",
            27,
            29,
            "void global_helper() {\n    // free function in kebab namespace\n}".to_string(),
        ),
        (
            "main",
            31,
            34,
            "int main() {\n    kebab::chunk::MdHeadingV1Chunker c;\n    return 0;\n}".to_string(),
        ),
    ];

    let blocks: Vec<Block> = raw_units
        .iter()
        .enumerate()
        .map(|(i, (sym, ls, le, code))| {
            let span = SourceSpan::Code {
                line_start: *ls,
                line_end: *le,
                symbol: Some((*sym).to_string()),
                lang: Some("cpp".into()),
            };
            let bid = id_for_block(&doc_id, "code", &[], i as u32, &span);
            Block::Code(CodeBlock {
                common: CommonBlock {
                    block_id: bid,
                    heading_path: vec![],
                    source_span: span,
                },
                lang: Some("cpp".into()),
                code: code.clone(),
            })
        })
        .collect();

    CanonicalDocument {
        doc_id,
        source_asset_id: aid,
        workspace_path: wp,
        title: "record.cpp".into(),
        lang: Lang("und".into()),
        blocks,
        metadata: Metadata {
            aliases: vec![],
            tags: vec![],
            created_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
            updated_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
            source_type: SourceType::Note,
            trust_level: TrustLevel::Primary,
            user_id_alias: None,
            user: Default::default(),
            repo: Some("kebab".into()),
            git_branch: Some("main".into()),
            git_commit: Some("0".repeat(40)),
            code_lang: Some("cpp".into()),
        },
        provenance: Provenance { events: vec![] },
        parser_version: pv,
        schema_version: 1,
        doc_version: 1,
        last_chunker_version: None,
        last_embedding_version: None,
    }
}

fn fixed_policy() -> ChunkPolicy {
    ChunkPolicy {
        target_tokens: 500,
        overlap_tokens: 80,
        respect_markdown_headings: false,
        chunker_version: ChunkerVersion("code-cpp-ast-v1".into()),
    }
}

#[test]
fn code_cpp_ast_chunks_snapshot() {
    let doc = fixed_doc();
    let policy = fixed_policy();

    let chunks = CodeCppAstV1Chunker.chunk(&doc, &policy).expect("chunk");
    let actual = serde_json::to_value(&chunks).unwrap();

    let dir = fixtures_dir();
    let baseline_path = dir.join("code-sample.cpp.chunks.snapshot.json");
    let baseline_text = match std::fs::read_to_string(&baseline_path) {
        Ok(s) => s,
        Err(_) if std::env::var("UPDATE_SNAPSHOTS").is_ok() => {
            std::fs::create_dir_all(&dir).unwrap();
            let pretty = serde_json::to_string_pretty(&actual).unwrap();
            std::fs::write(&baseline_path, format!("{pretty}\n")).unwrap();
            return;
        }
        Err(e) => panic!(
            "missing baseline {}; run with UPDATE_SNAPSHOTS=1 to create: {e}",
            baseline_path.display()
        ),
    };
    let expected: Value = serde_json::from_str(&baseline_text).expect("baseline parses as json");

    if actual != expected {
        if std::env::var("UPDATE_SNAPSHOTS").is_ok() {
            let pretty = serde_json::to_string_pretty(&actual).unwrap();
            std::fs::write(&baseline_path, format!("{pretty}\n")).unwrap();
            eprintln!("updated baseline {}", baseline_path.display());
            return;
        }
        let pretty = serde_json::to_string_pretty(&actual).unwrap();
        panic!(
            "code-cpp-ast-v1 chunks snapshot drift\n\
             --- expected ({}) ---\n{baseline_text}\n\
             --- actual ---\n{pretty}\n\
             If intentional, re-run with UPDATE_SNAPSHOTS=1.",
            baseline_path.display()
        );
    }
}

/// Determinism cross-check: re-running the same pipeline yields the same
/// chunk_ids byte-for-byte.
#[test]
fn code_cpp_ast_chunks_are_deterministic() {
    let policy = fixed_policy();
    let baseline: Vec<String> = CodeCppAstV1Chunker
        .chunk(&fixed_doc(), &policy)
        .unwrap()
        .into_iter()
        .map(|c| c.chunk_id.0)
        .collect();
    for _ in 0..5 {
        let again: Vec<String> = CodeCppAstV1Chunker
            .chunk(&fixed_doc(), &policy)
            .unwrap()
            .into_iter()
            .map(|c| c.chunk_id.0)
            .collect();
        assert_eq!(again, baseline);
    }
}
