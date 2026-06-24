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
# CPU only / RAM ≤ 16 GB 환경이면 ≤ 4B Q4 모델 권장 (gemma3:4b / qwen2.5:3b 등) —
# 8B+ 모델은 첫 RAG 답변이 5분 (기본 [models.llm] request_timeout_secs)
# 한도를 넘기 쉬워 `error: kb-rag: llm.generate_stream` 으로 떨어짐.
# 노브 늘리려면 config 에 request_timeout_secs = 1200 추가
# 또는 KEBAB_MODELS_LLM_REQUEST_TIMEOUT_SECS=1200 env. HOTFIXES 2026-05-25 참조.
```

sudo / systemd 없이 격리 디렉토리에 설치하는 경로 (컨테이너 / WSL2 / 회사 머신
유용):

```bash
# tarball 만 받아 사용자 디렉토리에 풀고 OLLAMA_MODELS 로 모델 디렉토리 분리.
mkdir -p /opt/ollama/{models,logs}
curl -fL https://ollama.com/download/ollama-linux-amd64.tar.zst -o /tmp/ollama.tar.zst
zstd -d /tmp/ollama.tar.zst -o /tmp/ollama.tar && tar -xf /tmp/ollama.tar -C /opt/ollama/
OLLAMA_MODELS=/opt/ollama/models OLLAMA_HOST=127.0.0.1:11434 \
    /opt/ollama/bin/ollama serve > /opt/ollama/logs/serve.log 2>&1 &
/opt/ollama/bin/ollama pull gemma3:4b
# 종료: pkill -f "ollama serve"
```

cold start 가 긴 모델 (8B+ 또는 첫 호출) 은 `kebab ask --stream` 으로 시도 권장
— 토큰을 stderr 에 ndjson 으로 흘려 받아 5분 timeout 한도 안에서도 첫 토큰이
빨리 surface 됨 (fb-33). 자세한 명령은 아래 "Streaming ask (fb-33)" 절.
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

# v0.28.0: 모든 형식 ingest 설정의 우산. 병렬도(← 옛 [indexing])는 [ingest] 스칼라로,
# chunking/code/image/pdf 는 [ingest.*] 하위로 통합. 옛 v2 파일은 로드 시 자동 변환됨.
[ingest]
max_parallel_extractors = 2
max_parallel_embeddings = 1
watch_filesystem = false

[ingest.chunking]
target_tokens = 500
overlap_tokens = 80
respect_markdown_headings = true
chunker_version = "md-heading-v2"
max_chunk_tokens = 4000              # v0.30.0 — 이 byte/3 토큰 초과 청크는 줄(→UTF-8 char) 경계로 분할(거대 list/code/log 덤프 대비)

[models.embedding]
provider = "fastembed"               # "fastembed"(기본, onnxruntime) / "candle"(순수 Rust, NUMA-안전)
                                     # / "ollama"(원격 HTTP /api/embed) / "none"(lexical-only — Ollama 불필요)
                                     # ⚠ provider/model 변경 시 아래 dimensions 도 맞춰야 함.
model = "multilingual-e5-small"      # candle/ollama 는 "snowflake-arctic-embed-l-v2.0"
                                     # (ollama 태그 "snowflake-arctic-embed2", 1024-dim) 도 지원 —
                                     # 설명형 query recall 보강. e5↔arctic 전환은
                                     # embedding_version cascade (재색인 필요).
version = "v1"
dimensions = 384                     # arctic / e5-large 는 1024.
batch_size = 64
num_threads = 0                      # candle 전용 CPU 스레드 캡 (0=auto). env KEBAB_EMBED_THREADS 우선.
# endpoint = "http://127.0.0.1:11434"  # provider="ollama" 전용; 생략 시 [models.llm].endpoint fallback.

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
# v0.18.0 fb-41 multi-hop NLI gate (default 0.0 = disabled).
# `kebab ask --multi-hop` 사용 시 0.5 권장 — entailment < 0.5 면 refuse.
# 첫 호출 시 mDeBERTa-v3 XNLI ONNX 모델 자동 다운로드 (~280 MB, ~30-60s),
# RAM peak ~7-8 GB (gemma3:4b 기준, 16 GB 환경 안전). model 실패 시
# `refusal_reason = "nli_model_unavailable"` — `nli_threshold = 0` 으로 disable.
nli_threshold = 0.0

[ui]
theme = "dark"                       # p9-fb-14 — TUI palette ("dark" / "light", default "dark")

[ingest.code]
skip_generated_header = true
max_file_bytes = 262144
max_file_lines = 5000
extra_skip_globs = []                # 사용자 추가 skip 패턴 (gitignore syntax)

[logging]
ingest_log_enabled = true
ingest_log_dir = "{state_dir}/logs"
keep_recent_runs = 100               # v0.20.x r2: 최근 N 개 run log 파일 보존
retention_days = 30                  # v0.20.x r2: N일 이상 된 log / OCR 이벤트 자동 삭제
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

### 한국어 morphological 검색 (v0.20.1)

`chunks_fts` 가 FTS5 `unicode61` tokenizer + 한국어 lindera ko-dic 형태소 분해된 별 column (`tokenized_korean_text`) 으로 동작 (V009 migration). 한국어 2-char query (`한국`, `서울`) 도 ko-dic morpheme 매칭 시 hit. V009 자동 backfill (`App::open_with_config` 의 first-boot hook) 이라 기존 KB 의 binary 만 v0.20.1+ 로 교체하면 첫 부팅에서 자동 재-tokenize (re-ingest 불필요). `kebab.sqlite` 파일 크기가 형태소 column + lindera-ko-dic embedded dict 의존성으로 다소 증가.

`fixtures/search/korean/hash-table.md` (또는 등가) 를 워크스페이스에 두고 ingest 한 후:

```bash
# 2-char Korean (v0.20.1 의 핵심 가치 — Bug #8 close)
KB search --mode lexical "한국"
KB search --mode lexical "서울"

