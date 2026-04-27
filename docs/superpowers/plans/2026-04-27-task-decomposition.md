# KB Task Decomposition Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Decompose the KB project into ~30 component-level task spec files (one self-contained PR/agent unit each) so that AI-driven implementation runs with stable contracts and minimal cross-task spec drift.

**Architecture:**
- Phase A — Author the canonical task spec template (`tasks/_template.md`).
- Phase B — Decompose P1 (Markdown ingestion) into 6 component task specs to validate the template.
- Phase C — After Phase B passes review, decompose P0 + P2..P9 into the remaining ~24 task specs in one pass.
- Each task spec cites only the frozen design doc (`docs/superpowers/specs/2026-04-27-kb-final-form-design.md`) for types, traits, schema, layout. No new domain types or traits are introduced inside task specs.

**Tech Stack:** Plain Markdown documents under `tasks/`. No code changes in this plan — produces specs that AI sub-agents will later implement against.

**Frozen contract source:** [docs/superpowers/specs/2026-04-27-kb-final-form-design.md](../specs/2026-04-27-kb-final-form-design.md). All task specs reference this. Modifications to the contract require updating that file first, then re-checking dependent task specs.

**Phase task index (target file layout):**

```
tasks/
├── INDEX.md                    # already exists — to be updated to link component tasks
├── _template.md                # Phase A
├── p0/
│   └── p0-1-skeleton.md
├── p1/
│   ├── p1-1-source-fs.md
│   ├── p1-2-parse-md-frontmatter.md
│   ├── p1-3-parse-md-blocks.md
│   ├── p1-4-normalize.md
│   ├── p1-5-chunk.md
│   └── p1-6-store-sqlite.md
├── p2/
│   ├── p2-1-fts-schema.md
│   └── p2-2-lexical-retriever.md
├── p3/
│   ├── p3-1-embedder-trait.md
│   ├── p3-2-fastembed-adapter.md
│   ├── p3-3-lancedb-store.md
│   └── p3-4-hybrid-fusion.md
├── p4/
│   ├── p4-1-llm-trait.md
│   ├── p4-2-ollama-adapter.md
│   └── p4-3-rag-pipeline.md
├── p5/
│   ├── p5-1-golden-fixture-runner.md
│   └── p5-2-metrics-compare.md
├── p6/
│   ├── p6-1-image-extractor-exif.md
│   ├── p6-2-ocr-adapter.md
│   └── p6-3-caption-adapter.md
├── p7/
│   ├── p7-1-pdf-text-extractor.md
│   └── p7-2-pdf-page-chunker.md
├── p8/
│   ├── p8-1-whisper-adapter.md
│   └── p8-2-segment-chunker.md
└── p9/
    ├── p9-1-tui-library.md
    ├── p9-2-tui-search.md
    ├── p9-3-tui-ask.md
    ├── p9-4-tui-inspect.md
    └── p9-5-desktop-tauri.md
```

Existing per-phase epic files (`tasks/phase-0-skeleton.md` … `phase-9-ui.md`) stay as epic-level overviews. Component task files under `tasks/p<N>/` are the actual unit-of-work for AI sub-agents.

**Acceptance for plan as a whole:**
- `tasks/_template.md` exists with all required sections.
- `tasks/p1/*.md` (6 files) exist, each cites the frozen design doc, lists Allowed/Forbidden deps, has self-contained Test plan.
- `tasks/p0/*.md`, `tasks/p2..p9/*.md` (~24 files) follow the same template.
- `tasks/INDEX.md` updated to link component tasks under each phase.
- `cargo` is not run in this plan (no code).

---

## Phase A — Authoring the task spec template

### Task A1: Write `tasks/_template.md`

**Files:**
- Create: `tasks/_template.md`

- [ ] **Step 1: Verify the design doc path resolves**

```bash
test -f docs/superpowers/specs/2026-04-27-kb-final-form-design.md && echo OK
```

Expected: `OK`

- [ ] **Step 2: Write the template file**

Write the following content verbatim to `tasks/_template.md`:

````markdown
---
phase: P<N>
component: <crate-or-module-name>
task_id: p<N>-<i>
title: "<Component title>"
status: planned
depends_on: []                            # other task_ids
unblocks: []                              # other task_ids
contract_source: ../docs/superpowers/specs/2026-04-27-kb-final-form-design.md
contract_sections: []                     # e.g. [§3.5, §5.5, §7.2]
---

# <task_id> — <Component title>

## Goal

<One sentence. The user-facing outcome of this task.>

## Why now / why this size

<One paragraph. Why this is the right unit of work and how it slots into the phase.>

## Allowed dependencies

- `kb-core`
- <other crates per design §8>
- <external crates with versions>

## Forbidden dependencies

- <list — every crate banned per design §8 Allowed/Forbidden table>

If any item here is needed during implementation, STOP and update the frozen design doc first.

## Inputs

| input | type | source |
|-------|------|--------|
| ...   | ...  | ...    |

## Outputs

| output | type | downstream consumer |
|--------|------|---------------------|
| ...    | ...  | ...                 |

## Public surface (signatures only — no new types)

```rust
// Cite only types/traits already defined in the frozen design doc.
// If a new helper is needed, mark it "internal" and keep it crate-private.
```

## Behavior contract

- <bullet list of must-hold invariants>
- <reference to design doc section numbers>
- <determinism / version recording / error policy>

## Storage / wire effects

- DB tables touched (read/write)
- LanceDB tables touched (read/write)
- Filesystem paths created/read
- Wire schema objects emitted (must conform to `*.v1`)

## Test plan

| kind | description | fixture / data |
|------|-------------|----------------|
| unit | ...         | ...            |
| snapshot | ... (JSON freeze) | `fixtures/...` |
| contract | trait round-trip | mock impls |
| integration | end-to-end via `kb-app` facade | tmp workspace |

All tests must run under `cargo test -p <crate>` and not require external network or Ollama unless explicitly stated.

## Definition of Done

- [ ] `cargo check -p <crate>` passes
- [ ] `cargo test -p <crate>` passes
- [ ] No imports outside Allowed dependencies
- [ ] All emitted wire JSON validates against `docs/wire-schema/v1/<schema>.schema.json` (when applicable)
- [ ] All record version fields populated per design §9
- [ ] PR body links the relevant design section numbers

## Out of scope

- <explicit list — features that other tasks cover>
- <future-phase work>

## Risks / notes

- <one paragraph max — known traps, version coupling, perf concerns>
````

- [ ] **Step 3: Verify file exists and is non-trivial**

```bash
test -s tasks/_template.md && wc -l tasks/_template.md
```

Expected: > 50 lines reported.

- [ ] **Step 4: Commit**

```bash
git add tasks/_template.md
git commit -m "tasks: add component task spec template"
```

---

## Phase B — P1 (Markdown ingestion) decomposition

P1 epic: [tasks/phase-1-markdown-ingestion.md](../../../tasks/phase-1-markdown-ingestion.md). 6 component tasks. Each cites the frozen design doc sections and lists allowed/forbidden deps per design §8.

### Task B0: Create P1 directory and update INDEX

**Files:**
- Create: `tasks/p1/` (directory)
- Modify: `tasks/INDEX.md`

- [ ] **Step 1: Create directory**

```bash
mkdir -p tasks/p1
```

- [ ] **Step 2: Append component-task subsection to `tasks/INDEX.md`**

Add this section near the end of `tasks/INDEX.md` (just before the "## 모든 task 공통 규약" heading):

```markdown
## Component task decomposition (per phase)

각 phase 의 component-level 분해. AI sub-agent 1세션 = 1 task 가 sweet spot.

- P1 — [p1/](p1/) — Markdown ingestion 6 components
  - [p1-1 source-fs](p1/p1-1-source-fs.md)
  - [p1-2 parse-md frontmatter](p1/p1-2-parse-md-frontmatter.md)
  - [p1-3 parse-md blocks](p1/p1-3-parse-md-blocks.md)
  - [p1-4 normalize](p1/p1-4-normalize.md)
  - [p1-5 chunk](p1/p1-5-chunk.md)
  - [p1-6 store-sqlite](p1/p1-6-store-sqlite.md)
```

- [ ] **Step 3: Commit**

```bash
git add tasks/p1 tasks/INDEX.md
git commit -m "tasks: prepare P1 component decomposition skeleton"
```

### Task B1: `p1-1-source-fs.md` (kb-source-fs)

**Files:**
- Create: `tasks/p1/p1-1-source-fs.md`

