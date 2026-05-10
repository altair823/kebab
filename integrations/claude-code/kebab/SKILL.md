---
name: kebab
description: Local knowledge base + RAG over the user's pre-indexed documents (wiki crawls, Markdown notes, PDFs, images). Use when answering questions that need internal context the user has indexed locally — e.g. team-specific procedures, internal runbooks, infrastructure docs, credentials registries, project-specific conventions. Also use when a domain question (Kubernetes, MLOps, internal tooling, etc.) needs additional grounding from indexed docs before answering. Do NOT use for general public questions, code in the working directory, or anything obviously outside the indexed corpus.
---

# kebab — local KB / RAG access

`kebab` indexes the user's personal documents and exposes them via lexical / vector / hybrid search and a local-LLM RAG answer. All output speaks frozen wire schema v1 — every JSON record carries a `schema_version` field.

Two surfaces ship: an **MCP server** (`kebab mcp`, preferred — process stays hot across calls) and a **CLI** (`~/.cargo/bin/kebab`, fallback for hosts without MCP).

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

## MCP tools (preferred)

When `kebab` is registered as an MCP server (see `~/.claude/mcp.json` example below), eight tools are exposed as `mcp__kebab__<name>`:

| tool | purpose | mutation |
|------|---------|----------|
| `mcp__kebab__search` | corpus search → `search_response.v1` (`{hits, next_cursor, truncated}`) | no |
| `mcp__kebab__bulk_search` | N queries in one call → `bulk_search_response.v1` (`{results, summary}`) | no |
| `mcp__kebab__ask` | RAG answer → `answer.v1` | no |
| `mcp__kebab__fetch` | verbatim text → `fetch_result.v1` (chunk / doc / span) | no |
| `mcp__kebab__schema` | capability discovery → `schema.v1` | no |
| `mcp__kebab__doctor` | health check → `doctor.v1` | no |
| `mcp__kebab__ingest_file` | save single file → `ingest_report.v1` | yes |
| `mcp__kebab__ingest_stdin` | save markdown blob → `ingest_report.v1` | yes |

Mutation tools require explicit user intent — never auto-invoke.

### `mcp__kebab__search` — when you need the source

Use when the user wants to **find** a doc, or when you (the model) need raw chunks to reason from before answering.

Input:
```json
{ "query": "<query>", "mode": "hybrid", "k": 10, "max_tokens": null, "snippet_chars": null, "cursor": null, "tags": null, "lang": null, "path_glob": null, "trust_min": null, "media": null, "ingested_after": null, "doc_id": null, "trace": null }
```

- `mode = "hybrid"` is the default-correct choice. Use `"vector"` for semantic-only ("docs about X concept"), `"lexical"` for exact strings ("the literal flag `--foo-bar`").
- **`max_tokens` / `snippet_chars` / `cursor` (p9-fb-34)** — agent budget controls. Set `max_tokens` to cap result wire size (chars/4 estimate); set `cursor` to the previous response's `next_cursor` to fetch the next page.
- **p9-fb-36 filter inputs:** `tags` (string array — OR-within, AND across keys), `lang` (BCP-47 language code), `path_glob` (glob pattern matched against doc path), `trust_min` (`"primary"` | `"secondary"` | `"generated"` — includes that level and above), `media` (string array — IN-list of `"markdown"` | `"pdf"` | `"image"` | `"audio"` | `"other"`; alias `"md"` → `"markdown"`), `ingested_after` (RFC3339 UTC string), `doc_id` (exact doc UUID). AND combinator across keys. Invalid `ingested_after` or unknown `trust_min` → `error.v1.code = invalid_input`. Unknown `media` value → empty hits, no error.
- Output is `search_response.v1`: `{ hits: search_hit.v1[], next_cursor: string|null, truncated: bool }`. Iterate `response.hits[]` for individual hits. Key hit fields: `rank`, `score`, `doc_path`, `heading_path[]`, `section_label`, `snippet`, `citation` (line range / page), `chunk_id`.
- **`hits[].score_kind` (p9-fb-38):** `"rrf"` (hybrid) / `"bm25"` (lexical) / `"cosine"` (vector). Declares the meaning of the top-level `score` — it is a **ranking signal**, not a confidence value. If you need a trust threshold, use `retrieval.lexical_score` (BM25 raw) / `retrieval.vector_score` (cosine raw) instead of the top-level `score`.
- Cite back to the user as `doc_path § heading_path[-1]` so they can open the source.
- When `truncated: true`, the budget loop modified the page (snippet shortening or k reduction). `next_cursor` is **independent** — non-null whenever more hits may be reachable. Caller may widen `max_tokens` (re-issue same query for fuller snippets / more hits per page) or follow `next_cursor` (advance through more hits) or both. Mismatched cursor (corpus_revision changed) returns `error.v1.code = stale_cursor` — re-issue the search to obtain a fresh one.
- **`trace: true` (p9-fb-37)** — debug aid. Response carries an extra `trace` block: `lexical[]` + `vector[]` (pre-fusion candidates), `rrf_inputs[]` (RRF union before final cut), and `timing` (`lexical_ms`, `vector_ms`, `fusion_ms`, `total_ms`). Trace bypasses the search cache (always cold). Use sparingly — it bloats the wire response and is for diagnosing "why did this hit / not hit", not normal retrieval.

