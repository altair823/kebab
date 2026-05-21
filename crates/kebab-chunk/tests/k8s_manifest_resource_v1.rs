//! Behavioural tests for `K8sManifestResourceV1Chunker`.
//!
//! Documents are constructed manually (no kebab-parse-code dependency) by
//! placing the raw YAML text into a single `Block::Code`, mirroring the
//! pattern used in `code_rust_ast_snapshot.rs`.

use std::path::PathBuf;

use kebab_chunk::K8sManifestResourceV1Chunker;
use kebab_core::{
    AssetId, Block, CanonicalDocument, ChunkPolicy, Chunker, ChunkerVersion, CodeBlock,
    CommonBlock, Lang, Metadata, ParserVersion, Provenance, SourceSpan, SourceType, TrustLevel,
    WorkspacePath, id_for_block, id_for_doc,
};
use time::OffsetDateTime;

// ── helpers ──────────────────────────────────────────────────────────────────

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

/// Build a `CanonicalDocument` with a single `Block::Code` containing `yaml_text`.
fn yaml_doc(yaml_text: &str) -> CanonicalDocument {
    let wp = WorkspacePath("manifests/deploy.yaml".into());
    let aid = AssetId("c".repeat(64));
    let pv = ParserVersion("code-yaml-v1".into());
    let doc_id = id_for_doc(&wp, &aid, &pv);

    let line_count = yaml_text.lines().count() as u32;
    let span = SourceSpan::Code {
        line_start: 1,
        line_end: line_count.max(1),
        symbol: None,
        lang: Some("yaml".into()),
    };
    let bid = id_for_block(&doc_id, "code", &[], 0, &span);
    let block = Block::Code(CodeBlock {
        common: CommonBlock {
            block_id: bid,
            heading_path: vec![],
            source_span: span,
        },
        lang: Some("yaml".into()),
        code: yaml_text.to_string(),
    });

    CanonicalDocument {
        doc_id,
        source_asset_id: aid,
        workspace_path: wp,
        title: "deploy.yaml".into(),
        lang: Lang("und".into()),
        blocks: vec![block],
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
            code_lang: Some("yaml".into()),
        },
        provenance: Provenance { events: vec![] },
        parser_version: pv,
        schema_version: 1,
        doc_version: 1,
        last_chunker_version: None,
        last_embedding_version: None,
    }
}

