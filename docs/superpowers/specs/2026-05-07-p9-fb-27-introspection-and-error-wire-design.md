---
title: "p9-fb-27 — Introspection (`kebab schema`) + structured error wire"
date: 2026-05-07
status: design (brainstorm 완료, plan 단계 대기)
target_version: 0.3.0
task_spec: ../../../tasks/p9/p9-fb-27-introspection-and-error-wire.md
contract_source: ../specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§10 에러 모델 + exit codes, wire-schema 전반]
unblocks: [p9-fb-30]
---

# Introspection + structured error wire — 설계

## 동기

agent (Claude Code skill, 미래 fb-30 MCP, fb-29 daemon) 가 kebab 인스턴스의 wire 버전 / 기능 / 모델 / 인덱스 통계를 한 번의 호출로 알아내야 통합이 안전하다. 현재는 README / 코드 / `kebab doctor` 출력을 따로 봐야 하고, agent 입장에서 parsable 한 path 가 없다.

또한 error 가 stderr text (`error: <msg>\n  hint: <h>`) — agent 가 substring 으로 분기 (timeout vs config-missing vs not-indexed) 해야 하는데 i18n / 메시지 변경에 깨진다.

본 설계는 다음 두 surface 를 도입한다:

1. `kebab schema [--json]` — 정적 (wire / capabilities / models) + 동적 (stats) introspection 한 명령.
2. `error.v1` wire schema — `--json` 모드에서 fatal error 가 stderr 에 ndjson 으로 emit. 비 `--json` 은 기존 stderr text 그대로.

## Surface 1 — `kebab schema`

### CLI 형태

| flag | 동작 |
|------|------|
| (없음) | 사람 친화 텍스트 (doctor 풍) — stdout |
| `--json` | `schema.v1` JSON object 한 줄 — stdout |

`--config <path>` honor (P3-5 / P4-3 회귀 패턴 회피 — `kebab_app::schema_with_config` 사용).

### Wire schema (`schema.v1`)

```json
{
  "schema_version": "schema.v1",
  "kebab_version": "0.2.1",
  "wire": {
    "schemas": [
      "answer.v1", "search_hit.v1", "doc_summary.v1",
      "chunk_inspection.v1", "doctor.v1",
      "ingest_report.v1", "ingest_progress.v1",
      "reset_report.v1", "citation.v1",
      "schema.v1", "error.v1"
    ]
  },
  "capabilities": {
    "json_mode": true,
    "ingest_progress": true,
    "ingest_cancellation": true,
    "rag_multi_turn": true,
    "search_cache": true,
    "incremental_ingest": true,
    "streaming_ask": false,
    "http_daemon": false,
    "mcp_server": false,
    "single_file_ingest": false
  },
  "models": {
    "parser_version": "md-frontmatter-v2",
    "chunker_version": "md-heading-v1",
    "embedding_version": "fastembed-mle5small-384-v1",
    "prompt_template_version": "rag-v1",
    "index_version": "lance-flat-l2-384-v1",
    "corpus_revision": 42
  },
  "stats": {
    "doc_count": 128,
    "chunk_count": 2147,
    "asset_count": 130,
    "last_ingest_at": "2026-05-07T03:14:00Z"
  }
}
```

**필드 의미**:

- `kebab_version` — `env!("CARGO_PKG_VERSION")` (workspace `Cargo.toml` 의 `version`, kebab-cli 빌드 시 compile-in).
- `wire.schemas` — 본 binary 가 emit 가능한 모든 wire schema 의 fully-qualified id list. parsing 시 v1 / v2 분기 지표.
- `capabilities` — bool 만. 미래 surface (streaming_ask / http_daemon / mcp_server / single_file_ingest) 의 placeholder 도 항상 포함. 해당 fb 머지 시 false → true flip. agent 가 한 호출로 "이 binary 가 streaming 지원하나" 결정.
- `models.parser_version` — `kebab-parse-md` / `kebab-parse-image` / `kebab-parse-pdf` 의 active const (현재 markdown 만 표시 — multi-medium 동시 표시는 plan 단계). 또는 `Config::active_parser_version()` helper.
- `models.chunker_version` — `Config::chunking.chunker_version` (markdown). PDF 는 항상 `pdf-page-v1` hardcode (P7-3 deviation).
- `models.embedding_version` — `Config::models.embedding.id` (config 의 사용자 지정 model id).
- `models.prompt_template_version` — `kebab-rag::PROMPT_TEMPLATE_VERSION` const.
- `models.index_version` — `kebab-store-vector::INDEX_VERSION` const (lance flat L2 384d).
- `models.corpus_revision` — `kv.corpus_revision` (p9-fb-19 V004) 를 u64 로 read.
- `stats.doc_count` / `chunk_count` / `asset_count` — `SELECT COUNT(*) FROM documents | chunks | assets`.
- `stats.last_ingest_at` — `SELECT MAX(updated_at) FROM documents`. RFC3339 string. KB 비어 있으면 `null`.

`stats.last_ingest_at` 은 별도 stamp 안 함 — 기존 `documents.updated_at` 가 idempotent ingest 의 source of truth.

### 사람 친화 출력 (비 `--json`)

```text
$ kebab schema
kebab v0.2.1

wire schemas
  answer.v1, search_hit.v1, doc_summary.v1, ...

capabilities
  ✓ json_mode
  ✓ ingest_progress
  ✓ ingest_cancellation
  ✓ rag_multi_turn
  ✓ search_cache
  ✓ incremental_ingest
  ✗ streaming_ask
  ✗ http_daemon
  ✗ mcp_server
  ✗ single_file_ingest

models
  parser_version          md-frontmatter-v2
  chunker_version         md-heading-v1
  embedding_version       fastembed-mle5small-384-v1
  prompt_template_version rag-v1
  index_version           lance-flat-l2-384-v1
  corpus_revision         42

stats
  doc_count               128
  chunk_count             2147
  asset_count             130
  last_ingest_at          2026-05-07T03:14:00Z
```

doctor 와 시각 일관 — 체크/엑스 마크 + key-value padding.

## Surface 2 — `error.v1` wire

### Shape

```json
{
  "schema_version": "error.v1",
  "code": "model_not_pulled",
  "message": "Ollama model not pulled: gemma4:e4b",
  "details": {
    "model": "gemma4:e4b",
    "endpoint": "http://127.0.0.1:11434",
    "operation": "ask"
  },
  "hint": "ollama pull gemma4:e4b"
}
```

### Field 규약

- `schema_version` — literal `"error.v1"`.
- `code` — machine-readable enum string (catalog 아래).
- `message` — 한 줄 사람 메시지 (anyhow root cause + 짧은 context).
- `details` — code 별 free-form object. 모든 code 가 자체 schema. agent 는 `code` 보고 `details` 의 field 안다.
- `hint` — string. 다음 단계 한 줄. hint 없으면 `null` 또는 omit.

### Emission 정책

- `--json` 일 때 `Cli::run` 의 `Err(e)` 도달 시 `serde_json::to_writer(stderr, &error_v1)?; stderr.write_all(b"\n")?;`. stderr text 는 emit 안 함.
- 비 `--json` 일 때 기존 그대로 (`error: <msg>\n  hint: <h>` + verbose chain).
- **refusal** (`RefusalSignal`) → `answer.v1` 의 `grounded: false`. stdout JSON, exit 1. error.v1 으로 가지 않음.
- **no-hit** (`NoHitSignal`) → `search_hit.v1` 빈 list. stdout JSON, exit 1. error.v1 으로 가지 않음.
- **doctor unhealthy** (`DoctorUnhealthy`) → `doctor.v1` 의 `healthy: false`. stdout JSON, exit 3. error.v1 으로 가지 않음.

### Error code catalog

초기 7개. 각 code 가 typed signal 또는 anyhow chain root 에 매핑.