### `mcp__kebab__bulk_search`

N개 query 한 번에 — agent loop 효율 개선. 각 query 는 `mcp__kebab__search` 와 동일 input shape (query 필수, 나머지 optional). Cap 100.

Input:
```json
{"queries": [{"query": "..."}, {"query": "...", "mode": "lexical"}, ...]}
```

Output: `bulk_search_response.v1` envelope — `results: [bulk_search_item.v1]` (각 item = `{query, response | null, error | null}`) + `summary: {total, succeeded, failed}`. Per-query 실패는 item 의 error 에 격리, 다른 query 계속 진행.

### `mcp__kebab__ask` — when you need the answer

Use when the user wants a synthesized answer, not a list of links.

Input:
```json
{ "query": "<question>", "session_id": "<optional-slug>", "mode": "hybrid" }
```

- Returns `answer.v1`: `answer` (markdown), `citations[]`, `grounded` (bool), `refusal_reason`, `model`, `conversation_id`, `turn_index`.
- **If `grounded == false`** → KB doesn't have enough context. Don't paraphrase the refusal as if it were an answer. Tell the user the KB came up dry and fall back to your own knowledge or ask for the source.
- For follow-up turns on the same topic, pass `session_id` (e.g. `"team-onboarding-2026-05"`) and reuse it across the conversation. Sessions persist until `kebab reset --data-only`.
- p9-fb-40: 기본 `prompt_template_version = "rag-v2"`. 답변이 더 strict — fact 인용 시 verbatim span, 학습 지식 동원 금지, 근거 모호 시 "확실하지 않다" 출현 가능. user 가 `[rag] prompt_template_version = "rag-v1"` 명시 시 legacy 동작.

### `mcp__kebab__fetch` — when you need raw text

Use after `search` to read the verbatim chunk text + surrounding context, or to pull a full doc / line range.

Input:
```json
{ "kind": "chunk", "chunk_id": "<id>", "context": 2 }
{ "kind": "doc", "doc_id": "<id>", "max_tokens": 1000 }
{ "kind": "span", "doc_id": "<id>", "line_start": 1, "line_end": 5 }
```

- `chunk` mode: `context: N` returns ordinal-adjacent chunks before/after for surrounding paragraphs.
- `doc` mode: full normalized markdown. `max_tokens` (chars/4) caps the response — `truncated: true` when applied.
- `span` mode: 1-based inclusive line range. PDF / audio docs reject as `error.v1.code = span_not_supported` (use `chunk` mode instead — PDF chunks are page-aligned).
- `error.v1.code = chunk_not_found` / `doc_not_found` are non-retryable from the same id — re-issue search to get a fresh one.

## CLI fallback

If MCP tools aren't in scope (host without MCP support, or `mcp.json` not configured), call the CLI via Bash:

```bash
kebab search "<query>" --mode hybrid --json 2>/dev/null
kebab ask "<question>" --json 2>/dev/null
kebab ask "<question>" --session <stable-id> --json 2>/dev/null
kebab ask "<question>" --stream  # ndjson answer_event.v1 on stderr, final answer.v1 on stdout
```

Same wire shapes as MCP. CLI pays cold start (~1-2s) per call — prefer MCP when available.

`--stream` (p9-fb-33) emits `retrieval_done` → `token`* → `final` events on stderr while the answer streams; the final stdout line is the standard `answer.v1` for backwards compat. Use when you need progressive token consumption; otherwise the default non-streaming path is simpler. Refusal paths (score-gate / no-chunks) emit `retrieval_done` then no `token`/`final` — read stdout `answer.v1` for the canonical refusal signal.

## MCP host config

Register `kebab mcp` once in your host's MCP config. For Claude Code, edit `~/.claude/mcp.json`:

```json
{
  "mcpServers": {
    "kebab": {
      "command": "kebab",
      "args": ["mcp"]
    }
  }
}
```

Claude Code spawns `kebab mcp` at session start; the process stays alive across all tool calls so SQLite / Lance / fastembed are hot after the first call (~1-2s cold, sub-100ms thereafter). For Cursor / OpenAI Agents / Copilot CLI host examples plus per-tool input/output reference, see [docs/mcp-usage.md](../../../docs/mcp-usage.md) in the kebab repo.

## Parsing tips

