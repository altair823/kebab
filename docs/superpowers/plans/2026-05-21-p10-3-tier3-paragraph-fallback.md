# p10-3 Tier 3 Paragraph Fallback Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Activate the `code-text-paragraph-v1` chunker — paragraph + line-window fallback for shell scripts and for Tier 1/2 0-chunk / Err results (non-k8s YAML, invalid YAML, AST extractor failures).

**Architecture:** Single new chunker module `crates/kebab-chunk/src/code_text_paragraph_v1.rs` using blank-line paragraph segmentation and 80-line / 20-overlap line-window split for oversize paragraphs. `tier2_shared::build_chunk` is exposed as `pub(crate)` so Tier 3 shares the same Chunk-construction semantics as Tier 1/2. `ingest_one_code_asset` gains a `"shell"` arm in its 4-arm match plus a post-match fallback wrapper that retries Tier 1/2 results in `Ok(empty)` / `Err(_)` shape against Tier 3, swapping `chunker_version` + `parser_version` for downstream stamping.

**Tech Stack:** Rust 2024 workspace. No new external deps (string operations only). Reuses `tier2_shared::build_chunk` (Task D of p10-2 / commit 8996e73).

**Memory note:** Host has been OOM'd previously. Per-crate cargo only. ONE full-suite + clippy gate at Task I. NO `cargo test --workspace` outside that gate.

---

## Pre-flight

Branch `feat/p10-3-tier3-paragraph` already exists (spec commit `9d4a60a`).

- [ ] **Disk hygiene**: `df -h /` 점검. 80% 넘으면 `cargo clean`.

Reference files (read on-demand per task):
- `tasks/p10/p10-3-tier3-paragraph-fallback.md` — frozen contract.
- `crates/kebab-chunk/src/tier2_shared.rs` — `build_chunk` source; the visibility upgrade lives here.
- `crates/kebab-chunk/src/k8s_manifest_resource_v1.rs` — closest Tier 2 chunker template (uses `tier2_shared::push_chunks_with_oversize`).
- `crates/kebab-chunk/tests/k8s_manifest_resource_v1.rs` — `yaml_doc` helper pattern + `policy()` helper; Tier 3 tests mirror this shape.
- `crates/kebab-app/src/lib.rs` lines 950-970 (allowlist) + 1794-2040 (ingest_one_code_asset).
- `crates/kebab-app/tests/code_ingest_smoke.rs` — 12 existing tests; Tier 3 tests mirror the `TestEnv::lexical_only()` pattern.

---

## Task A: expose `tier2_shared::build_chunk` as `pub(crate)`

**Files:**
- Modify: `crates/kebab-chunk/src/tier2_shared.rs`

- [ ] **Step 1**: Read `crates/kebab-chunk/src/tier2_shared.rs` to confirm `build_chunk`'s current visibility (likely module-private `fn`).

- [ ] **Step 2**: Change `fn build_chunk(...)` to `pub(crate) fn build_chunk(...)`. Signature unchanged:

```rust
pub(crate) fn build_chunk(
    doc: &Document,
    policy: &ChunkPolicy,
    text: &str,
    line_start: u32,
    line_end: u32,
    symbol: &str,
    lang: &str,
    chunker_version: &str,
) -> Result<Chunk> {
    // body unchanged
}
```

- [ ] **Step 3**: Per-crate build sanity:

```bash
cargo build -p kebab-chunk 2>&1 | tail -3
```

Expected: clean.

- [ ] **Step 4**: Commit:

```bash
git add crates/kebab-chunk/src/tier2_shared.rs
git commit -m "$(cat <<'EOF'
refactor(p10-3): expose tier2_shared::build_chunk as pub(crate)

Tier 3 chunker (next task) needs to call the same Chunk-construction helper
to keep id / hash / token-count / policy_hash semantics identical with
Tier 2. Visibility-only change; signature and body unchanged.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task B: `code-text-paragraph-v1` chunker (TDD)

**Files:**
- Create: `crates/kebab-chunk/src/code_text_paragraph_v1.rs`
- Create: `crates/kebab-chunk/tests/fixtures/sample_shell.sh`
- Create: `crates/kebab-chunk/tests/fixtures/sample_long_paragraph.txt`
- Create: `crates/kebab-chunk/tests/code_text_paragraph_v1.rs`
- Modify: `crates/kebab-chunk/src/lib.rs` (pub mod + pub use)

### B.1 — fixtures

- [ ] **Step 1**: Create `crates/kebab-chunk/tests/fixtures/sample_shell.sh` (3-paragraph shell, each < 80 lines):

```sh
#!/usr/bin/env bash
set -euo pipefail

# First paragraph: env setup
export KEBAB_HOME="${KEBAB_HOME:-$HOME/.local/share/kebab}"
mkdir -p "$KEBAB_HOME"
cd "$KEBAB_HOME"

# Second paragraph: ingest

echo "ingesting workspace..."
kebab ingest --config /etc/kebab/config.toml

# Third paragraph: report

echo "done"
kebab schema --json | jq '.stats'
```

Note: blank lines BETWEEN the three logical sections are the paragraph boundaries. Each section starts with a `#` comment and runs ~3-4 lines. Total file ~13 lines.

- [ ] **Step 2**: Create `crates/kebab-chunk/tests/fixtures/sample_long_paragraph.txt` (single 200-line paragraph, no blank lines — exercises line-window split):

```bash
# generate with a small loop — content is irrelevant, the line count is the test
python3 -c 'print("\n".join(f"line {i:03d}" for i in range(1, 201)))' \
  > crates/kebab-chunk/tests/fixtures/sample_long_paragraph.txt
```

Verify line count = 200:

```bash
wc -l crates/kebab-chunk/tests/fixtures/sample_long_paragraph.txt
```

Expected: `200 crates/kebab-chunk/tests/fixtures/sample_long_paragraph.txt`.

### B.2 — failing tests

- [ ] **Step 3**: Create `crates/kebab-chunk/tests/code_text_paragraph_v1.rs` with the same helper structure as `tests/k8s_manifest_resource_v1.rs` (which has a `yaml_doc(text) -> CanonicalDocument` helper). Mirror it as `text_doc(lang, text) -> CanonicalDocument`. Then four tests:

