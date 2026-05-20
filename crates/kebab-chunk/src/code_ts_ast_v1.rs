//! `code-ts-ast-v1` — maps a tree-sitter-derived TypeScript AST
//! `CanonicalDocument` (one `Block::Code` per semantic unit, each with
//! `SourceSpan::Code`) to chunks 1:1. A unit longer than
//! `AST_CHUNK_MAX_LINES` is split into `<symbol> [part i/N]` sub-chunks
//! at blank-line paragraph boundaries (design §9.1 oversize fallback).
//!
//! tree-sitter is intentionally NOT a dependency here: AST work is
//! parser-side (`kebab-parse-code`, design §6.3). This chunker only
//! consumes the `CanonicalDocument`.
//!
//! `AST_CHUNK_MAX_LINES` is a constant matching
//! `IngestCodeCfg::default().ast_chunk_max_lines` (200). Per-medium
//! config threading needs a chunker registry (P+); same deviation
//! pattern as `pdf-page-v1`'s pinned `chunker_version`
//! (`tasks/HOTFIXES.md`).

use kebab_core::{
    Block, BlockId, CanonicalDocument, Chunk, ChunkPolicy, Chunker, ChunkerVersion, DocumentId,
    SourceSpan, id_for_chunk,
};

const VERSION_LABEL: &str = "code-ts-ast-v1";
const BYTES_PER_TOKEN: usize = 3;
const POLICY_HASH_HEX_LEN: usize = 16;
const AST_CHUNK_MAX_LINES: u32 = 200;

#[derive(Clone, Copy, Debug, Default)]
pub struct CodeTsAstV1Chunker;

impl Chunker for CodeTsAstV1Chunker {
    fn chunker_version(&self) -> ChunkerVersion {
        ChunkerVersion(VERSION_LABEL.to_string())
    }

    fn policy_hash(&self, policy: &ChunkPolicy) -> String {
        let bytes = serde_json_canonicalizer::to_vec(policy)
            .expect("canonical JSON serialization of ChunkPolicy must not fail");
        let hex = blake3::hash(&bytes).to_hex().to_string();
        hex[..POLICY_HASH_HEX_LEN].to_string()
    }

    fn chunk(
        &self,
        doc: &CanonicalDocument,
        policy: &ChunkPolicy,
    ) -> anyhow::Result<Vec<Chunk>> {
        for b in &doc.blocks {
            let c = match b {
                Block::Code(c) => c,
                _ => anyhow::bail!(
                    "CodeTsAstV1Chunker only handles code docs (got non-Code block)"
                ),
            };
            if !matches!(c.common.source_span, SourceSpan::Code { .. }) {
                anyhow::bail!(
                    "CodeTsAstV1Chunker only handles code docs (got non-Code source_span)"
                );
            }
        }

        let base_policy_hash = self.policy_hash(policy);
        let chunker_version = self.chunker_version();
        let mut out: Vec<Chunk> = Vec::new();

        for b in &doc.blocks {
            let cb = match b {
                Block::Code(c) => c,
                _ => unreachable!("validated above"),
            };
            let (ls, le, symbol, lang) = match &cb.common.source_span {
                SourceSpan::Code { line_start, line_end, symbol, lang } => {
                    (*line_start, *line_end, symbol.clone(), lang.clone())
                }
                _ => unreachable!("validated above"),
            };
            let block_ids: Vec<BlockId> = vec![cb.common.block_id.clone()];
            let span_lines = le.saturating_sub(ls) + 1;

            if span_lines <= AST_CHUNK_MAX_LINES {
                let span = SourceSpan::Code {
                    line_start: ls,
                    line_end: le,
                    symbol: symbol.clone(),
                    lang: lang.clone(),
                };
                out.push(make_chunk(
                    doc, &chunker_version, &block_ids, &base_policy_hash,
                    None, span, cb.code.clone(),
                ));
            } else {
                let parts = split_oversize(&cb.code);
                let n = parts.len();
                for (i, (off_start, off_end, text)) in parts.into_iter().enumerate() {
                    let part_ls = ls + off_start;
                    let part_le = ls + off_end;
                    let part_sym = symbol
                        .as_ref()
                        .map(|s| format!("{s} [part {}/{n}]", i + 1));
                    let span = SourceSpan::Code {
                        line_start: part_ls,
                        line_end: part_le,
                        symbol: part_sym,
                        lang: lang.clone(),
                    };
                    out.push(make_chunk(
                        doc, &chunker_version, &block_ids, &base_policy_hash,
                        Some(part_ls), span, text,
                    ));
                }
            }
        }

        tracing::debug!(
            target: "kebab-chunk",
            doc_id = %doc.doc_id,
            chunks = out.len(),
            "code-ts-ast-v1 chunked",
        );
        Ok(out)
    }
}