- MCP tools return JSON content blocks; CLI prints **one JSON value to stdout**, progress / warnings to stderr. Capture stdout only: `kebab search ... --json 2>/dev/null`.
- `search` output can be large for broad queries. Project relevant fields when summarizing — for CLI: `jq '.hits[] | {rank, doc_path, heading: .heading_path[-1], snippet}'` (note: `.hits[]`, not `.[]` — fb-34 wrapped the array). Use `--max-tokens N` (CLI) / `max_tokens` (MCP) to cap wire size in advance.
- Pagination: `search_response.v1.next_cursor` is opaque base64 — pass back as `--cursor` (CLI) or `cursor` (MCP) for the next page. `null` means no more hits. `corpus_revision` mismatch returns `error.v1.code = stale_cursor` — re-issue search to obtain a fresh cursor.
- `search_response.v1.truncated = true` means budget forced snippet shortening or k reduction. Independent of `next_cursor`: widen `max_tokens` for fuller snippets, follow `next_cursor` for more hits, or both.
- `ask`'s `citations[]` mirrors `search_hit.v1` minus retrieval internals — same `doc_path` / `citation` shape.
- Schema reference lives in the kebab repo at `docs/wire-schema/v1/*.schema.json` if a field is unclear.
- `search_hit.v1` and `answer.v1.citations[]` carry `indexed_at` (RFC3339) + `stale` (bool). When `stale == true`, the source doc hasn't been re-processed since `config.search.stale_threshold_days`. Surface this caveat to the user when summarizing — the cited snapshot may not reflect current reality.

## Capability discovery

Before using streaming or multi-turn features, probe what this binary supports — call `mcp__kebab__schema` (or CLI `kebab schema --json`):

Returns `schema.v1`: `wire.schemas` (supported wire ids), `capabilities` (bool flags — e.g. `streaming_ask`, `rag_multi_turn`), `models` (version cascade 6-axis), `stats` (doc/chunk/asset count + last_ingest_at, plus p9-fb-37 health surface: `media_breakdown` per-kind doc counts (5 zero-padded keys: markdown / pdf / image / audio / other), `lang_breakdown` per BCP-47 lang (NULL keyed as the literal string `"null"`), `index_bytes.{sqlite,lancedb}` on-disk byte sums, `stale_doc_count` for docs older than `config.search.stale_threshold_days`). Gate streaming / session flows on `capabilities.streaming_ask` / `capabilities.rag_multi_turn` being `true`. Cheap call (no LLM), once per session.

## Quick health check

If a call fails or returns suspicious output, call `mcp__kebab__doctor` (or CLI `kebab doctor`) first — it surfaces config-load / data-dir / Ollama-reachability problems in one line each. Don't silently retry on errors; report the doctor output.

## Workflow recipes

**Recipe A — user asks an internal-context question, you want grounded answer:**

1. Call `mcp__kebab__ask` (or CLI `kebab ask "<question>" --json`).
2. If `grounded`, cite `citations[].doc_path` in your reply and quote the `answer` (translate / condense as needed).
3. If `!grounded`, call `mcp__kebab__search` with the same query and look at top 3 hits — sometimes content exists but RAG threshold rejected it. If hits look relevant, summarize from snippets and cite. If still nothing, tell the user.

**Recipe B — domain question where internal context might exist:**

1. Call `mcp__kebab__search` with key terms (cheap — no LLM).
2. If top hit's `score` is low (< ~0.3) or no hits, answer from general knowledge without mentioning the KB.
3. If top hit is relevant, fold its content into your answer and cite `doc_path`.

**Recipe C — user wants to know "what's in the KB about X":**

1. Call `mcp__kebab__search` with the topic.
2. List unique `doc_path`s back to the user as a discovery surface.

**Recipe D — agent fetched a web doc, save to KB:**

When you've fetched a markdown article (e.g. via WebFetch) that the user might query later:

1. Call `mcp__kebab__ingest_stdin` with:
   - `content`: the markdown body
   - `title`: a stable title (article H1 or page title)
   - `source_uri`: the URL you fetched from

The doc lands in `<workspace.root>/_external/<hash>.md` and is indexed for `search` / `ask` immediately. Subsequent calls with identical content are no-ops (content-hash dedup).

Don't loop ingest the same article — dedup makes it safe but wastes embedding cost.

For files already on disk the user references, prefer `mcp__kebab__ingest_file` with the path — kebab handles the copy + dedup.

## Don't

- Don't auto-invoke `mcp__kebab__ingest_file` / `mcp__kebab__ingest_stdin` / `kebab ingest` / `kebab reset` / `kebab init`. Those mutate state — the user must explicitly request.
- Don't pass user-supplied raw text into the query without trimming — long queries (> a few hundred chars) waste embedding budget. Extract the question.
- Don't fabricate `doc_path`s. If you didn't see a doc in `search` / `ask` output, it's not in the KB.
- Don't use `kebab tui` from a skill — it's interactive only.