| code | trigger | details fields | exit | source |
|------|---------|----------------|------|--------|
| `config_invalid` | `Config::load` 실패, `--config` 경로 누락, TOML 파싱 / validation 실패 | `path: String`, `cause: String` | 2 | `ConfigInvalid` 신규 signal |
| `not_indexed` | `kebab.sqlite` 미존재 / migration 미실행 / V00X mismatch | `data_dir: String`, `expected: String`, `found: Option<String>` | 3 | `DoctorUnhealthy` extension |
| `model_unreachable` | Ollama endpoint 연결 실패 (TCP refused / DNS / connect timeout) | `endpoint: String`, `operation: "ask"\|"caption"\|"ocr"` | 2 | `ModelUnreachable` 신규 signal |
| `model_not_pulled` | Ollama 200 응답이 "model not found" body | `model: String`, `endpoint: String`, `operation: ...` | 2 | `ModelNotPulled` 신규 signal |
| `timeout` | LLM stream / embed batch deadline 초과 | `operation: String`, `elapsed_ms: u64`, `deadline_ms: u64` | 2 | `OpTimeout` 신규 signal |
| `io_error` | filesystem / 권한 / disk full | `path: String`, `op: "read"\|"write"\|"create"` | 2 | `IoFailure` 신규 signal |
| `generic` | 위 catalog 외 모든 anyhow | `chain: Vec<String>` (verbose 시) | 2 | catch-all |

**확장 정책**:

- 새 code 추가 = additive — `error.v1` major bump 불필요.
- code 제거 / 의미 변경 = `error.v2` breaking.
- fb-29/30/33 머지 시 자체 code 추가 가능 (예 `daemon_locked`, `mcp_protocol_error`, `stream_aborted`).

## Internal architecture

### 새 typed signal 모듈

```rust
// crates/kebab-app/src/error_signal.rs (신규)
use std::path::PathBuf;

#[derive(Debug)]
pub struct ConfigInvalid {
    pub path: PathBuf,
    pub cause: String,
}

#[derive(Debug)]
pub struct ModelUnreachable {
    pub endpoint: String,
    pub operation: &'static str, // "ask" | "caption" | "ocr"
}

#[derive(Debug)]
pub struct ModelNotPulled {
    pub model: String,
    pub endpoint: String,
    pub operation: &'static str,
}

#[derive(Debug)]
pub struct OpTimeout {
    pub operation: &'static str,
    pub elapsed_ms: u64,
    pub deadline_ms: u64,
}

#[derive(Debug)]
pub struct IoFailure {
    pub path: PathBuf,
    pub op: &'static str, // "read" | "write" | "create"
}
```

각 signal 은 `std::error::Error + Send + Sync` 자동 derive 또는 thiserror impl. 발생지 (`kebab-config`, `kebab-llm-local`, `kebab-store-sqlite`) 가 `anyhow::Error::new(signal).context(...)` 로 wrap. `classify` 가 downcast 로 분기.

기존 signal — `RefusalSignal` (kebab-rag), `NoHitSignal` (kebab-app), `DoctorUnhealthy` (kebab-app) — 변경 없음.

### `classify` 함수

```rust
// crates/kebab-cli/src/error_classify.rs (신규)
use kebab_app::error_signal::*;
use crate::wire::ErrorV1;

pub fn classify(err: &anyhow::Error, verbose: bool) -> ErrorV1 {
    if let Some(s) = err.downcast_ref::<ConfigInvalid>() {
        return ErrorV1::config_invalid(&s.path, &s.cause);
    }
    if let Some(s) = err.downcast_ref::<ModelUnreachable>() {
        return ErrorV1::model_unreachable(&s.endpoint, s.operation);
    }
    if let Some(s) = err.downcast_ref::<ModelNotPulled>() {
        return ErrorV1::model_not_pulled(&s.model, &s.endpoint, s.operation);
    }
    if let Some(s) = err.downcast_ref::<OpTimeout>() {
        return ErrorV1::timeout(s.operation, s.elapsed_ms, s.deadline_ms);
    }
    if let Some(s) = err.downcast_ref::<IoFailure>() {
        return ErrorV1::io_error(&s.path, s.op);
    }
    // not_indexed 는 DoctorUnhealthy 가 아닌 별 signal? — skeleton 단계
    // store-sqlite 의 schema mismatch 는 별 signal type 정의하거나 anyhow context 매칭
    ErrorV1::generic(err, verbose)
}
```