```rust
//! Behavioural tests for `CodeTextParagraphV1Chunker`.

use std::path::PathBuf;

use kebab_chunk::{ChunkPolicy, Chunker, CodeTextParagraphV1Chunker};
use kebab_core::{
    AssetId, Block, CanonicalDocument, CodeBlock, CommonBlock, Lang, Metadata, ParserVersion,
    Provenance, SourceSpan, SourceType, TrustLevel, WorkspacePath, id_for_block, id_for_doc,
};
use time::OffsetDateTime;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

fn text_doc(lang: &str, text: &str) -> CanonicalDocument {
    let wp = WorkspacePath(format!("script.{lang}"));
    let aid = AssetId("a".repeat(64));
    let pv = ParserVersion("none-v1".into());
    let doc_id = id_for_doc(&wp, &aid, &pv);

    let line_count = text.lines().count() as u32;
    let span = SourceSpan::Code {
        line_start: 1,
        line_end: line_count.max(1),
        symbol: None,
        lang: Some(lang.into()),
    };
    let bid = id_for_block(&doc_id, "code", &[], 0, &span);
    let block = Block::Code(CodeBlock {
        common: CommonBlock {
            block_id: bid,
            heading_path: vec![],
            source_span: span,
        },
        lang: Some(lang.into()),
        code: text.to_string(),
    });

    CanonicalDocument {
        doc_id,
        source_asset_id: aid,
        workspace_path: wp,
        title: format!("script.{lang}"),
        lang: Lang("und".into()),
        blocks: vec![block],
        metadata: Metadata {
            aliases: vec![],
            tags: vec![],
            created_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
            updated_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
            source_type: SourceType::Note,
            trust_level: TrustLevel::Primary,
            user_id_alias: None,
            user: Default::default(),
            repo: Some("kebab".into()),
            git_branch: Some("main".into()),
            git_commit: Some("0".repeat(40)),
            code_lang: Some(lang.into()),
        },
        provenance: Provenance { events: vec![] },
        parser_version: pv,
        schema_version: 1,
        doc_version: 1,
        last_chunker_version: None,
        last_embedding_version: None,
    }
}

fn policy() -> ChunkPolicy {
    ChunkPolicy::default()
}

#[test]
fn shell_multi_paragraph_splits_on_blank_lines() {
    let text = std::fs::read_to_string(fixtures_dir().join("sample_shell.sh"))
        .expect("read sample_shell.sh");
    let doc = text_doc("shell", &text);
    let chunks = CodeTextParagraphV1Chunker.chunk(&doc, &policy()).expect("chunk");

    // 3 paragraphs separated by blank lines.
    assert_eq!(chunks.len(), 3, "expected 3 paragraph chunks, got {}", chunks.len());

    for c in &chunks {
        match &c.source_spans[0] {
            SourceSpan::Code { symbol, lang, .. } => {
                assert_eq!(symbol.as_deref(), None, "Tier 3 symbol must be None");
                assert_eq!(lang.as_deref(), Some("shell"));
            }
            other => panic!("expected Code span, got {other:?}"),
        }
    }

    // Line ranges must be ascending and not overlap (blank lines are NOT in any chunk).
    let ranges: Vec<(u32, u32)> = chunks.iter().map(|c| match &c.source_spans[0] {
        SourceSpan::Code { line_start, line_end, .. } => (*line_start, *line_end),
        _ => unreachable!(),
    }).collect();
    for w in ranges.windows(2) {
        assert!(w[0].1 < w[1].0, "paragraph ranges must be strictly ascending; got {:?}", ranges);
    }
}

#[test]
fn single_long_paragraph_line_window_split() {
    let text = std::fs::read_to_string(fixtures_dir().join("sample_long_paragraph.txt"))
        .expect("read sample_long_paragraph.txt");
    let doc = text_doc("shell", &text);
    let chunks = CodeTextParagraphV1Chunker.chunk(&doc, &policy()).expect("chunk");

    // 200 lines / window 80 / overlap 20 / stride 60
    //   chunk[0] = 1..80   (80 lines)
    //   chunk[1] = 61..140 (80 lines)
    //   chunk[2] = 121..200 (80 lines)
    // → exactly 3 chunks.
    assert_eq!(chunks.len(), 3, "expected 3 windows for 200-line paragraph, got {}", chunks.len());

    let ranges: Vec<(u32, u32)> = chunks.iter().map(|c| match &c.source_spans[0] {
        SourceSpan::Code { line_start, line_end, .. } => (*line_start, *line_end),
        _ => unreachable!(),
    }).collect();
    assert_eq!(ranges, vec![(1, 80), (61, 140), (121, 200)]);

    // chunk_ids must all differ (id_for_chunk's split_key suffix).
    let ids: std::collections::HashSet<_> = chunks.iter().map(|c| c.chunk_id.clone()).collect();
    assert_eq!(ids.len(), 3, "line-window chunks must have distinct chunk_ids");
}

#[test]
fn empty_file_emits_zero_chunks() {
    let doc = text_doc("shell", "");
    let chunks = CodeTextParagraphV1Chunker.chunk(&doc, &policy()).expect("chunk");
    assert!(chunks.is_empty(), "empty text → 0 chunks");
}

#[test]
fn lang_field_preserved_from_input_doc() {
    let yaml = "key1: value1\nkey2: value2\n";
    let doc = text_doc("yaml", yaml);
    let chunks = CodeTextParagraphV1Chunker.chunk(&doc, &policy()).expect("chunk");
    assert_eq!(chunks.len(), 1);
    match &chunks[0].source_spans[0] {
        SourceSpan::Code { lang, symbol, .. } => {
            assert_eq!(symbol.as_deref(), None);
            assert_eq!(lang.as_deref(), Some("yaml"), "Tier 3 must preserve input lang");
        }
        other => panic!("expected Code span, got {other:?}"),
    }
}
```

- [ ] **Step 4**: Run tests → FAIL (module/struct not yet defined):

```bash
cargo test -p kebab-chunk --test code_text_paragraph_v1 -- --nocapture 2>&1 | tail -10
```

Expected: compile error `CodeTextParagraphV1Chunker not found`.

### B.3 — chunker implementation

- [ ] **Step 5**: Create `crates/kebab-chunk/src/code_text_paragraph_v1.rs`:

