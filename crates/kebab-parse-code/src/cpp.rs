//! `kebab-parse-code::cpp` — tree-sitter C++ AST extractor (P10-1D Task C).
//!
//! Implements [`kebab_core::Extractor`] for [`MediaType::Code("cpp")`].
//! Walks the tree-sitter parse tree and emits one [`Block::Code`] per
//! top-level AST semantic unit, each carrying [`SourceSpan::Code`] with
//! the unit's `::` separated symbol path (design §3.4 C++ row).
//!
//! ## Symbol formation
//!
//! Symbol = `namespace::Class::method` via recursive `build_blocks`:
//!
//! - `namespace_definition` (named) → push namespace name, recurse into body.
//! - Anonymous namespace (`namespace { ... }`) → push `<anonymous>`, recurse.
//! - `nested_namespace_specifier` (`outer::inner`) → push all segments, recurse.
//! - `class_specifier` / `struct_specifier` (named) → emit class unit + recurse
//!   into body with class name pushed.
//! - `function_definition` → emit method/function unit. Symbol is built from
//!   the prefix chain + the extracted declarator name component.
//! - Out-of-class method def (`void Foo::bar() {}`) — the declarator's inner
//!   node is a `qualified_identifier`; its scope chain is prepended to the
//!   current prefix to form the full symbol.
//! - `template_declaration` → recurse into named children with same prefix;
//!   the inner function/class body is matched by its own arm. Template params
//!   are NOT included in the symbol.
//! - `enum_specifier` (named) → emit type unit.
//! - `concept_definition` (C++20) → emit type unit.
//! - `linkage_specification` (extern "C") → recurse into body with same prefix.
//!
//! ## Constructor / destructor / operator overload
//!
//! - Constructor: `function_declarator > identifier` matching the class name.
//!   Symbol = `Class::Class` (name duplicated, same convention as Java).
//! - Destructor: `function_declarator > destructor_name`. Symbol = `Class::~Foo`.
//! - Operator overload: `function_declarator > operator_name`. Symbol = `Class::operator+`.
//! - Conversion operator: `function_definition.declarator` is `operator_cast`.
//!   Symbol = `Class::operator <type>` (e.g. `Class::operator bool`).
//!
//! ## Glue
//!
//! Everything not in the unit list collapses into a single `<top-level>` glue
//! chunk (preproc, declarations, using, typedef, etc.). If the file produces
//! zero units AND zero glue, the `<module>` post-pass emits one unit covering
//! the whole file.
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

use crate::scaffold::{filename_from_workspace_path, strip_extension};

pub const PARSER_VERSION: &str = "code-cpp-v1";

/// C++ AST extractor. Per-unit blocks via tree-sitter-cpp 0.23.4
/// (`LANGUAGE: LanguageFn`) parsed by tree-sitter 0.26.
pub struct CppAstExtractor;

impl CppAstExtractor {
    pub fn new() -> Self {
        Self
    }
}

