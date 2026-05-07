---
title: "p9-fb-31 — Single-file / stdin ingest — agent on-demand 저장"
date: 2026-05-07
status: design (brainstorm 완료, plan 단계 대기)
target_version: 0.3.x
task_spec: ../../../tasks/p9/p9-fb-31-single-file-stdin-ingest.md
contract_source: ../specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§3 ingest, §6 filesystem, §10 UX]
depends_on: []
unblocks: []
---

# Single-file / stdin ingest — 설계

## 동기

agent (Claude Code via MCP, fb-30) 가 web 에서 fetch 한 markdown / pdf 를 KB 에 저장하려면 현재는:

1. agent 가 workspace 디렉토리에 file 쓰기.
2. `kebab ingest` 전체 walk 재실행.

(2) 가 비효율 — 100+ doc workspace 면 모든 doc 의 incremental check 비용. agent 메모리상 string contents 면 임시 file 거치는 우회.

본 task 는 두 신규 명령 도입:

- `kebab ingest-file <path>` — 단일 file (workspace 외부 포함) 만 ingest.
- `kebab ingest-stdin --title <T> [--source-uri <URI>]` — stdin 에서 markdown 본문 read 후 ingest.

MCP tool `ingest_file` + `ingest_stdin` 도 동시 추가 — agent 가 CLI 우회 없이 직접 호출.

## 결정 요약

| 결정 | 선택 |
|------|------|
| 외부 file 저장 정책 | Copy in (`<workspace.root>/_external/<hash12>.<ext>`) |
| CLI surface | 신규 subcommand 2개 (`ingest-file` + `ingest-stdin`) |
| MCP tool | 동시 추가 (4 → 6 tool) — `ingest_file` + `ingest_stdin` |
| .kebabignore | bypass + warn (explicit ingest 가 default bypass intent) |
| stdin v1 scope | markdown 전용 + flag → frontmatter 자동 주입 |

## Surface 1 — `kebab ingest-file`

### CLI

```
kebab ingest-file <path> [--config <path>]
```

- `path`: positional, absolute / relative file path. workspace 외부 가능.
- `--config <path>`: 기존 facade rule 일관 (P3-5 / P4-3 패턴).
- 추가 flag 없음 — 명시 ingest 자체가 .kebabignore bypass intent.

### Behavior

1. file 존재 여부 + 크기 + media type (extension) 검증.
2. workspace.root 의 `.kebabignore` pattern 과 source path 매치 검사 — 매치 시 stderr warn (`warn: <path> matches .kebabignore patterns; proceeding (explicit ingest bypasses ignore)`). 진행은 계속.
3. blake3 content hash 계산 → `_external/<hash12>.<ext>` workspace 상대 경로 derive.
4. `<workspace.root>/_external/` 디렉토리 자동 생성 (없으면). 첫 생성 시 `<workspace.root>/.kebabignore` 에 `_external/` line 자동 append (없으면) — 향후 walk 중복 방지.
5. file content → `<workspace.root>/_external/<hash12>.<ext>` 로 copy. 동일 hash 면 skip (idempotent).
6. 단일 asset 으로 기존 ingest pipeline 재사용 (parse → chunk → embed → vector store + SQLite upsert). incremental ingest (fb-23) 가 동일 hash 면 unchanged 처리.
7. `IngestReport` (`ingest_report.v1`) 반환 — single asset count.

### Output

stdout 은 기존 `kebab ingest` 와 동일 — 사람 모드는 한 줄 summary, `--json` 은 `ingest_report.v1` JSON.

```text
$ kebab ingest-file ~/Downloads/article.md
ingested 1 new (~/Downloads/article.md → _external/a3f7b9e2c1d4.md)
```

`--json`:

```json
{"schema_version":"ingest_report.v1","scope":{"root":".../_external/a3f7b9e2c1d4.md","include":[],"exclude":[]},"scanned":1,"new":1,"updated":0,"skipped":0,"unchanged":0,"errors":0,...}
```

(`scope.root` 표현은 plan 단계 결정 — 단일 file path 또는 fake scope.)

## Surface 2 — `kebab ingest-stdin`

### CLI

```
kebab ingest-stdin --title <T> [--source-uri <URI>] [--config <path>]
```

- `--title <T>`: 필수. frontmatter `title` field 채움.
- `--source-uri <URI>`: 옵션. 제공 시 frontmatter `source_uri` field 채움.
- v1 markdown 전용 — `--media` flag 없음.

### Behavior

1. stdin 전체 read → `String content`.
2. **Frontmatter pre-check**: `content.trim_start().starts_with("---\n")` 면 — `Err`: `"stdin already has frontmatter; use \`kebab ingest-file\` for files with metadata"`. exit 2.
3. 그 외 frontmatter block prepend:

