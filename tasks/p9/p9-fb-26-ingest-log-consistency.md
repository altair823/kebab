---
phase: P9
component: kebab-cli
task_id: p9-fb-26
title: "Ingest 로그 출력 일관성 (in-place vs 새 줄 혼재)"
status: open
target_version: 0.3.0
depends_on: [p9-fb-02]
unblocks: []
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§7 ingest, §10 UX, §2.4a IngestEvent]
source_feedback: 사용자 도그푸딩 2026-05-05 — `kebab ingest` 진행 로그가 어떨 땐 새 라인으로, 어떨 땐 기존 라인 in-place 갱신으로 나와 일관성 떨어짐.
---

# p9-fb-26 — Ingest 로그 출력 일관성

> ⏳ **백로그 only — 미구현.** 본 spec 은 도그푸딩 피드백 skeleton. 구현 착수 전 [superpowers:brainstorming](../../docs/superpowers/) 으로 설계 단계 선행 필요. 옵션 A/B/C 중 결정 + behavior contract 확정 후 implementation PR 진행.

## 증상

`kebab ingest` 실행 중 진행 출력이 두 패턴 혼재:

- **TTY**: indicatif `ProgressBar` 단일 라인 in-place 갱신 (`ScanStarted` / `ScanCompleted` / `AssetStarted` / `AssetFinished`). 마지막 `Completed` / `Aborted` summary 만 별도 새 라인.
- **non-TTY (pipe / less / CI)**: 매 event 마다 `writeln!` 새 라인.
- 같은 세션 안에서도 환경 변경 (예: `kebab ingest 2>&1 | tee log`) 시 출력 형식 바뀜.

사용자 인지: "어떨 땐 새 라인, 어떨 땐 기존 라인 업데이트". 시각적 노이즈 + 스크롤백에 진행 흔적 일부만 남거나 전체 남는 비대칭.

## 원인

[crates/kebab-cli/src/progress.rs:99-188](../../crates/kebab-cli/src/progress.rs#L99-L188):

- TTY branch 는 `bar.set_message` / `bar.set_position` 으로 단일 라인 갱신.
- non-TTY branch (`if !tty { writeln!(err, ...) }`) 가 모든 event 마다 추가 라인.
- Completed / Aborted 는 TTY 에서도 `writeln!` 항상 호출 — bar finish 후 summary 한 줄.

## Goal

진행 로그의 출력 형식을 환경 무관하게 예측 가능하게 만든다. 사용자는 한 가지 형식을 보고 다른 환경에서도 같은 형식을 기대해야 함.

## Behavior contract (제안 — brainstorming 단계, 머지 전 사용자 확인)

옵션 A — **TTY = in-place, non-TTY = append-only, 둘 다 명시적**:
- TTY: 진행 라인은 in-place, summary 도 마지막에 같은 라인 commit (현재 처럼 새 줄 X) 또는 한 줄 띄우고 명시적 final.
- non-TTY: 매 event 한 줄 (현재 동작 유지) — pipe / log redirect 에서 진행 흔적 남는 게 정답.
- summary 라인은 두 모드 동일한 prefix (`ingest: complete (...)` / `ingest: aborted (...)`) 로 통일.

옵션 B — **항상 append-only**:
- TTY 에서도 spinner 끄고 매 event 새 라인. 단순, 진행 흔적 보존.
- 단점: bar UX 손실 — long ingest 에서 화면 가득.

옵션 C — **항상 in-place (TTY 만)**:
- non-TTY 면 마지막 summary 한 줄만, 중간 event silent.
- 단점: CI / log redirect 에서 진행 알 수 없음.

권장: **옵션 A** — 환경별 의미 명확, 두 형식 다 의도된 것임을 README 에 명시.

## 검증 / 테스트

- `kebab ingest` TTY 모드: spinner 한 라인만 차지, 종료 후 summary 한 줄.
- `kebab ingest 2>&1 | cat`: append-only 형식, 매 asset / scan event 한 줄.
- `kebab ingest --json`: 기존 ndjson 동작 유지 (영향 없음).
- snapshot test: non-TTY 스트림 포맷 안정.

## 관련 항목

- p9-fb-01 / p9-fb-02 (progress 인프라). 본 항목은 그 위의 일관성 follow-up.
- 사용자 visible surface — README **명령** 표 / Quick start 의 ingest 출력 예시 갱신 필요.

## Risks / notes

- indicatif `ProgressDrawTarget::stderr()` 가 일부 터미널 (TUI multiplexer, 일부 ssh client) 에서 in-place 갱신 fallback 으로 새 라인 그릴 수 있음 — 조사 필요.
- CI 가 TTY-emulating wrapper (예: GitHub Actions 일부) 면 의도 안 한 in-place 모드 진입 가능. 명시적 `KEBAB_PROGRESS=plain` env override 검토.
