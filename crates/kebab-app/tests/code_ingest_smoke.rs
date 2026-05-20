//! p10-1A-2 Task 8: smoke test for Rust code ingest dispatch.
//!
//! Writes a single `.rs` file into a TempDir workspace, ingests it via
//! `kebab_app::ingest_with_config`, then searches for the symbol name and
//! asserts that the resulting `SearchHit` carries a `Citation::Code`
//! with the expected `lang`, `symbol`, and `line_start`.
//!
//! Mirrors the `pdf_pipeline.rs` harness: lexical-only (no AVX/fastembed),
//! no OCR / caption adapters needed.

mod common;

use common::{TestEnv, lexical_query};

use kebab_core::{Citation, IngestItemKind};

/// A `.rs` file with a single `pub fn add` symbol is ingested, and a
/// lexical search for "add" must return at least one `Citation::Code`
/// hit whose `lang == "rust"`, `symbol == Some("add")`, and
/// `line_start >= 1`.
#[test]
fn rust_file_ingests_and_searches_as_code_citation() {
    let env = TestEnv::lexical_only();

    // Write a minimal Rust file into the workspace root.
    std::fs::write(
        env.workspace_root.join("demo.rs"),
        "/// adds two integers\npub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n",
    )
    .unwrap();

    let report =
        kebab_app::ingest_with_config(env.config.clone(), env.scope(), false)
            .expect("ingest must succeed");

    assert_eq!(report.errors, 0, "no errors expected: {report:?}");
    let items = report.items.as_ref().expect("items present");
    let code_item = items
        .iter()
        .find(|i| i.doc_path.0.ends_with("demo.rs"))
        .expect("demo.rs item present");
    assert_eq!(
        code_item.kind,
        IngestItemKind::New,
        "first ingest must be New: {code_item:?}"
    );
    assert!(
        code_item.block_count.unwrap_or(0) >= 1,
        "at least one block expected: {code_item:?}"
    );
    assert!(
        code_item.chunk_count.unwrap_or(0) >= 1,
        "at least one chunk expected: {code_item:?}"
    );
    assert_eq!(
        code_item.parser_version.as_ref().map(|p| p.0.as_str()),
        Some("code-rust-v1"),
        "parser_version must be code-rust-v1"
    );
    assert_eq!(
        code_item.chunker_version.as_ref().map(|c| c.0.as_str()),
        Some("code-rust-ast-v1"),
        "chunker_version must be code-rust-ast-v1"
    );

    // Lexical search for the symbol name "add".
    let hits = kebab_app::search_with_config(env.config.clone(), lexical_query("add"))
        .expect("search must succeed");

    let h = hits
        .iter()
        .find(|h| matches!(&h.citation, Citation::Code { .. }))
        .expect("at least one Citation::Code hit for 'add'");

    match &h.citation {
        Citation::Code {
            lang,
            symbol,
            line_start,
            ..
        } => {
            assert_eq!(
                lang.as_deref(),
                Some("rust"),
                "citation.lang must be 'rust'"
            );
            assert_eq!(
                symbol.as_deref(),
                Some("add"),
                "citation.symbol must be 'add'"
            );
            assert!(*line_start >= 1, "line_start must be ≥1");
        }
        _ => unreachable!(),
    }

    assert_eq!(
        h.code_lang.as_deref(),
        Some("rust"),
        "SearchHit.code_lang must be 'rust'"
    );
}