`not_indexed` 의 매핑은 plan 단계 결정 — `DoctorUnhealthy` 의 reason 분류 또는 `kebab-store-sqlite` 의 schema-mismatch 별 signal.

### CLI main.rs 변경

`Cmd::Schema` arm 신규:

```rust
Cmd::Schema => {
    let cfg = kebab_config::Config::load(cli.config.as_deref())?;
    let report = kebab_app::schema_with_config(&cfg)?;
    if cli.json {
        let v = serde_json::to_value(&report)?;
        let v = wire::tag_object(v, "schema.v1");
        println!("{}", serde_json::to_string(&v)?);
    } else {
        wire::print_schema_text(&report);
    }
    Ok(())
}
```

`main()` 의 `Err(e)` arm 분기:

```rust
match run(&cli) {
    Ok(()) => ExitCode::from(0),
    Err(e) => {
        let code = exit_code(&e);
        if code != 1 {
            if cli.json {
                let err_v1 = error_classify::classify(&e, cli.verbose);
                let v = serde_json::to_value(&err_v1).unwrap();
                let v = wire::tag_object(v, "error.v1");
                eprintln!("{}", serde_json::to_string(&v).unwrap());
            } else {
                eprintln!("error: {e}");
                if cli.verbose {
                    for cause in e.chain().skip(1) {
                        eprintln!("  caused by: {cause}");
                    }
                }
            }
        }
        ExitCode::from(code)
    }
}
```

`exit_code()` 함수 unchanged — typed signal 3개 (`RefusalSignal`, `NoHitSignal`, `DoctorUnhealthy`) 만 보고 1/3 결정. 신규 5 signal 모두 fall-through → 2.

### Facade (kebab-app) 변경

- `pub fn schema_with_config(cfg: &Config) -> Result<SchemaV1>` 신규 — wire / capabilities / models / stats 빌드.
- `pub mod error_signal` — public, kebab-cli 가 import.
- 기존 facade 시그니처 무영향.

### 의존 경계

- `error_signal` 모듈 = `kebab-app` 내. UI crate (`kebab-cli`) 만 import.
- `kebab-core` 침범 없음.
- `kebab-store-sqlite` / `kebab-llm-local` / `kebab-config` 가 발생지에서 signal 받아 anyhow wrap — 각자 `kebab-app` 의존 없이 `kebab-core` extension trait 또는 별 sub-crate 로 import. plan 단계 결정.

  **대안 1**: `error_signal` 을 `kebab-core` 에 두고 모든 발생지가 kebab-core 만 의존 (이미 의존 중). 단순. 하지만 §8 의존 경계 룰: `kebab-core` 는 도메인 타입만. signal 이 도메인 타입인가? 모호. plan 단계 brainstorm.
  **대안 2**: 신규 crate `kebab-error` — signal type 만 보유. 모든 crate 가 의존. 새 crate 도입 비용.
  **대안 3 (recommended)**: signal type 을 발생지 crate (kebab-config / kebab-llm / kebab-llm-local / kebab-store-sqlite) 자체에 정의. kebab-cli 의 `classify` 가 모두 import. kebab-app 은 re-export 만.

## Testing 전략

