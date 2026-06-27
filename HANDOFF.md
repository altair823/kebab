# HANDOFF — 진척도

> "지금 어디까지 됐고 다음에 뭘 할지"의 phase-단위 단일 출처. 사용법 → [README.md](README.md), 구조 → [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md), per-component 진행 → [tasks/INDEX.md](tasks/INDEX.md), 머지 후 deviation(동작 진실) → [tasks/HOTFIXES.md](tasks/HOTFIXES.md), 버전별 변경 → [CHANGELOG.md](CHANGELOG.md). 전체 문서 지도 → [DOCS.md](DOCS.md).

## 한 줄 요약

현재 **v0.32.0**. P0–P7 + P9(CLI/MCP) + P10 머지 완료, P8(audio)·P9-5(desktop)만 보류. `kebab ingest` 가 markdown · 이미지(OCR+caption) · PDF(텍스트+page citation+scanned OCR) · 소스코드(Rust/Python/TS/JS/Go/Java/Kotlin/C/C++ AST + Tier2 리소스 + Tier3 fallback) 를 단일 binary 로 색인. `kebab search`(lexical/vector/hybrid RRF) / `kebab ask`(RAG + 근거 인용 + NLI groundedness + multi-hop) 가 매체 가로질러 동작. UI 는 CLI + MCP stdio server(TUI 는 v0.31.0 에서 제거).

## Phase 로드맵

| Phase | 내용 | 핵심 crate | 상태 |
|-------|------|-----------|------|
| **P0** | Workspace 뼈대 + 도메인 계약 + ID recipe | `kebab-core`/`kebab-config`/`kebab-app`/`kebab-cli` | ✅ |
| **P1** | Markdown ingestion (walk→parse→chunk→SQLite) | `kebab-source-fs`/`kebab-parse-md`/`kebab-chunk`/`kebab-store-sqlite` | ✅ |
| **P2** | FTS5 lexical 검색 + citation | `kebab-search` | ✅ |
| **P3** | embedding + LanceDB + hybrid(RRF) | `kebab-embed-local`/`kebab-store-vector`/`kebab-search` | ✅ |
| **P4** | LLM + RAG + grounded answer + NLI | `kebab-llm-local`/`kebab-rag`/`kebab-nli` | ✅ |
| **P5** | golden query / regression eval | `kebab-eval` | ✅ |
| **P6** | 이미지 ingestion (OCR + caption) | `kebab-parse-image` | ✅ |
| **P7** | PDF text + page citation + scanned OCR | `kebab-parse-pdf` + `kebab-app::pdf_ocr_apply` | ✅ |
| **P8** | 음성 transcription | `kebab-parse-audio` | ⏸ 보류 (whisper-rs 시스템 dep brainstorm 필요) |
| **P9** | agent surface (CLI/MCP/desktop) | `kebab-mcp`, `kebab-desktop`(P9-5) | 🟡 CLI/MCP ✅, desktop ⏸ |
| **P10** | code ingest framework | `kebab-parse-code`/`kebab-chunk` | ✅ (Rust/Py/TS/JS/Go/Java/Kotlin/C/C++ + Tier2/3) |

> trait 전용이던 `kebab-embed`/`kebab-llm`/`kebab-parse-types`/`kebab-normalize` 는 흡수됨(각각 kebab-core mock feature / kebab-parse-md module) — 현재 20 crate. 상세는 [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## 최근 마일스톤

버전별 변경은 [CHANGELOG.md](CHANGELOG.md), 머지 후 deviation 의 dated 로그는 [tasks/HOTFIXES.md](tasks/HOTFIXES.md)(동작이 설계와 다를 때의 진실)에 있다. 굵직한 흐름만:

- **v0.32.0** — ponytail-audit over-engineering 정리 (죽은 scaffold 제거, 9 AST chunker→1, shim crate 흡수 22→20).
- **v0.31.0** — 척추 단순화(TUI·세션·candle 제거, config v5) + 임베딩/OCR/caption 캐시 전면화.
- **v0.27–0.30** — paddle-onnx 네이티브 OCR · provenance 출처 필터 · md-heading-v2 oversize 분할.
- **v0.18–0.26** — multi-hop RAG + NLI 검증 · 코드 ingest(P10) · arctic 임베더 · 한국어 형태소 검색.

## 다음 task 후보

- **P9-5 desktop (Tauri)** — 마지막 구조적 미완 component. `kebab-desktop` crate + PDF citation rendering UI. 사용자 우선순위(책·PDF) 부합.
- **P8 audio** — whisper-rs 시스템 dep vs 외부 transcription endpoint 결정 필요. 사용자 패턴상 보류.
- 그 외는 도그푸딩 follow-up — 발견 시 HOTFIXES 에 dated entry + 필요 시 ARCHITECTURE 갱신.

## 검증

릴리스 binary 의 종단 동작은 [docs/SMOKE.md](docs/SMOKE.md)(격리 KB 1세션 검증) + [docs/DOGFOOD.md](docs/DOGFOOD.md)(기능별 시나리오). 실 데이터 도그푸딩 evidence 는 HOTFIXES dated entry.
