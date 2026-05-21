//! `kebab-parse-code::c` — tree-sitter C AST extractor (P10-1D Task B).
//!
//! Implements [`kebab_core::Extractor`] for [`MediaType::Code("c")`].
//! Walks the tree-sitter parse tree and emits one [`Block::Code`] per
//! top-level AST semantic unit:
//!
//! - `function_definition` → 1 unit, symbol = function name (extracted
//!   from the declarator's innermost `identifier`, handles pointer-returning
//!   functions where the declarator is wrapped in `pointer_declarator`).
//! - `struct_specifier` (named) → 1 unit, symbol = struct name.
//! - `enum_specifier` (named) → 1 unit, symbol = enum name.
//! - `union_specifier` (named) → 1 unit, symbol = union name.
//!
//! Everything else (`declaration`, `preproc_*`, `type_definition`,
//! `linkage_specification`, etc.) collapses into a single `<top-level>`
//! glue chunk. If the file produces zero units **and** zero glue, the
//! `<module>` post-pass emits one unit covering the whole file (1A-2
//! pattern).
//!
//! C symbol = function name only — no namespace, no class nesting
//! (design §3.4 C row). Per design §3.4 / §9.1 / §9 versioning.

use anyhow::Result;
use kebab_core::{
    Block, CanonicalDocument, CodeBlock, CommonBlock, Extractor, Lang, MediaType, Metadata,
    ParserVersion, Provenance, ProvenanceEvent, ProvenanceKind, SourceSpan, SourceType, TrustLevel,
    id_for_block, id_for_doc,
};
use serde_json::Map;
use time::OffsetDateTime;

use crate::scaffold::{filename_from_workspace_path, strip_extension};

pub const PARSER_VERSION: &str = "code-c-v1";

/// C AST extractor. Per-unit blocks via tree-sitter-c 0.24.2
/// (`LANGUAGE: LanguageFn`) parsed by tree-sitter 0.26.
pub struct CAstExtractor;

impl CAstExtractor {
    pub fn new() -> Self {
        Self
    }
}

impl Default for CAstExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl Extractor for CAstExtractor {
    fn supports(&self, m: &MediaType) -> bool {
        matches!(m, MediaType::Code(l) if l == "c")
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
                "kebab-parse-code: unsupported media_type for CAstExtractor: {:?}",
                asset.media_type
            );
        }

        let parser_version = self.parser_version();
        let doc_id = id_for_doc(&asset.workspace_path, &asset.asset_id, &parser_version);

        let source = String::from_utf8(bytes.to_vec())
            .map_err(|e| anyhow::anyhow!("kebab-parse-code: C source is not valid UTF-8: {e}"))?;

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
            code_lang: Some("c".to_string()),
        };

        tracing::debug!(
            target: "kebab-parse-code",
            "extracted C doc_id={} workspace_path={} units={}",
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

/// Walk down the declarator chain of a `function_definition` to find
/// the innermost `identifier` — the function name.
///
/// The tree for `int *foo(int x) { ... }` looks like:
/// ```text
/// function_definition
///   type: primitive_type  "int"
///   declarator: pointer_declarator
///     declarator: function_declarator
///       declarator: identifier  "foo"
///       parameters: parameter_list
///   body: compound_statement
/// ```
/// We walk `declarator` fields recursively until we reach an `identifier`
/// or run out of nodes. Returns `None` if no identifier is found
/// (malformed / unsupported declarator shape).
fn extract_fn_name<'a>(decl_node: tree_sitter::Node, src: &'a str) -> Option<&'a str> {
    let mut cur = decl_node;
    loop {
        match cur.kind() {
            "identifier" => return Some(&src[cur.start_byte()..cur.end_byte()]),
            // pointer_declarator, function_declarator, array_declarator,
            // attributed_declarator, parenthesized_declarator —
            // all carry a `declarator` field pointing deeper.
            _ => {
                if let Some(inner) = cur.child_by_field_name("declarator") {
                    cur = inner;
                } else {
                    // No further `declarator` field; give up.
                    return None;
                }
            }
        }
    }
}

