---
phase: P9
component: tui + cli + app
task_id: p9-dogfooding
title: "도그푸딩 피드백 — UX 개선 잡음 수집"
status: open
depends_on: [p9-1, p9-2, p9-3, p9-4]
unblocks: []
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§7, §10]
---

# p9-dogfooding — 도그푸딩 피드백

P9-1 ~ P9-4 머지 + `cargo install --path crates/kebab-cli --locked` 후 사용자가 직접 `~/KnowledgeBase` 에서 ingest / search / ask / inspect 돌려보며 발견한 UX 잡음.

각 항목은 후속 task spec 으로 분리될 수 있는 단위로 정리. 머지 전 frozen 설계 §10 (UX) 와 충돌 시 spec 갱신 필요한지 표기.

## 발견 일자

2026-05-02 — PR #47 (gemma4 default + tilde fix) 머지 직후. 환경: Ubuntu, kebab 0.1.0, ratatui 0.28, gemma4:e4b on remote Ollama (192.168.0.47).

## 항목

### 1. 장시간 작업 진행 표시 (CLI / TUI / Desktop 공통)

ingest / embed / vector index build / RAG (긴 답변 streaming) 같이 초 단위 이상 걸리는 모든 작업에서 현재 진행 상황을 사용자에게 보여줘야 함.

**증상**: 사용자가 `kebab ingest` 돌렸는데 1.8 초 동안 "묵묵부답" 후 `scanned 0 new 0 ...` 만 표시. 사용자는 스캔 0 건 vs hung process 구분 불가.

**요구**:
- CLI: indeterminate spinner + "scanning <N> files / parsing <path> / embedding chunk <i>/<n>" 식 단계 표시. `--json` 모드면 line-delimited progress event (`schema_version=ingest_progress.v1`).
- TUI: Library / Status bar 에 progress 라인. 현재 ingest 는 background 가 아니라 blocking — 우선 background runner 도입 필요.
- Desktop (P9-5): 동일.

**spec 영향**: 설계 §7 ingest pipeline + §10 UX 에 progress event 명시 필요. wire schema `ingest_progress.v1` 추가.

### 2. 장시간 작업 즉시 중단 (Ctrl-C / Esc)

ingest / embed / RAG streaming 모두 사용자가 언제든 중단 가능해야 함. resume 가능하면 좋고, 불가능하면 처음부터 다시 라도 OK — 핵심은 **즉시 응답**.

**증상**: 현재 `kebab ingest` 는 SIGINT 받으면 단순 종료 — 마지막 commit 까지의 SQLite row 는 살아있어 다음 ingest 가 idempotent 하게 동작하긴 함. 다만 진행 중 chunk 임베딩이 long-running 이면 Ctrl-C 응답이 느림.

**요구**:
- Cooperative cancellation token (`std::sync::atomic::AtomicBool` 또는 `tokio_util::sync::CancellationToken`) 을 ingest 루프 내부 step boundary 에 check.
- 부분 진행 commit-as-you-go 는 현재 동작 유지 (SQLite per-asset transaction).
- TUI: Esc / Ctrl-C 두 키 모두 cancel 신호 보내고 worker thread 가 step boundary 에 도달하면 종료.

**spec 영향**: §7 의 ingest 결정성 절은 유지 — partial 결과는 valid 한 prefix.

### 3. workspace.root 상대 경로 + init placeholder 명확화

`config.toml` 의 `workspace.root` 가 상대경로면 어디 기준인지 모호. 절대경로만 강제하거나, 기준 디렉토리 명시.

