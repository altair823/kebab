---
title: "p9-fb-30 — MCP server (stdio) — agent host 무관 protocol surface"
date: 2026-05-07
status: design (brainstorm 완료, plan 단계 대기)
target_version: 0.4.0
task_spec: ../../../tasks/p9/p9-fb-30-mcp-server.md
contract_source: ../specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§7 RAG, §10 UX]
depends_on: [p9-fb-27]
unblocks: []
---

# MCP server (stdio) — 설계

## 동기

현재 외부 AI 통합은 `integrations/claude-code/kebab/` skill 한 종류 — Claude Code subprocess wrapper. Cursor / OpenAI Agents / Copilot CLI 등 다른 host 는 별도 wrapper 작성 필요.

MCP (Model Context Protocol) 가 표준 — 한 번 server 구현하면 MCP-aware host 모두 지원. 본 task 는 stdio MCP server 도입. fb-29 HTTP daemon 은 deferred (single-user local-first 환경에서 daemon 복잡도 비대 — fb-30 stdio 가 동일 사용자 가치 제공).

fb-27 (introspection + error wire) 의 capability matrix + error.v1 wire 가 본 task 의 prerequisite ✅.

## 결정 요약

| 결정 | 선택 |
|------|------|
| Dispatch | `kebab mcp` subcommand (kebab-cli 내) |
| Tool surface (v1) | `search` / `ask` / `schema` / `doctor` (read-only, 4 개) |
| Resources / Prompts | 모두 skip (tools only) |
| 구현 | Rust MCP SDK (`rmcp` 또는 plan 단계 채택) |
| Transport | stdio 단일 (HTTP-SSE 는 fb-29 deferral 따라 P+) |
| Output | 모든 tool 이 wire schema v1 JSON 을 text content 로 반환 |
| Multi-turn `ask` | optional `session_id` (kebab-app 의 `ask_with_session_with_config` 활용) |

## Surface 1 — `kebab mcp` 신규 subcommand

### CLI

```
kebab mcp                       # stdio JSON-RPC server 시작
kebab mcp --config <path>       # config 명시 (P3-5 / P4-3 패턴)
```

`--config` 외 추가 flag 없음. agent host 가 spawn 명령에서 환경 변수로 추가 설정 주입.

### Crate boundary

새 crate `crates/kebab-mcp/` (lib only). `kebab-cli` 의 `Cmd::Mcp` arm 이 한 줄 entry — `kebab_mcp::serve_stdio(cfg)?`.

```
kebab-cli ──► kebab-mcp ──► kebab-app ──► kebab-store-* / kebab-llm-* / kebab-parse-*
                  │            │
                  └─ rmcp ─────┴─ kebab-config / kebab-core
```

CLAUDE.md facade 룰 준수:
- `kebab-mcp` 는 `kebab-app` facade + `kebab-config` + `kebab-core` 만 import. 구현 crate 직접 금지.
- `kebab-cli` 는 `kebab-mcp` 만 알고 MCP 내부 미인지 — 다른 UI crate (TUI / desktop) 가 mcp surface 필요해지면 동일하게 import.
- `rmcp` (또는 채택 SDK) 는 `kebab-mcp` 의 `[dependencies]` 만 — kebab-cli 는 transitive.

CLAUDE.md 의 "UI crates" 카테고리에 `kebab-mcp` 추가 (의존 경계 절).

## Surface 2 — Tool catalog (4 tools)

`tools/list` response 가 4 tool 을 노출. 각 tool 은 inputSchema (JSON Schema) 가 inline.

### `search`

| 항목 | 값 |
|------|-----|
| description | "Lexical / vector / hybrid retrieval over indexed corpus." |
| input | `{ query: string (required), mode?: "lexical" \| "vector" \| "hybrid" (default "hybrid"), k?: integer (default 10, range 1-100) }` |
| facade | `kebab_app::search_with_config(&cfg, query, mode, k)` |
| output | text content = `serde_json::to_string(wire::wire_search_hits(&hits))` |