#[allow(clippy::too_many_arguments)]
fn make_chunk(
    doc: &CanonicalDocument,
    chunker_version: &ChunkerVersion,
    block_ids: &[BlockId],
    base_policy_hash: &str,
    split_key: Option<u32>,
    span: SourceSpan,
    text: String,
) -> Chunk {
    let id_hash = match split_key {
        Some(k) => format!("{base_policy_hash}#L{k}"),
        None => base_policy_hash.to_string(),
    };
    let chunk_id = id_for_chunk(&doc.doc_id, chunker_version, block_ids, &id_hash);
    let token_estimate = text.len().div_ceil(BYTES_PER_TOKEN);
    Chunk {
        chunk_id,
        doc_id: DocumentId(doc.doc_id.0.clone()),
        block_ids: block_ids.to_vec(),
        text,
        heading_path: Vec::new(),
        source_spans: vec![span],
        token_estimate,
        chunker_version: chunker_version.clone(),
        policy_hash: base_policy_hash.to_string(),
    }
}

/// Split an oversize unit at blank-line paragraph boundaries, greedily
/// gluing paragraphs until ~`AST_CHUNK_MAX_LINES` lines accumulate.
/// Returns `(line_offset_start, line_offset_end, text)` where offsets are
/// 0-based within the unit (caller adds the unit's absolute `line_start`).
fn split_oversize(code: &str) -> Vec<(u32, u32, String)> {
    let lines: Vec<&str> = code.split('\n').collect();
    let total = lines.len() as u32;
    let mut out: Vec<(u32, u32, String)> = Vec::new();
    let mut start: u32 = 0;
    while start < total {
        let mut end = (start + AST_CHUNK_MAX_LINES).min(total);
        let floor = start + (AST_CHUNK_MAX_LINES * 4 / 5);
        if end < total {
            if let Some(b) = (floor.min(end)..end)
                .rev()
                .find(|&i| lines[i as usize].trim().is_empty())
            {
                end = b + 1;
            }
        }
        let text = lines[start as usize..end as usize].join("\n");
        out.push((start, end.saturating_sub(1), text));
        start = end;
    }
    if out.is_empty() {
        out.push((0, total.saturating_sub(1), code.to_string()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use kebab_core::{
        Block, CanonicalDocument, ChunkPolicy, Chunker, ChunkerVersion, CodeBlock, CommonBlock,
        SourceSpan, id_for_block, id_for_doc, AssetId, Lang, Metadata, ParserVersion, Provenance,
        SourceType, TrustLevel, WorkspacePath,
    };
    use time::OffsetDateTime;

    fn code_doc(units: &[(&str, u32, u32, &str)]) -> CanonicalDocument {
        let wp = WorkspacePath("crates/x/src/a.ts".into());
        let aid = AssetId("a".repeat(64));
        let pv = ParserVersion("code-ts-v1".into());
        let doc_id = id_for_doc(&wp, &aid, &pv);
        let blocks = units
            .iter()
            .enumerate()
            .map(|(i, (sym, ls, le, code))| {
                let span = SourceSpan::Code {
                    line_start: *ls,
                    line_end: *le,
                    symbol: Some((*sym).to_string()),
                    lang: Some("typescript".into()),
                };
                let bid = id_for_block(&doc_id, "code", &[], i as u32, &span);
                Block::Code(CodeBlock {
                    common: CommonBlock { block_id: bid, heading_path: vec![], source_span: span },
                    lang: Some("typescript".into()),
                    code: (*code).to_string(),
                })
            })
            .collect();
        CanonicalDocument {
            doc_id, source_asset_id: aid, workspace_path: wp, title: "a".into(),
            lang: Lang("und".into()), blocks,
            metadata: Metadata {
                aliases: vec![], tags: vec![],
                created_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
                updated_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
                source_type: SourceType::Note, trust_level: TrustLevel::Primary,
                user_id_alias: None, user: Default::default(),
                repo: Some("kebab".into()), git_branch: Some("main".into()),
                git_commit: Some("0".repeat(40)), code_lang: Some("typescript".into()),
            },
            provenance: Provenance { events: vec![] },
            parser_version: pv, schema_version: 1, doc_version: 1,
            last_chunker_version: None, last_embedding_version: None,
        }
    }
    fn policy() -> ChunkPolicy {
        ChunkPolicy { target_tokens: 500, overlap_tokens: 80,
            respect_markdown_headings: false,
            chunker_version: ChunkerVersion(VERSION_LABEL.into()) }
    }

    #[test]
    fn chunker_version_is_code_ts_ast_v1() {
        assert_eq!(CodeTsAstV1Chunker.chunker_version(),
            ChunkerVersion("code-ts-ast-v1".into()));
    }

    #[test]
    fn one_chunk_per_unit_preserves_code_span() {
        let doc = code_doc(&[
            ("parse", 1, 3, "function parse(): void {\n    // x\n}"),
            ("Foo.double", 5, 7, "function double(): number {\n    //\n    return 0;\n}"),
        ]);
        let chunks = CodeTsAstV1Chunker.chunk(&doc, &policy()).unwrap();
        assert_eq!(chunks.len(), 2);
        for c in &chunks {
            assert_eq!(c.source_spans.len(), 1);
            assert!(matches!(c.source_spans[0], SourceSpan::Code { .. }));
            assert_eq!(c.heading_path, Vec::<String>::new());
            assert_eq!(c.chunker_version.0, "code-ts-ast-v1");
        }
        match &chunks[0].source_spans[0] {
            SourceSpan::Code { symbol, line_start, line_end, .. } => {
                assert_eq!(symbol.as_deref(), Some("parse"));
                assert_eq!((*line_start, *line_end), (1, 3));
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn oversize_unit_splits_into_parts_with_unique_ids() {
        let body = (0..500).map(|i| format!("    const x{i} = {i};")).collect::<Vec<_>>().join("\n");
        let code = format!("function big(): void {{\n{body}\n}}");
        let doc = code_doc(&[("big", 1, 502, &code)]);
        let chunks = CodeTsAstV1Chunker.chunk(&doc, &policy()).unwrap();
        assert!(chunks.len() >= 2, "oversize unit must split, got {}", chunks.len());
        for c in &chunks {
            match &c.source_spans[0] {
                SourceSpan::Code { symbol, .. } => {
                    assert!(symbol.as_deref().unwrap().starts_with("big [part "),
                        "part-numbered symbol, got {symbol:?}");
                }
                _ => unreachable!(),
            }
        }
        let mut ids: Vec<&str> = chunks.iter().map(|c| c.chunk_id.0.as_str()).collect();
        let n = ids.len(); ids.sort(); ids.dedup();
        assert_eq!(ids.len(), n, "chunk_ids unique across split parts");
    }

    #[test]
    fn non_code_doc_errors() {
        use kebab_core::TextBlock;
        let mut doc = code_doc(&[("parse", 1, 1, "function parse(): void {}")]);
        doc.blocks = vec![Block::Paragraph(TextBlock {
            common: CommonBlock {
                block_id: kebab_core::BlockId("b".into()),
                heading_path: vec![],
                source_span: SourceSpan::Line { start: 1, end: 1 },
            },
            text: "x".into(), inlines: vec![],
        })];
        let err = CodeTsAstV1Chunker.chunk(&doc, &policy()).unwrap_err();
        assert!(err.to_string().contains("CodeTsAstV1Chunker"));
    }

    #[test]
    fn deterministic_chunk_ids_1000() {
        let doc = code_doc(&[("parse", 1, 2, "function parse(): void {}\n")]);
        let base: Vec<String> = CodeTsAstV1Chunker.chunk(&doc, &policy())
            .unwrap().into_iter().map(|c| c.chunk_id.0).collect();
        for _ in 0..1000 {
            let again: Vec<String> = CodeTsAstV1Chunker.chunk(&doc, &policy())
                .unwrap().into_iter().map(|c| c.chunk_id.0).collect();
            assert_eq!(again, base);
        }
    }

    #[test]
    fn policy_hash_matches_md_heading_v1() {
        let p = policy();
        assert_eq!(CodeTsAstV1Chunker.policy_hash(&p),
            crate::MdHeadingV1Chunker.policy_hash(&p));
    }
}
