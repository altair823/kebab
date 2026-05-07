---
name: kebab
description: Local knowledge base + RAG over the user's pre-indexed documents (wiki crawls, Markdown notes, PDFs, images). Use when answering questions that need internal context the user has indexed locally — e.g. team-specific procedures, internal runbooks, infrastructure docs, credentials registries, project-specific conventions. Also use when a domain question (Kubernetes, MLOps, internal tooling, etc.) needs additional grounding from indexed docs before answering. Do NOT use for general public questions, code in the working directory, or anything obviously outside the indexed corpus.
---

# kebab — local KB / RAG access

`kebab` is a CLI installed at `~/.cargo/bin/kebab` (binary name: `kebab`). It indexes the user's personal documents and exposes them via lexical / vector / hybrid search and a local-LLM RAG answer. All output speaks frozen wire schema v1 — every JSON record carries a `schema_version` field.

## When to invoke

Trigger when the user's question matches **any** of:

- Refers to internal/organization-specific systems, procedures, or jargon that a generic public answer would miss.
- Names a runbook or procedure the user is likely to have indexed ("how do I X", "what's our policy on Y", "where's the doc for Z").
- Domain-technical question where additional internal context (custom CRDs, internal naming, team conventions) would change the answer vs. a generic public answer.
- User explicitly references "the wiki", "내부 문서", "kb", or asks "do we have docs on X".

**Skip** when:

- The question is about public OSS, language semantics, or anything in the current working directory.
- The user is editing kebab's own source — that's a code task, not a KB query.
- A previous `kebab` call in this session already returned `grounded: false` on a near-identical query (don't loop).

User-specific trigger keywords (team names, system names, internal acronyms) belong in a per-user override of this SKILL.md, not in this repo-shipped version.

## Two surfaces, pick the right one

### `kebab search` — when you need the source

Use when the user wants to **find** a doc, or when you (the model) need raw chunks to reason from before answering.

```bash
kebab search "<query>" --mode hybrid --json
```

- `--mode hybrid` is the default-correct choice. Use `vector` for semantic-only ("docs about X concept"), `lexical` for exact strings ("the literal flag `--foo-bar`").
- Output is a JSON array of `search_hit.v1` objects. Key fields: `rank`, `score`, `doc_path`, `heading_path[]`, `section_label`, `snippet`, `citation` (has line range / page), `chunk_id`.
- Cite back to the user as `doc_path § heading_path[-1]` so they can open the source.

### `kebab ask` — when you need the answer

Use when the user wants a synthesized answer, not a list of links.

```bash
kebab ask "<question>" --json
```

- Returns one `answer.v1` object: `answer` (markdown), `citations[]`, `grounded` (bool), `refusal_reason`, `model`.
- **If `grounded == false`** → the KB doesn't have enough context. Don't paraphrase the refusal as if it were an answer. Tell the user the KB came up dry and fall back to your own knowledge or ask for the source.
- For follow-up turns on the same topic, pass `--session <stable-id>` so kebab gets prior history. Pick a slug (`team-onboarding-2026-05`) and reuse it across the conversation. Sessions persist across Claude sessions until `kebab reset --data-only`.

## Parsing tips

- Both commands print **one JSON value to stdout**, progress / warnings to stderr. Capture stdout only: `kebab search ... --json 2>/dev/null`.
- `search --json` output can be large for broad queries. Pipe through `jq` to project: `jq '.[] | {rank, doc_path, heading: .heading_path[-1], snippet}'`.
- `ask --json`'s `citations[]` mirrors `search_hit.v1` minus retrieval internals — same `doc_path` / `citation` shape.
- Schema reference lives in the kebab repo at `docs/wire-schema/v1/*.schema.json` if a field is unclear.

## Capability discovery

Before using streaming or multi-turn features, you can probe what this binary supports:

```bash
kebab schema --json
```

Returns a `schema.v1` object with: `wire.schemas` (supported wire ids), `capabilities` (bool flags — e.g. `multi_turn`, `streaming_ingest`), `models` (version cascade 6-axis), and `stats` (doc/chunk/asset count + last_ingest_at). Gate streaming / session flows on `capabilities.streaming_ingest` / `capabilities.multi_turn` being `true`. This call is cheap (no LLM) and can be run once per session.

## Quick health check

If a call fails or returns suspicious output, run `kebab doctor` first — it surfaces config-load / data-dir / Ollama-reachability problems in one line each. Don't silently retry on errors; report the doctor output.

## Workflow recipes

**Recipe A — user asks an internal-context question, you want grounded answer:**

1. `kebab ask "<question>" --json`
2. If `grounded`, cite `citations[].doc_path` in your reply and quote the user's `answer` (translate / condense as needed).
3. If `!grounded`, switch to `kebab search "<question>" --mode hybrid --json` and look at top 3 hits — sometimes content exists but RAG threshold rejected it. If hits look relevant, summarize from snippets and cite. If still nothing, tell the user.

**Recipe B — domain question where internal context might exist:**

1. Run `kebab search "<key terms>" --mode hybrid --json` quickly (cheap, no LLM).
2. If top hit's `score` is low (< ~0.3) or no hits, answer from general knowledge without mentioning the KB.
3. If top hit is relevant, fold its content into your answer and cite `doc_path`.

**Recipe C — user wants to know "what's in the KB about X":**

1. `kebab search "X" --mode hybrid --json | jq '.[] | {doc_path, heading: .heading_path[-1]}'`
2. List unique `doc_path`s back to the user as a discovery surface.

## Don't

- Don't run `kebab ingest` / `kebab reset` / `kebab init` automatically. Those mutate state — the user runs them.
- Don't pass user-supplied raw text into the query without trimming — long queries (> a few hundred chars) waste embedding budget. Extract the question.
- Don't fabricate `doc_path`s. If you didn't see a doc in `search` / `ask` output, it's not in the KB.
- Don't use `kebab tui` from a skill — it's interactive only.
