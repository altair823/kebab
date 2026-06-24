# Spine Simplification — Phase 0 (Baseline) + Phase 1 (Cuts) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Establish a frozen core-quality baseline, then delete 5 unused/legacy subsystems (search cache, RAG v1/v2 templates, candle embedder, multi-turn sessions, TUI) — each verified to leave core dogfood quality unchanged.

**Architecture:** Phase 0 captures a deterministic eval + chunk-dump baseline on current `main`. Phase 1 removes each subsystem as an independent PR; every PR ends with the **Parity Gate** (eval-metric deltas ≈ 0 + ingest-output byte-diff = 0) before merge. Deletions only — no behavior change to the kept core (md/code/image/pdf ingest, lexical/vector/hybrid search, single-hop + multi-hop RAG, citation, NLI, multisource, eval, MCP).

**Tech Stack:** Rust 2024 workspace, `cargo`, SQLite (sqlx migrations), `kebab eval` harness, `sqlite3` CLI, `jq`.

**Spec:** `docs/superpowers/specs/2026-06-24-spine-rewrite-simplification-design.md`

## Global Constraints

- **CORE DOGFOOD QUALITY PARITY (HARD GATE):** every task ends with the Parity Gate below; merge forbidden until it passes. (Spec §"코어 도그푸딩 품질 패리티 게이트")
- **ingest output byte-identical:** `parser_version` / `chunker_version` / `embedding_version` MUST NOT change in Phase 1. No re-index.
- **wire output contract unchanged:** `search_hit.v1` / `answer.v1` shape stays (MCP + Claude skill unaffected).
- Build target: `CARGO_TARGET_DIR=/home/user/large_data/out/kebab/target` (root-disk protection — CLAUDE.md).
- Full workspace test runs `-j 1` (linker OOM otherwise). Per-crate parallel OK.
- One PR per task. Branch `refactor/spine-<slug>`. PR via Gitea REST (gitea-ops); `gh` does not work.
- `cargo clean` after each merged PR (CLAUDE.md disk hygiene).
- Line numbers below are from baseline `main` — after earlier tasks shift a shared file, **re-locate the symbol by `grep`/`rg`**, do not trust the absolute line.

---

## Parity Gate (referenced by every Phase 1 task)

Run before requesting merge of any Phase 1 task. Uses the artifacts frozen in Task 0.

````bash
T=/home/user/large_data/out/kebab/target
GATE=/home/user/large_data/out/kebab-parity           # reproducible parity KB (Task 0)
BASE=$GATE/baseline                                    # frozen baseline outputs

# 1. Build this branch's binary
CARGO_TARGET_DIR=$T cargo build --release --bin kebab

# 2. Re-run the FIXED query set; SEARCH output must be byte-identical
#    (deletion-only — retrieval logic untouched; no LLM needed for this check)
mkdir -p /tmp/parity
: > /tmp/parity/search.jsonl
while IFS= read -r q; do
  "$T/release/kebab" --config "$GATE/config.toml" search --json --quiet --mode lexical "$q" >> /tmp/parity/search.jsonl
  "$T/release/kebab" --config "$GATE/config.toml" search --json --quiet --mode hybrid  "$q" >> /tmp/parity/search.jsonl
done < "$BASE/queries.txt"
diff "$BASE/search.jsonl" /tmp/parity/search.jsonl && echo "SEARCH IDENTICAL"

# 3. RAG answers byte-identical (temp 0 / seed fixed). Strip volatile fields
#    (run-id, elapsed_ms, timestamps) — keep answer/citations/grounded/refusal/template.
: > /tmp/parity/ask.jsonl
while IFS= read -r q; do
  "$T/release/kebab" --config "$GATE/config.toml" ask --json --quiet --temperature 0.0 --seed 12345 "$q" \
    | jq -cS '{answer, citations, grounded, refusal_reason, prompt_template_version}' >> /tmp/parity/ask.jsonl