```md
---
title: "<T>"
source_uri: "<URI>"   # only if --source-uri provided
---

<stdin contents>
```

(YAML escaping for title — `serde_yaml::to_string` 또는 inline quote escape. plan 단계 결정.)

4. 합친 markdown 의 blake3 hash → `<workspace.root>/_external/<hash12>.md` 로 write.
5. ingest-file path 의 5-7 단계 재사용.

### Output

```text
$ echo "## Body" | kebab ingest-stdin --title "Article X" --source-uri "https://example.com/x"
ingested 1 new (stdin → _external/7c8e1f3a2b9d.md)
```

`--json` 동일 `ingest_report.v1`.

### source_uri metadata 흐름

- frontmatter `source_uri` 는 markdown parser 가 `Document.metadata` 의 free-form map 에 string field 로 저장 (이미 처리됨 — frontmatter 의 모든 key 가 metadata 로 흘러감).
- `kebab inspect` / `kebab search --json` 의 `doc_meta` 에 자동 포함. agent 가 search 결과의 source_uri 로 원본 web URL 추적 가능.
- v1 wire schema 추가 변경 없음 — `metadata` 가 이미 free-form map.

## Surface 3 — MCP tools `ingest_file` + `ingest_stdin`

### `ingest_file`

```rust
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct IngestFileInput {
    /// Absolute or relative path to the file to ingest.
    pub path: String,
}
```

facade: `kebab_app::ingest_file_with_config(cfg, &Path) -> Result<IngestReport>`.

handle: `spawn_blocking` wrap (touches embedder + SqliteStore). text content = `ingest_report.v1` JSON.

### `ingest_stdin`

```rust
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct IngestStdinInput {
    /// Markdown body content. v1 supports markdown only.
    pub content: String,
    /// Title for frontmatter injection.
    pub title: String,
    /// Optional source URI (e.g. https URL agent fetched from).
    pub source_uri: Option<String>,
}
```

facade: `kebab_app::ingest_stdin_with_config(cfg, content, title, source_uri) -> Result<IngestReport>`.

handle: `spawn_blocking` wrap. text content = `ingest_report.v1` JSON.

### `KebabHandler` 변경

- `build_tools_vec()` 가 4 → 6 entries 반환.
- `call_tool` match 에 `"ingest_file"` + `"ingest_stdin"` arm 추가 (spawn_tool helper 재사용).
- 신규 module `crates/kebab-mcp/src/tools/ingest_file.rs` + `ingest_stdin.rs`.

### Mutation tool 첫 도입

fb-30 v1 은 read-only 4 tool. fb-31 머지로 mutation surface 등장 — agent 가 KB 에 직접 write 가능. 의도된 진화 — agent flow 의 자연스러운 다음 단계. HOTFIXES entry 명시.

## `_external/` 디렉토리 정책

- 위치: `<workspace.root>/_external/`.
- 첫 ingest-file / ingest-stdin 호출 시 자동 생성.
- 생성과 동시에 `<workspace.root>/.kebabignore` 에 `_external/` line append (없으면) — 향후 `kebab ingest` 전체 walk 가 이 디렉토리 재 walk 안 함 (re-ingestion 무한 루프 방지).
- 파일명 = `blake3(content) 12-char prefix + 원래 ext`. deterministic — 동일 content 재 ingest 면 같은 파일명, idempotent (incremental ingest 가 unchanged 처리).
- 사용자가 `_external/` 안 파일 직접 수정해도 OK — explicit `ingest-file` 또는 manual `kebab ingest` (`.kebabignore` 우회 시) 가 incremental 변경 감지.

## 의존 경계 + 신규 facade

- `kebab-app::ingest_file_with_config(cfg, &Path) -> Result<IngestReport>` — 신규 facade fn.
- `kebab-app::ingest_stdin_with_config(cfg, content, title, source_uri) -> Result<IngestReport>` — 신규.
- 둘 모두 내부적으로 기존 `ingest_with_config_opts` 의 single-asset 변종 OR 별 helper. plan 단계 구체화.
- frontmatter injection helper (`kebab-app::frontmatter::inject(content, title, source_uri) -> String`) — kebab-app 안. kebab-mcp + kebab-cli 둘 다 facade 통해 호출.
- `_external/` 디렉토리 + `.kebabignore` 자동 추가도 `kebab-app` 책임 (ingest_file_with_config 안에서).

## Wire schema impact

**없음**. 모두 기존 schema 재사용:

- `ingest_report.v1` — single-asset count. 기존 shape 그대로.
- `error.v1` — file not found / frontmatter precheck 등 facade error 가 기존 code 로 매핑 (`io_error`, `generic`).

`source_uri` 는 `Document.metadata` 의 free-form map 안 — `inspect` / `search_hit.v1` / `answer.v1` 의 `doc_meta` 에 자동 포함.