/// p10-1A-2 Task 8b: a code search hit must carry `SearchHit.repo` filled
/// from the document's `Metadata.repo` (which is set by `detect_repo` during
/// ingest). `detect_repo` returns the name of the directory that contains
/// `.git/`, so we `git init` the workspace root before ingesting and then
/// assert that `h.repo == Some("workspace")`.
#[test]
fn rust_code_search_hit_has_repo() {
    let env = TestEnv::lexical_only();

    // `detect_repo` walks up from the file looking for `.git/`.
    // Initialise a bare git repo at the workspace root so it is
    // discoverable. We only need the `.git/` directory — no commits
    // required.
    let git_status = std::process::Command::new("git")
        .args(["init", "--quiet"])
        .arg(env.workspace_root.as_os_str())
        .status()
        .expect("git init");
    assert!(git_status.success(), "git init must succeed");

    std::fs::write(
        env.workspace_root.join("repo_demo.rs"),
        "/// multiplies two integers\npub fn mul(a: i32, b: i32) -> i32 {\n    a * b\n}\n",
    )
    .unwrap();

    let report =
        kebab_app::ingest_with_config(env.config.clone(), env.scope(), false)
            .expect("ingest must succeed");
    assert_eq!(report.errors, 0, "no ingest errors: {report:?}");

    let hits = kebab_app::search_with_config(env.config.clone(), lexical_query("mul"))
        .expect("search must succeed");

    let h = hits
        .iter()
        .find(|h| matches!(&h.citation, Citation::Code { .. }))
        .expect("at least one Citation::Code hit for 'mul'");

    // The workspace root directory is named "workspace" by `TestEnv`.
    let expected_repo = env
        .workspace_root
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_owned);
    assert_eq!(
        h.repo,
        expected_repo,
        "SearchHit.repo must match the workspace dir name (detect_repo result)"
    );
    // Also sanity-check code_lang is still filled.
    assert_eq!(
        h.code_lang.as_deref(),
        Some("rust"),
        "SearchHit.code_lang must be 'rust'"
    );
}

/// p10-1b Task G: a `.py` file in a sub-directory is ingested and the
/// resulting `Citation::Code` hit must carry `lang="python"`,
/// `symbol="kebab_eval.metrics.compute_mrr"`, and `line_start >= 1`.
/// The sub-directory (`kebab_eval/`) ensures `module_path_for_python`
/// produces a non-empty prefix so the fully-qualified symbol assertion
/// exercises the prefix wiring end-to-end.
#[test]
fn python_file_ingests_and_searches_as_code_citation() {
    let env = TestEnv::lexical_only();

    let module_dir = env.workspace_root.join("kebab_eval");
    std::fs::create_dir_all(&module_dir).unwrap();
    std::fs::write(
        module_dir.join("metrics.py"),
        "\"\"\"compute metrics.\"\"\"\ndef compute_mrr(scores):\n    return sum(scores) / max(len(scores), 1)\n",
    )
    .unwrap();

    let report =
        kebab_app::ingest_with_config(env.config.clone(), env.scope(), false)
            .expect("ingest must succeed");

    assert!(report.new >= 1, "python file ingested: {report:?}");

    let items = report.items.as_ref().expect("items present");
    let py_item = items
        .iter()
        .find(|i| i.doc_path.0.ends_with("metrics.py"))
        .expect("metrics.py item");
    assert_eq!(
        py_item.parser_version.as_ref().map(|p| p.0.as_str()),
        Some("code-python-v1"),
        "parser_version must be code-python-v1"
    );
    assert_eq!(
        py_item.chunker_version.as_ref().map(|c| c.0.as_str()),
        Some("code-python-ast-v1"),
        "chunker_version must be code-python-ast-v1"
    );

    let hits = kebab_app::search_with_config(env.config.clone(), lexical_query("compute_mrr"))
        .expect("search must succeed");

    let h = hits
        .iter()
        .find(|h| matches!(&h.citation, Citation::Code { .. }))
        .expect("at least one Citation::Code hit for 'compute_mrr'");

    match &h.citation {
        Citation::Code {
            lang,
            symbol,
            line_start,
            ..
        } => {
            assert_eq!(
                lang.as_deref(),
                Some("python"),
                "citation.lang must be 'python'"
            );
            assert_eq!(
                symbol.as_deref(),
                Some("kebab_eval.metrics.compute_mrr"),
                "citation.symbol must be 'kebab_eval.metrics.compute_mrr'"
            );
            assert!(*line_start >= 1, "line_start must be >=1");
        }
        _ => unreachable!(),
    }

    assert_eq!(
        h.code_lang.as_deref(),
        Some("python"),
        "SearchHit.code_lang must be 'python'"
    );
}