빈 결과 = 정상 응답 (empty `search_hit.v1` array). NoHitSignal 의 exit-code 분기는 stdio 무관.

### `ask`

| 항목 | 값 |
|------|-----|
| description | "Grounded RAG answer with citations. Returns answer.v1 with grounded=false when KB lacks context." |
| input | `{ query: string (required), session_id?: string }` |
| facade | `session_id` 있으면 `ask_with_session_with_config`, 없으면 `ask_with_config` |
| output | text content = `serde_json::to_string(wire::wire_answer(&answer))` |

Refusal (`grounded: false`) = 정상 응답. agent 가 wire payload 의 `grounded` flag 로 분기. `refusal_reason` 도 답변에 포함.

### `schema`

| 항목 | 값 |
|------|-----|
| description | "Introspection — wire schemas, capabilities, model versions, index stats." |
| input | `{}` (no args) |
| facade | `schema_with_config(&cfg)` |
| output | text content = `serde_json::to_string(wire::wire_schema(&schema))` |

`capabilities.mcp_server` 가 `true` (본 PR 에서 `capabilities_snapshot()` 갱신).

### `doctor`

| 항목 | 값 |
|------|-----|
| description | "Health check — config / data dir / Ollama reachability." |
| input | `{}` (no args) |
| facade | `doctor_with_config_path(cli.config.as_deref())` (or equivalent) |
| output | text content = `serde_json::to_string(wire::wire_doctor(&report))` |

DoctorUnhealthy = 정상 응답 (doctor.v1 with `ok: false`). agent 가 검사.

## Surface 3 — Lifecycle / capabilities / error mapping

### Initialize handshake

server `initialize` 응답:

```jsonc
{
  "protocolVersion": "<rmcp 가 pin 하는 stable version, 예: 2025-03-26>",
  "capabilities": {
    "tools": { "listChanged": false }
  },
  "serverInfo": {
    "name": "kebab",
    "version": "<env!(\"CARGO_PKG_VERSION\")>"
  }
}
```

resources / prompts / sampling / notifications — 모두 미선언.

### Tool error envelope

| 시나리오 | MCP 응답 | content |
|----------|----------|---------|
| facade `Err(e)` | `{ isError: true, content: [{ type: "text", text: <error.v1 JSON> }] }` | `error_wire::classify(&e, false)` 결과 |
| facade `Ok(...)` | `{ isError: false, content: [{ type: "text", text: <wire JSON> }] }` | search_hit.v1 / answer.v1 / schema.v1 / doctor.v1 |

protocol-level error (invalid method / malformed params / panic) 는 SDK 가 JSON-RPC error envelope 으로 자동 처리.

### Refusal / no-hit / unhealthy 가 isError 아님

CLI 의 exit code 1 (refusal/no-hit) / 3 (doctor unhealthy) 는 stdio 환경에 의미 없음. 모두 `isError: false` 정상 응답으로 반환 — agent 가 wire payload 의 semantic flag (`grounded` / 빈 array / `ok: false`) 로 분기. 이게 MCP 표준 패턴 — error envelope 은 protocol 실패 전용.

### classify 모듈 이전 (load-bearing 구조 변경)

`kebab-cli::error_classify` (fb-27 도입) 를 `kebab-app::error_wire` 로 promotion. 본 PR 에 포함:

- `crates/kebab-app/src/error_wire.rs` 신규 — `ErrorV1` struct + `classify(&anyhow::Error, verbose: bool) -> ErrorV1` 함수 + `classify_llm` helper. fb-27 commit `c91228e` 의 `error_classify.rs` 코드 그대로 이전.
- `crates/kebab-app/src/lib.rs` `pub mod error_wire;` + re-export.
- `crates/kebab-cli/src/error_classify.rs` 삭제 — `kebab-cli::main` 의 import 가 `kebab_app::error_wire::*` 로 변경.
- 기존 7 unit test 도 함께 이전 (`kebab-app/src/error_wire.rs::tests`).
- `kebab-cli::wire::wire_error_v1` 의 `&crate::error_classify::ErrorV1` → `&kebab_app::ErrorV1` 1 줄 변경.
- kebab-cli 의 reqwest dev-dep 는 유지 (`llm_unreachable_classifies` 가 함께 이동) — 또는 reqwest dev-dep 도 kebab-app 으로 이전.

