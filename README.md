# kebab — Local-first Knowledge Base

`kebab` 는 개인용 로컬 knowledge base + RAG 도구다. Markdown / PDF / 이미지를 한 곳에 색인하고, 의미 검색 + page-단위 citation 포함 LLM 답변을 단일 binary 로 제공한다. 모든 추론은 로컬 (Ollama / fastembed) 에서 돌아간다. 대상 하드웨어: M4 48GB MacBook 1대, 사용자 1명.

## 사전 요구

- **Rust toolchain** ≥ 1.85 (workspace 가 edition 2024 + resolver 3 사용). [rustup](https://rustup.rs) 권장.
- **Ollama** — `kebab ask` 와 이미지 OCR/caption 가 사용. `https://ollama.com/download` 에서 설치 후 `ollama serve` 실행. 기본 LLM 은 gemma4 계열 (`ollama pull gemma4:e4b`) — OCR / caption 도 같은 family 라 모델 하나만 pull 하면 됨. 더 큰 variant 원하면 `gemma4:26b` 등으로 config override. config 의 `[models.llm].endpoint` 에 host:port 명시.
- **빌드 디스크** — 첫 빌드 시 `target/` 가 6–10 GB (Lance + DataFusion + fastembed). 여유 확인.
- **fastembed 모델** — 첫 `kebab ingest` 시 `multilingual-e5-small` (~470 MB) 자동 다운로드.

## 설치

표준 경로는 `cargo install` — `~/.cargo/bin/kebab` 가 PATH 에 있는지만 확인하면 끝.

```bash
# 1) repo clone
git clone https://gitea.altair823.xyz/altair823-org/kebab.git
cd kebab

# 2) binary 빌드 + 설치 (~/.cargo/bin/kebab)
cargo install --path crates/kebab-cli --locked

# 3) PATH 확인 (아직 추가 안 했으면 ~/.bashrc / ~/.zshrc 에 추가)
which kebab          # → /Users/<you>/.cargo/bin/kebab 같은 경로
kebab --version      # → kebab 0.1.0
```

git URL 직접 install 도 가능 (clone 없이):

```bash
cargo install --git https://gitea.altair823.xyz/altair823-org/kebab.git --bin kebab --locked
```

업데이트는 `git pull && cargo install --path crates/kebab-cli --locked --force` 또는 git URL 형식의 경우 `cargo install --git ... --force`.

제거는 `cargo uninstall kebab-cli`. 이 명령은 binary 만 지우고 워크스페이스 데이터는 그대로 남는다. 데이터까지 정리하려면 `kebab reset --all --yes` (config + data + cache + state 4 개 XDG 경로 모두 wipe — **irreversible**, 재시작 시 `kebab init` 다시 실행). 부분 wipe 는 `kebab reset --data-only` (config 보존), `kebab reset --vector-only` (Lance + `embedding_records` 만, 다음 ingest 가 re-embed) 등.

## Quick start

```bash
# 첫 실행 — XDG 경로에 데이터 디렉토리 + config.toml 생성
kebab init

# config 손보고 — `[workspace] include` 에 *.md / *.png / *.pdf 등 추가, 모델 endpoint 등
${EDITOR:-vi} ~/.config/kebab/config.toml

# 색인 (Markdown / 이미지 / PDF 모두 한 번에)
kebab ingest

# 검색 (citation 의 source_span 이 매체별로 line / region / page)
kebab search "Markdown chunking 규칙" --mode hybrid

# 질문 (Ollama 필요, PDF 인용 시 page 번호 surface)
kebab ask "내 KB 설계에서 저장소 전략은?"

# Ratatui 셸 (Library + Search + Ask + Inspect 패널, desktop 진행 중)
kebab tui

# 헬스 체크 (config 경로 / 데이터 디렉토리 쓰기 가능 여부)
kebab doctor
```

격리된 임시 워크스페이스로 돌려보는 절차는 [docs/SMOKE.md](docs/SMOKE.md) — `--config <path>` 로 분리. 이미지 / PDF fixture 가 필요하면 두 example 바이너리 (`cargo run --release --example gen_smoke_pdf -p kebab-parse-pdf` / `gen_smoke_png -p kebab-parse-image`) 로 시스템 dep 없이 in-tree 생성 가능.

설치 없이 dev 흐름으로 돌려볼 때는 `cargo run --release -p kebab-cli -- <subcommand>` 또는 `cargo build --release && ./target/release/kebab <subcommand>`.

## 명령

| 명령 | 동작 |
|------|------|
| `kebab init` | XDG 경로에 데이터 디렉토리 + config.toml 생성 |
| `kebab ingest [<path>]` | Markdown / 이미지 / PDF 색인 (idempotent) |
| `kebab search --mode {lexical,vector,hybrid} "<query>"` | 검색. hybrid는 RRF fusion, citation 포함 |
| `kebab list docs` | 색인된 문서 목록 |
| `kebab inspect doc <id>` / `kebab inspect chunk <id>` | raw record 보기 |
| `kebab ask "<query>"` | RAG 답변 + 근거 인용. 근거 부족 시 거절. Ollama 필요 |
| `kebab doctor` | 설정/모델/DB 헬스 체크 |
| `kebab tui` | Ratatui 셸 (Library + Search + Ask + Inspect 패널, desktop 진행 중) |
| `kebab reset [--all / --data-only / --vector-only / --config-only] [--yes]` | XDG 데이터 wipe. **Irreversible.** TTY 면 confirm prompt, 아니면 `--yes` 필수. `--vector-only` 는 SQLite `embedding_records` 도 함께 truncate (orphan 방지) |
| `kebab eval run / compare` | golden query 회귀 측정 |

모든 명령에 `--json` 플래그. 출력은 frozen wire schema v1 (`schema_version` 항상 포함, 예: `ingest_report.v1`, `search_hit.v1`, `answer.v1`, `doctor.v1`).

## 논리 아키텍처

```mermaid
flowchart TB
    user(["사용자"])

    subgraph UI["UI binary"]
        cli["kebab CLI"]
        tui["kebab TUI"]
    end

    subgraph App["Facade"]
        app["kebab-app"]
    end

    subgraph Pipeline["도메인 + 파이프라인"]
        parse["parse-md / parse-pdf / parse-image"]
        chunker["chunker (md-heading-v1, pdf-page-v1)"]
        embedder["embedder (fastembed multilingual-e5-small)"]
        retriever["retriever (lexical / vector / hybrid RRF)"]
        rag["RAG pipeline"]
    end

    subgraph Store["저장소"]
        sqlite[("SQLite + FTS5")]
        lance[("LanceDB")]
        assets[("asset bytes")]
    end

    subgraph External["외부"]
        fs[("workspace files")]
        ollama[("Ollama HTTP")]
    end

    user --> cli
    user --> tui
    cli --> app
    tui --> app

    app --> parse
    app --> chunker
    app --> embedder
    app --> retriever
    app --> rag

    fs --> parse
    parse -. vision OCR / caption .-> ollama
    parse --> sqlite
    parse --> assets

    chunker --> sqlite
    embedder --> lance
    retriever --> sqlite
    retriever --> lance

    rag --> retriever
    rag --> ollama
```

`kebab-app` 가 facade — UI binary 가 store / parse / search / llm / rag 를 직접 참조하지 않는다 (frozen 설계 §8). 자세한 crate-level 의존성 + 디렉토리 + 핵심 기술 결정은 [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Configuration

- `~/.config/kebab/config.toml` — `kebab init` 가 XDG 경로에 생성. `[workspace] include`, `[storage]`, `[chunking]`, `[models.embedding]`, `[models.llm]`, `[image.ocr]`, `[image.caption]`, `[search]`, `[rag]` 절.
- `--config <path>` flag — 임시 워크스페이스 / 격리 테스트 시 사용. CLI / TUI 모두 honor.
- `KEBAB_*` env — 일부 키 override (`KEBAB_RAG_SCORE_GATE`, `KEBAB_EVAL_GOLDEN`, `KEBAB_COMMIT_HASH` 등).
- XDG layout: `~/.config/kebab/`, `~/.local/share/kebab/`, `~/.cache/kebab/`, `~/.local/state/kebab/`.

config 예시는 [docs/SMOKE.md](docs/SMOKE.md) 의 `/tmp/kebab-smoke/config.toml` 블록 참조.

## 외부 AI 통합

`--json` 출력 + frozen wire schema v1 가 stable contract. 통합 옵션:

- **Claude Code / Codex skill** — `kebab search --json` / `kebab ask --json` 호출하는 ~50줄 wrapper.
- **MCP server** — stdio JSON-RPC 로 `kebab-app` facade 1:1 노출.
- **HTTP wrapper** — `kebab serve --bind 127.0.0.1:7711` (P+, local-only 가치 신중).

## 비-목표

다중 사용자 SaaS / K8s / 원격 vector DB / enterprise RBAC / 실시간 협업 / 모든 파일 포맷의 완벽한 parsing / agent 임의 파일 수정 / multi-workspace / LLM-as-judge eval / CLIP 시각 embedding / `kebab://` protocol handler — frozen 설계 §11 / §0 참조.

## 라이선스

`MIT OR Apache-2.0` (workspace `Cargo.toml` 의 `license` 필드).

## 참고

- 진척도: [HANDOFF.md](HANDOFF.md)
- 아키텍처: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
- Frozen 설계: [docs/superpowers/specs/2026-04-27-kebab-final-form-design.md](docs/superpowers/specs/2026-04-27-kebab-final-form-design.md)
- Task 인덱스: [tasks/INDEX.md](tasks/INDEX.md)
- 머지 후 hotfix 로그: [tasks/HOTFIXES.md](tasks/HOTFIXES.md)
- Smoke 절차: [docs/SMOKE.md](docs/SMOKE.md)
