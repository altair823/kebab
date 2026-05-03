---
phase: P9
component: kebab-cli + kebab-tui
task_id: p9-fb-20
title: "Citation full path + scrollable pane (CLI block + TUI pane + jump)"
status: in_progress
depends_on: [p9-fb-09, p9-fb-16]
unblocks: []
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§7 RAG, §10 UX]
source_feedback: p9-dogfooding-feedback.md item 16
---

# p9-fb-20 — Citation surface

## Goal

ask 답변의 citation 을 사용자가 풀 경로로 보고, scroll 하고, 원본으로 점프 가능. CLI / TUI 둘 다.

## Allowed dependencies

- 기존 deps. p9-fb-09 의 editor restore helper 재사용.

## Public surface

CLI:
- `kebab ask "Q" [--show-citations | --hide-citations]` (default: show).
- 출력 형식 (사람-친화):
  ```
  답변:
  ...

  근거:
  [1] crates/kebab-app/src/lib.rs#L120-L140  (score=0.78, doc_id=abc123)
  [2] notes/foo.md#L12-L34                    (score=0.71, doc_id=def456)
  ```
- `--json` 은 항상 citations 포함 (`answer.v1.citations`, 변경 X).

TUI ask pane (p9-fb-16 conversation 위에):
- 화면 분할: 위 transcript / 아래 input. 추가로 옵션 우측 1/3 citation pane (`c` 키 toggle).
- citation pane 내부:
  - 각 항목 한 줄 (full path + line range + score). truncate 안 함 — long path 는 wrap.
  - turn 별로 group (`▾ Turn 2 (3)`). fold/unfold.
  - 선택 + Enter 또는 `o` → 외부 editor 로 path 점프 (p9-fb-09 helper).
  - 선택 + `i` → P9-4 inspect pane 으로 (Doc 또는 Chunk).

## Behavior contract

- TUI citation pane 의 `c` toggle 은 NORMAL 모드 (p9-fb-12 에 등록).
- citation pane 자체 scroll (`j/k`, PageUp/Down) — focus 가 citation pane 일 때.
- focus 이동: `Tab` 으로 transcript / citation 사이.
- editor jump 시 line range 의 시작 line 으로 (`+L120` 옵션 vim/code).

## Test plan

| kind | description |
|------|-------------|
| unit | CLI 출력 형식 snapshot (full path 포함) |
| unit | TUI citation pane fold/unfold |
| unit | `o` 키 → editor spawn (mock) |
| unit | `i` 키 → InspectTarget::Chunk 진입 |

## DoD

- [ ] `cargo test -p kebab-cli -p kebab-tui` 통과
- [ ] README ask 절에 citation 동작 안내
- [ ] HOTFIXES X (신규)

## Out of scope

- citation 의 inline preview (path 위에 마우스 hover 같은) — 터미널 한계
- citation 의 PDF page render (P9-5 desktop 용)
