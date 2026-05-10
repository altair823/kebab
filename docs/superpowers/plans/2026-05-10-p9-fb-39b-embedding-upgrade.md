# fb-39b Embedding Model Upgrade Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Upgrade default embedding model from `multilingual-e5-small` (384 dim) to `multilingual-e5-large` (1024 dim) so retrieval precision can improve on Korean dogfooding corpus. Existing user TOMLs pinning `multilingual-e5-small` keep working unchanged.

**Architecture:** Three-line code surface: a new arm in `kebab-embed-local::resolve_model`, defaults flipped in `kebab-config::Config::defaults` (and the TOML template), and the existing test asserting the 384 default updated. LanceDB tables are already namespaced by `(model, dim)` so an upgraded model writes to a fresh table; fb-23 incremental ingest detects the `embedding_version` mismatch and auto-re-embeds on next ingest. No migration tooling — orphan old-model tables cleaned via `kebab reset --vector-only`.

**Tech Stack:** Rust 2024, fastembed 4.9.1 (`MultilingualE5Large` enum already shipped), LanceDB.

**Spec:** `docs/superpowers/specs/2026-05-10-p9-fb-39b-embedding-upgrade-design.md`

---

## File map

**Modify:**
- `crates/kebab-embed-local/src/lib.rs` — add `multilingual-e5-large` arm in `resolve_model`. Update or add `check_dim` test for 1024.
- `crates/kebab-config/src/lib.rs` — flip `Config::defaults().models.embedding.{model, dimensions}` and the TOML template at line ~952. Update default test at line 767.
- `README.md` — `[models.embedding]` section: mention new default + small opt-out + dim mismatch hint.
- `docs/SMOKE.md` — append "Embedding upgrade (fb-39b)" walkthrough showing the `kebab reset --vector-only && kebab ingest` sequence + first-run ONNX download warning.
- `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` §5 storage / §9 versioning — update default model + dim references.
- `tasks/HOTFIXES.md` — entry for embedding upgrade UX (orphan tables on model swap, reset --vector-only flow).
- `tasks/p9/p9-fb-39-retrieval-precision-tuning.md` banner — append note "fb-39b lever 적용 (embedding upgrade) ✅".
- `tasks/INDEX.md` — fb-39b row ✅ (new row alongside fb-39).

**Create:**
- `tasks/p9/p9-fb-39b-embedding-upgrade.md` — new task spec mirroring fb-39 frontmatter (status: completed, design + plan links).

---

## Task 1: Add multilingual-e5-large to kebab-embed-local

**Files:**
- Modify: `crates/kebab-embed-local/src/lib.rs`

- [ ] **Step 1: Append failing tests**

Find the existing `mod tests` (~line 230). Append:

```rust
#[test]
fn resolve_model_supports_e5_large() {
    let m = resolve_model("multilingual-e5-large").expect("e5-large should resolve");
    // The fastembed enum is non-comparable in some versions; we only need
    // to confirm Ok and that the underlying TextEmbedding could be built.
    // Avoid actually constructing the model in tests (1.3 GB ONNX download).
    let _ = m;
}

#[test]
fn check_dim_passes_for_1024() {
    check_dim(1024, 1024).expect("matching dims must pass");
}

#[test]
fn check_dim_rejects_384_vs_1024() {
    let err = check_dim(384, 1024).expect_err("dim mismatch must error");
    let msg = format!("{err}");
    assert!(msg.contains("384") && msg.contains("1024"),
        "error must mention both dims, got: {msg}");
}
```

- [ ] **Step 2: Run tests to confirm failures**

```bash
cargo test -p kebab-embed-local resolve_model_supports_e5_large
cargo test -p kebab-embed-local check_dim_passes_for_1024
```
Expected: `resolve_model_supports_e5_large` fails (no arm); `check_dim_*` passes already (helper is generic).

- [ ] **Step 3: Add arm to resolve_model**

In `crates/kebab-embed-local/src/lib.rs`, find `fn resolve_model` (~line 199). Replace the match body:

```rust
fn resolve_model(name: &str) -> Result<EmbeddingModel> {
    match name {
        "multilingual-e5-small" => Ok(EmbeddingModel::MultilingualE5Small),
        "multilingual-e5-large" => Ok(EmbeddingModel::MultilingualE5Large),
        other => anyhow::bail!(
            "kb-embed-local: unsupported embedding model {other:?}; \
             this adapter currently ships `multilingual-e5-small` and \
             `multilingual-e5-large`. Add a new arm to `resolve_model` \
             (and a fastembed feature flag if needed) to support more."
        ),
    }
}
```

