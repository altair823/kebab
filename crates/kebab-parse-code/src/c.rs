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

pub const PARSER_VERSION: &str = "code-c-v2";

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
                    // Anonymous struct/enum/union at the top level (not
                    // wrapped in typedef) — glue. typedef-wrapped case
                    // is recovered in the `type_definition` arm below.
                    glue.push((s, e));
                }
            }
            "type_definition" => {
                // v0.17.0 PR-B: typedef-wrapped anonymous aggregate
                // recovery. `typedef struct { ... } Foo;` exposes only
                // the alias `Foo` as a useful symbol — the inner
                // struct_specifier has no `name` field. Pre-v0.17.0
                // this whole construct collapsed into glue and hid the
                // alias from search (HOTFIXES 2026-05-21). v2 recovers
                // the alias from the `declarator` field and emits a
                // synthetic unit so `Citation::Code.symbol = "Foo"`.
                // Plain `typedef int MyInt;` (no inner aggregate) stays
                // glue — there's no struct body to name.
                if let Some(name) = recover_typedef_alias(child, source) {
                    flush_glue(&mut glue, &mut units);
                    units.push((name, s, e, true));
                } else {
                    glue.push((s, e));
                }
            }
            // Everything else: preprocessor directives, plain declarations
            // (global var / fn prototype), linkage_specification, etc.
            // — all collapse into glue.
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
        for (sym, _, _, _) in &mut units {
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
        let code = lines[(line_start as usize - 1)..(line_end as usize)].join("\n");
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

/// v0.17.0 PR-B: try to recover the typedef alias name from a
/// `type_definition` node *iff* the inner type-specifier is an
/// anonymous struct/enum/union. Returns `None` for any other shape
/// (named aggregate handled elsewhere, plain type alias has no body
/// worth naming).
fn recover_typedef_alias(node: tree_sitter::Node, source: &str) -> Option<String> {
    let mut has_anon_aggregate = false;
    let mut cursor = node.walk();
    for sub in node.children(&mut cursor) {
        match sub.kind() {
            "struct_specifier" | "enum_specifier" | "union_specifier" => {
                if sub.child_by_field_name("name").is_none() {
                    has_anon_aggregate = true;
                } else {
                    // Named inner aggregate (e.g. `typedef struct Pt {...} P;`)
                    // — the named struct itself is the primary symbol and
                    // is *not* extracted at the top level today (it lives
                    // inside `type_definition`, not as a sibling
                    // `struct_specifier`). For v2 we keep behavior conservative:
                    // return None so the type_definition stays glue, matching
                    // pre-v2 behavior for this minor case. Real-world C tends
                    // to use one of: bare named struct, typedef alias only,
                    // or typedef on anonymous body — the latter is what we fix.
                    return None;
                }
            }
            _ => {}
        }
    }
    if !has_anon_aggregate {
        return None;
    }
    let decl = node.child_by_field_name("declarator")?;
    extract_typedef_alias_name(decl, source).map(str::to_string)
}

/// Extract the typedef alias identifier from a declarator subtree.
/// Handles the common shapes: direct `type_identifier`, or one wrapped
/// in pointer / function declarator nodes (the alias is always the
/// rightmost `type_identifier` descendant).
fn extract_typedef_alias_name<'a>(
    decl: tree_sitter::Node,
    source: &'a str,
) -> Option<&'a str> {
    if decl.kind() == "type_identifier" {
        return Some(&source[decl.start_byte()..decl.end_byte()]);
    }
    let mut cursor = decl.walk();
    for sub in decl.children(&mut cursor) {
        if let Some(found) = extract_typedef_alias_name(sub, source) {
            return Some(found);
        }
    }
    None
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
pub(crate) mod tests_support {
    use kebab_core::*;
    use std::path::PathBuf;
    use time::OffsetDateTime;

    pub fn fixed_code_asset(workspace_path: &str, lang: &str) -> RawAsset {
        RawAsset {
            asset_id: AssetId("a".repeat(64)),
            source_uri: SourceUri::File(PathBuf::from(workspace_path)),
            workspace_path: WorkspacePath(workspace_path.to_string()),
            media_type: MediaType::Code(lang.to_string()),
            byte_len: 0,
            checksum: Checksum("b".repeat(64)),
            discovered_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
            stored: AssetStorage::Reference {
                path: PathBuf::from(workspace_path),
                sha: Checksum("b".repeat(64)),
            },
        }
    }

