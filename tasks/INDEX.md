---
title: "작업 단위 인덱스 (per-component 진척)"
source: kebab_local_rust_report.md
date: 2026-04-27
---

# 작업 단위 인덱스 (per-component 진척)

> phase 단위 진척 → [HANDOFF.md](../HANDOFF.md), 동작이 설계와 다를 때의 진실 → [HOTFIXES.md](HOTFIXES.md), crate 구조·기술결정 → [docs/ARCHITECTURE.md](../docs/ARCHITECTURE.md), 전체 문서 지도 → [DOCS.md](../DOCS.md). 설계 원안 → [frozen 계약](../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md).
>
> 컴포넌트별 task 의 원본 명세(`tasks/p*/`)는 2026-06-27 doc-reorg 에서 제거됨 — 코드가 진실이고, 설계는 frozen 계약, 머지 후 변경은 HOTFIXES. 옛 task spec 은 `git log --all -- tasks/p<N>/` 로 복구 가능.

## 컴포넌트 상태 (phase별)

전부 ✅ 머지(P8 audio·P9-5 desktop 제외). 새 task 작성 템플릿은 [_template.md](_template.md), phase epic 개요는 `tasks/phase-*.md`.

| Phase | 컴포넌트 | crate / 산출 | 상태 |
|-------|----------|--------------|------|
| **P0** | skeleton · 도메인 계약 · config · app facade · cli | `kebab-core`/`kebab-config`/`kebab-app`/`kebab-cli` | ✅ |
| **P1** | source-fs walk · md frontmatter+blocks · chunk · store-sqlite | `kebab-source-fs`/`kebab-parse-md`/`kebab-chunk`/`kebab-store-sqlite` | ✅ |
| **P2** | lexical 검색 (FTS5) + citation | `kebab-search` | ✅ |
| **P3** | embedder trait · fastembed adapter · LanceDB · hybrid(RRF) · app wiring | `kebab-embed-local`/`kebab-store-vector`/`kebab-search` | ✅ |
| **P4** | llm trait · ollama adapter · RAG pipeline · NLI verifier | `kebab-llm-local`/`kebab-rag`/`kebab-nli` | ✅ |
| **P5** | golden 러너 · metrics/compare | `kebab-eval` | ✅ |
| **P6** | image extractor · OCR(ollama-vision/paddle-onnx) · caption · ingest wiring | `kebab-parse-image` | ✅ |
| **P7** | pdf text extractor · page chunker · ingest wiring · scanned OCR enrich | `kebab-parse-pdf`/`kebab-app::pdf_ocr_apply` | ✅ |
| **P8** | audio transcription | `kebab-parse-audio` | ⏸ 보류 |
| **P9** | reset · progress/cancel · multi-turn · introspection/error-wire · MCP · single-file/stdin ingest · search filters · streaming · fetch · trace/stats · multi-hop RAG · bulk (+CLI/MCP UX fb-01~42) | `kebab-mcp` 등 | ✅ (desktop P9-5 ⏸) |
| **P10** | code ingest framework · Rust/Py/TS/JS/Go/Java/Kotlin/C/C++ AST · Tier2 리소스 · Tier3 fallback | `kebab-parse-code`/`kebab-chunk` | ✅ |

> trait-전용이던 `kebab-embed`/`kebab-llm`/`kebab-parse-types`/`kebab-normalize` 흡수됨 → 현재 20 crate (ARCHITECTURE 참조).

## 그 외

- **머지 후 핫픽스 / deviation**: [HOTFIXES.md](HOTFIXES.md) (dated, live 진실).
- **버전별 변경 이력**: [CHANGELOG.md](../CHANGELOG.md).
- **Future / deferred**: P9-5 desktop(Tauri), P8 audio(whisper-rs 시스템 dep 결정 대기). HANDOFF §다음 task 후보 참조.
- **공통 규약**(facade 룰 · 의존성 경계 · 버전 cascade · wire schema)은 [CLAUDE.md](../CLAUDE.md) + [설계 계약](../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md) §8/§9.
