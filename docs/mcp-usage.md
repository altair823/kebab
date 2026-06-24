# MCP usage — agent integration guide

`kebab mcp` runs an MCP (Model Context Protocol) stdio JSON-RPC server. agent host (Claude Code / Cursor / OpenAI Agents / Copilot CLI 등) 가 본 binary 를 spawn 하여 KB 검색 / 답변 / ingest 를 호출.

shipped since **v0.3.1** (fb-30). 6 tool 으로 확장 (v0.3.2, fb-31).

---

## Quick start

binary 를 PATH 에 두고 (`cargo install --path crates/kebab-cli` 또는 release tarball), agent host 의 mcp config 에 등록:

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

session 시작 시 host 가 `kebab mcp` 를 spawn — process 가 session 동안 살아 있어 SQLite / Lance / fastembed 가 hot. 첫 tool call 만 cold-start 비용, 이후 sub-100ms.

`--config` 옵션 thread:

```json
{
  "mcpServers": {
    "kebab": {
      "command": "kebab",
      "args": ["--config", "/Users/me/.config/kebab/agent.toml", "mcp"]
    }
  }
}
```

---

## Host config 예시

### Claude Code

`~/.claude/mcp.json` (또는 OS 별 동등 위치):

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

session 재시작 후 `kebab` server 가 tool list 에 등장. agent 가 `mcp__kebab__search` / `mcp__kebab__ask` 등 호출 가능.

### Cursor

`~/.cursor/mcp.json`:

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

Cursor 의 Composer / Agent 모드에서 활성화.

### OpenAI Agents (`agents-sdk`)

Python:

```python
from openai_agents import Agent, MCPServerStdio

kebab = MCPServerStdio(
    name="kebab",
    params={"command": "kebab", "args": ["mcp"]},
)

agent = Agent(
    name="researcher",
    mcp_servers=[kebab],
)
```

Node:

```ts
import { Agent, MCPServerStdio } from "openai-agents";

const kebab = new MCPServerStdio({
  name: "kebab",
  params: { command: "kebab", args: ["mcp"] },
});

const agent = new Agent({ name: "researcher", mcpServers: [kebab] });
```

### Copilot CLI

`~/.config/copilot-cli/mcp.json` (or wherever the CLI looks):

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

### 기타 host

stdio JSON-RPC MCP 표준을 따르는 모든 host 가 지원. 위 형식 (`command` + `args`) 만 맞추면 동작.

---

## Tool catalog (6 tools)

모든 tool 의 출력은 wire schema v1 JSON 을 MCP `text` content block 으로 직렬화. CLI `--json` 모드와 byte-동일 (single source of truth).

### `search` — corpus 검색

| | |
|---|---|
| Input | `{ "query": string, "mode"?: "lexical"\|"vector"\|"hybrid", "k"?: 1-100 }` |
| Defaults | `mode = "hybrid"`, `k = 10` |
| Output | `search_hit.v1` array, ranked |

예시:

```json
{
  "name": "search",
  "arguments": {
    "query": "Kubernetes ingress controller setup",
    "mode": "hybrid",
    "k": 5
  }
}
```

응답 (한 hit 발췌):

```json
[
  {
    "schema_version": "search_hit.v1",
    "rank": 1,
    "score": 0.847,
    "doc_id": "...",
    "chunk_id": "...",
    "doc_path": "k8s/ingress.md",
    "heading_path": ["Setup", "Ingress controller"],
    "snippet": "...",
    "citation": { ... }
  },
  ...
]
```

**언제 사용**: 사용자가 \"문서 어디 있는지\" 묻거나, agent 가 답변 전 raw chunk 가 필요할 때.

### `ask` — RAG 답변

| | |
|---|---|
| Input | `{ "query": string, "mode"?: "lexical"\|"vector"\|"hybrid" }` |
| Defaults | `mode = "hybrid"` |
| Output | `answer.v1` (single object) |

예시:

```json
{
  "name": "ask",
  "arguments": {
    "query": "What's our internal Kubernetes ingress setup?"
  }
}
```

응답:

```json
{
  "schema_version": "answer.v1",
  "answer": "...",
  "citations": [ ... ],
  "grounded": true,
  "refusal_reason": null,
  "model": { ... }
}
```

