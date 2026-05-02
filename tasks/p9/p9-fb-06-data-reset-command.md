---
phase: P9
component: kebab-cli + kebab-app
task_id: p9-fb-06
title: "kebab reset / nuke command"
status: completed
depends_on: []
unblocks: []
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§10 UX]
source_feedback: p9-dogfooding-feedback.md item 4
---

# p9-fb-06 — Reset 명령

## Goal

`kebab reset` 단일 명령으로 XDG 데이터 wipe. 부분 wipe variant 도 제공.

## Allowed dependencies

- 기존 + `dialoguer` (또는 자체 prompt) — confirm UI.

## Public surface

CLI:
```
kebab reset [--all | --data-only | --vector-only | --config-only] [--yes]
```

flag 의미:
- `--all`: 4 XDG 경로 전부 (config + data + cache + state).
- `--data-only`: data + cache + state. config 보존 (기본).
- `--vector-only`: lance dir 만. SQLite 보존 + re-embed 필요한 chunks 표시.
- `--config-only`: config dir 만.
- `--yes`: confirm prompt skip.

`--config <path>` 도 honor — isolated workspace wipe.

## Behavior contract

- 기본 confirm: 삭제 대상 경로 4 줄 + 총 byte 추정 + `(y/N)`. n 또는 빈 입력은 abort.
- TTY 아닌 경우 `--yes` 없이 abort (silent destruction 금지).
- vector-only 의 경우: SQLite `embedding_records` row 도 같이 truncate (orphan 방지). re-embed 는 next ingest 가 처리.
- wipe 후 `kebab init` 자동 호출 X — 사용자가 명시적으로 다시 init.

## Test plan

| kind | description |
|------|-------------|
| unit | path 추정 + confirm 메시지 빌드 |
| integration | tmp config + `--data-only --yes` → data dir 삭제, config 보존 |
| integration | `--vector-only --yes` → lance dir 사라짐, embedding_records=0 |

## DoD

- [ ] `cargo test -p kebab-cli` 통과
- [ ] README **명령** 표 + Quick start 갱신 (reset 명령 + safety 안내)
- [ ] 위험성 강조: README + `--help` 에서 "irreversible"

## Out of scope

- snapshot / backup 생성 (P+ — `kebab backup` 별도)
- confirm 우회용 env (`KEBAB_RESET_YES=1` 같은 magic) 금지 — `--yes` 로 충분
