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
        Some("code-ts-v1"),
        "parser_version must be code-ts-v1"
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

/// p10-1b Task L: a `.js` file in a sub-directory is ingested and the
/// resulting `Citation::Code` hit must carry `lang="javascript"`,
/// `symbol="src/Bar.Bar.baz"`, and `line_start >= 1`.
/// The sub-directory (`src/`) ensures `module_path_for_tsjs` produces
/// a non-empty prefix so the fully-qualified symbol assertion exercises
/// the prefix wiring end-to-end.
#[test]
fn javascript_file_ingests_and_searches_as_code_citation() {
    let env = TestEnv::lexical_only();

    let src_dir = env.workspace_root.join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(
        src_dir.join("Bar.js"),
        "export class Bar {\n    baz() { return 7; }\n}\n",
    )
    .unwrap();

    let report =
        kebab_app::ingest_with_config(env.config.clone(), env.scope(), false)
            .expect("ingest must succeed");

    assert!(report.new >= 1, "js file ingested: {report:?}");

    let items = report.items.as_ref().expect("items present");
    let js_item = items
        .iter()
        .find(|i| i.doc_path.0.ends_with("Bar.js"))
        .expect("Bar.js item");
    assert_eq!(
        js_item.parser_version.as_ref().map(|p| p.0.as_str()),
        Some("code-js-v1"),
        "parser_version must be code-js-v1"
    );
    assert_eq!(
        js_item.chunker_version.as_ref().map(|c| c.0.as_str()),
        Some("code-js-ast-v1"),
        "chunker_version must be code-js-ast-v1"
    );

    let hits = kebab_app::search_with_config(env.config.clone(), lexical_query("baz"))
        .expect("search must succeed");

    let h = hits
        .iter()
        .find(|h| matches!(&h.citation, Citation::Code { .. }))
        .expect("at least one Citation::Code hit for 'baz'");

    match &h.citation {
        Citation::Code {
            lang,
            symbol,
            line_start,
            ..
        } => {
            assert_eq!(
                lang.as_deref(),
                Some("javascript"),
                "citation.lang must be 'javascript'"
            );
            assert_eq!(
                symbol.as_deref(),
                Some("src/Bar.Bar.baz"),
                "citation.symbol must be 'src/Bar.Bar.baz'"
            );
            assert!(*line_start >= 1, "line_start must be >=1");
        }
        _ => unreachable!(),
    }

    assert_eq!(
        h.code_lang.as_deref(),
        Some("javascript"),
        "SearchHit.code_lang must be 'javascript'"
    );
}

/// p10-1c-go Task F: a `.go` file in a sub-directory is ingested and the
/// resulting `Citation::Code` hit must carry `lang="go"`,
/// `symbol="chunk.ParseDoc"`, and `line_start >= 1`.
/// The sub-directory (`chunk/`) ensures the Go package-prefix wiring
/// produces a non-empty module prefix so the fully-qualified symbol assertion
/// exercises that path end-to-end.
#[test]
fn go_file_ingests_and_searches_as_code_citation() {
    let env = TestEnv::lexical_only();

    let pkg_dir = env.workspace_root.join("chunk");
    std::fs::create_dir_all(&pkg_dir).unwrap();
    std::fs::write(
        pkg_dir.join("ast.go"),
        "package chunk\n\nfunc ParseDoc(input string) string {\n    return input\n}\n",
    )
    .unwrap();

    let report = kebab_app::ingest_with_config(env.config.clone(), env.scope(), false)
        .expect("ingest must succeed");
    assert_eq!(report.errors, 0);
    assert!(report.new >= 1);

    let go_item = report
        .items
        .as_ref()
        .expect("items present")
        .iter()
        .find(|i| i.doc_path.0.ends_with("ast.go"))
        .expect("ast.go item present");
    assert_eq!(
        go_item.parser_version.as_ref().map(|p| p.0.as_str()),
        Some("code-go-v1"),
        "parser_version must be code-go-v1"
    );
    assert_eq!(
        go_item.chunker_version.as_ref().map(|c| c.0.as_str()),
        Some("code-go-ast-v1"),
        "chunker_version must be code-go-ast-v1"
    );

    let hits = kebab_app::search_with_config(env.config.clone(), lexical_query("ParseDoc"))
        .expect("search must succeed");
    let h = hits
        .iter()
        .find(|h| matches!(&h.citation, kebab_core::Citation::Code { .. }))
        .expect("Citation::Code hit");
    match &h.citation {
        kebab_core::Citation::Code {
            lang,
            symbol,
            line_start,
            ..
        } => {
            assert_eq!(lang.as_deref(), Some("go"), "citation.lang must be 'go'");
            assert_eq!(
                symbol.as_deref(),
                Some("chunk.ParseDoc"),
                "citation.symbol must be 'chunk.ParseDoc'"
            );
            assert!(*line_start >= 1, "line_start must be >=1");
        }
        _ => unreachable!(),
    }
    assert_eq!(
        h.code_lang.as_deref(),
        Some("go"),
        "SearchHit.code_lang must be 'go'"
    );
}

