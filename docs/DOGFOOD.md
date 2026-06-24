# DOGFOOD — kebab 도그푸딩 시나리오

> 본 file 은 kebab 의 모든 기능에 대한 dogfood 시나리오 의 single reference. 새 dogfood request 시 본 file 의 시나리오 list 를 참조. 새 sub-item 또는 새 release 마다 §관련 section 의 시나리오 갱신.
>
> 사용법: 도그푸딩 요청 시 본 doc 의 시나리오 카탈로그 → 적절한 시나리오 list 선정 → 환경 setup (§0) → 시나리오 실행 → bug log → 발견된 bug 즉시 fix cycle.

## 목차
- [§0 환경 + 사전 준비](#0-환경--사전-준비)
- [§1 Ingest](#1-ingest)
- [§2 Search](#2-search)
- [§3 Ask (RAG)](#3-ask-rag)
- [§4 Inspect / Fetch / List](#4-inspect--fetch--list)
- [§5 Version cascade](#5-version-cascade)
- [§6 Wire schema (`--json`)](#6-wire-schema---json)
- [§7 TUI (P9)](#7-tui-p9)
- [§8 MCP (P9-fb-30)](#8-mcp-p9-fb-30)
- [§9 Doctor / Schema / Reset](#9-doctor--schema--reset)
- [§10 Eval (P5)](#10-eval-p5)
- [§11 Edge cases](#11-edge-cases)
- [§12 Bug discovery checklist](#12-bug-discovery-checklist)
- [§13 Reference dogfood corpus](#13-reference-dogfood-corpus)

---

## §0 환경 + 사전 준비

### 0.1 release binary

```bash
export CARGO_TARGET_DIR=/build/out/cargo-target/target
export RELEASE_BIN="${CARGO_TARGET_DIR:-target}/release/kebab"
cargo build --release -p kebab-cli -j 4 2>&1 | tail -5
"$RELEASE_BIN" --version  # 기대: kebab X.Y.Z (current workspace version)
```

### 0.2 Ollama endpoint check

```bash
# Remote (사용자 dogfood host)
curl -s --connect-timeout 3 http://192.168.0.47:11434/api/tags | jq -r '.models[]?.name'
# 기대: qwen2.5vl:3b / gemma4:e4b / gemma4:26b / bge-m3:latest / nomic-embed-text:latest

# Local fallback
curl -s --connect-timeout 3 http://localhost:11434/api/tags 2>&1 | head -3
```

### 0.3 isolated KB workspace

```bash
DOGFOOD=/build/cache/tmp/<sub-item>-dogfood
mkdir -p "$DOGFOOD/kb" "$DOGFOOD/data"

# Default config from `kebab init` then customize
HOME=/tmp/dogfood-home XDG_CONFIG_HOME="$DOGFOOD/xdg" "$RELEASE_BIN" init
cp "$DOGFOOD/xdg/kebab/config.toml" "$DOGFOOD/config.toml"

# 사용자 customize: workspace.root, storage.data_dir, models.{embedding,llm}.endpoint, 기능 enable
```

### 0.4 disk + cargo clean policy

memory `feedback-cargo-clean-policy`:
- `/build` avail < 500GB OR target > 200GB 시만 cargo clean.
- 평소 incremental build.

### 0.5 dogfood corpus location

본 doc §13 reference. 표준 corpus = PoC 9 PDF + markdown notes + code samples + image set.

---

## §1 Ingest

### §1.1 Markdown ingest (P1)

**기본**:
```bash
# Test markdown KB
mkdir -p "$DOGFOOD/kb/notes"
cat > "$DOGFOOD/kb/notes/sample.md" <<'EOF'
# Title

본문 paragraph. **bold** + *italic*.

## Heading 2

- list item 1
- list item 2

```rust
fn main() { println!("code block"); }
```
EOF

"$RELEASE_BIN" ingest --config "$DOGFOOD/config.toml" --json | tail -1 | jq '.items[] | {doc_path, kind, block_count, chunk_count}'
```

**verify**:
- `kind` = `"new"` (first ingest) / `"unchanged"` (no change) / `"updated"` (modified).
- `block_count` ≥ heading/paragraph count.
- `chunk_count` ≥ 1.

**scenarios**:
- 1.1.a empty markdown (0 byte) → expected warning + 0 chunks.
- 1.1.b deeply nested heading (h1-h6) → block 모두 capture.
- 1.1.c frontmatter (YAML/TOML) → metadata preserve.
- 1.1.d code fence (Rust/Python/etc.) → code block 으로 chunk.
- 1.1.e mixed inline (link/image/inline-code/strong/em) → inline preserve.
- 1.1.f huge markdown (1 MB+) → walker pass + chunk count delta.

### §1.2 Image ingest (P6)

**Config**:
```toml
[image.ocr]
enabled = true
engine = "ollama-vision"
model = "gemma4:e4b"
endpoint = "http://192.168.0.47:11434"

[image.caption]
enabled = false  # opt-in
```

**verify**:
- `*.png` / `*.jpg` / `*.jpeg` 만 ingest target.
- OCR text 가 `Block::ImageRef.ocr.joined` 안.
- `[image.caption].enabled=true` 시 caption 도.

**scenarios**:
- 1.2.a Korean OCR (한국어 scan PNG) → OCR text + search hit.
- 1.2.b English OCR (typed text screenshot) → alnum > 90%.
- 1.2.c photo (자연 사진, OCR 없음) → empty OCR or warning.
- 1.2.d corrupt image → graceful error.
- 1.2.e oversized image (> max_pixels) → downscale.

### §1.3 PDF text ingest (P7-1)

**기본**:
- vector PDF (embedded text) — `PdfTextExtractor` 가 page 별 text 추출.
- 1 `Block::Paragraph` per page (P7-1 invariant).

**verify**:
- `parser_version = "pdf-text-v1"`.
- `chunker_version = "pdf-page-v1"` (또는 `"pdf-page-v1.1"` from v0.20.1).
- `block_count` ≥ page count.

**scenarios**:
- 1.3.a single page vector PDF → 1 block.
- 1.3.b multi-page (10+ pages) → block per page.
- 1.3.c large PDF (50+ pages, 50 MB+) → ingest 시간 + memory monitor.
- 1.3.d encrypted PDF → friendly error wording (`qpdf --decrypt` hint).
- 1.3.e corrupt PDF (truncated bytes) → graceful error.
- 1.3.f PDF with annotations / forms → extract 동작.

### §1.4 PDF scanned OCR ingest (v0.20.0 sub-item 1)

**Config**:
```toml
[pdf.ocr]
enabled = true
always_on = false
model = "qwen2.5vl:3b"
endpoint = "http://192.168.0.47:11434"
valid_ratio_threshold = 0.5
min_char_count = 20
```

**verify**:
- IngestEvent::PdfOcrStarted / PdfOcrFinished emit.
- `IngestItem.pdf_ocr_pages > 0` for scanned PDF.
- `IngestItem.pdf_ocr_ms_total > 0`.
- CLI printer: `📷 OCR page N...` / `✓ OCR page N (chars chars, msms via ollama-vision)`.

**scenarios** (이전 dogfood report 의 9 시나리오):
- 1.4.a scanned 한국어 (F1 / F2) → OCR text indexed + search hit.
- 1.4.b multi-scanned PDF (5+ files) → chunk_id collision 0 (Bug #3 fix verify).
- 1.4.c always_on=true → vector PDF page 도 dual-block OCR.
- 1.4.d valid_ratio_threshold variation (0.3 / 0.5 / 0.8) → mojibake / 정상 page 분류.
- 1.4.e min_char_count variation (5 / 20 / 100) → 짧은 page OCR 호출.
- 1.4.f DCTDecode-only skip (F6 FlateDecode / F7 CCITTFax) → warning + skip.
- 1.4.g force-reingest (`--force-reingest`) → OCR 재실행.
- 1.4.h cancel handle (Ctrl+C 또는 SIGINT) → graceful abort.
- 1.4.i v0.19 → v0.20 upgrade UX (parser_version 보존 + manual force-reingest 필요).

### §1.5 Code ingest (P10)

**Tier 1 (AST chunkers)** — Rust / Python / TS / JS / Go / Java / Kotlin / C / C++

**Config**:
```toml
[ingest.code]
max_file_bytes = 262144
max_file_lines = 5000
ast_chunk_max_lines = 200
```

**verify per lang**:
- `chunker_version` = `code-{lang}-ast-v1`.
- symbol path correct (file-scope nesting / module-path / class-method nesting).
- `extra_skip_globs` 동작.

**scenarios**:
- 1.5.a Rust crate (workspace + multiple modules) → mod / fn / impl chunks.
- 1.5.b Python package (src/ layout, dotted module path).
- 1.5.c TypeScript/JavaScript (decorators, generators, classes).
- 1.5.d Go (package + struct methods).
- 1.5.e Java/Kotlin (class + inner class + method).
- 1.5.f C (typedef alias unit, header).
- 1.5.g C++ (namespace::Class::method recursive).
- 1.5.h `.h` 파일 (C vs C++ syntax) → tree-sitter-c parse 실패 시 Tier 3 fallback.

**Tier 2 (resource-aware)** — yaml/k8s, dockerfile, manifest (toml/json/xml/groovy)

**verify**:
- k8s YAML: `apiVersion+kind` per resource.
- Dockerfile: whole-file `dockerfile-file-v1`.
- Cargo.toml / package.json / pom.xml: whole-file `manifest-file-v1`.

**scenarios**:
- 1.5.i k8s manifest (multi-resource via `---`).
- 1.5.j Dockerfile (multi-stage build).
- 1.5.k Cargo.toml workspace (members + dependencies).
- 1.5.l invalid YAML → Tier 3 fallback.

**Tier 3 (paragraph fallback)** — shell, non-k8s YAML, AST failure

**verify**:
- `chunker_version = "code-text-paragraph-v1"`.
- Blank-line paragraph segmentation + 80-line / 20-overlap window for oversize.

**scenarios**:
- 1.5.m `.sh` / `.bash` / `.zsh` → paragraph chunks.
- 1.5.n empty file → 0 chunks + warning.
- 1.5.o very long shell script (1000+ lines) → line-window split.

### §1.6 Single-file / stdin ingest (p9-fb-31)

```bash
# Workspace 외부 file
"$RELEASE_BIN" ingest-file ~/Documents/external.md --config "$DOGFOOD/config.toml"
# 기대: _external/<hash>.md 에 copy + ingest.

# stdin
echo "# stdin content" | "$RELEASE_BIN" ingest-stdin --title "from stdin" --config "$DOGFOOD/config.toml"
```

**scenarios**:
- 1.6.a external markdown ingest.
- 1.6.b stdin with `--source-uri` flag.
- 1.6.c .kebabignore matched file → warn + 진행.
- 1.6.d binary file → reject (markdown only).

### §1.7 Incremental ingest

**verify**:
- 첫 ingest 후 다시 ingest → 모두 `unchanged`.
- 일부 file 수정 후 → 해당 file 만 `updated`.
- 새 file 추가 → `new`.
- 삭제된 file → `purged_deleted_files` count.

**scenarios**:
- 1.7.a unchanged path (parser_version + chunker_version + embedding_version match).
- 1.7.b stale file purge.
- 1.7.c `--force-reingest` (force-update path).

### §1.8 Ingest progress (wire `ingest_progress.v1`)

**`--json` mode 의 ndjson stream**:
- `scan_started` → `scan_completed` → `(asset_started → [pdf_ocr_*]* → asset_finished)+` → `completed` | `aborted`.

**verify**:
- ordering invariant (design §2.4a).
- per-asset `idx/total/path/media/result/chunks`.
- aggregate `counts` on `completed` / `aborted`.

---

## §2 Search

### §2.1 Lexical search (P2)

```bash
"$RELEASE_BIN" search --config "$DOGFOOD/config.toml" "한국어" --mode lexical --k 10
```

**verify**:
- FTS5 `unicode61` + lindera ko-dic 형태소 분해 column (v0.20.1, V009 migration).
- `chunks_fts` schema (`text`, `heading_path` 별 column) — V009 의 chunks_ai/au trigger 가 `tokenized_korean_text` 를 CASE expression 으로 raw text 앞에 prepend.
- 한국어 2-char query (`한국`, `서울`) 가 chunk 의 ko-dic 분해된 morpheme 또는 explicit 공백 분리된 token 과 일치 시 hit.
- 영어는 V002 의 whole-token 매칭으로 회귀 (`token` query 는 `token` 토큰만 hit, `tokenizer` substring 은 hit X). substring recall 이 필요하면 vector/hybrid mode 권장.

**scenarios**:
- 2.1.a Korean 2-char query (`한국`, `서울` → ≥ 1 hit on Korean wiki fixture).
- 2.1.b Korean compound noun (`한국어`, `서울특별시` → ko-dic 의 형태소 분해 + 단일 noun 동시 매칭).
- 2.1.c English/Korean mixed (`Rust 최적화` → token-AND 두 토큰 모두 hit).
- 2.1.d 1-char query → 0 hit (MIN_QUERY_CHARS = 2 filter, `build_match_string` v0.20.1 갱신).
- 2.1.e English whole-token (`tokenizer` hit, `token` 은 `tokenizer` 의 substring 매칭 X — V007 trigram 회귀).
- 2.1.f raw mode escape (`heading_path : <token>`).
- 2.1.g FTS5 phrase query (`"specific phrase"`).
- 2.1.h exclusion (`-token`).

### §2.1bis V009 morphological tokenizer dogfood evidence (v0.20.1)

**Reference corpus** (이 fixture 로 ingest 시 모든 scenario 보장 hit):

```bash
mkdir -p $DOGFOOD/corpus
cat > $DOGFOOD/corpus/korea-overview.md <<'EOF'
# 한국 개요

한국 은 동아시아 의 반도 국가다. 한국 어 는 한반도 의 주요 언어다.
서울 은 한국 의 수도다. 서울 의 지하철 은 1974년 1호선 개통 후
지금까지 23개 노선으로 확장되었다.

## 한국 문화

한국 의 문화 는 오래 된 역사 와 깊은 전통 을 가진다.
EOF

cat > $DOGFOOD/corpus/korea-compound.md <<'EOF'
# 한국어 와 한국문화

한국어 학습 자료. 한국문화 의 핵심 은 정 (情) 이다.
서울특별시 와 부산광역시 는 한국 의 양대 도시다.
EOF
```

**검증 명령** (모두 hit ≥ 1):

```bash
KB="$RELEASE_BIN --config $DOGFOOD/config.toml"

# 한국어 2-char (Bug #8 close, v0.20.1 의 핵심 가치)
$KB search '한국' --mode lexical --json | jq '.hits | length'   # ≥ 1
$KB search '서울' --mode lexical --json | jq '.hits | length'   # ≥ 1

# 한국어 3-char + compound noun
$KB search '지하철' --mode lexical --json | jq '.hits | length' # ≥ 1
$KB search '한국어' --mode lexical --json | jq '.hits | length' # ≥ 1
$KB search '한국문화' --mode lexical --json | jq '.hits | length' # ≥ 1
$KB search '서울특별시' --mode lexical --json | jq '.hits | length' # ≥ 1

# 영어 whole-token (V002 동작 회귀)
$KB search 'token' --mode lexical --json | jq '.hits | length'   # 0 또는 별 token 으로 존재 시 hit
$KB search 'tokenizer' --mode lexical --json | jq '.hits | length' # ≥ 1 if corpus has 'tokenizer' word
```

**예상 snippet (lindera 분해 evidence)**:
- `'한국'` query → "한국 은 동아시아 의 반 도 국가 다" — ko-dic 의 명사 boundary + 조사 분리 확인.
- `'서울'` query → "서울특별시 와" — ko-dic 의 compound `서울특별시` → `[서울, 특별시]` 분해.

**Known limitation (spec critic R1 #3 acceptance, Option α)**:
- ko-dic 이 compound noun 을 단일 token 으로 저장하는 경우 (예: `한국정부` 가 한 token) → `'한국'` query 는 그 chunk 에 hit X.
- KB 가 영어/code 위주 (예: 사용자 KnowledgeBase 가 React docs) 면 한국어 token 자체 부재로 0 hit 정상.
- N-gram supplement (Option β) 는 v0.21.x P9 follow-up.

### §2.2 Vector search (P3)

```bash
"$RELEASE_BIN" search --config "$DOGFOOD/config.toml" "어떤 문장의 의미적 검색" --mode vector --k 10
```

**verify**:
- `multilingual-e5-small` (384d) or `bge-m3` embedding.
- LanceDB 의 model 별 separate table.
- similarity score normalized.

**scenarios**:
- 2.2.a Korean semantic query.
- 2.2.b English semantic query.
- 2.2.c domain-specific (code semantic).
- 2.2.d cross-lingual (한영 mixed query).

### §2.3 Hybrid search (RRF, P3)

```bash
"$RELEASE_BIN" search --config "$DOGFOOD/config.toml" "query" --mode hybrid --k 10
```

**verify**:
- `fusion_score = [0, 1]` (normalize).
- lexical + vector → RRF fusion.

**scenarios**:
- 2.3.a hybrid 의 lexical-only / vector-only 가 못 잡는 case (RRF win).
- 2.3.b fusion_score ordering.

### §2.4 Search filters (p9-fb-36)

```bash
"$RELEASE_BIN" search --config "$DOGFOOD/config.toml" "query" --tag rust --tag api --lang en --path-glob 'src/**'
```

**verify**:
- `--tag` repeatable (OR within).
- `--lang` ISO code.
- `--path-glob` workspace_path glob.

### §2.4bis Source / provenance filters (`--source` / `--source-type`, v0.29.0)

```bash
# 출처 id 필터 ([[workspace.sources]] 의 id; 단일 root 는 "default").
"$RELEASE_BIN" search --config "$DOGFOOD/config.toml" "query" --source jira --json | jq '.hits | length'
# source_type 필터 (markdown/note/paper/reference/inbox).
"$RELEASE_BIN" search --config "$DOGFOOD/config.toml" "query" --source-type reference,markdown --json
```

**verify**:
- `--source` / `--source-type` repeatable + comma-sep, OR within.
- lexical · vector · hybrid 모든 모드에 동일 적용 (직접 인덱스 컬럼 `documents.source_id` / `source_type`).
- 모르는 값 → silently empty (no error).
- 멀티소스 KB 측정: `--source wiki` 가 개념 질의 오염 회복(MRR 0.780→0.810), `--source jira` 가 incident 0.918→0.975 (HOTFIXES 2026-06-21).
- trust precedence: `[[workspace.sources]]` 의 per-source `trust_level` 가 frontmatter 부재 시 적용 → `--trust-min primary` 와 조합 시 secondary source 배제.

### §2.5 Search pagination (p9-fb-34)

```bash
"$RELEASE_BIN" search "query" --max-tokens 1000 --snippet-chars 200 --json | jq '.next_cursor'
"$RELEASE_BIN" search "query" --cursor "$(...)" --json
```

**verify**:
- `next_cursor` opaque token.
- `corpus_revision` mismatch → `stale_cursor` error.

### §2.6 Search cache (p9-fb-19)

```bash
"$RELEASE_BIN" search "query" --json   # first call
"$RELEASE_BIN" search "query" --json   # cached (in-process LRU, no-op in CLI)
"$RELEASE_BIN" search "query" --no-cache --json   # force fresh
```

### §2.7 Bulk search

stdin ndjson — 줄당 하나의 query object (`{"query":"<text>"}` 필수, 나머지 optional):
```bash
printf '%s\n' \
  '{"query":"한국","mode":"lexical","k":3}' \
  '{"query":"tokenizer","mode":"hybrid"}' \
  '{"query":"lindera","mode":"vector","k":5}' \
  | "$RELEASE_BIN" search --bulk --json
```
기대: 줄당 `bulk_search_item.v1` (query echo + response 또는 error). `query` 누락 시 그 item 만 `error.v1` (code `invalid_input`, message 에 shape hint), 나머지 query 계속 진행. Cap 100.

---

## §3 Ask (RAG)

### §3.1 Basic ask (P4)

```bash
"$RELEASE_BIN" ask --config "$DOGFOOD/config.toml" "어떤 질문" --json
```

**verify**:
- `grounded` field (boolean).
- `citations` list (chunk references).
- `answer` text.

**scenarios**:
- 3.1.a in-corpus question (grounded=true).
- 3.1.b out-of-corpus question (grounded=false + refusal).
- 3.1.c hallucination check (paraphrase test, fb-41).

### §3.6 응답 언어 자동 매칭 (v0.20.2 Todo #1)

```bash
"$RELEASE_BIN" ask --config "$DOGFOOD/config.toml" "What is the tokenizer?" --hide-citations  # 영어 응답 기대
"$RELEASE_BIN" ask --config "$DOGFOOD/config.toml" "토크나이저가 뭐야?" --hide-citations        # 한국어 응답 기대
```

기대: query 언어 = response 언어 (`prompt_template_version = "rag-v4"` default). 큰따옴표 직접 인용은 원문 언어 보존. citation `[#번호]` 유지. 한국어 corpus 를 영어로 물으면 LLM 이 근거를 영어로 번역해 답함 (trade-off). `rag-v3` 로 pin 하면 legacy (provenance 라벨 discount 없음) 동작.

### §3.2 Streaming ask (v0.17.1)

```bash
"$RELEASE_BIN" ask "query" --stream
```

**verify**:
- per-event `answer.v1` (delta tokens).
- final event with `verification` + `citations`.

### §3.3 Multi-hop RAG (fb-41 / v0.18.0)

```bash
"$RELEASE_BIN" ask "complex question" --multi-hop --json
```

**verify**:
- decompose → decide → synthesize loop.
- `multi_hop_max_depth` / `multi_hop_max_sub_queries_per_iter` 따름.

### §3.4 NLI verification (fb-41 / v0.18.0)

```toml
[models.nli]
model = "Xenova/mDeBERTa-v3-base-xnli-multilingual-nli-2mil7"

[rag]
nli_threshold = 0.5
```

**verify**:
- `verification.entailment_score`.
- `refusal_reason = "nli_verification_failed"` (low score).

**scenarios**:
- 3.4.a known hallucination case (S7 caffeine) → reject with nli_score < threshold.
- 3.4.b legitimate grounded answer → entailment > 0.5.
- 3.4.c NLI model unavailable → `refusal_reason = "nli_model_unavailable"`.

### §3.5 Ask filters

```bash
"$RELEASE_BIN" ask "query" --tag rust --path-glob 'src/**' --lang en
```

---

## §4 Inspect / Fetch / List

### §4.1 Inspect

```bash
"$RELEASE_BIN" inspect document <doc_id> --json
"$RELEASE_BIN" inspect chunk <chunk_id> --json
```

**verify**:
- `chunk_inspection.v1` schema.
- `canonical_document.parser_version` / `chunker_version`.

### §4.2 Fetch (p9-fb-35)

verbatim chunk / doc / span fetch:
```bash
"$RELEASE_BIN" fetch <chunk_id> --json
"$RELEASE_BIN" fetch <doc_id> --span 12-34 --json
```

**verify**:
- `fetch_result.v1` schema.
- verbatim text (no chunk wrapping).

### §4.3 List

```bash
"$RELEASE_BIN" list documents --json
"$RELEASE_BIN" list chunks --json
```

---

## §5 Version cascade

design §9 cascade rule:
- `parser_version` 변경 → 해당 parser 의 모든 chunk 무효.
- `chunker_version` 변경 → 해당 chunker 의 모든 chunk 무효.
- `embedding_version` 변경 → 모든 embedding 무효.

**verify scenarios**:
- 5.a parser_version bump (예: `pdf-text-v1` → `pdf-text-v2`) → 자동 invalidation + 다음 ingest 가 재처리.
- 5.b chunker_version bump (예: v0.20.1 의 `pdf-page-v1` → `pdf-page-v1.1`) → chunk_id 재계산.
- 5.c embedding_version bump (예: `multilingual-e5-small/v1` → `/v2`) → LanceDB 의 별 table.
- 5.d 동일 asset 의 doc_id 다른 case → `purge_workspace_path_for_parser_bump` cascade.
- 5.e force-reingest 의 user-facing UX.

---

## §6 Wire schema (`--json`)

### §6.1 schemas list

`docs/wire-schema/v1/`:
- `ingest_progress.v1`, `ingest_report.v1`
- `search_hit.v1`, `search_response.v1`, `bulk_search_item.v1`, `bulk_search_response.v1`
- `answer.v1`, `answer_event.v1`
- `chunk_inspection.v1`, `citation.v1`, `doc_summary.v1`
- `doctor.v1`, `schema.v1`, `fetch_result.v1`, `reset_report.v1`
- `error.v1`

### §6.2 verify per schema

각 schema:
```bash
$RELEASE_BIN <subcommand> --json | jq '.schema_version'
# 기대: "<schema>.v1"
```

JSON schema validity:
```bash
jq -e 'has("schema_version")' <output>
ajv-cli validate -s docs/wire-schema/v1/<schema>.schema.json -d <output>
```

### §6.3 wire backward-compat

**verify**:
- additive minor (new field / new enum value) → older consumer 가 graceful.
- breaking change (field removal / type change) → `v2` major bump.

### §6.4 schema_version cascade

`schema.v1` (`kebab schema --json`) output 의 `wire_schemas` field:
- 16+ entry 의 `{name, version, capabilities}`.

---

## §7 TUI (P9)

### §7.1 Launch TUI

```bash
"$RELEASE_BIN" tui --config "$DOGFOOD/config.toml"
```

### §7.2 4 panel verify

- **P9-1 Library**: workspace document tree + recent assets.
- **P9-2 Search**: lexical / vector / hybrid search panel.
- **P9-3 Ask**: question + answer pane + citations.
- **P9-4 Inspect**: chunk / document detail.

### §7.3 keyboard shortcuts

- Tab / Shift+Tab — switch panel.
- Esc — cancel ongoing op.
- q — quit.
- 기타 panel-specific.

### §7.4 scenarios

- 7.4.a library tree navigation.
- 7.4.b search query + result selection → fetch.
- 7.4.c ask question + answer + citation click → inspect.
- 7.4.d cancel mid-ingest (Esc).
- 7.4.e quit + restart → state preserve.

---

## §8 MCP (P9-fb-30)

### §8.1 Launch MCP stdio server

```bash
"$RELEASE_BIN" mcp
# (agent host 가 stdio 로 호출)
```

### §8.2 6 MCP tools

- `search`, `ask`, `schema`, `doctor`, `ingest_file`, `ingest_stdin`.

### §8.3 verify per tool

- input schema (JSON Schema).
- output schema (wire `*.v1`).
- error path (graceful, exit code).

---

## §9 Doctor / Schema / Reset

### §9.1 Doctor (health check)

```bash
"$RELEASE_BIN" doctor --json
```

**verify**:
- `doctor.v1` schema.
- workspace / storage / models / ollama_reachability.

### §9.2 Schema (introspection report)

```bash
"$RELEASE_BIN" schema --json
```

**verify**:
- `schema.v1` schema.
- wire_schemas / capabilities / model_versions / stats.

### §9.3 Reset

```bash
"$RELEASE_BIN" reset --yes
"$RELEASE_BIN" reset --data-only --yes
"$RELEASE_BIN" reset --vector-only --yes
```

**verify**:
- XDG data dirs wipe (irreversible — TTY confirm 또는 `--yes`).
- macOS path collision 회피 (HOTFIXES 2026-05-07).
- `--data-only` 가 config 보호.

---

### config migrate (스키마 마이그레이션, v0.21.1)

```bash
# 옛 스키마 흉내(섹션 누락 + deprecated) 후 migrate.
printf 'schema_version = 1\n\n[workspace]\nroot = "~/MyNotes"\ninclude = ["*.md"]\n\n[search]\ndefault_k = 25\n' \
  > "$DOGFOOD/old.toml"
"$RELEASE_BIN" --config "$DOGFOOD/old.toml" config migrate --dry-run    # 미리보기, 파일 미수정
"$RELEASE_BIN" --config "$DOGFOOD/old.toml" config migrate              # .bak + 빠진 섹션 주석과 함께 추가
"$RELEASE_BIN" --config "$DOGFOOD/old.toml" config migrate              # 멱등
"$RELEASE_BIN" --config "$DOGFOOD/old.toml" doctor | grep config_migration  # ok 확인
```

기대: dry-run 파일 미수정 → apply 시 `old.toml.bak`(원본 byte-identical) + `[ingest.code]`·`[logging]`·`[pdf.ocr]` 가시화 + 손본 `default_k`/주석 보존 + `workspace.include` 제거 → 재실행 멱등 → doctor `config_migration` ok. v0.21.1 evidence 는 `tasks/HOTFIXES.md` 2026-05-31.

## §10 Eval (P5)

### §10.1 Basic eval run

```bash
KEBAB_EVAL_GOLDEN=/build/dogfood/golden_queries.yaml \
  "$RELEASE_BIN" --config "$DOGFOOD/config.toml" eval run --mode hybrid --k 10
```

**verify**:
- golden query suite 의 metrics (MRR / Recall / NDCG).
- regression detection (snapshot 비교).
- `eval aggregate <run_id> --json` 로 metric object 확인.

### §10.2 검색 품질 baseline (v0.20.2 golden suite, spec §4.6)

v0.20.2 dogfood 에서 확립한 baseline. eval `--config` facade 패치로 dogfood KB 를 직접 평가할 수 있게 됨.

**실행 절차**:

```bash
# 1. eval run (hybrid + lexical 각각)
KEBAB_EVAL_GOLDEN=/build/dogfood/golden_queries.yaml \
  "$RELEASE_BIN" --config /build/dogfood/config.toml eval run --mode hybrid --k 10 --json \
  | tee /build/dogfood/logs/eval-hybrid-$(date +%Y%m%d).json

KEBAB_EVAL_GOLDEN=/build/dogfood/golden_queries.yaml \
  "$RELEASE_BIN" --config /build/dogfood/config.toml eval run --mode lexical --k 10 --json \
  | tee /build/dogfood/logs/eval-lexical-$(date +%Y%m%d).json

# 2. aggregate
RUN_ID=$(jq -r '.run_id' /build/dogfood/logs/eval-hybrid-$(date +%Y%m%d).json | head -1)
"$RELEASE_BIN" --config /build/dogfood/config.toml eval aggregate "$RUN_ID" --json
```

**v0.20.2 metric baseline** (`/build/dogfood/golden_queries.yaml` 10 query):

| Mode | hit@1 | hit@3 | hit@10 | MRR | recall@10 | empty |
|------|-------|-------|--------|-----|-----------|-------|
| hybrid | 0.7 | **1.0** | 1.0 | **0.833** | 1.0 | 0 |
| lexical | 0.4 | 1.0 | 1.0 | 0.7 | 1.0 | 0 |

**정성 체크리스트**:
- [ ] 한국어 2자 정답 (`'한국'` / `'서울'` 등) 이 hit@3 이내에 등장.
- [ ] `empty_result_rate = 0` — 10개 query 전부 ≥ 1 hit.
- [ ] hybrid MRR ≥ 0.8 (baseline 0.833).
- [ ] lexical MRR ≥ 0.65 (baseline 0.7).
- [ ] `eval compare <run_a> <run_b>` 의 delta MRR 이 ±0.1 이내면 ranking 건강.

**큐레이션 절차 (spec §4.6)**:
- golden answer 는 "note 의 intent 와 가장 가까운 chunk" 가 아닌 "합리적으로 관련된 모든 doc" 포함 권장.
- 코드와 note 가 동시에 정답일 수 있음 — eval 분해 후 vector hit 를 직접 확인해 golden 보완.
- 초기 라벨링 후 `eval aggregate --json` 의 per-query breakdown 으로 false-negative 정정.
- 정정 시 `hit@3` 등 상위 metric 이 0.9→1.0 수준으로 개선되면 curated golden 으로 확정.

**인사이트**:
- hybrid 가 vector 덕분에 top-1 정확도 우위 (0.7 vs lexical 0.4). hit@3 이후는 두 모드 모두 완벽.
- lexical (V009 형태소) 이 짧은 한국어 토큰을 top-3 에 정확히 배치.
- ranking 조정 없이 현재 hybrid RRF 가 baseline 달성 (`[[project_ranking_deferred]]` 결정 유효).

Cross-link: `tasks/HOTFIXES.md` (2026-05-29 — 검색 품질 baseline entry), `/build/dogfood/golden_queries.yaml`, `/build/dogfood/logs/`.

---

## §11 Edge cases

### §11.1 Encrypted PDF

- input: thermal-pos-printer.pdf / thermal-label.pdf (사용자 dogfood corpus).
- expected: `kind: "Error"` + friendly wording (`qpdf --decrypt` hint).

### §11.2 Corrupt file

- input: truncated bytes / invalid magic.
- expected: `kind: "Error"` + graceful error.

### §11.3 Empty file

- input: 0-byte file.
- expected: 0 chunks + warning.

### §11.4 Very large file

- markdown 100 MB+, PDF 100 MB+.
- expected: walker pass (per file type limit), parser graceful.

### §11.5 env variable overrides

```bash
KEBAB_PDF_OCR_ENABLED=true \
    KEBAB_PDF_OCR_MODEL=qwen2.5vl:7b \
    "$RELEASE_BIN" ingest --config "$DOGFOOD/config.toml"
```

**verify per env**:
- `KEBAB_PDF_OCR_*` (11 env, v0.20.0).
- `KEBAB_IMAGE_OCR_*` (P6).
- `KEBAB_MODELS_LLM_*`, `KEBAB_MODELS_EMBEDDING_*`.
- `KEBAB_READONLY` (write-path subcommand 차단).

### §11.6 Cancel handle

- 대형 PDF (metro-korea 58MB) ingest 도중 SIGINT (Ctrl+C).
- expected: graceful abort + `IngestEvent::Aborted` + partial counts.

### §11.7 `--readonly` mode

```bash
KEBAB_READONLY=1 "$RELEASE_BIN" ingest  # 기대: refuse
KEBAB_READONLY=1 "$RELEASE_BIN" search "query"  # 기대: OK
```

### §11.8 `--quiet` mode

```bash
"$RELEASE_BIN" ingest --quiet  # stderr 0
"$RELEASE_BIN" ingest --json  # implies quiet
```

### §11.9 `.kebabignore`

```text
# In workspace root
node_modules/
*.tmp
draft/**
```

**verify**:
- ignore patterns 정확히 적용.
- per-directory `.kebabignore` cascading.

### §11.10 Config edge cases

- missing config → fall back to XDG default.
- malformed TOML → `error.v1` with `config_invalid`.
- unknown field → tolerant (forward-compat) 또는 strict (TBD).
- workspace.root path 변수 (`~`, `${XDG_DATA_HOME}`, relative path) 모두 동작.

### §11.11 Concurrent access

- 동일 KB 의 multiple ingest 동시 실행 → SQLite lock 또는 graceful queue.
- ingest 중 search → consistent snapshot.

### §11.12 macOS path collision (HOTFIXES 2026-05-07)

- `config_dir()` ≠ `data_dir()` 보장.
- legacy `~/Library/...` path 의 자동 migration.

---

## §12 Bug discovery checklist

새 feature 또는 새 release 마다 본 checklist 실행:

### §12.1 Pre-flight
- [ ] release binary build clean.
- [ ] Ollama endpoint reachable + 사용 model pull.
- [ ] isolated KB workspace 분리.
- [ ] config.toml = default + minimal customize.

### §12.2 Ingest path
- [ ] 각 media type 의 baseline (markdown / image / pdf / code / Tier2/3).
- [ ] empty / corrupt / encrypted file.
- [ ] very large file (size limit verify).
- [ ] incremental + force-reingest cycle.
- [ ] env var override.
- [ ] cancel handle mid-ingest.

### §12.3 Search path
- [ ] lexical (한국어 + 영어 + mixed).
- [ ] vector (semantic).
- [ ] hybrid (RRF).
- [ ] filter (tag/lang/path-glob).
- [ ] pagination cursor.
- [ ] bulk search.

### §12.4 Ask path
- [ ] in-corpus question (grounded=true).
- [ ] out-of-corpus question (grounded=false).
- [ ] streaming (`--stream`).
- [ ] multi-hop (`--multi-hop`).
- [ ] NLI verification (known hallucination case).

### §12.5 Surface verify
- [ ] CLI flags (각 subcommand 의 --help + actual behavior).
- [ ] TUI panel + keyboard shortcuts.
- [ ] MCP stdio (each tool).
- [ ] Wire schema (`--json` mode 의 schema_version + jq validity).

### §12.6 Version cascade
- [ ] parser_version bump → 자동 invalidation.
- [ ] chunker_version bump → chunk_id 재계산.
- [ ] embedding_version bump → LanceDB 별 table.

### §12.7 Edge cases
- [ ] env override (각 KEBAB_* env).
- [ ] readonly mode.
- [ ] quiet mode.
- [ ] .kebabignore.
- [ ] config edge cases.
- [ ] concurrent access.

### §12.8 Doc + Wire schema 정합
- [ ] README + HANDOFF + ARCHITECTURE 의 사용자 visible surface 일치.
- [ ] wire schema 의 actual emit field 와 schema.json 일치.
- [ ] error.v1 의 `code` 값이 실제 surface 와 일치.

### §12.9 Bug discovery → immediate fix cycle

사용자 명시: "아무리 작은거여도 발견되면 바로 이어서 그걸 수정하는 작업들을 하도록 해".

bug 발견 시:
1. **immediate stop dogfood** (현 scenario 중단).
2. **bug log** — `.omc/reviews/<date>-<sub-item>-dogfood-report.md` 에 다음 fields:
   - file:line of root cause.
   - reproduction command.
   - expected vs actual.
   - severity (Critical / Important / Minor).
3. **spec/plan/executor cycle** (size-adapted):
   - **small bug** (1-line fix / wording typo / config field rename): simplified spec (1 page) + plan (1-2 step) + executor.
   - **large bug** (cross-crate / wire schema / new invariant): full spec/plan/executor cycle (이전 v0.20-sub1-bugfix scale).
4. **fix verify** — workspace test + clippy + dogfood scenario 재확인.
5. **dogfood resume** from where it left off.

---

## §13 Reference dogfood corpus

### §13.1 PDF (9 file, PoC + sub-item 1)

| # | File | Size | Source | Use case |
|---|------|------|--------|----------|
| 1 | scanned_page1.pdf (F1) | 466 KB | `crates/kebab-parse-pdf/tests/fixtures/` | scanned 한국어 일반 (OCR alnum > 85%) |
| 2 | scanned_page2.pdf (F2) | 773 KB | `crates/kebab-parse-pdf/tests/fixtures/` | scanned 받침 intensive (OCR alnum > 70%) |
| 3 | metro-korea.pdf | 58 MB | `/build/cache/pdf-ocr-poc/fixtures/` | real-world 신문 multi-page vector PDF |
| 4 | mojibake.pdf (F4) | 23 KB | `crates/kebab-parse-pdf/tests/fixtures/` | vector PDF (Latin + no ToUnicode CMap) |
| 5 | flate_raw.pdf (F6) | 872 B | `crates/kebab-parse-pdf/tests/fixtures/` | FlateDecode skip path |
| 6 | ccitt.pdf (F7) | 2 KB | `crates/kebab-parse-pdf/tests/fixtures/` | CCITTFax skip path |
| 7 | thermal-pos-printer.pdf | 1.1 MB | `~/paperboy/` | ENG manual PDF (encrypted) |
| 8 | thermal-label.pdf | 2.7 MB | `~/paperboy/` | ENG manual PDF (encrypted) |
| 9 | internals-presentation.pdf | 820 KB | `~/namu-crawler/docs/` | slide deck PDF (vector text) |

### §13.2 PoC ground-truth (alnum e2e)

`/build/cache/pdf-ocr-poc/ground-truth/`:
- `page1.txt` (1489 byte) — F1 ground-truth.
- `page2-batchim.txt` (3282 byte) — F2 ground-truth (받침 intensive).

### §13.3 Markdown / code corpus

(향후 sub-item 별 추가 — `~/Documents/notes/` 또는 별 prepared fixture).

### §13.4 Image corpus

(P6 dogfood — 향후 추가).

---

## 부록 A — 본 doc 의 갱신 정책

- 새 release / sub-item 머지 시 §1 (Ingest) 또는 §2-§9 (관련 section) 갱신.
- 새 dogfood report (`.omc/reviews/<date>-<sub-item>-dogfood-report.md`) 의 scenarios 가 본 doc 의 표준 시나리오 와 align.
- 본 doc 의 시나리오 자체에 새 scenarios 추가 시 PR 안 §12 checklist update.
- HOTFIXES.md 의 deviation 발견 시 본 doc 의 관련 section 의 verify step 강화.

## 부록 B — Reviewer 의 dogfood verify

PR review 시 reviewer 가 본 doc 의 §12 checklist 참조:
- 새 feature 의 변경 surface (CLI flag / wire schema / config field) 가 §1-§9 의 시나리오 cover 하는지.
- new bug discovery 시 §12.9 의 immediate fix cycle 적용.