impl Default for CppAstExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl Extractor for CppAstExtractor {
    fn supports(&self, m: &MediaType) -> bool {
        matches!(m, MediaType::Code(l) if l == "cpp")
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
                "kebab-parse-code: unsupported media_type for CppAstExtractor: {:?}",
                asset.media_type
            );
        }

        let parser_version = self.parser_version();
        let doc_id = id_for_doc(&asset.workspace_path, &asset.asset_id, &parser_version);

        let source = String::from_utf8(bytes.to_vec())
            .map_err(|e| anyhow::anyhow!("kebab-parse-code: C++ source is not valid UTF-8: {e}"))?;

        let blocks = build_blocks_top(&source, &doc_id)?;
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
            code_lang: Some("cpp".to_string()),
        };

        tracing::debug!(
            target: "kebab-parse-code",
            "extracted C++ doc_id={} workspace_path={} units={}",
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

// ---------------------------------------------------------------------------
// Core block-building logic
// ---------------------------------------------------------------------------

/// Top-level entry: parse source, walk the `translation_unit` root, assemble
/// units + glue, apply the `<module>` post-pass, and emit `Block::Code`s.
fn build_blocks_top(
    source: &str,
    doc_id: &kebab_core::DocumentId,
) -> anyhow::Result<Vec<kebab_core::Block>> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_cpp::LANGUAGE.into())
        .map_err(|e| anyhow::anyhow!("set tree-sitter-cpp language: {e}"))?;
    let tree = parser
        .parse(source.as_bytes(), None)
        .ok_or_else(|| anyhow::anyhow!("tree-sitter failed to parse C++ source"))?;
    let lines: Vec<&str> = source.split('\n').collect();
    let root = tree.root_node();

    // units: (symbol, line_start, line_end, is_real_semantic_unit).
    // Glue is accumulated as (start, end) pairs and flushed into one
    // "<top-level>" block (or "<module>" if no real unit exists).
    let mut units: Vec<(String, u32, u32, bool)> = Vec::new();
    let mut glue: Vec<(u32, u32)> = Vec::new();

    build_blocks(root, source, &[], &mut units, &mut glue);
    flush_glue(&mut glue, &mut units);

    // Post-pass: if the file has no real semantic unit (only glue, or
    // completely empty), rename the single glue unit to "<module>".
    // If there are zero units AND zero glue, synthesize a one-line
    // "<module>" covering the whole file.
    let has_real_unit = units.iter().any(|(_, _, _, is_real)| *is_real);

    if units.is_empty() {
        let total = lines.len() as u32;
        units.push(("<module>".to_string(), 1, total.max(1), false));
    }
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
            lang: Some("cpp".to_string()),
        };
        let block_id = id_for_block(doc_id, "code", &[], ordinal as u32, &span);
        let code = lines[(line_start as usize - 1)..(line_end as usize)].join("\n");
        blocks.push(Block::Code(CodeBlock {
            common: CommonBlock {
                block_id,
                heading_path: Vec::new(),
                source_span: span,
            },
            lang: Some("cpp".to_string()),
            code,
        }));
    }
    Ok(blocks)
}

/// Walk preceding `comment` siblings to extend the unit's line range upward,
/// folding leading doc / line comments into the unit (1B pattern).
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

fn flush_glue(glue: &mut Vec<(u32, u32)>, units: &mut Vec<(String, u32, u32, bool)>) {
    if glue.is_empty() {
        return;
    }
    let s = glue.iter().map(|(a, _)| *a).min().unwrap();
    let e = glue.iter().map(|(_, b)| *b).max().unwrap();
    units.push(("<top-level>".to_string(), s, e, false));
    glue.clear();
}