done < "$BASE/queries.txt"
diff "$BASE/ask.jsonl" /tmp/parity/ask.jsonl && echo "ASK IDENTICAL"

# 4. Chunk byte-identity (no sqlite3 CLI on this host → python3 stdlib). DB at
#    $GATE/data/kebab.sqlite; chunks table has no embedding_version (use policy_hash).
python3 - "$GATE/data/kebab.sqlite" /tmp/parity/chunks.tsv <<'PYEOF'
import sys, sqlite3
c = sqlite3.connect(sys.argv[1])
rows = c.execute("SELECT chunk_id, text, chunker_version, policy_hash FROM chunks ORDER BY chunk_id").fetchall()
open(sys.argv[2], "w").write("\n".join("\t".join(str(x) for x in r) for r in rows) + "\n")
PYEOF
diff "$BASE/chunks.tsv" /tmp/parity/chunks.tsv && echo "CHUNKS IDENTICAL"
````

> **Env (this host, verified Task 0):** parity KB `$GATE=/home/user/large_data/out/kebab-parity` (183 docs / 7676 chunks), embedder `snowflake-arctic-embed2` + LLM `gemma3:4b` on **GPU ollama `192.168.0.244:11434`** (lemonade stopped for Phase 1; restore after). `search` AND `ask` deterministic at temp 0 / seed 12345 (2× byte-identical — verified). One gate run ≈ 65s. `sqlite3` CLI NOT installed — python3 stdlib for all DB reads.

**PASS criteria (all are byte-diffs — output-equality, label-free):**
- `SEARCH IDENTICAL` — every query's lexical + hybrid hits byte-identical. No LLM needed; deletions don't touch retrieval. ANY diff is a real regression.
- `ASK IDENTICAL` — every query's answer + citations + grounded + refusal + template version identical (volatile timing/run-id stripped via jq). RAG-touching cuts (templates v1/v2, sessions) must leave the default rag-v4 / non-session path identical. (LLM = the KB config's endpoint, self-consistent before/after.)
- `CHUNKS IDENTICAL` — chunk dump byte-identical.
- Any non-empty diff → root-cause, fix on the same branch, re-gate. Do **not** merge.

Record the three `* IDENTICAL` confirmations (or the offending diff) in the PR body + a dated `tasks/HOTFIXES.md` entry.

---

## Task 0: Build the reproducible parity KB + freeze baseline outputs

Output-equality baseline (label-free). A small, fully-reproducible KB: kebab's own
docs as corpus, **fastembed local** embedder (no network at query time), **lemonade**
(`.243:13305`, self-consistent) LLM for RAG. Controller-run (ops task, not an implementer).

**Files (all under `/home/user/large_data/out/kebab-parity/`):**
- Create: `config.toml` (fastembed embed + lemonade llm, data under this dir)
- Create: `data/kebab/kebab.sqlite` (ingested corpus)
- Create: `baseline/queries.txt` (fixed query set)
- Create: `baseline/search.jsonl` (lexical+hybrid `search --json` per query)
- Create: `baseline/ask.jsonl` (normalized `ask --json` per query)
- Create: `baseline/chunks.tsv` (deterministic chunk dump)

**Interfaces:**
- Produces the `$BASE/{queries.txt,search.jsonl,ask.jsonl,chunks.tsv}` the Parity Gate diffs against.

- [ ] **Step 1: Build the baseline binary from current `main`**

```bash
git checkout main && git pull --ff-only
T=/home/user/large_data/out/kebab/target
CARGO_TARGET_DIR=$T cargo build --release --bin kebab
```
Expected: `Finished release`, binary at `$T/release/kebab`.

- [ ] **Step 2: Scaffold the parity KB config**

