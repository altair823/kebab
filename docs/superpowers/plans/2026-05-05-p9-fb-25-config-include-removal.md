# p9-fb-25 — Config `workspace.include` 제거 + 지원 형식 가시성 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove the dead `WorkspaceCfg.include` config field, surface skip reasons (unsupported media type) per file via `IngestItem.warnings`, and aggregate them into `IngestReport.skipped_by_extension` with CLI / TUI / README / `kebab init` template all calling out the four supported extensions (`.md`, `.png`, `.jpg/.jpeg`, `.pdf`).

**Architecture:** Two coordinated streams. (1) Config: drop `WorkspaceCfg.include` + emit a one-shot `tracing::warn!` when an old config still has it (raw TOML key probe). (2) Ingest pipeline: every Skipped per-asset return gains `warnings: vec!["unsupported media type: .ext"]` (or `"kb:// URI not yet supported"`); the asset loop bumps a new `aggregate.skipped_by_extension: BTreeMap<String, u32>` keyed by lowercase ext (`<no-ext>` sentinel for files without one). CLI summary + TUI status_line render the breakdown desc-sorted on terminal events.

**Tech Stack:** Rust 2024, serde, toml 0.8, tracing. `BTreeMap` for stable JSON key order. No new deps. No SQLite migration.

**Spec:** `docs/superpowers/specs/2026-05-05-p9-fb-25-config-include-removal-design.md`

---

## File Structure

**Modified:**
- `crates/kebab-config/src/lib.rs` — drop `WorkspaceCfg.include` field + update `WorkspaceCfg::defaults()` + add `from_file` deprecation probe.
- `crates/kebab-app/src/lib.rs` — `init_workspace` header comment lists supported extensions + (no code change needed beyond default config no longer carrying include); ingest pipeline's three Skipped emit sites populate `warnings` + bump `skipped_by_extension`.
- `crates/kebab-app/src/ingest_progress.rs` — `AggregateCounts.skipped_by_extension: BTreeMap<String, u32>`.
- `crates/kebab-core/src/ingest.rs` — `IngestReport.skipped_by_extension: BTreeMap<String, u32>`.
- `crates/kebab-cli/src/main.rs` — drop `include: cfg.workspace.include.clone()` from SourceScope construction; render breakdown in summary print.
- `crates/kebab-tui/src/ingest_progress.rs` — drop `include: cfg.workspace.include.clone()` from SourceScope construction; render breakdown in `status_line` final / aborted.
- `docs/wire-schema/v1/ingest_report.schema.json` — additive `skipped_by_extension` (object, additionalProperties integer ≥ 0).
- `README.md` — `kebab tui` / `kebab ingest` cell appends supported-extension list + skip-reason mention.
- `HANDOFF.md`, `tasks/HOTFIXES.md`, `tasks/INDEX.md`, `tasks/p9/p9-fb-25-config-include-removal.md`.

`SourceScope` (in `kebab-core/src/traits.rs`) keeps its `include: Vec<String>` field — it's a design-level abstraction (§7.1) that connectors / routers may use later. Removing it is a separate spec.

---

### Task 1: Drop `WorkspaceCfg.include` + add deprecation probe

**Files:**
- Modify: `crates/kebab-config/src/lib.rs` (struct + defaults + from_file)

This task removes the field but keeps backward-compat: an old `config.toml` with `include = [...]` still loads (serde ignores unknown keys without `deny_unknown_fields`). On detection, emit a one-shot `tracing::warn!`.

- [ ] **Step 1: Write the failing tests**

Append to the existing `#[cfg(test)] mod tests` block in `crates/kebab-config/src/lib.rs` (find via `grep -n "fn defaults_are_serde_roundtrip_stable" crates/kebab-config/src/lib.rs`):

```rust
    /// p9-fb-25: legacy config with `workspace.include = [...]` must
    /// still deserialize cleanly (silent unknown-field acceptance).
    #[test]
    fn legacy_include_field_is_ignored_silently() {
        let toml_text = r#"
schema_version = 1

[workspace]
root = "/tmp/kebab-legacy"
include = ["**/*.md", "**/*.txt"]
exclude = [".git/**"]

[storage]
data_dir = "/tmp/kebab-data"
sqlite = "{data_dir}/kebab.sqlite"
vector_dir = "{data_dir}/lancedb"
asset_dir = "{data_dir}/assets"
artifact_dir = "{data_dir}/artifacts"
model_dir = "{data_dir}/models"
runs_dir = "{data_dir}/runs"
copy_threshold_mb = 100

[indexing]
max_parallel_extractors = 2
max_parallel_embeddings = 1
watch_filesystem = false
"#;
        // NOTE: a real legacy config has many more sections (chunking,
        // models, etc.). For this test we rely on `#[serde(default)]`
        // on each top-level Config field — if any field is missing
        // a serde default at this point, we accept that as a separate
        // bug and adjust below. The point of THIS test is the
        // workspace.include field tolerance.
        let parsed: Result<Config, _> = toml::from_str(toml_text);
        assert!(parsed.is_ok(), "legacy include must not break load: {:?}", parsed.err());
        let cfg = parsed.unwrap();
        assert_eq!(cfg.workspace.root, "/tmp/kebab-legacy");
        assert_eq!(cfg.workspace.exclude, vec![".git/**".to_string()]);
    }

    /// p9-fb-25: `WorkspaceCfg::defaults()` no longer carries `include`.
    #[test]
    fn workspace_defaults_have_no_include_field() {
        let ws = WorkspaceCfg::defaults();
        // We're not asserting include is absent (Rust struct field
        // doesn't have such a query). We assert the default has the
        // expected exclude shape and the struct definition reflects
        // the removal — this test will fail to compile if a stray
        // reference to ws.include lingers.
        assert!(!ws.exclude.is_empty(), "default exclude should retain `.git/**` etc.");
    }