- [ ] **Step 1: Write the spec**

Write the following content to `tasks/p1/p1-1-source-fs.md`:

````markdown
---
phase: P1
component: kb-source-fs
task_id: p1-1
title: "Local filesystem source connector"
status: planned
depends_on: [p0-1]
unblocks: [p1-2, p1-3, p1-4, p1-5, p1-6]
contract_source: ../../docs/superpowers/specs/2026-04-27-kb-final-form-design.md
contract_sections: [§3.3, §6.2, §6.6, §7.1, §7.2 SourceConnector, §8]
---

# p1-1 — Local filesystem source connector

## Goal

Walk the workspace root, apply gitignore-style filters, compute BLAKE3 checksums, and produce `Vec<RawAsset>`.

## Why now / why this size

`SourceConnector` is the entry point of every ingest. Stable `RawAsset` output unblocks every downstream P1 task (parser, normalize, chunk, store). Small enough to deliver in one PR with full test coverage.

## Allowed dependencies

- `kb-core`
- `kb-config`
- `ignore` (gitignore semantics)
- `blake3`
- `walkdir`
- `time`
- `serde`
- `thiserror`
- `tracing`

## Forbidden dependencies

- `kb-parse-*`, `kb-normalize`, `kb-chunk`, `kb-store-*`, `kb-embed*`, `kb-search`, `kb-llm*`, `kb-rag`, `kb-tui`, `kb-desktop`

## Inputs

| input | type | source |
|-------|------|--------|
| `SourceScope` | `kb_core::SourceScope` | `kb-app` from config |
| filesystem | `&Path` | OS |
| `.kbignore` | text file | workspace root, optional |

## Outputs

| output | type | downstream consumer |
|--------|------|---------------------|
| `Vec<RawAsset>` | `kb_core::RawAsset` | `kb-parse-md`, asset writer in `kb-store-sqlite` (via `kb-app`) |

## Public surface (signatures only — no new types)

```rust
pub struct FsSourceConnector { /* internal */ }

impl FsSourceConnector {
    pub fn new(config: &kb_config::Config) -> anyhow::Result<Self>;
}

impl kb_core::SourceConnector for FsSourceConnector {
    fn scan(&self, scope: &kb_core::SourceScope) -> anyhow::Result<Vec<kb_core::RawAsset>>;
}
```

## Behavior contract

- POSIX-normalize every emitted `workspace_path` (NFC, leading `./` stripped, single `/`).
- `asset_id` derived per design §4.2 from `blake3(raw bytes)` full hex.
- `media_type` selected from extension + libmagic-like sniff fallback (`.md` → Markdown, others fall through to `MediaType::Other`).
- `discovered_at` = current `OffsetDateTime::now_utc()` at scan time.
- Combine `config.workspace.exclude` ∪ `.kbignore` for filter (union; ordering does not matter).
- Symbolic links: follow once, detect cycles via `canonicalize` + visited set.
- Files larger than `storage.copy_threshold_mb` MB → emit `AssetStorage::Reference { path, sha }` (do not copy bytes here; copying is done by the asset writer task).
- Idempotent: same input → same `Vec<RawAsset>` (sort by `workspace_path`).

## Storage / wire effects

- Reads: filesystem under `config.workspace.root`.
- Writes: nothing. (Asset copy is handled by the asset writer in `kb-store-sqlite`.)

## Test plan

| kind | description | fixture / data |
|------|-------------|----------------|
| unit | POSIX path normalization | inline cases incl. `./a/b.md`, `a//b.md`, `a/b.md` → identical |
| unit | blake3 of known bytes matches expected hex | inline |
| unit | gitignore filter (`*.tmp`, `node_modules/**`) excludes correctly | tmp tree built in test |
| unit | `.kbignore` ∪ config exclude works | tmp tree |
| unit | symlink cycle does not loop | tmp tree with `a -> b -> a` |
| snapshot | `Vec<RawAsset>` serialized JSON for fixture tree is stable | `fixtures/source-fs/tree-1` |
| determinism | re-running scan twice produces byte-identical JSON | `fixtures/source-fs/tree-1` |

All tests run under `cargo test -p kb-source-fs` with no network and no model.

## Definition of Done

- [ ] `cargo check -p kb-source-fs` passes
- [ ] `cargo test -p kb-source-fs` passes
- [ ] Snapshot test `fixtures/source-fs/tree-1` round-trips deterministically
- [ ] No imports outside Allowed dependencies (verified via `cargo tree -p kb-source-fs`)
- [ ] PR description links to design §3.3, §6.2, §7.2

## Out of scope

- File watching (P+).
- Asset copy/reference storage on disk (`kb-store-sqlite` task p1-6).
- Non-fs source connectors (HTTP, S3 — P+).

## Risks / notes

- BLAKE3 of large files (>1 GB) is fast but allocate streaming; do not load whole file in memory.
- macOS resource forks / `.DS_Store` should be excluded by default.
````

- [ ] **Step 2: Verify file exists**

```bash
test -s tasks/p1/p1-1-source-fs.md && echo OK
```

Expected: `OK`

- [ ] **Step 3: Commit**

```bash
git add tasks/p1/p1-1-source-fs.md
git commit -m "tasks: add p1-1 source-fs component spec"
```

### Task B2: `p1-2-parse-md-frontmatter.md`

**Files:**
- Create: `tasks/p1/p1-2-parse-md-frontmatter.md`

- [ ] **Step 1: Write the spec**

Write to `tasks/p1/p1-2-parse-md-frontmatter.md`:

````markdown
---
phase: P1
component: kb-parse-md (frontmatter submodule)
task_id: p1-2
title: "Markdown frontmatter parsing → Metadata"
status: planned
depends_on: [p0-1]
unblocks: [p1-4]
contract_source: ../../docs/superpowers/specs/2026-04-27-kb-final-form-design.md
contract_sections: [§3.6 Metadata, §0 Q9 frontmatter, §10 errors]
---

# p1-2 — Markdown frontmatter parsing

## Goal

Parse YAML/TOML frontmatter from Markdown bytes into `kb_core::Metadata`, with auto-derive defaults and unknown-key preservation in `metadata.user`.

## Why now / why this size

Frontmatter is small but contractually load-bearing (Q9 spec). Isolating it from block parsing keeps both halves of `kb-parse-md` simple and lets us reach 100% test coverage on the rules in design §0 Q9.

## Allowed dependencies

- `kb-core`
- `serde`
- `serde_yaml` (or `yaml-rust2`) for YAML
- `toml` for TOML
- `time`
- `lingua` (lang auto-detect — accept feature-gate if heavy)
- `thiserror`

## Forbidden dependencies

- `kb-store-*`, `kb-llm*`, `kb-rag`, `kb-embed*`, `kb-search`, `kb-tui`, `kb-desktop`, `kb-source-fs`, `kb-chunk`, `kb-normalize`, `pulldown-cmark` (block parser is a sibling task)

## Inputs

| input | type | source |
|-------|------|--------|
| Markdown bytes | `&[u8]` | extractor |
| body fallbacks | `BodyHints { first_h1: Option<String>, fs_ctime: OffsetDateTime, fs_mtime: OffsetDateTime, fallback_lang: Option<String> }` | caller |

## Outputs

| output | type | downstream |
|--------|------|------------|
| `(Metadata, Option<FrontmatterSpan>, Vec<Warning>)` | tuple | `kb-normalize` → CanonicalDocument |

## Public surface (signatures only — no new types)

```rust
pub fn parse_frontmatter(
    bytes: &[u8],
    hints: &BodyHints,
) -> anyhow::Result<(kb_core::Metadata, Option<FrontmatterSpan>, Vec<Warning>)>;
```

`FrontmatterSpan` and `Warning` are crate-internal helpers; if any new public type is needed, STOP and update the frozen design doc first.

## Behavior contract

- All Metadata fields are optional in input. Missing fields populated per design §0 Q9 derive table:
  - `title` ← first H1 (from `BodyHints.first_h1`) → filename without extension if no H1.
  - `lang` ← lingua auto-detect on first 4 KB of body → fallback `BodyHints.fallback_lang` or `"und"`.
  - `created_at` / `updated_at` ← `BodyHints.fs_ctime` / `fs_mtime` if missing.
  - `source_type` default `markdown`; `trust_level` default `primary`.
  - `aliases`, `tags` default empty.