```rust
//! p10-3: Tier 3 paragraph + line-window fallback chunker.
//!
//! Triggered for shell scripts (`.sh`/`.bash`/`.zsh`) directly, and as a
//! fallback when Tier 1/2 chunkers return `Ok(empty)` or `Err`. Splits by
//! blank lines into paragraphs; paragraphs > 80 lines are further split
//! into 80-line windows with 20-line overlap.

use crate::tier2_shared::build_chunk;
use crate::{Chunker, ChunkPolicy};
use anyhow::Result;
use kebab_core::{Block, Chunk, Document};

pub const VERSION_LABEL: &str = "code-text-paragraph-v1";

const FALLBACK_LINES_PER_CHUNK: usize = 80;
const FALLBACK_LINES_OVERLAP: usize = 20;
// stride = FALLBACK_LINES_PER_CHUNK - FALLBACK_LINES_OVERLAP = 60.

pub struct CodeTextParagraphV1Chunker;

impl Chunker for CodeTextParagraphV1Chunker {
    fn chunker_version(&self) -> &'static str { VERSION_LABEL }

    fn chunk(&self, doc: &Document, policy: &ChunkPolicy) -> Result<Vec<Chunk>> {
        let Some(Block::Code { text, lang, .. }) = doc.blocks.first() else {
            return Ok(vec![]);
        };
        let lang_str = lang.as_deref().unwrap_or("");

        let mut chunks = Vec::new();
        for para in split_paragraphs(text) {
            push_paragraph(&mut chunks, doc, policy, &para, lang_str)?;
        }
        Ok(chunks)
    }
}

/// Single paragraph + its 1-indexed line range.
struct Paragraph<'a> {
    text: String,           // joined lines (no trailing newline)
    line_start: u32,
    line_end: u32,
    // unused but kept for future ergonomics:
    _src: std::marker::PhantomData<&'a ()>,
}

fn split_paragraphs(text: &str) -> Vec<Paragraph<'_>> {
    let mut paragraphs = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    let mut current_start: Option<u32> = None;  // 1-indexed line number where current paragraph began

    for (idx, line) in text.lines().enumerate() {
        let line_no = (idx + 1) as u32;  // 1-indexed
        let is_blank = line.trim().is_empty();
        if is_blank {
            // Boundary: flush current paragraph.
            if let Some(start) = current_start.take() {
                let end = start + current.len() as u32 - 1;
                paragraphs.push(Paragraph {
                    text: current.join("\n"),
                    line_start: start,
                    line_end: end,
                    _src: std::marker::PhantomData,
                });
                current.clear();
            }
        } else {
            if current_start.is_none() {
                current_start = Some(line_no);
            }
            current.push(line);
        }
    }
    // Trailing paragraph at EOF (no boundary blank line).
    if let Some(start) = current_start.take() {
        let end = start + current.len() as u32 - 1;
        paragraphs.push(Paragraph {
            text: current.join("\n"),
            line_start: start,
            line_end: end,
            _src: std::marker::PhantomData,
        });
    }
    paragraphs
}

fn push_paragraph(
    out: &mut Vec<Chunk>,
    doc: &Document,
    policy: &ChunkPolicy,
    para: &Paragraph<'_>,
    lang: &str,
) -> Result<()> {
    let n_lines = (para.line_end - para.line_start + 1) as usize;
    if n_lines <= FALLBACK_LINES_PER_CHUNK {
        out.push(build_chunk(
            doc, policy,
            &para.text,
            para.line_start, para.line_end,
            "",   // empty symbol — build_chunk wraps as Some(""); see Step 7 note
            lang,
            VERSION_LABEL,
        )?);
        return Ok(());
    }

    // Line-window split. Stride = window - overlap = 60.
    let stride = FALLBACK_LINES_PER_CHUNK - FALLBACK_LINES_OVERLAP;
    let lines: Vec<&str> = para.text.lines().collect();
    let mut i = 0usize;
    while i < lines.len() {
        let end = (i + FALLBACK_LINES_PER_CHUNK).min(lines.len());
        let window_text = lines[i..end].join("\n");
        let window_start = para.line_start + i as u32;
        let window_end = para.line_start + (end as u32) - 1;
        out.push(build_chunk(
            doc, policy,
            &window_text,
            window_start, window_end,
            "",
            lang,
            VERSION_LABEL,
        )?);
        if end == lines.len() {
            break;
        }
        i += stride;
    }
    Ok(())
}
```

- [ ] **Step 6**: Register the module in `crates/kebab-chunk/src/lib.rs` (next to existing Tier 2 chunker exports):

```rust
pub mod code_text_paragraph_v1;
pub use code_text_paragraph_v1::CodeTextParagraphV1Chunker;
```

- [ ] **Step 7**: **`symbol = ""` vs `symbol = None` correction**. The spec says Tier 3 chunks must have `Citation::Code.symbol = None`. But `tier2_shared::build_chunk` takes `symbol: &str` and likely wraps it as `Some(s.to_string())`. Two options:
  - **(preferred)** Add a sibling helper `build_chunk_no_symbol(doc, policy, text, line_start, line_end, lang, chunker_version) -> Result<Chunk>` in `tier2_shared.rs` that constructs `SourceSpan::Code { ..., symbol: None, lang: Some(lang.to_string()) }`. The current `build_chunk` keeps wrapping `Some(symbol)`.
  - (alternative) Change `build_chunk`'s symbol parameter to `Option<&str>`. More disruption (Tier 2 callers need an update).

Take the preferred path. Edit `crates/kebab-chunk/src/tier2_shared.rs`: add (next to `build_chunk`):

```rust
/// Like `build_chunk` but emits `symbol: None`. Used by Tier 3.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_chunk_no_symbol(
    doc: &Document,
    policy: &ChunkPolicy,
    text: &str,
    line_start: u32,
    line_end: u32,
    lang: &str,
    chunker_version: &str,
) -> Result<Chunk> {
    // Mirror build_chunk's body but with symbol: None in the span.
    // The simplest implementation calls build_chunk's underlying machinery —
    // pull the body into a helper if needed, or inline minimally:
    let span = SourceSpan::Code {
        line_start,
        line_end,
        symbol: None,
        lang: Some(lang.to_string()),
    };
    build_chunk_from_span(doc, policy, text, span, chunker_version)
}
```

If `build_chunk` has a `_from_span` substructure already (see `tier2_shared.rs`), call into it. If not, extract a `build_chunk_from_span(doc, policy, text, span, chunker_version) -> Result<Chunk>` private helper, then have both `build_chunk` (with `symbol: Some(...)`) and `build_chunk_no_symbol` call into it. The diff stays small.

Update `code_text_paragraph_v1.rs::push_paragraph` to call `build_chunk_no_symbol` instead of `build_chunk(..., "", ...)`:

```rust
use crate::tier2_shared::build_chunk_no_symbol;
// ...
out.push(build_chunk_no_symbol(
    doc, policy,
    &para.text,
    para.line_start, para.line_end,
    lang,
    VERSION_LABEL,
)?);
```

And the same swap in the line-window branch.

- [ ] **Step 8**: Run tests:

```bash
cargo test -p kebab-chunk --test code_text_paragraph_v1 -- --nocapture 2>&1 | tail -25
```

Expected: 4 PASS.

- [ ] **Step 9**: Run all kebab-chunk tests (no regression on Tier 1/2 from `tier2_shared` edit):

```bash
cargo test -p kebab-chunk -- --nocapture 2>&1 | tail -20
```

Expected: all PASS.

- [ ] **Step 10**: Clippy + commit:

```bash
cargo clippy -p kebab-chunk --all-targets -- -D warnings
git add crates/kebab-chunk/src/code_text_paragraph_v1.rs \
        crates/kebab-chunk/src/tier2_shared.rs \
        crates/kebab-chunk/src/lib.rs \
        crates/kebab-chunk/tests/fixtures/sample_shell.sh \
        crates/kebab-chunk/tests/fixtures/sample_long_paragraph.txt \
        crates/kebab-chunk/tests/code_text_paragraph_v1.rs
git commit -m "$(cat <<'EOF'
feat(p10-3): code-text-paragraph-v1 chunker — paragraph + line-window fallback

Blank-line paragraph segmentation (whitespace-only lines as boundaries,
blank lines themselves never in any chunk's range). Paragraphs > 80 lines
split into 80-line windows with 20-line overlap (stride 60), sharing the
input lang and symbol=None per spec §9.3. tier2_shared exposes a new
build_chunk_no_symbol helper so Chunk id/hash/token semantics stay
identical with Tier 1/2.

4 unit tests cover multi-paragraph shell, 200-line oversize line-window
split (chunks 1-80 / 61-140 / 121-200), empty file, and lang preservation
when input is yaml.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task C: shell direct routing in `ingest_one_code_asset`

**Files:**
- Modify: `crates/kebab-app/src/lib.rs`

### C.1 — allowlist + 4-arm match

- [ ] **Step 1**: Open `crates/kebab-app/src/lib.rs` line 953. Current allowlist:

```rust
if matches!(lang.as_str(),
    "rust" | "python" | "typescript" | "javascript" | "go" | "java" | "kotlin"
    | "yaml" | "dockerfile" | "toml" | "json" | "xml" | "groovy" | "go-mod")
```

Add `"shell"`:

```rust
if matches!(lang.as_str(),
    "rust" | "python" | "typescript" | "javascript" | "go" | "java" | "kotlin"
    | "yaml" | "dockerfile" | "toml" | "json" | "xml" | "groovy" | "go-mod"
    | "shell")
