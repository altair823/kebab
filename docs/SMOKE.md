---
title: "kebab 스모크 실행 가이드"
date: 2026-05-01
---

# kebab 스모크 실행 가이드

P3-5 머지 후 (`kebab-app::ingest` / `search` / `list` / `inspect` 와이어링) 부터, 그리고 P4-3 머지 후 (`kebab ask` 와이어링) 부터 사용자가 자기 설치본을 직접 검증할 수 있다. 이 문서는 사용자 환경 (`~/.config/kebab/`, `~/.local/share/kebab/`) 을 건드리지 않고 임시 디렉토리에 격리된 KB 를 띄워 전체 파이프라인을 1세션 안에 한 번 돌리는 절차다.

## 준비

빌드:

```bash
cargo build --release -p kebab-cli   # debug 도 무방. 디버그가 더 빠르게 빌드됨.
```

원격 Ollama (선택, `kebab ask` 만 필요):

```bash
# Mac 등 별도 호스트에서
OLLAMA_HOST=0.0.0.0:11434 ollama serve
ollama pull gemma4:e4b           # 기본 default. 더 큰 variant 원하면 gemma4:26b
```

본 머신에서 reachability 검증:

```bash
curl http://<host>:11434/api/tags
```

`{"models": [...]}` 가 나오면 네트워크 + 방화벽 OK.

## 격리된 워크스페이스 생성

```bash
mkdir -p /tmp/kebab-smoke/{workspace,data}
cat > /tmp/kebab-smoke/workspace/intro.md <<'EOF'
---
title: 인사말
tags: [demo]
lang: ko
---
# 안녕

이 문서는 스모크 테스트 fixture 다.
EOF
```

여러 파일을 시드하고 싶으면 본인 KB 일부를 `cp -r` 으로 복사해도 좋다 (다음 절차는 6개 markdown 가정).

## 격리된 config

`/tmp/kebab-smoke/config.toml`:

```toml
schema_version = 1

[workspace]
root = "/tmp/kebab-smoke/workspace"
include = ["**/*.md"]
exclude = [".git/**", "node_modules/**", ".obsidian/**"]

[storage]
data_dir = "/tmp/kebab-smoke/data"
sqlite = "{data_dir}/kebab.sqlite"
vector_dir = "{data_dir}/lancedb"
asset_dir = "{data_dir}/assets"
artifact_dir = "{data_dir}/artifacts"
model_dir = "{data_dir}/models"
runs_dir = "{data_dir}/runs"
copy_threshold_mb = 100

[indexing]
max_parallel_extractors = 2
max_parallel_embeddings = 1
watch_filesystem = false

[chunking]
target_tokens = 500
overlap_tokens = 80
respect_markdown_headings = true
chunker_version = "md-heading-v1"

[models.embedding]
provider = "fastembed"               # "none" 으로 두면 lexical-only — Ollama 불필요
model = "multilingual-e5-small"
version = "v1"
dimensions = 384
batch_size = 64

[models.llm]
provider = "ollama"
model = "gemma4:26b"                 # 사용자 환경에 맞춰 교체
context_tokens = 16384
endpoint = "http://192.168.0.47:11434"
temperature = 0.2
seed = 42

[search]
default_k = 10
hybrid_fusion = "rrf"
rrf_k = 60
snippet_chars = 220
cache_capacity = 256                 # p9-fb-19 — in-process LRU cap; 0 disables, default 256
stale_threshold_days = 30            # p9-fb-32 — 0 = disable. Marks hits/citations whose source doc was last reindexed > N days ago.

[rag]
prompt_template_version = "rag-v1"
score_gate = 0.05                    # RRF 정규화 후 [0, 1] 범위라 default 그대로 OK
explain_default = false
max_context_tokens = 6000

[ui]
theme = "dark"                       # p9-fb-14 — TUI palette ("dark" / "light", default "dark")
```

`KEBAB_*` 환경변수로 override 가능 (`KEBAB_MODELS_LLM_MODEL=gemma4:26b kebab …` 등). 자세한 키 목록은 `crates/kebab-config/src/lib.rs` 의 `apply_env` 매치 암. `KEBAB_READONLY=1` — write-path 비활성화 (CI 안전망). `KEBAB_PROGRESS=plain` — non-TTY 환경에서 진행 상황을 plain 한 줄씩 stderr 출력 (spinner 대신).