/// Walk a scope node (translation_unit, declaration_list, field_declaration_list)
/// emitting unit + glue blocks. `prefix` is the current namespace/class chain
/// (e.g. `["kebab", "Chunk", "Foo"]`).
///
/// After returning, any pending glue in `glue` is NOT flushed — callers
/// responsible for flushing at the scope boundary (top-level flush in
/// `build_blocks_top`). Within recursive scope bodies (namespace/class) we
/// do flush before returning so that glue doesn't leak across scopes.
fn build_blocks(
    node: tree_sitter::Node,
    source: &str,
    prefix: &[String],
    units: &mut Vec<(String, u32, u32, bool)>,
    glue: &mut Vec<(u32, u32)>,
) {
    let mut cur = node.walk();
    for child in node.named_children(&mut cur) {
        let s = unit_start(&child);
        let e = child.end_position().row as u32 + 1;

        match child.kind() {
            "namespace_definition" => {
                // Flush pending glue before starting this namespace block.
                flush_glue(glue, units);

                let name_node = child.child_by_field_name("name");
                let body = child.child_by_field_name("body").unwrap_or(child);

                match name_node {
                    None => {
                        // Anonymous namespace: push "<anonymous>", recurse.
                        let mut new_prefix = prefix.to_vec();
                        new_prefix.push("<anonymous>".to_string());
                        build_blocks(body, source, &new_prefix, units, glue);
                        flush_glue(glue, units);
                    }
                    Some(nn) => match nn.kind() {
                        "namespace_identifier" => {
                            let name = &source[nn.start_byte()..nn.end_byte()];
                            let mut new_prefix = prefix.to_vec();
                            new_prefix.push(name.to_string());
                            build_blocks(body, source, &new_prefix, units, glue);
                            flush_glue(glue, units);
                        }
                        "nested_namespace_specifier" => {
                            // e.g. `namespace outer::inner { ... }`
                            // All named children are namespace_identifier nodes.
                            let mut new_prefix = prefix.to_vec();
                            let mut nc = nn.walk();
                            for seg in nn.named_children(&mut nc) {
                                new_prefix
                                    .push(source[seg.start_byte()..seg.end_byte()].to_string());
                            }
                            build_blocks(body, source, &new_prefix, units, glue);
                            flush_glue(glue, units);
                        }
                        _ => {
                            // Unknown name kind — treat entire namespace as glue.
                            glue.push((s, e));
                        }
                    },
                }
            }

            "class_specifier" | "struct_specifier" => {
                let name_node = child.child_by_field_name("name");
                let Some(nn) = name_node else {
                    // Anonymous class/struct — glue.
                    glue.push((s, e));
                    continue;
                };
                let name = match nn.kind() {
                    "type_identifier" => &source[nn.start_byte()..nn.end_byte()],
                    _ => {
                        // template_type or qualified_identifier — use full text
                        // as the symbol segment (includes template args).
                        &source[nn.start_byte()..nn.end_byte()]
                    }
                };

                flush_glue(glue, units);
                let sym = build_symbol(prefix, &[name]);
                units.push((sym, s, e, true));

                if let Some(body) = child.child_by_field_name("body") {
                    let mut new_prefix = prefix.to_vec();
                    new_prefix.push(name.to_string());
                    build_blocks(body, source, &new_prefix, units, glue);
                    flush_glue(glue, units);
                }
            }

            "function_definition" => {
                let decl = child.child_by_field_name("declarator");
                let Some(decl_node) = decl else {
                    glue.push((s, e));
                    continue;
                };

                match extract_fn_symbol(decl_node, source, prefix) {
                    Some(sym) => {
                        flush_glue(glue, units);
                        units.push((sym, s, e, true));
                    }
                    None => {
                        glue.push((s, e));
                    }
                }
            }

            "template_declaration" => {
                // Unwrap: recurse into named children with same prefix.
                // The inner function/class/concept will be matched by their own
                // arms. template_parameter_list is not a unit; it will fall
                // through to glue (it's not a named child of the template_declaration
                // that matches any of our arms).
                build_blocks(child, source, prefix, units, glue);
                // Do NOT flush glue here — template body may be part of a glue group.
            }

            "enum_specifier" => {
                if let Some(nn) = child.child_by_field_name("name") {
                    let name = &source[nn.start_byte()..nn.end_byte()];
                    flush_glue(glue, units);
                    let sym = build_symbol(prefix, &[name]);
                    units.push((sym, s, e, true));
                } else {
                    // Anonymous enum — glue.
                    glue.push((s, e));
                }
            }

            "concept_definition" => {
                // C++20. Has required "name" field (identifier).
                if let Some(nn) = child.child_by_field_name("name") {
                    let name = &source[nn.start_byte()..nn.end_byte()];
                    flush_glue(glue, units);
                    let sym = build_symbol(prefix, &[name]);
                    units.push((sym, s, e, true));
                } else {
                    glue.push((s, e));
                }
            }

            "linkage_specification" => {
                // extern "C" { ... } — glue-wrapper, but recurse into body
                // with same prefix so inner definitions are extracted.
                let body = child.child_by_field_name("body").unwrap_or(child);
                // The linkage_spec itself is glue; inner defs handled by recursion.
                // Don't emit the wrapper as a unit; but also don't push it as glue
                // since recursion will push its inner children individually.
                build_blocks(body, source, prefix, units, glue);
            }

            // Everything else: preproc, declarations, using, typedef, etc.
            _ => {
                glue.push((s, e));
            }
        }
    }
}

/// Join prefix + extras into a `::` separated symbol.
fn build_symbol(prefix: &[String], extras: &[&str]) -> String {
    let mut parts: Vec<&str> = prefix.iter().map(String::as_str).collect();
    parts.extend_from_slice(extras);
    parts.join("::")
}