```

- [ ] **Step 2**: At the top-of-file `use kebab_chunk::{...}` line, add `CodeTextParagraphV1Chunker`:

```rust
use kebab_chunk::{
    /* existing items */,
    CodeTextParagraphV1Chunker,
};
```

- [ ] **Step 3**: parser_version match (line ~1825):

```rust
let parser_version = match code_lang {
    // ... existing 7 Tier 1 arms ...
    "kotlin" => ParserVersion(kebab_parse_code::KOTLIN_PARSER_VERSION.to_string()),
    "yaml" | "dockerfile" | "toml" | "json" | "xml" | "groovy" | "go-mod"
        => ParserVersion("none-v1".to_string()),
    // p10-3: shell also uses Tier 3 (no parse step).
    "shell" => ParserVersion("none-v1".to_string()),
    other => anyhow::bail!("unsupported code_lang: {other}"),
};
```

- [ ] **Step 4**: chunker_version match:

```rust
let chunker_version = match code_lang {
    // ... existing arms ...
    "toml" | "json" | "xml" | "groovy" | "go-mod"
                 => ManifestFileV1Chunker.chunker_version(),
    // p10-3:
    "shell"      => CodeTextParagraphV1Chunker.chunker_version(),
    other => anyhow::bail!("unreachable chunker_version: {other}"),
};
```

- [ ] **Step 5**: extract match (canonical Document construction):

```rust
let mut canonical = match code_lang {
    // ... existing Tier 1 + Tier 2 arms ...
    "yaml" | "dockerfile" | "toml" | "json" | "xml" | "groovy" | "go-mod" => {
        synthesize_tier2_document(asset, &bytes, code_lang, &parser_version)?
    }
    // p10-3: shell reuses the same synthesizer — single Block::Code with raw text.
    "shell" => synthesize_tier2_document(asset, &bytes, "shell", &parser_version)?,
    other => anyhow::bail!("unreachable (extract): {other}"),
};
```

- [ ] **Step 6**: chunks match:

```rust
let chunks = match code_lang {
    // ... existing Tier 1 + Tier 2 arms ...
    "toml" | "json" | "xml" | "groovy" | "go-mod"
                 => ManifestFileV1Chunker.chunk(&canonical, chunk_policy)
                       .context("kb-chunk::ManifestFileV1Chunker::chunk")?,
    // p10-3:
    "shell"      => CodeTextParagraphV1Chunker.chunk(&canonical, chunk_policy)
                       .context("kb-chunk::CodeTextParagraphV1Chunker::chunk (code:shell)")?,
    other => anyhow::bail!("unreachable (chunk): {other}"),
};
```

- [ ] **Step 7**: Build:

```bash
cargo build -p kebab-app 2>&1 | tail -5
```

Expected: clean.

- [ ] **Step 8**: Clippy + interim commit (allowlist + 4 arms only; fallback wrapper is the next task):

```bash
cargo clippy -p kebab-app --all-targets -- -D warnings
git add crates/kebab-app/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(p10-3): activate shell direct routing through Tier 3 chunker

Extends ingest_one_code_asset's allowlist + 4-arm match (parser_version /
chunker_version / extract / chunks) to admit code_lang "shell" and route it
to CodeTextParagraphV1Chunker. parser_version "none-v1" + synthesize_tier2_document
reused.

Tier 1/2 fallback wrapper lands in the next commit.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task D: Tier 1/2 → Tier 3 fallback wrapper

**Files:**
- Modify: `crates/kebab-app/src/lib.rs`

The post-chunk fallback wrapper. After the `chunks` match resolves, if the result is `Ok(empty)` (Tier 2 invalid YAML / non-k8s YAML) or `Err(_)` (Tier 1 extractor / chunker failure), retry with Tier 3.

- [ ] **Step 1**: Reshape the `chunks` match to NOT use `?`, instead bind a `Result<Vec<Chunk>>`:

```rust
let chunks_result: anyhow::Result<Vec<Chunk>> = match code_lang {
    "rust"       => CodeRustAstV1Chunker.chunk(&canonical, chunk_policy)
                       .context("kb-chunk::CodeRustAstV1Chunker::chunk (code:rust)"),
    "python"     => CodePythonAstV1Chunker.chunk(&canonical, chunk_policy)
                       .context("kb-chunk::CodePythonAstV1Chunker::chunk (code:python)"),
    // ... existing arms similarly bind Result, no `?` ...
    "shell"      => CodeTextParagraphV1Chunker.chunk(&canonical, chunk_policy)
                       .context("kb-chunk::CodeTextParagraphV1Chunker::chunk (code:shell)"),
    other => anyhow::bail!("unreachable (chunk): {other}"),
};
```

(Every existing arm: replace its `.context(...)?` with `.context(...)` — drop the trailing `?`. The result of the whole match is now `anyhow::Result<Vec<Chunk>>`.)

- [ ] **Step 2**: Add the fallback wrapper directly after the match:

```rust
// p10-3: Tier 1/2 0-chunk OR error → Tier 3 fallback retry.
// "shell" is direct Tier 3 already; don't retry-double-up.
let chunks = match chunks_result {
    Ok(v) if !v.is_empty() => v,
    other if code_lang == "shell" => {
        // shell direct call already IS Tier 3 — don't retry. Propagate.
        other?
    }
    Ok(_empty) => {
        tracing::warn!(
            workspace_path = %asset.workspace_path,
            code_lang = code_lang,
            "tier1/2 emitted 0 chunks; falling back to tier 3 (code-text-paragraph-v1)"
        );
        chunker_version = CodeTextParagraphV1Chunker.chunker_version();
        canonical.parser_version = ParserVersion("none-v1".to_string());
        CodeTextParagraphV1Chunker.chunk(&canonical, chunk_policy)
            .context("kb-chunk::CodeTextParagraphV1Chunker::chunk (tier 3 fallback)")?
    }
    Err(e) => {
        tracing::warn!(
            workspace_path = %asset.workspace_path,
            code_lang = code_lang,
            error = %e,
            "tier1/2 errored; falling back to tier 3 (code-text-paragraph-v1)"
        );
        chunker_version = CodeTextParagraphV1Chunker.chunker_version();
        canonical.parser_version = ParserVersion("none-v1".to_string());
        CodeTextParagraphV1Chunker.chunk(&canonical, chunk_policy)
            .context("kb-chunk::CodeTextParagraphV1Chunker::chunk (tier 3 fallback after error)")?
    }
};
```

Notes:
- `chunker_version` must already be a `let mut` binding (Task C step 4). If it's currently `let`, change to `let mut`.
- `canonical.parser_version` mutation requires `canonical` to be `let mut` (it already is per Task G of p10-2 — `let mut canonical = match ...`).
- The `workspace_path` field on `asset` is a `WorkspacePath(String)` newtype; the `%` formatter uses its `Display` impl. Verify by `git grep "fn fmt" crates/kebab-core/src/ids.rs` if needed — `WorkspacePath` derives `Display`.