근거: kebab-cli + kebab-mcp 둘 다 동일 classify 사용. UI crate (kebab-cli) 가 다른 UI crate import 는 facade 룰 위반. kebab-app 으로 promotion 이 정공법.

### Concurrency

stdio JSON-RPC 는 클라이언트 측 순차 호출 default 지만 MCP spec 은 동시 호출 허용. tokio runtime (kebab-app 의 `rt-multi-thread` feature 활용):

- 각 `tools/call` request 가 독립 task — `tokio::task::spawn`.
- facade 가 sync API (현재 대부분) → `tokio::task::spawn_blocking` wrap.
- 한 process 안에 SQLite / Lance / fastembed connection 공유 — 한 번 init 후 모든 tool call 이 hot. `kebab-app` 의 facade 가 매 호출 마다 `Config` load + store open 시 cold-start 절감 효과 약화 — plan 단계에서 server-scope `App` 인스턴스 (혹은 connection pool) 도입 검토.

세부 async pattern + connection lifetime 은 plan 단계 결정 (rmcp SDK 의 dispatch 모델에 의존).

## Out of scope (defer)

- **HTTP-SSE transport** — fb-29 P+ 와 묶어 진행. 본 task 는 stdio 단일.
- **Resources** — `kebab://chunk/<id>` / `kebab://doc/<id>` URI scheme. fb-35 verbatim fetch 와 함께 v2.
- **Prompts** — reusable prompt template. RAG 자체가 prompt template 내장 — 사용자 가치 약함, defer.
- **Streaming `ask`** — fb-33 streaming ask 와 함께 ndjson delta tool 결과.
- **`ingest_file` / `ingest_stdin` tools** — fb-31 single-file ingest 머지 시 추가.
- **`fetch` (verbatim doc/chunk)** — fb-35 verbatim fetch 머지 시 추가.
- **`list_docs` / `inspect_chunk` tools** — demand 발생 시.
- **Server logging notifications** (`notifications/message`) — SDK 자동 처리만.
- **Sampling capability** — 본 server 는 sampling 미수행.

## Testing 전략

| crate | type | 파일 | 검증 |
|-------|------|------|------|
| `kebab-app` | unit | `src/error_wire.rs::tests` | 기존 7 classify test (fb-27 에서 promotion) |
| `kebab-mcp` | unit | `src/lib.rs::tests` | tool input schema parse + dispatch (mock or TempDir) |
| `kebab-mcp` | integration | `tests/initialize.rs` | initialize handshake — protocolVersion / serverInfo / capabilities.tools 정확 |
| `kebab-mcp` | integration | `tests/tools_list.rs` | `tools/list` 가 4 tool name + inputSchema 정확 반환 |
| `kebab-mcp` | integration | `tests/tools_call_search.rs` | search tool call → text content = search_hit.v1 array, isError=false |
| `kebab-mcp` | integration | `tests/tools_call_ask.rs` | ask tool call → answer.v1 (refusal 시 grounded=false 정상) |
| `kebab-mcp` | integration | `tests/tools_call_schema.rs` | schema.v1 정확 + capabilities.mcp_server=true 검증 |
| `kebab-mcp` | integration | `tests/tools_call_doctor.rs` | doctor.v1 정확 |
| `kebab-mcp` | integration | `tests/error_mapping.rs` | bad config 로 호출 → tool error with error.v1 + isError=true |
| `kebab-cli` | integration | `tests/cli_mcp_smoke.rs` | `target/debug/kebab mcp` spawn + 1 round-trip JSON-RPC |
| `kebab-app` | unit | `tests/schema_report.rs` (기존) | `capabilities.mcp_server == true` assertion 1 줄 추가 |

