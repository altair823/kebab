//! `kebab-parse-code::python` — tree-sitter Python AST extractor (P10-1B Task E).
//!
//! Implements [`kebab_core::Extractor`] for [`MediaType::Code("python")`].
//! Walks the tree-sitter parse tree and emits one [`Block::Code`] per
//! top-level AST semantic unit (free fn, class, each method, recursively
//! per nested class), each carrying [`SourceSpan::Code`] with the unit's
//! dotted self-reference symbol path prefixed by `module_path_for_python`
//! (design §3.4). Glue declarations (`import` / `import from` /
//! `expression_statement` / `assignment` / `global_statement` /
//! `future_import_statement`) collapse into one grouped `<top-level>`
//! (or `<module>`) unit.
//!
//! Decorators are folded into the decorated unit's line range via the
//! `decorated_definition` unwrap arm (analog of the Rust `attribute_item`
//! re-absorption in 1A — see §9.1).
//!
//! Scope follows 1A: AST unit extraction + dotted symbol paths + line
//! ranges. Per design §3.4 / §9.1 / §9 versioning.

use anyhow::Result;
use kebab_core::{
    Block, CanonicalDocument, CodeBlock, CommonBlock, Extractor, Lang, MediaType, Metadata,
    ParserVersion, Provenance, ProvenanceEvent, ProvenanceKind, SourceSpan, SourceType, TrustLevel,
    id_for_block, id_for_doc,
};
use serde_json::Map;
use time::OffsetDateTime;

use crate::scaffold::{filename_from_workspace_path, join_symbol, strip_extension};

pub const PARSER_VERSION: &str = "code-python-v1";

/// Python AST extractor. Per-unit blocks via tree-sitter-python 0.25
/// (`LANGUAGE: LanguageFn`) parsed by tree-sitter 0.26.
pub struct PythonAstExtractor;

impl PythonAstExtractor {
    pub fn new() -> Self {
        Self
    }
}

