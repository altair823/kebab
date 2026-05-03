---
title: "KB 작업 단위 인덱스"
source: kebab_local_rust_report.md
date: 2026-04-27
---

# KB 작업 단위 인덱스

[`kebab_local_rust_report.md`](../kebab_local_rust_report.md) 의 Phase 로드맵을 아키텍처 수준 작업 단위로 분해. 각 task 문서는 독립적으로 착수/검수 가능한 단위.

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
| P0 | [phase-0-skeleton.md](phase-0-skeleton.md) | Workspace 뼈대 + 도메인 계약 | kebab-core, kebab-parse-types, kebab-config, kebab-app, kebab-cli | – |
| P1 | [phase-1-markdown-ingestion.md](phase-1-markdown-ingestion.md) | Markdown ingestion 파이프라인 | kebab-source-fs, kebab-parse-md, kebab-normalize, kebab-chunk, kebab-store-sqlite | P0 |
| P2 | [phase-2-lexical-search.md](phase-2-lexical-search.md) | SQLite FTS5 lexical 검색 + citation | kebab-search (lexical) | P1 |
| P3 | [phase-3-vector-hybrid.md](phase-3-vector-hybrid.md) | Local embedding + LanceDB + hybrid | kebab-embed, kebab-embed-local, kebab-store-vector, kebab-search | P2 |
| P4 | [phase-4-local-llm-rag.md](phase-4-local-llm-rag.md) | Local LLM + RAG + grounded answer | kebab-llm, kebab-llm-local, kebab-rag | P3 |
| P5 | [phase-5-evaluation.md](phase-5-evaluation.md) | Golden query / regression eval | kebab-eval | P4 |
| P6 | [phase-6-image.md](phase-6-image.md) | 이미지 ingestion (OCR + caption) | kebab-parse-image | P5 |
| P7 | [phase-7-pdf.md](phase-7-pdf.md) | PDF text + page citation | kebab-parse-pdf | P5 |
| P8 | [phase-8-audio.md](phase-8-audio.md) | 음성 transcription + timestamp citation | kebab-parse-audio | P5 |
| P9 | [phase-9-ui.md](phase-9-ui.md) | TUI + desktop app | kebab-tui, kebab-desktop | P5 |

## Component task decomposition (per phase)

각 phase 의 component-level 분해. AI sub-agent 1세션 = 1 task 가 sweet spot.

- P0 — [p0/](p0/) — 1 component
  - [p0-1 skeleton](p0/p0-1-skeleton.md)
- P1 — [p1/](p1/) — 6 components
  - [p1-1 source-fs](p1/p1-1-source-fs.md)
  - [p1-2 parse-md frontmatter](p1/p1-2-parse-md-frontmatter.md)
  - [p1-3 parse-md blocks](p1/p1-3-parse-md-blocks.md)
  - [p1-4 normalize](p1/p1-4-normalize.md)
  - [p1-5 chunk](p1/p1-5-chunk.md)
  - [p1-6 store-sqlite](p1/p1-6-store-sqlite.md)
- P2 — [p2/](p2/) — 2 components
  - [p2-1 fts-schema](p2/p2-1-fts-schema.md)
  - [p2-2 lexical-retriever](p2/p2-2-lexical-retriever.md)
- P3 — [p3/](p3/) — 5 components
  - [p3-1 embedder-trait](p3/p3-1-embedder-trait.md)
  - [p3-2 fastembed-adapter](p3/p3-2-fastembed-adapter.md)
  - [p3-3 lancedb-store](p3/p3-3-lancedb-store.md)
  - [p3-4 hybrid-fusion](p3/p3-4-hybrid-fusion.md)
  - [p3-5 app-wiring](p3/p3-5-app-wiring.md)
- P4 — [p4/](p4/) — 3 components
  - [p4-1 llm-trait](p4/p4-1-llm-trait.md)
  - [p4-2 ollama-adapter](p4/p4-2-ollama-adapter.md)
  - [p4-3 rag-pipeline](p4/p4-3-rag-pipeline.md)
- P5 — [p5/](p5/) — 2 components
  - [p5-1 golden-fixture-runner](p5/p5-1-golden-fixture-runner.md)
  - [p5-2 metrics-compare](p5/p5-2-metrics-compare.md)
- P6 — [p6/](p6/) — 4 components
  - [p6-1 image-extractor-exif](p6/p6-1-image-extractor-exif.md)
  - [p6-2 ocr-adapter](p6/p6-2-ocr-adapter.md)
  - [p6-3 caption-adapter](p6/p6-3-caption-adapter.md)
  - [p6-4 image-ingest-wiring](p6/p6-4-image-ingest-wiring.md)
