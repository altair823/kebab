//! `kebab-parse-code::rust` — tree-sitter Rust AST extractor (P10-1A-2).
//!
//! Implements [`kebab_core::Extractor`] for [`MediaType::Code("rust")`].
//! Walks the tree-sitter parse tree and emits one [`Block::Code`] per
//! top-level AST semantic unit (free fn, type, trait, macro, each impl
//! method, recursively per module), each carrying [`SourceSpan::Code`]
//! with the unit's self-reference symbol path (design §3.4). Glue
//! declarations (`use` / `const` / `static` / bodyless `mod` / top-level
//! attributes / macro invocations) collapse into one grouped
//! `<top-level>` (or `<module>`) unit.
//!
//! Doc comments and attributes immediately preceding an item are folded
//! into that item's line range (design §9.1 "선언 + doc comment").
//!
//! Scope is intentionally narrow: AST unit extraction + symbol paths +
//! line ranges for Rust. The `CanonicalDocument` scaffold mirrors
//! `kebab-parse-pdf`. Per design §3.4 / §9.1 / §9 versioning.

use anyhow::Result;
use kebab_core::{
    Block, CanonicalDocument, CodeBlock, CommonBlock, Extractor, Lang, MediaType, Metadata,
    ParserVersion, Provenance, ProvenanceEvent, ProvenanceKind, SourceSpan, SourceType, TrustLevel,
    id_for_block, id_for_doc,
};
use serde_json::Map;
use time::OffsetDateTime;

pub const PARSER_VERSION: &str = "code-rust-v1";

/// Rust AST extractor. Per-unit blocks via tree-sitter-rust 0.24
/// (`LANGUAGE: LanguageFn`) parsed by tree-sitter 0.26.
pub struct RustAstExtractor;

impl RustAstExtractor {
    pub fn new() -> Self {
        Self
    }
}

impl Default for RustAstExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl Extractor for RustAstExtractor {
    fn supports(&self, m: &MediaType) -> bool {
        matches!(m, MediaType::Code(l) if l == "rust")
    }

    fn parser_version(&self) -> ParserVersion {
        ParserVersion(PARSER_VERSION.to_string())
    }

    fn extract(
        &self,
        ctx: &kebab_core::ExtractContext<'_>,
        bytes: &[u8],
    ) -> Result<CanonicalDocument> {
        let asset = ctx.asset;
        if !self.supports(&asset.media_type) {
            anyhow::bail!(
                "kebab-parse-code: unsupported media_type for RustAstExtractor: {:?}",
                asset.media_type
            );
        }

        let parser_version = self.parser_version();
        let doc_id = id_for_doc(&asset.workspace_path, &asset.asset_id, &parser_version);

        let source = String::from_utf8(bytes.to_vec()).map_err(|e| {
            anyhow::anyhow!("kebab-parse-code: Rust source is not valid UTF-8: {e}")
        })?;

        let blocks = build_blocks(&source, &doc_id)?;
        let unit_count = blocks.len() as u32;

        let now = OffsetDateTime::now_utc();
        let mut events: Vec<ProvenanceEvent> = Vec::with_capacity(2);
        events.push(ProvenanceEvent {
            at: asset.discovered_at,
            agent: "kb-source-fs".to_string(),
            kind: ProvenanceKind::Discovered,
            note: None,
        });
        events.push(ProvenanceEvent {
            at: now,
            agent: "kb-parse-code".to_string(),
            kind: ProvenanceKind::Parsed,
            note: Some(format!(
                "parser_version={}; unit_count={}",
                parser_version.0, unit_count
            )),
        });

        let title = {
            let fname = filename_from_workspace_path(&asset.workspace_path.0);
            strip_extension(&fname)
        };

        // Resolve the file's absolute path for repo detection. If the
        // source URI carries a relative path, anchor it at the workspace
        // root so the `.git/` walk-up starts from the right place.
        let abs_path = match &asset.source_uri {
            kebab_core::SourceUri::File(p) => {
                if p.is_absolute() {
                    p.clone()
                } else {
                    ctx.workspace_root.join(p)
                }
            }
            kebab_core::SourceUri::Kb(_) => ctx.workspace_root.to_path_buf(),
        };
        let (repo, git_branch, git_commit) = match crate::repo::detect_repo(&abs_path) {
            Some(r) => (Some(r.name), r.branch, r.commit),
            None => (None, None, None),
        };

        let metadata = Metadata {
            aliases: Vec::new(),
            tags: Vec::new(),
            created_at: asset.discovered_at,
            updated_at: asset.discovered_at,
            source_type: SourceType::Note,
            trust_level: TrustLevel::Primary,
            user_id_alias: None,
            user: Map::new(),
            repo,
            git_branch,
            git_commit,
            code_lang: Some("rust".to_string()),
        };

        tracing::debug!(
            target: "kebab-parse-code",
            "extracted Rust doc_id={} workspace_path={} units={}",
            doc_id.0,
            asset.workspace_path.0,
            unit_count
        );

        Ok(CanonicalDocument {
            doc_id,
            source_asset_id: asset.asset_id.clone(),
            workspace_path: asset.workspace_path.clone(),
            title,
            lang: Lang("und".to_string()),
            blocks,
            metadata,
            provenance: Provenance { events },
            parser_version,
            schema_version: 1,
            doc_version: 1,
            last_chunker_version: None,
            last_embedding_version: None,
        })
    }
}