- Unknown keys → `metadata.user` (`serde_json::Map`), preserved verbatim, no warning.
- Unknown enum value (e.g. `trust_level: weird`) → warning + replaced with default; ingest continues.
- Malformed YAML → frontmatter discarded, body still parsed, warning emitted.
- No frontmatter at all → defaults applied silently.
- `id:` field captured into `metadata.user_id_alias` (alias only — does NOT influence `doc_id` per design §4.2).

## Storage / wire effects

- None. Pure function.

## Test plan

| kind | description | fixture / data |
|------|-------------|----------------|
| unit | YAML frontmatter happy path → Metadata fields | inline |
| unit | TOML frontmatter happy path | inline |
| unit | unknown keys preserved in `metadata.user` | inline |
| unit | unknown enum value → warning + default | inline |
| unit | malformed YAML → empty Metadata + warning | inline |
| unit | no frontmatter → derive from BodyHints | inline |
| unit | `id:` field becomes `user_id_alias`, not `doc_id` factor | inline + assert via §4.2 recipe stub |
| snapshot | `fixtures/markdown/frontmatter-only.md` produces stable JSON | fixture |
| snapshot | mixed-language body with no `lang:` detects `ko` or `en` | `fixtures/markdown/mixed-lang.md` |

All tests under `cargo test -p kb-parse-md --lib frontmatter`.

## Definition of Done

- [ ] `cargo check -p kb-parse-md` passes
- [ ] `cargo test -p kb-parse-md frontmatter` passes
- [ ] No `pulldown-cmark` import in this submodule
- [ ] Snapshot tests stable across two consecutive runs
- [ ] PR links design §0 Q9, §3.6

## Out of scope

- Block parsing (p1-3).
- Building `CanonicalDocument` (p1-4).
- Persisting metadata (p1-6).

## Risks / notes

- `lingua` model load is heavy on first call; tests should reuse a static instance.
- timezone normalization: parse `created_at`/`updated_at` to UTC; preserve original offset only in `metadata.user.original_timestamps` if present and non-UTC.
````

- [ ] **Step 2: Verify and commit**

```bash
test -s tasks/p1/p1-2-parse-md-frontmatter.md && echo OK
git add tasks/p1/p1-2-parse-md-frontmatter.md
git commit -m "tasks: add p1-2 parse-md frontmatter component spec"
```

Expected: `OK`, then commit succeeds.

### Task B3: `p1-3-parse-md-blocks.md`

**Files:**
- Create: `tasks/p1/p1-3-parse-md-blocks.md`

- [ ] **Step 1: Write the spec**

Write to `tasks/p1/p1-3-parse-md-blocks.md`:

````markdown
---
phase: P1
component: kb-parse-md (blocks submodule)
task_id: p1-3
title: "Markdown body → Block tree with line spans"
status: planned
depends_on: [p0-1]
unblocks: [p1-4]
contract_source: ../../docs/superpowers/specs/2026-04-27-kb-final-form-design.md
contract_sections: [§3.4 Block, §3.4 SourceSpan, §0 Q3 citation]
---

# p1-3 — Markdown body → Block tree

## Goal

Parse Markdown body bytes into a flat `Vec<ParsedBlock>` (intermediate, crate-private) with heading paths and line ranges preserved, ready for `kb-normalize` to lift into `CanonicalDocument`.

## Why now / why this size

This is the heaviest part of P1 parser. Separating it from frontmatter and from normalization keeps each piece tractable. Determinism of line ranges directly determines citation quality (design §0 Q3 / §3.4 SourceSpan::Line).

## Allowed dependencies

- `kb-core`
- `pulldown-cmark` (CommonMark with source-map; GFM tables enabled via feature)
- `serde`
- `thiserror`

## Forbidden dependencies

- `kb-store-*`, `kb-llm*`, `kb-rag`, `kb-embed*`, `kb-search`, `kb-source-fs`, `kb-chunk`, `kb-normalize`, `kb-tui`, `kb-desktop`, `comrak` (alternative parser; pick one)

## Inputs

| input | type | source |
|-------|------|--------|
| Markdown body bytes | `&[u8]` | extractor (after frontmatter stripped) |
| `body_offset_lines` | `u32` | extractor (so line ranges are reported relative to original file) |

## Outputs

| output | type | downstream |
|--------|------|------------|
| `Vec<ParsedBlock>` (intermediate type, crate-private) | – | `kb-normalize` |
| `Vec<Warning>` | – | propagated into Provenance |

## Public surface (signatures only — no new types)

```rust
pub fn parse_blocks(body: &[u8], body_offset_lines: u32) -> anyhow::Result<(Vec<ParsedBlock>, Vec<Warning>)>;
```

`ParsedBlock` is a crate-internal mirror that maps 1:1 to `kb_core::Block` variants once `kb-normalize` assigns `BlockId`s.

## Behavior contract