```bash
GATE=/home/user/large_data/out/kebab-parity
mkdir -p "$GATE/data" "$GATE/baseline" "$GATE/corpus"
"$T/release/kebab" --config "$GATE/config.toml" init --force   # writes default config + dirs
```
Then edit `$GATE/config.toml`: `[storage] data_dir` under `$GATE/data`; `[models.embedding] provider="fastembed"` (default e5, local); `[models.llm] endpoint="http://192.168.0.243:13305"` model = lemonade's instruct model (`Gemma-4-31B-it-GGUF`); `[workspace] root="$GATE/corpus"`. Confirm `kebab --config "$GATE/config.toml" doctor` is green.

- [ ] **Step 3: Assemble a fixed corpus (kebab's own docs — aligns with golden queries)**

```bash
cp -r docs README.md HANDOFF.md CLAUDE.md "$GATE/corpus/"   # stable, in-repo, covers g001-g005 topics
"$T/release/kebab" --config "$GATE/config.toml" ingest 2>&1 | tail -5
sqlite3 "$GATE/data/kebab/kebab.sqlite" "SELECT count(*) FROM chunks;"   # expect > 0
```
Expected: ingest completes errors=0, chunk count > 0. (fastembed downloads e5 once if not cached.)

- [ ] **Step 4: Define the fixed query set**

```bash
cat > "$GATE/baseline/queries.txt" <<'EOF'
Cargo workspace 멤버 추가하는 법
What is the facade rule?
Markdown chunking 규칙은?
How does FTS5 tokenization work for Korean text?
RAG citation 검증은 어떻게 동작?
embedding version cascade
search hybrid fusion
EOF
```

- [ ] **Step 5: Freeze baseline search + ask outputs**

```bash
: > "$GATE/baseline/search.jsonl"
while IFS= read -r q; do
  "$T/release/kebab" --config "$GATE/config.toml" search --json --quiet --mode lexical "$q" >> "$GATE/baseline/search.jsonl"
  "$T/release/kebab" --config "$GATE/config.toml" search --json --quiet --mode hybrid  "$q" >> "$GATE/baseline/search.jsonl"
done < "$GATE/baseline/queries.txt"

: > "$GATE/baseline/ask.jsonl"
while IFS= read -r q; do
  "$T/release/kebab" --config "$GATE/config.toml" ask --json --quiet --temperature 0.0 --seed 12345 "$q" \
    | jq -cS '{answer, citations, grounded, refusal_reason, prompt_template_version}' >> "$GATE/baseline/ask.jsonl"
done < "$GATE/baseline/queries.txt"
wc -l "$GATE/baseline/search.jsonl" "$GATE/baseline/ask.jsonl"
```
Expected: search.jsonl has 14 lines (7 queries × 2 modes), ask.jsonl 7 lines. Re-run Step 5 a second time and confirm it reproduces byte-identically (determinism check — if `ask` is non-deterministic at temp 0, note it; search must be identical).

- [ ] **Step 6: Freeze the deterministic chunk dump**

```bash
sqlite3 "$GATE/data/kebab/kebab.sqlite" \
  "SELECT chunk_id, text, chunker_version, embedding_version FROM chunks ORDER BY chunk_id" \
  > "$GATE/baseline/chunks.tsv"
wc -l "$GATE/baseline/chunks.tsv"
```

- [ ] **Step 7: Record baseline in HOTFIXES**

Add a dated `tasks/HOTFIXES.md` entry: parity KB path, corpus (kebab docs), embedder (fastembed e5) + LLM (lemonade), chunk count, `main` SHA, and the determinism note from Step 5. Immutable reference for the whole spine rewrite.

```bash
git add tasks/HOTFIXES.md
git commit -m "docs(hotfix): spine-rewrite 출력-동등성 parity baseline 동결 (재현 KB + search/ask/chunk 캡처)"
```

---

## Task 1: Delete the search LRU cache (p9-fb-19)

Lowest-risk cut — internal optimization, no behavior change, no migration. Establishes the gate rhythm.

**Files:**
- Modify: `crates/kebab-app/src/app.rs` (remove cache field, type, methods, get/put)
- Modify: `crates/kebab-config/src/lib.rs` (remove `cache_capacity` + default)
- Modify: `crates/kebab-cli/src/main.rs` (remove `clear_search_cache` call + doctor cap)
- Modify: `crates/kebab-cli/src/wire.rs` (doctor capability)
- Modify: `Cargo.toml` (drop `lru` dep if unused elsewhere)
- Test: `crates/kebab-app/tests/ask_smoke.rs` (remove cache assertions)

**Interfaces:**
- Produces: `App::search()` becomes identical to the old `search_uncached()` (cache removed); `SearchCfg` loses `cache_capacity`; doctor `schema.v1.capabilities.search_cache` becomes `false` (kept as field for wire stability, value flips).

- [ ] **Step 1: Remove cache from `kebab-app/src/app.rs`**

Remove (re-locate by symbol): `use lru::LruCache;`; the `SearchCacheKey` struct; the `search_cache: Option<Mutex<LruCache<...>>>` field; the `impl SearchCacheKey {…}` block; the `NonZeroUsize::new(config.search.cache_capacity)` init + `search_cache,` initializer field; the cache lookup + insert blocks inside `search()` (replace `search()` body with a direct call to the existing uncached path); `build_cache_key()`; `clear_search_cache()`. Rename `search_uncached` → `search` (or make `search` the only method).

- [ ] **Step 2: Remove `cache_capacity` from `kebab-config/src/lib.rs`**

Remove the `#[serde(default = "default_cache_capacity")] pub cache_capacity: usize,` field from `SearchCfg`, the `default_cache_capacity()` fn, and the `cache_capacity:` line in `SearchCfg::default()`.

- [ ] **Step 3: Remove cache refs from CLI**

`crates/kebab-cli/src/main.rs`: remove the `app.clear_search_cache();` call. In doctor capability output, set `search_cache` to `false` (keep the wire field). `crates/kebab-cli/src/wire.rs`: set `search_cache: false` in the doctor capabilities initializer.

- [ ] **Step 4: Drop the `lru` dependency**

```bash
rg -n "lru" --type toml --type rust crates/ Cargo.toml
```
If `lru` is referenced only by the removed code, delete `lru = "0.12"` from workspace `Cargo.toml` and the `lru` line in `kebab-app/Cargo.toml`. If referenced elsewhere, leave it.

- [ ] **Step 5: Fix the smoke test**

`crates/kebab-app/tests/ask_smoke.rs`: remove any assertions referencing cache fields/behavior. Keep the rest.

- [ ] **Step 6: Build + per-crate test**

```bash
T=/home/user/large_data/out/kebab/target
CARGO_TARGET_DIR=$T cargo build --release --bin kebab
CARGO_TARGET_DIR=$T cargo test -p kebab-app -p kebab-config -p kebab-cli
```
Expected: clean build, tests pass (minus removed cache tests).

- [ ] **Step 7: clippy gate**

```bash
CARGO_TARGET_DIR=$T cargo clippy -p kebab-app -p kebab-config -p kebab-cli --all-targets -- -D warnings
```
Expected: 0 warnings.

- [ ] **Step 8: PARITY GATE**

Run the Parity Gate (top of doc). Cache removal must yield **identical** retrieval (deltas all `0.0`) and `CHUNKS IDENTICAL`. Record deltas in PR + HOTFIXES.

- [ ] **Step 9: Commit + PR**

```bash
git checkout -b refactor/spine-drop-search-cache
git add -A
git commit -m "refactor(app): search LRU 캐시 제거 (p9-fb-19) — 동작 불변, 표면 축소"
# PR via gitea-ops; body carries parity deltas + CHUNKS IDENTICAL
```

---

## Task 2: Delete legacy RAG templates v1/v2

Default is `rag-v4`; v1/v2 are unreachable legacy. No migration.

**Files:**
- Modify: `crates/kebab-rag/src/pipeline.rs` (constants + match arms)
- Modify: `crates/kebab-rag/tests/prompt_template_dispatch.rs` (drop v1/v2 tests, update unknown-version test)
- Modify: `docs/SMOKE.md`, `README.md`, `tasks/p*/` (grep refs)

**Interfaces:**
- Produces: `system_prompt_for()` accepts only `rag-v3` / `rag-v4` / `rag-multi-hop-v2`; unknown → error listing only the live versions.

- [ ] **Step 1: Remove constants + arms**

`crates/kebab-rag/src/pipeline.rs`: delete `SYSTEM_PROMPT_RAG_V1` and `SYSTEM_PROMPT_RAG_V2` constants; delete the `"rag-v1" => …` and `"rag-v2" => …` match arms in `system_prompt_for()`. Update the unknown-version error string to list only `rag-v3`, `rag-v4`.

- [ ] **Step 2: Update tests**

`crates/kebab-rag/tests/prompt_template_dispatch.rs`: delete `test_system_prompt_for_rag_v1_returns_v1_const` and `…_rag_v2_…`; update `test_system_prompt_for_unknown_version_returns_err_with_hint` expected text (no v1/v2); delete/update any v2-marker-format test.

- [ ] **Step 3: Scrub docs**

```bash
rg -n "rag-v1|rag-v2" docs/ README.md tasks/
```
Replace user-facing mentions with `rag-v4` (current default); leave frozen task-spec historical mentions but add a HOTFIXES note that v1/v2 were removed.

- [ ] **Step 4: Build + test + clippy**

```bash
T=/home/user/large_data/out/kebab/target
CARGO_TARGET_DIR=$T cargo build --release --bin kebab
CARGO_TARGET_DIR=$T cargo test -p kebab-rag
CARGO_TARGET_DIR=$T cargo clippy -p kebab-rag --all-targets -- -D warnings
```
Expected: clean; dispatch tests pass with only v3/v4.

- [ ] **Step 5: PARITY GATE**

Default template unchanged (rag-v4) → RAG metrics within ±0.02, retrieval deltas 0.0, CHUNKS IDENTICAL.

- [ ] **Step 6: Commit + PR**

```bash
git checkout -b refactor/spine-drop-rag-v1-v2
git commit -am "refactor(rag): legacy 템플릿 rag-v1/v2 제거 — v3/v4만 유지"
```

---

## Task 3: Delete the candle embedder crate

Default provider is `fastembed` (e5) / `ollama` (arctic); candle is unused. Verify the dogfood config provider is NOT `candle` before starting (else parity would shift embeddings).

**Files:**
- Delete: `crates/kebab-embed-candle/` (entire crate incl. `tests/parity.rs`, `tests/arctic_ollama_parity.rs`, `tests/thread_cap.rs`)
- Modify: `Cargo.toml` (member), `crates/kebab-app/Cargo.toml` (dep + `embed_metal` feature), `crates/kebab-app/src/app.rs` (import + match arm), `crates/kebab-cli/src/main.rs` (backend display), `crates/kebab-config/src/lib.rs` (provider doc + `num_threads`), `crates/kebab-config/src/migrate.rs` (doc strings), docs (README/ARCHITECTURE/HANDOFF)

**Interfaces:**
- Produces: embedding provider enum accepts `fastembed` | `ollama` | `none`; `candle` → unknown-provider error.

- [ ] **Step 1: Precondition — confirm dogfood config is not candle**

```bash
grep -n "provider" /home/user/large_data/out/kebab-dogfood/config.toml
```
Expected: `provider = "fastembed"` or `"ollama"`. If `candle`, STOP — switch the dogfood config + re-baseline first (candle removal would otherwise change embeddings and fail parity legitimately).

- [ ] **Step 2: Delete the crate + workspace member**

```bash
git rm -r crates/kebab-embed-candle
# Cargo.toml: delete the "crates/kebab-embed-candle", members line
```

- [ ] **Step 3: Remove candle from `kebab-app`**

`crates/kebab-app/Cargo.toml`: delete `kebab-embed-candle = …` dep and the `embed_metal = ["kebab-embed-candle/metal"]` feature. `crates/kebab-app/src/app.rs`: delete `use kebab_embed_candle::CandleEmbedder;` and the `"candle" => Arc::new(CandleEmbedder::new(...)?)` match arm; update the unknown-provider error string to drop `candle`.

- [ ] **Step 4: Remove candle from CLI + config**

`crates/kebab-cli/src/main.rs`: delete the two `"candle" …` arms in the backend-display match (+ any `embed_metal` feature passthrough in `kebab-cli/Cargo.toml`). `crates/kebab-config/src/lib.rs`: drop `candle` from the provider doc comment; remove the candle-exclusive `num_threads` field + its doc (confirm via `rg "num_threads"` that nothing else reads it; if `KEBAB_EMBED_THREADS` legacy still wires it, keep the field but drop candle wording). `crates/kebab-config/src/migrate.rs`: update the provider/num_threads doc strings.

- [ ] **Step 5: Scrub docs**

Remove candle from README (provider examples, NUMA/Metal sections), `docs/ARCHITECTURE.md` (mermaid node `embedcandle` + edges + dir-tree line + decision table rows), `HANDOFF.md` (candle entry).

- [ ] **Step 6: Build + workspace test + clippy**

```bash
T=/home/user/large_data/out/kebab/target
CARGO_TARGET_DIR=$T cargo build --release --bin kebab
CARGO_TARGET_DIR=$T cargo test -p kebab-app -p kebab-config -p kebab-cli
CARGO_TARGET_DIR=$T cargo clippy --workspace --all-targets -- -D warnings
```
Expected: clean (no references to removed crate).

- [ ] **Step 7: PARITY GATE**

Provider unchanged in dogfood → embeddings identical → retrieval deltas 0.0, CHUNKS IDENTICAL.

- [ ] **Step 8: Commit + PR**

```bash
git checkout -b refactor/spine-drop-candle-embedder
git commit -am "refactor(embed): candle provider/crate 제거 — fastembed+ollama로 충분 (crate 23→22)"
```

---

## Task 4: Delete multi-turn sessions (+ V015 migration)

Removes `ask --session`, session storage, and history threading. Needs a drop migration.

**Files:**
- Delete: `migrations/V005__chat_sessions.sql`? **NO** — keep historical migration; add a new drop migration. Delete: `crates/kebab-store-sqlite/src/chat_sessions.rs`, `crates/kebab-store-sqlite/tests/chat_sessions.rs`
- Create: `migrations/V015__drop_chat_sessions.sql`
- Modify: `crates/kebab-core/src/answer.rs` (Answer.conversation_id/turn_index, `Turn`), `crates/kebab-core/src/traits.rs` (`ChatSessionRow`/`ChatTurnRow`/`ChatSessionRepo`), `crates/kebab-store-sqlite/src/lib.rs` (`mod chat_sessions`), `crates/kebab-rag/src/pipeline.rs` (AskOpts history fields, `ask_with_history`, history helpers + tests), `crates/kebab-app/src/app.rs` + `lib.rs` (`ask_with_session*`), `crates/kebab-cli/src/main.rs` (`--session`), `crates/kebab-mcp/src/tools/ask.rs` (conversation_id if present)

**Interfaces:**
- Produces: `AskOpts` without `history`/`conversation_id`/`turn_index`; `Answer` without `conversation_id`/`turn_index`; `RagPipeline::ask(query, opts)` only (no `ask_with_history`).

- [ ] **Step 1: Add the drop migration**

Create `migrations/V015__drop_chat_sessions.sql`:
```sql
DROP TABLE IF EXISTS chat_turns;
DROP TABLE IF EXISTS chat_sessions;
```

- [ ] **Step 2: Remove store layer**

```bash
git rm crates/kebab-store-sqlite/src/chat_sessions.rs crates/kebab-store-sqlite/tests/chat_sessions.rs
```
`crates/kebab-store-sqlite/src/lib.rs`: remove `mod chat_sessions;` and any `use kebab_core::traits::ChatSessionRepo;`.

- [ ] **Step 3: Remove core types**

`crates/kebab-core/src/answer.rs`: remove `Answer.conversation_id`, `Answer.turn_index` (+ serde attrs), and the `Turn` struct. `crates/kebab-core/src/traits.rs`: remove `ChatSessionRow`, `ChatTurnRow`, `ChatSessionRepo`.

- [ ] **Step 4: Remove RAG history threading**

`crates/kebab-rag/src/pipeline.rs`: drop `Turn` from the `use`; remove `AskOpts.history`/`conversation_id`/`turn_index` (+ defaults); delete `ask_with_history()`; in `ask()` replace `expand_query_with_history(query, &opts.history)` with `query.to_string()` and remove the history prompt-budget branch; set `Answer.conversation_id` to `None` (or remove field usage) in `ask()` + `ask_multi_hop()`; delete `expand_query_with_history`, `remaining_history_budget_chars`, `serialize_history`, and their unit tests + `fake_turn` helper.

- [ ] **Step 5: Remove app + CLI + MCP session paths**

`kebab-app/src/app.rs`: delete `ask_with_session()`. `kebab-app/src/lib.rs`: delete `ask_with_session_with_config()`. `kebab-cli/src/main.rs`: remove `session: Option<String>` from the `Ask` clap struct, the session dispatch branch, and `turn_index: None,` from the `AskOpts` initializer. `kebab-mcp/src/tools/ask.rs`: `rg conversation_id` and remove if present.

- [ ] **Step 6: Update affected tests**

```bash
rg -n "conversation_id|ask_with_history|--session|\.history" crates/*/tests crates/*/src
```
Update `streaming_events.rs`, `multi_hop*.rs`, `ask_smoke.rs` to drop session assertions.

- [ ] **Step 7: Build + workspace test + clippy + migration check**

```bash
T=/home/user/large_data/out/kebab/target
CARGO_TARGET_DIR=$T cargo build --release --bin kebab
CARGO_TARGET_DIR=$T cargo test --workspace --no-fail-fast -j 1
CARGO_TARGET_DIR=$T cargo clippy --workspace --all-targets -- -D warnings
# Verify V015 applies cleanly on a fresh DB
"$T/release/kebab" --config /tmp/kebab-mig/config.toml doctor   # after init in a temp dir
```
Expected: clean; V015 drops tables; doctor green.

- [ ] **Step 8: PARITY GATE**

Sessions don't affect single-pass/multi-hop quality on the (session-less) golden set → retrieval deltas 0.0, RAG within ±0.02, CHUNKS IDENTICAL.

- [ ] **Step 9: Commit + PR (MINOR — `--session` removed + V015)**

```bash
git checkout -b refactor/spine-drop-sessions
git commit -am "refactor: multi-turn 세션 제거 (ask --session, chat_sessions/turns) + V015 drop migration"
```

---

## Task 5: Delete the TUI crate

Whole interface removal — no effect on CLI/MCP core.

**Files:**
- Delete: `crates/kebab-tui/` (entire crate)
- Modify: `Cargo.toml` (member), `crates/kebab-cli/Cargo.toml` (dep), `crates/kebab-cli/src/main.rs` (`Tui` subcommand + handler), `README.md`, `docs/ARCHITECTURE.md`, `HANDOFF.md`, `CLAUDE.md` (tui mentions)

**Interfaces:**
- Produces: `kebab` binary without the `tui` subcommand; UI surface = CLI + MCP only.

- [ ] **Step 1: Delete crate + member**

```bash
git rm -r crates/kebab-tui
# Cargo.toml: delete "crates/kebab-tui", members line
```

- [ ] **Step 2: Remove the CLI subcommand**

`crates/kebab-cli/Cargo.toml`: delete the `kebab-tui = { path = "../kebab-tui" }` dep block. `crates/kebab-cli/src/main.rs`: delete the `Tui` variant from the subcommand enum and the `Cmd::Tui => { … kebab_tui::App::new(config)?.run() }` handler arm.

- [ ] **Step 3: Scrub docs**

README: remove the `kebab tui` command-table row, the prose line, and the `tui` mermaid node + edges; fix the facade-rule line. `docs/ARCHITECTURE.md`: remove `tui` mermaid node + `tui --> app` edge + dir-tree line + the TUI features-table row + synopsis mention. `HANDOFF.md`: edit the P9 phase row and delete the TUI-only HOTFIXES entries (p9-fb-09/10/11/12/13/14/24). `CLAUDE.md`: drop `kebab-tui` from the two UI-crate lists.

- [ ] **Step 4: Build + workspace test + clippy**

```bash
T=/home/user/large_data/out/kebab/target
CARGO_TARGET_DIR=$T cargo build --release --bin kebab
CARGO_TARGET_DIR=$T cargo test --workspace --no-fail-fast -j 1
CARGO_TARGET_DIR=$T cargo clippy --workspace --all-targets -- -D warnings
```
Expected: clean; `kebab tui` no longer a subcommand (`kebab tui` → clap error).

- [ ] **Step 5: PARITY GATE**

TUI is orthogonal to core retrieval/RAG → all deltas 0.0 / within ±0.02, CHUNKS IDENTICAL.

- [ ] **Step 6: Commit + PR (MINOR — `tui` subcommand removed)**

```bash
git checkout -b refactor/spine-drop-tui
git commit -am "refactor: kebab-tui crate + tui 서브커맨드 제거 (crate 22→21, UI=CLI/MCP)"
```

---

## Phase 1 exit criteria

- 5 PRs merged, each with a recorded passing Parity Gate (deltas + CHUNKS IDENTICAL) in its body + a HOTFIXES entry.
- `git grep -i` finds no live references to: `LruCache`/`search_cache`, `rag-v1`/`rag-v2` (outside frozen task specs), `candle`/`CandleEmbedder`, `chat_sessions`/`ask_with_history`/`--session`, `kebab-tui`/`kebab_tui`.
- crate count 24 → 22 (tui, embed-candle gone).
- Batched release (or per-PR) version decision per CLAUDE.md (sessions/tui removal = MINOR; cache/templates = PATCH). Recommend ONE batched MINOR after Phase 1.
- Baseline artifacts under `large_data/out/kebab-dogfood/baseline/` remain frozen for Phases 2–5.

## Next: Phase 2 plan (Config slices + surface trim)

Written when Phase 1 merges (its exact edits depend on the post-deletion config/app surface). Will reuse this doc's Parity Gate verbatim. Phase 2 introduces config schema v4→v5 → the Parity Gate's chunk-diff step gains a one-time **expected** re-ingest only if a chunking-affecting key moves (none planned; OCR-key consolidation must preserve `ingest_config_signature` → still byte-identical).

## Self-review notes

- Spec coverage: Phase 1 covers all 5 spec "Cuts" except `ingest API 5→1` (deliberately moved to Phase 3 ingest-spine to avoid double work — noted in spec §Scope and here).
- No placeholders: all deletion refs are concrete file+symbol (line numbers flagged as drift-prone → grep). Baseline KB path assumes the standard dogfood store; Step 1 of Task 0 hard-fails if absent.
- Type consistency: `AskOpts` field removals (Task 4) are consumed by CLI (Task 4 Step 5) and tests (Step 6) in the same task — no cross-task signature drift. `SearchCfg.cache_capacity` removal (Task 1) is self-contained.