## Testing 전략

| crate | type | 파일 | 검증 |
|-------|------|------|------|
| `kebab-app` | unit | `tests/ingest_file.rs` | external file → `_external/<hash>.md` copy + IngestReport new=1, 두 번째 호출 unchanged=1, .kebabignore match warn (stderr capture), file-not-found Err |
| `kebab-app` | unit | `tests/ingest_stdin.rs` | content + title → frontmatter prepend + ingest, source_uri 옵션 처리, stdin already-frontmatter Err |
| `kebab-mcp` | integration | `tests/tools_call_ingest_file.rs` | tool call → ingest_report.v1 (isError=false), idempotent 두 번째 호출 unchanged=1 |
| `kebab-mcp` | integration | `tests/tools_call_ingest_stdin.rs` | content + title input → ingest_report.v1, frontmatter precheck error 시 isError=true + error.v1 |
| `kebab-mcp` | integration | `tests/tools_list.rs` (기존) | 4 → 6 tool 검증 (assertion update) |
| `kebab-cli` | integration | `tests/cli_ingest_file.rs` | spawn `kebab ingest-file <tempfile>` → ingest_report.v1 stdout, exit 0 |
| `kebab-cli` | integration | `tests/cli_ingest_stdin.rs` | spawn `kebab ingest-stdin --title X` + stdin pipe → ingest_report.v1, exit 0 |

## Spec / doc sync (PR 같은 commit)

1. **frozen design §3 / §6** — `_external/` 디렉토리 + .kebabignore auto-add 정책 명시.
2. **README** — 명령 표 에 `kebab ingest-file` + `kebab ingest-stdin` 두 row + MCP usage section 의 tool list 4 → 6 update.
3. **HANDOFF** — post-도그푸딩 entry.
4. **CLAUDE.md** — wire schema 목록 변경 없음 (`ingest_report.v1` 재사용). `_external/` 디렉토리 + naming convention 한 줄.
5. **integrations/claude-code/kebab/SKILL.md** — `ingest_file` / `ingest_stdin` MCP tool 사용 안내 + agent fetch flow 예시.
6. **HOTFIXES** — 신규 entry. fb-30 v1 read-only 정책 변경 (mutation tool 도입) 명시.
7. **`tasks/p9/p9-fb-31-single-file-stdin-ingest.md`** — status `open` → `completed`.

## Release trigger

0.3.1 → **0.3.2** patch — additive only (신규 subcommand + 신규 MCP tool, 기존 surface 동작 무영향, wire schema 변경 없음). pre-1.0 patch 정책 일관 (fb-30 도 0.3.1 patch 였음).

## Out of scope (defer)

- **PDF / image stdin** — binary stream + base64 처리 v2.
- **다른 metadata field** (tags, language hint, custom kv) — `--title` + `--source-uri` 외 v2.
- **자동 dedup by source_uri** — content hash 기반 dedup 은 incremental ingest 가 이미 처리. URI 별 lookup 은 별 task.
- **Storage quota / TTL** — agent 무한 ingest 시 KB 비대 우려. monitor + 별 task.
- **Frontmatter merge** (stdin 이 이미 frontmatter 보유 시 머지) — v1 은 error. user 가 경우에 맞게 ingest-file 사용.
- **`--force-ignore` flag** — 명시 ingest 가 default bypass 라 flag 불필요.
- **MCP `ingest_file` 의 multi-file batch** (`paths: Vec<String>`) — v1 single path. 여러 file 호출은 agent 가 N 회.

## Risks / notes

- **`_external/` 디렉토리 명명**: underscore prefix 가 dotfile 만큼 강한 hide 신호 아님. 사용자 workspace listing 시 보임. README 에 명시.
- **.kebabignore auto-append 의 idempotency**: file 이 이미 `_external/` line 보유 시 중복 append 안 함. 정확한 정합 검사 필요.
- **YAML escaping**: title 에 quote / special char 포함 시 frontmatter parse 실패 위험. `serde_yaml` 사용 또는 strict escape.
- **Mutation tool 의 input validation**: `ingest_stdin` 의 `content` 가 매우 클 경우 (수 MB markdown) 메모리 압박. v1 size limit 없음 — agent 책임. monitor + 별 task.
- **agent 의 무한 ingest**: KB 비대 + cost (embedding). 사용자 쪽 monitoring + storage quota 별 task.
- **`_external/` workspace 외부 이동 / 백업 정책**: workspace 안 일반 파일과 동일 — 사용자 백업 정책 일관.
- **hash collision 확률**: blake3 12-char prefix = 48 bit. ~16M files 까지 안전 (birthday bound). single-user KB 에 충분. 충돌 시 first-write-wins (idempotency 와 동일 동작).
