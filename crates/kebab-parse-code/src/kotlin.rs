//! `kebab-parse-code::kotlin` — tree-sitter Kotlin AST extractor (P10-1C-JK Task G).
//!
//! Implements [`kebab_core::Extractor`] for [`MediaType::Code("kotlin")`].
//! Mirrors the Java extractor (JVM family, source-side `package` extraction +
//! class-nesting) with Kotlin-specific adjustments:
//!
//! * Root is `source_file` (not `program`).
//! * `package_header` carries a single `qualified_identifier` child whose
//!   slice text IS the dotted package path — never a bare `identifier`
//!   sub-form for the package (the grammar always wraps a single segment
//!   in `qualified_identifier` too).
//! * `class_declaration` covers `class`, `data class`, `sealed class`,
//!   `enum class`, AND `interface` — Kotlin uses ONE node kind with a
//!   `modifiers` child rather than separate `interface_declaration` /
//!   `enum_declaration` nodes (verified via tree-sitter-kotlin-ng
//!   `node-types.json`).
//! * The body child of `class_declaration` is either `class_body` (normal
//!   classes / interfaces) OR `enum_class_body` (enum class). Neither
//!   carries a `body` field name, so it is matched by kind, not by
//!   `child_by_field_name("body")`.
//! * `companion_object` is a SEPARATE node kind (not `object_declaration`
//!   with a modifier). Its `name` field is OPTIONAL — when omitted (the
//!   common case `companion object { ... }`) the symbol uses the
//!   implicit Kotlin convention name `Companion`.
//! * `object_declaration` (named singleton) carries a `name` field and a
//!   `class_body` child.
//! * `function_declaration` may appear at top level (Kotlin top-level
//!   function) AND inside `class_body` — same node kind, the
//!   `mod_path` state distinguishes the two emit forms.
//!
//! Enum bodies (`enum_class_body`) are NOT recursed for the 1차 cut —
//! `enum_entry` declarations are not emitted as units, matching the
//! Java extractor's enum policy (design §3.4 1차 scope).
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

pub const PARSER_VERSION: &str = "code-kotlin-v1";

/// Kotlin AST extractor. Per-unit blocks via tree-sitter-kotlin-ng 1.1
/// (`LANGUAGE: LanguageFn`) parsed by tree-sitter 0.26.
pub struct KotlinAstExtractor;

impl KotlinAstExtractor {
    pub fn new() -> Self {
        Self
    }
}

impl Default for KotlinAstExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl Extractor for KotlinAstExtractor {
    fn supports(&self, m: &MediaType) -> bool {
        matches!(m, MediaType::Code(l) if l == "kotlin")
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
                "kebab-parse-code: unsupported media_type for KotlinAstExtractor: {:?}",
                asset.media_type
            );
        }

        let parser_version = self.parser_version();
        let doc_id = id_for_doc(&asset.workspace_path, &asset.asset_id, &parser_version);

        let source = String::from_utf8(bytes.to_vec()).map_err(|e| {
            anyhow::anyhow!("kebab-parse-code: Kotlin source is not valid UTF-8: {e}")
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
            code_lang: Some("kotlin".to_string()),
        };

        tracing::debug!(
            target: "kebab-parse-code",
            "extracted Kotlin doc_id={} workspace_path={} units={}",
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

/// p10-1C-JK: extract `package` declaration text from a tree-sitter-kotlin
/// `source_file`. Returns `None` if no `package_header` (default-package
/// Kotlin file). The package_header's single named child is a
/// `qualified_identifier`; its slice text is the dotted path. Per design
/// §3.4 Kotlin row.
fn extract_package(root: tree_sitter::Node, src: &str) -> Option<String> {
    let mut cur = root.walk();
    for child in root.named_children(&mut cur) {
        if child.kind() == "package_header" {
            let mut c2 = child.walk();
            for sub in child.named_children(&mut c2) {
                let k = sub.kind();
                if k == "qualified_identifier" || k == "identifier" {
                    return Some(src[sub.start_byte()..sub.end_byte()].to_string());
                }
            }
        }
    }
    None
}

/// Walk preceding `line_comment` / `block_comment` siblings to extend
/// the unit's line range upward, folding leading KDoc / line comments
/// into the unit. Modifiers / annotations live INSIDE the declaration
/// node itself, so their lines are already inside `n.start_position()`.
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

/// Find the first child of a node with one of the given kinds. Used to
/// locate `class_body` / `enum_class_body` on `class_declaration` since
/// the kotlin grammar attaches them without a `body` field name.
fn first_child_of_kinds<'a>(
    n: &tree_sitter::Node<'a>,
    kinds: &[&str],
) -> Option<tree_sitter::Node<'a>> {
    let mut cur = n.walk();
    n.named_children(&mut cur)
        .find(|child| kinds.contains(&child.kind()))
}