# 3-char Korean
KB search --mode lexical "한국어"
KB search --mode lexical "지하철"

# Compound noun (ko-dic 가 형태소 분해 — '서울특별시' → [서울, 특별시])
KB search --mode lexical "서울특별시"

# multi-token Korean
KB search --mode lexical "해시 충돌"

# 한영 혼합
KB search --mode lexical "Rust 최적화"

# 1자 query — `build_match_string` 의 MIN_QUERY_CHARS=2 filter 로 0 hit
KB search --mode lexical "키"

# raw FTS5 mode
KB search --mode lexical "'충돌'"
```

영어 lexical 은 V002 의 whole-token 매칭으로 회귀 — `KB search --mode lexical "token"` 은 `token` 토큰만 hit, `tokenizer` 의 substring 으로 매칭 X. substring recall 이 필요하면 vector/hybrid mode 권장 (spec §3 Non-Goals Path A).

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

### Verbatim fetch (fb-35)

```bash
# Search to get a chunk_id.
CHUNK_ID=$(kebab search "rust ownership" --json --k 1 | jq -r '.hits[0].chunk_id')

# Fetch verbatim with surrounding context.
kebab fetch chunk "$CHUNK_ID" --context 2 --json | jq .

# Fetch the full doc as markdown.
DOC_ID=$(kebab search "rust ownership" --json --k 1 | jq -r '.hits[0].doc_id')
kebab fetch doc "$DOC_ID" --max-tokens 1000 --json | jq '{kind, truncated, len: (.text | length)}'

# Fetch a line range (markdown / text only).
kebab fetch span "$DOC_ID" 1 5 --json | jq '{line_start, line_end, effective_end, text}'
```

PDF / audio docs reject `fetch span` with `error.v1.code = span_not_supported` — use `fetch chunk` (PDF chunks are page-aligned) or `fetch doc` instead.

### Filter args (fb-36)

````bash
# Filter by media kind (md alias normalizes to markdown).
kebab search "rust" --media md --json | jq '.hits | length'

# Filter by ingest timestamp (RFC3339).
kebab search "rust" --ingested-after 2026-04-01T00:00:00Z --json

# Combine: doc-id scope + tag (AND across flags).
kebab search "rust" --doc-id "<doc-id>" --tag rust --json
````

Bad `--ingested-after` → `error.v1.code = config_invalid`, exit 2.
Unknown `--media` value → silently empty (no error).

### Source filters (`--source` / `--source-type`)

````bash
# 단일 root 워크스페이스는 implicit `default` source 로 정규화되므로
# 모든 문서가 source_id="default" — 이 필터는 전체와 동일하다.
kebab search "rust" --source default --json | jq '.hits | length'

# source_type 필터 (frontmatter 의 source_type: 또는 source 기본값).
kebab search "rust" --source-type markdown,reference --json | jq '.hits | length'
````

멀티소스 KB 는 `[[workspace.sources]]` 로 명명 source 를 선언하면
`--source <id>` 로 출처를 좁힌다 (예: `--source jira` → jira 문서만).
빈 값 = 무필터, 콤마/반복 = OR. 모르는 값 → silently empty (no error).

### Trace + stats (fb-37)

Re-run a search with `--trace` to see per-stage candidate lists + timing:

```bash
kebab --config /tmp/kebab-smoke/config.toml search "rust async" --trace --json | jq .trace
```

Inspect the corpus health surface:

```bash
kebab --config /tmp/kebab-smoke/config.toml schema --json | jq .stats
```

Look for: `media_breakdown` (5 keys), `lang_breakdown`, `index_bytes`, `stale_doc_count`.

### Bulk multi-query (fb-42)

Stdin ndjson으로 N query 한 번에:

```bash
printf '{"query":"rust","mode":"lexical"}\n{"query":"async","mode":"lexical"}\n' \
  | kebab --config /tmp/kebab-smoke/config.toml search --bulk --json
