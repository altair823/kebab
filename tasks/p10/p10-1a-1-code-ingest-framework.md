# p10-1A-1 — code ingest framework

**Status:** 🟡 진행 중
**Contract sections:** §2.1 (Citation `code` variant), §2.2 (SearchHit repo/code_lang), §2.4 (IngestReport skip counters), §2 schema.v1 (code_lang_breakdown + repo_breakdown), §3.6 (Metadata fields), §8 (kebab-parse-code crate boundary), §11 (code ingest no longer 비-스코프).
**Design:** [2026-05-15-kebab-code-ingest-design.md](../../docs/superpowers/specs/2026-05-15-kebab-code-ingest-design.md) §1A-1.
**Plan:** [2026-05-15-p10-1a-1-code-ingest-framework.md](../../docs/superpowers/plans/2026-05-15-p10-1a-1-code-ingest-framework.md).

## Goal

Land the *framework surface* for code ingest — wire schema (additive minor), CLI filter flags, ignore policy, skip policy infrastructure, `kebab-parse-code` crate skeleton, `[ingest.code]` config section — without enabling any code chunker. 1A-2 plugs in the Rust AST chunker on top.

## Acceptance criteria

- `cargo test --workspace --no-fail-fast -j 1` passes.
- Regression test (`wire_search_hit_no_code_fields`, `wire_citation_5_variants_unchanged`) passes — markdown corpus wire output unchanged.
- `cargo clippy --workspace --all-targets -- -D warnings` passes.
- `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` updated per design §10.1.
- README + HANDOFF + SMOKE updated.

## Allowed dependencies

- `kebab-parse-code` may depend on `kebab-core`, `anyhow`, `gix`. NOT on store / embed / llm / rag / UI.
- Source-fs may depend on `kebab-parse-code`.

## Forbidden dependencies

- UI crates (cli / mcp / tui) must NOT import `kebab-parse-code` directly.

## Risks / notes

- `.gitignore` honor changes existing behavior for markdown corpora whose files live in gitignored areas. Regression test covers the standard case (no overlap). If a user reports missing docs after 1A-1 lands, log to HOTFIXES.