```

If `Config` doesn't have `#[serde(default)]` at the section level (chunking / models / etc.), the legacy-config test will fail because the abbreviated TOML omits required sections. In that case, expand the legacy TOML in the test to include all required sections — the goal is to verify `include` is ignored, not to test default fallback. Use `Config::defaults()` and serialize → modify → deserialize as a shortcut:

```rust
    #[test]
    fn legacy_include_field_is_ignored_silently() {
        let mut cfg = Config::defaults();
        cfg.workspace.root = "/tmp/kebab-legacy".to_string();
        let mut toml_text = toml::to_string(&cfg).expect("default round-trips");
        // Inject a legacy `include = [...]` line into the [workspace] block.
        toml_text = toml_text.replace(
            "[workspace]",
            "[workspace]\ninclude = [\"**/*.md\", \"**/*.txt\"]",
        );
        let parsed: Result<Config, _> = toml::from_str(&toml_text);
        assert!(parsed.is_ok(), "legacy include must not break load: {:?}", parsed.err());
        let cfg = parsed.unwrap();
        assert_eq!(cfg.workspace.root, "/tmp/kebab-legacy");
    }
```

Run: `cargo test -p kebab-config --lib legacy_include_field_is_ignored_silently`
Expected: FAIL — current `WorkspaceCfg` still has `include: Vec<String>` so deserialize SUCCEEDS but `workspace_defaults_have_no_include_field` test compiles and passes; the first test passes too because serde default for `Vec<String>` is empty. Wait — the FAIL precondition is wrong. Both tests pass against current code.

Reframe: write a **forward-looking** test that asserts the field is gone:

```rust
    /// p9-fb-25: `WorkspaceCfg` must NOT have an `include` field.
    /// Compile-time proof: this test references every field of
    /// `WorkspaceCfg` exhaustively. If a future commit re-introduces
    /// `include`, the destructure here breaks (refactor failure).
    #[test]
    fn workspace_cfg_has_only_root_and_exclude_fields() {
        let ws = WorkspaceCfg::defaults();
        // Exhaustive destructure — adding a new field would break
        // this on the next compile.
        let WorkspaceCfg { root: _, exclude: _ } = &ws;
    }
```

This test will NOT compile against current code (because `WorkspaceCfg` still has `include`). The compile error IS the test failure.

Run: `cargo build -p kebab-config --tests`
Expected: error[E0027] missing structure fields OR error mentioning `include`. **This is the failing test.**

- [ ] **Step 2: Drop the field + update defaults**

Open `crates/kebab-config/src/lib.rs`. Find `pub struct WorkspaceCfg` (around line 51). Remove the `pub include: Vec<String>` line:

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceCfg {
    pub root: String,
    pub exclude: Vec<String>,
}
```

Find `WorkspaceCfg::defaults()` or the `Config::defaults()` body that constructs `WorkspaceCfg { root, include, exclude }` (around line 252). Drop the `include: vec!["**/*.md".to_string()],` line:

```rust
            workspace: WorkspaceCfg {
                root: "~/KnowledgeBase".to_string(),
                exclude: vec![
                    ".git/**".to_string(),
                    "node_modules/**".to_string(),
                    ".obsidian/**".to_string(),
                ],
            },
```

- [ ] **Step 3: Add deprecation probe in `from_file`**

In the same file, find `pub fn from_file` (around line 397). Replace the body to probe for the legacy `include` key BEFORE typed deserialize:

```rust
    pub fn from_file(path: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)?;

        // p9-fb-25: probe for the legacy `workspace.include` key — if
        // present, emit a one-shot deprecation warning. Detection uses
        // raw `toml::Value` lookup; the warning is fired via a
        // process-level `OnceLock` so a long-running TUI / CLI run
        // doesn't spam the log on every Config::load.
        if let Ok(value) = toml::from_str::<toml::Value>(&text) {
            if value
                .get("workspace")
                .and_then(|v| v.get("include"))
                .is_some()
            {
                static DEPRECATION_FIRED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
                DEPRECATION_FIRED.get_or_init(|| {
                    tracing::warn!(
                        target: "kebab-config",
                        config = %path.display(),
                        "deprecated config: `workspace.include` 필드는 더 이상 사용되지 않습니다 (p9-fb-25). 처리 가능한 형식 (md / png / jpg / pdf) 은 extractor 가 자동 결정. 다음 버전부터 config 갱신 권장."
                    );
                });
            }
        }

        let mut cfg: Self = toml::from_str(&text)?;
        cfg.source_dir = path.parent().map(Path::to_path_buf);
        Ok(cfg)
    }
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p kebab-config --lib`
Expected: all pass. The forward-looking exhaustive-destructure test now compiles + asserts the struct shape.

- [ ] **Step 5: Build the workspace + surface compile errors**

Run: `cargo build --workspace`
Expected: compile errors at every site that constructs `WorkspaceCfg { ..., include: ..., ... }` or reads `cfg.workspace.include`. The known sites:
- `crates/kebab-cli/src/main.rs:329` — drop `include: cfg.workspace.include.clone(),` (Task 2 will add the proper SourceScope construction).
- `crates/kebab-tui/src/ingest_progress.rs:39` — same.
- Test fixtures inside `kebab-config` if any — drop the `include: vec![...]` literal.

For Task 1's commit, fix ONLY the kebab-config sites (the test passes fix). Other crates' compile errors roll into Task 2.

For now, in `crates/kebab-cli/src/main.rs` line 329 and `crates/kebab-tui/src/ingest_progress.rs` line 39, replace `include: cfg.workspace.include.clone()` with `include: Vec::new()` (Task 2 will use `..Default::default()` once we touch the structures more carefully).

- [ ] **Step 6: clippy + commit**

Run: `cargo clippy -p kebab-config --all-targets -- -D warnings`
Expected: clean.

```bash
git add crates/kebab-config/src/lib.rs crates/kebab-cli/src/main.rs crates/kebab-tui/src/ingest_progress.rs
git commit -m "feat(kebab-config): p9-fb-25 task 1 — drop WorkspaceCfg.include + deprecation probe

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: Switch SourceScope construction to `..Default::default()` for cleaner removal