JSON-RPC client = rmcp 의 in-process test harness (지원 시) 또는 hand-roll line write/read 헬퍼.

## Spec / doc sync (PR 같은 commit)

1. **frozen design §10.1** — MCP transport 절 추가 (또는 §10.2 신설). stdio-only / 4 tool / capability flag flip 명시.
2. **README.md** — 명령 표 에 `kebab mcp` row + MCP usage section (Claude Code `~/.claude/mcp.json` config 예시).
3. **HANDOFF.md** — `2026-05-?? P9 post-도그푸딩 (p9-fb-30)` 한 줄.
4. **CLAUDE.md** — facade 룰 절 에 `kebab-mcp` UI crate 카테고리 추가. 새 crate 카운트 갱신 (~20 → ~21).
5. **integrations/claude-code/kebab/SKILL.md** — MCP 사용 권장 + Claude Code `~/.claude/mcp.json` 예시 한 블록 추가. 기존 subprocess wrapper 형태도 backwards-compat 유지 (일부 사용자가 MCP 미지원 host 에서 호출).
6. **HOTFIXES.md** — `2026-05-?? — fb-30` entry. classify 모듈 이전 + capability flag flip + 기타 deviation 명시.
7. **`tasks/p9/p9-fb-30-mcp-server.md`** — status `open` → `completed`, banner 갱신, depends_on 갱신 (이미 fb-29 제거됨 from 2026-05-07 commit).

## Release trigger

0.3.0 → **0.4.0** minor bump — fb-30 머지 = 신규 CLI surface (`kebab mcp`) + new crate (`kebab-mcp`) + capability flag flip (`mcp_server: true`) + design §10 변경 (3 trigger 모두 발동).

agent integration "MVP" 완성 신호. release notes 에 강조: "MCP 표준 protocol 으로 Claude Code / Cursor / OpenAI Agents 등 host-agnostic 사용 가능."

## Risks / notes

- **rmcp version maturity**: plan 단계 verify 필요. 미존재 / 미성숙 시 hand-roll JSON-RPC 또는 hybrid (transport hand-roll + spec literal struct serde) fallback. rmcp 채택 가정 하 spec 작성됨 — 심각한 호환성 문제 발생 시 spec 갱신 + HOTFIXES.
- **classify 이전의 회귀 위험**: kebab-cli 의 7 test + 1 wire test 가 import path 변경. mechanical 이지만 누락 시 컴파일 실패로 catch.
- **`Config` resolution per call**: 매 tool call 마다 `Config::load(...)` + `App` open 하면 daemon 의 hot-cache 효과 미미. 첫 call 시 server-scope `App` 인스턴스 만들고 이후 재사용 — plan 단계 concrete 설계.
- **MCP version evolution**: spec 가 진화 중. SDK pin 따르고 README 에 명시. major change 발생 시 별 task.
- **ask 의 multi-turn session 의 정합**: kebab session 은 kebab 의 RAG history. agent host 도 자체 conversation 추적. 둘이 다른 식별자 — sync 필요 시 사용자가 명시적으로 `session_id` 매핑. 본 PR scope 밖 — agent 사용 가이드에 명시.
- **Tool error 에 hint 손실 위험**: `error_wire::classify` 가 `hint: Option<String>` 채움. MCP 응답에서 `hint` 가 보존되는지 — text content 의 JSON 이 `hint` field 그대로 가짐. agent 가 parse 하면 readable. OK.
- **stdin/stdout 충돌**: kebab-mcp 가 stdin/stdout 으로 JSON-RPC 통신. `tracing` log 가 stdout 으로 쓰면 protocol 깨짐. 모든 log 는 stderr 또는 file (`~/.local/state/kebab/logs/`) — kebab-app 의 logging init 가 이미 stderr 기본. 명시 verify.
