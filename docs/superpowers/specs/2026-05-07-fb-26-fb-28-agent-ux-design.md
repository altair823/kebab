---
date: 2026-05-07
tasks: [p9-fb-26, p9-fb-28, claude-md-schema-sync]
title: "Agent UX improvements: ingest log consistency + invocation flags + schema list sync"
status: approved
target_version: 0.3.3
branch: feat/p9-fb-26-fb-28-agent-ux
---

# Agent UX improvements

Three bundled changes shipped as one PR: CLAUDE.md wire schema list sync (doc-only), ingest log consistency fix (fb-26), and agent invocation flags (fb-28).

---

## §1 — CLAUDE.md wire schema list sync

### Problem

`CLAUDE.md` §Wire schema v1 lists schemas that do not exist on disk, and omits schemas that do.

| Schema | CLAUDE.md | `docs/wire-schema/v1/` |
|---|---|---|
| `eval_run.v1` | listed | **missing** |
| `eval_compare.v1` | listed | **missing** |
| `list_docs.v1` | listed | **missing** |
| `chunk_inspection.v1` | **absent** | present |
| `citation.v1` | **absent** | present |
| `doc_summary.v1` | **absent** | present |

### Fix

Update the schema list in `CLAUDE.md` to match `docs/wire-schema/v1/` exactly. No other changes.

**Correct list**: `ingest_report.v1`, `ingest_progress.v1`, `search_hit.v1`, `answer.v1`, `doctor.v1`, `reset_report.v1`, `schema.v1`, `error.v1`, `chunk_inspection.v1`, `citation.v1`, `doc_summary.v1`

---

## §2 — fb-26: Ingest log consistency

### Problem

`crates/kebab-cli/src/progress.rs` has two bugs that break the TTY/non-TTY symmetry:

1. **`Aborted` handler** (`L170-188`): `writeln!` is unconditional — fires in TTY mode too, printing a duplicate summary below the spinner's abandoned message.
2. **`Completed` TTY path** (`L153-169`): `bar.finish_and_clear()` clears the bar with no subsequent summary line. Users see the run end silently.

Additionally, there is no escape hatch for CI environments that emulate a TTY (pty wrapper), which causes unintended spinner output in CI logs.

### Design

**Behavioral contract** (Option A — already the intent, bug-fixed):

| Mode | Progress | Final summary |
|---|---|---|
| TTY | indicatif in-place spinner → progress bar | single `ingest: complete / aborted` writeln after bar clears |
| non-TTY | append-only writeln per event | same `ingest: complete / aborted` writeln |
| `--json` | silent stderr | `ingest_report.v1` stdout only |

**Changes to `handle_human`:**

1. `Completed` TTY: after `bar.finish_and_clear()`, add `writeln!(stderr, "ingest: complete (...)")`  — same format as non-TTY branch.
2. `Aborted` TTY: wrap the existing unconditional `writeln!` in `if !tty { ... }`. The `bar.abandon_with_message(...)` already prints the spinner's final state on TTY.
3. Unify summary format string: `ingest: complete (scanned={} new={} updated={} skipped={} errors={})` and `ingest: aborted (...)` — identical prefix in both modes.

**`KEBAB_PROGRESS=plain` env override:**

- When set (any non-empty value), force non-TTY branch regardless of `IsTerminal`.
- Implemented in `ProgressMode::from_flags` — check `KEBAB_PROGRESS=plain` env, set `tty=false` when present.
- Allows CI with pty wrappers to opt-in to append-only output explicitly.

### Testing

- Snapshot test: non-TTY stream for a minimal ingest (2-file TempDir KB) captures `ScanStarted`, `ScanCompleted`, `AssetStarted × 2`, `Completed` with correct prefixes.
- `KEBAB_PROGRESS=plain` env: TTY path still uses append-only output.
- `KEBAB_PROGRESS=plain` + `--json`: `--json` takes precedence, no human lines.
- Manual smoke: `kebab ingest --config /tmp/... 2>&1 | cat` shows all event lines + final summary.

---

## §3 — fb-28: Agent invocation flags

### Problem