**Files:**
- Modify: `crates/kebab-cli/src/main.rs` (line 327)
- Modify: `crates/kebab-tui/src/ingest_progress.rs` (line 38)

Task 1's quick fix (`include: Vec::new()`) is functional but ugly. This task replaces those with the idiomatic `..Default::default()` pattern.

- [ ] **Step 1: Update CLI ingest dispatch**

Open `crates/kebab-cli/src/main.rs`. Find the `Cmd::Ingest { ... }` arm (around line 321). Replace the SourceScope literal:

```rust
            let scope = kebab_core::SourceScope {
                root: root.clone().unwrap_or_else(|| PathBuf::from(&cfg.workspace.root)),
                exclude: cfg.workspace.exclude.clone(),
                ..Default::default()
            };
```

(SourceScope derives `Default` per `kebab-core/src/traits.rs`; `..Default::default()` fills `include: Vec::new()` for now. If `include` is removed from `SourceScope` in a future spec, this site needs no change.)

- [ ] **Step 2: Update TUI ingest dispatch**

Open `crates/kebab-tui/src/ingest_progress.rs`. Find the SourceScope construction (around line 38):

```rust
        scope: kebab_core::SourceScope {
            root: PathBuf::from(&cfg.workspace.root),
            exclude: cfg.workspace.exclude.clone(),
            ..Default::default()
        },
```

- [ ] **Step 3: Build + test**

Run: `cargo build --workspace`
Run: `cargo test -p kebab-cli -p kebab-tui --lib`
Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: all clean.

- [ ] **Step 4: Commit**

```bash
git add crates/kebab-cli/src/main.rs crates/kebab-tui/src/ingest_progress.rs
git commit -m "refactor(kebab-cli, kebab-tui): p9-fb-25 task 2 — SourceScope via ..Default::default()

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: `init_workspace` header — supported extensions

**Files:**
- Modify: `crates/kebab-app/src/lib.rs` (`init_workspace`, around line 138)

- [ ] **Step 1: Update the header comment**

Open `crates/kebab-app/src/lib.rs`. Find `let header = "\` inside `init_workspace` (around line 138). Replace the entire `header` string with a version that adds the supported-extensions block AND removes any reference to `include`:

```rust
        let header = "\
# kebab config — `~/.config/kebab/config.toml`.
#
# `workspace.root` accepts:
#   • absolute paths       (`/home/me/KnowledgeBase`)
#   • tilde                (`~/KnowledgeBase`)         ← default
#   • env vars             (`${XDG_DATA_HOME}/kebab`)
#   • relative paths       (`./notes`, `notes`, `../shared/x`)
#     — relative paths resolve against the directory of THIS
#       config file, NOT the user's `cwd` at invocation time.
#
# 처리 가능한 형식 (extractor 가 자동 결정 — config 에 명시할 수 없음):
#   • Markdown: .md
#   • 이미지:   .png .jpg .jpeg  (OCR + caption)
#   • PDF:      .pdf
# 다른 확장자는 ingest 시 자동 skip + warning. 처리 대상 폴더의
# 일부만 ingest 하고 싶으면 `kebab ingest <path>` 로 root 명시
# 또는 `.kebabignore` 파일 / 본 `workspace.exclude` 로 denylist.
#
# Override individual keys at runtime with `KEBAB_*` env vars
# (e.g. `KEBAB_WORKSPACE_ROOT=/tmp/test kebab ingest`).
\n";
```

- [ ] **Step 2: Smoke test by running `kebab init` against a temp config**

Add (or extend) a test in `crates/kebab-app/tests/`:

```rust
#[test]
fn init_workspace_header_lists_supported_extensions() {
    let tmp = tempfile::tempdir().unwrap();
    // SAFETY: each test sets KEBAB env vars in a process-wide manner;
    // this test relies on init_workspace writing relative to the
    // current XDG_CONFIG_HOME. We override XDG_CONFIG_HOME to the
    // tmp dir so the produced config sits inside `tmp`.
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", tmp.path());
    }
    kebab_app::init_workspace(true).expect("init_workspace");
    let cfg_path = kebab_config::Config::xdg_config_path();
    let body = std::fs::read_to_string(&cfg_path).unwrap();
    assert!(body.contains("처리 가능한 형식"), "header lists supported types");
    assert!(body.contains("Markdown: .md"), "md listed");
    assert!(body.contains(".png .jpg .jpeg"), "image extensions listed");
    assert!(body.contains("PDF:      .pdf"), "pdf listed");
    assert!(!body.contains("workspace.include"), "no leftover include reference");
}
```

If the existing kebab-app test infra already has an `init_workspace` test, extend it. Otherwise create a new file `crates/kebab-app/tests/init_template.rs`.

`unsafe`: Rust 2024 + recent toolchain may flag `set_var` as unsafe. Wrap in `unsafe { ... }` block per current Rust semantics.

- [ ] **Step 3: Run + commit**

```bash
cargo test -p kebab-app --test init_template  # or whatever filename
cargo clippy --workspace --all-targets -- -D warnings
git add -u
git commit -m "feat(kebab-app): p9-fb-25 task 3 — init_workspace header lists supported extensions

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: Add `skipped_by_extension` to `IngestReport` + `AggregateCounts` + wire schema

**Files:**
- Modify: `crates/kebab-core/src/ingest.rs`
- Modify: `crates/kebab-app/src/ingest_progress.rs`
- Modify: `docs/wire-schema/v1/ingest_report.schema.json`

- [ ] **Step 1: Add field to `IngestReport`**

Open `crates/kebab-core/src/ingest.rs`. Replace the `IngestReport` struct (around lines 10-22) by adding the new field:

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IngestReport {
    pub scope: SourceScope,
    pub scanned: u32,
    pub new: u32,
    pub updated: u32,
    pub skipped: u32,
    pub unchanged: u32,
    pub errors: u32,
    pub duration_ms: u32,
    /// p9-fb-25: per-extension skip count. Key = lowercase extension
    /// without leading dot (e.g. "docx", "txt"); files without an
    /// extension key under "<no-ext>". `BTreeMap` so the wire JSON
    /// has stable key order across runs.
    pub skipped_by_extension: std::collections::BTreeMap<String, u32>,
    pub items: Option<Vec<IngestItem>>,
}
```

The import for `BTreeMap` lives inside the field annotation via the full path; don't add a `use` at the top of the file.

- [ ] **Step 2: Add field to `AggregateCounts`**

Open `crates/kebab-app/src/ingest_progress.rs`. Replace the struct (around lines 25-37):

```rust
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AggregateCounts {
    pub scanned: u32,
    pub new: u32,
    pub updated: u32,
    pub skipped: u32,
    pub unchanged: u32,
    pub errors: u32,
    pub chunks_indexed: u32,
    pub embeddings_indexed: u32,
    /// p9-fb-25: per-extension skip count. See [`IngestReport.skipped_by_extension`].
    pub skipped_by_extension: std::collections::BTreeMap<String, u32>,
}
```

Note: removed `Copy` from the derive. `BTreeMap` is not `Copy`. Replaces `Copy + Eq + PartialEq` with `Eq + PartialEq` only. `Copy`-using callers (if any) need to switch to `clone()`.

Run: `cargo build -p kebab-app`. Look for `error: ... requires Copy` errors. Fix each by `.clone()`.

- [ ] **Step 3: Update wire schema**

Open `docs/wire-schema/v1/ingest_report.schema.json`. Inside `properties`, add:

```json
        "skipped_by_extension": {
          "type": "object",
          "additionalProperties": {
            "type": "integer",
            "minimum": 0
          },
          "description": "p9-fb-25: per-extension skip count. Key = lowercase extension without leading dot (e.g. 'docx'). Files without extension key under '<no-ext>'."
        },
```

If `required` array exists, add `skipped_by_extension` to it (always present, even if empty `{}`).

- [ ] **Step 4: Build + fix construction sites**

Run: `cargo build --workspace`. The compiler will surface every `IngestReport { ... }` and `AggregateCounts { ... }` literal that omits the new field. Add `skipped_by_extension: BTreeMap::new()` (or `Default::default()`) at each.

Add `use std::collections::BTreeMap;` at the top of test fixture files where the literal is constructed (so the addition is concise — `BTreeMap::new()` rather than `std::collections::BTreeMap::new()`).

Snapshot fixtures (`crates/kebab-store-sqlite/snapshots/ingest_report.snapshot.json`) — add `"skipped_by_extension": {}` between two existing fields (alphabetic / logical order — between `skipped` and `errors` to match struct declaration order).

- [ ] **Step 5: Run + commit**

```bash
cargo test --workspace --no-fail-fast -j 1 2>&1 | grep "^test result:"
cargo clippy --workspace --all-targets -- -D warnings
git add -u
git commit -m "feat(kebab-core, kebab-app): p9-fb-25 task 4 — IngestReport.skipped_by_extension + wire schema additive

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

### Task 5: Populate `IngestItem.warnings` for Skipped paths + bump `skipped_by_extension`

**Files:**
- Modify: `crates/kebab-app/src/lib.rs` (three Skipped emit sites + one asset loop)

- [ ] **Step 1: Write the failing test**

Create `crates/kebab-app/tests/skip_reason.rs`:

```rust
//! p9-fb-25: skipped per-asset items must carry a human-readable reason
//! in `warnings`, and the report's `skipped_by_extension` must aggregate
//! by lowercase extension.