/// p10-1c-jk Task F: a `.java` file in a package directory is ingested and the
/// resulting `Citation::Code` hit must carry `lang="java"`,
/// `symbol="com.foo.Foo.bar"`, and `line_start >= 1`.
/// The sub-directory (`com/foo/`) ensures the Java package-prefix wiring
/// produces a non-empty module prefix so the fully-qualified symbol assertion
/// exercises that path end-to-end.
#[test]
fn java_file_ingests_and_searches_as_code_citation() {
    let env = TestEnv::lexical_only();

    let pkg_dir = env.workspace_root.join("com").join("foo");
    std::fs::create_dir_all(&pkg_dir).unwrap();
    std::fs::write(
        pkg_dir.join("Foo.java"),
        "package com.foo;\n\npublic class Foo {\n    public String bar() { return \"x\"; }\n}\n",
    )
    .unwrap();

    let report = kebab_app::ingest_with_config(env.config.clone(), env.scope(), false)
        .expect("ingest must succeed");
    assert_eq!(report.errors, 0);
    assert!(report.new >= 1);

    let java_item = report
        .items
        .as_ref()
        .expect("items present")
        .iter()
        .find(|i| i.doc_path.0.ends_with("Foo.java"))
        .expect("Foo.java item present");
    assert_eq!(
        java_item.parser_version.as_ref().map(|p| p.0.as_str()),
        Some("code-java-v1"),
        "parser_version must be code-java-v1"
    );
    assert_eq!(
        java_item.chunker_version.as_ref().map(|c| c.0.as_str()),
        Some("code-java-ast-v1"),
        "chunker_version must be code-java-ast-v1"
    );

    let hits = kebab_app::search_with_config(env.config.clone(), lexical_query("bar"))
        .expect("search must succeed");
    let h = hits
        .iter()
        .find(|h| matches!(&h.citation, kebab_core::Citation::Code { .. }))
        .expect("Citation::Code hit");
    match &h.citation {
        kebab_core::Citation::Code {
            lang,
            symbol,
            line_start,
            ..
        } => {
            assert_eq!(lang.as_deref(), Some("java"), "citation.lang must be 'java'");
            assert_eq!(
                symbol.as_deref(),
                Some("com.foo.Foo.bar"),
                "citation.symbol must be 'com.foo.Foo.bar'"
            );
            assert!(*line_start >= 1, "line_start must be >=1");
        }
        _ => unreachable!(),
    }
    assert_eq!(
        h.code_lang.as_deref(),
        Some("java"),
        "SearchHit.code_lang must be 'java'"
    );
}

