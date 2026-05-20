//! `kebab-parse-code::typescript` — tree-sitter TypeScript / TSX AST
//! extractor (P10-1B Task H).
//!
//! Implements [`kebab_core::Extractor`] for [`MediaType::Code("typescript")`].
//! Walks the tree-sitter parse tree (one of two grammars selected by the
//! workspace path's extension — `.tsx` uses [`tree_sitter_typescript::LANGUAGE_TSX`],
//! everything else uses [`tree_sitter_typescript::LANGUAGE_TYPESCRIPT`]) and
//! emits one [`Block::Code`] per top-level AST semantic unit (free fn,
//! class, each method, interface, type alias, enum, recursively per
//! nested class), each carrying [`SourceSpan::Code`] with the unit's
//! dotted symbol path prefixed by [`module_path_for_tsjs`].
//!
//! Glue declarations (`import_statement`, bare `export_statement`
//! re-exports, `lexical_declaration` / `variable_declaration` at the
//! module level, namespace / module declarations, etc.) collapse into
//! one grouped `<top-level>` (or `<module>`) unit.
//!
//! `export_statement` is unwrapped: an `export function|class|interface
//! |type|enum` is treated as the inner declaration arm but the unit's
//! line range comes from the OUTER `export_statement` so the `export `
//! prefix is folded in. `export default function () {}` / `export
//! default class {}` (no `name` field) emits `default` as the symbol
//! name.
//!
//! Scope follows 1A-2 / 1B Task E: AST unit extraction + dotted symbol
//! paths + line ranges. Per design §3.4 / §9.1 / §9 versioning.

use anyhow::Result;
use kebab_core::{
    Block, CanonicalDocument, CodeBlock, CommonBlock, Extractor, Lang, MediaType, Metadata,
    ParserVersion, Provenance, ProvenanceEvent, ProvenanceKind, SourceSpan, SourceType, TrustLevel,
    id_for_block, id_for_doc,
};
use serde_json::Map;
use time::OffsetDateTime;

use crate::scaffold::{filename_from_workspace_path, join_symbol, strip_extension};

pub const PARSER_VERSION: &str = "code-ts-v1";

/// TypeScript / TSX AST extractor. Per-unit blocks via
/// tree-sitter-typescript 0.23 (`LANGUAGE_TYPESCRIPT` / `LANGUAGE_TSX`
/// — two `LanguageFn`s, selected by extension) parsed by tree-sitter
/// 0.26.
pub struct TypescriptAstExtractor;

impl TypescriptAstExtractor {
    pub fn new() -> Self {
        Self
    }
}