mod common;

use common::TestEnv;

#[test]
fn unsupported_extension_skip_carries_warning_and_is_aggregated() {
    let env = TestEnv::lexical_only();
    // Workspace already populated by TestEnv; add a `.docx` and a
    // file with no extension to trigger Skipped paths.
    let workspace_root = std::path::PathBuf::from(&env.config.workspace.root);
    std::fs::write(workspace_root.join("legacy.docx"), b"unsupported").unwrap();
    std::fs::write(workspace_root.join("Makefile"), b"unsupported").unwrap();

    let report = kebab_app::ingest_with_config(
        env.config.clone(),
        env.scope(),
        false,
    ).unwrap();

    let items = report.items.as_ref().expect("items array populated");
    let docx_item = items.iter().find(|i| i.doc_path.0.ends_with("legacy.docx")).unwrap();
    assert_eq!(docx_item.kind, kebab_core::IngestItemKind::Skipped);
    assert_eq!(
        docx_item.warnings,
        vec!["unsupported media type: .docx".to_string()],
    );
    let makefile_item = items.iter().find(|i| i.doc_path.0.ends_with("Makefile")).unwrap();
    assert_eq!(makefile_item.kind, kebab_core::IngestItemKind::Skipped);
    assert_eq!(
        makefile_item.warnings,
        vec!["unsupported media type: <no-ext>".to_string()],
    );
    assert_eq!(report.skipped_by_extension.get("docx").copied(), Some(1));
    assert_eq!(report.skipped_by_extension.get("<no-ext>").copied(), Some(1));
}
```

If `TestEnv::lexical_only()` populates a workspace with markdown fixtures already, the docx + Makefile additions are extra. If not, build a fresh `TempDir` workspace following the pattern other kebab-app tests use.

Run: `cargo test -p kebab-app --test skip_reason`
Expected: FAIL — current `warnings: Vec::new()` for Skipped + `skipped_by_extension` is always empty.

- [ ] **Step 2: Update the three `IngestItemKind::Skipped` emit sites**

Open `crates/kebab-app/src/lib.rs`. Three sites currently return `IngestItem { kind: Skipped, ..., warnings: Vec::new(), ... }`:

- One at the top of `ingest_one_asset` (the `_ =>` fallback when MediaType doesn't match).
- One when `SourceUri::Kb` (kb:// URI not yet supported).
- One in `ingest_one_image_asset` when SourceUri is kb://.
- One in `ingest_one_pdf_asset` when SourceUri is kb://.

For each Skipped emit, replace `warnings: Vec::new()` with the appropriate reason.

For the **media-type fallback** at the top of `ingest_one_asset`: extract the extension from `asset.workspace_path.0` (lowercase, no dot, `<no-ext>` sentinel) and emit:

```rust
let ext = ext_for_skip_warning(&asset.workspace_path.0);
return Ok(kebab_core::IngestItem {
    kind: kebab_core::IngestItemKind::Skipped,
    doc_id: None,
    doc_path: asset.workspace_path.clone(),
    asset_id: Some(asset.asset_id.clone()),
    byte_len: Some(asset.byte_len),
    block_count: None,
    chunk_count: None,
    parser_version: None,
    chunker_version: None,
    warnings: vec![format!("unsupported media type: .{ext}")],
    error: None,
});
```

For the **kb:// URI** sites:

```rust
warnings: vec!["kb:// URI not yet supported".to_string()],
```

Add the helper near the other per-asset helpers:

```rust
/// p9-fb-25: extract the lowercase extension (no leading dot) from a
/// workspace path for use in the `unsupported media type: .X`
/// warning + `IngestReport.skipped_by_extension` key. Returns
/// `"<no-ext>"` for paths with no extension. Always lowercase so
/// `Foo.DOCX` and `bar.docx` aggregate under the same key.
fn ext_for_skip_warning(path: &str) -> String {
    std::path::Path::new(path)
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_else(|| "<no-ext>".to_string())
}
```

Note: for the `<no-ext>` case the warning should read `"unsupported media type: <no-ext>"` (no leading dot — sentinel). Adjust the format! call:

```rust
let ext = ext_for_skip_warning(&asset.workspace_path.0);
let warning = if ext == "<no-ext>" {
    "unsupported media type: <no-ext>".to_string()
} else {
    format!("unsupported media type: .{ext}")
};
warnings: vec![warning],
```

- [ ] **Step 3: Bump `aggregate.skipped_by_extension` in the asset loop**

Find the asset loop in `ingest_with_config_opts` (search for `IngestItemKind::Skipped =>` arms in the per-result match). Where the loop currently does `aggregate.skipped += 1`, also bump the per-extension counter using the same `ext_for_skip_warning` helper:

```rust
IngestItemKind::Skipped => {
    aggregate.skipped += 1;
    let ext = ext_for_skip_warning(&item.doc_path.0);
    *aggregate.skipped_by_extension.entry(ext).or_insert(0) += 1;
}
```

After the loop, when building the final `IngestReport`, populate `skipped_by_extension: aggregate.skipped_by_extension.clone()`.

- [ ] **Step 4: Run the test**

Run: `cargo test -p kebab-app --test skip_reason`
Expected: PASS.

- [ ] **Step 5: Run the full kebab-app suite for regressions**

Run: `cargo test -p kebab-app`
Expected: existing tests pass. The change is additive — Skipped items previously had empty `warnings` and `skipped_by_extension` was 0. Tests that asserted `warnings.is_empty()` may need updating (search):

```bash
grep -rn 'warnings.is_empty\|warnings, vec!\[\]\|warnings == Vec::new' crates/kebab-app/tests/
```

Update any failing test to either skip the warnings assertion (if not the focus) or assert the new content.

- [ ] **Step 6: clippy + commit**

```bash
cargo clippy --workspace --all-targets -- -D warnings
git add -u
git commit -m "feat(kebab-app): p9-fb-25 task 5 — Skipped warnings + skipped_by_extension aggregation

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