fn policy() -> ChunkPolicy {
    ChunkPolicy {
        target_tokens: 500,
        overlap_tokens: 80,
        respect_markdown_headings: false,
        chunker_version: ChunkerVersion("k8s-manifest-resource-v1".into()),
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

/// Three YAML documents: 2 valid k8s resources + 1 non-k8s (no apiVersion).
/// The chunker must emit exactly 2 chunks with the correct symbols and lang.
#[test]
fn k8s_multi_doc_emits_one_chunk_per_resource() {
    let fixture_path = fixtures_dir().join("sample_k8s.yaml");
    let text = std::fs::read_to_string(&fixture_path)
        .unwrap_or_else(|e| panic!("cannot read fixture {}: {e}", fixture_path.display()));

    let doc = yaml_doc(&text);
    let chunks = K8sManifestResourceV1Chunker
        .chunk(&doc, &policy())
        .expect("chunk");

    assert_eq!(
        chunks.len(),
        2,
        "expected 2 k8s chunks, got {}: {chunks:#?}",
        chunks.len()
    );

    let symbols: Vec<&str> = chunks
        .iter()
        .map(|c| {
            match &c.source_spans[0] {
                SourceSpan::Code { symbol, .. } => {
                    symbol.as_deref().expect("symbol must be Some for k8s chunks")
                }
                other => panic!("expected Code span, got {other:?}"),
            }
        })
        .collect();

    assert_eq!(
        symbols,
        vec!["Deployment/prod/api-server", "Service/prod/api-server"],
        "symbols mismatch: {symbols:?}"
    );

    // Verify lang = "yaml" on every chunk.
    for chunk in &chunks {
        match &chunk.source_spans[0] {
            SourceSpan::Code { lang, .. } => {
                assert_eq!(lang.as_deref(), Some("yaml"), "lang must be 'yaml'");
            }
            other => panic!("expected Code span, got {other:?}"),
        }
    }

    // Verify chunker_version label.
    for chunk in &chunks {
        assert_eq!(chunk.chunker_version.0, "k8s-manifest-resource-v1");
    }

    // Every chunk from a multi-resource file must have a distinct chunk_id.
    // Without the fix, all non-oversize resources get split_key=None which
    // collapses to the same id_hash (= base_policy_hash) → UNIQUE constraint
    // violation on the second resource.
    let ids: std::collections::HashSet<_> = chunks.iter().map(|c| c.chunk_id.clone()).collect();
    assert_eq!(
        ids.len(),
        chunks.len(),
        "every k8s resource chunk must have a distinct chunk_id (multi-resource collision regression)"
    );
}

/// A YAML document with an indentation error (tab in a space-indented context)
/// must cause the chunker to return 0 chunks for the entire file.
#[test]
fn k8s_invalid_yaml_emits_zero_chunks() {
    // serde_yaml 0.9 is lenient about duplicate keys (last wins), so use a
    // genuine YAML structural error (unclosed flow sequence) to force a parse
    // failure.
    let actually_bad = "apiVersion: v1\nkind: Service\nfoo: [\nbar\n";

    let doc = yaml_doc(actually_bad);
    let chunks = K8sManifestResourceV1Chunker
        .chunk(&doc, &policy())
        .expect("chunk should not error — return Ok(vec![]) for invalid yaml");

    assert_eq!(
        chunks.len(),
        0,
        "invalid YAML must yield 0 chunks, got {}: {chunks:#?}",
        chunks.len()
    );
}

/// A cluster-scoped resource (no `metadata.namespace`) must produce a symbol
/// of the form `<Kind>/<name>` (two components, no namespace segment).
#[test]
fn k8s_cluster_scoped_resource_symbol() {
    let yaml = "\
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRole
metadata:
  name: cluster-admin
rules:
- apiGroups: [\"*\"]
  resources: [\"*\"]
  verbs: [\"*\"]
";

    let doc = yaml_doc(yaml);
    let chunks = K8sManifestResourceV1Chunker
        .chunk(&doc, &policy())
        .expect("chunk");

    assert_eq!(
        chunks.len(),
        1,
        "expected 1 chunk for cluster-scoped resource, got {}: {chunks:#?}",
        chunks.len()
    );

    match &chunks[0].source_spans[0] {
        SourceSpan::Code { symbol, lang, .. } => {
            assert_eq!(
                symbol.as_deref(),
                Some("ClusterRole/cluster-admin"),
                "cluster-scoped symbol must be <Kind>/<name>"
            );
            assert_eq!(lang.as_deref(), Some("yaml"));
        }
        other => panic!("expected Code span, got {other:?}"),
    }
}

/// 200+ line resource exercises `tier2_shared::push_chunks_with_oversize`'s
/// line-window split branch. All chunks must share the same symbol
/// (`<Kind>/<ns>/<name>`); their line ranges must form a contiguous
/// partition; chunk_ids must all differ (the `#L{k}` suffix on `id_for_chunk`
/// ensures uniqueness across windows). Spec p10-2 risks section explicitly
/// flags "거대 ConfigMap" — this test covers that path.
#[test]
fn k8s_oversize_splits_into_line_windows_sharing_symbol() {
    // ConfigMap with 250 data keys → ~256 total lines, > AST_CHUNK_MAX_LINES (200).
    let mut yaml = String::from(
        "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: big\n  namespace: prod\ndata:\n",
    );
    for i in 0..250 {
        yaml.push_str(&format!("  key{i}: value{i}\n"));
    }

    let doc = yaml_doc(&yaml);
    let chunks = K8sManifestResourceV1Chunker
        .chunk(&doc, &policy())
        .expect("chunk");

    assert!(
        chunks.len() >= 2,
        "expected ≥2 chunks for oversize resource, got {}",
        chunks.len()
    );

    // Every chunk must share the same symbol + lang.
    let expected_symbol = "ConfigMap/prod/big";
    for (i, c) in chunks.iter().enumerate() {
        match &c.source_spans[0] {
            SourceSpan::Code { symbol, lang, .. } => {
                assert_eq!(
                    symbol.as_deref(),
                    Some(expected_symbol),
                    "chunk[{i}] symbol must equal `{expected_symbol}`"
                );
                assert_eq!(lang.as_deref(), Some("yaml"));
            }
            other => panic!("chunk[{i}]: expected Code span, got {other:?}"),
        }
    }

    // chunk_ids must all be distinct (oversize fallback's #L{k} suffix).
    let ids: std::collections::HashSet<_> = chunks.iter().map(|c| c.chunk_id.clone()).collect();
    assert_eq!(
        ids.len(),
        chunks.len(),
        "oversize chunks must have distinct chunk_ids (the #L{{k}} suffix should disambiguate)"
    );

    // Line ranges must form a contiguous partition: chunk[i].line_end + 1 == chunk[i+1].line_start.
    let ranges: Vec<(u32, u32)> = chunks
        .iter()
        .map(|c| match &c.source_spans[0] {
            SourceSpan::Code { line_start, line_end, .. } => (*line_start, *line_end),
            other => panic!("expected Code span, got {other:?}"),
        })
        .collect();
    for w in ranges.windows(2) {
        let (_, prev_end) = w[0];
        let (next_start, _) = w[1];
        assert_eq!(
            prev_end + 1,
            next_start,
            "line ranges must be contiguous: {} → {} (got gap or overlap)",
            prev_end,
            next_start
        );
    }
}
