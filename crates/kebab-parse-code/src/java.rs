//! `kebab-parse-code::java` — tree-sitter Java AST extractor (P10-1C-JK Task D).
//!
//! Implements [`kebab_core::Extractor`] for [`MediaType::Code("java")`].
//! Walks the tree-sitter parse tree and emits one [`Block::Code`] per
//! top-level AST semantic unit (class / interface / enum / record /
//! annotation-type at any nesting level, plus methods + constructors
//! inside class / interface / record bodies), each carrying
//! [`SourceSpan::Code`] with the unit's dotted self-reference symbol
//! path (design §3.4 Java row). Glue declarations (`import`) collapse
//! into one grouped `<top-level>` (or `<module>`) unit.
//!
//! Like the Go extractor, Java's package identity comes from the
//! source itself (the `package_declaration` clause), not from the
//! workspace file path — `extract_package` reads it from the AST. If
//! the clause is missing the prefix falls back to `"<unknown>"`.
//!
//! Class/interface/record bodies are recursed (1B Python pattern):
//! the type name is pushed onto `mod_path` so methods and nested
//! types become `<pkg>.<Outer>.<Inner>.<method>`. Constructors use
//! the Java convention `<pkg>.<...>.<Class>.<ClassName>` (name
//! duplicated, per design §3.4). Enum bodies are not recursed for
//! the 1차 cut — enum constants are not emitted as units.
//!
//! Javadoc (`/** ... */` → `block_comment`) and line comments
//! immediately preceding an item are folded into that item's line
//! range via `unit_start` (1B pattern). Annotations are children of
//! the declaration node itself (inside `modifiers`), so they are
//! already part of the declaration's span — no separate unwrap arm.
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

pub const PARSER_VERSION: &str = "code-java-v1";

/// Java AST extractor. Per-unit blocks via tree-sitter-java 0.23
/// (`LANGUAGE: LanguageFn`) parsed by tree-sitter 0.26.
pub struct JavaAstExtractor;

impl JavaAstExtractor {
    pub fn new() -> Self {
        Self
    }
}

impl Default for JavaAstExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl Extractor for JavaAstExtractor {
    fn supports(&self, m: &MediaType) -> bool {
        matches!(m, MediaType::Code(l) if l == "java")
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
                "kebab-parse-code: unsupported media_type for JavaAstExtractor: {:?}",
                asset.media_type
            );
        }

        let parser_version = self.parser_version();
        let doc_id = id_for_doc(&asset.workspace_path, &asset.asset_id, &parser_version);

        let source = String::from_utf8(bytes.to_vec())
            .map_err(|e| anyhow::anyhow!("kebab-parse-code: Java source is not valid UTF-8: {e}"))?;

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
            code_lang: Some("java".to_string()),
        };

        tracing::debug!(
            target: "kebab-parse-code",
            "extracted Java doc_id={} workspace_path={} units={}",
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

/// p10-1C-JK: extract `package` declaration text from a tree-sitter-java
/// `program`. Returns `None` if no `package_declaration` (default-package
/// Java file). The package_declaration's named children are either a
/// single `identifier` (single-segment package, rare) or a
/// `scoped_identifier` (dotted, common). Per design §3.4 Java row.
fn extract_package(root: tree_sitter::Node, src: &str) -> Option<String> {
    let mut cur = root.walk();
    for child in root.named_children(&mut cur) {
        if child.kind() == "package_declaration" {
            let mut c2 = child.walk();
            for sub in child.named_children(&mut c2) {
                if sub.kind() == "scoped_identifier" || sub.kind() == "identifier" {
                    return Some(src[sub.start_byte()..sub.end_byte()].to_string());
                }
            }
        }
    }
    None
}

/// Walk preceding `line_comment` / `block_comment` siblings to extend
/// the unit's line range upward, folding leading Javadoc / line
/// comments into the unit. Annotations live INSIDE `modifiers` on the
/// declaration node itself, so their lines are already inside
/// `n.start_position()` — no separate unwrap arm is needed for them.
fn unit_start(n: &tree_sitter::Node) -> u32 {
    let mut start = n.start_position().row as u32 + 1;
    let mut prev = n.prev_sibling();
    while let Some(p) = prev {
        let k = p.kind();
        if k == "line_comment" || k == "block_comment" {
            start = p.start_position().row as u32 + 1;
            prev = p.prev_sibling();
        } else {
            break;
        }
    }
    start
}

fn node_name_text<'a>(n: &tree_sitter::Node, src: &'a str) -> Option<&'a str> {
    n.child_by_field_name("name")
        .map(|c| &src[c.start_byte()..c.end_byte()])
}