### Task 6: CLI summary + TUI status_line render breakdown

**Files:**
- Modify: `crates/kebab-cli/src/main.rs` (ingest summary print)
- Modify: `crates/kebab-tui/src/ingest_progress.rs` (status_line)

- [ ] **Step 1: Update CLI summary**

Open `crates/kebab-cli/src/main.rs`. Find the human-mode summary print (search for `"new" =>` or similar around the ingest-finished branch). The current format is `"... {N} skipped, ..."`. Replace with:

```rust
let skipped_breakdown = if report.skipped_by_extension.is_empty() {
    String::new()
} else {
    let mut entries: Vec<_> = report.skipped_by_extension.iter().collect();
    // desc by count, ties broken by key for stable output.
    entries.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
    let parts: Vec<String> = entries.iter().map(|(k, v)| format!("{v} {k}")).collect();
    format!(": {}", parts.join(", "))
};
println!(
    "✓ ingest: {} docs ({} new, {} updated, {} unchanged, {} skipped{}), {} chunks indexed in {}s",
    report.scanned,
    report.new,
    report.updated,
    report.unchanged,
    report.skipped,
    skipped_breakdown,
    /* chunks_indexed: derive from items or store in IngestReport */ 0,  // adapt to actual
    report.duration_ms / 1000,
);
```

Adapt to the actual print site — the existing print may use `report.duration_ms` directly or have a `chunks_indexed` already plumbed. Match the existing surrounding pattern.

- [ ] **Step 2: Update TUI status_line**

Open `crates/kebab-tui/src/ingest_progress.rs`. Find the success branch in `pub fn status_line` (around line 170+). Apply the same breakdown logic:

```rust
let skipped_breakdown = if state.counts.skipped_by_extension.is_empty() {
    String::new()
} else {
    let mut entries: Vec<_> = state.counts.skipped_by_extension.iter().collect();
    entries.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
    let parts: Vec<String> = entries.iter().map(|(k, v)| format!("{v} {k}")).collect();
    format!(": {}", parts.join(", "))
};
return format!(
    "✓ ingest: {} docs ({} new, {} updated, {} unchanged, {} skipped{}), {} chunks indexed in {}s",
    state.counts.scanned,
    state.counts.new,
    state.counts.updated,
    state.counts.unchanged,
    state.counts.skipped,
    skipped_breakdown,
    state.counts.chunks_indexed,
    secs,
);
```

Apply the same to the aborted branch:

```rust
return format!(
    "✗ ingest aborted at {}/{} after {}s (new={} updated={} unchanged={} skipped={}{} errors={})",
    state.counts.scanned.saturating_sub(state.counts.errors),
    state.counts.scanned,
    secs,
    state.counts.new,
    state.counts.updated,
    state.counts.unchanged,
    state.counts.skipped,
    skipped_breakdown,
    state.counts.errors,
);
```

In-flight branch unchanged.

- [ ] **Step 3: Update existing status_line tests**

Find `#[cfg(test)] mod tests` in `crates/kebab-tui/src/ingest_progress.rs`. Existing tests construct `AggregateCounts` literals. After Task 4 they already have `skipped_by_extension: BTreeMap::new()`. For tests that exercise the breakdown, build an `AggregateCounts` with a populated map:

```rust
#[test]
fn status_line_includes_skipped_breakdown() {
    use std::collections::BTreeMap;
    let mut counts = AggregateCounts::default();
    counts.scanned = 10;
    counts.skipped = 3;
    counts.skipped_by_extension.insert("docx".into(), 2);
    counts.skipped_by_extension.insert("txt".into(), 1);
    let state = IngestState { /* fill mandatory fields */ };
    state.counts = counts;
    state.terminal_at = Some(std::time::Instant::now());  // make `if state.terminal_at.is_some()` true
    let line = status_line(&state);
    assert!(line.contains("3 skipped: 2 docx, 1 txt"), "got: {line}");
}
```

Adapt to the actual `IngestState` struct fields.

- [ ] **Step 4: Run + commit**