/// `true` iff a `class_declaration` carries the `enum` class modifier.
/// Detected by walking `modifiers` → `class_modifier` and checking the
/// child text. The grammar exposes "enum" / "sealed" / "data" /
/// "annotation" / "inner" as named `class_modifier` children of
/// `modifiers`. We only need to know about "enum" to decide whether to
/// look for `class_body` or `enum_class_body` and whether to skip body
/// recursion.
fn class_decl_is_enum(n: &tree_sitter::Node, src: &str) -> bool {
    let mut cur = n.walk();
    for child in n.named_children(&mut cur) {
        if child.kind() == "modifiers" {
            let mut c2 = child.walk();
            for sub in child.named_children(&mut c2) {
                if sub.kind() == "class_modifier" {
                    let text = &src[sub.start_byte()..sub.end_byte()];
                    if text == "enum" {
                        return true;
                    }
                }
            }
        }
    }
    false
}

fn build_blocks(
    source: &str,
    doc_id: &kebab_core::DocumentId,
) -> anyhow::Result<Vec<kebab_core::Block>> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_kotlin_ng::LANGUAGE.into())
        .map_err(|e| anyhow::anyhow!("set tree-sitter-kotlin-ng language: {e}"))?;
    let tree = parser
        .parse(source.as_bytes(), None)
        .ok_or_else(|| anyhow::anyhow!("tree-sitter failed to parse Kotlin source"))?;
    let lines: Vec<&str> = source.split('\n').collect();

    let root = tree.root_node();
    let mod_prefix = extract_package(root, source).unwrap_or_else(|| "<unknown>".to_string());

    // units: (symbol, line_start, line_end, is_real_semantic_unit).
    // Glue groups are pushed with a sentinel symbol + is_real=false so a
    // post-pass can decide `<module>` vs `<top-level>` (JVM family pattern).
    let mut units: Vec<(String, u32, u32, bool)> = Vec::new();
    // (is_import 0/1, s, e). `is_import` flags `import` — used by the
    // glue flush to pick `<module>` vs `<top-level>` provisional label.
    let mut glue: Vec<(usize, u32, u32)> = Vec::new();

    walk_top(root, source, &mod_prefix, &mut units, &mut glue);

    // `<module>` is correct only when the file produced no real unit.
    // Otherwise the import-only group becomes `<top-level>` (same
    // post-pass as 1B / 1C-Go / Java).
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
            lang: Some("kotlin".to_string()),
        };
        let block_id = id_for_block(doc_id, "code", &[], ordinal as u32, &span);
        let code = lines[(line_start as usize - 1)..=(line_end as usize - 1)].join("\n");
        blocks.push(Block::Code(CodeBlock {
            common: CommonBlock {
                block_id,
                heading_path: Vec::new(),
                source_span: span,
            },
            lang: Some("kotlin".to_string()),
            code,
        }));
    }
    Ok(blocks)
}