**증상**: 사용자가 `~/KnowledgeBase` (tilde) 로 두고 ingest 했을 때 expand_tilde 가 적용된 후 정상 동작했지만, 이전에는 절대경로로 변경해야만 동작했음 (PR #47 fix 전). 도그푸딩 사용자는 처음에 무엇이 root 인지 헷갈렸음.

**요구**:
- 가능하면 상대경로도 허용 — base 는 `current_working_dir` 또는 `config 파일 dir` 중 무엇? 후자 선호 (config 따라다님).
- 불가능하면 `kebab init` 가 생성하는 placeholder 를 절대경로로 (예: `/home/<user>/KnowledgeBase` 가 아니라 `~/KnowledgeBase` 권장 — tilde 는 expand 됨).
- README 의 Quick start 에서 `${EDITOR} ~/.config/kebab/config.toml` 후 해야 할 일 절대경로로 안내. 현재는 "absolute path 로 변경" 만 막연.
- config.toml 코멘트 한 줄 추가: `# absolute path or ~/...; relative path is currently NOT supported`.

**spec 영향**: §6.2 (workspace 정의) 에 명시 추가.

### 4. 전체 데이터 삭제 명령

사용자가 여러 config / 모델 조합 테스트 중 — `kebab` 데이터를 깔끔히 초기화하는 단일 명령 필요.

**증상**: 현재 `cargo uninstall kebab-cli` 만 하고 데이터는 수동 `rm -rf ~/.local/share/kebab ~/.config/kebab ~/.cache/kebab ~/.local/state/kebab`. 4 개 경로 외우기 부담.

**요구**:
- `kebab nuke` (또는 `kebab reset --all`) — XDG 경로 4 개 + binary 는 보존, 데이터만 wipe. 첫 실행 confirm prompt + `--yes` flag.
- `kebab reset --data-only` (sqlite + lance 만, config 보존), `kebab reset --vector-only` 등 부분 wipe variant.
- `--config <path>` 도 honor — isolated workspace wipe.

**spec 영향**: §10 UX 에 reset/nuke 명령 추가.

### 5. inspect 화면의 title 공백 (도그푸딩 corpus)

`~/KnowledgeBase/testdata/coding-md-corpus/python/python-360-annotation-issues-at-runtime.md` 같은 파일들 ingest 후 `kebab tui` Library + Inspect 에서 모든 doc 의 title 이 공백.

**증상**: title 추출 로직이 frontmatter `title:` 또는 첫 H1 (`# Title`) 둘 중 하나에 의존. corpus md 가 frontmatter 없고 H1 없이 H2 부터 시작하면 title=빈 문자열.

**요구**:
- title fallback 우선순위 명시: (1) frontmatter `title` → (2) 첫 H1 → (3) 첫 H2 → (4) 첫 non-empty paragraph 의 첫 N 자 → (5) 파일명 (확장자 제외).
- 현재 로직 `kebab-parse-md/src/...` 확인 후 수정. parser_version cascade — bump 필요.

**spec 영향**: §3.5 / §5.5 normalize 에 title fallback 명시. parser_version `md-frontmatter-v1` → `md-frontmatter-v2`.

### 6. search 의 keystroke-by-keystroke 지연

TUI search pane 에서 한 글자 칠 때마다 검색 호출 → SQLite FTS + Lance vector + RRF 모두 hot path 라 체감 지연.

**증상**: 사용자가 문장 칠 때 키 입력 직후 frame 잠김. backspace 도 마찬가지.

**요구**:
- 옵션 A: debounce — 마지막 keystroke 후 N ms (예: 250ms) 무입력이면 검색 trigger.
- 옵션 B: Enter 만 trigger — 단순. 사용자도 "OK" 라고 말함.
- 옵션 A + B 결합: debounce default + Enter 즉시 trigger override.

**구현**: search worker thread 에 `latest_query: Mutex<String>` + `debounce_at: Instant` — main loop tick 마다 check. 또는 mpsc channel 로 `Query(s, generation)` 보내고 worker 가 `generation` 비교해 stale 결과 drop.

**spec 영향**: 없음 — 구현 detail.

### 7. search → editor (`g` 키) → `:q` 복귀 후 화면 깨짐

search pane 에서 `g` 로 vim/code 띄우고 종료 후 TUI 화면이 일부만 redraw. 입력 한 글자가 바뀐 부분만 다시 그려지고 나머지는 invisible.

**증상**: external editor exit 후 ratatui 의 alternate screen + mouse capture restore 누락. crossterm 의 `disable_raw_mode` / `LeaveAlternateScreen` 을 editor spawn 시 해주고 복귀 후 `EnterAlternateScreen` + `enable_raw_mode` 다시 호출 필요.

**요구**:
- `kebab-tui::search::open_in_editor` 직전 `Terminal::leave_alternate_screen` + `disable_raw_mode`, child 종료 후 `Terminal::clear()` + `enter_alternate_screen` + `enable_raw_mode` + 다음 frame 강제 redraw (`force_redraw: bool` flag).
- 이 패턴은 inspect / library 의 file open 도 동일 — 공통 helper.

**spec 영향**: 없음 — 구현 detail.

### 8. 한글 입력 (IME / multi-byte)

전반적으로 한글로 검색 / 질문 칠 텐데 한글 입력이 의도한 곳과 다른 곳에 들어감.

**증상**: 추측 — IME composing 중간 글자가 search input 이 아닌 다른 키 (e/j/k 등 single-char command) 로 라우팅되거나, multi-byte buffer 가 잘려 wide-char 깨짐.

**요구**:
- 한글 (CJK) IME 입력 흐름 정리. crossterm 은 IME composing event 를 native 로 surface 안 함 — `KeyCode::Char(c)` 로 자모 단위 도착. 입력 mode 일 때는 모든 `Char` 가 buffer push, command mode 가 따로 있어야 e/j/k/g 같은 키가 의미를 가짐 (10번 항목과 묶음).
- 한글 wide-char rendering: 현재 `unicode-width::UnicodeWidthStr` 사용 중인지 확인. width 계산 누락이면 cursor 위치 어긋남.
- 테스트 fixture: 한글 query (`러스트 비동기`), 한글 답변 streaming, 한글 파일명 (`테스트.md`).

**spec 영향**: 없음 — UX detail.

### 9. ask 답변의 markdown 렌더링

LLM 답변이 markdown (bold, italic, table, code block, list) 일 때 raw `**bold**` / `| col |` 그대로 출력. ratatui 에 markdown 렌더링 없음.

**요구**:
- ratatui 의 `Span` / `Line` styling 으로 inline 변환: bold (`**text**`) → `Style::default().add_modifier(Modifier::BOLD)`, italic (`*text*` / `_text_`) → `ITALIC`, inline code (`` `code` ``) → `Style::default().bg(Color::DarkGray)`.
- block 단위: heading (#) → 큰 fg color, list bullet, code fence (```) → 박스 + monospace, table → ratatui `Table` widget.
- 파서: `pulldown-cmark` 이미 workspace 에 있음 — 재사용.
- 답변 streaming 중에는 incremental render — 마지막 incomplete inline span 만 raw 로 출력하고 complete span 부터 styled.

**spec 영향**: §7 의 RAG output 절에 markdown rendering hint.

### 10. 입력 mode vs command mode (search / ask)

search / ask 의 query input box 에 모든 키가 입력으로 들어가 e=explain 같은 single-char command 사용 불가. 현재 P9-3 ask 가 input-empty 일 때만 e/j/k 를 command 로 인식 (heuristic) — 사용자는 명시적 mode 선호.

**요구**:
- vim 식 `i` (insert) / `Esc` (normal/command) — TUI 전반에 통일. status bar 에 현재 mode 표시 (`-- INSERT --` / `-- NORMAL --`).
- 또는 input box 가 focused 가 아닐 때 keys 가 command 로 (Tab/Shift-Tab 으로 focus 이동). focus indicator (테두리 색상) 명확히.
- 첫 옵션 선호 — vim 친숙도 + 모드 전환이 명시적.

**spec 영향**: §10 UX 에 mode 모델 명시 필요.

### 11. 조작법 친절 안내 (vim 비익숙 사용자)

현재 TUI 하단 hint 라인이 단순 키 나열. vim 비익숙 사용자에게는 의미 불투명.

**요구**:
- 각 pane 마다 키 매핑 cheatsheet — `?` 누르면 modal popup 으로 전체 키 목록 + 짧은 설명.
- hint line 자체도 명사구가 아니라 동사구 ("k=up" 보다 "↑/k 위로 이동").
- mode 별 hint 분기 (10번과 결합): NORMAL 은 navigation 키, INSERT 는 "Esc 로 normal" 만.
- README 에도 TUI 키 cheatsheet 표 추가.

**spec 영향**: §10 UX 추가.

### 12. TUI 색상 팔레트 확장

현재 TUI 가 거의 monochrome — 테두리 / 강조 1~2 색 정도. 사용자가 "더 많은 색깔의 글씨" 원함.

**증상**: Library / Search / Ask / Inspect 모든 pane 이 default fg + 1~2 accent. 정보 종류 (title vs path vs score vs warning vs streaming token) 가 색으로 구분 안 됨.

**요구**:
- 의미 단위 color role 정의 (단순 색 채우기 X — 의미 매핑):
  - `title` (Doc / Section heading) → Cyan / bright
  - `path` (workspace_path) → DarkGray / Dim
  - `score` (search hit RRF / relevance) → Green (high) → Yellow → Red (low) gradient
  - `mode` (lexical / vector / hybrid) → 각각 다른 hue
  - `warning` / `error` → Yellow / Red bg
  - `streaming` token (ask 답변 새로 도착한 부분) → 옅은 Blue background, 1초 후 default 로 fade
  - `citation` source span → underline + Magenta
  - `keyword highlight` (search query 매치 부분) → Yellow bg
- ratatui `Style` + `Color::Indexed`/`Color::Rgb` 활용. 256-color terminal 가정.
- color theme module — 한 곳 (`kebab-tui::theme`) 에 role → Style 매핑. light/dark theme 토글 (`theme = "dark"` config 또는 `T` 키 cycle).
- accessibility — color 단독으로 의미 전달 X (color blind 고려). 색상은 보조, primary 정보는 텍스트 / 위치.

**spec 영향**: §10 UX 에 color role 명시 권장. 다만 구현 detail 에 가까움.

### 13. ask 멀티턴 (대화 history 컨텍스트 + scrollback UI)

현재 ask 는 질문 1 회 → 답변 1 회 단발. 사용자는 "꼬리 물기" 자연스러움 — 이전 질답을 컨텍스트로 LLM 에 같이 전달 + UI 도 history 보이는 conversation view 로 변경.

**증상**: "X 가 뭐야?" 답변 후 "그럼 Y 와는 어떤 차이?" 물으면 LLM 은 X 를 모름. retrieval 로 일부 회복되긴 하지만 사용자 의도 (= "방금 답한 X 와 비교") 는 사라짐.

**요구**:
- **컨텍스트 전달**: ask 세션 내부에 `Vec<Turn>` (`Turn { question, answer, citations, ts }`) 유지. LLM prompt 빌드 시 system + (history N turns) + retrieved chunks + 새 question. token budget (`rag.max_context_tokens`) 안에 fit — history 가 budget 침범하면 retrieval k 줄이거나 oldest turn drop. 정책 명시 필요.
- **retrieval 강화**: 새 question 단독 검색 X — 직전 answer + question 합쳐 query expansion (간단: concat. 고급: LLM 으로 standalone question rewriting). 우선 concat 으로 시작.
- **UI**:
  - ask pane 을 conversation 형태로 — 위 scrollable transcript (Q1 / A1 / citations / Q2 / A2 / ...) + 아래 input box.
  - Q 와 A 시각 구분 (12번 color role 활용 — Q=Cyan bold, A=default).
  - citation 은 turn 별로 fold (`▸ 근거 N 건`). 펼침 키.
  - PageUp/PageDown / k/j 로 scroll. 최신 답변 도착 시 자동 bottom.
- **세션 영속**: P+ 옵션 — 종료 시 SQLite `chat_sessions` / `chat_turns` 테이블에 저장. 다음 실행 시 "이전 대화 이어가기" 또는 "새 대화" 선택. 우선 in-memory 만.
- **세션 reset**: `Ctrl-L` 또는 `:new` 로 history clear (현재 ask pane 리셋 동등).
- **mode 전환** (10번과 결합): conversation scrollback 보면서 NORMAL 키 (j/k/PageUp) 동작, INSERT 진입 시 input box 만 영향.

**spec 영향**: §7 RAG 절에 multi-turn / conversation history 정책 추가. wire schema `answer.v1` 에 `conversation_id` / `turn_index` 옵션 필드. SQLite 새 테이블 (`chat_sessions`, `chat_turns`) — migration `V004`. token budget 정책 명시.

**구현 사이즈**: 큼. `kebab-rag` (history-aware prompt 빌드) + `kebab-store-sqlite` (V004 migration, 영속화하면) + `kebab-tui::ask` (conversation UI 전면 재작성) + `kebab-app` facade (`ask_with_session(...)`) + wire schema. 단발 기능 제거 X — `kebab ask` CLI 한 turn 모드는 유지, TUI 만 multi-turn (CLI multi-turn 은 14 번 참조).

### 14. ask CLI multi-turn (옵션)

13 번 conversation history 가 TUI 만이 아닌 CLI 도 동작하면 유용. 사용자가 명시적으로 enable.

**증상**: 현재 `kebab ask "Q"` 는 항상 1 회. shell history 로 재호출해도 LLM 은 직전 답변 모름.

**요구**:
- **옵션 A — 세션 ID 명시**: `kebab ask --session <id> "Q"`. 같은 `<id>` 로 호출 시 SQLite `chat_sessions` 에 누적. ID 미지정이면 단발 (현재 동작 유지). 13 번의 SQLite V004 와 공유.
- **옵션 B — REPL 모드**: `kebab ask --repl` 실행 시 stdin 로 prompt → answer 반복. Ctrl-D / `:q` 종료. session 은 in-memory (영속 원하면 `--session <id>` 결합).
- **옵션 C — auto-session**: 같은 TTY 에서 N 분 내 연속 호출이면 자동 묶음 (ad-hoc session). PID + tty + ts hash. 우선순위 낮음 — magic 하고 디버깅 어려움.
- 권장: A + B 조합. C 는 P+.
- `--json` 모드 호환: `answer.v1` 에 `conversation_id` / `turn_index` 필드 (13 번에서 추가) — 외부 도구가 session 추적 가능.
- **외부 AI 통합 효과** (README 의 외부 AI 섹션): Claude Code skill / MCP server 도 `--session` 으로 conversation context 보존. 이 부분이 multi-turn CLI 의 진짜 가치 — 내장 TUI 만 쓰는 사용자보다 외부 wrapper 사용자가 큼.

**spec 영향**: §7 RAG 절 multi-turn 정책 + §externalAI 통합 절 (READE 와 ARCHITECTURE 동기화) 에 session 모델 추가. CLI flag 표 (`--session` / `--repl`) README 갱신.

### 15. search 결과 캐싱 (incremental invalidation)

같은 query 반복 시 매번 SQLite FTS + Lance vector + RRF 재계산. cache 가능.

**증상**: 사용자가 search pane 에서 같은 query 로 이동하거나 (Library → Search → Library → Search), TUI 다른 pane 갔다 다시 와도 재검색. CLI 도 마찬가지.

**요구**:
- **cache key**: `(query_normalized, mode, k, snippet_chars, embedding_version, chunker_version, prompt_template_version=N/A)` tuple → `Vec<SearchHit>`.
  - query 정규화: trim + NFKC + lowercase. 공백 / 대소문자 차이가 hit 동일하면 같은 entry.
- **cache 위치**:
  - 옵션 A: in-memory LRU (TUI session 단위) — 단순, 빠름. TUI 가 session 시작 시 빈 cache.
  - 옵션 B: SQLite `search_cache` 테이블 — process 간 공유, 영속. SELECT 기반이라 sub-ms.
  - 옵션 A 우선 — 단순하고 도그푸딩 막힘 해결. B 는 P+ (CLI 호출 빈도 높을 때).
- **invalidation (incremental — 핵심 요청 부분)**: ingest 가 doc 추가/삭제하면 기존 cache entry 도 영향 받음. 두 전략:
  - 전략 1 — **bump generation counter**: `index_version: u64` 단조 증가. ingest 가 1 chunk 라도 변경하면 +1. cache key 에 `index_version` 포함 → 모든 stale entry 자동 무효. 단순, 모든 cache hit 무효화 — 캐시 가치 적음.
  - 전략 2 — **dirty doc set**: ingest 가 변경한 `Vec<doc_id>` 기록. cache lookup 시 entry 의 hits 가 dirty doc 포함하면 stale 처리. cache entry 보존율 ↑, 복잡도 ↑.
  - 전략 3 — **patch-and-merge** (사용자가 말한 "추가되는 내용만 끼워넣음"): cache entry 보관, ingest 의 새 chunks 만 별도 검색 → 기존 결과와 RRF 재합성. lexical (FTS) 는 새 doc 만 매치 검사 + score 정규화 재계산. vector 는 새 doc 의 embedding 만 query vector 와 cos sim 계산. 정확하지만 RRF normalization (post-merge hotfix `2/(k+1)` 정규화 — frozen design 표) 가 전체 hit set 기준 재계산이라 incremental 어려움.
  - 권장: 1 → 3 단계. 우선 1 (단순) 도입, 측정 후 3 도입 결정. 2 는 중간 단계라 skip.
- **TTL**: in-memory cache 는 process 수명. SQLite 영속 시 `created_at` + 1 일 TTL 또는 ingest 시 wipe.
- **CLI**: `kebab search --no-cache` 로 강제 bypass. 디버깅용.

**spec 영향**: cache 자체는 구현 detail. 다만 `index_version` (전략 1) 는 design §9 의 versioning cascade 에 새 차원 — 명시 필요. 사용자 주의: cache miss/hit 동작이 다르면 외부 도구 (skill/MCP) 가 timing 의존하면 안 됨 — 결과 자체는 동일 보장.

### 16. ask 답변의 citation 풀 경로 보기 + scroll

ask 답변 끝에 citation list 가 나오는데 path 가 truncated 또는 한 줄에 몰림. 사용자가 풀 경로 확인하려 함.

**증상**: 현재 ask 출력 (CLI 와 TUI 둘 다) citation 이 inline `[1] notes/foo.md#L12-L34` 형태로 narrow terminal 에서 잘림. TUI ask pane 은 답변 본문 + citation 같은 buffer — 별도 scroll 영역 없음.

**요구**:
- **CLI**:
  - 답변 후 citation 절 — path 한 줄씩, full path (workspace_path + fragment). truncate 안 함.
  - `--show-citations / --hide-citations` flag (default: show). `--json` 은 항상 포함.
  - 형식: `[1] crates/kebab-app/src/lib.rs#L120-L140  (score=0.78, doc_id=abc123)`.
- **TUI ask pane**:
  - layout 분리 — 위 conversation transcript (13 번), 아래 citation pane (toggle-able). citation pane 은 turn 별 인용 list, 각 항목 (`[1] full/path/here.md`, line range, score).
  - citation pane 자체 scroll (`j/k` 또는 PageUp/PageDown).
  - 항목 선택 + Enter 또는 `o` → P9-2 search 의 editor jump 와 동일 동작 (vim/code 로 path 열기 — line range 점프).
  - 항목 선택 + `i` → P9-4 inspect 로 (Doc / Chunk).
- **layout 모드 토글**: `:cite` 또는 `c` 키로 citation pane fold/expand. 기본은 expanded.

**spec 영향**: §10 UX 의 ask 절에 citation surface 명시. wire schema `answer.v1` 의 `citations` 배열은 이미 존재 — UI 가 활용 못 했을 뿐.

## 후속 작업 분리 (분해 결과)

20 개 task spec 으로 분해 완료. 각 spec 은 single-PR 단위. 의존 관계는 frontmatter `depends_on` / `unblocks` 참조.

| task | 제목 | 의존 | feedback 항목 |
|------|------|------|---------------|
| [p9-fb-01](p9-fb-01-ingest-progress-callback.md) | Ingest progress callback | – | 1 (backend) |
| [p9-fb-02](p9-fb-02-cli-progress-display.md) | CLI progress display | 01 | 1 (CLI) |
| [p9-fb-03](p9-fb-03-tui-ingest-background.md) | TUI ingest background + status | 01 | 1 (TUI) |
| [p9-fb-04](p9-fb-04-ingest-cancellation.md) | Cooperative cancellation | 01 | 2 |
| [p9-fb-05](p9-fb-05-config-path-policy.md) | workspace.root path policy | – | 3 |
| [p9-fb-06](p9-fb-06-data-reset-command.md) | `kebab reset` 명령 | – | 4 |
| [p9-fb-07](p9-fb-07-md-title-fallback.md) | MD title fallback chain | – | 5 |
| [p9-fb-08](p9-fb-08-search-debounce.md) | Search debounce + Enter | – | 6 |
| [p9-fb-09](p9-fb-09-tui-editor-restore.md) | External editor return restore | – | 7 |
| [p9-fb-10](p9-fb-10-tui-cjk-input.md) | CJK input + wide-char | 12 | 8 |
| [p9-fb-11](p9-fb-11-ask-markdown-render.md) | Ask markdown render | 14 | 9 |
| [p9-fb-12](p9-fb-12-tui-mode-machine.md) | NORMAL / INSERT mode | – | 10 |
| [p9-fb-13](p9-fb-13-tui-cheatsheet.md) | Cheatsheet popup + keymap | 12 | 11 |
| [p9-fb-14](p9-fb-14-tui-color-theme.md) | Color theme module | – | 12 |
| [p9-fb-15](p9-fb-15-rag-multi-turn-core.md) | RAG history + token budget | – | 13 (core) |
| [p9-fb-16](p9-fb-16-tui-ask-conversation.md) | TUI ask conversation UI | 15, 12 | 13 (UI) |
| [p9-fb-17](p9-fb-17-chat-session-storage.md) | SQLite V004 chat sessions | 15 | 13, 14 |
| [p9-fb-18](p9-fb-18-cli-ask-session-repl.md) | CLI `--session` / `--repl` | 15, 17 | 14 |
| [p9-fb-19](p9-fb-19-search-cache.md) | Search result LRU cache | – | 15 |
| [p9-fb-20](p9-fb-20-citation-surface.md) | Citation full path + scroll | 09, 16 | 16 |

## 권장 실행 순서 (도그푸딩 막힘 강도)

1. **p9-fb-06** (reset) — 테스트 반복 시 막힘 가장 큼.
2. **p9-fb-01 / 02 / 03** (progress) — ingest hung vs empty 구분 못 함이 다음 큰 막힘.
3. **p9-fb-04** (cancel) — progress 와 같은 PR batch 가능.
4. **p9-fb-15 / 16** (multi-turn 핵심) — 사용자 의도 (꼬리 물기) 직결.
5. **p9-fb-20** (citation) — 출처 보기, 도그푸딩 후속 검증의 prerequisite.
6. **p9-fb-07** (title fallback) — Inspect 가 비어있는 문제. 작은 size.
7. **p9-fb-19** (search cache) — 5번 debounce 와 함께 응답성.
8. **p9-fb-08** (debounce) — 작은 size, 즉시 효과.
9. **p9-fb-09** (editor restore) — 작은 size.
10. **p9-fb-12 → 13** (mode → cheatsheet) — UX 일관성 묶음.
11. **p9-fb-14** (theme) — 12 와 같은 batch 가능, 11 prerequisite.
12. **p9-fb-11** (markdown render) — theme 위에 build.
13. **p9-fb-17** (V004 storage) — multi-turn 영속화 prerequisite for 18.
14. **p9-fb-18** (CLI session) — 외부 AI 통합 효과 큼.
15. **p9-fb-10** (CJK) — mode machine 후. 한글 사용자에게 큼.
16. **p9-fb-05** (path policy) — 절대 경로 우회 가능, 후순위.

## spec PR vs impl PR

frozen design §10 (UX) / §7 (RAG multi-turn) / §5 (storage chat tables) / §9 (versioning index_version 추가) 갱신 동반 task:

- p9-fb-01 (ingest_progress.v1 wire schema)
- p9-fb-06 (reset 명령)
- p9-fb-07 (parser_version cascade — 이미 §9 covered, 신규 spec 변경 없음)
- p9-fb-12 (mode machine)
- p9-fb-13 (cheatsheet)
- p9-fb-15 (RAG multi-turn 정책)
- p9-fb-17 (chat_sessions / chat_turns 테이블)
- p9-fb-18 (answer.v1 의 conversation_id / turn_index)
- p9-fb-19 (index_version cascade)

위 task 의 PR 직전 spec 갱신 필요. spec PR 한 번에 묶어 진행 권장 (frozen 설계 일관성).

## Risks / notes

- 항목 중 일부는 frozen design §10 UX 갱신 동반 (1, 4, 5, 10, 11). spec PR 먼저 → impl PR 분리.
- title fallback (5) 은 parser_version cascade — 기존 ingest 된 doc 재처리 필요. 사용자 데이터 wipe (4) 와 함께 진행하면 깔끔.
- 한글 + IME (8) 는 ratatui / crossterm 의 한계가 있을 수 있음 — 완전 해결 어려우면 `ja/ko/zh user 는 외부 editor 권장` fallback 안내.