/// p10-1c-jk Task I: a `.kt` file in a package directory is ingested and the
/// resulting `Citation::Code` hit must carry `lang="kotlin"`,
/// `symbol="com.foo.Foo.bar"`, and `line_start >= 1`.
/// The sub-directory (`com/foo/`) ensures the Kotlin package-prefix wiring
/// produces a non-empty module prefix so the fully-qualified symbol assertion
/// exercises that path end-to-end.
#[test]
fn kotlin_file_ingests_and_searches_as_code_citation() {
    let env = TestEnv::lexical_only();

    let pkg_dir = env.workspace_root.join("com").join("foo");
    std::fs::create_dir_all(&pkg_dir).unwrap();
    std::fs::write(
        pkg_dir.join("Foo.kt"),
        "package com.foo\n\nclass Foo {\n    fun bar(): String = \"x\"\n}\n",
    )
    .unwrap();

    let report = kebab_app::ingest_with_config(env.config.clone(), env.scope(), false)
        .expect("ingest must succeed");
    assert_eq!(report.errors, 0);
    assert!(report.new >= 1);

    let kt_item = report
        .items
        .as_ref()
        .expect("items present")
        .iter()
        .find(|i| i.doc_path.0.ends_with("Foo.kt"))
        .expect("Foo.kt item present");
    assert_eq!(
        kt_item.parser_version.as_ref().map(|p| p.0.as_str()),
        Some("code-kotlin-v1"),
        "parser_version must be code-kotlin-v1"
    );
    assert_eq!(
        kt_item.chunker_version.as_ref().map(|c| c.0.as_str()),
        Some("code-kotlin-ast-v1"),
        "chunker_version must be code-kotlin-ast-v1"
    );

    let hits = kebab_app::search_with_config(env.config.clone(), lexical_query("bar"))
        .expect("search must succeed");
    let h = hits
        .iter()
        .find(|h| matches!(&h.citation, kebab_core::Citation::Code { .. }))
        .expect("Citation::Code hit");
    match &h.citation {
        kebab_core::Citation::Code {
            lang,
            symbol,
            line_start,
            ..
        } => {
            assert_eq!(lang.as_deref(), Some("kotlin"), "citation.lang must be 'kotlin'");
            assert_eq!(
                symbol.as_deref(),
                Some("com.foo.Foo.bar"),
                "citation.symbol must be 'com.foo.Foo.bar'"
            );
            assert!(*line_start >= 1, "line_start must be >=1");
        }
        _ => unreachable!(),
    }
    assert_eq!(
        h.code_lang.as_deref(),
        Some("kotlin"),
        "SearchHit.code_lang must be 'kotlin'"
    );
}

/// p10-2 Task H: a `k8s/deploy.yaml` file with a Deployment resource is
/// ingested and the resulting `Citation::Code` hit must carry
/// `lang="yaml"`, `symbol="Deployment/prod/api"`, and `line_start >= 1`.
/// Exercises the k8s-manifest-resource-v1 chunker end-to-end.
#[test]
fn tier2_k8s_yaml_ingest_searchable() {
    let env = TestEnv::lexical_only();

    let k8s_dir = env.workspace_root.join("k8s");
    std::fs::create_dir_all(&k8s_dir).unwrap();
    std::fs::write(
        k8s_dir.join("deploy.yaml"),
        "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: api\n  namespace: prod\nspec:\n  replicas: 1\n",
    )
    .unwrap();

    let report = kebab_app::ingest_with_config(env.config.clone(), env.scope(), false)
        .expect("ingest must succeed");
    assert_eq!(report.errors, 0, "no ingest errors: {report:?}");
    assert!(report.new >= 1, "yaml file ingested: {report:?}");

    let yaml_item = report
        .items
        .as_ref()
        .expect("items present")
        .iter()
        .find(|i| i.doc_path.0.ends_with("deploy.yaml"))
        .expect("deploy.yaml item present");
    assert_eq!(
        yaml_item.parser_version.as_ref().map(|p| p.0.as_str()),
        Some("none-v1"),
        "parser_version must be none-v1"
    );
    assert_eq!(
        yaml_item.chunker_version.as_ref().map(|c| c.0.as_str()),
        Some("k8s-manifest-resource-v1"),
        "chunker_version must be k8s-manifest-resource-v1"
    );

    let query = kebab_core::SearchQuery {
        text: "api".to_string(),
        mode: kebab_core::SearchMode::Lexical,
        k: 10,
        filters: kebab_core::SearchFilters {
            code_lang: vec!["yaml".to_string()],
            ..Default::default()
        },
    };
    let hits = kebab_app::search_with_config(env.config.clone(), query)
        .expect("search must succeed");

    let h = hits
        .iter()
        .find(|h| matches!(&h.citation, Citation::Code { .. }))
        .expect("at least one Citation::Code hit for 'api'");

    match &h.citation {
        Citation::Code {
            lang,
            symbol,
            line_start,
            ..
        } => {
            assert_eq!(lang.as_deref(), Some("yaml"), "citation.lang must be 'yaml'");
            assert_eq!(
                symbol.as_deref(),
                Some("Deployment/prod/api"),
                "citation.symbol must be 'Deployment/prod/api'"
            );
            assert!(*line_start >= 1, "line_start must be >=1");
        }
        _ => unreachable!(),
    }

    assert_eq!(
        h.code_lang.as_deref(),
        Some("yaml"),
        "SearchHit.code_lang must be 'yaml'"
    );
}