/// Walk the file's top-level children — `source_file` named children:
/// `package_header` (handled by `extract_package`), `import` (glue),
/// `class_declaration` (class / interface / enum class), `object_declaration`,
/// `function_declaration` (top-level), `property_declaration` (top-level),
/// `type_alias` (currently treated as glue). Class / object bodies are
/// recursed via [`walk_body`] with the type name pushed onto `mod_path`
/// (JVM family pattern). Enum bodies are NOT recursed (1차 cut).
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
            "class_declaration" => {
                // Covers class / data class / sealed class / interface /
                // enum class — single grammar node, the modifiers child
                // distinguishes them. The body is `class_body` for
                // non-enum and `enum_class_body` for enum class; both
                // attach without a `body` field name.
                if let Some(name) = node_name_text(&child, src) {
                    glue.retain(|(_, gs, _)| *gs < s);
                    flush_glue(glue, units, mod_prefix, mod_path);
                    let sym = join_symbol(mod_prefix, mod_path, name);
                    units.push((sym, s, e, true));
                    let is_enum = class_decl_is_enum(&child, src);
                    if !is_enum {
                        if let Some(body) = first_child_of_kinds(&child, &["class_body"]) {
                            let np: Vec<String> = vec![name.to_string()];
                            walk_body(body, src, mod_prefix, &np, units);
                        }
                    }
                    // enum_class_body NOT recursed — enum constants are
                    // not emitted as units (1차 scope, matches Java).
                }
            }
            "object_declaration" => {
                // Singleton object — name field is required by the grammar.
                if let Some(name) = node_name_text(&child, src) {
                    glue.retain(|(_, gs, _)| *gs < s);
                    flush_glue(glue, units, mod_prefix, mod_path);
                    let sym = join_symbol(mod_prefix, mod_path, name);
                    units.push((sym, s, e, true));
                    if let Some(body) = first_child_of_kinds(&child, &["class_body"]) {
                        let np: Vec<String> = vec![name.to_string()];
                        walk_body(body, src, mod_prefix, &np, units);
                    }
                }
            }
            "function_declaration" => {
                // Top-level Kotlin function (unlike Java).
                if let Some(name) = node_name_text(&child, src) {
                    glue.retain(|(_, gs, _)| *gs < s);
                    flush_glue(glue, units, mod_prefix, mod_path);
                    let sym = join_symbol(mod_prefix, mod_path, name);
                    units.push((sym, s, e, true));
                }
            }
            "import" => {
                glue.push((1, s, e));
            }
            // `property_declaration` (top-level val/var) and `type_alias`
            // are not emitted as standalone units in the 1차 cut — they
            // glue into the import group instead. `package_header` is
            // handled by `extract_package` (structural metadata, not a
            // unit).
            _ => {}
        }
    }
    flush_glue(glue, units, mod_prefix, mod_path);
}