/// Extract the symbol for a `function_definition` given its top-level
/// `declarator` node. Returns `None` if the name cannot be determined.
///
/// The declarator chain may be:
/// - `function_declarator` (plain fn or method)
/// - `pointer_declarator` wrapping `function_declarator` (fn returning pointer)
/// - `reference_declarator` wrapping `function_declarator` (fn returning ref)
/// - `operator_cast` (conversion operator — e.g. `operator bool`)
///
/// The inner `function_declarator.declarator` is one of:
/// - `identifier` → free fn or constructor, symbol = `prefix::name`
/// - `field_identifier` → method in class body, symbol = `prefix::name`
/// - `destructor_name` → `~Foo`, symbol = `prefix::~Foo`
/// - `operator_name` → `operator+` etc., symbol = `prefix::operator+`
/// - `qualified_identifier` → out-of-class def `Foo::bar` or `ns::Foo::bar`;
///   the scope chain is extracted and prepended to prefix.
///
/// For `qualified_identifier`, the scope hierarchy (which may itself be a
/// `qualified_identifier`) is flattened into a list of segments. These
/// segments REPLACE the current prefix (since out-of-class defs carry their
/// full scope explicitly). Example: `void ns::Foo::bar() {}` at top level
/// with prefix=[] → segments=[ns, Foo, bar] → symbol = `ns::Foo::bar`.
fn extract_fn_symbol(
    decl_node: tree_sitter::Node,
    source: &str,
    prefix: &[String],
) -> Option<String> {
    // Walk down pointer/reference wrapper layers to reach the
    // function_declarator (or operator_cast at definition level).
    let fn_decl = unwrap_to_fn_declarator(decl_node, source)?;

    match fn_decl.kind() {
        "operator_cast" => {
            // e.g. `operator bool() const` — the function_definition.declarator
            // IS the operator_cast (no function_declarator wrapper).
            // Symbol = `prefix::operator <type>`.
            let type_node = fn_decl.child_by_field_name("type")?;
            let type_text = &source[type_node.start_byte()..type_node.end_byte()];
            Some(build_symbol(prefix, &[&format!("operator {type_text}")]))
        }
        "function_declarator" => {
            let inner = fn_decl.child_by_field_name("declarator")?;
            extract_name_node(inner, source, prefix)
        }
        _ => None,
    }
}

/// Walk pointer_declarator / reference_declarator chains down to the
/// first `function_declarator` or `operator_cast` node.
///
/// Returns `None` if no such node is found (e.g. a function definition
/// whose declarator is malformed or unknown).
fn unwrap_to_fn_declarator<'a>(
    mut node: tree_sitter::Node<'a>,
    _source: &str,
) -> Option<tree_sitter::Node<'a>> {
    loop {
        match node.kind() {
            "function_declarator" | "operator_cast" => return Some(node),
            "pointer_declarator" => {
                node = node.child_by_field_name("declarator")?;
            }
            "reference_declarator" | "rvalue_reference_declarator" => {
                // reference_declarator has no `declarator` field; its child
                // is in the unnamed children list.
                let mut walker = node.walk();
                node = node.named_children(&mut walker).next()?;
            }
            _ => return None,
        }
    }
}

/// Given the innermost name node of a function_declarator, produce the symbol.
fn extract_name_node(inner: tree_sitter::Node, source: &str, prefix: &[String]) -> Option<String> {
    match inner.kind() {
        "identifier" | "field_identifier" => {
            let name = &source[inner.start_byte()..inner.end_byte()];
            Some(build_symbol(prefix, &[name]))
        }
        "destructor_name" => {
            // destructor_name text includes the `~` prefix (e.g. "~Foo").
            let full = &source[inner.start_byte()..inner.end_byte()];
            Some(build_symbol(prefix, &[full]))
        }
        "operator_name" => {
            // Full text e.g. "operator+", "operator->", "operator()".
            let full = &source[inner.start_byte()..inner.end_byte()];
            Some(build_symbol(prefix, &[full]))
        }
        "template_function" | "template_method" => {
            // Template function like `foo<int>()`. Use the `name` field
            // (the identifier / field_identifier before `<`).
            let name_node = inner.child_by_field_name("name")?;
            let name = &source[name_node.start_byte()..name_node.end_byte()];
            Some(build_symbol(prefix, &[name]))
        }
        "qualified_identifier" => {
            // Out-of-class method definition. Flatten the nested
            // qualified_identifier chain into ordered segments.
            // Example: `ns::Foo::method`
            //   qualified_identifier {
            //     scope: namespace_identifier "ns"
            //     name: qualified_identifier {
            //       scope: namespace_identifier "Foo"
            //       name: identifier "method"
            //     }
            //   }
            // → ["ns", "Foo", "method"]
            //
            // These segments are combined with the current prefix so that a
            // top-level out-of-class def `void Foo::bar() {}` inside a
            // namespace body with prefix=["ns"] produces `ns::Foo::bar`.
            let mut segments: Vec<String> = Vec::new();
            flatten_qualified_id(inner, source, &mut segments);
            if segments.is_empty() {
                return None;
            }
            // Build: prefix + all segments (scope chain + leaf).
            let mut all: Vec<&str> = prefix.iter().map(String::as_str).collect();
            for seg in &segments {
                all.push(seg.as_str());
            }
            Some(all.join("::"))
        }
        _ => None,
    }
}