/// p10-1b Task J: a `.ts` file in a sub-directory is ingested and the
/// resulting `Citation::Code` hit must carry `lang="typescript"`,
/// `symbol="src/Foo.Foo.bar"`, and `line_start >= 1`.
/// The sub-directory (`src/`) ensures `module_path_for_tsjs` produces
/// a non-empty prefix so the fully-qualified symbol assertion exercises
/// the prefix wiring end-to-end.
#[test]
fn typescript_file_ingests_and_searches_as_code_citation() {
    let env = TestEnv::lexical_only();

    let src_dir = env.workspace_root.join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(
        src_dir.join("Foo.ts"),
        "export class Foo {\n    bar(): number { return 42; }\n}\n",
    )
    .unwrap();

    let report =
        kebab_app::ingest_with_config(env.config.clone(), env.scope(), false)
            .expect("ingest must succeed");

    assert!(report.new >= 1, "ts file ingested: {report:?}");

    let items = report.items.as_ref().expect("items present");
    let ts_item = items
        .iter()
        .find(|i| i.doc_path.0.ends_with("Foo.ts"))
        .expect("Foo.ts item");
    assert_eq!(
        ts_item.parser_version.as_ref().map(|p| p.0.as_str()),
        Some("code-typescript-v1"),
        "parser_version must be code-typescript-v1"
    );
    assert_eq!(
        ts_item.chunker_version.as_ref().map(|c| c.0.as_str()),
        Some("code-ts-ast-v1"),
        "chunker_version must be code-ts-ast-v1"
    );

    let hits = kebab_app::search_with_config(env.config.clone(), lexical_query("bar"))
        .expect("search must succeed");

    let h = hits
        .iter()
        .find(|h| matches!(&h.citation, Citation::Code { .. }))
        .expect("at least one Citation::Code hit for 'bar'");

    match &h.citation {
        Citation::Code {
            lang,
            symbol,
            line_start,
            ..
        } => {
            assert_eq!(
                lang.as_deref(),
                Some("typescript"),
                "citation.lang must be 'typescript'"
            );
            assert_eq!(
                symbol.as_deref(),
                Some("src/Foo.Foo.bar"),
                "citation.symbol must be 'src/Foo.Foo.bar'"
            );
            assert!(*line_start >= 1, "line_start must be >=1");
        }
        _ => unreachable!(),
    }

    assert_eq!(
        h.code_lang.as_deref(),
        Some("typescript"),
        "SearchHit.code_lang must be 'typescript'"
    );
}

/// Re-ingesting the same `.rs` file without changes must report
/// `Unchanged` (incremental-skip path exercised).
#[test]
fn rust_file_re_ingest_is_unchanged() {
    let env = TestEnv::lexical_only();

    std::fs::write(
        env.workspace_root.join("stable.rs"),
        "pub fn noop() {}\n",
    )
    .unwrap();

    let r1 =
        kebab_app::ingest_with_config(env.config.clone(), env.scope(), false).unwrap();
    let item1 = r1
        .items
        .as_ref()
        .unwrap()
        .iter()
        .find(|i| i.doc_path.0.ends_with("stable.rs"))
        .cloned()
        .unwrap();
    assert_eq!(item1.kind, IngestItemKind::New);

    let r2 =
        kebab_app::ingest_with_config(env.config.clone(), env.scope(), false).unwrap();
    let item2 = r2
        .items
        .unwrap()
        .into_iter()
        .find(|i| i.doc_path.0.ends_with("stable.rs"))
        .unwrap();
    assert_eq!(
        item2.kind,
        IngestItemKind::Unchanged,
        "identical bytes → Unchanged"
    );
    assert_eq!(item2.doc_id, item1.doc_id);
}