/// p10-2 Task H: a `Dockerfile` is ingested and the resulting
/// `Citation::Code` hit must carry `lang="dockerfile"`,
/// `symbol="<dockerfile>"`, and `line_start >= 1`.
/// Exercises the dockerfile-file-v1 chunker end-to-end.
#[test]
fn tier2_dockerfile_ingest_searchable() {
    let env = TestEnv::lexical_only();

    std::fs::write(
        env.workspace_root.join("Dockerfile"),
        "FROM rust:1.94\nRUN cargo install foo\n",
    )
    .unwrap();

    let report = kebab_app::ingest_with_config(env.config.clone(), env.scope(), false)
        .expect("ingest must succeed");
    assert_eq!(report.errors, 0, "no ingest errors: {report:?}");
    assert!(report.new >= 1, "Dockerfile ingested: {report:?}");

    let df_item = report
        .items
        .as_ref()
        .expect("items present")
        .iter()
        .find(|i| i.doc_path.0.ends_with("Dockerfile"))
        .expect("Dockerfile item present");
    assert_eq!(
        df_item.parser_version.as_ref().map(|p| p.0.as_str()),
        Some("none-v1"),
        "parser_version must be none-v1"
    );
    assert_eq!(
        df_item.chunker_version.as_ref().map(|c| c.0.as_str()),
        Some("dockerfile-file-v1"),
        "chunker_version must be dockerfile-file-v1"
    );

    let query = kebab_core::SearchQuery {
        text: "cargo".to_string(),
        mode: kebab_core::SearchMode::Lexical,
        k: 10,
        filters: kebab_core::SearchFilters {
            code_lang: vec!["dockerfile".to_string()],
            ..Default::default()
        },
    };
    let hits = kebab_app::search_with_config(env.config.clone(), query)
        .expect("search must succeed");

    let h = hits
        .iter()
        .find(|h| matches!(&h.citation, Citation::Code { .. }))
        .expect("at least one Citation::Code hit for 'cargo'");

    match &h.citation {
        Citation::Code {
            lang,
            symbol,
            line_start,
            ..
        } => {
            assert_eq!(
                lang.as_deref(),
                Some("dockerfile"),
                "citation.lang must be 'dockerfile'"
            );
            assert_eq!(
                symbol.as_deref(),
                Some("<dockerfile>"),
                "citation.symbol must be '<dockerfile>'"
            );
            assert!(*line_start >= 1, "line_start must be >=1");
        }
        _ => unreachable!(),
    }

    assert_eq!(
        h.code_lang.as_deref(),
        Some("dockerfile"),
        "SearchHit.code_lang must be 'dockerfile'"
    );
}