```

stdout: per-query ndjson (`bulk_search_item.v1`). stderr: `bulk_summary: total=2 succeeded=2 failed=0`.

MCP tool 동등:

```json
{"name":"kebab__bulk_search","arguments":{"queries":[{"query":"rust"},{"query":"async"}]}}
```

## P6-4 이미지 ingestion 옵션

`config.toml` 에 다음 절을 추가하면 `kebab ingest` 가 `**/*.png` / `**/*.jpg` 등 이미지 자산도 함께 색인합니다 (텍스트만 색인하려면 생략):

```toml
[workspace]
include = ["**/*.md", "**/*.png", "**/*.jpg"]

[ingest.image.ocr]
enabled = true                        # vision LM 으로 이미지 안 텍스트 전사
engine = "ollama-vision"
model = "gemma4:e4b"                  # 사용자 환경의 비전 모델
endpoint = "http://192.168.0.47:11434"  # 비우면 models.llm.endpoint fallback
languages = ["eng", "kor"]
max_pixels = 1600                     # long-edge cap

[ingest.image.caption]
enabled = true                        # vision LM 으로 한 문장 객관 설명 생성
max_pixels = 768
prompt_template_version = "caption-v1"

[ingest.pdf.ocr]
enabled = true               # smoke test 의 OCR path 활성화 (manual invoke)
always_on = false
engine = "ollama-vision"
model = "qwen2.5vl:3b"
# endpoint = "http://192.168.0.47:11434"   # 사용자 dogfood host
languages = ["eng", "kor"]
max_pixels = 2048
request_timeout_secs = 600
valid_ratio_threshold = 0.5
min_char_count = 20
lang_hint = "kor"
```

이미지 자산 한 장당 OCR 1 호출 + Caption 1 호출 → ~3-6초 (`gemma4:e4b` 기준). 다이어그램 / 카메라 사진 / 스크린샷 위주 워크스페이스에 권장. 책 / 스캔본은 P7 PDF 라인으로.

**v0.27.0 — paddle-onnx 엔진 (오프라인, Ollama 불필요).** `[ingest.image.ocr] engine = "paddle-onnx"` 로 바꾸면 PP-OCRv5 ONNX 를 in-process 로 실행한다 (원격 vision LM 불필요, 큰 페이지 CPU <4초). embedding 까지 끄려면 `[models.embedding] provider = "none"` (lexical-only) 로 두면 Ollama 없이 OCR→FTS5 검색 전체 경로를 스모크할 수 있다:

```toml
[models.embedding]
provider = "none"            # lexical-only — Ollama 불필요

[ingest.image.ocr]
enabled = true
engine = "paddle-onnx"       # PP-OCRv5 ONNX in-process (Python/원격 0)
model = "ppocrv5-mobile-kor"
languages = ["kor", "eng"]
max_pixels = 1600
# det_model / rec_model / dict 로 번들 모델 경로 override 가능 (생략 시 번들 사용)
# score_thresh = 0.3 / unclip_ratio = 1.5 / max_boxes = 1000 으로 검출 튜닝
```

스모크: `kebab ingest --config <cfg>` 후 `kebab search --config <cfg> --mode lexical "<이미지 안 한국어 단어>"` 가 그 image chunk 를 반환하면 OCR→FTS5 wiring 정상. engine 또는 모델을 바꾸면 다음 ingest 가 영향 이미지를 자동 재색인한다.

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

## P10-1A-2 Rust 코드 색인

`kebab-parse-code` 의 tree-sitter Rust AST extractor + `code-rust-ast-v1` chunker 를 격리된 TempDir KB 에서 검증하는 절차.

```bash
# 1) 워크스페이스에 Rust 소스 파일 추가 (crate 하나 복사 또는 단일 .rs 파일)
cp -r crates/kebab-parse-code /tmp/kebab-smoke/workspace/kebab-parse-code