impl Default for TypescriptAstExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl Extractor for TypescriptAstExtractor {
    fn supports(&self, m: &MediaType) -> bool {
        matches!(m, MediaType::Code(l) if l == "typescript")
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
                "kebab-parse-code: unsupported media_type for TypescriptAstExtractor: {:?}",
                asset.media_type
            );
        }

        let parser_version = self.parser_version();
        let doc_id = id_for_doc(&asset.workspace_path, &asset.asset_id, &parser_version);

        let source = String::from_utf8(bytes.to_vec()).map_err(|e| {
            anyhow::anyhow!("kebab-parse-code: TypeScript source is not valid UTF-8: {e}")
        })?;

        let mod_prefix = crate::lang::module_path_for_tsjs(&asset.workspace_path.0);
        let language = select_grammar(&asset.workspace_path.0);
        let blocks = build_blocks(&source, &doc_id, &mod_prefix, language)?;
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
            code_lang: Some("typescript".to_string()),
        };

        tracing::debug!(
            target: "kebab-parse-code",
            "extracted TypeScript doc_id={} workspace_path={} units={}",
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

/// Select the tree-sitter grammar based on the workspace path's
/// extension. `.tsx` → TSX grammar; everything else (`.ts`, `.d.ts`,
/// missing extension) → TypeScript grammar.
fn select_grammar(workspace_path: &str) -> tree_sitter::Language {
    if workspace_path.ends_with(".tsx") {
        tree_sitter_typescript::LANGUAGE_TSX.into()
    } else {
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
    }
}

fn build_blocks(
    source: &str,
    doc_id: &kebab_core::DocumentId,
    mod_prefix: &str,
    language: tree_sitter::Language,
) -> anyhow::Result<Vec<kebab_core::Block>> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&language)
        .map_err(|e| anyhow::anyhow!("set tree-sitter-typescript language: {e}"))?;
    let tree = parser
        .parse(source.as_bytes(), None)
        .ok_or_else(|| anyhow::anyhow!("tree-sitter failed to parse TypeScript source"))?;
    let lines: Vec<&str> = source.split('\n').collect();

    // units: (symbol, line_start, line_end, is_real_semantic_unit).
    // Glue groups are pushed with a sentinel symbol + is_real=false so a
    // post-pass can decide `<module>` vs `<top-level>` (same algorithm
    // as 1A Gap 1 / 1B Python).
    let mut units: Vec<(String, u32, u32, bool)> = Vec::new();
    // (is_module_only_kind 0/1, s, e). `is_module_only_kind` flags
    // `import_statement` and bare re-export `export_statement`s — used by
    // the glue flush to pick `<module>` vs `<top-level>` provisional
    // label (1A's `is_mod_decl` analog).
    let mut glue: Vec<(usize, u32, u32)> = Vec::new();

    /// Walk preceding `comment` and `decorator` siblings to extend the
    /// unit's line range upward, folding leading doc/line comments and
    /// decorators into the unit.
    ///
    /// In tree-sitter-typescript 0.23, TS class-method decorators (and
    /// class-level decorators) are **`class_body` siblings** that
    /// immediately precede the `method_definition` node — they are NOT
    /// children of `method_definition`. (Contrast with
    /// tree-sitter-javascript, where the `decorator` IS stored inside
    /// `method_definition` as a named child via the `decorator` field, so
    /// `method_definition.start_row` already covers the decorator line
    /// there — no sibling walk needed in `javascript.rs`.)
    ///
    /// Extending backward over `decorator` siblings here matches Python's
    /// `decorated_definition` arm behavior: the decorator line is folded
    /// into the emitted unit's line range.
    fn unit_start(n: &tree_sitter::Node) -> u32 {
        let mut start = n.start_position().row as u32 + 1;
        let mut prev = n.prev_sibling();
        while let Some(p) = prev {
            if p.kind() == "comment" || p.kind() == "decorator" {
                start = p.start_position().row as u32 + 1;
                prev = p.prev_sibling();
            } else {
                break;
            }
        }
        start
    }
    fn name_text<'a>(n: &tree_sitter::Node, src: &'a str) -> Option<&'a str> {
        n.child_by_field_name("name")
            .map(|c| &src[c.start_byte()..c.end_byte()])
    }
    /// Walk a class body, emitting one unit per `method_definition`.
    /// Class names already pushed onto `mod_path` by the caller, so
    /// method symbols come out as `<mod_prefix>.<Class>.<method>`.
    fn walk_class_body(
        body: tree_sitter::Node,
        src: &str,
        mod_prefix: &str,
        mod_path: &[String],
        units: &mut Vec<(String, u32, u32, bool)>,
    ) {
        let mut cur = body.walk();
        for child in body.named_children(&mut cur) {
            if child.kind() == "method_definition" {
                if let Some(name) = name_text(&child, src) {
                    let s = unit_start(&child);
                    let e = child.end_position().row as u32 + 1;
                    let sym = join_symbol(mod_prefix, mod_path, name);
                    units.push((sym, s, e, true));
                }
            }
        }
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
            let s = unit_start(&child);
            let e = child.end_position().row as u32 + 1;
            match child.kind() {
                "function_declaration" => {
                    if let Some(name) = name_text(&child, src) {
                        glue.retain(|(_, gs, _)| *gs < s);
                        flush_glue(glue, units, mod_prefix, mod_path);
                        let sym = join_symbol(mod_prefix, mod_path, name);
                        units.push((sym, s, e, true));
                    }
                }
                "class_declaration" => {
                    if let Some(name) = name_text(&child, src) {
                        glue.retain(|(_, gs, _)| *gs < s);
                        flush_glue(glue, units, mod_prefix, mod_path);
                        let sym = join_symbol(mod_prefix, mod_path, name);
                        units.push((sym, s, e, true));
                        if let Some(body) = child.child_by_field_name("body") {
                            let mut np = mod_path.to_vec();
                            np.push(name.to_string());
                            walk_class_body(body, src, mod_prefix, &np, units);
                        }
                    }
                }
                "interface_declaration"
                | "type_alias_declaration"
                | "enum_declaration" => {
                    if let Some(name) = name_text(&child, src) {
                        glue.retain(|(_, gs, _)| *gs < s);
                        flush_glue(glue, units, mod_prefix, mod_path);
                        let sym = join_symbol(mod_prefix, mod_path, name);
                        units.push((sym, s, e, true));
                    }
                }
                "export_statement" => {
                    // Try field "declaration" first (export class /
                    // function / interface / type / enum). If absent,
                    // fall back to "value" — `export default function
                    // () {}` / `export default class {}` expose the
                    // anonymous function_expression / class under the
                    // `value` field (TS grammar 0.23).
                    let outer_s = s; // includes `export ` prefix line
                    let outer_e = e;
                    if let Some(inner) = child.child_by_field_name("declaration") {
                        let inner_kind = inner.kind();
                        match inner_kind {
                            "function_declaration"
                            | "class_declaration"
                            | "interface_declaration"
                            | "type_alias_declaration"
                            | "enum_declaration" => {
                                let name_opt = name_text(&inner, src).map(|s| s.to_string());
                                if let Some(name) = name_opt {
                                    glue.retain(|(_, gs, _)| *gs < outer_s);
                                    flush_glue(glue, units, mod_prefix, mod_path);
                                    let sym =
                                        join_symbol(mod_prefix, mod_path, &name);
                                    units.push((sym, outer_s, outer_e, true));
                                    if inner_kind == "class_declaration" {
                                        if let Some(body) =
                                            inner.child_by_field_name("body")
                                        {
                                            let mut np = mod_path.to_vec();
                                            np.push(name);
                                            walk_class_body(
                                                body, src, mod_prefix, &np, units,
                                            );
                                        }
                                    }
                                } else {
                                    // `export default function foo() {}`
                                    // path is covered by name_opt =
                                    // Some(_) above; the no-name path
                                    // here is `export default` with a
                                    // function_declaration that
                                    // somehow lacks `name`. Emit
                                    // `default` defensively.
                                    glue.retain(|(_, gs, _)| *gs < outer_s);
                                    flush_glue(glue, units, mod_prefix, mod_path);
                                    let sym =
                                        join_symbol(mod_prefix, mod_path, "default");
                                    units.push((sym, outer_s, outer_e, true));
                                }
                            }
                            // `lexical_declaration` etc. wrapped in
                            // export: treat as glue (assigned arrow
                            // fns / consts don't get their own unit).
                            _ => {
                                glue.push((0, s, e));
                            }
                        }
                    } else if let Some(value) = child.child_by_field_name("value") {
                        // `export default <expr>`. We emit a unit only
                        // for the function / class shapes (named or
                        // anonymous); other value shapes are glue.
                        match value.kind() {
                            "function_expression"
                            | "function_declaration"
                            | "class"
                            | "class_declaration" => {
                                let name_opt =
                                    name_text(&value, src).map(|s| s.to_string());
                                let leaf = name_opt
                                    .as_deref()
                                    .unwrap_or("default")
                                    .to_string();
                                glue.retain(|(_, gs, _)| *gs < outer_s);
                                flush_glue(glue, units, mod_prefix, mod_path);
                                let sym = join_symbol(mod_prefix, mod_path, &leaf);
                                units.push((sym, outer_s, outer_e, true));
                                // Recurse into class body if we have one.
                                if matches!(
                                    value.kind(),
                                    "class" | "class_declaration"
                                ) {
                                    if let Some(body) =
                                        value.child_by_field_name("body")
                                    {
                                        let mut np = mod_path.to_vec();
                                        np.push(leaf);
                                        walk_class_body(
                                            body, src, mod_prefix, &np, units,
                                        );
                                    }
                                }
                            }
                            _ => {
                                glue.push((0, s, e));
                            }
                        }
                    } else {
                        // Bare `export { x };` / `export * from "..."` —
                        // a re-export, glue with module-only flag set
                        // (we have no `declaration` / `value` field for
                        // it).
                        glue.push((1, s, e));
                    }
                }
                "import_statement" => {
                    glue.push((1, s, e));
                }
                "lexical_declaration" | "variable_declaration" => {
                    glue.push((0, s, e));
                }
                // Namespace / module declarations (rare in app code,
                // common in `.d.ts`): treat as glue per plan §Task H
                // (1B 1차 scope; documented under spec Risks).
                "internal_module" | "module" | "ambient_declaration" => {
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
        let only_module = glue.iter().all(|(is_mod, _, _)| *is_mod == 1);
        let label = if only_module { "<module>" } else { "<top-level>" };
        units.push((join_symbol(mod_prefix, mod_path, label), s, e, false));
        glue.clear();
    }

    walk(
        tree.root_node(),
        source,
        mod_prefix,
        &[],
        &mut units,
        &mut glue,
    );

    // `<module>` is correct only when the file produced no real unit.
    // Otherwise the import-only group becomes `<top-level>` (same
    // post-pass as 1A Gap 1 / Python).
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
            lang: Some("typescript".to_string()),
        };
        let block_id = id_for_block(doc_id, "code", &[], ordinal as u32, &span);
        let code = lines[(line_start as usize - 1)..=(line_end as usize - 1)].join("\n");
        blocks.push(Block::Code(CodeBlock {
            common: CommonBlock {
                block_id,
                heading_path: Vec::new(),
                source_span: span,
            },
            lang: Some("typescript".to_string()),
            code,
        }));
    }
    Ok(blocks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kebab_core::{Block, MediaType, SourceSpan};

    fn extract_fixture(name: &str, workspace_path: &str) -> kebab_core::CanonicalDocument {
        let bytes = std::fs::read(format!(
            concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/{}"),
            name
        ))
        .unwrap();
        let asset = crate::rust::tests_support::fixed_code_asset(workspace_path, "typescript");
        let cfg = kebab_core::ExtractConfig::default();
        let root = std::path::PathBuf::from("/tmp");
        let ctx = kebab_core::ExtractContext {
            asset: &asset,
            workspace_root: &root,
            config: &cfg,
        };
        TypescriptAstExtractor::new()
            .extract(&ctx, &bytes)
            .unwrap()
    }

    fn symbols(doc: &kebab_core::CanonicalDocument) -> Vec<String> {
        let mut s: Vec<String> = doc
            .blocks
            .iter()
            .filter_map(|b| match b {
                Block::Code(c) => match &c.common.source_span {
                    SourceSpan::Code { symbol, lang, .. } => {
                        assert_eq!(lang.as_deref(), Some("typescript"));
                        symbol.clone()
                    }
                    _ => None,
                },
                _ => None,
            })
            .collect();
        s.sort();
        s
    }

    #[test]
    fn extractor_supports_only_media_code_typescript() {
        let e = TypescriptAstExtractor::new();
        assert!(e.supports(&MediaType::Code("typescript".into())));
        assert!(!e.supports(&MediaType::Code("rust".into())));
        assert!(!e.supports(&MediaType::Markdown));
    }

    #[test]
    fn ts_units_match_design_3_4_symbols() {
        // workspace_path `src/sample.ts` → mod_prefix `src/sample`
        let doc = extract_fixture("sample.ts", "src/sample.ts");
        let syms = symbols(&doc);
        assert!(syms.iter().any(|s| s == "src/sample.add"), "got {syms:?}");
        assert!(syms.iter().any(|s| s == "src/sample.Greet"));
        assert!(syms.iter().any(|s| s == "src/sample.Maybe"));
        assert!(syms.iter().any(|s| s == "src/sample.Retriever"));
        assert!(syms.iter().any(|s| s == "src/sample.Retriever.search"));
        assert!(syms.iter().any(|s| s == "src/sample.Retriever.create"));
        assert!(syms.iter().any(|s| s == "src/sample.default"));
        assert!(syms.iter().any(|s| s == "src/sample.<top-level>"));
    }

    #[test]
    fn tsx_uses_tsx_grammar_and_emits_units() {
        let doc = extract_fixture("sample.tsx", "src/sample.tsx");
        let syms = symbols(&doc);
        assert!(
            syms.iter().any(|s| s == "src/sample.Hello"),
            "got {syms:?}"
        );
        assert!(
            syms.iter().any(|s| s == "src/sample.<top-level>"),
            "arrow fn + import should roll into top-level glue"
        );
    }

    #[test]
    fn deterministic_across_runs() {
        let a = extract_fixture("sample.ts", "src/sample.ts");
        for _ in 0..30 {
            assert_eq!(extract_fixture("sample.ts", "src/sample.ts").blocks, a.blocks);
        }
    }

    /// Regression: TS class-method decorators are `class_body` preceding
    /// siblings (not children of `method_definition`). The `unit_start`
    /// backward walk must fold the decorator line into the emitted unit's
    /// line range, matching Python's `decorated_definition` behavior.
    #[test]
    fn class_method_decorator_folded_into_method_unit() {
        // Line 1 (1-indexed): "class Foo {"
        // Line 2:             "    @Log()"   <- decorator
        // Line 3:             "    bar() { return 1; }"
        // Line 4:             "}"
        let bytes = b"class Foo {\n    @Log()\n    bar() { return 1; }\n}\n";
        let asset = crate::rust::tests_support::fixed_code_asset("src/foo.ts", "typescript");
        let cfg = kebab_core::ExtractConfig::default();
        let root = std::path::PathBuf::from("/tmp");
        let ctx = kebab_core::ExtractContext {
            asset: &asset,
            workspace_root: &root,
            config: &cfg,
        };
        let doc = TypescriptAstExtractor::new().extract(&ctx, bytes).unwrap();

        let bar_block = doc
            .blocks
            .iter()
            .find_map(|b| match b {
                Block::Code(c) => match &c.common.source_span {
                    SourceSpan::Code { symbol, .. }
                        if symbol.as_deref() == Some("src/foo.Foo.bar") =>
                    {
                        Some(c)
                    }
                    _ => None,
                },
                _ => None,
            })
            .expect("src/foo.Foo.bar block should be present");

        // After the fix, the unit MUST include the @Log() decorator line.
        assert!(
            bar_block.code.contains("@Log()"),
            "decorator must be folded into class-method unit (Python parity); got code: {:?}",
            bar_block.code
        );

        // line_start must be 2 (the @Log() line), NOT 3 (the bar() line).
        match &bar_block.common.source_span {
            SourceSpan::Code { line_start, .. } => {
                assert_eq!(
                    *line_start, 2,
                    "line_start must cover the @Log() decorator line (got {line_start})"
                );
            }
            _ => unreachable!(),
        }
    }

    /// Class-level decorator (preceding sibling of `class_declaration` in
    /// the module root): same `unit_start` backward walk folds it in.
    /// Line 1: "@Injectable()"
    /// Line 2: "class Service {"
    /// Line 3: "}"
    #[test]
    fn ts_class_decorator_folded_into_class_unit() {
        let bytes = b"@Injectable()\nclass Service {\n}\n";
        let asset = crate::rust::tests_support::fixed_code_asset("src/svc.ts", "typescript");
        let cfg = kebab_core::ExtractConfig::default();
        let root = std::path::PathBuf::from("/tmp");
        let ctx = kebab_core::ExtractContext {
            asset: &asset,
            workspace_root: &root,
            config: &cfg,
        };
        let doc = TypescriptAstExtractor::new().extract(&ctx, bytes).unwrap();

        let svc_block = doc
            .blocks
            .iter()
            .find_map(|b| match b {
                Block::Code(c) => match &c.common.source_span {
                    SourceSpan::Code { symbol, .. }
                        if symbol.as_deref() == Some("src/svc.Service") =>
                    {
                        Some(c)
                    }
                    _ => None,
                },
                _ => None,
            })
            .expect("src/svc.Service block should be present");

        assert!(
            svc_block.code.contains("@Injectable()"),
            "class-level decorator must be folded into the class unit; got code: {:?}",
            svc_block.code
        );
        match &svc_block.common.source_span {
            SourceSpan::Code { line_start, .. } => {
                assert_eq!(
                    *line_start, 1,
                    "line_start must cover the @Injectable() line (got {line_start})"
                );
            }
            _ => unreachable!(),
        }
    }
}