fn filename_from_workspace_path(p: &str) -> String {
    p.rsplit('/').next().unwrap_or(p).to_string()
}

fn strip_extension(filename: &str) -> String {
    match filename.rfind('.') {
        Some(0) => filename.to_string(),
        Some(idx) => filename[..idx].to_string(),
        None => filename.to_string(),
    }
}

fn build_blocks(
    source: &str,
    doc_id: &kebab_core::DocumentId,
) -> anyhow::Result<Vec<kebab_core::Block>> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .map_err(|e| anyhow::anyhow!("set tree-sitter-rust language: {e}"))?;
    let tree = parser
        .parse(source.as_bytes(), None)
        .ok_or_else(|| anyhow::anyhow!("tree-sitter failed to parse Rust source"))?;
    let lines: Vec<&str> = source.split('\n').collect();

    let mut units: Vec<(String, u32, u32)> = Vec::new();
    let mut glue: Vec<(usize, u32, u32)> = Vec::new(); // (is_mod_decl 0/1, s, e)

    fn node_name<'a>(n: &tree_sitter::Node, src: &'a str) -> Option<&'a str> {
        n.child_by_field_name("name")
            .map(|c| &src[c.start_byte()..c.end_byte()])
    }
    fn unit_start(n: &tree_sitter::Node) -> u32 {
        let mut start = n.start_position().row as u32 + 1;
        let mut prev = n.prev_sibling();
        while let Some(p) = prev {
            let k = p.kind();
            if k == "line_comment" || k == "block_comment" || k == "attribute_item" {
                start = p.start_position().row as u32 + 1;
                prev = p.prev_sibling();
            } else {
                break;
            }
        }
        start
    }
    fn walk(
        node: tree_sitter::Node,
        src: &str,
        mod_path: &[String],
        units: &mut Vec<(String, u32, u32)>,
        glue: &mut Vec<(usize, u32, u32)>,
    ) {
        let mut cur = node.walk();
        for child in node.named_children(&mut cur) {
            let s = unit_start(&child);
            let e = child.end_position().row as u32 + 1;
            let prefix = if mod_path.is_empty() {
                String::new()
            } else {
                format!("{}::", mod_path.join("::"))
            };
            match child.kind() {
                "function_item" | "struct_item" | "enum_item" | "union_item"
                | "trait_item" | "type_item" => {
                    if let Some(name) = node_name(&child, src) {
                        flush_glue(glue, units);
                        units.push((format!("{prefix}{name}"), s, e));
                    }
                }
                "macro_definition" => {
                    if let Some(name) = node_name(&child, src) {
                        flush_glue(glue, units);
                        units.push((format!("{prefix}{name}!"), s, e));
                    }
                }
                "impl_item" => {
                    flush_glue(glue, units);
                    let ty = child
                        .child_by_field_name("type")
                        .map(|c| src[c.start_byte()..c.end_byte()].trim().to_string());
                    let tr = child
                        .child_by_field_name("trait")
                        .map(|c| src[c.start_byte()..c.end_byte()].trim().to_string());
                    let owner = tr.or(ty).unwrap_or_else(|| "<impl>".to_string());
                    if let Some(body) = child.child_by_field_name("body") {
                        let mut bc = body.walk();
                        for m in body.named_children(&mut bc) {
                            if m.kind() == "function_item" {
                                if let Some(mn) = node_name(&m, src) {
                                    let ms = unit_start(&m);
                                    let me = m.end_position().row as u32 + 1;
                                    units.push((format!("{prefix}{owner}::{mn}"), ms, me));
                                }
                            }
                        }
                    }
                }
                "mod_item" => {
                    if let Some(body) = child.child_by_field_name("body") {
                        flush_glue(glue, units);
                        let name = node_name(&child, src).unwrap_or("mod").to_string();
                        let mut np = mod_path.to_vec();
                        np.push(name);
                        walk(body, src, &np, units, glue);
                    } else {
                        glue.push((1, s, e));
                    }
                }
                "use_declaration" | "extern_crate_declaration" | "const_item"
                | "static_item" | "attribute_item" | "macro_invocation" => {
                    glue.push((0, s, e));
                }
                _ => {}
            }
        }
        flush_glue(glue, units);
    }
    fn flush_glue(glue: &mut Vec<(usize, u32, u32)>, units: &mut Vec<(String, u32, u32)>) {
        if glue.is_empty() {
            return;
        }
        let s = glue.iter().map(|(_, a, _)| *a).min().unwrap();
        let e = glue.iter().map(|(_, _, b)| *b).max().unwrap();
        let only_mod_decls = glue.iter().all(|(is_mod, _, _)| *is_mod == 1);
        let sym = if only_mod_decls { "<module>" } else { "<top-level>" };
        units.push((sym.to_string(), s, e));
        glue.clear();
    }

    walk(tree.root_node(), source, &[], &mut units, &mut glue);

    let total_lines = lines.len() as u32;
    let mut blocks = Vec::with_capacity(units.len());
    for (ordinal, (symbol, ls, le)) in units.into_iter().enumerate() {
        let line_start = ls.max(1);
        let line_end = le.min(total_lines.max(1));
        let span = SourceSpan::Code {
            line_start,
            line_end,
            symbol: Some(symbol),
            lang: Some("rust".to_string()),
        };
        let block_id = id_for_block(doc_id, "code", &[], ordinal as u32, &span);
        let code = lines[(line_start as usize - 1)..=(line_end as usize - 1)].join("\n");
        blocks.push(Block::Code(CodeBlock {
            common: CommonBlock {
                block_id,
                heading_path: Vec::new(),
                source_span: span,
            },
            lang: Some("rust".to_string()),
            code,
        }));
    }
    Ok(blocks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kebab_core::{Block, MediaType, SourceSpan};

    fn extract_fixture() -> kebab_core::CanonicalDocument {
        let bytes = std::fs::read(
            concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/sample.rs"),
        )
        .unwrap();
        let asset = kebab_parse_code_test_support::fixed_rust_asset("crates/x/src/sample.rs");
        let cfg = kebab_core::ExtractConfig::default();
        let root = std::path::PathBuf::from("/tmp");
        let ctx = kebab_core::ExtractContext { asset: &asset, workspace_root: &root, config: &cfg };
        RustAstExtractor::new().extract(&ctx, &bytes).unwrap()
    }

    #[test]
    fn extractor_supports_only_media_code_rust() {
        let e = RustAstExtractor::new();
        assert!(e.supports(&MediaType::Code("rust".into())));
        assert!(!e.supports(&MediaType::Code("python".into())));
        assert!(!e.supports(&MediaType::Markdown));
    }

    #[test]
    fn emits_one_block_per_semantic_unit_with_symbols() {
        let doc = extract_fixture();
        let mut syms: Vec<(String, u32, u32)> = doc
            .blocks
            .iter()
            .map(|b| match b {
                Block::Code(c) => match &c.common.source_span {
                    SourceSpan::Code { symbol, line_start, line_end, lang } => {
                        assert_eq!(lang.as_deref(), Some("rust"));
                        (symbol.clone().unwrap(), *line_start, *line_end)
                    }
                    _ => panic!("code block must carry SourceSpan::Code"),
                },
                other => panic!("expected Block::Code, got {other:?}"),
            })
            .collect();
        syms.sort();
        let names: Vec<&str> = syms.iter().map(|(s, _, _)| s.as_str()).collect();
        assert!(names.contains(&"parse"));
        assert!(names.contains(&"Foo"));
        assert!(names.contains(&"Foo::double"));
        assert!(names.contains(&"Foo::name"));
        assert!(names.contains(&"Greet"));
        assert!(names.contains(&"inner::helper"));
        assert!(names.contains(&"<top-level>")); // use + const grouped
        let parse_src = doc.blocks.iter().find_map(|b| match b {
            Block::Code(c) if matches!(&c.common.source_span, SourceSpan::Code{symbol,..} if symbol.as_deref()==Some("parse")) => Some(c.code.clone()),
            _ => None,
        }).unwrap();
        assert!(parse_src.contains("/// Doc comment on a free fn."), "doc comment folded in: {parse_src}");
    }

    #[test]
    fn deterministic_across_runs() {
        let a = extract_fixture();
        for _ in 0..50 {
            assert_eq!(extract_fixture().blocks, a.blocks);
        }
    }
}

#[cfg(test)]
mod kebab_parse_code_test_support {
    use kebab_core::*;
    use time::OffsetDateTime;
    pub fn fixed_rust_asset(path: &str) -> RawAsset {
        RawAsset {
            asset_id: AssetId("a".repeat(64)),
            source_uri: SourceUri::File(std::path::PathBuf::from(path)),
            workspace_path: WorkspacePath(path.to_string()),
            media_type: MediaType::Code("rust".to_string()),
            byte_len: 0,
            checksum: Checksum("b".repeat(64)),
            discovered_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
            stored: AssetStorage::Reference {
                path: std::path::PathBuf::from(path),
                sha: Checksum("b".repeat(64)),
            },
        }
    }
}
