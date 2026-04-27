---
title: "KB 작업 단위 인덱스"
source: kb_local_rust_report.md
date: 2026-04-27
---

# KB 작업 단위 인덱스

[`kb_local_rust_report.md`](../kb_local_rust_report.md) 의 Phase 로드맵을 아키텍처 수준 작업 단위로 분해. 각 task 문서는 독립적으로 착수/검수 가능한 단위.

## 의존 그래프

```text
P0 ── P1 ── P2 ── P3 ── P4 ── P5
                              │
                              ├─ P6 (image)
                              ├─ P7 (pdf)
                              ├─ P8 (audio)
                              └─ P9 (TUI/desktop)
```

P0~P5 는 직렬. P6~P9 는 P5 이후 병렬 가능.

## 작업 단위

| # | 코드 | 제목 | 핵심 산출 crate | 선행 |
|---|------|------|----------------|------|
| P0 | [phase-0-skeleton.md](phase-0-skeleton.md) | Workspace 뼈대 + 도메인 계약 | kb-core, kb-config, kb-app, kb-cli | – |
| P1 | [phase-1-markdown-ingestion.md](phase-1-markdown-ingestion.md) | Markdown ingestion 파이프라인 | kb-source-fs, kb-parse-md, kb-normalize, kb-chunk, kb-store-sqlite | P0 |
| P2 | [phase-2-lexical-search.md](phase-2-lexical-search.md) | SQLite FTS5 lexical 검색 + citation | kb-search (lexical) | P1 |
| P3 | [phase-3-vector-hybrid.md](phase-3-vector-hybrid.md) | Local embedding + LanceDB + hybrid | kb-embed, kb-embed-local, kb-store-vector, kb-search | P2 |
| P4 | [phase-4-local-llm-rag.md](phase-4-local-llm-rag.md) | Local LLM + RAG + grounded answer | kb-llm, kb-llm-local, kb-rag | P3 |
| P5 | [phase-5-evaluation.md](phase-5-evaluation.md) | Golden query / regression eval | kb-eval | P4 |
| P6 | [phase-6-image.md](phase-6-image.md) | 이미지 ingestion (OCR + caption) | kb-parse-image | P5 |
| P7 | [phase-7-pdf.md](phase-7-pdf.md) | PDF text + page citation | kb-parse-pdf | P5 |
| P8 | [phase-8-audio.md](phase-8-audio.md) | 음성 transcription + timestamp citation | kb-parse-audio | P5 |
| P9 | [phase-9-ui.md](phase-9-ui.md) | TUI + desktop app | kb-tui, kb-desktop | P5 |

## Component task decomposition (per phase)

각 phase 의 component-level 분해. AI sub-agent 1세션 = 1 task 가 sweet spot.

- P1 — [p1/](p1/) — Markdown ingestion 6 components
  - [p1-1 source-fs](p1/p1-1-source-fs.md)
  - [p1-2 parse-md frontmatter](p1/p1-2-parse-md-frontmatter.md)
  - [p1-3 parse-md blocks](p1/p1-3-parse-md-blocks.md)
  - [p1-4 normalize](p1/p1-4-normalize.md)
  - [p1-5 chunk](p1/p1-5-chunk.md)
  - [p1-6 store-sqlite](p1/p1-6-store-sqlite.md)

## 모든 task 공통 규약

- 의존성 경계 (`Allowed` / `Forbidden`) 위반 금지. report §19 참조.
- citation 없는 검색 결과 / RAG 응답 금지.
- 원본 파일 파괴 금지. 파생물만 재생성.
- 모든 record 에 version (parser/chunker/embedding/index/prompt) 기록.
- 각 phase 완료 = `cargo check --workspace && cargo test --workspace` 통과 + 해당 phase 의 완료 조건 CLI 데모 통과.
