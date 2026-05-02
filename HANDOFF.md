# HANDOFF — 진척도

> 새 conversation / 다른 사람이 이어받을 때 \"지금 어디까지 됐고 다음에 뭘 할지\" 의 단일 출처. 사용자 사용법은 [README.md](README.md), 아키텍처는 [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md), per-component 진행은 [tasks/INDEX.md](tasks/INDEX.md), 머지 후 발견된 버그는 [tasks/HOTFIXES.md](tasks/HOTFIXES.md). 이 파일은 \"phase 단위 진척\" + \"다음 task 후보\" 만 담는다.

## 한 줄 요약

P0–P5 + P6 + P7 + P9-1/2/3/4 (Library / Search / Ask / Inspect) 머지 완료. `kebab ingest` 가 markdown / image / PDF 모두 처리. `kebab search` / `kebab ask` 가 매체 가로질러 결과 + page citation 반환. `kebab tui` 가 4 패널 (Library + Search + Ask + Inspect) 제공 — 사용자가 `?` 로 ask, `/` 로 search, Library Enter / Search `i` 로 inspect, Search `g` 로 editor jump. 다음 후보 = P9-5 (desktop tauri) 또는 보류 중인 P8 (audio) 의 시스템 dep brainstorm.

## Phase 로드맵

| Phase | 내용 | 핵심 산출 crate | 선행 | 상태 |
|-------|------|----------------|------|------|
| **P0** | Workspace 뼈대 + 도메인 계약 + ID recipe | `kebab-core`, `kebab-parse-types`, `kebab-config`, `kebab-app`, `kebab-cli` | – | ✅ 완료 |
| **P1** | Markdown ingestion (walk → parse → chunk → SQLite) | `kebab-source-fs`, `kebab-parse-md`, `kebab-normalize`, `kebab-chunk`, `kebab-store-sqlite` | P0 | ✅ 완료 |
| **P2** | SQLite FTS5 lexical 검색 + citation | `kebab-search` (lexical) | P1 | ✅ 완료 |
| **P3** | Local embedding + LanceDB + hybrid (RRF) + kebab-app wiring | `kebab-embed`, `kebab-embed-local`, `kebab-store-vector`, `kebab-search` | P2 | ✅ 완료 |
| **P4** | Local LLM + RAG + grounded answer | `kebab-llm`, `kebab-llm-local`, `kebab-rag` | P3 | ✅ 완료 |
| **P5** | Golden query / regression eval | `kebab-eval` | P4 | ✅ 완료 |
| **P6** | 이미지 ingestion (OCR + caption) | `kebab-parse-image` | P5 | ✅ 완료 (4/4 component, OCR/caption Ollama-vision) |
| **P7** | PDF text + page citation | `kebab-parse-pdf` | P5 | ✅ 완료 (3/3 component, page-level chunker + ingest wiring) |
| **P8** | 음성 transcription + timestamp citation | `kebab-parse-audio` | P5 | ⏸ 보류 (whisper-rs 시스템 dep brainstorm 필요) |
| **P9** | TUI + desktop app | `kebab-tui`, `kebab-desktop` | P5 | 🟡 진행 (4/5 component — P9-1/2/3/4 완료 [Library / Search / Ask / Inspect], P9-5 desktop 예정) |

P0~P5 직렬. P6~P9 P5 이후 병렬 가능.

## Component 카운트

총 33 component task — spec 시점 31 개 + 후속 wiring task 3 (P3-5 / P6-4 / P7-3) 가 머지 시점에 추가됨. per-component 진행 + status 는 [tasks/INDEX.md](tasks/INDEX.md).

## 머지 후 발견된 버그 / 결정 (요약)

머지 후 발견된 모든 deviation / hotfix 의 dated 로그는 [tasks/HOTFIXES.md](tasks/HOTFIXES.md). 본 요약은 \"누군가가 인수받을 때 알아두면 시간을 많이 절약하는\" 항목만:

