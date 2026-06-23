//! `kebab-parse-code::go` — tree-sitter Go AST extractor (P10-1C-Go Task D).
//!
//! Implements [`kebab_core::Extractor`] for [`MediaType::Code("go")`].
//! Walks the tree-sitter parse tree and emits one [`Block::Code`] per
//! top-level AST semantic unit (free fn, method, each type spec) carrying
//! [`SourceSpan::Code`] with the unit's self-reference symbol path
//! (design §3.4 Go row). Glue declarations (`import` / `const` / `var`)
//! collapse into one grouped `<top-level>` (or `<module>`) unit.
//!
//! Unlike the Python/TS/JS extractors which path-derive their module
//! prefix from the workspace file path, Go's package identity comes from
//! the source itself (the leading `package` clause) — `extract_package`
//! reads it from the AST. If the `package_clause` is missing (invalid Go
//! in practice) the prefix falls back to `"<unknown>"`.
//!
//! Doc comments immediately preceding an item are folded into that
//! item's line range via `unit_start` (1B pattern). Go has no separate
//! attribute/decorator AST nodes.
//!
//! Per design §3.4 / §9.1 / §9 versioning.

use anyhow::Result;
use kebab_core::{
    Block, CanonicalDocument, CodeBlock, CommonBlock, Extractor, Lang, MediaType, Metadata,
    ParserVersion, Provenance, ProvenanceEvent, ProvenanceKind, SourceSpan, SourceType, TrustLevel,
    id_for_block, id_for_doc,
};
use serde_json::Map;
use time::OffsetDateTime;

use crate::scaffold::{filename_from_workspace_path, join_symbol, strip_extension};

pub const PARSER_VERSION: &str = "code-go-v1";

/// Go AST extractor. Per-unit blocks via tree-sitter-go 0.25
/// (`LANGUAGE: LanguageFn`) parsed by tree-sitter 0.26.
pub struct GoAstExtractor;

impl GoAstExtractor {
    pub fn new() -> Self {
        Self
    }
}