## 명령 시퀀스

```bash
KEBAB() { ./target/debug/kebab --config /tmp/kebab-smoke/config.toml "$@"; }

KB doctor                                          # 1. health check
KB ingest                                          # 2. 워크스페이스 색인 (markdown + image)
KB list docs                                       # 3. 색인 결과 목록 (markdown + image 모두 표시)
KB search --mode lexical "코루틴" --k 3            # 4. lexical 검색
KB search --mode vector "memory safety" --k 3      # 5. vector 검색
KB search --mode hybrid "Cargo workspace" --k 3    # 6. hybrid 검색
KB search --mode lexical "Hello World" --k 3       # 7. image OCR 텍스트 검색 (P6-4)
KB inspect chunk <chunk_id>                        # 8. raw chunk 보기
KB ask "이 KB 안에서 ..." --mode hybrid --k 5     # 9. RAG 답변 (Ollama 필요)
KB --json ask "..." --mode hybrid                  # 10. 기계 친화 출력 검증
```

### Stale doc indicator

Each search hit and RAG citation carries `indexed_at` (RFC3339 of the doc's last
re-process) and `stale` (computed against `[search] stale_threshold_days`).
A 30-day default flags docs that haven't been touched in a month — the
intent is to nudge a reingest before relying on the snapshot. Set to `0`
to disable.

### Streaming ask (fb-33)

```bash
kebab ask "what is rust ownership" --stream 2> events.ndjson > final.json
```

stderr 의 events.ndjson 은 한 줄 = 한 event 의 ndjson — `retrieval_done` 한 번, `token` 여러 번, `final` 한 번 (refusal 경로는 `final` 생략). final.json 은 기존 `answer.v1` 그대로 (backwards-compat).

agent 가 stderr 를 닫으면 (`head -c 1` 등) pipeline 이 LLM stream 을 즉시 중단하고 `RefusalReason::LlmStreamAborted` 로 partial answer 를 `answers` 테이블에 기록.

### Pagination + budget (fb-34)

```bash
# First page
kebab search "rust" --json --k 5 > page1.json
jq '.next_cursor' page1.json

# Next page using the returned cursor
NEXT=$(jq -r '.next_cursor' page1.json)
kebab search "rust" --json --k 5 --cursor "$NEXT" > page2.json

# Budget cap — returns smaller snippet / fewer hits + truncated=true
kebab search "rust" --json --max-tokens 200 | jq '.truncated, (.hits | length)'
```

`next_cursor` 는 corpus_revision 변경 (이후 ingest 등) 시 invalid — 다음 호출이 `error.v1.code = stale_cursor` 로 거절. agent 는 새 search 로 재발급 받기.

`--json` 출력은 `search_response.v1` wrapper (`{hits, next_cursor, truncated}`) — pre-fb-34 의 bare `search_hit.v1[]` 배열과 호환 안 됨.

## P6-4 이미지 ingestion 옵션

`config.toml` 에 다음 절을 추가하면 `kebab ingest` 가 `**/*.png` / `**/*.jpg` 등 이미지 자산도 함께 색인합니다 (텍스트만 색인하려면 생략):

```toml
[workspace]
include = ["**/*.md", "**/*.png", "**/*.jpg"]

[image.ocr]
enabled = true                        # vision LM 으로 이미지 안 텍스트 전사
engine = "ollama-vision"
model = "gemma4:e4b"                  # 사용자 환경의 비전 모델
endpoint = "http://192.168.0.47:11434"  # 비우면 models.llm.endpoint fallback
languages = ["eng", "kor"]
max_pixels = 1600                     # long-edge cap

[image.caption]
enabled = true                        # vision LM 으로 한 문장 객관 설명 생성
max_pixels = 768
prompt_template_version = "caption-v1"
```

이미지 자산 한 장당 OCR 1 호출 + Caption 1 호출 → ~3-6초 (`gemma4:e4b` 기준). 다이어그램 / 카메라 사진 / 스크린샷 위주 워크스페이스에 권장. 책 / 스캔본은 P7 PDF 라인으로.

## P7-3 PDF ingestion

