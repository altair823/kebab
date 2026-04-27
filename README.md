# kb — Local-first Knowledge Base

> **상태:** spec 동결 단계. 코드 0줄. 30 component task spec 모두 작성/리뷰/머지 완료. 다음 단계 = P0 구현.

`kb` 는 개인용 로컬 knowledge base + RAG 도구다. Markdown / PDF / 이미지 / 음성을 한 곳에 색인하고, 의미 검색 + citation 포함 LLM 답변을 단일 binary 로 제공한다. 모든 추론은 로컬 (Ollama / fastembed / whisper.cpp) 에서 돌아간다.

대상 하드웨어: M4 48GB MacBook 1대, 사용자 1명.

---

## 무엇인가

| 명령 | 동작 |
|------|------|
| `kb init` | XDG 경로에 데이터 디렉토리 + config.toml 생성 |
| `kb ingest <path>` | Markdown/PDF/이미지/음성 색인 (idempotent) |
| `kb search "<query>"` | hybrid (lexical + vector) top-k 검색 — citation 포함 |
| `kb ask "<query>"` | RAG 답변 + 근거 인용. 근거 부족 시 거절 |
| `kb inspect doc/chunk <id>` | 디버그용 raw record 보기 |
| `kb doctor` | 설정/모델/DB 헬스 체크 |
| `kb eval run / compare` | golden query 회귀 측정 (chunker/모델 교체 평가) |

기계 친화 모드: 모든 명령에 `--json` 플래그. 출력은 frozen wire schema v1 (`schema_version` 필드 항상 포함).

---

## 핵심 결정 (lock 됨)

| 결정 | 값 |
|------|-----|
| 언어 | Rust 2024 (resolver=3, edition 2024) |
| repo | Cargo workspace (single repo, 함수 호출 기반 모듈러 모놀리스) |
| 원본 저장 | filesystem + blake3 content-addressable copy (대용량은 reference + checksum) |
| metadata | SQLite + FTS5 (lexical search) |
| vector | LanceDB (embedded, model 별 분리 table) |
| Markdown parser | `pulldown-cmark` |
| embedding | `fastembed-rs` (`multilingual-e5-small`, 384d) |
| LLM | Ollama HTTP (default `qwen2.5:14b-instruct`) |
| 음성 ASR | `whisper.cpp` (via `whisper-rs`) |
| OCR | Tesseract (default) + macOS Apple Vision sidecar (feature gate) |
| TUI | Ratatui + crossterm |
| Desktop | Tauri 2 + `pdfjs-dist` (native PDF render backend 금지) |
| citation 형식 | URI fragment (`path#L12-L34`, W3C Media Fragments) |
| ID 생성 | `blake3(canonical_json(tuple))[..32]` hex |
| layout | XDG (`~/.local/share/kb/`, `~/.config/kb/`, …) |

전체는 [docs/superpowers/specs/2026-04-27-kb-final-form-design.md](docs/superpowers/specs/2026-04-27-kb-final-form-design.md) 참조.

---

## 의존성 그래프

```text
kb-cli, kb-tui, kb-desktop
   └─> kb-app
         ├─> kb-source-fs
         ├─> kb-parse-md / kb-parse-pdf / kb-parse-image / kb-parse-audio
         │     └─> kb-parse-types
         ├─> kb-normalize
         │     └─> kb-parse-types
         ├─> kb-chunk
         ├─> kb-store-sqlite
         ├─> kb-store-vector
         ├─> kb-embed-local
         ├─> kb-search
         ├─> kb-llm-local
         ├─> kb-rag
         ├─> kb-eval
         └─> kb-config
              └─> kb-core (모두 의존)
```

UI → store/llm/parse 직접 의존 금지. 모든 user-facing 진입은 `kb-app` facade 만 통한다 (design §8).

---

## Phase 로드맵

| Phase | 내용 | 핵심 산출 crate | 선행 |
|-------|------|----------------|------|
| **P0** | Workspace 뼈대 + 도메인 계약 + ID recipe | `kb-core`, `kb-parse-types`, `kb-config`, `kb-app`, `kb-cli` | – |
| **P1** | Markdown ingestion (walk → parse → chunk → SQLite) | `kb-source-fs`, `kb-parse-md`, `kb-normalize`, `kb-chunk`, `kb-store-sqlite` | P0 |
| **P2** | SQLite FTS5 lexical 검색 + citation | `kb-search` (lexical) | P1 |
| **P3** | Local embedding + LanceDB + hybrid (RRF) | `kb-embed`, `kb-embed-local`, `kb-store-vector`, `kb-search` | P2 |
| **P4** | Local LLM + RAG + grounded answer | `kb-llm`, `kb-llm-local`, `kb-rag` | P3 |
| **P5** | Golden query / regression eval | `kb-eval` | P4 |
| **P6** | 이미지 ingestion (OCR + caption) | `kb-parse-image` | P5 |
| **P7** | PDF text + page citation | `kb-parse-pdf` | P5 |
| **P8** | 음성 transcription + timestamp citation | `kb-parse-audio` | P5 |
| **P9** | TUI + desktop app | `kb-tui`, `kb-desktop` | P5 |

P0~P5 직렬. P6~P9 P5 이후 병렬 가능.

각 phase 는 component-level 단위로 더 분해되어 있다 (총 30 component task). 자세한 분해는 [tasks/INDEX.md](tasks/INDEX.md).