fn build_blocks(
    source: &str,
    doc_id: &kebab_core::DocumentId,
) -> anyhow::Result<Vec<kebab_core::Block>> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_java::LANGUAGE.into())
        .map_err(|e| anyhow::anyhow!("set tree-sitter-java language: {e}"))?;
    let tree = parser
        .parse(source.as_bytes(), None)
        .ok_or_else(|| anyhow::anyhow!("tree-sitter failed to parse Java source"))?;
    let lines: Vec<&str> = source.split('\n').collect();

    let root = tree.root_node();
    let mod_prefix = extract_package(root, source).unwrap_or_else(|| "<unknown>".to_string());

    // units: (symbol, line_start, line_end, is_real_semantic_unit).
    // Glue groups are pushed with a sentinel symbol + is_real=false so a
    // post-pass can decide `<module>` vs `<top-level>` (1B/1C-Go pattern).
    let mut units: Vec<(String, u32, u32, bool)> = Vec::new();
    // (is_import 0/1, s, e). `is_import` flags `import_declaration` —
    // used by the glue flush to pick `<module>` vs `<top-level>`
    // provisional label.
    let mut glue: Vec<(usize, u32, u32)> = Vec::new();

    walk_top(root, source, &mod_prefix, &mut units, &mut glue);

    // `<module>` is correct only when the file produced no real unit.
    // Otherwise the import-only group becomes `<top-level>` (same
    // post-pass as 1B / 1C-Go).
    let has_real_unit = units.iter().any(|(_, _, _, is_real)| *is_real);
    if has_real_unit {
        for (sym, _, _, is_real) in units.iter_mut() {
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
            lang: Some("java".to_string()),
        };
        let block_id = id_for_block(doc_id, "code", &[], ordinal as u32, &span);
        let code = lines[(line_start as usize - 1)..=(line_end as usize - 1)].join("\n");
        blocks.push(Block::Code(CodeBlock {
            common: CommonBlock {
                block_id,
                heading_path: Vec::new(),
                source_span: span,
            },
            lang: Some("java".to_string()),
            code,
        }));
    }
    Ok(blocks)
}

/// Walk the file's top-level children — `program` named children:
/// `package_declaration` (handled by `extract_package`), `import_declaration`
/// (glue), and the five type declarations (`class` / `interface` /
/// `enum` / `record` / `annotation_type`). Type-declaration bodies
/// are recursed via [`walk_body`] with the type name pushed onto
/// `mod_path` (1B Python pattern). Enum bodies are NOT recursed
/// (1차 cut — see module-level doc).
fn walk_top(
    node: tree_sitter::Node,
    src: &str,
    mod_prefix: &str,
    units: &mut Vec<(String, u32, u32, bool)>,
    glue: &mut Vec<(usize, u32, u32)>,
) {
    let mod_path: &[String] = &[];
    let mut cur = node.walk();
    for child in node.named_children(&mut cur) {
        let s = unit_start(&child);
        let e = child.end_position().row as u32 + 1;
        match child.kind() {
            "class_declaration"
            | "interface_declaration"
            | "record_declaration" => {
                if let Some(name) = node_name_text(&child, src) {
                    glue.retain(|(_, gs, _)| *gs < s);
                    flush_glue(glue, units, mod_prefix, mod_path);
                    let sym = join_symbol(mod_prefix, mod_path, name);
                    units.push((sym, s, e, true));
                    if let Some(body) = child.child_by_field_name("body") {
                        let np: Vec<String> = vec![name.to_string()];
                        walk_body(body, src, mod_prefix, &np, units);
                    }
                }
            }
            "enum_declaration" => {
                if let Some(name) = node_name_text(&child, src) {
                    glue.retain(|(_, gs, _)| *gs < s);
                    flush_glue(glue, units, mod_prefix, mod_path);
                    let sym = join_symbol(mod_prefix, mod_path, name);
                    units.push((sym, s, e, true));
                    // Enum body NOT recursed for 1차 — enum constants are
                    // not emitted as units, and method declarations inside
                    // enum bodies (rare) live under `enum_body_declarations`
                    // not `class_body`. Skip per design §3.4 1차 scope.
                }
            }
            "annotation_type_declaration" => {
                if let Some(name) = node_name_text(&child, src) {
                    glue.retain(|(_, gs, _)| *gs < s);
                    flush_glue(glue, units, mod_prefix, mod_path);
                    let sym = join_symbol(mod_prefix, mod_path, name);
                    units.push((sym, s, e, true));
                }
            }
            "import_declaration" => {
                glue.push((1, s, e));
            }
            // package_declaration is handled by `extract_package`; no
            // glue entry — it's structural metadata, not a unit.
            _ => {}
        }
    }
    flush_glue(glue, units, mod_prefix, mod_path);
}