- P7 — [p7/](p7/) — 3 components
  - [p7-1 pdf-text-extractor](p7/p7-1-pdf-text-extractor.md)
  - [p7-2 pdf-page-chunker](p7/p7-2-pdf-page-chunker.md)
  - [p7-3 pdf-ingest-wiring](p7/p7-3-pdf-ingest-wiring.md)
- P8 — [p8/](p8/) — 2 components
  - [p8-1 whisper-adapter](p8/p8-1-whisper-adapter.md)
  - [p8-2 segment-chunker](p8/p8-2-segment-chunker.md)
- P9 — [p9/](p9/) — 5 components + 도그푸딩 피드백
  - [p9-1 tui-library](p9/p9-1-tui-library.md)
  - [p9-2 tui-search](p9/p9-2-tui-search.md)
  - [p9-3 tui-ask](p9/p9-3-tui-ask.md)
  - [p9-4 tui-inspect](p9/p9-4-tui-inspect.md)
  - [p9-5 desktop-tauri](p9/p9-5-desktop-tauri.md)
  - [p9-dogfooding 피드백 인덱스](p9/p9-dogfooding-feedback.md) — 사용자가 직접 돌려보며 수집한 UX 잡음 → p9-fb-01 ~ 20 으로 분해 + 구현 **20/20 ✅ (2026-05-03 완료)**
  - [p9-fb-01 ingest progress callback](p9/p9-fb-01-ingest-progress-callback.md)
  - [p9-fb-02 CLI progress display](p9/p9-fb-02-cli-progress-display.md)
  - [p9-fb-03 TUI ingest background](p9/p9-fb-03-tui-ingest-background.md)
  - [p9-fb-04 ingest cancellation](p9/p9-fb-04-ingest-cancellation.md)
  - [p9-fb-05 config path policy](p9/p9-fb-05-config-path-policy.md)
  - [p9-fb-06 kebab reset](p9/p9-fb-06-data-reset-command.md)
  - [p9-fb-07 MD title fallback](p9/p9-fb-07-md-title-fallback.md)
  - [p9-fb-08 search debounce](p9/p9-fb-08-search-debounce.md)
  - [p9-fb-09 editor restore](p9/p9-fb-09-tui-editor-restore.md)
  - [p9-fb-10 CJK input](p9/p9-fb-10-tui-cjk-input.md)
  - [p9-fb-11 ask markdown render](p9/p9-fb-11-ask-markdown-render.md)
  - [p9-fb-12 mode machine](p9/p9-fb-12-tui-mode-machine.md)
  - [p9-fb-13 cheatsheet](p9/p9-fb-13-tui-cheatsheet.md)
  - [p9-fb-14 color theme](p9/p9-fb-14-tui-color-theme.md)
  - [p9-fb-15 RAG multi-turn core](p9/p9-fb-15-rag-multi-turn-core.md)
  - [p9-fb-16 TUI ask conversation](p9/p9-fb-16-tui-ask-conversation.md)
  - [p9-fb-17 chat session storage (V004)](p9/p9-fb-17-chat-session-storage.md)
  - [p9-fb-18 CLI ask session/repl](p9/p9-fb-18-cli-ask-session-repl.md)
  - [p9-fb-19 search cache](p9/p9-fb-19-search-cache.md)
  - [p9-fb-20 citation surface](p9/p9-fb-20-citation-surface.md)
  - [p9-fb-21 Insert-key + F1 visibility (post-도그푸딩)](p9/p9-fb-21-tui-insert-key-discoverability.md)

## Post-merge 핫픽스

머지 후 발견된 버그들과 그 follow-up PR들은 [HOTFIXES.md](HOTFIXES.md)에 dated 로그로 기록한다. 원래 task spec은 frozen 상태로 두고, post-merge 동작 변경은 HOTFIXES.md를 source of truth로 본다.

## 모든 task 공통 규약

- 의존성 경계 (`Allowed` / `Forbidden`) 위반 금지. report §19 참조.
- citation 없는 검색 결과 / RAG 응답 금지.
- 원본 파일 파괴 금지. 파생물만 재생성.
- 모든 record 에 version (parser/chunker/embedding/index/prompt) 기록.
- 각 phase 완료 = `cargo check --workspace && cargo test --workspace` 통과 + 해당 phase 의 완료 조건 CLI 데모 통과.
