# p10-2 — Tier 2 resource-aware chunkers (k8s + Dockerfile + manifest)

**Status:** 🟡 진행 중
**Contract sections:** §3.3 (chunker_version `k8s-manifest-resource-v1` + `dockerfile-file-v1` + `manifest-file-v1`), §3.4 (citation symbol — `<kind>/<namespace>/<name>` / `<dockerfile>` / `<manifest>`), §3.5 (code_lang 추가 매핑 `xml` / `groovy` / `go-mod`), §6.1 (`kebab-parse-code/src/lang.rs` 갱신 + `kebab-source-fs/src/media.rs` 의 inline duplication 정리), §6.2 (`kebab-chunk/src/{k8s_manifest_resource_v1,dockerfile_file_v1,manifest_file_v1}.rs`), §9.2 (Tier 2 정의), §10.1 (deactivation log 한 줄).
**Design:** [2026-05-15-kebab-code-ingest-design.md](../../docs/superpowers/specs/2026-05-15-kebab-code-ingest-design.md) §1.2 (Phase 2) + §9.2.
**Plan:** [2026-05-20-p10-2-tier2-resource-aware.md](../../docs/superpowers/plans/2026-05-20-p10-2-tier2-resource-aware.md).

## Goal

p10-1A-2 / 1B / 1C 인프라 위에 Tier 2 resource-aware chunker 3종을 단일 PR 로 활성화. AST 가 아닌 file/document-level chunking — 1B (Python+TS+JS) 의 묶음 패턴 따름. 머지 시점부터 `.yaml` / `.yml` / `Dockerfile` / 매니페스트 7종 dogfooding 가능.

비-k8s YAML (Helm values, CI yml, docker-compose 등) 및 invalid YAML 은 본 phase 에선 skip — p10-3 의 paragraph fallback 이 머지되면 자동으로 wire 됨.

## 동결된 설계 결정 (이 task 로 확정)

### 공통

- **3 chunker = self-contained**. `kebab-parse-code` 에 Tier 2 용 extractor 모듈 추가 없음. lang.rs 의 `code_lang_for_path` 갱신만. AST 가 아니라 추상화 비용이 코드 보상보다 큼.
- **`code_lang_for_path` = single source of truth** (design §3.5). `kebab-source-fs/src/media.rs` 의 inline 확장자 match 는 이 함수 호출로 통일 (1A-1 부터 누적된 duplication 정리, 작은 리팩토링).
- **parser_version** = `"none-v1"` 통일. Tier 2 는 parse 단계가 없음을 명시하는 sentinel. chunker_version cascade 만 의미 있음.
- **oversize fallback** = AST chunker 와 동일 정책 (`AST_CHUNK_MAX_LINES = 200` 초과 시 line-window split). 거대 ConfigMap / multi-stage Dockerfile / aggregate POM 대비. split chunk 는 같은 symbol 공유 (line range 만 다름).
- **frozen design 갱신** (본 PR 안에서):
  - §3.5 `code_lang` 매핑 표에 3 줄 추가:
    - XML (`.xml`, `pom.xml`) → `xml`
    - Groovy (`build.gradle`, `.gradle`) → `groovy`
    - Go module (`go.mod`) → `go-mod`
  - §10.1 deactivation log 한 줄 추가: "p10-2 활성화 — Tier 2 chunker 3종 active."

### k8s-manifest-resource-v1

- **Trigger**: `MediaType::Code("yaml")` (= `.yaml` / `.yml`).
- **k8s 식별**: YAML document 의 top-level mapping 에 `apiVersion: <string>` + `kind: <string>` 둘 다 있어야 인정. 하나라도 없거나 string 타입이 아니면 그 document skip (전체 파일 skip 아님 — 다른 document 는 정상 처리).
- **Multi-document split 구현**: `serde_yaml::Deserializer::from_str` 의 multi-document iterator 가 line offset 을 안 줘서, 원본 텍스트의 `^---\s*$` 줄 정규식 기준으로 pre-split 후 각 슬라이스를 deserialize. line_start/line_end 는 pre-split 단계에서 추적. trailing `---` 의 빈 슬라이스는 skip.
- **Symbol**: `<kind>/<metadata.namespace>/<metadata.name>` (namespace 있으면) 또는 `<kind>/<metadata.name>` (cluster-scoped) 또는 `<kind>/<unnamed>` (name 누락). 예: `Deployment/prod/api-server`, `ClusterRole/cluster-admin`, `ConfigMap/<unnamed>`.
- **Chunk text**: pre-split 슬라이스의 원본 텍스트 그대로 (deserialized form 아님 — 원본 보존).
- **Citation**: `Citation::Code { path, line_start, line_end, symbol: Some(<위>), lang: Some("yaml") }`.
- **Failure modes**:
  - Invalid YAML (어떤 document 라도 deserialize 실패) → 파일 전체 emit 0 chunk + warning log `invalid yaml: {path}`. p10-3 의 paragraph fallback 이 picked up.
  - 인정된 document 0개 (모두 비-k8s) → 파일 전체 emit 0 chunk. 동일 fallback.