- Source-map: each `ParsedBlock` carries `SourceSpan::Line { start, end }` relative to the original file (i.e., add `body_offset_lines`).
- Heading tree: every block records its ancestor heading texts in order (e.g., `["아키텍처", "Chunking 정책"]`).
- Code blocks: language tag preserved (` ```rust ` → `Some("rust")`), fenced content not split.
- Tables: GFM tables produce `TableBlock` with header row + body rows; if a table cell is malformed, fall back to a `Paragraph` block + warning.
- Image references: `![alt](src)` produces `ImageRefBlock` with `asset_id = None`, `src = "..."`, `alt = "..."`. Resolution to `AssetId` happens later in `kb-normalize`.
- Lists: ordered/unordered preserved; nested list items flattened into one `ListBlock` with each top-level item's text.
- Inline elements: only `Text`, `Code`, `Link`, `Strong`, `Emph` (per design §3.4). Drop other inlines silently.
- Malformed input never panics. Worst case: empty `Vec<ParsedBlock>` + warning.

## Storage / wire effects

- None.

## Test plan

| kind | description | fixture / data |
|------|-------------|----------------|
| unit | heading tree depth + heading_path correctness | inline |
| unit | code block lang tag preserved | inline |
| unit | GFM table parses; malformed table degrades to paragraph + warning | inline |
| unit | line range correct under various line-ending styles (LF / CRLF) | inline |
| unit | image ref captured with src/alt | inline |
| unit | nested list flattens correctly | inline |
| unit | malformed input does not panic | inline (random byte slices) |
| snapshot | `fixtures/markdown/nested-headings.md` → ParsedBlock JSON stable | fixture |
| snapshot | `fixtures/markdown/code-and-table.md` → JSON stable | fixture |

All tests under `cargo test -p kb-parse-md --lib blocks`.

## Definition of Done

- [ ] `cargo check -p kb-parse-md` passes
- [ ] `cargo test -p kb-parse-md blocks` passes
- [ ] Snapshot tests stable across two runs
- [ ] No imports outside Allowed dependencies
- [ ] PR links design §3.4

## Out of scope

- Frontmatter (p1-2).
- Lifting `ParsedBlock` → `kb_core::Block` with `BlockId` (p1-4).
- Chunking (p1-5).

## Risks / notes

- `pulldown-cmark` source-map may not include exact byte ranges for all event kinds; line ranges are the binding contract per design (line-range citation is the primary form for Markdown).
- CRLF normalization: convert internally to LF for span math but report line numbers from the original byte stream.
````

- [ ] **Step 2: Verify and commit**

```bash
test -s tasks/p1/p1-3-parse-md-blocks.md && echo OK
git add tasks/p1/p1-3-parse-md-blocks.md
git commit -m "tasks: add p1-3 parse-md blocks component spec"
```

### Task B4: `p1-4-normalize.md`

**Files:**
- Create: `tasks/p1/p1-4-normalize.md`

- [ ] **Step 1: Write the spec**

Write to `tasks/p1/p1-4-normalize.md`:

````markdown
---
phase: P1
component: kb-normalize
task_id: p1-4
title: "Lift parser output → CanonicalDocument with deterministic IDs"
status: planned
depends_on: [p1-2, p1-3]
unblocks: [p1-5, p1-6]
contract_source: ../../docs/superpowers/specs/2026-04-27-kb-final-form-design.md
contract_sections: [§3.4, §4 ID recipe, §3.6 Provenance]
---

# p1-4 — Lift to CanonicalDocument

## Goal

Combine `Metadata` (p1-2) + `Vec<ParsedBlock>` (p1-3) + `RawAsset` (p1-1) into a `CanonicalDocument` with deterministic `doc_id` and `block_id`s per design §4 recipe.

## Why now / why this size

Single responsibility: ID generation + struct assembly. Keeps `kb-parse-md` purely a parser and isolates the (security-critical) deterministic ID logic in one crate.

## Allowed dependencies

- `kb-core`
- `kb-config`
- `serde`
- `serde-json-canonicalizer` (canonical JSON for ID hashing)
- `blake3`
- `unicode-normalization` (NFC)
- `time`
- `thiserror`

## Forbidden dependencies

- `kb-source-fs`, `kb-parse-md` (consumed via plain types only — must not couple back), `kb-chunk`, `kb-store-*`, `kb-embed*`, `kb-search`, `kb-llm*`, `kb-rag`, `kb-tui`, `kb-desktop`

Note: this crate accepts `ParsedBlock` from `kb-parse-md` either by (a) exposing `ParsedBlock` as a `kb-core` type, or (b) `kb-parse-md` re-exporting via a public DTO. Pick (a): move `ParsedBlock` into `kb-core` so this task does not import `kb-parse-md`.

## Inputs

| input | type | source |
|-------|------|--------|
| `RawAsset` | `kb_core::RawAsset` | p1-1 |
| `Metadata` + frontmatter span + warnings | from p1-2 | parser caller |
| `Vec<ParsedBlock>` + warnings | from p1-3 | parser caller |
| `parser_version` | `kb_core::ParserVersion` | constant in `kb-parse-md` |

## Outputs

| output | type | downstream |
|--------|------|------------|
| `CanonicalDocument` | `kb_core::CanonicalDocument` | `kb-chunk`, `kb-store-sqlite` |

## Public surface (signatures only — no new types)

```rust
pub fn build_canonical_document(
    asset: &kb_core::RawAsset,
    metadata: kb_core::Metadata,
    blocks: Vec<kb_core::ParsedBlock>,
    parser_version: &kb_core::ParserVersion,
    warnings: Vec<Warning>,
) -> anyhow::Result<kb_core::CanonicalDocument>;

pub fn id_for_doc(workspace_path: &kb_core::WorkspacePath, asset: &kb_core::AssetId, parser_version: &kb_core::ParserVersion) -> kb_core::DocumentId;
pub fn id_for_block(doc: &kb_core::DocumentId, kind: &str, heading_path: &[String], ordinal: u32, span: &kb_core::SourceSpan) -> kb_core::BlockId;
```

## Behavior contract

- ID generation strictly follows design §4.2 (canonical JSON of tagged tuple, blake3 hex truncated to 32 chars).
- `block_id` ordinal: per `(heading_path, kind)` group, 0-based, in document order.
- All input strings normalized to NFC before hashing.
- POSIX path normalization applied to `workspace_path`.
- Unicode line endings normalized internally; `SourceSpan::Line` indices preserved as-is from p1-3.
- `Provenance` built with one event per pipeline stage encountered: `Discovered`, `Parsed`, `Normalized`. Warnings appended as `ProvenanceKind::Warning` with `note`.
- Determinism property test: same inputs → byte-identical `CanonicalDocument` JSON, including ID stability across runs.

## Storage / wire effects

- None.

## Test plan

| kind | description | fixture / data |
|------|-------------|----------------|
| unit | id_for_doc deterministic across 1000 runs | inline |
| unit | NFC vs NFD Korean inputs produce identical IDs | inline |
| unit | POSIX path with `./` and `//` collapse to same `doc_id` | inline |
| unit | block ordinal numbering inside same heading_path is correct | inline |
| unit | provenance contains Discovered/Parsed/Normalized in order | inline |
| snapshot | `fixtures/markdown/code-and-table.md` → CanonicalDocument JSON stable (incl. all IDs) | fixture |

All tests under `cargo test -p kb-normalize`.

## Definition of Done

- [ ] `cargo check -p kb-normalize` passes
- [ ] `cargo test -p kb-normalize` passes
- [ ] Determinism test runs ≥ 1000 iterations under 1 second
- [ ] No `kb-parse-md` import (consumed via `kb-core::ParsedBlock`)
- [ ] PR links design §4.2, §4.3

## Out of scope

- Chunking (p1-5).
- DB writes (p1-6).
- Block validation beyond what is needed to assign IDs (e.g., we do NOT verify image src exists on disk here).

## Risks / notes

- If ID recipe changes, all dependent records become stale. Treat any change to `id_for_doc`/`id_for_block` as a `parser_version` bump (design §9).
````

- [ ] **Step 2: Verify and commit**

```bash
test -s tasks/p1/p1-4-normalize.md && echo OK
git add tasks/p1/p1-4-normalize.md
git commit -m "tasks: add p1-4 normalize component spec"
```

### Task B5: `p1-5-chunk.md`

**Files:**
- Create: `tasks/p1/p1-5-chunk.md`

- [ ] **Step 1: Write the spec**

Write to `tasks/p1/p1-5-chunk.md`:

````markdown
---
phase: P1
component: kb-chunk
task_id: p1-5
title: "Markdown heading-aware chunker (md-heading-v1)"
status: planned
depends_on: [p1-4]
unblocks: [p1-6, p2-2, p3-2]
contract_source: ../../docs/superpowers/specs/2026-04-27-kb-final-form-design.md
contract_sections: [§3.5 Chunk, §4.2 chunk_id recipe, §7.2 Chunker, §0 Q3 citation]
---

# p1-5 — Markdown heading-aware chunker

## Goal

Implement `Chunker` trait emitting `chunker_version = "md-heading-v1"`. Block-aware: respect heading boundaries, never split code/table, propagate `heading_path` and merged `source_spans`.

## Why now / why this size

The first concrete `Chunker`. Establishes how subsequent chunkers (PDF page chunker, audio segment chunker) are scoped: per-medium chunker version label. Independent of any store/embed.

## Allowed dependencies

- `kb-core`
- `kb-config`
- `serde`
- `blake3` (policy_hash)
- `serde-json-canonicalizer`
- `thiserror`

## Forbidden dependencies

- `kb-source-fs`, `kb-parse-md`, `kb-normalize` (consumes `CanonicalDocument` only via `kb-core`), `kb-store-*`, `kb-embed*`, `kb-search`, `kb-llm*`, `kb-rag`, `kb-tui`, `kb-desktop`

## Inputs

| input | type | source |
|-------|------|--------|
| `CanonicalDocument` | `kb_core::CanonicalDocument` | p1-4 |
| `ChunkPolicy` | `kb_core::ChunkPolicy` | `kb-app` from config |

## Outputs

| output | type | downstream |
|--------|------|------------|
| `Vec<Chunk>` | `kb_core::Chunk` | `kb-store-sqlite` (p1-6), `kb-embed*` (P3) |

## Public surface (signatures only — no new types)

```rust
pub struct MdHeadingV1Chunker;

impl kb_core::Chunker for MdHeadingV1Chunker {
    fn chunker_version(&self) -> kb_core::ChunkerVersion;
    fn policy_hash(&self, policy: &kb_core::ChunkPolicy) -> String;
    fn chunk(&self, doc: &kb_core::CanonicalDocument, policy: &kb_core::ChunkPolicy) -> anyhow::Result<Vec<kb_core::Chunk>>;
}
```

`policy_hash` = `blake3(canonical_json(policy))` hex truncated to 16 chars.

## Behavior contract

- Priority order (per design §0 / report §14):
  1. heading boundary first
  2. never split a code block
  3. table stays in a single chunk if possible
  4. long sections split by paragraph
  5. propagate `heading_path` from blocks
  6. carry merged `source_spans` (each chunk lists every contributing block's span)
  7. record `chunker_version = "md-heading-v1"` and `policy_hash`
- `target_tokens` and `overlap_tokens` from `ChunkPolicy`. Token estimate is byte-based proxy until a real tokenizer is introduced (note in `Chunk.token_estimate`).
- `chunk_id` per design §4.2: tagged tuple of `(doc_id, chunker_version, block_ids, policy_hash)`.
- `block_ids` listed in document order (significant — affects ID).
- ImageRef / AudioRef blocks are emitted as their own chunks (text portion = alt + caption preview if present, else empty string with `token_estimate=0`). They still receive `chunk_id` so future image/audio search can locate them.

## Storage / wire effects

- None directly. Outputs feed p1-6.

## Test plan

| kind | description | fixture / data |
|------|-------------|----------------|
| unit | heading boundary respected (no chunk crosses H2 → H2) | inline |
| unit | code block of 800 tokens stays in one chunk even when target=500 | inline |
| unit | table block stays single chunk if size < 2× target | inline |
| unit | long paragraph split with overlap_tokens applied | inline |
| unit | ImageRefBlock produces a chunk with token_estimate=0 | inline |
| determinism | identical input + identical policy → identical chunk_ids | inline |
| snapshot | `fixtures/markdown/long-section.md` → Vec<Chunk> JSON stable | fixture |

All tests under `cargo test -p kb-chunk`.

## Definition of Done

- [ ] `cargo check -p kb-chunk` passes
- [ ] `cargo test -p kb-chunk` passes
- [ ] Snapshot stable across two runs
- [ ] No imports outside Allowed dependencies
- [ ] PR links design §3.5, §4.2

## Out of scope

- DB persistence (p1-6).
- Embedding (P3).
- Reranking / hybrid (P3).

## Risks / notes

- Token estimate proxy: a real tokenizer (e.g., sentencepiece for the embedding model) replaces this in P3. The proxy must err toward overestimation so chunks fit in real tokenizer budget.
- Changing `chunker_version` invalidates all downstream embedding records. Bump only with PR documenting the migration plan (design §9).
````

- [ ] **Step 2: Verify and commit**

```bash
test -s tasks/p1/p1-5-chunk.md && echo OK
git add tasks/p1/p1-5-chunk.md
git commit -m "tasks: add p1-5 chunk component spec"
```

### Task B6: `p1-6-store-sqlite.md`

**Files:**
- Create: `tasks/p1/p1-6-store-sqlite.md`

- [ ] **Step 1: Write the spec**

Write to `tasks/p1/p1-6-store-sqlite.md`:

````markdown
---
phase: P1
component: kb-store-sqlite (P1 subset)
task_id: p1-6
title: "SQLite store: assets/documents/blocks/chunks + asset writer + migrations"
status: planned
depends_on: [p1-1, p1-4, p1-5]
unblocks: [p2-1, p3-3, p4-3]
contract_source: ../../docs/superpowers/specs/2026-04-27-kb-final-form-design.md
contract_sections: [§5 DDL (5.1, 5.2, 5.3, 5.4, 5.5 chunks only — FTS handled in p2-1), §5.7 jobs/ingest_runs, §5.8 transactions, §6.3 data_dir layout]
---

# p1-6 — SQLite store (P1 subset)

## Goal

Persist `RawAsset`, `CanonicalDocument`, `Block`s, `Chunk`s into SQLite per design §5; copy raw asset bytes into `data_dir/assets/<aa>/<asset_id>` (or reference if larger than threshold); record an `ingest_runs` row.

## Why now / why this size

P1's terminal task. Closes the loop `walk → parse → chunk → store`. The FTS5 virtual table and triggers are intentionally deferred to p2-1 to keep this task focused on the relational schema and asset I/O.

## Allowed dependencies

- `kb-core`
- `kb-config`
- `rusqlite` (with `bundled-sqlcipher` disabled; use `bundled` feature)
- `refinery` for migrations
- `serde_json`
- `time`
- `blake3` (asset copy verification)
- `tracing`
- `thiserror`

## Forbidden dependencies

- `kb-source-fs` (only types via `kb-core`), `kb-parse-md`, `kb-normalize`, `kb-chunk` (only types via `kb-core`), `kb-store-vector`, `kb-embed*`, `kb-search`, `kb-llm*`, `kb-rag`, `kb-tui`, `kb-desktop`

## Inputs

| input | type | source |
|-------|------|--------|
| migrations | `migrations/V001__init.sql` | repo |
| `RawAsset` + bytes | `(RawAsset, Vec<u8>)` | p1-1 + reader |
| `CanonicalDocument` | `kb_core::CanonicalDocument` | p1-4 |
| `Vec<Chunk>` | `kb_core::Chunk` | p1-5 |
| `IngestRun` aggregates | `(scope, counts, duration)` | `kb-app` |

## Outputs

| output | type | downstream |
|--------|------|------------|
| `data_dir/kb.sqlite` rows in `assets`, `documents`, `blocks`, `chunks`, `document_tags`, `ingest_runs`, `jobs`, `schema_meta`, `migrations` | – | every later phase |
| `data_dir/assets/<aa>/<asset_id>` bytes (when copied) | – | future re-extraction, integrity verification |
| `IngestReport` (wire schema v1) | `kb_core::IngestReport` | `kb-cli`, eval |

## Public surface (signatures only — no new types)

```rust
pub struct SqliteStore { /* internal */ }

impl SqliteStore {
    pub fn open(config: &kb_config::Config) -> anyhow::Result<Self>;
    pub fn run_migrations(&self) -> anyhow::Result<()>;

    pub fn put_asset_with_bytes(&self, asset: &kb_core::RawAsset, bytes: &[u8]) -> anyhow::Result<()>;
}

impl kb_core::DocumentStore for SqliteStore {
    fn put_asset(&self, a: &kb_core::RawAsset) -> anyhow::Result<()>;
    fn put_document(&self, d: &kb_core::CanonicalDocument) -> anyhow::Result<()>;
    fn put_blocks(&self, doc: &kb_core::DocumentId, blocks: &[kb_core::Block]) -> anyhow::Result<()>;
    fn put_chunks(&self, doc: &kb_core::DocumentId, chunks: &[kb_core::Chunk]) -> anyhow::Result<()>;
    fn get_document(&self, id: &kb_core::DocumentId) -> anyhow::Result<Option<kb_core::CanonicalDocument>>;
    fn get_chunk(&self, id: &kb_core::ChunkId) -> anyhow::Result<Option<kb_core::Chunk>>;
    fn list_documents(&self, filter: &kb_core::DocFilter) -> anyhow::Result<Vec<kb_core::DocSummary>>;
}

impl kb_core::JobRepo for SqliteStore { /* per design §7.2 signatures */ }
```

## Behavior contract

- DDL: `migrations/V001__init.sql` ships exactly the SQL in design §5.1, §5.2, §5.3, §5.4, §5.5 (chunks table only — FTS table & triggers come in p2-1 as `V002`), §5.7 jobs/ingest_runs/answers/eval_runs/eval_query_results, §5.6 embedding_records.
- Pragmas at open: `foreign_keys=ON`, `journal_mode=WAL`, `synchronous=NORMAL`, `temp_store=MEMORY`.
- One ingest of one document = one transaction (BEGIN..COMMIT). Partial failures roll back; warnings are not failures.
- Bulk ingest commits per-document.
- Asset writer:
  - if `asset.byte_len <= storage.copy_threshold_mb * 1_048_576`: write bytes to `assets_dir/<asset_id[..2]>/<asset_id>` (mode 0o644), record `storage_kind='copied'`.
  - else: do not copy; record `storage_kind='reference'` with `storage_path = asset.source_uri`'s file path.
  - In either case, recompute `blake3` of the source bytes once on write/verify and store in `assets.checksum`. Mismatch → return `StoreError::Conflict`.
- Idempotency: re-ingesting the same `(workspace_path, asset_id, parser_version)` updates `documents.updated_at`, increments `doc_version`, replaces blocks/chunks. No row duplication.
- `document_tags`: re-derived from `Metadata.tags` on each put.
- `ingest_runs.items_json` is null when caller passes `summary_only=true`.
- All wire JSON returned (`IngestReport`) conforms to `docs/wire-schema/v1/ingest_report.schema.json`. Fail loudly if schema not present (caller must vendor it).

## Storage / wire effects

- Writes: `kb.sqlite` (multiple tables), `data_dir/assets/<aa>/<asset_id>` (copied case).
- Reads on subsequent calls: same DB.

## Test plan

| kind | description | fixture / data |
|------|-------------|----------------|
| migration | fresh DB after `run_migrations` has all P1 tables and indexes | tmp dir |
| unit | put_asset_with_bytes copy mode writes file with correct mode and bytes | tmp dir |
| unit | put_asset_with_bytes reference mode does not write file but records path | tmp dir + large fake size |
| unit | checksum mismatch returns Conflict error | tmp dir + tampered bytes |
| unit | put_document idempotency: same input twice → 1 row, doc_version bumped | tmp dir |
| unit | put_blocks + put_chunks transactional rollback on simulated failure | tmp dir |
| contract | DocumentStore trait round-trip for fixture document | `fixtures/markdown/code-and-table.md` |
| snapshot | IngestReport JSON for fixture run | fixture |

All tests under `cargo test -p kb-store-sqlite` with no network.

## Definition of Done

- [ ] `cargo check -p kb-store-sqlite` passes
- [ ] `cargo test -p kb-store-sqlite` passes
- [ ] migration `V001__init.sql` matches design §5 verbatim (diff-checked in CI)
- [ ] Writes to `~/.local/share/kb/` are gated by `kb-config`'s `data_dir` and never escape it
- [ ] No imports outside Allowed dependencies
- [ ] PR links design §5

## Out of scope

- FTS5 virtual table and triggers (p2-1).
- Vector store (p3-3).
- Embedding records writer (p3-2).
- Search queries (p2-2).

## Risks / notes

- WAL mode requires careful test cleanup: tests must drop the connection before removing `kb.sqlite-wal` / `-shm`.
- Asset directory shard prefix uses `asset_id[..2]`; using `asset_id[..1]` would create at most 16 dirs (insufficient).
````

- [ ] **Step 2: Verify and commit**

```bash
test -s tasks/p1/p1-6-store-sqlite.md && echo OK
git add tasks/p1/p1-6-store-sqlite.md
git commit -m "tasks: add p1-6 store-sqlite component spec"
```

### Task B7: Validate Phase B output

- [ ] **Step 1: Confirm 6 P1 specs exist and reference the design doc**

```bash
ls tasks/p1/ | sort
for f in tasks/p1/p1-*.md; do grep -q '2026-04-27-kb-final-form-design.md' "$f" || echo "MISSING REF in $f"; done
echo done
```

Expected: lists `p1-1-source-fs.md` … `p1-6-store-sqlite.md`, no `MISSING REF` lines, ends with `done`.

- [ ] **Step 2: Confirm Allowed/Forbidden sections present in every spec**

```bash
for f in tasks/p1/p1-*.md; do
  grep -q '^## Allowed dependencies' "$f"  || echo "no Allowed in $f"
  grep -q '^## Forbidden dependencies' "$f" || echo "no Forbidden in $f"
done
echo done
```

Expected: only `done` printed.

- [ ] **Step 3: Pause for user review**

Stop here. Wait for the user to skim `tasks/p1/*.md` and approve before Phase C kicks off. Phase C reuses Phase B's template shape, so a template-shape correction is cheaper now than after 24 more files.

---

## Phase C — Decompose remaining phases (P0, P2..P9)

For each component task below, the steps are: (1) write file, (2) verify, (3) commit. Body content follows the same template skeleton as Phase B (frontmatter + Goal + Why + Allowed/Forbidden + Inputs + Outputs + Public surface + Behavior contract + Storage/wire + Test plan + DoD + Out of scope + Risks). Each task **must** cite the listed `contract_sections` of the frozen design doc.

### Task C1: `tasks/p0/p0-1-skeleton.md`

**Files:** Create: `tasks/p0/p0-1-skeleton.md`. Also `mkdir -p tasks/p0`.

`contract_sections`: §3 (all subsections), §4, §5 (migrations meta only), §6, §7, §8, §10. `Allowed`: workspace + `kb-core` + `kb-config` + `kb-app` + `kb-cli` only.

Body covers: workspace `Cargo.toml` resolver=3, edition 2024, member list (`kb-core`, `kb-config`, `kb-app`, `kb-cli`), workspace dependencies, `kb-core` types and traits per design §3 / §7, deterministic ID functions per §4 (with full unit tests), `kb-config` loader (TOML + env + CLI override per §6.4), `kb-app` facade signatures (`ingest`, `search`, `ask`, `inspect_doc`, `inspect_chunk`, `doctor`, `init`), `kb-cli` skeleton with clap + `--help`. DoD: `cargo check --workspace`, `cargo test --workspace` (Newtype+ID+canonical-json tests only), `kb --help` works, `docs/spec/*` stubs created (link to frozen design doc), `docs/wire-schema/v1/*.schema.json` stubs (one file per object in §2).

- [ ] **Step 1: Create directory and file**

```bash
mkdir -p tasks/p0
```

- [ ] **Step 2: Write the spec using the Phase B template, with the contents described above**

Use the Phase B Task B1 (`p1-1-source-fs.md`) file as the structural template. Replace fields: `phase: P0`, `task_id: p0-1`, `component: workspace`, `Allowed` and `Forbidden` per §8, `Inputs/Outputs` per §6/§7, `Public surface` listing every type and trait from §3 and §7, `Behavior contract` covering ID determinism + facade boundary, `Test plan` running ID determinism (1000 iters), Newtype Display/FromStr round-trip, canonical_json snapshot. `Out of scope`: anything that needs another crate added.

- [ ] **Step 3: Verify and commit**

```bash
test -s tasks/p0/p0-1-skeleton.md && echo OK
git add tasks/p0 tasks/INDEX.md
git -c user.name=kb -c user.email=kb@local commit -m "tasks: add p0-1 skeleton component spec"
```

### Task C2: `tasks/p2/` (P2 — 2 specs)

**Files:** Create: `tasks/p2/p2-1-fts-schema.md`, `tasks/p2/p2-2-lexical-retriever.md`. `mkdir -p tasks/p2`.

#### `p2-1-fts-schema.md`

`contract_sections`: §5.5 FTS5 + triggers, §9 versioning. `Allowed`: `kb-core`, `kb-config`, `kb-store-sqlite` (extends migrations). `depends_on: [p1-6]`. Migration `V002__fts.sql` adds `chunks_fts` virtual table and three triggers verbatim from §5.5. Tests: backfill from existing chunks via `INSERT INTO chunks_fts SELECT ... FROM chunks`, then assert FTS row count == chunks row count; insert/update/delete in `chunks` reflects in `chunks_fts`.

#### `p2-2-lexical-retriever.md`

`contract_sections`: §3.7 SearchQuery/Hit, §0 Q3 citation (URI fragment), §1.5 search output (for snippet length defaults), §2.2 wire schema. `Allowed`: `kb-core`, `kb-config`, `kb-store-sqlite`. `depends_on: [p2-1]`. Implements `Retriever` trait with `bm25(chunks_fts)` ranking, snippet via SQLite `snippet()` (≤ `snippet_chars` chars), citation built per §0 Q3 from `source_spans`. Tests: top-k correctness on fixture corpus, citation line range round-trip against original Markdown, deterministic across two runs.

- [ ] **Step 1: Create directory and both spec files** (template per Phase B; bodies as described above).
- [ ] **Step 2: Verify with `for f in tasks/p2/p2-*.md; do test -s "$f" || echo MISSING $f; done; echo done` (expect only `done`).**
- [ ] **Step 3: Commit**

```bash
git add tasks/p2 && git -c user.name=kb -c user.email=kb@local commit -m "tasks: add P2 component specs (fts-schema, lexical-retriever)"
```

### Task C3: `tasks/p3/` (P3 — 4 specs)

**Files:** `mkdir -p tasks/p3` and create `p3-1-embedder-trait.md`, `p3-2-fastembed-adapter.md`, `p3-3-lancedb-store.md`, `p3-4-hybrid-fusion.md`.

- `p3-1-embedder-trait.md`: §3.7, §7.2 Embedder, §11. Allowed: `kb-core`, `kb-config`. No external embedding dep. Public surface: `Embedder` trait + `EmbeddingInput`/`EmbeddingKind` already in core (validate they exist; if not, this task is also a `kb-core` patch). `depends_on: [p0-1]`. Tests: trait dyn dispatch, mock embedder.
- `p3-2-fastembed-adapter.md`: §11.3, §6.4 `[models.embedding]`. Allowed: `kb-core`, `kb-config`, `fastembed`, `tokenizers`, `ort`. `depends_on: [p3-1]`. Provides `FastembedEmbedder` implementing `Embedder` for `multilingual-e5-small` (default), with required Document/Query prefix per §11.3. Tests: dimension check, deterministic vector for fixed input (hash compare on first 8 floats with epsilon), batch size respected.
- `p3-3-lancedb-store.md`: §3.5, §5.6 embedding_records, §6.3 lancedb table naming. Allowed: `kb-core`, `kb-config`, `lancedb`, `arrow`, `kb-store-sqlite` (write `embedding_records` row only — no other table). `depends_on: [p3-2, p1-6]`. Implements `VectorStore` trait. Table naming `chunk_embeddings_<model>_<dim>.lance`. `ensure_table` creates if missing. `upsert` inserts vectors and writes a matching `embedding_records` row in same logical operation (best-effort 2PC: lance commit, then SQLite insert; on SQLite failure, log warning + leave lance row — re-upsert is idempotent because of the `UNIQUE(chunk_id, model_id, model_version, dimensions)` constraint and lance upsert semantics). `search` filters via SearchFilters and returns top-k. Tests: smoke (insert+search), dimension mismatch error, model isolation (two models stay in two tables).
- `p3-4-hybrid-fusion.md`: §3.7 RetrievalDetail, §0 Q3, §1.6 search --explain, §6.4 `[search]` rrf settings. Allowed: `kb-core`, `kb-config`, `kb-store-sqlite` (lexical Retriever from p2-2), `kb-store-vector` (vector Retriever wrapper around `VectorStore::search`). `depends_on: [p2-2, p3-3]`. Implements `HybridRetriever` that dispatches by `SearchMode`, fuses with RRF (k from config, default 60), populates `lexical_score`, `vector_score`, `lexical_rank`, `vector_rank`, `fusion_score`. Tests: pure lexical mode == p2-2 output; pure vector mode == p3-3 output; hybrid produces strictly larger or equal coverage of expected hits than either single mode on a small fixture; deterministic.

- [ ] **Step 1: Create directory and 4 files** (template per Phase B).
- [ ] **Step 2: Verify**

```bash
for f in tasks/p3/p3-*.md; do test -s "$f" || echo MISSING $f; done; echo done
```

Expect only `done`.

- [ ] **Step 3: Commit**

```bash
git add tasks/p3 && git -c user.name=kb -c user.email=kb@local commit -m "tasks: add P3 component specs (embedder, fastembed, lancedb, hybrid)"
```

### Task C4: `tasks/p4/` (P4 — 3 specs)

**Files:** `mkdir -p tasks/p4` and create `p4-1-llm-trait.md`, `p4-2-ollama-adapter.md`, `p4-3-rag-pipeline.md`.

- `p4-1-llm-trait.md`: §7.2 LanguageModel + TokenChunk, §0 Q5 streaming, §3.8 Answer types referenced. Allowed: `kb-core`, `kb-config`. `depends_on: [p0-1]`. Defines (or validates) `LanguageModel` trait, `GenerateRequest`, `TokenChunk`, `FinishReason`, `TokenUsage` per design. Tests: trait dyn dispatch, mock LM streams 3 tokens.
- `p4-2-ollama-adapter.md`: §11.2 Ollama, §6.4 `[models.llm]`, §0 Q5 streaming. Allowed: `kb-core`, `kb-config`, `reqwest` (blocking + json + stream feature) or `ureq` + manual SSE; `serde_json`, `tokio`/runtime if needed. `depends_on: [p4-1]`. Implements `OllamaLanguageModel` with streaming `/api/generate`. `temperature=0.0` default, `seed` honored for determinism. Reachability/missing-model errors map to `LlmError` per design §10. Tests: against a mock HTTP server (`wiremock` or hand-rolled `tiny_http`); deterministic stream collect equals buffered concatenation; missing model returns `LlmError::ModelNotPulled` with proper hint.
- `p4-3-rag-pipeline.md`: §0 Q4 refusal (two-layer), §0 Q7 footer, §1.1–1.4 ask scenes, §2.3 Answer wire, §3.8 internal Answer, §6.4 `[rag]`. Allowed: `kb-core`, `kb-config`, `kb-search` (Retriever), `kb-llm` (LanguageModel). `depends_on: [p3-4, p4-2]`. Pipeline: retrieve top-k → score gate (`refusal_reason: ScoreGate` if top1 < gate) → context packer (token budget + heading_path header `[#n doc=… heading=… span=…]`) → render `rag-v1` prompt → stream → collect → citation extraction (regex `\[(\d+)\]`) → citation validation (each `[n]` must map to a packed chunk; otherwise `grounded=false`, `refusal_reason: LlmSelfJudge`) → write `answers` row. Tests: happy path produces grounded Answer with citations; query with all chunks below gate produces ScoreGate refusal; query whose LLM emits a citation pointing to non-existent `[7]` becomes LlmSelfJudge refusal; identical query under temperature=0 produces byte-identical Answer (snapshot).

- [ ] **Step 1, 2, 3** as in C3.

```bash
mkdir -p tasks/p4
# write three files
for f in tasks/p4/p4-*.md; do test -s "$f" || echo MISSING $f; done; echo done
git add tasks/p4 && git -c user.name=kb -c user.email=kb@local commit -m "tasks: add P4 component specs (llm-trait, ollama, rag-pipeline)"
```

### Task C5: `tasks/p5/` (P5 — 2 specs)

**Files:** `mkdir -p tasks/p5` and create `p5-1-golden-fixture-runner.md`, `p5-2-metrics-compare.md`.

- `p5-1-golden-fixture-runner.md`: phase epic + §5.7 eval_runs/eval_query_results, §6.3 runs_dir. Allowed: `kb-core`, `kb-config`, `kb-app` (calls facade for search/ask), `serde_yaml`. `depends_on: [p4-3]`. Loads `fixtures/golden_queries.yaml`, runs each query in selected mode (lexical/vector/hybrid/rag), captures per-query results to `eval_query_results` and to `runs_dir/<run_id>/per_query.jsonl`. Tests: fixture with 3 queries runs end-to-end on a tiny corpus, all rows recorded.
- `p5-2-metrics-compare.md`: phase epic, §0 Q6 wire schema. Allowed: `kb-core`, `kb-config`, `kb-store-sqlite` (read eval rows). `depends_on: [p5-1]`. Computes hit@k, MRR, recall@k_doc, citation_coverage, groundedness (rule-based via `must_contain`), empty_result_rate, refusal_correctness. `kb eval compare a b` produces wins/losses/draws + delta. Tests: fixed input rows produce expected metric values; compare produces stable sorted output.

- [ ] **Step 1, 2, 3** as in C3.

```bash
mkdir -p tasks/p5
# write two files
for f in tasks/p5/p5-*.md; do test -s "$f" || echo MISSING $f; done; echo done
git add tasks/p5 && git -c user.name=kb -c user.email=kb@local commit -m "tasks: add P5 component specs (runner, metrics)"
```

### Task C6: `tasks/p6/` (P6 — 3 specs)

`mkdir -p tasks/p6` and create:

- `p6-1-image-extractor-exif.md`: phase epic §9.1, §3.4 ImageRefBlock, §3.7a ImageType. Allowed: `kb-core`, `kb-config`, `image`, `kamadak-exif`. Implements `Extractor` for `MediaType::Image(_)` producing a `CanonicalDocument` whose body is exactly one `ImageRefBlock`. EXIF goes to `metadata.user`. `depends_on: [p0-1, p1-6]`. Tests: PNG/JPEG decode metadata; EXIF extraction; deterministic doc_id.
- `p6-2-ocr-adapter.md`: phase epic §9.1. Allowed: `kb-core`, `kb-config`, `image`, OS-specific OCR (feature `apple-vision` for macOS via sidecar binary; feature `tesseract` for cross-platform; default tesseract). Defines `OcrEngine` trait + adapter. Populates `ImageRefBlock.ocr` `OcrText` (`joined`, regions, engine, engine_version). `depends_on: [p6-1]`. Tests: deterministic text on a fixed fixture image with high-confidence text.
- `p6-3-caption-adapter.md`: phase epic §9.1 caption section, §3.7a ModelCaption. Allowed: `kb-core`, `kb-config`, `kb-llm` (reuse LanguageModel for VLM). Optional/feature-gated. `depends_on: [p6-1, p4-2]`. Populates `ImageRefBlock.caption`. Tests: with mock LM, caption recorded with model id; absence of feature flag leaves caption=None.

- [ ] **Step 1, 2, 3** as in C3.

```bash
mkdir -p tasks/p6
# write three files
for f in tasks/p6/p6-*.md; do test -s "$f" || echo MISSING $f; done; echo done
git add tasks/p6 && git -c user.name=kb -c user.email=kb@local commit -m "tasks: add P6 component specs (image-exif, ocr, caption)"
```

### Task C7: `tasks/p7/` (P7 — 2 specs)

`mkdir -p tasks/p7` and create:

- `p7-1-pdf-text-extractor.md`: phase epic §9.2, §3.4 SourceSpan::Page. Allowed: `kb-core`, `kb-config`, `pdf-extract`, `lopdf` (page metadata). `depends_on: [p0-1, p1-6]`. Extractor for `MediaType::Pdf` produces a `CanonicalDocument` with one `Paragraph` per page, `SourceSpan::Page`. Failed-text pages are emitted as paragraphs with empty text and a `Provenance` warning marking them as scanned candidates. Tests: page count, span correctness, failure handling.
- `p7-2-pdf-page-chunker.md`: phase epic §9.2, §3.5, §0 Q3 citation. Allowed: `kb-core`, `kb-config`. New chunker version `pdf-page-v1` that respects page boundaries. `depends_on: [p7-1]`. Tests: chunk does not cross page boundary; very long page subdivides per `target_tokens`.

- [ ] **Step 1, 2, 3** as in C3.

```bash
mkdir -p tasks/p7
# write two files
for f in tasks/p7/p7-*.md; do test -s "$f" || echo MISSING $f; done; echo done
git add tasks/p7 && git -c user.name=kb -c user.email=kb@local commit -m "tasks: add P7 component specs (pdf-extractor, pdf-chunker)"
```

### Task C8: `tasks/p8/` (P8 — 2 specs)

`mkdir -p tasks/p8` and create:

- `p8-1-whisper-adapter.md`: phase epic §9.3, §3.4 AudioRefBlock + `Transcript`. Allowed: `kb-core`, `kb-config`, whisper.cpp Rust binding (`whisper-rs`) or sidecar binary. `depends_on: [p0-1, p1-6]`. Implements `Transcriber` trait. Default model `large-v3` via config; tests use a tiny model (e.g., `base.en`) for speed. Tests: monotone segment timestamps, language detection populated, deterministic transcript on fixed audio.
- `p8-2-segment-chunker.md`: phase epic §9.3, §3.5. New `audio-segment-v1` chunker that groups segments up to `target_tokens` with priority on speaker turn boundaries (when present). `depends_on: [p8-1]`. Tests: chunk timestamp == first/last segment timestamp; speaker change forces split.

- [ ] **Step 1, 2, 3** as in C3.

```bash
mkdir -p tasks/p8
# write two files
for f in tasks/p8/p8-*.md; do test -s "$f" || echo MISSING $f; done; echo done
git add tasks/p8 && git -c user.name=kb -c user.email=kb@local commit -m "tasks: add P8 component specs (whisper, audio-chunker)"
```

### Task C9: `tasks/p9/` (P9 — 5 specs)

`mkdir -p tasks/p9` and create:

- `p9-1-tui-library.md`: phase epic §16.2, §3.7. Allowed: `kb-core`, `kb-app` only (UI law). `ratatui`, `crossterm`. `depends_on: [p1-6]`. Library list view + tag filter. Tests: snapshot of rendered frame against fixture corpus list.
- `p9-2-tui-search.md`: phase epic §16.2, §1.5. Allowed: same as p9-1. `depends_on: [p2-2, p3-4]`. Search input + result list + preview pane; `Enter` triggers external editor jump (`$EDITOR +<line> <path>`). Tests: search results render; `g` keybinding constructs the correct editor command.
- `p9-3-tui-ask.md`: phase epic §16.2, §1.1, §1.2. Allowed: same. `depends_on: [p4-3]`. Ask pane shows streaming tokens; `--explain` toggle. Tests: streaming render, refusal render.
- `p9-4-tui-inspect.md`: §1.6 inspect, §3.5. Allowed: same. `depends_on: [p1-6, p3-3]`. Renders Document and Chunk inspection per wire schemas 2.5/2.6.
- `p9-5-desktop-tauri.md`: phase epic §16.3, §1 all scenes. Allowed: `kb-core`, `kb-app`, Tauri backend; frontend stack TBD by user (vanilla TS by default). `depends_on: [p9-1, p9-2, p9-3, p9-4]`. Backend exposes Tauri commands that wrap `kb-app` 1:1. Source viewer per medium (Markdown render, PDF page, image with region overlay, audio with seek). Tests: backend command unit tests (no frontend e2e in this task).

- [ ] **Step 1, 2, 3** as in C3.

```bash
mkdir -p tasks/p9
# write five files
for f in tasks/p9/p9-*.md; do test -s "$f" || echo MISSING $f; done; echo done
git add tasks/p9 && git -c user.name=kb -c user.email=kb@local commit -m "tasks: add P9 component specs (tui x4, desktop)"
```

### Task C10: Final INDEX update

**Files:** Modify: `tasks/INDEX.md` — extend the "Component task decomposition" subsection added in B0 to list every phase.

- [ ] **Step 1: Replace the subsection added in B0 with the full list**

The new subsection reads:

```markdown
## Component task decomposition (per phase)

각 phase 의 component-level 분해. AI sub-agent 1세션 = 1 task 가 sweet spot.

- P0 — [p0/](p0/) — 1 component
  - [p0-1 skeleton](p0/p0-1-skeleton.md)
- P1 — [p1/](p1/) — 6 components
  - [p1-1 source-fs](p1/p1-1-source-fs.md)
  - [p1-2 parse-md frontmatter](p1/p1-2-parse-md-frontmatter.md)
  - [p1-3 parse-md blocks](p1/p1-3-parse-md-blocks.md)
  - [p1-4 normalize](p1/p1-4-normalize.md)
  - [p1-5 chunk](p1/p1-5-chunk.md)
  - [p1-6 store-sqlite](p1/p1-6-store-sqlite.md)
- P2 — [p2/](p2/) — 2 components
  - [p2-1 fts-schema](p2/p2-1-fts-schema.md)
  - [p2-2 lexical-retriever](p2/p2-2-lexical-retriever.md)
- P3 — [p3/](p3/) — 4 components
  - [p3-1 embedder-trait](p3/p3-1-embedder-trait.md)
  - [p3-2 fastembed-adapter](p3/p3-2-fastembed-adapter.md)
  - [p3-3 lancedb-store](p3/p3-3-lancedb-store.md)
  - [p3-4 hybrid-fusion](p3/p3-4-hybrid-fusion.md)
- P4 — [p4/](p4/) — 3 components
  - [p4-1 llm-trait](p4/p4-1-llm-trait.md)
  - [p4-2 ollama-adapter](p4/p4-2-ollama-adapter.md)
  - [p4-3 rag-pipeline](p4/p4-3-rag-pipeline.md)
- P5 — [p5/](p5/) — 2 components
  - [p5-1 golden-fixture-runner](p5/p5-1-golden-fixture-runner.md)
  - [p5-2 metrics-compare](p5/p5-2-metrics-compare.md)
- P6 — [p6/](p6/) — 3 components
  - [p6-1 image-extractor-exif](p6/p6-1-image-extractor-exif.md)
  - [p6-2 ocr-adapter](p6/p6-2-ocr-adapter.md)
  - [p6-3 caption-adapter](p6/p6-3-caption-adapter.md)
- P7 — [p7/](p7/) — 2 components
  - [p7-1 pdf-text-extractor](p7/p7-1-pdf-text-extractor.md)
  - [p7-2 pdf-page-chunker](p7/p7-2-pdf-page-chunker.md)
- P8 — [p8/](p8/) — 2 components
  - [p8-1 whisper-adapter](p8/p8-1-whisper-adapter.md)
  - [p8-2 segment-chunker](p8/p8-2-segment-chunker.md)
- P9 — [p9/](p9/) — 5 components
  - [p9-1 tui-library](p9/p9-1-tui-library.md)
  - [p9-2 tui-search](p9/p9-2-tui-search.md)
  - [p9-3 tui-ask](p9/p9-3-tui-ask.md)
  - [p9-4 tui-inspect](p9/p9-4-tui-inspect.md)
  - [p9-5 desktop-tauri](p9/p9-5-desktop-tauri.md)
```

- [ ] **Step 2: Verify total count**

```bash
ls tasks/p?/p*-*.md | wc -l
```

Expected: `30` (= 1 + 6 + 2 + 4 + 3 + 2 + 3 + 2 + 2 + 5).

- [ ] **Step 3: Commit**

```bash
git add tasks/INDEX.md
git -c user.name=kb -c user.email=kb@local commit -m "tasks: update INDEX with full component-task tree (30 specs)"
```

---

## Final acceptance

- [ ] `tasks/_template.md` exists.
- [ ] `tasks/p0/`, `tasks/p1/` … `tasks/p9/` exist with the component spec files listed above.
- [ ] Every component spec contains: frontmatter (with `contract_sections`), Allowed/Forbidden, Inputs, Outputs, Public surface, Behavior contract, Storage/wire effects, Test plan, Definition of Done, Out of scope, Risks/notes.
- [ ] `tasks/INDEX.md` lists every component task.
- [ ] No new domain types introduced inside any component spec — every type referenced is defined in [docs/superpowers/specs/2026-04-27-kb-final-form-design.md](../specs/2026-04-27-kb-final-form-design.md).
- [ ] All commits authored sequentially per task; rollback is per-task.