---

## 디렉토리 구조

```text
kb/
├── README.md                                       # 이 파일
├── kb_local_rust_report.md                         # 최초 설계 보고서 (방향성 + 근거)
├── docs/
│   ├── superpowers/
│   │   ├── specs/
│   │   │   └── 2026-04-27-kb-final-form-design.md  # frozen design (12 sections)
│   │   └── plans/
│   │       └── 2026-04-27-task-decomposition.md    # task 분해 implementation plan
│   ├── spec/                                       # P0 에서 생성 — 도메인 모델 / ID / module-boundary 등 stub
│   └── wire-schema/v1/                             # P0 에서 생성 — JSON Schema 7개 (citation, search_hit, answer, …)
├── tasks/
│   ├── INDEX.md                                    # phase 인덱스 + component task 트리
│   ├── _template.md                                # task spec 작성 템플릿
│   ├── phase-0-skeleton.md … phase-9-ui.md         # phase epic (high-level)
│   ├── p0/p0-1-skeleton.md                         # component task (1 spec)
│   ├── p1/p1-1 … p1-6                              # component tasks (6)
│   ├── p2/p2-1, p2-2                               # (2)
│   ├── p3/p3-1 … p3-4                              # (4)
│   ├── p4/p4-1 … p4-3                              # (3)
│   ├── p5/p5-1, p5-2                               # (2)
│   ├── p6/p6-1 … p6-3                              # (3)
│   ├── p7/p7-1, p7-2                               # (2)
│   ├── p8/p8-1, p8-2                               # (2)
│   └── p9/p9-1 … p9-5                              # (5)
├── crates/                                         # P0 에서 생성 — Rust crates
│   ├── kb-core/
│   ├── kb-parse-types/
│   ├── kb-config/
│   ├── kb-app/
│   └── kb-cli/
└── fixtures/                                       # P0 에서 생성 — 테스트 fixture 트리
    ├── markdown/  source-fs/  search/  embed/  vector/
    ├── rag/  eval/  image/  pdf/  audio/
    └── …
```

---

## 빌드 + 실행 (P0 완료 후)

```bash
# build
cargo build --release

# 첫 실행
./target/release/kb init

# config 손보고
${EDITOR:-vi} ~/.config/kb/config.toml

# 색인
./target/release/kb ingest ~/KnowledgeBase

# 검색
./target/release/kb search "Markdown chunking 규칙"

# 질문
./target/release/kb ask "내 KB 설계에서 저장소 전략은?"
```

**현재는 P0 미시작 — 위 명령 모두 동작하지 않는다.** spec 만 동결됐다.

---

## 비-목표 (frozen design §11 / §0)

- 다중 사용자 SaaS, K8s 배포, 원격 vector DB
- enterprise RBAC/ABAC, 실시간 협업
- 모든 파일 포맷의 완벽한 parsing
- agent 가 임의로 파일을 수정하는 자동화
- multi-workspace (P+ 후순위)
- LLM-as-judge eval (rule-based `must_contain` 만)
- visual embedding (CLIP) — P+
- desktop app `kb://` protocol handler — P+

---

## 외부 AI 통합

`kb` 의 `--json` 모드 + frozen wire schema v1 은 외부 자동화의 stable contract. 가능한 통합:

1. **Claude Code / Codex skill** — 얇은 wrapper (`kb search --json` / `kb ask --json` 호출). ~50 lines.
2. **MCP server** — `kb-mcp` binary (stdio JSON-RPC) 가 `kb-app` facade 를 1:1 노출. Claude Desktop / Cursor / Zed 등 공유.
3. **HTTP wrapper** — `kb serve --bind 127.0.0.1:7711` (P+, local-only 가치 깨므로 신중).

---

## 기여 / 작업 흐름

이 repo 는 단일 사용자 프로젝트지만 spec 변경 절차는 명문화되어 있다.

1. **frozen design 변경** — `docs/superpowers/specs/2026-04-27-kb-final-form-design.md` 가 단일 contract. 변경 시 영향 받는 component task 모두 동시 갱신 필요. PR 1개로 묶기.
2. **새 component task 추가** — `tasks/_template.md` 복사 후 `tasks/p<phase>/p<phase>-<n>-<name>.md` 생성. `contract_sections` 에 design doc 섹션 명시. `Allowed/Forbidden dependencies` 는 design §8 module-boundary 표 따름.
3. **구현** — component task 1개당 sub-agent 1세션 권장. `cargo test -p <crate>` + DoD 체크리스트 통과. PR 으로 머지.
4. **버전 변경** — `parser_version` / `chunker_version` / `embedding_version` 등 변경은 design §9 의 cascade rule 따름. 영향 받는 record 는 재처리 필요.

---

## 라이선스

미정 (frozen design 에는 `MIT OR Apache-2.0` 가 workspace.package 의 license 필드로 권장됨; P0 lock 시 결정).

---

## 참고

- 최초 설계 보고서: [kb_local_rust_report.md](kb_local_rust_report.md)
- Frozen design: [docs/superpowers/specs/2026-04-27-kb-final-form-design.md](docs/superpowers/specs/2026-04-27-kb-final-form-design.md)
- Task 분해 plan: [docs/superpowers/plans/2026-04-27-task-decomposition.md](docs/superpowers/plans/2026-04-27-task-decomposition.md)
- Task 인덱스: [tasks/INDEX.md](tasks/INDEX.md)