- [ ] **Step 3**: When fallback fires, the extract step for a Tier 1 lang (`"rust"` / `"python"` / ...) **didn't run** (because the Tier 1 `extract` call errored before reaching `chunks_result`). So `canonical` may already be set up correctly for the Tier 3 chunker — IF the extract step succeeded but chunking returned empty. But if extract itself errored (e.g. tree-sitter parse failure), `canonical` was never built and the chunks_result match arm never executed.

  Reshape the **extract** match similarly:

```rust
let canonical_result: anyhow::Result<kebab_core::CanonicalDocument> = match code_lang {
    "rust" => RustAstExtractor::new().extract(&ctx, &bytes)
                .context("kb-parse-code::RustAstExtractor::extract (code:rust)"),
    // ... existing arms ... → drop trailing `?`
    "shell" => Ok(synthesize_tier2_document(asset, &bytes, "shell", &parser_version)?),
    // (synthesize_tier2_document returns anyhow::Result<CanonicalDocument>; the `?` here
    //  is fine because the Tier 2 synthesizer call is itself the inner Result. If it
    //  failed, we want to propagate the synthesizer error — synthesize_tier2_document
    //  can fail on non-utf8 bytes; falling back from a non-utf8 file makes no sense.)
    other => anyhow::bail!("unreachable (extract): {other}"),
};

// p10-3: extract failure (e.g. tree-sitter parse error) → Tier 3 fallback with
// a synthesized Document.
let mut canonical = match canonical_result {
    Ok(d) => d,
    Err(_) if code_lang == "shell" => {
        // shell's extract goes through synthesize_tier2_document — if THAT fails (non-utf8),
        // there's nothing to fall back to. Propagate.
        canonical_result?
    }
    Err(e) => {
        tracing::warn!(
            workspace_path = %asset.workspace_path,
            code_lang = code_lang,
            error = %e,
            "tier1/2 extract errored; falling back to tier 3 synthesized doc"
        );
        chunker_version = CodeTextParagraphV1Chunker.chunker_version();
        // Build the Tier 3 doc from raw bytes. parser_version was originally Tier 1's
        // (e.g. RUST_PARSER_VERSION); swap to "none-v1" so try_skip_unchanged keys correctly.
        let tier3_parser_version = ParserVersion("none-v1".to_string());
        let mut tier3_doc = synthesize_tier2_document(asset, &bytes, code_lang, &tier3_parser_version)?;
        tier3_doc.parser_version = tier3_parser_version;
        tier3_doc
    }
};
```

If after extract fallback the original `chunks_result` match was going to run a Tier 1 chunker against a Tier 3 doc — that would crash because Tier 1 chunkers expect AST output. So when extract fell back, the *chunks* match must also use Tier 3 directly. Solution: drop the chunks match into an `if-else` flow:

```rust
let extract_fell_back = matches!(canonical.parser_version.0.as_str(), "none-v1")
    && !matches!(code_lang, "yaml" | "dockerfile" | "toml" | "json" | "xml" | "groovy" | "go-mod" | "shell");

let chunks = if extract_fell_back {
    // Extract already fell back to Tier 3 doc shape; run Tier 3 chunker directly.
    CodeTextParagraphV1Chunker.chunk(&canonical, chunk_policy)
        .context("kb-chunk::CodeTextParagraphV1Chunker::chunk (tier 3 after extract fallback)")?
} else {
    // Normal path — Tier 1/2/3 chunker per code_lang.
    let chunks_result: anyhow::Result<Vec<Chunk>> = match code_lang {
        // ... arms above ...
    };
    // ... fallback wrapper from Step 2 ...
};
```

This is getting complex; consider a helper. **Refactor signal**: extract this logic into a single function `tier3_fallback_chunks(asset, bytes, code_lang, chunk_policy, original_parser_version, &mut canonical, &mut chunker_version) -> Result<Vec<Chunk>>` if the inline becomes hard to read.

For the plan, keep it inline but readable. The reviewer will catch readability issues. If a subagent reports DONE_WITH_CONCERNS citing complexity, refactor in a follow-up step.

- [ ] **Step 4**: Build:

```bash
cargo build -p kebab-app 2>&1 | tail -5
```

Expected: clean.

- [ ] **Step 5**: Run existing kebab-app unit tests (no regression):

```bash
cargo test -p kebab-app --lib -- --nocapture 2>&1 | tail -10
```

Expected: 52 PASS (matching the count after Task G of p10-2).

- [ ] **Step 6**: Clippy + commit:

```bash
cargo clippy -p kebab-app --all-targets -- -D warnings
git add crates/kebab-app/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(p10-3): Tier 1/2 → Tier 3 fallback wrapper in ingest_one_code_asset

After the chunks match resolves, an Ok(empty) result (Tier 2 invalid YAML
/ non-k8s YAML / similar) or Err (Tier 1 extractor / chunker failure) is
retried against CodeTextParagraphV1Chunker. On retry, chunker_version is
swapped to "code-text-paragraph-v1" and canonical.parser_version to
"none-v1" so downstream stamping + try_skip_unchanged remain consistent.

Extract failure is handled similarly — when a Tier 1 extractor errors
(e.g. tree-sitter parse failure), a synthesize_tier2_document-shaped
fallback doc is built from raw bytes and routed through Tier 3 chunker
directly.

shell direct path is exempted from the fallback chain (it IS Tier 3
already).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task E: integration smoke tests (Tier 3)

**Files:**
- Modify: `crates/kebab-app/tests/code_ingest_smoke.rs`

Two new tests. Mirror the pattern from p10-2's three Tier 2 tests (commit `166e1dd`).

- [ ] **Step 1**: Read the existing `tests/code_ingest_smoke.rs` first — especially the three p10-2 Tier 2 tests near the end (`tier2_k8s_yaml_ingest_searchable`, `tier2_dockerfile_ingest_searchable`, `tier2_cargo_toml_ingest_searchable`). Replicate the `TestEnv::lexical_only()` + ingest_with_config + search_with_config pattern.

- [ ] **Step 2**: Append two tests at the end of the file:

```rust
#[test]
fn tier3_shell_ingest_searchable() {
    let env = TestEnv::lexical_only();
    let workspace = env.workspace_root();
    std::fs::write(
        workspace.join("deploy.sh"),
        "#!/usr/bin/env bash\nset -e\necho hello\n\nkebab ingest --json\n",
    )
    .unwrap();

    let report = env.ingest().expect("ingest");
    assert!(report.new_docs >= 1, "expected at least 1 new doc, got {}", report.new_docs);

    let hits = env.search_code_lang("shell", "kebab").expect("search");
    assert!(!hits.is_empty(), "expected at least 1 shell hit");

    let citation = match &hits[0].citation {
        Citation::Code { symbol, lang, .. } => (symbol.clone(), lang.clone()),
        other => panic!("expected Citation::Code, got {other:?}"),
    };
    assert_eq!(citation.0, None, "Tier 3 symbol must be None");
    assert_eq!(citation.1.as_deref(), Some("shell"));

    // chunker_version should be code-text-paragraph-v1.
    assert_eq!(
        hits[0].chunker_version.as_deref(),
        Some("code-text-paragraph-v1"),
        "shell chunks must be stamped with the Tier 3 chunker_version"
    );
}

