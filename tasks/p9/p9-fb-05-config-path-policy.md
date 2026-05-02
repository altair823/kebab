---
phase: P9
component: kebab-config + kebab-cli (init) + README
task_id: p9-fb-05
title: "workspace.root path policy (relative? + init placeholder + README)"
status: planned
depends_on: []
unblocks: []
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§6.2 workspace]
source_feedback: p9-dogfooding-feedback.md item 3
---

# p9-fb-05 — Path policy

## Goal

`workspace.root` 의 허용 형식 명확화: tilde / 절대 / 상대 경로 모두 지원하되 base 정의 명시. `kebab init` 가 생성하는 placeholder + 코멘트 + README 도 동시 갱신.

## Allowed dependencies

- 기존 kebab-config deps.

## Public surface

`kebab_config::expand_path` 가 이미 tilde + env 처리. relative path 처리 추가:

```rust
/// `path` 를 expand. relative 인 경우 `base_dir` 기준으로 절대화.
pub fn expand_path_with_base(path: &str, data_dir: &str, base_dir: &Path) -> PathBuf;
```

`workspace.root` 의 base_dir 은 **config.toml 자체가 위치한 디렉토리**. config 가 따라다니므로 사용자의 cwd 무관.

## Behavior contract

- 허용 형식: 절대 (`/foo/bar`) / tilde (`~/KnowledgeBase`) / env (`${XDG_DATA_HOME}/...`) / 상대 (`./notes`, `notes`, `../parent/x`).
- 상대 경로의 base = config 파일 dir. `--config /tmp/test/config.toml` + `root = "kb"` → `/tmp/test/kb`.
- `kebab init` placeholder: `~/KnowledgeBase` 그대로 — tilde 가 가장 친숙. config.toml 코멘트로 base 정의 명시:
  ```toml
  [workspace]
  # 절대 / `~` / `${VAR}` / 상대 경로 모두 가능. 상대 경로는
  # 이 config.toml 이 있는 디렉토리 기준.
  root = "~/KnowledgeBase"
  ```
- README **Configuration** 절에 base 정의 추가.
- SMOKE.md 의 `/tmp/kebab-smoke/config.toml` 예시도 갱신 가능 (기존 절대 경로라 OK).

## Test plan

| kind | description |
|------|-------------|
| unit | `expand_path_with_base("./notes", "", "/tmp/test")` → `/tmp/test/notes` |
| unit | `expand_path_with_base("~/x", "", "/tmp/test")` → `$HOME/x` |
| integration | `kebab ingest --config /tmp/cfg.toml` (root 가 상대) 가 cfg dir 기준 |

## DoD

- [ ] `cargo test -p kebab-config` 통과
- [ ] `kebab-app::ingest` 의 root expand 가 `expand_path_with_base` 로 통일
- [ ] `kebab init` 코멘트 갱신
- [ ] README + SMOKE.md 동시 갱신

## Out of scope

- `expand_tilde` helper 통일 (P+ — HOTFIXES caveat)
- 다른 경로 필드 (`storage.data_dir` 등) policy 변경 — 현재 그대로 OK