/// Recursively flatten a `qualified_identifier` node into ordered string
/// segments. For `ns::Foo::method` this produces `["ns", "Foo", "method"]`.
fn flatten_qualified_id(node: tree_sitter::Node, source: &str, out: &mut Vec<String>) {
    // A qualified_identifier has:
    //   scope: namespace_identifier | (None for global-scope `::foo`)
    //   name:  identifier | field_identifier | destructor_name |
    //          operator_name | qualified_identifier | template_function |
    //          template_method | ...
    let scope_node = node.child_by_field_name("scope");
    let name_node = node.child_by_field_name("name");

    if let Some(s) = scope_node {
        out.push(source[s.start_byte()..s.end_byte()].to_string());
    }

    match name_node {
        Some(n) if n.kind() == "qualified_identifier" => {
            // Recurse: more nesting.
            flatten_qualified_id(n, source, out);
        }
        Some(n) => {
            // Leaf name — push its text.
            out.push(source[n.start_byte()..n.end_byte()].to_string());
        }
        None => {}
    }
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

    pub fn extract_cpp(src: &str, path: &str) -> kebab_core::CanonicalDocument {
        use super::CppAstExtractor;
        use kebab_core::Extractor;
        let asset = fixed_code_asset(path, "cpp");
        let cfg = ExtractConfig::default();
        let root = PathBuf::from("/tmp");
        let ctx = ExtractContext {
            asset: &asset,
            workspace_root: &root,
            config: &cfg,
        };
        CppAstExtractor::new()
            .extract(&ctx, src.as_bytes())
            .unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kebab_core::{Block, MediaType, SourceSpan};

    fn syms(doc: &kebab_core::CanonicalDocument) -> Vec<String> {
        let mut s: Vec<String> = doc
            .blocks
            .iter()
            .filter_map(|b| match b {
                Block::Code(c) => match &c.common.source_span {
                    SourceSpan::Code { symbol, .. } => symbol.clone(),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        s.sort();
        s
    }

    #[test]
    fn extractor_supports_only_media_code_cpp() {
        let e = CppAstExtractor::new();
        assert!(e.supports(&MediaType::Code("cpp".into())));
        assert!(!e.supports(&MediaType::Code("c".into())));
        assert!(!e.supports(&MediaType::Code("rust".into())));
        assert!(!e.supports(&MediaType::Markdown));
    }

    #[test]
    fn free_function() {
        let src = "void foo() {}\n";
        let doc = tests_support::extract_cpp(src, "x/foo.cpp");
        let s = syms(&doc);
        assert!(s.iter().any(|x| x == "foo"), "got {s:?}");
    }

    #[test]
    fn namespace_and_class() {
        let src = r"
namespace ns {
    class Foo {
    public:
        void method() {}
        Foo() {}
        ~Foo() {}
        int operator+(const Foo& o) { return 0; }
    };
}
";
        let doc = tests_support::extract_cpp(src, "x/foo.cpp");
        let s = syms(&doc);
        assert!(s.iter().any(|x| x == "ns::Foo"), "ns::Foo missing: {s:?}");
        assert!(
            s.iter().any(|x| x == "ns::Foo::method"),
            "method missing: {s:?}"
        );
        assert!(s.iter().any(|x| x == "ns::Foo::Foo"), "ctor missing: {s:?}");
        assert!(
            s.iter().any(|x| x == "ns::Foo::~Foo"),
            "dtor missing: {s:?}"
        );
        assert!(
            s.iter().any(|x| x == "ns::Foo::operator+"),
            "op+ missing: {s:?}"
        );
    }

    #[test]
    fn anonymous_namespace() {
        let src = r"
namespace {
    void hidden_fn() {}
}
";
        let doc = tests_support::extract_cpp(src, "x/foo.cpp");
        let s = syms(&doc);
        assert!(
            s.iter().any(|x| x == "<anonymous>::hidden_fn"),
            "anon fn missing: {s:?}"
        );
    }

    #[test]
    fn nested_namespace_specifier() {
        let src = r"
namespace outer::inner {
    void fn_in_nested() {}
}
";
        let doc = tests_support::extract_cpp(src, "x/foo.cpp");
        let s = syms(&doc);
        assert!(
            s.iter().any(|x| x == "outer::inner::fn_in_nested"),
            "nested ns fn missing: {s:?}"
        );
    }

    #[test]
    fn out_of_class_method_def() {
        let src = r"
void ns::Foo::method() { }
";
        let doc = tests_support::extract_cpp(src, "x/foo.cpp");
        let s = syms(&doc);
        assert!(
            s.iter().any(|x| x == "ns::Foo::method"),
            "out-of-class method missing: {s:?}"
        );
    }

    #[test]
    fn template_declaration() {
        let src = r"
template<typename T>
class Bar {
    void tmpl_method() {}
};

template<typename T>
void tmpl_free_fn(T x) {}
";
        let doc = tests_support::extract_cpp(src, "x/foo.cpp");
        let s = syms(&doc);
        assert!(s.iter().any(|x| x == "Bar"), "Bar class missing: {s:?}");
        assert!(
            s.iter().any(|x| x == "Bar::tmpl_method"),
            "Bar::tmpl_method missing: {s:?}"
        );
        assert!(
            s.iter().any(|x| x == "tmpl_free_fn"),
            "tmpl_free_fn missing: {s:?}"
        );
    }

    #[test]
    fn enum_and_concept() {
        let src = r"
enum class Color { Red, Green };

template<typename T>
concept Printable = requires(T t) { t.print(); };
";
        let doc = tests_support::extract_cpp(src, "x/foo.cpp");
        let s = syms(&doc);
        assert!(s.iter().any(|x| x == "Color"), "Color missing: {s:?}");
        assert!(
            s.iter().any(|x| x == "Printable"),
            "Printable missing: {s:?}"
        );
    }

    #[test]
    fn extern_c_block() {
        let src = r#"
extern "C" {
    void c_fn1() {}
    void c_fn2() {}
}
"#;
        let doc = tests_support::extract_cpp(src, "x/foo.cpp");
        let s = syms(&doc);
        assert!(s.iter().any(|x| x == "c_fn1"), "c_fn1 missing: {s:?}");
        assert!(s.iter().any(|x| x == "c_fn2"), "c_fn2 missing: {s:?}");
    }

    #[test]
    fn conversion_operator() {
        let src = r"
class Foo {
    operator bool() const { return true; }
};
";
        let doc = tests_support::extract_cpp(src, "x/foo.cpp");
        let s = syms(&doc);
        assert!(
            s.iter().any(|x| x == "Foo::operator bool"),
            "conversion op missing: {s:?}"
        );
    }

    #[test]
    fn empty_file_produces_module() {
        let src = "";
        let doc = tests_support::extract_cpp(src, "x/empty.cpp");
        let s = syms(&doc);
        assert_eq!(s, vec!["<module>"], "expected <module>: got {s:?}");
    }

    #[test]
    fn glue_only_produces_module() {
        let src = "#include <vector>\nusing namespace std;\n";
        let doc = tests_support::extract_cpp(src, "x/glue.cpp");
        let s = syms(&doc);
        assert!(
            s.iter().any(|x| x == "<module>"),
            "expected <module>: got {s:?}"
        );
    }

    #[test]
    fn ptr_returning_function() {
        let src = "int* ptr_fn(int x) { return &x; }\n";
        let doc = tests_support::extract_cpp(src, "x/foo.cpp");
        let s = syms(&doc);
        assert!(s.iter().any(|x| x == "ptr_fn"), "ptr_fn missing: {s:?}");
    }

    #[test]
    fn ref_returning_operator() {
        let src = r"
class Foo {
    Foo& operator=(const Foo& o) { return *this; }
};
";
        let doc = tests_support::extract_cpp(src, "x/foo.cpp");
        let s = syms(&doc);
        assert!(
            s.iter().any(|x| x == "Foo::operator="),
            "operator= missing: {s:?}"
        );
    }

    #[test]
    fn deterministic_across_runs() {
        let src = r"
namespace ns {
    class Foo {
        void method() {}
    };
}
void free_fn() {}
";
        let a = tests_support::extract_cpp(src, "x/foo.cpp");
        for _ in 0..20 {
            assert_eq!(
                tests_support::extract_cpp(src, "x/foo.cpp").blocks,
                a.blocks
            );
        }
    }
}
