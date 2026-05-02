---
phase: P9
component: kebab-cli
task_id: p9-fb-02
title: "CLI progress display (spinner + text + --json line events)"
status: completed
depends_on: [p9-fb-01]
unblocks: []
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§7 ingest, §10 UX]
source_feedback: p9-dogfooding-feedback.md item 1
---

# p9-fb-02 — CLI progress display

## Goal

`kebab ingest` (그리고 추후 `kebab eval run` 등 long-running 명령) 이 stderr 에 spinner + 진행 라인을 그리고, `--json` 모드면 stdout 에 line-delimited progress event 를 dump.

## Allowed dependencies

- `kebab-app` (progress event 소비)
- `indicatif = "0.17"` 또는 자체 minimal spinner (선호: indicatif — 검증된 라이브러리)
- `serde_json`

## Public surface

`kebab-cli` 내부 함수 — public API 변경 없음. progress receiver thread 가 event 받아 indicatif `ProgressBar` 갱신, `--json` 이면 별도로 stdout 한 줄.

## Behavior contract

- TTY 감지: `is_terminal()` (`std::io::IsTerminal`). non-TTY (CI / pipe) 에서는 spinner 끄고 매 N 초마다 한 줄 progress 출력.
- `--json` 은 spinner 끄고 line-delimited JSON 만. 마지막 줄은 기존 `ingest_report.v1` 그대로.
- progress JSON wire schema 는 새 `ingest_progress.v1` — `docs/wire-schema/v1/ingest_progress.schema.json` 추가.
- stderr 사용 (stdout 는 `--json` 결과만, redirection 깔끔).

## Test plan

| kind | description |
|------|-------------|
| unit | ProgressDisplay 가 IngestEvent stream → 사람-친화 텍스트 변환 (no panic) |
| snapshot | `--json` line stream 이 schema 에 validate |
| integration | `kebab ingest --json` non-TTY 에서 spinner 미출력 |

## DoD

- [ ] `cargo test -p kebab-cli` 통과
- [ ] 새 wire schema `ingest_progress.v1` JSON Schema 7 + 예시
- [ ] README **명령** 표 / Quick start 갱신 (spinner / `--json` 동작 명시)

## Out of scope

- `kebab eval run` 진행 표시 (이 task 의 surface 만 이식 가능하게 두고 별도 task)
- TUI 진행 표시 (p9-fb-03)