# 2) ingest — .rs 가 code-rust-ast-v1 로 처리됨
KB ingest

# 3) 결과 검증 — IngestReport.items 에 .rs 자산이 "new" 로 분류, parser_version = "code-rust-v1" (chunker_version = "code-rust-ast-v1")
KB --json ingest | jq '[.items[] | select(.doc_path | endswith(".rs"))]'

# 4) 코드 검색 — code_lang 필터 (wire: lang 은 citation.lang, code_lang 은 SearchHit top-level)
KB search --mode hybrid "RustAstExtractor" --code-lang rust --json | jq '{hits: [.hits[] | {symbol: .citation.symbol, code_lang: .citation.lang, repo: .repo}]}'

# 5) citation 확인 — kind="code", symbol 이 함수명 / 타입명, line range 가 포함
KB search --mode lexical "pub fn extract" --code-lang rust --json | jq '.hits[0].citation'
```

`[ingest.code]` 설정 (config.toml 에 이미 포함됨 — 위 격리 config 블록 참조):

```toml
[ingest.code]
skip_generated_header = true   # @generated / DO NOT EDIT 감지 시 skip
max_file_bytes = 262144        # 256 KiB cap — 초과 시 skip
max_file_lines = 5000          # 5000 줄 cap — 초과 시 skip
extra_skip_globs = []          # 사용자 추가 skip 패턴
```

**알려진 동작 (2026-05-19 기준)**:

- `ast_chunk_max_lines = 200` 은 config 가 아닌 chunker 모듈 상수. 현재 기본값과 동일하므로 user-visible 차이 없음. 자세한 내용: `tasks/HOTFIXES.md` (2026-05-19 `AST_CHUNK_MAX_LINES` 항목).
- `.rs` 파일은 `SourceType::Note` 로 분류됨 (kebab-core `SourceType::Code` variant 미존재). `--media code` filter 는 정상 동작 — `MediaType::Code("rust")` 로 별도 분류됨. 자세한 내용: `tasks/HOTFIXES.md` (2026-05-19 `SourceType::Code` 항목).
- `.gitignore` 가 honor 됨 — `target/` / `node_modules/` 등은 built-in 안전망으로 자동 skip.

## P10-1B Python / TypeScript / JavaScript 코드 색인

P10-1A-2 와 동일한 격리 KB 설정으로 Python / TypeScript / JavaScript 3 언어를 검증한다. 설정 블록은 P10-1A-2 와 동일 (`[ingest.code]` 절 포함).

```bash
# 1) 워크스페이스에 Python / TS / JS 파일 추가 (소규모 샘플로 충분)
mkdir -p /tmp/kebab-smoke/workspace/sample_code
# Python 예시
cat > /tmp/kebab-smoke/workspace/sample_code/metrics.py <<'EOF'
def compute_mrr(results):
    """Mean Reciprocal Rank."""
    total = 0.0
    for i, hit in enumerate(results, 1):
        if hit:
            total += 1.0 / i
            break
    return total
EOF
# TypeScript 예시
cat > /tmp/kebab-smoke/workspace/sample_code/searcher.ts <<'EOF'
export class Searcher {
  search(query: string): string[] {
    return [];
  }
}
EOF
# JavaScript 예시
cat > /tmp/kebab-smoke/workspace/sample_code/utils.js <<'EOF'
function formatResult(hit) {
  return `${hit.score}: ${hit.path}`;
}
module.exports = { formatResult };
EOF

# 2) ingest
KB ingest

# 3) 언어별 검색 (symbol + module path prefix 확인)
KB search --mode hybrid "compute_mrr" --code-lang python --json | \
  jq '{hits: [.hits[] | {symbol: .citation.symbol, lang: .citation.lang}]}'