`config.toml` 의 `[workspace] include` 에 `**/*.pdf` 를 추가하면 `kebab ingest` 가 텍스트 PDF 자산도 색인합니다. 외부 service 의존 없음 — `kebab-parse-pdf` 가 lopdf 로 페이지 단위 텍스트 추출, `kebab-chunk::PdfPageV1Chunker` 가 페이지 경계를 절대 넘지 않는 chunk 생성.

```toml
[workspace]
include = ["**/*.md", "**/*.pdf"]
```

PDF 한 권당 페이지 수만큼 (또는 페이지 텍스트가 길면 그 이상의) chunk 가 한 transaction 안에서 commit. 검색 결과의 `chunk.source_spans[0]` 가 `Page { page, char_start, char_end }` 형태라 인용 시 페이지 번호가 그대로 사용 가능. `kebab ask --json` 의 `citations[].citation` 도 `kind: "page"` + `page: <N>` + `path: <pdf_path>` 로 노출.

테스트 fixture 가 필요할 때는 두 example 바이너리를 사용 — `reportlab` / `qpdf` 같은 시스템 dep 없이 in-tree 로 PDF / PNG 생성:

```bash
cargo run --release --example gen_smoke_pdf -p kebab-parse-pdf -- \
  /tmp/kebab-smoke/workspace/whitepaper.pdf "page one body" "page two body"

cargo run --release --example gen_smoke_png -p kebab-parse-image -- \
  /tmp/kebab-smoke/workspace/diagram.png
```

```bash
kebab --config /tmp/kebab-smoke/config.toml ingest
kebab --config /tmp/kebab-smoke/config.toml search --mode hybrid "<본문 단어>"
kebab --config /tmp/kebab-smoke/config.toml inspect doc "<pdf_doc_id>"
kebab --config /tmp/kebab-smoke/config.toml ask "<PDF 본문에 관한 질문>" --json
```

암호화 PDF (예: DRM 책) → `errors+=1`, `error` 필드에 `qpdf --decrypt` 안내. 빈/스캔 페이지 (텍스트 추출 실패) → 0 chunk + `Provenance::Warning` ("scanned candidate"). v1 에서는 검색 불가, P+ scanned-PDF OCR fallback 까지 대기.

수정된 PDF 를 같은 path 에 다시 배치하면 `purge_orphan_at_workspace_path` 가 옛 doc / chunks / embeddings 를 sweep 하고 새 byte 가 새 `doc_id` 로 색인됨 — `IngestReport` 에 그 자산만 `new+=1` 로 분류 (다른 자산은 `updated`). HOTFIXES `2026-05-02 P7-3` 참조.

각 명령은 0 종료 코드면 정상. `kebab ask` 는 거절 시 종료 코드 1 (`RefusalSignal`) — 의도된 동작.

## 검증 체크리스트

- `kebab doctor` 가 `--config` path 를 honor 하고 그 안의 `storage.data_dir` 를 출력 (XDG default 가 아님).
- `kebab ingest` idempotent — 두 번째 실행이 `new=0 updated=N`.
- `kebab list docs` 출력에 frontmatter 의 `title` 이 아닌 deterministic `doc_id` (32-hex) + `workspace_path` 가 보임.
- `kebab search --mode hybrid` 의 `fusion_score` 가 `[0, 1]` 범위 (top-1 종종 1.0 — 두 retriever 모두 rank 1 일 때).
- `kebab ask` JSON 응답에 `model.id` 가 config 의 모델 (`gemma4:26b` 등) 과 일치, `embedding.id = multilingual-e5-small`, `citations[].marker` 가 `[1]` / `[2]` 형식 (square-bracketed bare index).
- 코퍼스에 없는 주제로 `kebab ask` → `refusal_reason: "llm_self_judge"` (또는 `no_chunks` / `score_gate`) + `grounded: false`.
- (P6-4) `image.ocr.enabled = true` 로 PNG 자산을 ingest 하면 `kebab list docs` 가 markdown 옆에 image doc 도 출력 (`workspace_path` 가 `*.png`). `kebab inspect doc <image_doc_id>` 의 `block.ocr.joined` 가 vision LM 의 OCR 결과 (예: 스크린샷 안의 텍스트). `kebab search --mode lexical "<OCR text>"` 가 그 image chunk 를 반환하면 wiring 정상.
- OCR / caption 부분 실패는 `errors` 카운터 미증가 — `kebab inspect doc <id>` 의 Provenance Warning 이벤트 또는 `--debug` 로그에서만 확인.
- (P7-3) `*.pdf` 자산을 워크스페이스에 두면 `kebab ingest` 출력에 PDF 도 `new` 카운터에 포함. `kebab inspect doc <pdf_doc_id>` 가 `parser_version = "pdf-text-v1"` + 페이지마다 `Block::Paragraph` + `SourceSpan::Page { page, char_start, char_end }`. 본문에 등장하는 단어로 `kebab search --mode hybrid` 시 PDF chunk 가 결과에 포함되고 `source_span.kind = "page"` 면 wiring 정상. 암호화 PDF 는 `errors+=1` 로 분류되며 `error` 필드에 `qpdf --decrypt` 안내 보존. 빈/스캔 페이지 (PDF 가 텍스트를 추출하지 못한 페이지) 는 0 chunk + `Provenance::Warning` ("scanned candidate") 로 표시 — P+ scanned-PDF OCR fallback 까지는 검색 불가.