/// p10-2 Task H: a `Cargo.toml` manifest is ingested and the resulting
/// `Citation::Code` hit must carry `lang="toml"`, `symbol="<manifest>"`,
/// and `line_start >= 1`.
/// Exercises the manifest-file-v1 chunker end-to-end.
#[test]
fn tier2_cargo_toml_ingest_searchable() {
    let env = TestEnv::lexical_only();

    std::fs::write(
        env.workspace_root.join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();

    let report = kebab_app::ingest_with_config(env.config.clone(), env.scope(), false)
        .expect("ingest must succeed");
    assert_eq!(report.errors, 0, "no ingest errors: {report:?}");
    assert!(report.new >= 1, "Cargo.toml ingested: {report:?}");

    let toml_item = report
        .items
        .as_ref()
        .expect("items present")
        .iter()
        .find(|i| i.doc_path.0.ends_with("Cargo.toml"))
        .expect("Cargo.toml item present");
    assert_eq!(
        toml_item.parser_version.as_ref().map(|p| p.0.as_str()),
        Some("none-v1"),
        "parser_version must be none-v1"
    );
    assert_eq!(
        toml_item.chunker_version.as_ref().map(|c| c.0.as_str()),
        Some("manifest-file-v1"),
        "chunker_version must be manifest-file-v1"
    );

    let query = kebab_core::SearchQuery {
        text: "demo".to_string(),
        mode: kebab_core::SearchMode::Lexical,
        k: 10,
        filters: kebab_core::SearchFilters {
            code_lang: vec!["toml".to_string()],
            ..Default::default()
        },
    };
    let hits = kebab_app::search_with_config(env.config.clone(), query)
        .expect("search must succeed");

    let h = hits
        .iter()
        .find(|h| matches!(&h.citation, Citation::Code { .. }))
        .expect("at least one Citation::Code hit for 'demo'");

    match &h.citation {
        Citation::Code {
            lang,
            symbol,
            line_start,
            ..
        } => {
            assert_eq!(
                lang.as_deref(),
                Some("toml"),
                "citation.lang must be 'toml'"
            );
            assert_eq!(
                symbol.as_deref(),
                Some("<manifest>"),
                "citation.symbol must be '<manifest>'"
            );
            assert!(*line_start >= 1, "line_start must be >=1");
        }
        _ => unreachable!(),
    }

    assert_eq!(
        h.code_lang.as_deref(),
        Some("toml"),
        "SearchHit.code_lang must be 'toml'"
    );
}

/// p10-3 Task E: a `.sh` file is ingested via the shell direct-Tier-3 path
/// and the resulting `Citation::Code` hit must carry `lang="shell"`,
/// `symbol=None`, `line_start >= 1`, and
/// `chunker_version = "code-text-paragraph-v1"`.
#[test]
fn tier3_shell_ingest_searchable() {
    let env = TestEnv::lexical_only();

    std::fs::write(
        env.workspace_root.join("deploy.sh"),
        "#!/usr/bin/env bash\nset -e\necho hello\n\nkebab ingest --json\n",
    )
    .unwrap();

    let report = kebab_app::ingest_with_config(env.config.clone(), env.scope(), false)
        .expect("ingest must succeed");
    assert_eq!(report.errors, 0, "no ingest errors: {report:?}");
    assert!(report.new >= 1, "shell file ingested: {report:?}");

    let sh_item = report
        .items
        .as_ref()
        .expect("items present")
        .iter()
        .find(|i| i.doc_path.0.ends_with("deploy.sh"))
        .expect("deploy.sh item present");
    assert_eq!(
        sh_item.parser_version.as_ref().map(|p| p.0.as_str()),
        Some("none-v1"),
        "parser_version must be none-v1 for shell (Tier 3 direct)"
    );
    assert_eq!(
        sh_item.chunker_version.as_ref().map(|c| c.0.as_str()),
        Some("code-text-paragraph-v1"),
        "chunker_version must be code-text-paragraph-v1 for shell"
    );

    let query = kebab_core::SearchQuery {
        text: "kebab".to_string(),
        mode: kebab_core::SearchMode::Lexical,
        k: 10,
        filters: kebab_core::SearchFilters {
            code_lang: vec!["shell".to_string()],
            ..Default::default()
        },
    };
    let hits = kebab_app::search_with_config(env.config.clone(), query)
        .expect("search must succeed");

    let h = hits
        .iter()
        .find(|h| matches!(&h.citation, Citation::Code { .. }))
        .expect("at least one Citation::Code hit for 'kebab'");

    match &h.citation {
        Citation::Code {
            lang,
            symbol,
            line_start,
            ..
        } => {
            assert_eq!(
                lang.as_deref(),
                Some("shell"),
                "citation.lang must be 'shell'"
            );
            assert_eq!(*symbol, None, "Tier 3 symbol must be None");
            assert!(*line_start >= 1, "line_start must be >=1");
        }
        _ => unreachable!(),
    }

    assert_eq!(
        h.code_lang.as_deref(),
        Some("shell"),
        "SearchHit.code_lang must be 'shell'"
    );
    assert_eq!(
        h.chunker_version.0.as_str(),
        "code-text-paragraph-v1",
        "shell chunks must be stamped with the Tier 3 chunker_version"
    );
}

/// p10-3 Task E: a docker-compose-shaped YAML file (no `apiVersion`/`kind`)
/// is ingested; the k8s chunker returns `Ok(vec![])` and the Tier 3 fallback
/// wrapper retries with `CodeTextParagraphV1Chunker`. The resulting
/// `Citation::Code` hit must carry `lang="yaml"`, `symbol=None`,
/// `line_start >= 1`, and `chunker_version = "code-text-paragraph-v1"`.
#[test]
fn tier3_yaml_fallback_picks_up_non_k8s_yaml() {
    let env = TestEnv::lexical_only();

    // docker-compose-shaped YAML — version + services but no apiVersion/kind.
    // The k8s chunker returns Ok(vec![]); Tier 3 fallback should pick this up.
    std::fs::write(
        env.workspace_root.join("docker-compose.yml"),
        "version: '3'\nservices:\n  api:\n    image: nginx:latest\n    ports:\n      - 8080:80\n",
    )
    .unwrap();

    let report = kebab_app::ingest_with_config(env.config.clone(), env.scope(), false)
        .expect("ingest must succeed");
    assert_eq!(report.errors, 0, "no ingest errors: {report:?}");
    assert!(
        report.new >= 1,
        "expected non-k8s yaml ingested via Tier 3, got {} new docs",
        report.new
    );

    let yaml_item = report
        .items
        .as_ref()
        .expect("items present")
        .iter()
        .find(|i| i.doc_path.0.ends_with("docker-compose.yml"))
        .expect("docker-compose.yml item present");
    assert_eq!(
        yaml_item.parser_version.as_ref().map(|p| p.0.as_str()),
        Some("none-v1"),
        "parser_version must be none-v1 after Tier 3 fallback"
    );
    assert_eq!(
        yaml_item.chunker_version.as_ref().map(|c| c.0.as_str()),
        Some("code-text-paragraph-v1"),
        "chunker_version must be code-text-paragraph-v1 after Tier 3 fallback"
    );

    let query = kebab_core::SearchQuery {
        text: "nginx".to_string(),
        mode: kebab_core::SearchMode::Lexical,
        k: 10,
        filters: kebab_core::SearchFilters {
            code_lang: vec!["yaml".to_string()],
            ..Default::default()
        },
    };
    let hits = kebab_app::search_with_config(env.config.clone(), query)
        .expect("search must succeed");

    let h = hits
        .iter()
        .find(|h| matches!(&h.citation, Citation::Code { .. }))
        .expect("at least one Citation::Code hit for 'nginx'");

    match &h.citation {
        Citation::Code {
            lang,
            symbol,
            line_start,
            ..
        } => {
            assert_eq!(
                lang.as_deref(),
                Some("yaml"),
                "citation.lang must be 'yaml'"
            );
            assert_eq!(*symbol, None, "Tier 3 fallback symbol must be None");
            assert!(*line_start >= 1, "line_start must be >=1");
        }
        _ => unreachable!(),
    }

    assert_eq!(
        h.code_lang.as_deref(),
        Some("yaml"),
        "SearchHit.code_lang must be 'yaml'"
    );
    assert_eq!(
        h.chunker_version.0.as_str(),
        "code-text-paragraph-v1",
        "non-k8s yaml fallback must be stamped code-text-paragraph-v1"
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

/// p10-3 fix regression: a docker-compose YAML that falls back to Tier 3
/// (k8s chunker returns empty, CodeTextParagraphV1Chunker retries) must
/// report Unchanged on the second ingest rather than re-processing.
/// Before the fix, try_skip_unchanged returned None because the stored
/// last_chunker_version ("code-text-paragraph-v1" / parser_version
/// "none-v1") never matched the caller's dispatch values.
#[test]
fn tier3_yaml_fallback_reingest_is_unchanged() {
    let env = TestEnv::lexical_only();

    std::fs::write(
        env.workspace_root.join("docker-compose.yml"),
        "version: '3'\nservices:\n  api:\n    image: nginx:latest\n",
    )
    .unwrap();

    let report1 =
        kebab_app::ingest_with_config(env.config.clone(), env.scope(), false)
            .expect("first ingest");
    let item1 = report1
        .items
        .as_ref()
        .expect("items present")
        .iter()
        .find(|i| i.doc_path.0.ends_with("docker-compose.yml"))
        .expect("docker-compose.yml in first report");
    assert!(
        matches!(item1.kind, IngestItemKind::New),
        "first ingest must be New, got {:?}", item1.kind
    );
    assert_eq!(
        item1.chunker_version.as_ref().map(|c| c.0.as_str()),
        Some("code-text-paragraph-v1"),
        "first ingest must use Tier 3 fallback chunker"
    );

    let report2 =
        kebab_app::ingest_with_config(env.config.clone(), env.scope(), false)
            .expect("second ingest");
    let item2 = report2
        .items
        .as_ref()
        .expect("items present")
        .iter()
        .find(|i| i.doc_path.0.ends_with("docker-compose.yml"))
        .expect("docker-compose.yml in second report");
    assert!(
        matches!(item2.kind, IngestItemKind::Unchanged),
        "second ingest must be Unchanged, got {:?}", item2.kind
    );
}

/// p10-3 fix regression: a shell file (direct Tier 3, not a fallback)
/// must also report Unchanged on re-ingest. Shell goes straight to
/// CodeTextParagraphV1Chunker so `stored_is_tier3_fallback` is false
/// (parser_version is "none-v1" and chunker matches the current dispatch),
/// but the normal equality path should pass regardless.
#[test]
fn tier3_shell_reingest_is_unchanged() {
    let env = TestEnv::lexical_only();

    std::fs::write(
        env.workspace_root.join("deploy.sh"),
        "#!/usr/bin/env bash\nset -e\necho hello\n",
    )
    .unwrap();

    let report1 =
        kebab_app::ingest_with_config(env.config.clone(), env.scope(), false)
            .expect("first ingest");
    let item1 = report1
        .items
        .as_ref()
        .expect("items present")
        .iter()
        .find(|i| i.doc_path.0.ends_with("deploy.sh"))
        .expect("deploy.sh in first report");
    assert!(
        matches!(item1.kind, IngestItemKind::New),
        "first ingest must be New, got {:?}", item1.kind
    );

    let report2 =
        kebab_app::ingest_with_config(env.config.clone(), env.scope(), false)
            .expect("second ingest");
    let item2 = report2
        .items
        .as_ref()
        .expect("items present")
        .iter()
        .find(|i| i.doc_path.0.ends_with("deploy.sh"))
        .expect("deploy.sh in second report");
    assert!(
        matches!(item2.kind, IngestItemKind::Unchanged),
        "shell reingest must be Unchanged, got {:?}", item2.kind
    );
}