```bash
cargo test -p kebab-cli -p kebab-tui --lib
cargo clippy --workspace --all-targets -- -D warnings
git add -u
git commit -m "feat(kebab-cli, kebab-tui): p9-fb-25 task 6 — render skipped-by-extension breakdown

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

### Task 7: Docs sync

**Files:**
- Modify: `README.md`, `HANDOFF.md`, `tasks/HOTFIXES.md`, `tasks/INDEX.md`
- Create: `tasks/p9/p9-fb-25-config-include-removal.md`

- [ ] **Step 1: README**

Open `README.md`. Find the `kebab ingest` row (or paragraph if it's not a row). Append:

```
**지원 형식** (extractor 자동 결정 — config 에 명시 불가): Markdown (`.md`), 이미지 (`.png` / `.jpg` / `.jpeg`, OCR + caption), PDF (`.pdf`). 다른 확장자는 자동 skip — `IngestItem.warnings` 에 사유 (`"unsupported media type: .docx"` 등), `IngestReport.skipped_by_extension` 에 카운트 분류, CLI / TUI summary 에 breakdown 표시.
```

If the `kebab tui` row already mentions some of this, integrate into existing text instead of duplicating.

- [ ] **Step 2: HANDOFF.md**

Add a new entry directly above the most recent `p9-fb-24` row (or wherever the dated list begins):

```
- **2026-05-05 P9 post-도그푸딩 (p9-fb-25)** — Config 의 `workspace.include` 필드 제거 + 지원 형식 가시성. 사용자 도그푸딩 피드백: include + exclude 동시 존재가 case 4 (둘 다 매치 안 함) 의미 모호 + 어차피 처리 가능 형식 (md / png / jpg / pdf) 이 정해져 있으니 명시 필요. `WorkspaceCfg.include` 제거 (옛 config 의 `include = [...]` 은 silently 무시 + 단발 deprecation warning). `IngestItem.warnings` 가 Skipped 시 사유 (`"unsupported media type: .docx"` 등) 채움. `IngestReport.skipped_by_extension: BTreeMap<String, u32>` 신규 (additive wire — release 트리거 안 됨). CLI / TUI summary 에 breakdown 표시 (`"5 skipped: 3 docx, 1 txt, 1 epub"`). README + `kebab init` 헤더 주석에 지원 형식 명시. spec: `tasks/p9/p9-fb-25-config-include-removal.md`. HOTFIXES `2026-05-05 — p9-fb-25` 가 source of truth.
```

- [ ] **Step 3: HOTFIXES.md**

Open `tasks/HOTFIXES.md`. Add new section above `## 2026-05-04 — p9-fb-24`:

```markdown
## 2026-05-05 — p9-fb-25 (post-dogfooding): config workspace.include 제거 + 지원 형식 가시성

**Source feedback**: 사용자 도그푸딩 2026-05-05 — config 의 `workspace.include` + `workspace.exclude` 동시 존재가 case 4 (둘 다 매치 안 함) 의미 모호 + 어차피 처리 가능 형식 (md / png / jpg / pdf) 이 정해져 있으니 사용자에게 명시 필요.

**Live binding 변경**:

- `kebab-config::WorkspaceCfg.include: Vec<String>` 제거. denylist-only 모델. 옛 config 의 `include = [...]` 은 serde 가 silently 무시 + `Config::from_file` 가 단발 `tracing::warn!` 으로 deprecation 안내 (`std::sync::OnceLock` — 같은 process 안에서 한 번만).
- `kebab-core::IngestItem.warnings` 가 Skipped 시 사유 채움: `"unsupported media type: .{ext}"` (ext 없으면 `"unsupported media type: <no-ext>"`) / `"kb:// URI not yet supported"`.
- `kebab-core::IngestReport.skipped_by_extension: BTreeMap<String, u32>` + `kebab-app::AggregateCounts.skipped_by_extension` 신규. key = lowercase ext (`docx`, `txt`), no-ext sentinel = `<no-ext>`. wire schema `ingest_report.v1` 에 additive 추가 (v1 호환 유지 — release 트리거 안 됨 per CLAUDE.md release 규약).
- CLI summary + TUI status_line final / aborted: `5 skipped: 3 docx, 1 txt, 1 epub` 형식. desc 정렬 + 모두 표시.
- `kebab-app::init_workspace` 헤더 주석에 지원 형식 명시 (Markdown / 이미지 / PDF + 각 확장자).
- README `kebab ingest` 설명에 지원 형식 + skip 사유 + breakdown 표시 명시.

**Spec contract impact**: design §6.2 의 `workspace.include` 항목 invalidate (frozen 그대로 두고 본 항목 + spec `tasks/p9/p9-fb-25-config-include-removal.md` 가 source of truth). design §3.x `IngestReport` + §2.4a `IngestEvent` 에 새 필드 / 새 warning 의미 추가 (additive).

**Tests added**: 약 6 신규 (kebab-config 단위 2: legacy include 무시 + WorkspaceCfg 필드 destructure / kebab-app 통합 1: skip_reason / kebab-tui 단위 1: breakdown 라인 / kebab-app 단위 1: init template 헤더 / kebab-app 단위 1: ext_for_skip_warning helper). 기존 723 워크스페이스 테스트 무수정 통과.

**Known limitation (deferred)**:

- `SourceScope.include` (`kebab-core::traits`) 는 그대로 — design §7.1 abstraction 이라 별 spec 으로 다룰 수 있음. 본 PR 은 config 단의 `WorkspaceCfg.include` 만 정리.
- 새 extractor (txt / docx / epub 등) 도입은 별 spec.
- `kebab doctor` 가 unsupported 파일 카운트 분석은 후속 task.
```

