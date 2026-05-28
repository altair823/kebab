# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

Single-user local-first knowledge base + RAG. Rust 2024 workspace, ~21 crates, single binary (`kebab`). All inference is local (Ollama + fastembed + whisper.cpp).

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

`target/` is 6–10 GB after a fresh build but **balloons to 90+ GB after a few task cycles** (each fb-* batch adds incremental compile artifacts on top of the existing 18 × test-binary debug info). The dev/test profile is already trimmed (`debug = "line-tables-only"`, `split-debuginfo = "unpacked"` — see workspace `Cargo.toml`). Run `cargo clean` **routinely after each merged PR**, not just "if pressure shows up" — disk space is tight and recovery via `cargo clean` is cheap (one re-link per crate on next build). Verified pattern: 92 GB → 0 GB in seconds, backtraces still resolve to function + line.

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
- UI crates (`kebab-cli`, `kebab-mcp`, `kebab-tui`, future `kebab-desktop`) MUST NOT import `kebab-store-*` / `kebab-llm-*` / `kebab-parse-*` directly — only `kebab-app`.

Read the relevant task spec's deps section before adding an import. New crates inherit the same boundary rules.

## Wire schema v1

All `--json` output carries a `schema_version` field. Current schemas: `ingest_report.v1`, `ingest_progress.v1`, `search_hit.v1`, `answer.v1`, `doctor.v1`, `reset_report.v1`, `schema.v1`, `error.v1`, `chunk_inspection.v1`, `citation.v1`, `doc_summary.v1`. Schemas live in `docs/wire-schema/v1/`. The wire shape is the contract for external integrations (Claude Code skills, MCP, etc.); breaking it requires a `*.v2` major bump and parallel-running both for one phase. In `--json` mode, fatal errors emit `error.v1` to stderr as ndjson (non-`--json` mode keeps plain stderr text); exit codes 0/1/2/3 are unchanged — `error.v1.code` provides fine-grained agent branching.

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
2. release notes 는 사용자 도그푸딩에 영향이 가는 surface 변경을 위주로 — wire schema 추가, CLI flag 신규, TUI 키 변경, V00X migration 등 — 다룬다. 이때 추가된 기능과 변경사항은 유저가 이해할 수 있도록 친절하고 자세하게 풀어서 설명해야 하며, 단순히 commit subject 를 나열하는 형태로 끝내면 안 된다. 필요하다면 도그푸딩이나 테스트 결과도 함께 적어 둔다.
3. 프리-1.0 (`0.x.y`) 단계: minor bump 시 wire schema additive / surface 변경 누적, patch bump 시 bug fix only.

**bump 시점 = release 시점 같은 commit**. 즉 commit `chore: bump version 0.x → 0.y` 직후 같은 commit 에 tag. v0.1.0 (`2319206`) 처럼 bump 없이 tag 만 찍는 패턴은 후속 release 가 대상 commit 을 헷갈리게 함 — pre-release snapshot 은 SHA reference 로 충분.

## Dogfood trigger

도그푸딩 = 새 binary 를 실제 KB / 실제 query 로 돌려보고 user-visible 동작이 spec 의 의도와 일치하는지 확인하는 종단 검증. unit / integration test 가 못 잡는 회귀 (UX 어색함, performance regression, 의외의 token 처리, embedding drift, RAG hallucination) 를 catch 함. PR 머지 전 또는 머지 직후 release notes 작성 전에 실시.

### 도그푸딩이 필요한 시점

다음 트리거 중 하나라도 hit 시 도그푸딩 필수. **모두 release-level 또는 user-visible behavior 변경 임**.

**Schema / migration**:
- 신규 V00X migration (예: V007 trigram, V008 OCR mirror, V009 morphological) — `corpus_revision` cascade + auto-backfill 정책의 사용자 경험 확인.
- frozen design contract 변경 (`docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` §X 갱신) — verbatim CI diff-check 외의 user-visible side effect 확인.

**Wire schema / CLI surface**:
- 신규 `--json` 필드, exit code 변경, 또는 schema major bump (v1 → v2) — agent / external integration 의 호환성 검증.
- `kebab` 의 subcommand 또는 flag 추가/삭제/rename — agent skill / muscle memory 영향.

**Search / RAG behavior**:
- FTS5 tokenizer / chunker / embedder 모델 / RAG prompt template 변경 — 같은 query 의 hit ordering, snippet, RAG citation 패턴이 자연스럽게 변화하는지.
- score gate, RRF fusion ratio, NLI threshold 같은 ranking 파라미터 default 변경.

**Performance**:
- ingest / search / ask latency 의 의도된 변화 (예: lindera tokenize, OCR 추가, multi-hop RAG) — actual wall-clock 측정 + release notes 에 명시.
- 대용량 KB (수천 doc / 만 chunk) 의 first-boot eager backfill 시간이 사용자 hang 인지에 영향 안 가는지.

