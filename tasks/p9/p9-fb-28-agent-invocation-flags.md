---
phase: P9
component: kebab-cli + kebab-app
task_id: p9-fb-28
title: "Agent invocation flags (--readonly + --quiet)"
status: open
target_version: 0.3.0
depends_on: []
unblocks: []
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§10 UX]
source_feedback: 사용자 도그푸딩 2026-05-06 — agent 가 KB 안전하게 사용 + progress 노이즈 끄기 명시 control 필요. shared / multi-agent host 에서 destructive 명령 차단 필수.
---

# p9-fb-28 — Agent invocation flags

> ⏳ **백로그 only — 미구현.** 본 spec 은 도그푸딩 피드백 skeleton. 구현 착수 전 [superpowers:brainstorming](../../docs/superpowers/) 으로 설계 단계 선행 필요. read-only 의 강제력 (env vs flag vs sub-binary) / quiet 의 범위 (stderr 전체 vs progress 만) brainstorm 후 확정.

## 증상 / 동기

- agent 가 실수로 `kebab nuke` / `kebab reset` 호출 위험 — read-only mode 강제 필요.
- agent invoke 시 progress / spinner stderr 출력이 noise — 명시 quiet flag 필요.
- 현재 TTY auto-detect 로 부분 해결되지만 TTY 가 emulate 된 환경 (예: agent host 의 pty wrapper) 에서 의도 안 한 spinner.

## Goal (skeleton)

- `KEBAB_READONLY=1` env 또는 `kebab --readonly <subcommand>` — destructive 명령 (`reset`, `nuke`, `ingest --overwrite` 등) 거부.
- `kebab --quiet <subcommand>` — 모든 stderr progress / hint 끔. error 만 stderr.
- agent host 권장 patterns README 에 명시 (예: skill 의 invocation env block).

## 후속 작업 — brainstorm 필요 항목

- read-only 의 enforcement layer — argparse vs runtime check.
- quiet 와 `--json` 관계 — `--json` 이 자동 quiet 인지.
- destructive 명령 enumerate — ingest 가 idempotent 인데 destructive 인지 분류.
- daemon (fb-29) 위에서 read-only token / scope.

## Risks / notes

- read-only bypass 우회 (config 직접 수정 등) 는 막을 수 없음 — best-effort.
- 사용자가 자기 invoke 에 readonly 걸지 않게 README 안내.
- fb-30 MCP 의 tool 별 permission 과 통합 (read tool 만 노출 vs read+write).