fn build_blocks(
    source: &str,
    doc_id: &kebab_core::DocumentId,
) -> anyhow::Result<Vec<kebab_core::Block>> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_c::LANGUAGE.into())
        .map_err(|e| anyhow::anyhow!("set tree-sitter-c language: {e}"))?;
    let tree = parser
        .parse(source.as_bytes(), None)
        .ok_or_else(|| anyhow::anyhow!("tree-sitter failed to parse C source"))?;
    let lines: Vec<&str> = source.split('\n').collect();

    let root = tree.root_node();

    // units: (symbol, line_start, line_end, is_real_semantic_unit).
    // Glue is accumulated as (start, end) pairs and flushed into one
    // "<top-level>" block (or "<module>" if no real unit exists).
    let mut units: Vec<(String, u32, u32, bool)> = Vec::new();
    let mut glue: Vec<(u32, u32)> = Vec::new();

    /// Walk preceding `comment` siblings to extend the unit's line range
    /// upward, folding doc / line comments into the unit (1B pattern).
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

    let mut cur = root.walk();
    for child in root.named_children(&mut cur) {
        let s = unit_start(&child);
        let e = child.end_position().row as u32 + 1;

        match child.kind() {
            "function_definition" => {
                if let Some(decl) = child.child_by_field_name("declarator") {
                    if let Some(name) = extract_fn_name(decl, source) {
                        flush_glue(&mut glue, &mut units);
                        units.push((name.to_string(), s, e, true));
                    } else {
                        // Could not extract name — treat as glue.
                        glue.push((s, e));
                    }
                } else {
                    glue.push((s, e));
                }
            }
            "struct_specifier" | "enum_specifier" | "union_specifier" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = &source[name_node.start_byte()..name_node.end_byte()];
                    flush_glue(&mut glue, &mut units);
                    units.push((name.to_string(), s, e, true));
                } else {
                    // Anonymous struct/enum/union — glue.
                    glue.push((s, e));
                }
            }
            // Everything else: preprocessor directives, declarations
            // (typedef / global var / fn prototype), type_definition,
            // linkage_specification, etc. — all collapse into glue.
            _ => {
                glue.push((s, e));
            }
        }
    }
    flush_glue(&mut glue, &mut units);

    // Post-pass: if the file has no real semantic unit (only glue, or
    // completely empty), rename the single glue unit to "<module>" and
    // emit it. If there are zero units AND zero glue, synthesise a
    // one-line "<module>" covering the whole file.
    let has_real_unit = units.iter().any(|(_, _, _, is_real)| *is_real);

    if units.is_empty() {
        // Completely empty file or whitespace/comments only.
        let total = lines.len() as u32;
        units.push((
            "<module>".to_string(),
            1,
            total.max(1),
            false,
        ));
    }
    // If there is only glue (no real unit) the single pushed "<top-level>"
    // label should be "<module>" — rename it now.
    if !has_real_unit {
        for (sym, _, _, _) in units.iter_mut() {
            if sym == "<top-level>" {
                *sym = "<module>".to_string();
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
            lang: Some("c".to_string()),
        };
        let block_id = id_for_block(doc_id, "code", &[], ordinal as u32, &span);
        let code = lines[(line_start as usize - 1)..=(line_end as usize - 1)].join("\n");
        blocks.push(Block::Code(CodeBlock {
            common: CommonBlock {
                block_id,
                heading_path: Vec::new(),
                source_span: span,
            },
            lang: Some("c".to_string()),
            code,
        }));
    }
    Ok(blocks)
}

fn flush_glue(glue: &mut Vec<(u32, u32)>, units: &mut Vec<(String, u32, u32, bool)>) {
    if glue.is_empty() {
        return;
    }
    let s = glue.iter().map(|(a, _)| *a).min().unwrap();
    let e = glue.iter().map(|(_, b)| *b).max().unwrap();
    units.push(("<top-level>".to_string(), s, e, false));
    glue.clear();
}

// Tests for CAstExtractor (snapshot + unit assertions) are added in Task D
// alongside the C fixture file. This module is intentionally empty until then.