### dockerfile-file-v1

- **Trigger**: `MediaType::Code("dockerfile")` — 파일명이 정확히 `Dockerfile`, 또는 prefix `Dockerfile.` (e.g. `Dockerfile.dev`), 또는 확장자 `.dockerfile` (e.g. `myapp.dockerfile`).
- **Algorithm**: 파일 전체 텍스트 → 1 chunk emit.
- **Symbol**: 통일 `<dockerfile>`.
- **Citation**: `Citation::Code { path, line_start: 1, line_end: <EOF>, symbol: Some("<dockerfile>"), lang: Some("dockerfile") }`.

### manifest-file-v1

- **Trigger**: 파일명이 design §9.2 의 7종 중 하나:
  | basename       | code_lang |
  |----------------|-----------|
  | `Cargo.toml`   | `toml`    |
  | `pyproject.toml` | `toml`  |
  | `package.json` | `json`    |
  | `tsconfig.json`| `json`    |
  | `go.mod`       | `go-mod`  |
  | `pom.xml`      | `xml`     |
  | `build.gradle` | `groovy`  |
- **제외**: `build.gradle.kts` 는 1C-JK 의 Kotlin AST chunker (code-kotlin-ast-v1) 가 잡으므로 본 chunker 의 대상 아님.
- **Algorithm**: 파일 전체 텍스트 → 1 chunk emit.
- **Symbol**: 통일 `<manifest>` (7종 모두). manifest 종류 구분은 `code_lang` 으로 — 예: `--code-lang go-mod` 는 go.mod 만, `--code-lang toml` 은 Cargo.toml + pyproject.toml.
- **Citation**: `Citation::Code { path, line_start: 1, line_end: <EOF>, symbol: Some("<manifest>"), lang: Some(<위 매핑>) }`.

### Routing (kebab-app::ingest_one_code_asset)

기존 7-arm AST match 옆에 Tier 2 분기 추가:

```text
"rust" | "python" | "typescript" | "javascript"
  | "go" | "java" | "kotlin"      → 기존 AST chunker (1A-2 / 1B / 1C)
"yaml"                            → k8s_manifest_resource_v1
"dockerfile"                      → dockerfile_file_v1
"toml" | "json" | "xml"
  | "groovy" | "go-mod"           → manifest_file_v1
_                                 → skip (p10-3 fallback 의 자리)
```

`code_lang_for_path` 의 lookup 순서: basename 우선 매칭 (`Cargo.toml` / `Dockerfile.*` / etc.) → 확장자 fallback (`.yaml` / `.toml` / etc.).

## Acceptance criteria

- `cargo test --workspace --no-fail-fast -j 1` passes (memory-conscious: per-crate 위주, full-suite gate 는 docs task 직전 1회).
- `cargo clippy --workspace --all-targets -- -D warnings` passes.
- 각 chunker 의 snapshot test 안정:
  - `crates/kebab-chunk/tests/fixtures/sample.yaml` — 2 k8s doc (Deployment + Service) + 1 비-k8s doc (apiVersion 빠짐) → 2 chunk emit, 비-k8s doc skip.
  - `crates/kebab-chunk/tests/fixtures/sample.dockerfile` → 1 chunk, symbol `<dockerfile>`.
  - `crates/kebab-chunk/tests/fixtures/sample.Cargo.toml` + `sample.package.json` + `sample.pom.xml` + `sample.go.mod` (4종) → 각 1 chunk, symbol `<manifest>`, 매핑된 code_lang.