- [ ] **Step 4: INDEX.md**

Open `tasks/INDEX.md`. Append to the p9-fb section:

```
  - [p9-fb-25 config workspace.include 제거 + 지원 형식 가시성 (post-도그푸딩)](p9/p9-fb-25-config-include-removal.md)
```

- [ ] **Step 5: Per-task spec file**

Create `tasks/p9/p9-fb-25-config-include-removal.md`:

```markdown
---
phase: P9
component: kebab-config
task_id: p9-fb-25
title: "Config workspace.include 제거 + 지원 형식 가시성 (post-merge dogfooding)"
status: completed
depends_on: [p9-fb-23]
unblocks: []
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§6.2 Workspace, §3.x IngestReport, §2.4a IngestEvent]
source_feedback: 사용자 도그푸딩 2026-05-05 — include + exclude 의미 모호 + 지원 형식 가시성 부족.
---

# p9-fb-25 — Config `workspace.include` 제거 + 지원 형식 가시성

상세 설계: `docs/superpowers/specs/2026-05-05-p9-fb-25-config-include-removal-design.md`.
구현 계획: `docs/superpowers/plans/2026-05-05-p9-fb-25-config-include-removal.md`.

## Goal

- `WorkspaceCfg.include` 필드 제거 (denylist-only 모델 정착).
- 사용자가 ingest 결과에서 어떤 파일이 왜 skip 됐는지 즉시 파악.
- 지원 형식 (md / png / jpg / pdf) 을 README + `kebab init` config 주석에 명시.

## Behavior contract

- 옛 config 의 `include = [...]` 은 silently 무시 + 단발 deprecation warning.
- Skipped 시 `IngestItem.warnings` = `["unsupported media type: .ext"]` 또는 `["unsupported media type: <no-ext>"]` 또는 `["kb:// URI not yet supported"]`.
- `IngestReport.skipped_by_extension` = `BTreeMap<lowercase-ext, count>`. no-ext 키 = `<no-ext>`.
- CLI / TUI summary final / aborted 라인에 `"N skipped: A docx, B txt, ..."` (desc 정렬, 모두).

## Tests

- legacy include 무시 + 새 WorkspaceCfg 필드 destructure (kebab-config).
- skip_reason 통합 (kebab-app): docx + Makefile 두 파일 ingest → warnings + skipped_by_extension 채워짐.
- status_line breakdown (kebab-tui).
- init template 헤더 (kebab-app).
- ext_for_skip_warning helper (kebab-app).

## Risks / notes

- 옛 config 가 narrow allowlist (예: `include = ["**/*.md"]`) 면 본 변경 후 `.png` 등이 자동 ingest 시작 — deprecation warning + README 가 alarm.
- `SourceScope.include` (kebab-core) 는 그대로.

Live deviations 반영 위치: `tasks/HOTFIXES.md` `2026-05-05 — p9-fb-25` 항목.
```

- [ ] **Step 6: Final commit**

```bash
git add README.md HANDOFF.md tasks/HOTFIXES.md tasks/INDEX.md tasks/p9/p9-fb-25-config-include-removal.md
git commit -m "docs(p9-fb-25): README + HANDOFF + HOTFIXES + INDEX + per-task spec

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Self-Review Notes (writer)

**Spec coverage:**
- `WorkspaceCfg.include` 제거 + deprecation warning → Task 1.
- SourceScope construction cleanup → Task 2.
- `kebab init` header supported-extensions → Task 3.
- `IngestReport.skipped_by_extension` + AggregateCounts + wire schema → Task 4.
- `IngestItem.warnings` populate + asset loop bumps → Task 5.
- CLI / TUI summary breakdown render → Task 6.
- README + docs sync → Task 7.

**Type / API consistency:**
- `BTreeMap<String, u32>` used in both `IngestReport.skipped_by_extension` (Task 4) and `AggregateCounts.skipped_by_extension` (Task 4) — same type.
- `<no-ext>` sentinel used in BOTH `IngestItem.warnings` (Task 5) and `BTreeMap` key (Task 5).
- `ext_for_skip_warning` helper defined in Task 5, consumed by Tasks 5 + 6 (CLI + TUI consume the resulting `BTreeMap`, not the helper directly).
- `Copy` derive removed from `AggregateCounts` (Task 4) — callers using `let counts = state.counts;` continue to compile because `Clone` still works (assignment of non-Copy type via move; Rust borrow checker handles).

**Placeholder scan:** Each step has full code. Adapter-language ("adapt to actual existing helper") is reserved for genuine ambiguity (CLI summary print site, init template test fixture pattern) — the engineer must inspect 5-10 lines of context.

**Risks documented:**
- `Copy` removal on `AggregateCounts` may surface compile errors at call sites that rely on `Copy`. Plan flags this in Task 4 step 2 with grep instruction.
- Deprecation warning might fire from the `kebab init` test if it produces a config with `include = [...]` first. Task 3's test uses `force=true` on a fresh dir → no `include` in default → no warning. Acceptable.
- `set_var(XDG_CONFIG_HOME)` in init test relies on Rust 2024 `unsafe`. Plan flags the wrapping requirement.