KB search --mode hybrid "search" --code-lang typescript --json | \
  jq '{hits: [.hits[] | {symbol: .citation.symbol, lang: .citation.lang}]}'
KB search --mode hybrid "formatResult" --code-lang javascript --json | \
  jq '{hits: [.hits[] | {symbol: .citation.symbol, lang: .citation.lang}]}'

# 4) schema stats 에 3 언어 카운트 확인
KB --json schema | jq '.stats.code_lang_breakdown'
# 기대: {"python": N, "typescript": N, "javascript": N, "rust": M, ...}
```

**Symbol path 컨벤션 (2026-05-20 기준)**:

- **Python**: workspace 경로 → dotted module path prefix. `sample_code/metrics.py` 의 `compute_mrr` → symbol `sample_code.metrics.compute_mrr`.
- **TypeScript / JavaScript**: workspace 경로 → slash-style module path prefix. `sample_code/searcher.ts` 의 `search` → `sample_code/searcher.Searcher.search`. `.tsx` / `.mjs` / `.cjs` / `.jsx` 도 동일 처리.
- **Rust** (1A-2): file-scope nesting 만, workspace path prefix 없음 (예: `Foo::double`). Python/TS/JS 와 비일관 — HOTFIXES 2026-05-20 참조.

**알려진 동작**:

- `const foo = () => {...}` 같은 expression-level 함수는 `<top-level>` glue 로 잡힘 (declaration-level 단위만 1B 1차 범위). 자세한 내용: `tasks/HOTFIXES.md` (2026-05-20).
- `.gitignore` honor — `node_modules/` / `__pycache__/` / `.venv/` 등 built-in 안전망 자동 skip.

## P10-1C-Go Go 코드 색인

P10-1B 와 동일한 격리 KB 설정. `.go` 파일을 워크스페이스에 두고 ingest 하면 `code-go-ast-v1` chunker 가 package 단위 AST 로 처리한다.

```bash
cat > /tmp/kebab-smoke/workspace/sample_code/hello.go <<'EOF'
package main

import "fmt"

func Hello(name string) string {
    return fmt.Sprintf("Hello, %s!", name)
}
EOF

KB ingest
KB search --mode hybrid "Hello" --code-lang go --json | \
  jq '{hits: [.hits[] | {symbol: .citation.symbol, lang: .citation.lang}]}'
# 기대: symbol = "main.Hello", lang = "go"
```

## P10-2 Tier 2 리소스 파일 색인

P10-1C-Go 와 동일한 격리 KB 설정. `.yaml` / `Dockerfile` / `.toml` 등 Tier 2 리소스 파일을 워크스페이스에 두고 ingest 하면 각 확장자에 맞는 chunker 로 처리된다.

```bash
# 1) Kubernetes manifest (YAML multi-doc)
cat > /tmp/kebab-smoke/workspace/deploy.yaml <<'EOF'
apiVersion: apps/v1
kind: Deployment
metadata:
  name: my-app
  namespace: default
spec:
  replicas: 2
  selector:
    matchLabels:
      app: my-app
  template:
    metadata:
      labels:
        app: my-app
    spec:
      containers:
        - name: app
          image: my-app:latest
---
apiVersion: v1
kind: Service
metadata:
  name: my-app-svc
  namespace: default
spec:
  selector:
    app: my-app
  ports:
    - port: 80
EOF

# 2) Dockerfile (전체 파일 단일 chunk)
cat > /tmp/kebab-smoke/workspace/Dockerfile <<'EOF'
FROM rust:1.85 AS builder
WORKDIR /app
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
COPY --from=builder /app/target/release/kebab /usr/local/bin/kebab
ENTRYPOINT ["kebab"]
EOF

# 3) Cargo.toml (manifest — 전체 파일 단일 chunk)
cp Cargo.toml /tmp/kebab-smoke/workspace/Cargo.toml

# 4) ingest
KB ingest

# 5) 언어별 검색 (citation.symbol 확인)
KB search --mode hybrid "Deployment" --code-lang yaml --json | \
  jq '{hits: [.hits[] | {symbol: .citation.symbol, lang: .citation.lang}]}'
# 기대: symbol = "Deployment/default/my-app" (kind/namespace/name), lang = "yaml"

KB search --mode hybrid "rust:1.85" --code-lang dockerfile --json | \
  jq '{hits: [.hits[] | {symbol: .citation.symbol, lang: .citation.lang}]}'