#[test]
fn tier3_yaml_fallback_picks_up_non_k8s_yaml() {
    let env = TestEnv::lexical_only();
    let workspace = env.workspace_root();

    // docker-compose-shaped YAML — has `version:` and `services:` but no apiVersion/kind.
    // k8s chunker will return Ok(vec![]); the Tier 3 fallback should pick this up.
    std::fs::write(
        workspace.join("docker-compose.yml"),
        "version: '3'\nservices:\n  api:\n    image: nginx:latest\n    ports:\n      - 8080:80\n",
    )
    .unwrap();

    let report = env.ingest().expect("ingest");
    assert!(report.new_docs >= 1, "expected the non-k8s yaml to be ingested via Tier 3, got {} new docs", report.new_docs);

    let hits = env.search_code_lang("yaml", "nginx").expect("search");
    assert!(!hits.is_empty(), "expected at least 1 yaml fallback hit");

    let (symbol, lang) = match &hits[0].citation {
        Citation::Code { symbol, lang, .. } => (symbol.clone(), lang.clone()),
        other => panic!("expected Citation::Code, got {other:?}"),
    };
    assert_eq!(symbol, None, "Tier 3 fallback symbol must be None");
    assert_eq!(lang.as_deref(), Some("yaml"), "lang preserved through fallback");

    assert_eq!(
        hits[0].chunker_version.as_deref(),
        Some("code-text-paragraph-v1"),
        "non-k8s yaml fallback must be stamped code-text-paragraph-v1"
    );
}
```

(The helpers `TestEnv::lexical_only()`, `workspace_root()`, `ingest()`, `search_code_lang(lang, query)` — verify their actual names by reading the file. The first test in `code_ingest_smoke.rs` uses whatever the established API is; mirror it precisely.)

- [ ] **Step 3**: Run targeted tests:

```bash
cargo test -p kebab-app --test code_ingest_smoke tier3 -- --nocapture 2>&1 | tail -30
```

Expected: 2 PASS.

- [ ] **Step 4**: Run the entire smoke file:

```bash
cargo test -p kebab-app --test code_ingest_smoke -- --nocapture 2>&1 | tail -30
```

Expected: 14 PASS (12 existing + 2 new).

- [ ] **Step 5**: Clippy + commit:

```bash
cargo clippy -p kebab-app --tests -- -D warnings
git add crates/kebab-app/tests/code_ingest_smoke.rs
git commit -m "$(cat <<'EOF'
test(p10-3): integration smoke tests for Tier 3 (shell + yaml fallback)

Two new tests verify end-to-end Tier 3 wiring:
- tier3_shell_ingest_searchable: .sh file → --code-lang shell search →
  Citation::Code { symbol: None, lang: "shell" }, chunker_version
  "code-text-paragraph-v1".
- tier3_yaml_fallback_picks_up_non_k8s_yaml: docker-compose-shaped yaml
  (no apiVersion/kind) triggers k8s chunker's Ok(vec![]) result, fallback
  retries with Tier 3 → Citation::Code { symbol: None, lang: "yaml" } and
  chunker_version "code-text-paragraph-v1".

Brings code_ingest_smoke to 14 tests (Tier 1: 9, Tier 2: 3, Tier 3: 2).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task F: frozen design §10.1 + §10 activation log

**Files:**
- Modify: `docs/superpowers/specs/2026-05-15-kebab-code-ingest-design.md`
- Modify: `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md`

- [ ] **Step 1**: Read §10.1 of `docs/superpowers/specs/2026-05-15-kebab-code-ingest-design.md` to find the existing activation log format (Task I of p10-2 added the p10-2 entry there). Add a sibling entry right after:

```
| p10-3 | Tier 3 활성화 — code-text-paragraph-v1 active. shell direct routing + Tier 1/2 fallback wrapper (0-chunk or Err → Tier 3 retry). 비-k8s YAML / invalid YAML 자동 picked up. | 2026-05-21 |
```

(Match the table style exactly. If the existing entries are bullets, use a bullet.)

- [ ] **Step 2**: Read §10 of `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` (around line 1552 — the p10-2 entry is there). Add right after:

```
**p10-3 활성화 (Tier 3 paragraph fallback) (2026-05-21)**: Tier 3 chunker `code-text-paragraph-v1` 활성화. shell script (`.sh`/`.bash`/`.zsh`) direct routing + Tier 1/2 가 0 chunk 또는 Err 시 자동 fallback 으로 retry. 비-k8s YAML / invalid YAML / AST 실패 케이스 모두 picked up. lang 은 입력 보존 (shell → "shell", yaml → "yaml" 등), symbol 은 항상 None.
```

- [ ] **Step 3**: Commit:

```bash
git add docs/superpowers/specs/2026-05-15-kebab-code-ingest-design.md \
        docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
git commit -m "$(cat <<'EOF'
docs(p10-3): activate Tier 3 in frozen design §10.1 + §10

§10.1 (code-ingest design): add deactivation log entry for p10-3.
§10 (final-form design): mirror entry in the activation log.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task G: README + HANDOFF + ARCHITECTURE + SMOKE + tasks/INDEX + tasks/p10/INDEX

**Files:**
- Modify: `README.md`
- Modify: `HANDOFF.md`
- Modify: `docs/ARCHITECTURE.md`
- Modify: `docs/SMOKE.md`
- Modify: `tasks/INDEX.md`
- Modify: `tasks/p10/INDEX.md`

### G.1 — README

- [ ] **Step 1**: Open `README.md`. Find the `kebab ingest` row in the 명령 table (line ~73). Extend the supported-langs list with shell + the fallback note. Sample patch (adjust to actual current wording):

Find:

```
**소스코드** (`.rs` → `code-rust-ast-v1`, `.py` → ... , `.kt`/`.kts` → `code-kotlin-ast-v1` — 모두 tree-sitter AST chunker; **Tier 2 리소스 파일**: ...)
```

After the `Tier 2 리소스 파일: ...` clause, insert before the closing `)`:

```
; **Tier 3 paragraph fallback** (`.sh`/`.bash`/`.zsh` → `code-text-paragraph-v1`, blank-line paragraph split + 80-line/20-overlap line-window. Tier 1/2 가 0 chunk 또는 Err 시 자동 fallback — 비-k8s YAML 같은 케이스 picked up. symbol = None, lang 은 원본 보존.)
```

Also extend the `--code-lang` enumeration:

```
--code-lang ... / --code-lang shell / ...
```

- [ ] **Step 2**: Find the Mermaid diagram's chunker node (line ~135). Replace:

```
chunker["chunker (md-heading-v1, pdf-page-v1, code-{rust,python,ts,js,go,java,kotlin}-ast-v1, k8s-manifest-resource-v1, dockerfile-file-v1, manifest-file-v1)"]
```

with (add `code-text-paragraph-v1`):

```
chunker["chunker (md-heading-v1, pdf-page-v1, code-{rust,python,ts,js,go,java,kotlin}-ast-v1, k8s-manifest-resource-v1, dockerfile-file-v1, manifest-file-v1, code-text-paragraph-v1)"]
```

### G.2 — HANDOFF

- [ ] **Step 3**: Open `HANDOFF.md`. Find the phase table row for P10. The Tier 2 row was `**2 ✅ (Tier 2 resource-aware: ... — v0.14.0)**`. Add a sibling for Tier 3:

```
, **3 ✅ (Tier 3 paragraph fallback: code-text-paragraph-v1 — v0.15.0)**
```

(Insert into the same P10 list cell — match the comma + bold styling used by neighbors.)

Update the 한 줄 요약 at the top: replace `Tier 2 리소스 파일 (yaml/k8s / dockerfile / toml / json / xml / groovy / go-mod) 처리` with `... + Tier 3 paragraph fallback (shell / 비-k8s YAML / AST 실패) 처리`. And in 다음 후보: drop p10-3, leave `P10-1D (C/C++) 또는 P9-5 (desktop tauri) 또는 보류 중인 P8 (audio)`.

### G.3 — ARCHITECTURE

- [ ] **Step 4**: Open `docs/ARCHITECTURE.md`. Find the code parser table row (line ~25 per Task J of p10-2). After the Tier 2 sentence, add:

```
**Tier 3 (p10-3)**: shell scripts (`.sh`/`.bash`/`.zsh`) direct → `code-text-paragraph-v1` (blank-line paragraph segmentation + 80-line / 20-overlap line-window for oversize). Same chunker also serves as fallback when Tier 1/2 emit 0 chunks or Err — non-k8s YAML / invalid YAML / AST extractor failures all picked up. symbol = None; lang preserved from input doc.
```

- [ ] **Step 5**: Find the `flowchart TB` block (line ~52). The `pcode` node currently says `(P10-1A-2 + P10-1B + P10-1C-Go + P10-1C-JK + P10-2)`. Update to `(P10-1A-2 + P10-1B + P10-1C-Go + P10-1C-JK + P10-2 + P10-3)`.

- [ ] **Step 6**: Find the `crates/kebab-chunk/src/` tree (line ~165). Add an entry for `code_text_paragraph_v1.rs`:

```
│   │       ├── code_*_ast_v1.rs              # Tier 1 AST chunkers (rust/python/ts/js/go/java/kotlin)
│   │       ├── k8s_manifest_resource_v1.rs   # Tier 2 (p10-2): YAML multi-doc, apiVersion+kind per resource
│   │       ├── dockerfile_file_v1.rs         # Tier 2 (p10-2): whole-file Dockerfile
│   │       ├── manifest_file_v1.rs           # Tier 2 (p10-2): whole-file Cargo.toml / go.mod / .json / .xml / .groovy
│   │       ├── code_text_paragraph_v1.rs     # Tier 3 (p10-3): blank-line paragraph + 80/20 line-window fallback
│   │       └── tier2_shared.rs               # Tier 2 (p10-2): shared oversize fallback + Chunk builder helpers
```

### G.4 — SMOKE

- [ ] **Step 7**: Open `docs/SMOKE.md`. After the "P10-2 Tier 2 리소스 파일 색인" section, add a "P10-3 Tier 3 paragraph fallback" section:

```markdown
## P10-3 Tier 3 paragraph fallback