**`grounded: false` 처리**: KB 에 충분한 context 없음. `refusal_reason` 확인 후 사용자에게 \"KB 에 정보 없음\" 으로 안내, 본인 지식 fallback 또는 source 요청. **paraphrase 하면 안 됨** (hallucination 위험).

### `schema` — capability discovery

| | |
|---|---|
| Input | `{}` (no args) |
| Output | `schema.v1` |

예시:

```json
{ "name": "schema", "arguments": {} }
```

응답:

```json
{
  "schema_version": "schema.v1",
  "kebab_version": "0.3.2",
  "wire": { "schemas": ["answer.v1", "search_hit.v1", ...] },
  "capabilities": {
    "json_mode": true,
    "rag_multi_turn": false,
    "mcp_server": true,
    "streaming_ask": false,
    ...
  },
  "models": { "parser_version": "...", "embedding_version": "...", ... },
  "stats": { "doc_count": 128, "chunk_count": 2147, "asset_count": 130, ... }
}
```

**언제 사용**: session 시작 시 한 번 — feature gate 결정 (`capabilities.streaming_ask` true 면 streaming 사용 등). cheap call (no LLM, no embedder), session 동안 1 회 충분.

### `doctor` — health check

| | |
|---|---|
| Input | `{}` (no args) |
| Output | `doctor.v1` |

예시:

```json
{ "name": "doctor", "arguments": {} }
```

응답:

```json
{
  "schema_version": "doctor.v1",
  "ok": true,
  "checks": [
    { "name": "config_loaded", "ok": true, "detail": "..." },
    { "name": "ollama_reachable", "ok": true, "detail": "..." },
    ...
  ]
}
```

**언제 사용**: 다른 tool 이 실패하거나 비정상 응답 줄 때 first triage. `ok: false` 면 `checks[]` 의 failed entry 가 원인 — 사용자에게 보고 후 stop (자동 retry 금지).

### `ingest_file` — 단일 파일 저장 (mutation)

| | |
|---|---|
| Input | `{ "path": string }` |
| Supported ext | `.md` / `.pdf` / `.png` / `.jpg` / `.jpeg` (`unsupported extension` error 그 외) |
| Output | `ingest_report.v1` (single asset) |

예시:

```json
{
  "name": "ingest_file",
  "arguments": { "path": "/Users/me/Downloads/article.md" }
}
```

응답:

```json
{
  "schema_version": "ingest_report.v1",
  "scanned": 1,
  "new": 1,
  "updated": 0,
  "unchanged": 0,
  "skipped": 0,
  "errors": 0,
  ...
}
```

**언제 사용**: 사용자가 disk 의 file 을 KB 에 저장 의향 명시 시. workspace 외부 path OK — 파일은 `<workspace.root>/_external/<hash12>.<ext>` 으로 copy. 동일 content 재 ingest 면 idempotent (`unchanged: 1`).

**주의**: mutation tool — 사용자 명시 의도 없을 때 자동 호출 금지.

### `ingest_stdin` — stdin markdown 저장 (mutation)

| | |
|---|---|
| Input | `{ "content": string, "title": string, "source_uri"?: string }` |
| v1 scope | markdown only |
| Output | `ingest_report.v1` (single asset) |

예시:

```json
{
  "name": "ingest_stdin",
  "arguments": {
    "content": "## Article body\n\nMain text here.",
    "title": "Article X",
    "source_uri": "https://example.com/x"
  }
}
```

응답:

```json
{
  "schema_version": "ingest_report.v1",
  "scanned": 1,
  "new": 1,
  ...
}
```

**언제 사용**: agent 가 web fetch 한 markdown article 을 KB 에 저장. 사용자가 \"이거 나중에 또 보고 싶어\" 명시 시 또는 multi-turn 대화에서 자료 누적. content 가 이미 frontmatter (`---` 시작) 이면 error — `ingest_file` 사용.

`title` + `source_uri` 가 frontmatter 로 자동 prepend → `Document.metadata` 에 저장 → 후속 `search` 결과의 `doc_meta` 에 포함. agent 가 source URL 추적 가능.

**주의**: mutation tool. 같은 content 무한 ingest 안 함 (idempotent 보장이지만 embedding cost 낭비).

---

## Troubleshooting