**Language / locale**:
- 한국어 / 일본어 / 중국어 lexical 동작 변경 (V007 trigram, V009 morphological, future N-gram).
- 영어 substring 매칭 같은 ad-hoc 부산물의 회귀.

**File / asset surface**:
- 신규 source 형식 (PDF OCR, audio, video) — extractor / chunker 의 실제 corpus 동작.
- `.kebabignore` / `_external/` 같은 workspace 정책 변경.

**Release-level**: 위 트리거 중 하나가 hit 되어 `Cargo.toml` workspace `version` bump 가 필요하면, **bump commit 이전에 도그푸딩 evidence 가 HOTFIXES + release notes 에 명시** 되어 있어야 함. evidence 없는 release 는 사용자가 "왜 bump 했는지" 추적 불가.

### 도그푸딩 데이터 보관소

모든 도그푸딩 source 문서 + KB state + 로그는 `/build/dogfood/` 한 디렉토리에 누적 보관한다. **분류는 문서 의미 / 종류 / 형식 기준만** — kebab version, 생성 시점, scenario name 같은 prefix 금지 (`v0.20.1-dogfood/`, `dogfood-v018/` 같은 디렉토리 신설 X). 자세한 layout 은 `/build/dogfood/README.md` 참조.

- `/build/dogfood/corpus/` — source 문서 (read-only). format 별 분류 (`markdown/`, `code/`, `html/`, `images/`, `pdf/`, `manifest/`, `resources/`) + 각 format 내 category 별 (예: `markdown/{korean,english,bilingual,tech-docs,coding-md-corpus,topics,notes,edge-cases}`, `code/{rust,python,...}`). 새 fixture 는 적절한 category subdir 에 추가.
- `/build/dogfood/kb/` — 도그푸딩 run 의 KB 출력 (SQLite + LanceDB + assets + models). 매 run 마다 reset 가능. 별 KB 디렉토리 신설 X.
- `/build/dogfood/logs/` — 누적 실행 로그 (ndjson + stderr + summary).
- `/build/dogfood/config.toml` — canonical 도그푸딩 config (없으면 `kebab init` 후 path override).
- `/build/dogfood/_archive/` — regeneratable stale state (이전 run 의 sqlite/lancedb, XDG snapshot). 디스크 압박 시 wipe 가능.

`/tmp/kebab-smoke/`, `/tmp/kebab-*`, `/build/cache/dogfood*`, `/home/altair823/KnowledgeBase`, `~/.config/kebab/`, `~/.local/share/kebab/`, `~/.local/state/kebab/` 같은 위치 신규 사용 금지 — 모두 `/build/dogfood/` 로 일관. ad-hoc fixture 가 필요하면 `corpus/<format>/<category>/` 에 추가.

### 도그푸딩 결과 기록

도그푸딩 evidence 는 두 곳에 cascade:

1. **`tasks/HOTFIXES.md` 의 dated entry** — 시나리오 별 hit count 표 + snippet evidence + known limitation. 미래에 spec drift 의심 시 git history 외 immediate reference 가 됨.
2. **`docs/release-notes/v<X.Y.Z>-draft.md`** (또는 gitea release body) — 사용자 도그푸딩 영향에 영향이 가는 surface 변경을 4 단락 (변경 사실 / trade-off / mitigation / upgrade 절차) 으로 풀어서 설명. evidence link.

도그푸딩 단계에서 *발견된 bug* (spec 과 실제 동작의 mismatch, performance regression, UX 어색함) 는 즉시 fix → re-dogfood. fix 가 별 PR 으로 빠지면 머지 후 HOTFIXES 에 dated entry. 

DOGFOOD scenario catalog (§1~§13) 는 `docs/DOGFOOD.md`. 신규 release 마다 §관련 section 의 scenario list 갱신 + 신규 scenario 추가.

## Naming + paths

- Crate prefix: `kebab-` (kebab-case package, `kebab_` snake_case in Rust modules).
- Binary: `kebab`.
- Env var prefix: `KEBAB_*` (e.g. `KEBAB_RAG_SCORE_GATE`, `KEBAB_EVAL_GOLDEN`, `KEBAB_COMMIT_HASH`).
- XDG paths: `~/.config/kebab/`, `~/.local/share/kebab/`, `~/.cache/kebab/`, `~/.local/state/kebab/`.
- SQLite filename: `kebab.sqlite` (under `data_dir`).
- Workspace ignore: `.kebabignore` (per directory).
- `_external/` (under `workspace.root`): single-file / stdin ingest 가 외부 file 을 deterministic 명명 (`<blake3-12>.<ext>`) 으로 copy. 첫 생성 시 `.kebabignore` 자동 append.

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
