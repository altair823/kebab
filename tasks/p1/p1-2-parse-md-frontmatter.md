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
