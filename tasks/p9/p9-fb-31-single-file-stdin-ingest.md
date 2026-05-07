---
phase: P9
component: kebab-cli + kebab-app
task_id: p9-fb-31
title: "Single-file / stdin ingest — agent on-demand 저장"
status: completed
target_version: 0.3.0
depends_on: []
unblocks: []
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§3 ingest, §10 UX]
source_feedback: 사용자 도그푸딩 2026-05-06 — agent 가 읽은 article / fetch 한 page 를 즉시 KB 저장 needed. 현재 ingest 는 workspace 전체 scan 만.
---

# p9-fb-31 — Single-file / stdin ingest

> ✅ **구현 완료.** 본 spec 은 구현 시점의 frozen 상태. post-merge deviation 은 [HOTFIXES.md](../HOTFIXES.md) 의 `2026-05-07 — p9-fb-31` 항목 참조 — live source of truth.

## 증상 / 동기

- agent 가 web 에서 fetch 한 markdown / pdf 를 KB 에 저장하려 함 — 현재는 workspace 디렉토리에 file 쓰고 `kebab ingest` 전체 재실행.
- agent 메모리상 string contents 도 stdin 으로 ingest 가능해야 — 임시 파일 거치는 비효율 제거.

## Goal (skeleton)

- `kebab ingest --file <path>` — 단일 파일만 ingest, workspace 외부도 가능 (workspace 안 copy 또는 absolute path 등록).
- `kebab ingest --stdin --media md --title "X" [--source-uri "https://..."]` — stdin 에서 contents 읽고 KB 저장.
- 결과는 기존 `ingest_report.v1` 와 동일 shape (단일 asset).
- p9-fb-23 incremental ingest 와 호환 — 단일 파일도 mtime 기반 변경 감지.

## 후속 작업 — brainstorm 필요 항목

- workspace 외부 file 저장 정책 — copy in vs reference (path 만 저장).
- stdin contents 의 doc_id 결정 — content hash + title.
- source URI metadata 표현 — wire schema 추가 필드.
- .kebabignore 우회 — 명시 ingest 면 강제? 아니면 거부?

## Risks / notes

- workspace 정의 (§6.2) 와 충돌 가능 — workspace 안 copy 가 깔끔.
- agent 가 무한 ingest 시 KB 비대 — quota / TTL 필요할 수도.
- p9-fb-23, p9-fb-25 (workspace.include 제거) 와 정책 정합성 검토.