impl Default for GoAstExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl Extractor for GoAstExtractor {
    fn supports(&self, m: &MediaType) -> bool {
        matches!(m, MediaType::Code(l) if l == "go")
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
                "kebab-parse-code: unsupported media_type for GoAstExtractor: {:?}",
                asset.media_type
            );
        }

        let parser_version = self.parser_version();
        let doc_id = id_for_doc(&asset.workspace_path, &asset.asset_id, &parser_version);

        let source = String::from_utf8(bytes.to_vec())
            .map_err(|e| anyhow::anyhow!("kebab-parse-code: Go source is not valid UTF-8: {e}"))?;

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
            code_lang: Some("go".to_string()),
            source_id: None,
        };

        tracing::debug!(
            target: "kebab-parse-code",
            "extracted Go doc_id={} workspace_path={} units={}",
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

/// p10-1C-Go: extract `package` declaration text from a tree-sitter-go
/// `source_file`. Returns `None` if no `package_clause` (invalid Go in
/// practice but defense-in-depth). Per design §3.4 Go row.
fn extract_package(root: tree_sitter::Node, src: &str) -> Option<String> {
    let mut cur = root.walk();
    for child in root.named_children(&mut cur) {
        if child.kind() == "package_clause" {
            let mut c2 = child.walk();
            for sub in child.named_children(&mut c2) {
                if sub.kind() == "package_identifier" {
                    return Some(src[sub.start_byte()..sub.end_byte()].to_string());
                }
            }
        }
    }
    None
}

fn build_blocks(
    source: &str,
    doc_id: &kebab_core::DocumentId,
) -> anyhow::Result<Vec<kebab_core::Block>> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_go::LANGUAGE.into())
        .map_err(|e| anyhow::anyhow!("set tree-sitter-go language: {e}"))?;
    let tree = parser
        .parse(source.as_bytes(), None)
        .ok_or_else(|| anyhow::anyhow!("tree-sitter failed to parse Go source"))?;
    let lines: Vec<&str> = source.split('\n').collect();

    let root = tree.root_node();
    let mod_prefix = extract_package(root, source).unwrap_or_else(|| "<unknown>".to_string());

    // units: (symbol, line_start, line_end, is_real_semantic_unit).
    // Glue groups are pushed with a sentinel symbol + is_real=false so a
    // post-pass can decide `<module>` vs `<top-level>` (1B post-pass
    // mirror).
    let mut units: Vec<(String, u32, u32, bool)> = Vec::new();
    // (is_import 0/1, s, e). `is_import` flags `import_declaration` —
    // used by the glue flush to pick `<module>` vs `<top-level>`
    // provisional label.
    let mut glue: Vec<(usize, u32, u32)> = Vec::new();

    fn node_name_text<'a>(n: &tree_sitter::Node, src: &'a str) -> Option<&'a str> {
        n.child_by_field_name("name")
            .map(|c| &src[c.start_byte()..c.end_byte()])
    }
    /// Walk preceding `comment` siblings to extend the unit's line range
    /// upward, folding leading doc / line comments into the unit. Go has
    /// no decorator/attribute nodes — doc comments are simply preceding
    /// `comment` siblings (the 1B pattern).
    fn unit_start(n: &tree_sitter::Node) -> u32 {
        let mut start = n.start_position().row as u32 + 1;
        let mut prev = n.prev_sibling();
        while let Some(p) = prev {
            if p.kind() == "comment" {
                start = p.start_position().row as u32 + 1;
                prev = p.prev_sibling();
            } else {
                break;
            }
        }
        start
    }

    /// Extract the receiver type text for a `method_declaration`. The
    /// returned slice INCLUDES the leading `*` for pointer receivers
    /// (`(*Foo).Bar`) per design §3.4 Go row example. Returns `None` if
    /// the receiver is malformed (defense in depth).
    fn receiver_type_text<'a>(method_node: &tree_sitter::Node, src: &'a str) -> Option<&'a str> {
        let recv = method_node.child_by_field_name("receiver")?;
        let mut cw = recv.walk();
        for p in recv.named_children(&mut cw) {
            if p.kind() == "parameter_declaration" {
                if let Some(ty) = p.child_by_field_name("type") {
                    return Some(&src[ty.start_byte()..ty.end_byte()]);
                }
            }
        }
        None
    }

    let mut cur = root.walk();
    for child in root.named_children(&mut cur) {
        let s = unit_start(&child);
        let e = child.end_position().row as u32 + 1;
        match child.kind() {
            "function_declaration" => {
                if let Some(name) = node_name_text(&child, source) {
                    glue.retain(|(_, gs, _)| *gs < s);
                    flush_glue(&mut glue, &mut units, &mod_prefix);
                    let sym = join_symbol(&mod_prefix, &[], name);
                    units.push((sym, s, e, true));
                }
            }
            "method_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    glue.retain(|(_, gs, _)| *gs < s);
                    flush_glue(&mut glue, &mut units, &mod_prefix);
                    let owner = receiver_type_text(&child, source).unwrap_or("<unknown>");
                    let method_name = &source[name_node.start_byte()..name_node.end_byte()];
                    let sym = format!("{mod_prefix}.({owner}).{method_name}");
                    units.push((sym, s, e, true));
                }
            }
            "type_declaration" => {
                // One unit per inner `type_spec`. Each type_spec gets
                // the type_declaration's whole upward-folded `s` range
                // start so doc comments are attached to the first spec;
                // subsequent specs use their own start. Match 1B
                // pattern: keep the outer `s` only when there's a single
                // spec; otherwise use the spec's own start.
                let mut tcur = child.walk();
                let specs: Vec<tree_sitter::Node> = child
                    .named_children(&mut tcur)
                    .filter(|c| c.kind() == "type_spec")
                    .collect();
                let single = specs.len() == 1;
                for spec in specs {
                    let name_node = match spec.child_by_field_name("name") {
                        Some(n) => n,
                        None => continue,
                    };
                    let spec_s = if single {
                        s
                    } else {
                        spec.start_position().row as u32 + 1
                    };
                    let spec_e = spec.end_position().row as u32 + 1;
                    glue.retain(|(_, gs, _)| *gs < spec_s);
                    flush_glue(&mut glue, &mut units, &mod_prefix);
                    let name = &source[name_node.start_byte()..name_node.end_byte()];
                    let sym = join_symbol(&mod_prefix, &[], name);
                    units.push((sym, spec_s, spec_e, true));
                }
            }
            "import_declaration" => {
                glue.push((1, s, e));
            }
            "const_declaration" | "var_declaration" => {
                glue.push((0, s, e));
            }
            _ => {}
        }
    }
    flush_glue(&mut glue, &mut units, &mod_prefix);

    // `<module>` is correct only when the file produced no real unit.
    // Otherwise the import/const/var-only group becomes `<top-level>`
    // (same post-pass as 1B). Match on the suffix so the demotion stays
    // mod-prefix-agnostic.
    let has_real_unit = units.iter().any(|(_, _, _, is_real)| *is_real);
    if has_real_unit {
        for (sym, _, _, is_real) in &mut units {
            if !*is_real && sym.ends_with("<module>") {
                let pre = &sym[..sym.len() - "<module>".len()];
                *sym = format!("{pre}<top-level>");
            }
        }
    }

    let total_lines = lines.len() as u32;
    let mut blocks = Vec::with_capacity(units.len());
    for (ordinal, (symbol, ls, le, _is_real)) in units.into_iter().enumerate() {
        let line_start = ls.max(1);
        let line_end = le.min(total_lines.max(1));
        let span = SourceSpan::Code {
            line_start,
            line_end,
            symbol: Some(symbol),
            lang: Some("go".to_string()),
        };
        let block_id = id_for_block(doc_id, "code", &[], ordinal as u32, &span);
        let code = lines[(line_start as usize - 1)..(line_end as usize)].join("\n");
        blocks.push(Block::Code(CodeBlock {
            common: CommonBlock {
                block_id,
                heading_path: Vec::new(),
                source_span: span,
            },
            lang: Some("go".to_string()),
            code,
        }));
    }
    Ok(blocks)
}