- **P3-5 / P4-3 `--config` 누락** — `kebab-cli` 가 `--config <path>` 를 honor 하려면 `kebab_app::*_with_config` companion 을 호출해야 함. 두 번 같은 모양으로 회귀했음.
- **P6-2 OCR 기본 엔진** — spec literal 의 Tesseract 가 시스템 dep 부담으로 거부됨, Ollama vision LM 으로 대체. `OcrEngine` trait 그대로라 future swap 가능.
- **P6-3 caption** — `GenerateRequest.images` 필드를 `kebab-core::LanguageModel` trait 에 신설. 기존 caller 모두 `images: Vec::new()` 로 마이그레이션.
- **P7-2 `chunk_id` 충돌** — pdf-page-v1 가 한 페이지 여러 chunk 분할 → 같은 `block_ids` 충돌. per-chunk `policy_hash#c{char_start}` 변형 으로 회피.
- **P7-3 storage UNIQUE bug** — `assets.workspace_path` UNIQUE + `upsert_asset_row` 의 `ON CONFLICT(asset_id)` gap 으로 byte 변경 re-ingest 실패. `purge_orphan_at_workspace_path` helper 추가, follow-up PR 으로 vector store orphan cleanup 까지 닫음 (`VectorStore::delete_by_chunk_ids`).
- **P9-1 ratatui 0.28** — spec literal 의 `render_library<B: Backend>` generic 이 ratatui 0.28 의 backend-agnostic Frame 과 어긋나 있어 제거. 테스트 seam `App::populate_library_for_testing` (`#[doc(hidden)]`) 추가.
- **P9-2 jump_to_citation workspace_root** — spec literal 의 `jump_to_citation(citation, editor_env)` 가 workspace_root 인자 누락. citation.path 가 workspace 상대라 editor 호출 시 절대 경로 필요 → `workspace_root: &Path` 인자 추가. 동일하게 `render_search<B: Backend>` generic 도 P9-1 과 같은 사유로 제거.
- **P9-3 e/j/k 키 의 \"input empty\" 분기** — spec 의 `e=toggle explain` / `j=k=scroll` 이 typing 과 충돌 (\"explain\" / \"javascript\" 같은 단어 입력 깨짐). input 이 비어 있을 때만 command 키로 동작 — vim \"command vs insert\" 컨벤션 변형. 사용자가 텍스트 입력 시 모든 알파벳 정상 통과.
- **P9-4 enter_inspect helper + Search `i` 키** — spec 의 진입 경로 (Library Enter → Doc inspect, Search `i` → Chunk inspect) 를 한 helper 로 묶음. `InspectTarget` enum (`Doc(DocumentId) | Chunk(ChunkId)`), `return_to: Pane` 가 Esc 시 원래 pane 으로 복귀. `c` 키가 모든 section (metadata / provenance / blocks / spans / text / embeddings) 일괄 collapse/expand — spec 의 \"focus 기반 selective collapse\" 는 v1 단순화.

## 다음 task 후보

- **P9-2 TUI search** — `App.search` slot 채움. Library 의 `/` 가 enable 됨.
- **P9-3 TUI ask** — `App.ask` slot 채움. `?` enable.
- **P9-4 TUI inspect** — `App.inspect` slot 채움. `Enter` enable.
- **P9-5 desktop tauri** — 별도 분기. PDF citation rendering UI 가치 큼.
- **P8 audio brainstorm** — whisper-rs 시스템 dep 받을지 / 외부 transcription endpoint 사용할지 사용자 결정 필요. 사용자 패턴 (책+PDF 위주, audio 의향 없음) 상 후순위.

P9-2/3/4 는 P9-1 의 parallel-safety contract (sub-state slot 패턴) 덕에 병렬 진행 가능 — 같은 `App` 손대지 않음.

## 검증된 운영 동작 (release binary, fastembed enabled)

P7-3 머지 직후 25 시나리오 smoke 통과 — markdown + image + PDF 5 자산 워크스페이스에서 doctor / ingest / list / inspect / search (lex/vec/hybrid) / re-ingest / byte-edit re-ingest / corrupt PDF / RAG ask + page citation 모두. 자세한 시나리오 표는 conversation 기록 참조; 워크스페이스에 직접 돌려보는 절차는 [docs/SMOKE.md](docs/SMOKE.md).