- [ ] **Step 4: Run tests — all pass**

```bash
cargo test -p kebab-embed-local
cargo clippy -p kebab-embed-local --all-targets -- -D warnings
```

- [ ] **Step 5: Commit**

```bash
git add crates/kebab-embed-local/src/lib.rs
git commit -m "feat(embed): add multilingual-e5-large arm to resolve_model (fb-39b)"
```

---

## Task 2: Flip kebab-config default to e5-large + 1024 dim

**Files:**
- Modify: `crates/kebab-config/src/lib.rs`

- [ ] **Step 1: Read existing default test + value sites**

```bash
grep -n "multilingual-e5-small\|dimensions: 384\|dimensions = 384\|default.*embedding" crates/kebab-config/src/lib.rs
```

Three sites to update:
- `Config::defaults()` body (~line 307): `dimensions: 384` and `model: "multilingual-e5-small"`.
- Default-assert test (~line 767): `assert_eq!(c.models.embedding.dimensions, 384)` and likely a sibling assertion on model.
- TOML template at ~line 952: `dimensions = 384` (and likely `model = "multilingual-e5-small"`).

- [ ] **Step 2: Add failing assertion to existing default test**

Find the test at ~line 763-768 (likely `defaults_match_design_64_score_gate` or similar). Read it:

```bash
sed -n '760,780p' crates/kebab-config/src/lib.rs
```

If the test asserts `dimensions == 384`, change to `1024`. If it doesn't assert model name, add:

```rust
    assert_eq!(c.models.embedding.model, "multilingual-e5-large");
    assert_eq!(c.models.embedding.dimensions, 1024);
```

- [ ] **Step 3: Run tests — expect failure**

```bash
cargo test -p kebab-config defaults_match
```
Expected: assertion failure on dimensions == 1024 (still 384) and/or model name.

- [ ] **Step 4: Flip the defaults**

In `crates/kebab-config/src/lib.rs:307` (the `EmbeddingCfg` defaults block):

```rust
EmbeddingCfg {
    provider: "fastembed".to_string(),
    model: "multilingual-e5-large".to_string(),
    version: "v1".to_string(),
    dimensions: 1024,
    // ... preserve other fields (batch_size etc.) ...
}
```