/// Walk a `class_body` (or object's `class_body`). Emits one unit per
/// method / secondary constructor and recurses into nested type
/// declarations + companion objects. Property declarations are NOT
/// emitted (would explode unit count, parallel to Java field policy).
///
/// `companion_object` carries an optional `name` field — when omitted
/// (the common case `companion object { ... }`) the implicit Kotlin
/// convention name `Companion` is used.
///
/// No `glue` parameter: Kotlin imports are file-level only.
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
            "function_declaration" => {
                if let Some(name) = node_name_text(&child, src) {
                    let sym = join_symbol(mod_prefix, mod_path, name);
                    units.push((sym, s, e, true));
                }
            }
            "secondary_constructor" => {
                // Kotlin secondary constructor — no `name` field on the
                // grammar node. Per design §3.4 (Java JVM convention) the
                // symbol uses the enclosing class name as the trailing
                // segment (matches the Java `<pkg>.<...>.<Class>.<Class>`
                // duplication for constructors).
                if let Some(class_name) = mod_path.last() {
                    let sym = join_symbol(mod_prefix, mod_path, class_name);
                    units.push((sym, s, e, true));
                }
            }
            "companion_object" => {
                // Companion's name field is OPTIONAL — fall back to the
                // Kotlin implicit name `Companion`.
                let name: &str = node_name_text(&child, src).unwrap_or("Companion");
                let sym = join_symbol(mod_prefix, mod_path, name);
                units.push((sym, s, e, true));
                if let Some(inner_body) = first_child_of_kinds(&child, &["class_body"]) {
                    let mut np = mod_path.to_vec();
                    np.push(name.to_string());
                    walk_body(inner_body, src, mod_prefix, &np, units);
                }
            }
            "class_declaration" => {
                let name = match node_name_text(&child, src) {
                    Some(n) => n,
                    None => continue,
                };
                let sym = join_symbol(mod_prefix, mod_path, name);
                units.push((sym, s, e, true));
                let is_enum = class_decl_is_enum(&child, src);
                if !is_enum {
                    if let Some(inner_body) = first_child_of_kinds(&child, &["class_body"]) {
                        let mut np = mod_path.to_vec();
                        np.push(name.to_string());
                        walk_body(inner_body, src, mod_prefix, &np, units);
                    }
                }
            }
            "object_declaration" => {
                let name = match node_name_text(&child, src) {
                    Some(n) => n,
                    None => continue,
                };
                let sym = join_symbol(mod_prefix, mod_path, name);
                units.push((sym, s, e, true));
                if let Some(inner_body) = first_child_of_kinds(&child, &["class_body"]) {
                    let mut np = mod_path.to_vec();
                    np.push(name.to_string());
                    walk_body(inner_body, src, mod_prefix, &np, units);
                }
            }
            // property_declaration, anonymous_initializer: NOT emitted.
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
    // imports. The post-pass demotes any `<module>` to `<top-level>` if
    // the file produced any real unit.
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
            "/tests/fixtures/sample.kt"
        ))
        .unwrap();
        let asset =
            crate::rust::tests_support::fixed_code_asset("crates/x/src/sample.kt", "kotlin");
        let cfg = kebab_core::ExtractConfig::default();
        let root = std::path::PathBuf::from("/tmp");
        let ctx = kebab_core::ExtractContext {
            asset: &asset,
            workspace_root: &root,
            config: &cfg,
        };
        KotlinAstExtractor::new().extract(&ctx, &bytes).unwrap()
    }

    #[test]
    fn extractor_supports_only_media_code_kotlin() {
        let e = KotlinAstExtractor::new();
        assert!(e.supports(&MediaType::Code("kotlin".into())));
        assert!(!e.supports(&MediaType::Code("java".into())));
        assert!(!e.supports(&MediaType::Code("rust".into())));
        assert!(!e.supports(&MediaType::Markdown));
    }

    #[test]
    fn kotlin_units_match_design_3_4_symbols() {
        let doc = extract_fixture();
        let mut syms: Vec<String> = doc
            .blocks
            .iter()
            .filter_map(|b| match b {
                Block::Code(c) => match &c.common.source_span {
                    SourceSpan::Code { symbol, lang, .. } => {
                        assert_eq!(lang.as_deref(), Some("kotlin"));
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
            syms.iter()
                .any(|s| s == "com.kebab.chunk.MdHeadingV1Chunker"),
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
        // Implicit companion object name = Companion (grammar leaves the
        // name field unset; the extractor fills it in).
        assert!(
            syms.iter()
                .any(|s| s == "com.kebab.chunk.MdHeadingV1Chunker.Companion"),
            "got {syms:?}"
        );
        assert!(
            syms.iter()
                .any(|s| s == "com.kebab.chunk.MdHeadingV1Chunker.Companion.withName"),
            "got {syms:?}"
        );
        // interface — also via class_declaration in the grammar
        assert!(
            syms.iter().any(|s| s == "com.kebab.chunk.Stringer"),
            "got {syms:?}"
        );
        // enum class — also via class_declaration; body NOT recursed
        assert!(
            syms.iter().any(|s| s == "com.kebab.chunk.Mode"),
            "got {syms:?}"
        );
        // Kotlin top-level fn — unlike Java
        assert!(
            syms.iter().any(|s| s == "com.kebab.chunk.freeFunction"),
            "got {syms:?}"
        );
        // Singleton object + its method
        assert!(
            syms.iter().any(|s| s == "com.kebab.chunk.Singleton"),
            "got {syms:?}"
        );
        assert!(
            syms.iter().any(|s| s == "com.kebab.chunk.Singleton.ping"),
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