/// Walk a `class_body` / `interface_body` (or record's `class_body`).
/// Emits one unit per method / constructor, and recurses into nested
/// type declarations. Field declarations are NOT emitted (would
/// explode unit count). `compact_constructor_declaration` (records)
/// is handled the same as `constructor_declaration`.
///
/// No `glue` parameter: Java does not have imports inside type
/// bodies — they only appear at file top level, handled by
/// [`walk_top`].
fn walk_body(
    body: tree_sitter::Node,
    src: &str,
    mod_prefix: &str,
    mod_path: &[String],
    units: &mut Vec<(String, u32, u32, bool)>,
) {
    let mut cur = body.walk();
    for child in body.named_children(&mut cur) {
        let s = unit_start(&child);
        let e = child.end_position().row as u32 + 1;
        match child.kind() {
            "method_declaration"
            | "constructor_declaration"
            | "compact_constructor_declaration" => {
                // Constructor: name field equals the class name. Per
                // design §3.4 Java convention, symbol is
                // `<pkg>.<mod_path>.<ClassName>` with the constructor
                // name (== class name) as the trailing segment. This
                // means the symbol duplicates the class name (e.g.
                // `com.x.Foo.Foo`), which is the documented convention.
                if let Some(name) = node_name_text(&child, src) {
                    let sym = join_symbol(mod_prefix, mod_path, name);
                    units.push((sym, s, e, true));
                }
            }
            "class_declaration"
            | "interface_declaration"
            | "record_declaration"
            | "enum_declaration"
            | "annotation_type_declaration" => {
                // Nested type — emit unit, then recurse into its body
                // (skipped for enum + annotation_type per 1차 scope).
                let name = match node_name_text(&child, src) {
                    Some(n) => n,
                    None => continue,
                };
                let sym = join_symbol(mod_prefix, mod_path, name);
                units.push((sym, s, e, true));
                if child.kind() != "enum_declaration"
                    && child.kind() != "annotation_type_declaration"
                {
                    if let Some(inner_body) = child.child_by_field_name("body") {
                        let mut np = mod_path.to_vec();
                        np.push(name.to_string());
                        walk_body(inner_body, src, mod_prefix, &np, units);
                    }
                }
            }
            // field_declaration, static_initializer, block: NOT emitted.
            _ => {}
        }
    }
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
    // imports (1A's `only_mod_decls` analog). The post-pass demotes any
    // `<module>` to `<top-level>` if the file produced any real unit.
    let only_imports = glue.iter().all(|(is_import, _, _)| *is_import == 1);
    let label = if only_imports { "<module>" } else { "<top-level>" };
    units.push((join_symbol(mod_prefix, mod_path, label), s, e, false));
    glue.clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use kebab_core::{Block, MediaType, SourceSpan};

    fn extract_fixture() -> kebab_core::CanonicalDocument {
        let bytes = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/sample.java"
        ))
        .unwrap();
        let asset =
            crate::rust::tests_support::fixed_code_asset("crates/x/src/sample.java", "java");
        let cfg = kebab_core::ExtractConfig::default();
        let root = std::path::PathBuf::from("/tmp");
        let ctx = kebab_core::ExtractContext {
            asset: &asset,
            workspace_root: &root,
            config: &cfg,
        };
        JavaAstExtractor::new().extract(&ctx, &bytes).unwrap()
    }

    #[test]
    fn extractor_supports_only_media_code_java() {
        let e = JavaAstExtractor::new();
        assert!(e.supports(&MediaType::Code("java".into())));
        assert!(!e.supports(&MediaType::Code("rust".into())));
        assert!(!e.supports(&MediaType::Markdown));
    }

    #[test]
    fn java_units_match_design_3_4_symbols() {
        let doc = extract_fixture();
        let mut syms: Vec<String> = doc
            .blocks
            .iter()
            .filter_map(|b| match b {
                Block::Code(c) => match &c.common.source_span {
                    SourceSpan::Code { symbol, lang, .. } => {
                        assert_eq!(lang.as_deref(), Some("java"));
                        symbol.clone()
                    }
                    _ => None,
                },
                _ => None,
            })
            .collect();
        syms.sort();
        // package extracted from source = com.kebab.chunk
        assert!(
            syms.iter().any(|s| s == "com.kebab.chunk.MdHeadingV1Chunker"),
            "got {syms:?}"
        );
        // constructor — Java convention is class-name-as-method-name
        assert!(
            syms.iter()
                .any(|s| s == "com.kebab.chunk.MdHeadingV1Chunker.MdHeadingV1Chunker"),
            "got {syms:?}"
        );
        assert!(
            syms.iter()
                .any(|s| s == "com.kebab.chunk.MdHeadingV1Chunker.chunkDoc"),
            "got {syms:?}"
        );
        assert!(
            syms.iter()
                .any(|s| s == "com.kebab.chunk.MdHeadingV1Chunker.getName"),
            "got {syms:?}"
        );
        // static nested class
        assert!(
            syms.iter()
                .any(|s| s == "com.kebab.chunk.MdHeadingV1Chunker.Builder"),
            "got {syms:?}"
        );
        assert!(
            syms.iter()
                .any(|s| s == "com.kebab.chunk.MdHeadingV1Chunker.Builder.withName"),
            "got {syms:?}"
        );
        assert!(
            syms.iter()
                .any(|s| s == "com.kebab.chunk.MdHeadingV1Chunker.Builder.build"),
            "got {syms:?}"
        );
        // package-private interface + enum
        assert!(
            syms.iter().any(|s| s == "com.kebab.chunk.Stringer"),
            "got {syms:?}"
        );
        assert!(
            syms.iter().any(|s| s == "com.kebab.chunk.Mode"),
            "got {syms:?}"
        );
        // import grouped as <top-level>
        assert!(
            syms.iter().any(|s| s == "com.kebab.chunk.<top-level>"),
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
