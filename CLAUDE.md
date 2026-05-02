# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

Single-user local-first knowledge base + RAG. Rust 2024 workspace, 18 crates, single binary (`kebab`). All inference is local (Ollama + fastembed + whisper.cpp).

The high-level overview, dependency graph, phase roadmap, and directory tree all live in [README.md](README.md). Don't restate them — link to that and add only what isn't already there.

## Build / test / lint

```bash
cargo test -p <crate>                          # preferred — workspace has 18 crates
cargo test -p <crate> <test_name>              # single test (substring match)
cargo test --workspace --no-fail-fast -j 1     # full suite — see -j 1 below
cargo clippy --workspace --all-targets -- -D warnings   # CI gate
cargo build --release                          # produces target/release/kebab
```

`-j 1` for the full workspace test isn't optional: 18 integration-test binaries each link `lance` + `datafusion` + `arrow` + `tantivy` and the parallel link step exhausts memory (linker gets SIGKILL'd, build silently fails partway). Per-crate runs are fine in parallel.

`target/` is 6–10 GB after a fresh build (DataFusion + Lance + fastembed + 18 × test-binary debug info). The dev/test profile is already trimmed (`debug = "line-tables-only"`, `split-debuginfo = "unpacked"` — see workspace `Cargo.toml`). Run `cargo clean` after phase merges if disk pressure shows up; backtraces still resolve to function + line.

## The facade rule

`kebab-app` is the only crate UI binaries (`kebab-cli`, future `kebab-tui`, `kebab-desktop`) may touch. Every user-facing entry has a `*_with_config(cfg, …)` companion that takes an explicit `Config`:

- `kebab-cli` calls the `*_with_config` form so `--config <path>` is honored.
- The bare `kebab_app::ingest(...)` / `search(...)` / `ask(...)` form re-loads `Config::load(None)` (XDG default) and silently bypasses any explicit path. Two regressions of exactly this shape are recorded in `tasks/HOTFIXES.md` (P3-5 + P4-3 follow-ups). When wiring a new CLI subcommand, always thread the `Config` through.

`*_with_config` is `#[doc(hidden)] pub fn` but it's the **official** config-explicit API, not a test seam.

## Spec contract

`docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` (12 sections) is the single contract for the whole workspace. Every component task spec under `tasks/p<N>/` lists which `contract_sections` it implements.

- Changing the design doc requires updating every referencing task spec in the same PR.
- Task specs themselves stay **frozen** as the historical contract once the task is merged. Don't edit them retroactively to match what shipped.
- Live deviations from the original contract go in `tasks/HOTFIXES.md` as dated entries, plus a one-line cross-link in the original spec's `Risks / notes`. Treat HOTFIXES.md as the live source of truth when behavior and spec disagree.

`tasks/INDEX.md` is the dashboard for which phases / components are done; update its phase status when a phase epic completes.

## Allowed / forbidden deps

Each task spec lists `Allowed dependencies` and `Forbidden dependencies` per design §8. The most load-bearing ones:

- `kebab-core` MUST NOT depend on any other `kebab-*` crate. Domain types only.
- `kebab-eval`'s `metrics` and `compare` modules MUST NOT import retrieval / embedding / LLM crates directly. The runner is allowed to use `kebab-app`'s facade (P5-1 inheritance — see deviations in that task spec).
- UI crates (`kebab-cli`, future `kebab-tui`, `kebab-desktop`) MUST NOT import `kebab-store-*` / `kebab-llm-*` / `kebab-parse-*` directly — only `kebab-app`.

Read the relevant task spec's deps section before adding an import. New crates inherit the same boundary rules.

## Wire schema v1

All `--json` output carries a `schema_version` field (`ingest_report.v1`, `search_hit.v1`, `answer.v1`, `doctor.v1`, …). Schemas live in `docs/wire-schema/v1/`. The wire shape is the contract for external integrations (Claude Code skills, MCP, etc.); breaking it requires a `*.v2` major bump and parallel-running both for one phase.

## Versioning cascade

`parser_version` / `chunker_version` / `embedding_version` / `prompt_template_version` / `index_version` follow the cascade rule in design §9. Changing any of these invalidates downstream records (chunks, embeddings, eval runs, …). When changing a version: either ship a re-process job or treat it as a breaking schema bump. The eval runner snapshots all five into `eval_runs.config_snapshot_json`.

## Naming + paths

- Crate prefix: `kebab-` (kebab-case package, `kebab_` snake_case in Rust modules).
- Binary: `kebab`.
- Env var prefix: `KEBAB_*` (e.g. `KEBAB_RAG_SCORE_GATE`, `KEBAB_EVAL_GOLDEN`, `KEBAB_COMMIT_HASH`).
- XDG paths: `~/.config/kebab/`, `~/.local/share/kebab/`, `~/.cache/kebab/`, `~/.local/state/kebab/`.
- SQLite filename: `kebab.sqlite` (under `data_dir`).
- Workspace ignore: `.kebabignore` (per directory).

The migration from the old `kb` name lives in commits `911fb49 / f1a448d / f9714aa`. If you spot a leftover `kb` reference, treat it as a leftover and fix it (the rename PR sweep covered crates/, docs/, tasks/, README, design doc, fixtures — but workspace root `Cargo.toml` comments needed a follow-up; assume similar misses are possible).

## Smoke + integration

`docs/SMOKE.md` walks through running the full pipeline against an isolated TempDir KB via `--config /tmp/kebab-smoke/config.toml`. Use this instead of touching `~/.local/share/kebab/` when verifying a fresh clone or a CLI flag change. Most CLI regressions surface here, not in unit tests (see HOTFIXES.md).

## Remote

Git remote is Gitea: `https://gitea.altair823.xyz/altair823-org/kebab.git`. PRs are created via the Gitea REST API (`POST /repos/altair823-org/kebab/pulls`) — `gh` CLI does not work against this host. Auth uses `~/.netrc` (populated via `git credential fill`).