P10-2 와 동일한 격리 KB 설정. `.sh` 파일은 direct, 비-k8s YAML 은 fallback 으로 들어간다.

```bash
# 1) shell script (direct Tier 3)
cat > /tmp/kebab-smoke/workspace/deploy.sh <<'EOF'
#!/usr/bin/env bash
set -e

echo "ingesting..."
kebab ingest

echo "done"
kebab schema --json | jq '.stats'
EOF

# 2) 비-k8s YAML (Tier 2 가 0 chunk → Tier 3 fallback)
cat > /tmp/kebab-smoke/workspace/docker-compose.yml <<'EOF'
version: '3'
services:
  api:
    image: nginx:latest
    ports:
      - 8080:80
EOF

# 3) ingest
KB ingest

# 4) 언어별 검색 (citation.symbol = None 확인)
KB search --mode hybrid "ingest" --code-lang shell --json | \
  jq '{hits: [.hits[] | {symbol: .citation.symbol, lang: .citation.lang, chunker: .chunker_version}]}'
# 기대: symbol = null, lang = "shell", chunker_version = "code-text-paragraph-v1"

KB search --mode hybrid "nginx" --code-lang yaml --json | \
  jq '{hits: [.hits[] | {symbol: .citation.symbol, lang: .citation.lang, chunker: .chunker_version}]}'
# 기대: symbol = null, lang = "yaml", chunker_version = "code-text-paragraph-v1"

# 5) schema stats 에 shell 카운트 확인
KB --json schema | jq '.stats.code_lang_breakdown'
# 기대: {"shell": N, "yaml": M, ...} (M 은 k8s yaml + Tier 3 fallback yaml 합계)
```

**Tier 3 citation.symbol 컨벤션**: 항상 `null`. 의미 단위 식별 안 함. `lang` 은 원본 lang 보존 (shell → `"shell"`, yaml → `"yaml"` 등).
```

Append a P10-3 entry to the 검증 체크리스트 at the bottom:

```
- (P10-3) `.sh`/`.bash`/`.zsh` 파일은 direct Tier 3 (`code-text-paragraph-v1`). 비-k8s YAML (apiVersion+kind 없는 yaml) 은 k8s chunker 가 0 chunk → Tier 3 fallback 으로 picked up. `--code-lang shell` / `--code-lang yaml` 검색이 `citation.symbol = null`, `chunker_version = "code-text-paragraph-v1"` 결과를 반환하면 wiring 정상. `kebab schema --json | jq .stats.code_lang_breakdown` 에 `"shell": N` 등장 확인.
```

### G.5 — INDEX files

- [ ] **Step 8**: `tasks/INDEX.md` — flip the p10-3 row to ✅:

Find:
```
  - p10-3 Tier 3 paragraph + line-window fallback — ⏳
```
Replace with:
```
  - p10-3 Tier 3 paragraph + line-window fallback — ✅ 머지 (v0.15.0, `code-text-paragraph-v1`)
```

- [ ] **Step 9**: `tasks/p10/INDEX.md` — same row, change to ✅:

```
| 3 | Tier 3 paragraph + line-window fallback | ✅ 머지 (v0.15.0) |
```

### G.6 — commit

- [ ] **Step 10**: Single commit for all 6 docs:

```bash
git add README.md HANDOFF.md docs/ARCHITECTURE.md docs/SMOKE.md tasks/INDEX.md tasks/p10/INDEX.md
git commit -m "$(cat <<'EOF'
docs(p10-3): README/HANDOFF/ARCHITECTURE/SMOKE/INDEX sync

- README adds Tier 3 to the ingest row (shell + fallback) and the Mermaid
  chunker enumeration; --code-lang shell admitted.
- HANDOFF flips p10-3 to ✅ (v0.15.0) and updates the 한 줄 요약 + next
  candidates.
- ARCHITECTURE adds Tier 3 to the code-parser row, extends the flowchart
  pcode node, and lists code_text_paragraph_v1.rs in the chunker tree.
- SMOKE adds a P10-3 walkthrough (shell + non-k8s YAML fallback) and a
  verification checklist entry.
- tasks/INDEX + tasks/p10/INDEX flip p10-3 to ✅.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task H: workspace test gate + clippy

**Files:** (none — gates only)

- [ ] **Step 1**: Disk check:

```bash
df -h /
```

If usage > 80%, run `cargo clean` first.

- [ ] **Step 2**: Workspace test gate (memory-conscious `-j 1`):

```bash
cargo test --workspace --no-fail-fast -j 1 2>&1 | tail -80
```

Expected: ALL PASS. Especially:
- `kebab-chunk`: 4 new Tier 3 tests + existing.
- `kebab-app`: 14 tests in `code_ingest_smoke` (12 + 2 new).