    pub fn extract_c(src: &str, path: &str) -> kebab_core::CanonicalDocument {
        use super::CAstExtractor;
        use kebab_core::Extractor;
        let asset = fixed_code_asset(path, "c");
        let cfg = ExtractConfig::default();
        let root = PathBuf::from("/tmp");
        let ctx = ExtractContext {
            asset: &asset,
            workspace_root: &root,
            config: &cfg,
        };
        CAstExtractor::new().extract(&ctx, src.as_bytes()).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kebab_core::{Block, MediaType, SourceSpan};

    fn syms(doc: &kebab_core::CanonicalDocument) -> Vec<String> {
        doc.blocks
            .iter()
            .filter_map(|b| match b {
                Block::Code(c) => match &c.common.source_span {
                    SourceSpan::Code { symbol, .. } => symbol.clone(),
                    _ => None,
                },
                _ => None,
            })
            .collect()
    }

    #[test]
    fn extractor_supports_only_media_code_c() {
        let e = CAstExtractor::new();
        assert!(e.supports(&MediaType::Code("c".into())));
        assert!(!e.supports(&MediaType::Code("cpp".into())));
        assert!(!e.supports(&MediaType::Code("rust".into())));
        assert!(!e.supports(&MediaType::Markdown));
    }

    #[test]
    fn c_extractor_simple_function() {
        let src = "int add(int a, int b) { return a + b; }\n";
        let doc = tests_support::extract_c(src, "x/math.c");
        let s = syms(&doc);
        assert!(s.iter().any(|x| x == "add"), "got {s:?}");
    }

    #[test]
    fn c_extractor_pointer_return_function() {
        let src = "int *find(int *arr, int n) { return arr; }\n";
        let doc = tests_support::extract_c(src, "x/find.c");
        let s = syms(&doc);
        assert!(s.iter().any(|x| x == "find"), "ptr-return fn missing: {s:?}");
    }

    #[test]
    fn c_extractor_static_function() {
        let src = "static void helper(void) {}\n";
        let doc = tests_support::extract_c(src, "x/helper.c");
        let s = syms(&doc);
        assert!(s.iter().any(|x| x == "helper"), "static fn missing: {s:?}");
    }

    #[test]
    fn c_extractor_extern_function() {
        let src = "extern int compute(int x);\n";
        // extern prototype is a declaration → glue
        let doc = tests_support::extract_c(src, "x/compute.c");
        let s = syms(&doc);
        // declaration (prototype) falls into glue → "<module>"
        assert!(
            s.iter().any(|x| x == "<module>"),
            "expected <module> for extern proto: {s:?}"
        );
    }

    #[test]
    fn c_extractor_inline_function() {
        let src = "inline int square(int x) { return x * x; }\n";
        let doc = tests_support::extract_c(src, "x/square.c");
        let s = syms(&doc);
        assert!(s.iter().any(|x| x == "square"), "inline fn missing: {s:?}");
    }

    #[test]
    fn c_extractor_named_struct() {
        let src = "struct Point { int x; int y; };\n";
        let doc = tests_support::extract_c(src, "x/point.c");
        let s = syms(&doc);
        assert!(s.iter().any(|x| x == "Point"), "struct missing: {s:?}");
    }

    #[test]
    fn c_extractor_named_enum() {
        let src = "enum Color { RED, GREEN, BLUE };\n";
        let doc = tests_support::extract_c(src, "x/color.c");
        let s = syms(&doc);
        assert!(s.iter().any(|x| x == "Color"), "enum missing: {s:?}");
    }

    #[test]
    fn c_extractor_named_union() {
        let src = "union Data { int i; float f; };\n";
        let doc = tests_support::extract_c(src, "x/data.c");
        let s = syms(&doc);
        assert!(s.iter().any(|x| x == "Data"), "union missing: {s:?}");
    }

    #[test]
    fn c_extractor_anonymous_struct_falls_into_glue() {
        // Anonymous struct (no name field) → glue → "<module>" (only glue, no real unit)
        let src = "struct { int x; int y; } origin;\n";
        let doc = tests_support::extract_c(src, "x/anon.c");
        let s = syms(&doc);
        // anonymous struct is a declaration containing anonymous struct_specifier → glue
        assert!(
            s.iter().any(|x| x == "<module>"),
            "expected <module> for anon struct: {s:?}"
        );
        // Must NOT emit a unit named after anything else
        assert!(
            !s.iter().any(|x| x == "origin"),
            "unexpected 'origin' unit: {s:?}"
        );
    }

    #[test]
    fn c_extractor_typedef_struct_emits_unit() {
        // v0.17.0 PR-B: `typedef struct { ... } Foo;` was previously a
        // hotfix-tracked deviation (HOTFIXES.md 2026-05-21) — the inner
        // struct_specifier is anonymous so the named-struct arm didn't
        // fire, dropping the whole construct into glue and hiding the
        // `Foo` alias from symbol search. The v2 extractor recovers the
        // typedef alias from the `declarator` field on the
        // `type_definition` node and emits a synthetic unit with that
        // name. parser_version bumped `code-c-v1` → `code-c-v2`.
        let src = "typedef struct { int x; int y; } Point;\n";
        let doc = tests_support::extract_c(src, "x/typedef.c");
        let s = syms(&doc);
        // The typedef alias surfaces as a Code symbol.
        assert!(
            s.iter().any(|x| x == "Point"),
            "expected 'Point' unit from typedef alias: {s:?}"
        );
        // No `<module>` (the file has exactly one semantic unit now,
        // the typedef alias — no glue-only fallback needed).
        assert!(
            !s.iter().any(|x| x == "<module>"),
            "no <module> fallback expected when typedef emits a unit: {s:?}"
        );
    }

    #[test]
    fn c_extractor_typedef_enum_emits_unit() {
        // Parallel coverage for enum_specifier — same typedef-alias
        // synthesis path. `typedef enum { A, B } Color;` → unit `Color`.
        let src = "typedef enum { A, B } Color;\n";
        let doc = tests_support::extract_c(src, "x/typedef_enum.c");
        let s = syms(&doc);
        assert!(
            s.iter().any(|x| x == "Color"),
            "expected 'Color' unit from typedef enum alias: {s:?}"
        );
    }

    #[test]
    fn c_extractor_typedef_union_emits_unit() {
        // Parallel coverage for union_specifier.
        let src = "typedef union { int i; float f; } IntOrFloat;\n";
        let doc = tests_support::extract_c(src, "x/typedef_union.c");
        let s = syms(&doc);
        assert!(
            s.iter().any(|x| x == "IntOrFloat"),
            "expected 'IntOrFloat' unit from typedef union alias: {s:?}"
        );
    }

    #[test]
    fn c_extractor_typedef_to_existing_type_stays_glue() {
        // Negative case: `typedef int MyInt;` has no inner struct/enum/
        // union — there's no struct body to attach the alias to, so the
        // construct falls into glue (becomes `<module>` when alone).
        // Confirms the new arm only fires for anonymous-struct typedef.
        let src = "typedef int MyInt;\n";
        let doc = tests_support::extract_c(src, "x/typedef_alias.c");
        let s = syms(&doc);
        assert!(
            s.iter().any(|x| x == "<module>"),
            "expected <module> for plain typedef alias: {s:?}"
        );
        assert!(
            !s.iter().any(|x| x == "MyInt"),
            "plain typedef alias must not emit a unit: {s:?}"
        );
    }

    #[test]
    fn c_extractor_preprocessor_directives_are_glue() {
        let src = "#include <stdio.h>\n#define MAX 100\n#ifdef DEBUG\n#endif\n";
        let doc = tests_support::extract_c(src, "x/macros.c");
        let s = syms(&doc);
        // Only preprocessor → no real unit → "<module>"
        assert!(
            s.iter().any(|x| x == "<module>"),
            "expected <module> for preproc-only file: {s:?}"
        );
        assert_eq!(s.len(), 1, "expected exactly 1 block: {s:?}");
    }

    #[test]
    fn c_extractor_multiple_functions_correct_count() {
        let src = "int foo(void) { return 1; }\nint bar(void) { return 2; }\nint baz(void) { return 3; }\n";
        let doc = tests_support::extract_c(src, "x/multi.c");
        let s = syms(&doc);
        assert!(s.iter().any(|x| x == "foo"), "foo missing: {s:?}");
        assert!(s.iter().any(|x| x == "bar"), "bar missing: {s:?}");
        assert!(s.iter().any(|x| x == "baz"), "baz missing: {s:?}");
        assert_eq!(s.len(), 3, "expected 3 units: {s:?}");
    }

    #[test]
    fn c_extractor_empty_file_produces_module() {
        let src = "";
        let doc = tests_support::extract_c(src, "x/empty.c");
        let s = syms(&doc);
        assert_eq!(s, vec!["<module>"], "expected <module>: got {s:?}");
    }

    #[test]
    fn c_extractor_preprocessor_only_produces_module() {
        let src = "#include <stdlib.h>\n#define VERSION \"1.0\"\n";
        let doc = tests_support::extract_c(src, "x/header.c");
        let s = syms(&doc);
        assert!(
            s.iter().any(|x| x == "<module>"),
            "expected <module> for preproc-only file: {s:?}"
        );
    }

    #[test]
    fn c_extractor_mixed_functions_and_glue() {
        let src = r#"#include <stdio.h>

int compute(int x) {
    return x * 2;
}

extern int lookup(int key);

void print_result(int v) {
    printf("%d\n", v);
}
"#;
        let doc = tests_support::extract_c(src, "x/mixed.c");
        let s = syms(&doc);
        // Two real functions + one glue block
        assert!(s.iter().any(|x| x == "compute"), "compute missing: {s:?}");
        assert!(s.iter().any(|x| x == "print_result"), "print_result missing: {s:?}");
        assert!(
            s.iter().any(|x| x == "<top-level>"),
            "<top-level> glue missing: {s:?}"
        );
    }

    #[test]
    fn c_extractor_deterministic_across_runs() {
        let src = r"
struct Node { int val; };
int sum(int a, int b) { return a + b; }
void noop(void) {}
";
        let a = tests_support::extract_c(src, "x/det.c");
        for _ in 0..20 {
            assert_eq!(
                tests_support::extract_c(src, "x/det.c").blocks,
                a.blocks
            );
        }
    }
}