- `code_lang_for_path` 의 basename 우선 매칭 + 확장자 fallback unit test.
- 격리 TempDir KB 에 yaml + Dockerfile + Cargo.toml 두고 `kebab search --code-lang yaml --json` / `--code-lang dockerfile --json` / `--code-lang toml --json` 각각 `Citation::Code` 반환 (기존 `code_ingest_smoke.rs` 에 3 테스트 추가, 총 12 테스트).
- `kebab schema --json | jq .stats.code_lang_breakdown` 에 `yaml` / `dockerfile` / `toml` / `json` / `xml` / `groovy` / `go-mod` 카운트 (사용된 것만 등장).
- README + HANDOFF + docs/ARCHITECTURE + docs/SMOKE + tasks/INDEX + tasks/p10/INDEX 갱신.
- frozen design §3.5 매핑 3 줄 + §10.1 활성화 한 줄.
- workspace `Cargo.toml` minor bump (0.13.0 → 0.14.0), gitea-release v0.14.0.

## Allowed dependencies

- `kebab-chunk` 에 새 모듈 3개 (`k8s_manifest_resource_v1.rs` / `dockerfile_file_v1.rs` / `manifest_file_v1.rs`) 및 dep entry `serde_yaml = { workspace = true }` (workspace 에 이미 존재). 기존 deps (kebab-core / serde_json_canonicalizer / blake3 / anyhow / tracing) 유지.
- `kebab-parse-code` 의 `lang.rs` 갱신만. extractor 모듈 추가 없음, 새 crate dep 없음.
- `kebab-source-fs/src/media.rs` — `code_lang_for_path` 호출로 inline match 정리. 기존 dep 유지 (kebab-parse-code 는 이미 의존).
- `kebab-app::ingest_one_code_asset` — match 분기 확장. 새 crate dep 없음.

## Forbidden dependencies

- `kebab-chunk` 가 store / embed / llm / rag / tree-sitter 직접 import 금지 (boundary §6.3 유지).
- `kebab-parse-code` 가 store / embed / llm / rag 직접 import 금지.
- UI crate (`kebab-cli` / `kebab-mcp` / `kebab-tui` / `kebab-desktop`) 가 `kebab-parse-code` / `kebab-chunk` 직접 import 금지 — `kebab-app` facade 만.

## Risks / notes

- **serde_yaml line offset 없음** → 원본 텍스트의 `^---\s*$` 정규식 split 으로 line 추적. trailing `---` 의 빈 슬라이스 / 첫 슬라이스에 `---` prefix 없음 / 비-표준 separator (예: `--- # comment`) 모두 fixture 로 검증.
- **apiVersion / kind 가 string 이 아닌 경우** (예: `kind: 42`) — `serde_yaml::Value::as_str()` 으로 string 체크 후 인정. 비-string 이면 비-k8s 취급.
- **cluster-scoped resource** (Namespace, ClusterRole, ClusterRoleBinding, …) — metadata.namespace 없음이 정상. symbol = `<kind>/<name>` 형태.
- **metadata.name 누락** — 비정상이지만 panic 금지. `<kind>/<unnamed>` fallback + warning log.
- **거대 ConfigMap / Helm-rendered manifest** — `AST_CHUNK_MAX_LINES = 200` oversize fallback. split chunk 가 같은 symbol 공유 → search 시 dedupe 또는 user-visible 두 hit 으로 보임 (1A-2 의 oversize 와 동일 동작).
- **YAML anchor / merge keys (`&`, `<<`, `*`)** — serde_yaml 가 자동 resolve. 원본 텍스트 보존 정책상 chunk text 는 원본 (resolve 전) 유지, 파싱은 resolve 후 값으로.
- **`Dockerfile.example` 같은 doc-purpose 파일** — 확장자/접두사 매칭에 잡힘. user intent 와 어긋날 수 있으나 본 phase 의 scope 밖 (skip 정책은 1A-1 의 size/built-in/generated 정책으로 통제). dogfood 후 false positive 빈도 보고 HOTFIXES 결정.
- **`pom.xml` aggregate parent POM** — 매우 큼 (수백~수천 줄). oversize fallback 으로 split. 거대 fixture 로 한 번 검증.
- **`media.rs` 정리** — 1A-1 부터 누적된 inline `match extension` duplication 을 `code_lang_for_path` 호출로 교체. 기존 단위 테스트 동작 보존 (테스트는 결과 값만 보므로 통과해야 함).
- **머지 후 deviation** 은 `tasks/HOTFIXES.md` dated 로그 + 본 spec `Risks / notes` 에 one-line cross-link.
- **[HOTFIXES 2026-05-21]** multi-resource k8s YAML (2+ document) 이 `chunk_id` 충돌로 ingest 실패 — `push_chunks_with_oversize` 의 non-oversize 분기가 `split_key = None` 하드코딩. PR #158 (v0.16.1) 에서 `base_split_key` 파라미터로 fix. See `tasks/HOTFIXES.md` 2026-05-21 entry.