# 기대: symbol = "<dockerfile>", lang = "dockerfile"

KB search --mode hybrid "kebab-cli" --code-lang toml --json | \
  jq '{hits: [.hits[] | {symbol: .citation.symbol, lang: .citation.lang}]}'
# 기대: symbol = "<manifest>", lang = "toml"

# 6) schema stats 에 Tier 2 언어 카운트 확인
KB --json schema | jq '.stats.code_lang_breakdown'
# 기대: {"yaml": N, "dockerfile": N, "toml": N, ...}
```

**Tier 2 citation.symbol 컨벤션**:

- **YAML k8s 리소스**: `<kind>/<namespace>/<name>` (예: `Deployment/default/my-app`). `namespace` 없으면 `<kind>/<name>`. multi-doc YAML 은 `---` 구분자 기준으로 resource 별 chunk.
- **Dockerfile**: `<dockerfile>` (고정 심볼, 전체 파일이 단일 chunk).
- **TOML / JSON / XML / Groovy / go.mod**: `<manifest>` (고정 심볼, 전체 파일이 단일 chunk). 단, 파일이 `tier2_shared` 의 oversize threshold 초과 시 줄 단위 fallback chunk.

## P10-3 Tier 3 paragraph fallback

P10-2 와 동일한 격리 KB 설정. `.sh` 파일은 direct, 비-k8s YAML 은 fallback 으로 들어간다.

```bash
# 1) shell script (direct Tier 3)
cat > /tmp/kebab-smoke/workspace/deploy.sh <<'EOF'
#!/usr/bin/env bash
set -e

echo "ingesting..."
kebab ingest

echo "done"
kebab schema --json | jq '.stats'
EOF

# 2) 비-k8s YAML (Tier 2 가 0 chunk → Tier 3 fallback)
cat > /tmp/kebab-smoke/workspace/docker-compose.yml <<'EOF'
version: '3'
services:
  api:
    image: nginx:latest
    ports:
      - 8080:80
EOF

# 3) ingest
KB ingest

# 4) 언어별 검색 (citation.symbol = None 확인)
KB search --mode hybrid "ingest" --code-lang shell --json | \
  jq '{hits: [.hits[] | {symbol: .citation.symbol, lang: .citation.lang, chunker: .chunker_version}]}'
# 기대: symbol = null, lang = "shell", chunker_version = "code-text-paragraph-v1"

KB search --mode hybrid "nginx" --code-lang yaml --json | \
  jq '{hits: [.hits[] | {symbol: .citation.symbol, lang: .citation.lang, chunker: .chunker_version}]}'
# 기대: symbol = null, lang = "yaml", chunker_version = "code-text-paragraph-v1"

# 5) schema stats 에 shell 카운트 확인
KB --json schema | jq '.stats.code_lang_breakdown'
# 기대: {"shell": N, "yaml": M, ...}
```

**Tier 3 citation.symbol 컨벤션**: 항상 `null`. 의미 단위 식별 안 함. `lang` 은 원본 lang 보존 (shell → `"shell"`, yaml → `"yaml"` 등).

## P10-1D C + C++ AST chunkers

P10-3 와 동일한 격리 KB 설정. `.c` 와 `.cpp` 파일이 각자의 AST chunker 로 처리된다.

```bash
# 1) C 파일 — top-level function symbol
cat > /tmp/kebab-smoke/workspace/parser.c <<'EOF'
#include <stdio.h>

int parse_record(const char *line) {
    if (line == NULL) return -1;
    return 0;
}
EOF

# 2) C++ 파일 — namespace::Class::method symbol
cat > /tmp/kebab-smoke/workspace/chunker.cpp <<'EOF'
namespace kebab {
namespace chunk {

class Foo {
public:
    void bar() { /* impl */ }
};

}  // namespace chunk
}  // namespace kebab
EOF

# 3) ingest
KB ingest

# 4) 언어별 검색 (citation.symbol 확인)
KB search --mode hybrid "parse_record" --code-lang c --json | \
  jq '{hits: [.hits[] | {symbol: .citation.symbol, lang: .citation.lang}]}'
# 기대: symbol = "parse_record" (function name only), lang = "c"

KB search --mode hybrid "bar" --code-lang cpp --json | \
  jq '{hits: [.hits[] | {symbol: .citation.symbol, lang: .citation.lang}]}'
