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
//!
//! Edge cases: a Rust file consisting solely of comments / whitespace
//! (no fn / type / impl / mod / glue items) yields zero blocks → zero
//! chunks → not surfaced in search. Safe (no panic) and consistent with
//! "an empty page produces no chunks" in `pdf-page-v1`.

use anyhow::Result;
use kebab_core::{
    Block, CanonicalDocument, CodeBlock, CommonBlock, Extractor, Lang, MediaType, Metadata,
    ParserVersion, Provenance, ProvenanceEvent, ProvenanceKind, SourceSpan, SourceType, TrustLevel,
    id_for_block, id_for_doc,
};
use serde_json::Map;
use time::OffsetDateTime;

use crate::scaffold::{filename_from_workspace_path, strip_extension};

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
            source_id: None,
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

    // units: (symbol, line_start, line_end, is_real_semantic_unit).
    // Glue groups are pushed with a sentinel symbol + is_real=false so a
    // post-pass can decide `<module>` vs `<top-level>` (Gap 1).
    let mut units: Vec<(String, u32, u32, bool)> = Vec::new();
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
        units: &mut Vec<(String, u32, u32, bool)>,
        glue: &mut Vec<(usize, u32, u32)>,
    ) {
        // Module-path prefix for this scope. Used for both real units
        // (`format!("{prefix}{name}")`) and glue group labels
        // (`format!("{prefix}<top-level>")`) so glue from `mod inner`
        // doesn't collide on symbol with file-top-level glue and keeps
        // module context downstream. Empty at file top level -> glue
        // stays exactly `<top-level>` / `<module>`.
        let prefix = if mod_path.is_empty() {
            String::new()
        } else {
            format!("{}::", mod_path.join("::"))
        };
        let mut cur = node.walk();
        for child in node.named_children(&mut cur) {
            let s = unit_start(&child);
            let e = child.end_position().row as u32 + 1;
            match child.kind() {
                "function_item" | "struct_item" | "enum_item" | "union_item" | "trait_item"
                | "type_item" => {
                    if let Some(name) = node_name(&child, src) {
                        // Gap 2: a leading attribute/comment that this unit
                        // re-absorbs (via `unit_start`'s upward extension to
                        // `s`) must not also remain in the glue group, or it
                        // would be emitted in both chunks. Drop glue entries
                        // at/after the unit's extended start.
                        glue.retain(|(_, gs, _)| *gs < s);
                        flush_glue(glue, units, &prefix);
                        units.push((format!("{prefix}{name}"), s, e, true));
                    }
                }
                "macro_definition" => {
                    if let Some(name) = node_name(&child, src) {
                        glue.retain(|(_, gs, _)| *gs < s);
                        flush_glue(glue, units, &prefix);
                        units.push((format!("{prefix}{name}!"), s, e, true));
                    }
                }
                // `impl` blocks: emit one unit per inner `function_item`.
                // Associated consts / types / non-fn members do not become
                // their own units in 1A (plan §1A scope; HOTFIXES will log
                // if a future need arises). See inner comment below.
                "impl_item" => {
                    glue.retain(|(_, gs, _)| *gs < s);
                    flush_glue(glue, units, &prefix);
                    let ty = child
                        .child_by_field_name("type")
                        .map(|c| src[c.start_byte()..c.end_byte()].trim().to_string());
                    let tr = child
                        .child_by_field_name("trait")
                        .map(|c| src[c.start_byte()..c.end_byte()].trim().to_string());
                    let owner = tr.or(ty).unwrap_or_else(|| "<impl>".to_string());
                    if let Some(body) = child.child_by_field_name("body") {
                        let mut bc = body.walk();
                        // 1A scope: only inner `function_item` children
                        // become units. Associated consts / types and other
                        // non-fn impl members are intentionally NOT emitted
                        // as separate units in 1A (plan spec: "1 per inner
                        // function_item").
                        for m in body.named_children(&mut bc) {
                            if m.kind() == "function_item" {
                                if let Some(mn) = node_name(&m, src) {
                                    let ms = unit_start(&m);
                                    let me = m.end_position().row as u32 + 1;
                                    units.push((format!("{prefix}{owner}::{mn}"), ms, me, true));
                                }
                            }
                        }
                    }
                }
                "mod_item" => {
                    if let Some(body) = child.child_by_field_name("body") {
                        flush_glue(glue, units, &prefix);
                        let name = node_name(&child, src).unwrap_or("mod").to_string();
                        let mut np = mod_path.to_vec();
                        np.push(name);
                        walk(body, src, &np, units, glue);
                        // Invariant: `glue` is shared by `&mut` across
                        // recursive `walk` calls; every `walk` path ends with
                        // a `flush_glue`, so inner-scope glue can never leak
                        // into this outer scope's group. Assert it structurally
                        // rather than relying on that being incidental.
                        debug_assert!(
                            glue.is_empty(),
                            "inner walk must flush its glue before returning"
                        );
                    } else {
                        glue.push((1, s, e));
                    }
                }
                "use_declaration"
                | "extern_crate_declaration"
                | "const_item"
                | "static_item"
                | "attribute_item"
                | "macro_invocation" => {
                    glue.push((0, s, e));
                }
                _ => {}
            }
        }
        flush_glue(glue, units, &prefix);
    }
    fn flush_glue(
        glue: &mut Vec<(usize, u32, u32)>,
        units: &mut Vec<(String, u32, u32, bool)>,
        prefix: &str,
    ) {
        if glue.is_empty() {
            return;
        }
        let s = glue.iter().map(|(_, a, _)| *a).min().unwrap();
        let e = glue.iter().map(|(_, _, b)| *b).max().unwrap();
        // Provisional label: `<module>` only if this group is exclusively
        // bodyless `mod foo;` declarations. The final decision (Gap 1) also
        // requires the *whole file* to have produced zero real units; that
        // demotion to `<top-level>` happens in the post-pass below.
        let only_mod_decls = glue.iter().all(|(is_mod, _, _)| *is_mod == 1);
        let label = if only_mod_decls {
            "<module>"
        } else {
            "<top-level>"
        };
        // Module-path-prefix the label so glue from `mod inner` carries
        // module context (`inner::<top-level>`) and doesn't collide with
        // file-top-level glue. `prefix` is empty at file top level, so the
        // symbol stays exactly `<top-level>` / `<module>` there.
        units.push((format!("{prefix}{label}"), s, e, false));
        glue.clear();
    }

    walk(tree.root_node(), source, &[], &mut units, &mut glue);

    // Gap 1: `<module>` is correct only when the file produced no real
    // (non-glue) semantic unit at all. If any real unit exists, every glue
    // group is `<top-level>`, even a pure mod-decl group.
    let has_real_unit = units.iter().any(|(_, _, _, is_real)| *is_real);
    if has_real_unit {
        for (sym, _, _, is_real) in &mut units {
            // Match on the *suffix*: a glue group may now carry a module
            // prefix (`inner::<module>`), so demote any `…<module>` to the
            // same-prefixed `…<top-level>` rather than only the bare form.
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
            lang: Some("rust".to_string()),
        };
        let block_id = id_for_block(doc_id, "code", &[], ordinal as u32, &span);
        let code = lines[(line_start as usize - 1)..(line_end as usize)].join("\n");
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
        let bytes = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/sample.rs"
        ))
        .unwrap();
        let asset = tests_support::fixed_code_asset("crates/x/src/sample.rs", "rust");
        let cfg = kebab_core::ExtractConfig::default();
        let root = std::path::PathBuf::from("/tmp");
        let ctx = kebab_core::ExtractContext {
            asset: &asset,
            workspace_root: &root,
            config: &cfg,
        };
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
                    SourceSpan::Code {
                        symbol,
                        line_start,
                        line_end,
                        lang,
                    } => {
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
        assert!(
            parse_src.contains("/// Doc comment on a free fn."),
            "doc comment folded in: {parse_src}"
        );
    }

    /// Run the extractor on an in-memory Rust source string (no fixture
    /// file) and return (symbol, code) for every emitted block.
    fn extract_inline(source: &str) -> Vec<(String, String)> {
        let asset = tests_support::fixed_code_asset("crates/x/src/inline.rs", "rust");
        let cfg = kebab_core::ExtractConfig::default();
        let root = std::path::PathBuf::from("/tmp");
        let ctx = kebab_core::ExtractContext {
            asset: &asset,
            workspace_root: &root,
            config: &cfg,
        };
        let doc = RustAstExtractor::new()
            .extract(&ctx, source.as_bytes())
            .unwrap();
        doc.blocks
            .iter()
            .map(|b| match b {
                Block::Code(c) => match &c.common.source_span {
                    SourceSpan::Code { symbol, .. } => (symbol.clone().unwrap(), c.code.clone()),
                    _ => panic!("code block must carry SourceSpan::Code"),
                },
                other => panic!("expected Block::Code, got {other:?}"),
            })
            .collect()
    }

    #[test]
    fn module_label_scope_and_attribute_dedup() {
        // Source A (Gap 2): leading attribute is re-absorbed into the unit
        // and must NOT also form a separate <top-level> glue chunk.
        let a = extract_inline("#[derive(Debug)]\npub struct Tagged { x: u32 }\n");
        assert_eq!(a.len(), 1, "Gap 2: exactly one block, got {a:?}");
        assert_eq!(a[0].0, "Tagged");
        assert!(
            a[0].1.contains("#[derive(Debug)]"),
            "attribute folded into unit: {:?}",
            a[0].1
        );
        assert!(
            !a.iter().any(|(s, _)| s == "<top-level>"),
            "attribute must not also form a glue chunk: {a:?}"
        );

        // Source B (Gap 1): file has no real units, only bodyless mod
        // decls -> the glue group is <module>.
        let b = extract_inline("mod a;\nmod b;\n");
        assert_eq!(b.len(), 1, "one glue block, got {b:?}");
        assert_eq!(b[0].0, "<module>");

        // Source C (Gap 1): mod decls + a real unit -> the glue group is
        // <top-level>, NOT <module>, because the file has a real unit.
        let c = extract_inline("mod a;\nmod b;\npub fn f() {}\n");
        let syms: Vec<&str> = c.iter().map(|(s, _)| s.as_str()).collect();
        assert!(syms.contains(&"f"), "real unit present: {c:?}");
        assert!(
            syms.contains(&"<top-level>"),
            "mod-decl glue demoted to <top-level>: {c:?}"
        );
        assert!(
            !syms.contains(&"<module>"),
            "must not be <module> when file has a real unit: {c:?}"
        );

        // Source D (Fix 1): glue inside a bodied `mod inner` must carry the
        // module-path prefix so it doesn't collide with file-top-level glue
        // and keeps module context downstream.
        let d = extract_inline("mod inner {\n    use std::fmt;\n    pub fn helper() {}\n}\n");
        let dsyms: Vec<&str> = d.iter().map(|(s, _)| s.as_str()).collect();
        assert!(
            dsyms.contains(&"inner::helper"),
            "real unit inside mod is prefixed: {d:?}"
        );
        assert!(
            dsyms.contains(&"inner::<top-level>"),
            "glue inside mod inner is module-prefixed, not bare: {d:?}"
        );
        assert!(
            !dsyms.contains(&"<top-level>"),
            "glue inside mod inner must NOT be the bare top-level symbol: {d:?}"
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

#[cfg(test)]
pub(crate) mod tests_support {
    use kebab_core::*;
    use time::OffsetDateTime;
    /// Test-only `RawAsset` builder for any tree-sitter language. Shared
    /// across `rust.rs` / `python.rs` / future TS+JS extractor tests so all
    /// in-crate code-extractor tests use a single canonical fixture shape.
    pub fn fixed_code_asset(workspace_path: &str, code_lang: &str) -> RawAsset {
        RawAsset {
            asset_id: AssetId("a".repeat(64)),
            source_uri: SourceUri::File(std::path::PathBuf::from(workspace_path)),
            workspace_path: WorkspacePath(workspace_path.to_string()),
            media_type: MediaType::Code(code_lang.to_string()),
            byte_len: 0,
            checksum: Checksum("b".repeat(64)),
            discovered_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
            stored: AssetStorage::Reference {
                path: std::path::PathBuf::from(workspace_path),
                sha: Checksum("b".repeat(64)),
            },
        }
    }
}
