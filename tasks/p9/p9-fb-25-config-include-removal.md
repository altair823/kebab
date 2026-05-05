---
phase: P9
component: kebab-config
task_id: p9-fb-25
title: "Config workspace.include 제거 + 지원 형식 가시성 (post-merge dogfooding)"
status: completed
depends_on: [p9-fb-23]
unblocks: []
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§6.2 Workspace, §3.x IngestReport, §2.4a IngestEvent]
source_feedback: 사용자 도그푸딩 2026-05-05 — include + exclude 의미 모호 + 지원 형식 가시성 부족.
---

# p9-fb-25 — Config `workspace.include` 제거 + 지원 형식 가시성

상세 설계: `docs/superpowers/specs/2026-05-05-p9-fb-25-config-include-removal-design.md`.
구현 계획: `docs/superpowers/plans/2026-05-05-p9-fb-25-config-include-removal.md`.

## Goal

- `WorkspaceCfg.include` 필드 제거 (denylist-only 모델 정착).
- 사용자가 ingest 결과에서 어떤 파일이 왜 skip 됐는지 즉시 파악.
- 지원 형식 (md / png / jpg / pdf) 을 README + `kebab init` config 주석에 명시.

## Behavior contract

- 옛 config 의 `include = [...]` 은 silently 무시 + 단발 deprecation warning.
- Skipped 시 `IngestItem.warnings` = `["unsupported media type: .ext"]` 또는 `["unsupported media type: <no-ext>"]` 또는 `["kb:// URI not yet supported"]`.
- `IngestReport.skipped_by_extension` = `BTreeMap<lowercase-ext, count>`. no-ext 키 = `<no-ext>`.
- CLI / TUI summary final / aborted 라인에 `"N skipped: A docx, B txt, ..."` (desc 정렬, 모두 표시, ties by key alphabetic).

## Tests

- legacy include 무시 + 새 WorkspaceCfg 필드 destructure (kebab-config).
- skip_reason 통합 (kebab-app): docx + Makefile 두 파일 ingest → warnings + skipped_by_extension 채워짐.
- init_template 헤더 (kebab-app).
- status_line breakdown 완료 / abort (kebab-tui).

## Risks / notes

- 옛 config 가 narrow allowlist (예: `include = ["**/*.md"]`) 면 본 변경 후 `.png` 등이 자동 ingest 시작 — deprecation warning + README 가 alarm.
- `SourceScope.include` (kebab-core) 는 그대로.

Live deviations 반영 위치: `tasks/HOTFIXES.md` `2026-05-05 — p9-fb-25` 항목.