(Read the surrounding lines first to confirm field names — if `version` field doesn't exist or has a different shape, only update `model` + `dimensions`.)

- [ ] **Step 5: Flip the TOML template**

In `crates/kebab-config/src/lib.rs` near line 952, the multi-line raw string contains the example TOML config. Find:

```toml
[models.embedding]
provider = "fastembed"
model = "multilingual-e5-small"
...
dimensions = 384
```

Replace with `model = "multilingual-e5-large"` and `dimensions = 1024`.

- [ ] **Step 6: Run tests — pass**

```bash
cargo test -p kebab-config
cargo clippy -p kebab-config --all-targets -- -D warnings
```

- [ ] **Step 7: Commit**

```bash
git add crates/kebab-config/src/lib.rs
git commit -m "feat(config): default embedding model multilingual-e5-large + 1024 dim (fb-39b)"
```

---

## Task 3: Cross-crate test fixture sweep

**Files:**
- Modify: any test fixture broken by Task 2's default flip.

- [ ] **Step 1: Find broken sites**

```bash
cargo build --workspace 2>&1 | tail -10
cargo test --workspace --no-run 2>&1 | grep -E "error\[|FAILED" | head -20
```

Likely candidates:
- `crates/kebab-app/tests/` — anywhere a test asserted `embedding.dimensions == 384`.
- `crates/kebab-cli/tests/cli_schema.rs` — a capability/model assertion may include the embedding model name.

For each failure, decide:
- **Pin to small intentionally** (test exercises small-specific behavior): set `cfg.models.embedding.model = "multilingual-e5-small"; cfg.models.embedding.dimensions = 384;` explicitly.
- **Inherit new default** (test just snapshots defaults): update assertion to `multilingual-e5-large` / `1024`.

The vast majority of integration tests use `provider = "none"` (no embeddings) — those are unaffected.

- [ ] **Step 2: Verify workspace builds**

```bash
cargo build --workspace 2>&1 | tail -5
```

- [ ] **Step 3: Run workspace tests**

```bash
cargo test --workspace --no-fail-fast -j 1 2>&1 | tail -10
cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -5
```

`-j 1` REQUIRED.

Expected: all green.

- [ ] **Step 4: Commit**

```bash
git add crates/
git commit -m "fix(fb-39b): update test fixtures for embedding default flip"
```

(Skip this commit if `cargo build --workspace` is already clean after Task 2 — meaning no fixture broke.)

---

## Task 4: Wire schema docs (design + HOTFIXES + new task spec)

**Files:**
- Modify: `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md`
- Modify: `tasks/HOTFIXES.md`
- Create: `tasks/p9/p9-fb-39b-embedding-upgrade.md`
- Modify: `tasks/p9/p9-fb-39-retrieval-precision-tuning.md`
- Modify: `tasks/INDEX.md`

- [ ] **Step 1: Update design §5 storage and §9 versioning**

```bash
grep -n "multilingual-e5-small\|^## §5\|^### §5\|^## §9\|384" docs/superpowers/specs/2026-04-27-kebab-final-form-design.md | head -10
```

Update any reference to `multilingual-e5-small` or `dim 384` in the design doc to read `multilingual-e5-large` and `dim 1024`. Keep historical version mentions intact (e.g. "0.6.0 shipped with multilingual-e5-small") if any — but the "current default" line must reflect the new model.

- [ ] **Step 2: Add HOTFIXES entry**

Append to `tasks/HOTFIXES.md` (under the dated log; place at top of the dated entries with today's date `2026-05-10`):

```markdown
- **2026-05-10 fb-39b — embedding upgrade UX**: default embedding flipped from `multilingual-e5-small` (384 dim) to `multilingual-e5-large` (1024 dim). LanceDB tables are namespaced by `(model, dim)` so the new model writes to a fresh table and the old `chunk_embeddings_multilingual-e5-small_384` table becomes orphan. fb-23 incremental ingest auto-re-embeds chunks (embedding_version mismatch) into the new table on next `kebab ingest`. To free disk before re-ingest, run `kebab reset --vector-only` first — this wipes both LanceDB and the SQLite `embedding_records` table. Search/ask against the new model returns empty hits until `kebab ingest` populates the new table.
```

- [ ] **Step 3: Create `tasks/p9/p9-fb-39b-embedding-upgrade.md`**

Mirror the fb-39 frontmatter shape:

```markdown
---
phase: P9
component: kebab-embed-local + kebab-config + kebab-store-vector + docs
task_id: p9-fb-39b
title: "Embedding model upgrade (multilingual-e5-large)"
status: completed
target_version: 0.7.0
depends_on: [p9-fb-39]
unblocks: []
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§4 search, §5 storage, §9 versioning cascade]
source_feedback: 사용자 도그푸딩 2026-05-06 — Claude Code 가 kebab CLI 사용 후 "rank 5+ 노이즈 섞임" 지적 (fb-39 의 lever 적용 측면).
---

# p9-fb-39b — Embedding model upgrade

> ✅ **구현 완료.** fb-39 의 lever 후보 4개 중 embedding model 업그레이드 lever 적용. P@k metric (fb-39) 으로 small vs large 비교 가능.
>
> - Design: [`docs/superpowers/specs/2026-05-10-p9-fb-39b-embedding-upgrade-design.md`](../../docs/superpowers/specs/2026-05-10-p9-fb-39b-embedding-upgrade-design.md)
> - Plan: [`docs/superpowers/plans/2026-05-10-p9-fb-39b-embedding-upgrade.md`](../../docs/superpowers/plans/2026-05-10-p9-fb-39b-embedding-upgrade.md)

## 요약

- `multilingual-e5-small` (384 dim) → `multilingual-e5-large` (1024 dim) default flip.
- 기존 user TOML 이 small 명시 시 그대로 (backwards-compat).
- fb-23 incremental ingest 가 embedding_version mismatch 감지 → 자동 re-embed.
- 0.6 → 0.7 minor bump 트리거 (design §9 cascade rule).
```

- [ ] **Step 4: Append fb-39b note to fb-39 task spec banner**

In `tasks/p9/p9-fb-39-retrieval-precision-tuning.md`, find the existing `> ✅ **Eval foundation 부분 구현 완료.**` banner. Append a line:

```markdown
> - fb-39b (lever 적용 — embedding upgrade): [`tasks/p9/p9-fb-39b-embedding-upgrade.md`](./p9-fb-39b-embedding-upgrade.md) ✅
```

- [ ] **Step 5: Add fb-39b row to INDEX**

In `tasks/INDEX.md`, find the fb-39 row. Add a sibling row immediately below:

```markdown
    - [p9-fb-39b embedding upgrade](p9/p9-fb-39b-embedding-upgrade.md) — ✅ 머지 (2026-05-10) — multilingual-e5-large default
```

(Adapt format to match neighbor rows.)

- [ ] **Step 6: Workspace test + clippy gate**

```bash
cargo test --workspace --no-fail-fast -j 1 2>&1 | tail -10
cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -5
```

`-j 1` REQUIRED.

- [ ] **Step 7: Commit**

```bash
git add docs/ tasks/
git commit -m "docs(fb-39b): design + HOTFIXES + new task spec + INDEX"
```

---

## Task 5: README + SMOKE walkthrough

**Files:**
- Modify: `README.md`
- Modify: `docs/SMOKE.md`

- [ ] **Step 1: Update README `[models.embedding]` section**

```bash
grep -n "models.embedding\|multilingual-e5-small\|fastembed" README.md | head -5
```

Locate the `[models.embedding]` config block in README. Update default values mentioned + add new bullet:

```markdown
- `model` (default `"multilingual-e5-large"`, fb-39b) — 다국어 sentence embedding 모델. 1024-dim. ONNX (~1.3 GB) 첫 실행 시 fastembed cache (`config.storage.model_dir/fastembed/`) 에 자동 다운로드. `"multilingual-e5-small"` (384 dim) 는 backwards-compat 으로 사용 가능 — TOML 에 명시.
- `dimensions` (default `1024`) — 모델의 embedding 차원. config 와 LanceDB stored dim 불일치 시 검색 결과 0 건 (orphan table). 모델 변경 시 `kebab reset --vector-only && kebab ingest` 로 vector index 재구축 권장.
```

- [ ] **Step 2: Append SMOKE walkthrough**

Append to `docs/SMOKE.md` after fb-39 section (or at end if absent):

````markdown
### Embedding upgrade (fb-39b)

`multilingual-e5-small` 에서 `multilingual-e5-large` 로 업그레이드 시퀀스:

```bash
# 기존 vector index 정리 (orphan table 회피)
kebab --config /tmp/kebab-smoke/config.toml reset --vector-only

# config.toml 의 [models.embedding] 갱신:
#   model = "multilingual-e5-large"
#   dimensions = 1024

# 재-ingest — fastembed 가 첫 실행 시 e5-large ONNX (~1.3 GB) 자동 다운로드.
# 다운로드 시간 + 모든 chunk re-embed 시간 (e5-small 대비 ~3-4×).
kebab --config /tmp/kebab-smoke/config.toml ingest

# fb-39 의 P@k metric 으로 small vs large 비교:
kebab --config /tmp/kebab-smoke/config.toml eval run
```
````

- [ ] **Step 3: Workspace test + clippy gate (sanity)**

```bash
cargo test --workspace --no-fail-fast -j 1 2>&1 | tail -5
cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -3
```

- [ ] **Step 4: Commit**

```bash
git add README.md docs/SMOKE.md
git commit -m "docs(fb-39b): README + SMOKE — embedding upgrade walkthrough"
```

---

## Final verification checklist

- [ ] `cargo test --workspace --no-fail-fast -j 1` green
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` clean
- [ ] `kebab schema --json | jq .models.embedding_version` reflects new model name (after a fresh ingest with new defaults)
- [ ] Manual smoke: `kebab reset --vector-only && kebab ingest` against `/tmp/kebab-smoke` triggers ONNX download (first run) then completes ingest into the new `chunk_embeddings_multilingual-e5-large_1024` table
- [ ] README + SMOKE + design + HOTFIXES + fb-39b spec + INDEX all updated
- [ ] **Post-merge**: cut version bump 0.6 → 0.7 + tag (CLAUDE.md `Versioning cascade` release rule — embedding_version cascade triggers minor bump)