### `isError: true` + `error.v1` content

tool dispatch 가 `Err` 반환 시. content 의 `error.v1` JSON 의 `code` 로 분기:

| code | 의미 | 조치 |
|------|------|------|
| `config_invalid` | `--config` path missing / TOML parse 실패 | path 확인 + `kebab schema` 로 검증. `details.path` + `details.cause` 확인. |
| `not_indexed` | `kebab.sqlite` 미존재 / migration 미실행 | 사용자에게 `kebab init` + `kebab ingest` 실행 안내. retry 자동 금지. |
| `model_unreachable` | Ollama endpoint 연결 실패 | Ollama 실행 확인 (`ollama serve`). `details.endpoint` 의 host 가 reachable 한지. retry 1-2 회 후 사용자 보고. |
| `model_not_pulled` | Ollama model not found | 사용자에게 `ollama pull <model>` 안내 — `details.model` 표시. |
| `timeout` | LLM stream / embed deadline 초과 | 일시적이면 retry 1 회. 재발 시 사용자 보고 (model 응답 느림 / Ollama load). |
| `io_error` | filesystem / 권한 / disk full | `details.kind` 보고 사용자에게 disk space / permission 확인 안내. |
| `generic` | catch-all | `details.chain` (verbose 시) 보고 사용자에게 그대로 전달. retry 금지. |

`hint` field 가 있으면 사용자에게 그대로 보여주기 (각 code 의 가장 빠른 조치).

### `grounded: false` (ask refusal)

`isError: false` (정상 응답). KB 에 충분한 context 없음. `refusal_reason` 확인 후:

- `NoChunks` — 검색 자체가 0 hit. 다른 표현 / 더 일반적인 query 시도.
- `LowScores` — hit 있지만 score gate 미달. `kebab search` (별도) 로 raw hit 확인.
- 그 외 — refusal 메시지 그대로 사용자에게 보고.

자동 paraphrase 금지. 사용자에게 \"KB 에 정보 없음\" 명시 후 본인 지식 또는 source 요청.

### `doctor` `ok: false`

다른 tool 호출 전 `doctor` 부터. `checks[]` 의 failed entry 원인 명시 — 사용자에게 보고 후 stop.

### empty `search` result

`isError: false`, content = `[]` (빈 array). KB 에 매칭 없음. `mode` 변경 (lexical → vector or vice versa) 또는 query 표현 다양화. 그래도 빈 결과면 KB coverage 부족 — 사용자에게 보고.

### tool not found

`tools/list` 에서 본 binary 의 6 tool 확인. 0.3.1 (fb-30) 은 4 tool, 0.3.2 (fb-31) 부터 6. binary version 확인:

```json
{ "name": "schema", "arguments": {} }
```

응답의 `kebab_version` 이 0.3.2+ 인지 확인.

---

## Performance

- **첫 tool call**: cold start ~1-2s (SQLite open + Lance dataset open + fastembed model load).
- **이후 tool call (same session)**: hot — search ~50-200ms, ask ~수 초 (Ollama LLM dominant).
- **session 종료** (host 가 process kill): 모든 cache lost. 다음 session 첫 call 다시 cold.
- **`schema` / `doctor`**: cheap (no LLM / no embedder), 매 call ~ms.
- **`ingest_file` / `ingest_stdin`**: 첫 call 시 fastembed cold start. 이후 file 당 ~수 백 ms (parse + chunk + embed).

cold-start 회피하려면 host 가 long-running session 유지 (Claude Code default).

---

## Security

- stdio MCP — 외부 네트워크 노출 없음. agent host 만 access.
- `kebab mcp` 가 호출하는 facade 는 `--config` 의 권한으로 동작. config 내 secret (Ollama API key 등) 은 process 환경에 한정.
- mutation tool (`ingest_file` / `ingest_stdin`) 는 사용자 명시 의도 없이 자동 호출 금지 — agent 측 가드.

---

## Related

- CLI usage: `kebab --help` + [README.md](../README.md)
- Wire schemas: `docs/wire-schema/v1/*.schema.json`
- design contract: `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` §10.2
- Claude Code 전용 skill: `integrations/claude-code/kebab/SKILL.md`
- HOTFIXES (post-merge deviations): `tasks/HOTFIXES.md`