# 기대: symbol = "kebab::chunk::Foo" 또는 "kebab::chunk::Foo::bar" (namespace::Class[::method]), lang = "cpp"

# 5) schema stats 에 C/C++ 카운트 확인
KB --json schema | jq '.stats.code_lang_breakdown'
# 기대: {"c": N, "cpp": M, ...}
```

**Tier 1 (p10-1D) citation.symbol 컨벤션**: C 는 function name only (`parse_record` 같이 nesting 없음). C++ 는 `namespace::Class::method` (recursive namespace + class nesting). `.h` 파일이 C++ syntax (namespace / template / class) 만나면 tree-sitter-c parse 실패 → p10-3 Tier 3 fallback (`code-text-paragraph-v1`) 으로 자동 picked up.

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

## config migrate (마이그레이션)

```bash
# 옛 스키마처럼 섹션이 빠진 config 를 흉내내 migrate 동작 확인.
printf 'schema_version = 1\n\n[workspace]\nroot = "/tmp/kb"\ninclude = ["*.md"]\n' \
  > /tmp/kebab-smoke/old.toml
kebab --config /tmp/kebab-smoke/old.toml config migrate --dry-run   # 변경 미리보기 (파일 미수정)
kebab --config /tmp/kebab-smoke/old.toml config migrate             # 적용 (.bak 백업)
kebab --config /tmp/kebab-smoke/old.toml config migrate             # 멱등: "config 이미 최신입니다"
kebab --config /tmp/kebab-smoke/old.toml --json config migrate --dry-run | jq .schema_version
```

기대: dry-run 은 추가될 섹션(`[ingest.code]`·`[logging]` 등)과 제거될 `workspace.include` 를 출력하고 **파일을 수정하지 않는다**. 적용 시 `old.toml.bak`(원본과 동일)이 생기고 빠진 섹션이 주석과 함께 추가되며 사용자가 손본 값·주석은 보존된다. 재실행은 멱등(`config 이미 최신입니다`), `--json` 은 `config_migration.v1`.

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
- (P10-1A-2) `.rs` 파일을 워크스페이스에 두면 `kebab ingest` 결과에 `new` 카운터에 포함. `kebab search --mode hybrid "<함수명>" --code-lang rust --json` 가 `citation.kind = "code"`, `citation.lang = "rust"` (SearchHit top-level `code_lang` 도 동일), `citation.symbol` (함수/타입 이름), `citation.line_start` / `citation.line_end` 를 반환하면 wiring 정상. `kebab schema --json | jq .stats.code_lang_breakdown` 에 `"rust": N` 이 나오면 chunk 가 색인됨.
- (P10-1B) `.py` / `.ts` / `.tsx` / `.js` / `.mjs` / `.cjs` / `.jsx` 파일을 워크스페이스에 두면 `kebab ingest` 결과에 `new` 카운터에 포함. `--code-lang python` / `--code-lang typescript` / `--code-lang javascript` 검색이 `citation.symbol` 에 module path prefix 를 포함한 결과를 반환하면 wiring 정상. `kebab schema --json | jq .stats.code_lang_breakdown` 에 해당 언어 카운트 등장 확인.
- (P10-1C-Go) `.go` 파일을 워크스페이스에 두면 `kebab ingest` 가 `code-go-ast-v1` 로 처리. `--code-lang go` 검색이 `citation.symbol` 에 `<package>.<Func>` / `<package>.(*Receiver).<Method>` 형식 결과를 반환하면 wiring 정상. `kebab schema --json | jq .stats.code_lang_breakdown` 에 `"go": N` 등장 확인.
- (P10-1C-JK) `.java` 파일은 `code-java-ast-v1`, `.kt`/`.kts` 파일은 `code-kotlin-ast-v1` 로 처리. `--code-lang java` / `--code-lang kotlin` 검색이 `citation.symbol` 에 `com.foo.Foo.bar` 형식 결과를 반환하면 wiring 정상. `kebab schema --json | jq .stats.code_lang_breakdown` 에 `"java": N` / `"kotlin": N` 등장 확인.
- (P10-2) `.yaml`/`.yml` 파일은 apiVersion+kind 파싱으로 k8s resource 별 chunk 생성 (`k8s-manifest-resource-v1`). `Dockerfile`/`Dockerfile.*` 는 전체 파일 단일 chunk (`dockerfile-file-v1`). `.toml`/`.json`/`.xml`/`.groovy`/`go.mod` 는 전체 파일 단일 chunk (`manifest-file-v1`). `--code-lang yaml` / `--code-lang dockerfile` / `--code-lang toml` 검색이 `citation.symbol` 에 각각 `Deployment/default/my-app` / `<dockerfile>` / `<manifest>` 형식 결과를 반환하면 wiring 정상. `kebab schema --json | jq .stats.code_lang_breakdown` 에 `"yaml": N` / `"dockerfile": N` / `"toml": N` 등장 확인.
- (P10-3) `.sh`/`.bash`/`.zsh` 파일은 direct Tier 3 (`code-text-paragraph-v1`). 비-k8s YAML (apiVersion+kind 없는 yaml) 은 k8s chunker 가 0 chunk → Tier 3 fallback 으로 picked up. `--code-lang shell` / `--code-lang yaml` 검색이 `citation.symbol = null`, `chunker_version = "code-text-paragraph-v1"` 결과를 반환하면 wiring 정상. `kebab schema --json | jq .stats.code_lang_breakdown` 에 `"shell": N` 등장 확인.
- (P10-1D) `.c` / `.h` 파일은 `code-c-ast-v1` (function name only symbol). `.cpp`/`.cc`/`.cxx`/`.hpp`/`.hh`/`.hxx` 는 `code-cpp-ast-v1` (`namespace::Class::method` symbol). `--code-lang c` / `--code-lang cpp` 검색 동작 + `kebab schema --json | jq .stats.code_lang_breakdown` 에 `"c": N` / `"cpp": M` 등장 확인. `.h` 파일이 C++ 내용 (namespace 등) 갖고 있으면 자동으로 Tier 3 (`code-text-paragraph-v1`) fallback 으로 picked up.
- (P7-3 + follow-up) 동일 path 에 byte 가 다른 PDF 를 두 번째 ingest 하면 `purge_vector_orphans_for_workspace_path` 가 옛 chunk_id 를 LanceDB 에서 먼저 삭제, 이어서 `purge_orphan_at_workspace_path` 가 옛 doc / chunks / embedding_records 를 SQLite 에서 sweep. 새 byte 가 새 `doc_id` 로 색인됨. `IngestReport` 에 그 자산만 `new+=1` (다른 자산은 `updated`). 두 store 모두 정합 — 옛 본문 검색 시 옛 chunks 가 더 이상 surface 되지 않음.

### Embedding upgrade (fb-39b)

`multilingual-e5-small` 에서 `multilingual-e5-large` 로 업그레이드 시퀀스:

```bash
# 기존 vector index 정리 (orphan table 회피)
kebab --config /tmp/kebab-smoke/config.toml reset --vector-only