If FAIL: common modes:
- A Tier 1/2 test inadvertently relied on the chunks match's prior `?`-propagation behavior — Task D's restructuring shouldn't change observable behavior but check.
- A test that expected `Err` from Tier 1 (e.g. invalid input fixture) now gets `Ok(vec![chunk])` (Tier 3 fallback). Such tests would be tests-of-failure-mode rather than tests-of-success — likely intentional regression coverage. Review case-by-case.

- [ ] **Step 3**: Workspace clippy:

```bash
cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -30
```

Expected: clean.

---

## Task I: workspace version bump + gitea PR

**Files:**
- Modify: `Cargo.toml`
- Modify: `Cargo.lock` (auto)

- [ ] **Step 1**: Edit `Cargo.toml` workspace `version = "0.14.0"` → `"0.15.0"`.

- [ ] **Step 2**: Refresh Cargo.lock:

```bash
cargo build -p kebab-cli 2>&1 | tail -5
```

Expected: clean. `Cargo.lock` cascades all 22 `kebab-*` crates to 0.15.0.

- [ ] **Step 3**: Commit:

```bash
git add Cargo.toml Cargo.lock
git commit -m "$(cat <<'EOF'
chore: bump version 0.14.0 → 0.15.0 (p10-3 Tier 3 paragraph fallback)

Minor bump — additive new chunker_version "code-text-paragraph-v1" + new
routing lang "shell" + new fallback wrapper behavior. No DB migration, no
wire schema major bump (Citation::Code.lang values were already a free
string field).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

- [ ] **Step 4**: Push branch + open gitea PR via REST API (per CLAUDE.md). Title:

```
feat(p10-3): Tier 3 paragraph + line-window fallback chunker — shell + 비-k8s YAML / AST 실패 자동 picked up
```

Body summary:

- `code-text-paragraph-v1` chunker activated (design §9.3).
- shell scripts (`.sh`/`.bash`/`.zsh`) ingest directly via Tier 3.
- Tier 1/2 0-chunk or Err results retry with Tier 3 — non-k8s YAML, invalid YAML, AST extractor failures all picked up. `chunker_version` + `parser_version` swap on fallback.
- 4 unit tests + 2 smoke tests = 6 new testing surfaces.
- frozen design §10.1 + §10 deltas.
- 0.14.0 → 0.15.0.

Test plan checkboxes:
- [x] `cargo test --workspace --no-fail-fast -j 1` PASS
- [x] `cargo clippy --workspace --all-targets -- -D warnings` clean
- [x] kebab-chunk 4 new Tier 3 unit tests PASS
- [x] kebab-app code_ingest_smoke 14 tests PASS (12 + 2 new)
- [ ] post-merge dogfood: multi-root KB ingest with mixed .sh + non-k8s yaml — verify --code-lang shell results and schema breakdown
- [ ] post-merge gitea-release v0.15.0

- [ ] **Step 5**: Wait for code-reviewer APPROVE, then merge via the gitea REST API (`POST /repos/altair823-org/kebab/pulls/<N>/merge`) and cut `gitea-release v0.15.0`.

---

## Verification matrix (final, after Task I merge)

| 검증 | 명령 | 기대 |
|------|------|------|
| shell direct | `kebab ingest /tmp/kebab-smoke/workspace/deploy.sh` + `kebab search --code-lang shell --json` | `Citation::Code { symbol: null, lang: "shell" }`, `chunker_version: "code-text-paragraph-v1"` |
| 비-k8s YAML fallback | `kebab ingest /tmp/kebab-smoke/workspace/docker-compose.yml` + `kebab search --code-lang yaml --json` | `Citation::Code { symbol: null, lang: "yaml" }`, `chunker_version: "code-text-paragraph-v1"` |
| invalid YAML fallback | malformed yaml ingest → search | Tier 3 chunks emitted (non-empty) |
| AST extractor 실패 fallback | (hard to trigger artificially — relies on tree-sitter parse failure on otherwise-valid Rust; this is dogfood territory) | `Citation::Code { symbol: null, lang: "rust" }`, `chunker_version: "code-text-paragraph-v1"` |
| `code_lang_breakdown` | `kebab schema --json | jq .stats.code_lang_breakdown` | `"shell": N`, `"yaml": M+K` (k8s + fallback) |

---

## Risks reminder (구현 중 주의)

- **Fallback wrapper 의 복잡도**: Task D 가 가장 위험 — extract failure + chunks failure 두 path 가 얽힘. 한 helper 로 추출하는 게 깔끔할 수 있음. 가독성이 나빠지면 subagent 가 DONE_WITH_CONCERNS 로 보고 → 후속 cleanup commit.
- **`tier2_shared::build_chunk_no_symbol` 추가**: 기존 `build_chunk` 의 body 를 재사용하려면 `build_chunk_from_span` 내부 helper 분리 가능. 분리하면 build_chunk + build_chunk_no_symbol 둘 다 한 곳에서 Chunk 구성.
- **shell fixture line count 정확성**: Task B Step 1 의 `sample_shell.sh` 가 정확히 3 paragraph 가 되도록 — `#` 줄과 명령 줄 사이 빈 줄이 없어야 같은 paragraph, paragraph 사이엔 정확히 1 빈 줄. fixture 생성 후 `cat -A` 로 확인 권장.
- **200-line fixture stride 계산**: Step 2 의 `sample_long_paragraph.txt` 가 정확히 200 lines. window 80 / stride 60 → 1-80, 61-140, 121-200. 마지막 window 의 시작이 121 인 이유 = 121 + 80 - 1 = 200 ≤ 200. 다음 stride (181) 의 시작은 181 인데 181 + 80 - 1 = 260 > 200 이라 그냥 EOF 까지 = 181-200 (20 lines). 즉 알고리즘에 따라 chunk 수가 3 또는 4 가 됨. Task B Step 5 의 impl 확인 — `while i < lines.len()` 에서 마지막 stride 가 EOF 를 넘어서면 짧은 마지막 window emit. 확인 결과 expectation 이 정확히 3 chunk 인지 4 chunk 인지 (3 이 맞으면 `break` 조건이 `if end == lines.len() { break }` 로 처리) 검증.
  - **재계산**: 200 lines, window 80, stride 60.
    - i=0, end=min(80, 200)=80 → chunk 1-80, break? end != len(200), 계속, i += 60 → i=60.
    - i=60, end=min(140, 200)=140 → chunk 61-140, end != 200, i=120.
    - i=120, end=min(200, 200)=200 → chunk 121-200, end == 200 → break.
  - 정확히 3 chunk. Test expectation 맞음.

- **`canonical.parser_version` mutation 의 영향 범위**: try_skip_unchanged 는 ingest_one_code_asset 시작부에서 `parser_version` (캡처된 값) 으로 체크. fallback 후 stored 가 `none-v1` 로 변경 — 다음 ingest 시 동일 lang 이면 동일 `parser_version` 으로 키 → skip 동작. Tier 1 chunker 가 미래에 정상 동작하기 시작하면 Tier 1 path 가 `RUST_PARSER_VERSION` 으로 새 키 → cache miss → reprocess. cascade rule 정상.

- **머지 후 deviation** 은 `tasks/HOTFIXES.md` dated 로그 + 본 spec `Risks / notes` cross-link.
