//! Snapshot test pinning the `Vec<Chunk>` JSON for a
//! representative Kotlin code `CanonicalDocument`.
//!
//! This is an integration test. `kebab-parse-code` is intentionally NOT
//! a dev-dep (design §6.3 / §8 boundary: AST extraction is parser-side).
//! The `CanonicalDocument` is built inline from hand-crafted `Block::Code`
//! units, which is the same pattern used in `code_rust_ast_v1.rs`'s
//! internal `code_doc` test helper.
//!
//! Set `UPDATE_SNAPSHOTS=1` to re-bake the baseline.

use std::path::PathBuf;

use kebab_chunk::CodeKotlinAstV1Chunker;
use kebab_core::{
    AssetId, Block, CanonicalDocument, ChunkPolicy, Chunker, ChunkerVersion, CodeBlock,
    CommonBlock, Lang, Metadata, ParserVersion, Provenance, SourceSpan, SourceType, TrustLevel,
    WorkspacePath, id_for_block, id_for_doc,
};
use serde_json::Value;
use time::OffsetDateTime;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

fn fixed_doc() -> CanonicalDocument {
    let wp = WorkspacePath("src/main/kotlin/com/example/Metrics.kt".into());
    let aid = AssetId("b".repeat(64));
    // Pin parser_version so doc_id / block_ids are reproducible.
    let pv = ParserVersion("code-kotlin-v1".into());
    let doc_id = id_for_doc(&wp, &aid, &pv);

    // Build a >200-line function body to force split_oversize.
    let big_body: String = {
        let header = "class BigCompute {\n    fun compute(data: IntArray): Int {\n";
        let body: String = (0..210u32)
            .map(|i| format!("        val v{i} = if ({i} < data.size) data[{i}] else 0\n"))
            .collect();
        let footer = "        return data.size\n    }\n}";
        format!("{header}{body}{footer}")
    };
    let big_line_count = big_body.lines().count() as u32;
    let big_line_end = 48 + big_line_count - 1;

    // Representative units:
    //  0. import block                      (lines 1–5,   ≤200)
    //  1. top-level fn `computeMRR`         (lines 7–12,  ≤200)
    //  2. data class `MetricsCollector`     (lines 14–20, ≤200)
    //  3. class `BaseEvaluator`             (lines 22–30, ≤200)
    //  4. method `MetricsCollector.run`     (lines 32–38, ≤200)
    //  5. method `MetricsCollector.report`  (lines 40–46, ≤200)
    //  6. BigCompute (>200 lines)           to force split_oversize
    let raw_units: Vec<(&str, u32, u32, String)> = vec![
        (
            "imports",
            1,
            5,
            "import kotlin.collections.List\nimport kotlin.collections.Map\nimport kotlin.collections.MutableList\nimport kotlin.collections.MutableMap\nimport kotlin.collections.mutableListOf".to_string(),
        ),
        (
            "computeMRR",
            7,
            12,
            "fun computeMRR(scores: List<Double>): Double {\n    if (scores.isEmpty()) {\n        return 0.0\n    }\n    return 1.0 / scores.size\n}".to_string(),
        ),
        (
            "MetricsCollector",
            14,
            20,
            "data class MetricsCollector(\n    val scores: MutableList<Double> = mutableListOf(),\n    val labels: MutableList<String> = mutableListOf(),\n    val counts: MutableMap<String, Int> = mutableMapOf(),\n    val totals: MutableMap<String, Double> = mutableMapOf(),\n    val tags: MutableList<String> = mutableListOf(),\n)".to_string(),
        ),
        (
            "BaseEvaluator",
            22,
            30,
            "open class BaseEvaluator(val name: String) {\n\n    fun evaluate(data: List<String>) {\n        val joined = data.joinToString(\",\")\n        println(joined)\n    }\n\n    open fun describe(): String = name\n}".to_string(),
        ),
        (
            "MetricsCollector.run",
            32,
            38,
            "fun MetricsCollector.run(inputs: List<Double>) {\n    for (inp in inputs) {\n        scores.add(\n            inp\n        )\n    }\n}".to_string(),
        ),
        (
            "MetricsCollector.report",
            40,
            46,
            "fun MetricsCollector.report(): Map<String, Any> {\n    return mapOf(\n        \"mean\" to 0.0,\n        \"count\" to scores.size,\n        \"tags\" to tags,\n    )\n}".to_string(),
        ),
        ("BigCompute", 48, big_line_end, big_body),
    ];

    let blocks: Vec<Block> = raw_units
        .iter()
        .enumerate()
        .map(|(i, (sym, ls, le, code))| {
            let span = SourceSpan::Code {
                line_start: *ls,
                line_end: *le,
                symbol: Some((*sym).to_string()),
                lang: Some("kotlin".into()),
            };
            let bid = id_for_block(&doc_id, "code", &[], i as u32, &span);
            Block::Code(CodeBlock {
                common: CommonBlock {
                    block_id: bid,
                    heading_path: vec![],
                    source_span: span,
                },
                lang: Some("kotlin".into()),
                code: code.clone(),
            })
        })
        .collect();

    CanonicalDocument {
        doc_id,
        source_asset_id: aid,
        workspace_path: wp,
        title: "Metrics.kt".into(),
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
            code_lang: Some("kotlin".into()),
            source_id: None,
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
        chunker_version: ChunkerVersion("code-kotlin-ast-v1".into()),
    }
}

#[test]
fn code_kotlin_ast_chunks_snapshot() {
    let doc = fixed_doc();
    let policy = fixed_policy();

    let chunks = CodeKotlinAstV1Chunker.chunk(&doc, &policy).expect("chunk");
    let actual = serde_json::to_value(&chunks).unwrap();

    let dir = fixtures_dir();
    let baseline_path = dir.join("code-sample.kt.chunks.snapshot.json");
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
            "code-kotlin-ast-v1 chunks snapshot drift\n\
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
fn code_kotlin_ast_chunks_are_deterministic() {
    let policy = fixed_policy();
    let baseline: Vec<String> = CodeKotlinAstV1Chunker
        .chunk(&fixed_doc(), &policy)
        .unwrap()
        .into_iter()
        .map(|c| c.chunk_id.0)
        .collect();
    for _ in 0..5 {
        let again: Vec<String> = CodeKotlinAstV1Chunker
            .chunk(&fixed_doc(), &policy)
            .unwrap()
            .into_iter()
            .map(|c| c.chunk_id.0)
            .collect();
        assert_eq!(again, baseline);
    }
}