# config.toml 의 [models.embedding] 갱신:
#   model = "multilingual-e5-large"
#   dimensions = 1024

# 재-ingest — fastembed 가 첫 실행 시 e5-large ONNX (~1.3 GB) 자동 다운로드.
# 다운로드 시간 + 모든 chunk re-embed 시간 (e5-small 대비 ~3-4×).
kebab --config /tmp/kebab-smoke/config.toml ingest

# fb-39 의 P@k metric 으로 small vs large 비교:
kebab --config /tmp/kebab-smoke/config.toml eval run
```

### v0.20 force-reingest (scanned PDF OCR)

v0.19 binary 로 indexed scanned PDF (책 스캔 등) 가 v0.20 upgrade 후 OCR path 진입 안 함 — `parser_version = "pdf-text-v1"` 보존이라 `try_skip_unchanged` 가 Unchanged 반환. 명시적 force:

```bash
# v0.19 에서 scanned PDF 가 빈 block + "scanned candidate" warning 으로 indexed:
KEBAB_PDF_OCR_ENABLED=false kebab --config /tmp/kebab-smoke/config.toml ingest

# v0.20 binary upgrade 후 OCR 활성화 (config 갱신 또는 env) + force-reingest:
KEBAB_PDF_OCR_ENABLED=true kebab --config /tmp/kebab-smoke/config.toml ingest --force-reingest

# 결과: 이전 빈 block 들이 OCR text block 으로 replace, provenance.events 에
# OcrApplied event 가 page 마다 추가. ingest_progress 의 pdf_ocr_started/finished
# 가 stderr 에 emit.
```

자세한 history 와 발견된 버그는 [tasks/HOTFIXES.md](../tasks/HOTFIXES.md) 참조.