| crate | test type | 파일 | 검증 |
|-------|-----------|------|------|
| `kebab-app` | unit | `tests/schema_report.rs` | TempDir KB ingest 후 `schema_with_config` — `models.parser_version == "md-frontmatter-v2"`, `stats.doc_count == 3`, `stats.last_ingest_at == max(documents.updated_at)`, 빈 KB → `last_ingest_at: None` |
| `kebab-app::error_signal` | unit | `src/error_signal.rs::tests` | 5 신규 signal 의 `Display` + `std::error::Error::source` chain 안정 |
| `kebab-cli::wire` | unit | `src/wire.rs::tests` | `SchemaV1` / `ErrorV1` round-trip — `tag_object` 가 `schema_version` 정확 wrap, `serde_json::from_str` 으로 다시 파싱 |
| `kebab-cli::error_classify` | unit | `src/error_classify.rs::tests` | 7 mock anyhow chain → 7 code 일대일 매핑, 8th anyhow → `code == "generic"`, verbose=true 시 `details.chain` 채움 |
| 통합 | binary | `tests/cli_schema.rs` | `kebab schema --json` exit 0 + stdout parse 가능 + `schema_version == "schema.v1"` |
| 통합 | binary | `tests/cli_error_wire.rs` | `kebab --json --config /nonexistent ingest` → exit 2 + stderr ndjson `code == "config_invalid"` |
| 회귀 | binary | 기존 smoke 6+ | 비 `--json` 모드 stderr text 포맷 unchanged — snapshot |

## Migration / 호환성

- 모든 변경 additive. wire schema v1 major bump 없음.
- 기존 9 wire schema literal 동일.
- `--json` 모드 에러 emit 은 신규 surface — 이전 binary 의 `--json` 사용자 (claude-code skill) 가 stderr 무시했던 패턴 그대로 동작. 추가 정보만 늘어남.
- exit code 매핑 동일 — 0/1/2/3.

## Spec / doc sync (PR 같은 commit)

1. **frozen design §10** — wire schema list 에 `schema.v1` / `error.v1` 추가, capability matrix 절 신설.
2. **`docs/wire-schema/v1/schema.schema.json`** + **`error.schema.json`** 신규.
3. **README.md** — 명령 표 에 `kebab schema` row, 짧은 capability flag 안내.
4. **HANDOFF.md** — "머지 후 발견된 결정" 한 줄.
5. **HOTFIXES.md** — 의도적 deviation 없으면 짧은 entry.
6. **CLAUDE.md** — wire schema 절에 두 신규 추가.
7. **integrations/claude-code/kebab/SKILL.md** — `kebab schema` 활용 안내 (additive).
8. **`tasks/p9/p9-fb-27-introspection-and-error-wire.md`** — frontmatter `status: open` → `in_progress` 또는 `completed`.

## Release trigger

0.3.0 minor bump — fb-27 머지 = "agent foundation" 첫 component. wire 추가 additive 라 release 의무 아님이지만 fb-26~31 묶어 0.3.0 한 번에 cut.

## Out of scope (deferred)

- fb-30 MCP 의 `initialize` response 가 `capabilities` 재사용 — fb-30 spec 에서 import.
- fb-37 trace + stats 가 `error.v1.details.trace_id` 추가 — additive.
- error code 확장 (예 `embedding_dim_mismatch`) — 발생지 추가 시점 case-by-case.
- `not_indexed` 의 정확한 source signal 결정 (`DoctorUnhealthy` extension vs 별 signal) — plan 단계.

## Risks / notes

- Cascade: capability flag 추가 / 제거 = wire schema additive — 기존 agent 가 새 flag 무시하면 OK, false 인 flag 의존 코드는 반드시 default 처리 필요.
- error code enumeration 의 i18n: `message` 필드는 영어 또는 한국어? — plan 단계 결정. agent 는 `code` 로만 분기, `message` 는 사람용. 현 stderr text 는 한국어 우세 → 동일.
- `not_indexed` 의 매핑이 `DoctorUnhealthy` 와 겹침. `DoctorUnhealthy` 가 wider scope (multiple subsystem) — `not_indexed` 만 별 signal 로 분리 vs reason field 로 구분.
- `last_ingest_at` 이 incremental ingest (fb-23) 의 `Unchanged` 도 `updated_at` bump 시키면 의미 모호 — code 확인 후 plan 단계 명시 (현재 idempotent UPSERT 가 항상 bump 라면 `last_change_at` 이 더 정확).