Agents invoking `kebab` face two issues:

1. No way to enforce read-only KB access — a hallucinating agent could call `kebab reset` or `kebab ingest` unexpectedly.
2. Progress/spinner output leaks to stderr even in non-TTY agent invocations where TTY is emulated, adding noise to agent context.

### Design

#### Global flags on `Cli`

```
kebab [--readonly] [--quiet] <subcommand> [...]
```

Both are global flags added to the `Cli` struct in `main.rs`. Evaluated before subcommand dispatch.

#### `--readonly` / `KEBAB_READONLY=1`

- Environment variable `KEBAB_READONLY=1` is equivalent to passing `--flag` (checked in `main` before dispatch; env wins if set).
- **Blocked subcommands**: `ingest`, `ingest-file`, `ingest-stdin`, `reset` (all write-path commands). (`nuke` does not exist as a subcommand.)
- **Allowed**: `search`, `ask`, `doctor`, `schema`, `mcp`, `tui` (read-path).
- On block: exit code 1 + error output:
  - `--json` mode: `error.v1` ndjson to stderr (`code: "readonly_mode"`, `message: "kebab: readonly mode — mutating commands are disabled"`)
  - plain mode: single `kebab: readonly mode — mutating commands are disabled\n` to stderr
- Implementation: `fn is_mutating(cmd: &Cmd) -> bool` + guard block in `main()` after flag parsing, before `match cli.cmd`.

#### `--quiet`

- Suppresses all human-readable stderr output: progress lines, hint messages.
- Does **not** suppress `error.v1` ndjson (in `--json` mode) or plain error text — errors always reach stderr.
- `--json` flag automatically implies `--quiet` behavior (already the case in practice; this makes it explicit and documented).
- Implementation: extend `ProgressMode::Human { tty: bool }` → `Human { tty: bool, quiet: bool }`. Update `ProgressMode::from_flags(json: bool, quiet: bool, plain_env: bool) -> Self`. When `quiet=true` (or `--json`), `Human { tty: _, quiet: true }` overrides draw target to `hidden` and skips all `writeln!(stderr, ...)` calls in non-TTY branch. `ProgressDisplay::new(mode: ProgressMode)` signature unchanged.
- `--quiet` without `--json` still emits `ingest_report.v1` to stdout at end (not suppressed).

#### New `error.v1` code

Construct `ErrorV1 { code: "readonly_mode", ... }` directly in the guard block in `main.rs` — no change to `classify()` (which dispatches on `anyhow::Error` types, not user-triggered state). Document the new code in `tasks/HOTFIXES.md`.

### Testing

- `kebab --readonly ingest` → exit 1, error message contains "readonly mode".
- `kebab --readonly ingest --json` → exit 1, stderr contains `error.v1` with `code: "readonly_mode"`.
- `KEBAB_READONLY=1 kebab ingest` → same as `--readonly`.
- `kebab --readonly search "q"` → passes through normally.
- `kebab --quiet ingest` → stderr silent during run, `ingest_report.v1` still on stdout.
- `kebab ingest --json` → no human lines on stderr (auto-quiet behavior documented).

---

## Bundling rationale

All three changes are small and independent — no shared code paths. Bundled into one branch to avoid PR noise for minor UX polish. The CLAUDE.md fix is doc-only and safe to merge first if needed.

## Files changed (expected)

| File | Change |
|---|---|
| `CLAUDE.md` | schema list update |
| `crates/kebab-cli/src/main.rs` | `--readonly`, `--quiet` global flags + guard block |
| `crates/kebab-cli/src/progress.rs` | Aborted/Completed bug fix, `KEBAB_PROGRESS=plain`, quiet threading |
| `crates/kebab-app/src/error_wire.rs` | `"readonly_mode"` code |
| `tasks/HOTFIXES.md` | new entry for `readonly_mode` error code |
| `tasks/p9/p9-fb-26-ingest-log-consistency.md` | status → merged |
| `tasks/p9/p9-fb-28-agent-invocation-flags.md` | status → merged |
| `tasks/INDEX.md` | status update |
| `HANDOFF.md` | one-liner |
