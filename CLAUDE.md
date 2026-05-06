# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

Single-user local-first knowledge base + RAG. Rust 2024 workspace, ~20 crates, single binary (`kebab`). All inference is local (Ollama + fastembed + whisper.cpp).

The repo's documentation is split by audience — don't duplicate across them:

- **[README.md](README.md)** — first stop for an end user. Quick start, command table, one Mermaid logical-architecture diagram, configuration pointers, license. Stays narrow.
- **[HANDOFF.md](HANDOFF.md)** — phase-level progress dashboard for someone picking the project up. Phase status table, component count, "next task candidates", short summary of post-merge deviations. The README never duplicates this.
- **[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)** — internal structure: crate dependency graph, directory tree, locked-in technical decisions. The README links here from the Mermaid diagram.
- **[docs/superpowers/specs/2026-04-27-kebab-final-form-design.md](docs/superpowers/specs/2026-04-27-kebab-final-form-design.md)** — frozen design contract.
- **[tasks/INDEX.md](tasks/INDEX.md)** — per-component task tree.
- **[tasks/HOTFIXES.md](tasks/HOTFIXES.md)** — dated post-merge deviation log; live source of truth where behavior and the frozen spec disagree.

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

In-tree integration packages live under `integrations/<host>/` — currently `integrations/claude-code/kebab/` (a Claude Code skill that calls `kebab search --json` / `kebab ask --json`). Any wire schema major bump (v1→v2) MUST update each shipped integration in the same PR, same as the version-cascade rule below. Per-user trigger keywords (team / system / acronym) belong in the user's local copy of the skill, not in the repo-shipped frontmatter — keep `integrations/claude-code/kebab/SKILL.md`'s `description` generic.

## Versioning cascade

`parser_version` / `chunker_version` / `embedding_version` / `prompt_template_version` / `index_version` follow the cascade rule in design §9. Changing any of these invalidates downstream records (chunks, embeddings, eval runs, …). When changing a version: either ship a re-process job or treat it as a breaking schema bump. The eval runner snapshots all five into `eval_runs.config_snapshot_json`.

## Release / binary version bump

Workspace `Cargo.toml` 의 `version` 은 binary release 의 정체성. 다음 트리거 중 하나 발생 시 **bump + 새 release 컷**:

- 사용자가 새 바이너리로 **도그푸딩** 또는 **실사용** 을 할 필요가 있다고 명시.
- breaking schema change (V00X migration / wire schema major bump v1→v2 등) 가 머지된 후 — 이전 릴리즈 binary 가 새 DB / 새 wire 와 호환 안 됨. wire 의 additive minor 변경 (예: `IngestReport.unchanged` 같은 필드 추가) 은 backward-compat 이라 본 트리거에 해당 안 됨.
- frozen design contract 변경 (design §X 갱신) 이 머지된 후.

Bump 자체는 단순 minor / patch 한 줄 수정 (`Cargo.toml` workspace `version`) — 이미 모든 kebab-* crate 가 `version = { workspace = true }` 라 자동 cascade. 동시에 `Cargo.lock` 자동 갱신.

Release 절차:

1. `gitea-release v<X.Y.Z>` (gitea-ops skill) 으로 tag + push + release notes.
2. release notes 는 사용자 도그푸딩에 영향 가는 surface 변경 위주 — wire schema 추가, CLI flag 신규, TUI 키 변경, V00X migration 등.
3. 프리-1.0 (`0.x.y`) 단계: minor bump 시 wire schema additive / surface 변경 누적, patch bump 시 bug fix only.

**bump 시점 = release 시점 같은 commit**. 즉 commit `chore: bump version 0.x → 0.y` 직후 같은 commit 에 tag. v0.1.0 (`2319206`) 처럼 bump 없이 tag 만 찍는 패턴은 후속 release 가 대상 commit 을 헷갈리게 함 — pre-release snapshot 은 SHA reference 로 충분.

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

## User-facing docs (README + HANDOFF + ARCHITECTURE)

Three sibling docs split the audience. Every implementation PR (`feat/*`) keeps them in sync; spec PRs (`spec/*`) don't touch any of the three.

**[README.md](README.md) — end user.** Stays narrow. The three surfaces a user touches:

- **CLI** — new `kebab <subcommand>`, flag, `--json` field, or exit-code change. Update the **명령** table and the **Quick start** block if the new flow needs a different invocation.
- **TUI** — new pane, key binding, or run-time behavior visible to a `kebab tui` user. Update the row in the **명령** table and the Mermaid diagram if a new external surface lands.
- **Configuration** — new `config.toml` field, `KEBAB_*` env, default change, or XDG path. Update the **Configuration** section AND the config example block in `docs/SMOKE.md`.

The Mermaid logical-architecture diagram stays the only diagram in the README. If a new media type / external service / store crosses the diagram boundary, update it; otherwise leave it alone.

The README does NOT carry: phase status, component count, post-merge deviations, crate dependency graph, directory tree, locked-in technical decisions. Those live in HANDOFF or ARCHITECTURE.

**[HANDOFF.md](HANDOFF.md) — handing off.** Phase-level progress + next-task candidates. Flip the relevant phase row from ⏳ to ✅ when a phase epic completes. Add a one-line entry under "머지 후 발견된 버그 / 결정 (요약)" when a HOTFIXES entry lands that's load-bearing for someone picking up the project. Per-component progress lives in `tasks/INDEX.md`, not here.

**[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) — implementation detail.** Crate dependency graph, directory tree, locked-in technical decisions. Update when:

- A new crate is added — extend the graph + directory tree.
- A locked-in decision flips (e.g. OCR engine default changes per a HOTFIXES entry) — update the table and link the HOTFIXES entry.
- A directory moves — update the tree.

Out of scope for all three: HOTFIXES detail (`tasks/HOTFIXES.md`), version cascade mechanics (CLAUDE.md §Versioning cascade), per-task spec rationale (`tasks/p<N>/`).

If a feature ships behind a flag that's off-by-default, mention the flag explicitly in the README so a user reading only the README knows the surface exists but is gated.

## Remote

Git remote is Gitea: `https://gitea.altair823.xyz/altair823-org/kebab.git`. PRs are created via the Gitea REST API (`POST /repos/altair823-org/kebab/pulls`) — `gh` CLI does not work against this host. Auth uses `~/.netrc` (populated via `git credential fill`).