## 정리

```bash
rm -rf /tmp/kebab-smoke/data        # 데이터만 날리고 다시 ingest 가능
rm -rf /tmp/kebab-smoke              # 통째로 정리
```

`~/.config/kebab/` 와 `~/.local/share/kebab/` 는 한 번도 터치되지 않는다 (`--config` flag 가 정확히 honor 되는 경우 — P3-5 hotfix 이후 보장).

## 알려진 동작

- 첫 `kebab ingest` 시 fastembed 모델 다운로드 (~470MB) — `data_dir/models/fastembed/` 에 캐시.
- `kebab ask` 응답 시간 = LLM 토큰 throughput 에 종속. M4 Pro 48GB + gemma4:26b 기준 답변 50–100 토큰에 20–55초.
- `--config` path 가 존재하지 않거나 malformed 면 `kebab doctor` 가 hard fail (defaults 가 silently mask 하지 않게 하는 hotfix 동작).
- 매 CLI invocation 마다 fastembed 모델 init 비용 (~4초) — process-level 캐시 부재 때문. P9 TUI 진입 시 `App` 의 `OnceLock` 으로 세션 동안 한 번만 init.
- (P6-4) `image.ocr.enabled = true` + `image.caption.enabled = true` 인 워크스페이스에 PNG 가 N장 있으면 ingest 시간 ≈ markdown_time + N × (OCR + Caption latency). `gemma4:e4b` + 192.168.0.47 로 자산당 ~5-10초. 다수의 책 페이지를 이미지로 넣지 말 것 — 책은 P7 PDF 라인 사용 권장.
- (P7-3) `config.chunking.chunker_version` 는 markdown 만 represent — PDF 자산은 `pdf-page-v1` 하드코딩. `config.toml` 의 `chunker_version = "md-heading-v1"` 을 봐도 PDF 는 영향 안 받음. HOTFIXES `2026-05-02 P7-3` entry 참조 (P+ chunker registry task 까지 유지).
- (P7-3) 한 PDF 가 N 페이지면 `kebab ingest` 가 N 개 (또는 그 이상의, 페이지 길면 multi-chunk) 의 chunk 를 한 transaction 안에서 commit. 500 페이지 책 → 500+ chunk 한 번에 → embedding throughput 가 bottleneck. 임베딩 활성 워크스페이스에서 큰 PDF 를 처음 ingest 하면 분-단위 시간 + WAL 크기 증가 가능 — P+ 스케일 hardening task 까지 정상 동작이지만 비용은 측정 가능.
- (P7-3 + follow-up) 동일 path 에 byte 가 다른 PDF 를 두 번째 ingest 하면 `purge_vector_orphans_for_workspace_path` 가 옛 chunk_id 를 LanceDB 에서 먼저 삭제, 이어서 `purge_orphan_at_workspace_path` 가 옛 doc / chunks / embedding_records 를 SQLite 에서 sweep. 새 byte 가 새 `doc_id` 로 색인됨. `IngestReport` 에 그 자산만 `new+=1` (다른 자산은 `updated`). 두 store 모두 정합 — 옛 본문 검색 시 옛 chunks 가 더 이상 surface 되지 않음.

자세한 history 와 발견된 버그는 [tasks/HOTFIXES.md](../tasks/HOTFIXES.md) 참조.