fn flush_glue(
    glue: &mut Vec<(usize, u32, u32)>,
    units: &mut Vec<(String, u32, u32, bool)>,
    mod_prefix: &str,
) {
    if glue.is_empty() {
        return;
    }
    let s = glue.iter().map(|(_, a, _)| *a).min().unwrap();
    let e = glue.iter().map(|(_, _, b)| *b).max().unwrap();
    // Provisional label: `<module>` only if the group is exclusively
    // imports (1A's `only_mod_decls` analog). The post-pass demotes any
    // `<module>` to `<top-level>` if the file produced any real unit.
    let only_imports = glue.iter().all(|(is_import, _, _)| *is_import == 1);
    let label = if only_imports {
        "<module>"
    } else {
        "<top-level>"
    };
    units.push((join_symbol(mod_prefix, &[], label), s, e, false));
    glue.clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use kebab_core::{Block, MediaType, SourceSpan};

    fn extract_fixture() -> kebab_core::CanonicalDocument {
        let bytes = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/sample.go"
        ))
        .unwrap();
        // Reuse the cross-language test-support helper promoted in 1B.
        let asset = crate::rust::tests_support::fixed_code_asset("crates/x/src/sample.go", "go");
        let cfg = kebab_core::ExtractConfig::default();
        let root = std::path::PathBuf::from("/tmp");
        let ctx = kebab_core::ExtractContext {
            asset: &asset,
            workspace_root: &root,
            config: &cfg,
        };
        GoAstExtractor::new().extract(&ctx, &bytes).unwrap()
    }

    #[test]
    fn extractor_supports_only_media_code_go() {
        let e = GoAstExtractor::new();
        assert!(e.supports(&MediaType::Code("go".into())));
        assert!(!e.supports(&MediaType::Code("rust".into())));
        assert!(!e.supports(&MediaType::Markdown));
    }

    #[test]
    fn go_units_match_design_3_4_symbols() {
        let doc = extract_fixture();
        let mut syms: Vec<String> = doc
            .blocks
            .iter()
            .filter_map(|b| match b {
                Block::Code(c) => match &c.common.source_span {
                    SourceSpan::Code { symbol, lang, .. } => {
                        assert_eq!(lang.as_deref(), Some("go"));
                        symbol.clone()
                    }
                    _ => None,
                },
                _ => None,
            })
            .collect();
        syms.sort();
        assert!(syms.iter().any(|s| s == "chunk.Free"), "got {syms:?}");
        assert!(syms.iter().any(|s| s == "chunk.init"), "got {syms:?}");
        assert!(
            syms.iter().any(|s| s == "chunk.MdHeadingV1Chunker"),
            "got {syms:?}"
        );
        assert!(
            syms.iter()
                .any(|s| s == "chunk.(*MdHeadingV1Chunker).ChunkDoc"),
            "got {syms:?}"
        );
        assert!(
            syms.iter().any(|s| s == "chunk.(MdHeadingV1Chunker).Name2"),
            "got {syms:?}"
        );
        assert!(syms.iter().any(|s| s == "chunk.Stringer"), "got {syms:?}");
        // import + const grouped into one glue unit (no isolated `<module>`).
        assert!(
            syms.iter().any(|s| s == "chunk.<top-level>"),
            "got {syms:?}"
        );
    }

    #[test]
    fn deterministic_across_runs() {
        let a = extract_fixture();
        for _ in 0..50 {
            assert_eq!(extract_fixture().blocks, a.blocks);
        }
    }
}