impl Default for PythonAstExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl Extractor for PythonAstExtractor {
    fn supports(&self, m: &MediaType) -> bool {
        matches!(m, MediaType::Code(l) if l == "python")
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
                "kebab-parse-code: unsupported media_type for PythonAstExtractor: {:?}",
                asset.media_type
            );
        }

        let parser_version = self.parser_version();
        let doc_id = id_for_doc(&asset.workspace_path, &asset.asset_id, &parser_version);

        let source = String::from_utf8(bytes.to_vec()).map_err(|e| {
            anyhow::anyhow!("kebab-parse-code: Python source is not valid UTF-8: {e}")
        })?;

        let mod_prefix = crate::lang::module_path_for_python(&asset.workspace_path.0);
        let blocks = build_blocks(&source, &doc_id, &mod_prefix)?;
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
            code_lang: Some("python".to_string()),
        };

        tracing::debug!(
            target: "kebab-parse-code",
            "extracted Python doc_id={} workspace_path={} units={}",
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

fn build_blocks(
    source: &str,
    doc_id: &kebab_core::DocumentId,
    mod_prefix: &str,
) -> anyhow::Result<Vec<kebab_core::Block>> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_python::LANGUAGE.into())
        .map_err(|e| anyhow::anyhow!("set tree-sitter-python language: {e}"))?;
    let tree = parser
        .parse(source.as_bytes(), None)
        .ok_or_else(|| anyhow::anyhow!("tree-sitter failed to parse Python source"))?;
    let lines: Vec<&str> = source.split('\n').collect();

    // units: (symbol, line_start, line_end, is_real_semantic_unit).
    // Glue groups are pushed with a sentinel symbol + is_real=false so a
    // post-pass can decide `<module>` vs `<top-level>` (same algorithm
    // as 1A Gap 1).
    let mut units: Vec<(String, u32, u32, bool)> = Vec::new();
    // (is_import 0/1, s, e). `is_import` flags `import_statement` /
    // `import_from_statement` / `future_import_statement` — used by the
    // glue flush to pick `<module>` vs `<top-level>` provisional label
    // (1A's `is_mod_decl` analog).
    let mut glue: Vec<(usize, u32, u32)> = Vec::new();

    fn node_name<'a>(n: &tree_sitter::Node, src: &'a str) -> Option<&'a str> {
        n.child_by_field_name("name")
            .map(|c| &src[c.start_byte()..c.end_byte()])
    }
    /// Walk preceding `comment` siblings to extend the unit's line range
    /// upward, folding leading doc / line comments into the unit. Note
    /// that Python decorators are NOT preceding siblings — they live
    /// INSIDE a `decorated_definition` parent — so they are handled by
    /// the unwrap arm below, not here.
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
    fn walk(
        node: tree_sitter::Node,
        src: &str,
        mod_prefix: &str,
        mod_path: &[String],
        units: &mut Vec<(String, u32, u32, bool)>,
        glue: &mut Vec<(usize, u32, u32)>,
    ) {
        let mut cur = node.walk();
        for child in node.named_children(&mut cur) {
            // Default unit line range — overridden by the
            // `decorated_definition` unwrap arm so decorator lines are
            // included.
            let s = unit_start(&child);
            let e = child.end_position().row as u32 + 1;
            match child.kind() {
                "function_definition" => {
                    if let Some(name) = node_name(&child, src) {
                        glue.retain(|(_, gs, _)| *gs < s);
                        flush_glue(glue, units, mod_prefix, mod_path);
                        let sym = join_symbol(mod_prefix, mod_path, name);
                        units.push((sym, s, e, true));
                    }
                }
                "class_definition" => {
                    if let Some(name) = node_name(&child, src) {
                        glue.retain(|(_, gs, _)| *gs < s);
                        flush_glue(glue, units, mod_prefix, mod_path);
                        let sym = join_symbol(mod_prefix, mod_path, name);
                        units.push((sym, s, e, true));
                        // Recurse into the class body with the class
                        // name pushed onto mod_path; methods become
                        // `<...>.<ClassName>.<method>` and nested
                        // classes recurse further with both names.
                        if let Some(body) = child.child_by_field_name("body") {
                            let mut np = mod_path.to_vec();
                            np.push(name.to_string());
                            walk(body, src, mod_prefix, &np, units, glue);
                            debug_assert!(
                                glue.is_empty(),
                                "inner walk must flush its glue before returning"
                            );
                        }
                    }
                }
                "decorated_definition" => {
                    // Unwrap: the inner definition supplies the symbol
                    // name, but the unit's line range comes from the
                    // OUTER `decorated_definition` so decorator lines
                    // are folded in (analog of `attribute_item`
                    // re-absorption in 1A — see plan §Task E note (b)).
                    if let Some(inner) = child.child_by_field_name("definition") {
                        let outer_s = s; // already includes decorators
                        let outer_e = e;
                        match inner.kind() {
                            "function_definition" => {
                                if let Some(name) = node_name(&inner, src) {
                                    glue.retain(|(_, gs, _)| *gs < outer_s);
                                    flush_glue(glue, units, mod_prefix, mod_path);
                                    let sym = join_symbol(mod_prefix, mod_path, name);
                                    units.push((sym, outer_s, outer_e, true));
                                }
                            }
                            "class_definition" => {
                                if let Some(name) = node_name(&inner, src) {
                                    glue.retain(|(_, gs, _)| *gs < outer_s);
                                    flush_glue(glue, units, mod_prefix, mod_path);
                                    let sym = join_symbol(mod_prefix, mod_path, name);
                                    units.push((sym, outer_s, outer_e, true));
                                    if let Some(body) = inner.child_by_field_name("body") {
                                        let mut np = mod_path.to_vec();
                                        np.push(name.to_string());
                                        walk(body, src, mod_prefix, &np, units, glue);
                                        debug_assert!(
                                            glue.is_empty(),
                                            "inner walk must flush its glue before returning"
                                        );
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                "import_statement" | "import_from_statement" | "future_import_statement" => {
                    glue.push((1, s, e));
                }
                "expression_statement" | "assignment" | "global_statement" => {
                    glue.push((0, s, e));
                }
                _ => {}
            }
        }
        flush_glue(glue, units, mod_prefix, mod_path);
    }
    fn flush_glue(
        glue: &mut Vec<(usize, u32, u32)>,
        units: &mut Vec<(String, u32, u32, bool)>,
        mod_prefix: &str,
        mod_path: &[String],
    ) {
        if glue.is_empty() {
            return;
        }
        let s = glue.iter().map(|(_, a, _)| *a).min().unwrap();
        let e = glue.iter().map(|(_, _, b)| *b).max().unwrap();
        // Provisional label: `<module>` only if the group is exclusively
        // imports (1A's `only_mod_decls` analog). The post-pass below
        // demotes any `<module>` to `<top-level>` if the file produced
        // any real unit.
        let only_imports = glue.iter().all(|(is_import, _, _)| *is_import == 1);
        let label = if only_imports { "<module>" } else { "<top-level>" };
        units.push((join_symbol(mod_prefix, mod_path, label), s, e, false));
        glue.clear();
    }

    walk(tree.root_node(), source, mod_prefix, &[], &mut units, &mut glue);

    // `<module>` is correct only when the file produced no real unit.
    // Otherwise the import-only group becomes `<top-level>` (same
    // algorithm as 1A Gap 1). Match on the suffix so a class-nested
    // glue group (which doesn't exist in current Python AST but is
    // future-proofed) still demotes correctly.
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
            lang: Some("python".to_string()),
        };
        let block_id = id_for_block(doc_id, "code", &[], ordinal as u32, &span);
        let code = lines[(line_start as usize - 1)..(line_end as usize)].join("\n");
        blocks.push(Block::Code(CodeBlock {
            common: CommonBlock {
                block_id,
                heading_path: Vec::new(),
                source_span: span,
            },
            lang: Some("python".to_string()),
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
            concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/sample.py"),
        )
        .unwrap();
        let asset = crate::rust::tests_support::fixed_code_asset(
            "kebab_eval/metrics.py", "python",
        );
        let cfg = kebab_core::ExtractConfig::default();
        let root = std::path::PathBuf::from("/tmp");
        let ctx = kebab_core::ExtractContext {
            asset: &asset, workspace_root: &root, config: &cfg,
        };
        PythonAstExtractor::new().extract(&ctx, &bytes).unwrap()
    }

    #[test]
    fn extractor_supports_only_media_code_python() {
        let e = PythonAstExtractor::new();
        assert!(e.supports(&MediaType::Code("python".into())));
        assert!(!e.supports(&MediaType::Code("rust".into())));
        assert!(!e.supports(&MediaType::Markdown));
    }

    #[test]
    fn python_units_carry_module_prefixed_symbols() {
        let doc = extract_fixture();
        let mut syms: Vec<String> = doc.blocks.iter().map(|b| match b {
            Block::Code(c) => match &c.common.source_span {
                SourceSpan::Code { symbol, lang, .. } => {
                    assert_eq!(lang.as_deref(), Some("python"));
                    symbol.clone().unwrap()
                }
                _ => panic!("expected SourceSpan::Code"),
            },
            other => panic!("expected Block::Code, got {other:?}"),
        }).collect();
        syms.sort();
        assert!(syms.iter().any(|s| s == "kebab_eval.metrics.free"));
        assert!(syms.iter().any(|s| s == "kebab_eval.metrics.Foo"));
        assert!(syms.iter().any(|s| s == "kebab_eval.metrics.Foo.double"));
        assert!(syms.iter().any(|s| s == "kebab_eval.metrics.Foo.name"));
        assert!(syms.iter().any(|s| s == "kebab_eval.metrics.Outer"));
        assert!(syms.iter().any(|s| s == "kebab_eval.metrics.Outer.Inner"));
        assert!(syms.iter().any(|s| s == "kebab_eval.metrics.Outer.Inner.helper"));
        assert!(syms.iter().any(|s| s == "kebab_eval.metrics.with_decorator"));
        assert!(syms.iter().any(|s| s == "kebab_eval.metrics.<top-level>"));
        // The `@no_type_check` decorator on `free` is folded into its
        // unit's line range (decorated_definition unwrap).
        let free_src = doc.blocks.iter().find_map(|b| match b {
            Block::Code(c) if matches!(&c.common.source_span,
                SourceSpan::Code{symbol,..} if symbol.as_deref()==Some("kebab_eval.metrics.free")) => Some(c.code.clone()),
            _ => None,
        }).unwrap();
        assert!(free_src.contains("@no_type_check"), "decorator folded in: {free_src}");
    }

    #[test]
    fn deterministic_across_runs() {
        let a = extract_fixture();
        for _ in 0..50 { assert_eq!(extract_fixture().blocks, a.blocks); }
    }
}
