---
title: "로컬 Knowledge Base 구축 최종 보고서"
subtitle: "Rust 2024, 단일 repo, 함수 호출 기반 모듈러 모놀리스 설계"
author: "ChatGPT"
date: "2026-04-27"
lang: ko-KR
geometry: margin=22mm
fontsize: 10.5pt
colorlinks: true
linkcolor: blue
urlcolor: blue
---

# 0. 이 보고서의 결론

당신이 만들려는 것은 HTTP API로 분리된 MSA가 아니라, **하나의 Rust 2024 workspace 안에 여러 crate를 둔 로컬-first 모듈러 모놀리스 knowledge base**다. 사용자는 당신 한 명이고, 1등급 타겟 하드웨어는 **M4 48GB MacBook**이다. 따라서 설계의 중심은 클라우드 확장성이나 다중 사용자 인증이 아니라, **원본 보존, 재현 가능한 인덱싱, 안정적인 모듈 계약, 로컬 LLM 연동, 좋은 검색 품질, citation 추적성**이어야 한다.

최종 방향은 다음 한 문장으로 요약할 수 있다.

> Markdown을 1등급 지식 소스로 삼고, 이미지, PDF, 음성은 각각 extractor adapter를 통해 동일한 `CanonicalDocument -> Chunk -> Embed -> Index -> Search -> RAG` 파이프라인으로 흘려보낸다. CLI, TUI, desktop app은 모두 같은 `kb-app` facade를 함수 호출로 사용한다.

가장 먼저 만들 것은 채팅 UI가 아니다. 먼저 만들어야 할 것은 다음 7가지다.

1. `kb-core`의 도메인 모델과 trait 계약
2. deterministic ID 규칙
3. Markdown canonicalization
4. chunking policy
5. SQLite metadata/FTS 저장
6. LanceDB 또는 대체 embedded vector store 연동
7. citation과 source span 보존

이 7개가 안정되면 local LLM, TUI, desktop app, image/PDF/audio support는 단계적으로 붙이면 된다. 반대로 이 7개 없이 LLM 채팅부터 만들면, 나중에 데이터 종류가 늘어날 때 전체를 다시 설계하게 될 가능성이 높다.

# 1. 전제와 비목표

## 1.1 전제

- 사용자는 한 명이다.
- 로컬 LLM을 주로 쓴다.
- 1등급 하드웨어는 M4 48GB MacBook이다.
- 언어는 Rust 2024를 선호한다.
- HTTP API 기반 MSA가 아니라 함수 호출 기반의 단일 repo 프로젝트다.
- Markdown 문서가 1등급 문서 소스다.
- 추후 입력 범위는 이미지, PDF, 음성 순으로 확장한다. 단, 텍스트 PDF support는 구현 난이도상 이미지와 병렬 또는 선행될 수 있다.
- 추후 TUI와 desktop app을 붙인다.

## 1.2 비목표

초기 버전에서 다음은 목표가 아니다.

- 다중 사용자 SaaS
- Kubernetes 배포
- 원격 vector DB 운영
- enterprise RBAC/ABAC
- 실시간 협업 편집
- 모든 파일 포맷의 완벽한 parsing
- agent가 임의로 파일을 수정하는 자동화

초기 목표는 **개인 로컬 지식 저장소**다. 따라서 단순하고 재현 가능한 구조가 가장 중요하다.

# 2. 핵심 아키텍처

전체 구조는 다음과 같다.

```text
Markdown files
   |
   v
kb-source-fs
   |
   v
kb-parse-md
   |
   v
CanonicalDocument
   |
   v
kb-chunk
   |
   v
Chunks
   |
   +--------------------+--------------------+
   |                    |                    |
   v                    v                    v
SQLite metadata/FTS     LanceDB vectors      Raw asset store
   |                    |                    |
   +--------------------+--------------------+
                        |
                        v
                    kb-search
                        |
                        v
                     kb-rag
                        |
        +---------------+---------------+
        |               |               |
        v               v               v
      kb-cli          kb-tui       kb-desktop
```

추후 확장 후 구조는 다음과 같다.

```text
Markdown ----+
Image -------+
PDF ---------+--> Extractor adapters --> CanonicalDocument
Audio -------+                              |
                                             v
                                           Chunk
                                             |
                                             v
                                      Embed / Index
                                             |
                                             v
                                      Search / RAG
                                             |
                                             v
                                  CLI / TUI / Desktop
```

핵심은 모든 입력을 결국 같은 canonical model로 변환한다는 점이다. Markdown 전용 검색, 이미지 전용 검색, PDF 전용 검색을 따로 만들면 장기적으로 유지보수가 어려워진다.

# 3. 왜 Rust 2024 workspace인가

Rust 2024에서는 `edition = "2024"`가 Cargo resolver 3을 의미하며, workspace의 의존성 해석에도 영향을 준다. 공식 Edition Guide는 Rust 2024에서 rust-version aware dependency resolver가 기본이 된다고 설명한다. [Rust Edition Guide - Cargo resolver](https://doc.rust-lang.org/edition-guide/rust-2024/cargo-resolver.html)

Cargo workspace는 여러 package를 함께 관리하는 구조이며, 공통 `Cargo.lock`, 공통 `target` directory, `cargo check --workspace` 같은 공통 명령을 제공한다. 이 특성은 당신이 말한 “작은 프로젝트들의 집합체”를 하나의 repo 안에서 관리하는 데 적합하다. [Cargo Book - Workspaces](https://rustwiki.org/en/cargo/reference/workspaces.html)

권장 root `Cargo.toml`은 다음과 같다.

```toml
[workspace]
resolver = "3"
members = [
  "crates/kb-core",
  "crates/kb-config",
  "crates/kb-source-fs",
  "crates/kb-parse-md",
  "crates/kb-normalize",
  "crates/kb-chunk",
  "crates/kb-store-sqlite",
  "crates/kb-store-vector",
  "crates/kb-embed",
  "crates/kb-embed-local",
  "crates/kb-search",
  "crates/kb-llm",
  "crates/kb-llm-local",
  "crates/kb-rag",
  "crates/kb-eval",
  "crates/kb-app",
  "crates/kb-cli"
]

[workspace.package]
edition = "2024"
rust-version = "1.85"
license = "MIT OR Apache-2.0"

[workspace.dependencies]
anyhow = "1"
thiserror = "2"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
time = { version = "0.3", features = ["serde"] }
uuid = { version = "1", features = ["v7", "serde"] }
blake3 = "1"
tracing = "0.1"
```

# 4. Repo 구조

초기 repo는 이렇게 잡는다.

```text
kb/
  Cargo.toml
  README.md
  docs/
    spec/
      domain-model.md
      ids.md
      canonical-document.md
      chunk-policy.md
      citation-policy.md
      module-boundaries.md
      ai-generation-guidelines.md
  fixtures/
    markdown/
      simple-note.md
      nested-headings.md
      code-and-table.md
  crates/
    kb-core/
    kb-config/
    kb-source-fs/
    kb-parse-md/
    kb-normalize/
    kb-chunk/
    kb-store-sqlite/
    kb-store-vector/
    kb-embed/
    kb-embed-local/
    kb-search/
    kb-llm/
    kb-llm-local/
    kb-rag/
    kb-eval/
    kb-app/
    kb-cli/
```

나중에 추가할 crate는 다음과 같다.

```text
crates/kb-parse-image/
crates/kb-parse-pdf/
crates/kb-parse-audio/
crates/kb-rerank/
crates/kb-tui/
crates/kb-desktop/
```

중요한 의존성 규칙은 다음과 같다.

```text
kb-cli, kb-tui, kb-desktop
    -> kb-app
        -> kb-index / kb-search / kb-rag
            -> kb-core traits
            -> concrete adapters
```

UI crate는 절대로 parser, DB, LLM adapter를 직접 호출하지 않는다. 모든 user-facing command는 `kb-app` facade를 통해 호출한다.

# 5. 컴포넌트 목록과 책임

| 컴포넌트 | 책임 | 초기 구현 |
|---|---|---|
| `kb-core` | domain type, trait, error, ID 규칙 | 필수 |
| `kb-config` | config 파일 로딩, 기본값, 경로 확장 | 필수 |
| `kb-source-fs` | 로컬 폴더 scan, checksum, 변경 감지 | 필수 |
| `kb-parse-md` | Markdown -> structured document | 필수 |
| `kb-normalize` | parser output -> `CanonicalDocument` | 필수 |
| `kb-chunk` | block-aware chunking | 필수 |
| `kb-store-sqlite` | metadata, document, chunk, job, FTS | 필수 |
| `kb-store-vector` | vector upsert/search | P1 |
| `kb-embed` | embedding trait | P1 |
| `kb-embed-local` | local embedding adapter | P1 |
| `kb-search` | lexical, vector, hybrid retrieval | P1 |
| `kb-llm` | language model trait | P1 |
| `kb-llm-local` | Ollama 또는 llama.cpp adapter | P1 |
| `kb-rag` | context packing, answer, citation | P1 |
| `kb-eval` | golden query, regression test | P1 |
| `kb-cli` | command line interface | 필수 |
| `kb-tui` | terminal UI | P2 |
| `kb-desktop` | desktop app | P3 |

# 6. 핵심 도메인 모델

## 6.1 RawAsset

원본 파일을 나타낸다. 원본은 절대 파기하지 않는다.

```rust
pub struct RawAsset {
    pub asset_id: AssetId,
    pub source_uri: SourceUri,
    pub media_type: MediaType,
    pub byte_len: u64,
    pub checksum: Checksum,
    pub discovered_at: OffsetDateTime,
}
```

## 6.2 CanonicalDocument

모든 입력 포맷이 도달해야 하는 공통 문서 표현이다.

```rust
pub struct CanonicalDocument {
    pub doc_id: DocumentId,
    pub source_asset_id: AssetId,
    pub title: String,
    pub lang: Lang,
    pub blocks: Vec<Block>,
    pub metadata: Metadata,
    pub provenance: Provenance,
}
```

## 6.3 Block

Markdown heading, paragraph, code, table, image reference 등을 구조적으로 보존한다.

```rust
pub enum Block {
    Heading(HeadingBlock),
    Paragraph(TextBlock),
    List(ListBlock),
    Code(CodeBlock),
    Table(TableBlock),
    Quote(TextBlock),
    ImageRef(ImageRefBlock),
    AudioRef(AudioRefBlock),
}
```

## 6.4 Chunk

검색의 최소 단위다. chunk는 텍스트뿐 아니라 source span을 반드시 가진다.

```rust
pub struct Chunk {
    pub chunk_id: ChunkId,
    pub doc_id: DocumentId,
    pub block_ids: Vec<BlockId>,
    pub text: String,
    pub heading_path: Vec<String>,
    pub source_spans: Vec<SourceSpan>,
    pub token_estimate: usize,
    pub chunker_version: String,
}
```

## 6.5 SearchHit

검색 결과는 반드시 citation으로 연결되어야 한다.

```rust
pub struct SearchHit {
    pub chunk_id: ChunkId,
    pub doc_id: DocumentId,
    pub score: f32,
    pub text: String,
    pub citation: Citation,
}
```

# 7. ID와 versioning 규칙

초기부터 deterministic ID를 잡아야 한다. 그래야 parser, chunker, embedding model이 바뀌어도 어떤 산출물을 재생성해야 하는지 알 수 있다.

권장 규칙은 다음과 같다.

```text
asset_id     = blake3(raw bytes)
doc_id       = stable source path + asset hash + parser version
block_id     = doc_id + block path + source span
chunk_id     = doc_id + chunker version + block ids
embedding_id = chunk_id + embedding model id + dimension
index_id     = collection name + index version + embedding model id
```

각 record에는 최소한 다음 version을 남긴다.

```text
doc_version
schema_version
parser_version
chunker_version
embedding_model
embedding_version
index_version
prompt_template_version
```

이 정책 덕분에 “원본은 그대로 두고 파생물만 재생성”하는 구조가 가능해진다.

# 8. Markdown을 1등급 소스로 다루는 법

Markdown은 단순 문자열이 아니라 구조화된 문서다. Markdown parser는 다음을 보존해야 한다.

- YAML/TOML frontmatter
- heading tree
- paragraph
- list
- code block과 language tag
- table
- blockquote
- link
- image reference
- line range 또는 byte range

Rust Markdown parser 후보는 다음과 같다.

- `pulldown-cmark`: CommonMark pull parser이며 source-map 지원을 강조한다. [pulldown-cmark GitHub](https://github.com/pulldown-cmark/pulldown-cmark)
- `comrak`: CommonMark 및 GitHub Flavored Markdown 호환 parser/renderer다. [Comrak 공식 문서](https://comrak.ee/)

추천은 다음과 같다.

```text
초기: pulldown-cmark
GFM table/task list/복잡한 Markdown 호환성이 중요해지면: comrak 검토
```

Markdown frontmatter 기본 규약은 다음 정도로 시작한다.

```yaml
---
id: rust-kb-architecture
title: Rust 로컬 Knowledge Base 설계
aliases:
  - local kb
  - rust rag
tags:
  - knowledge-base
  - rust
  - rag
created_at: 2026-04-27
updated_at: 2026-04-27
source_type: markdown
trust_level: primary
lang: ko
---
```

Markdown citation은 line range를 기본으로 한다.

```text
notes/rust/kb.md:L12-L34
```

# 9. 이미지, PDF, 음성 확장 전략

## 9.1 이미지

이미지는 Markdown 다음 확장 대상으로 둔다.

이미지에서 얻을 수 있는 지식은 최소 세 종류다.

1. 파일 metadata: 경로, EXIF, 크기, 생성일
2. OCR text: 이미지 안의 실제 텍스트
3. AI caption 또는 visual embedding: 모델이 해석한 이미지 의미

중요한 규칙은 **OCR 결과와 AI caption을 같은 신뢰도로 취급하지 않는 것**이다. OCR은 관찰된 텍스트이고, caption은 모델이 생성한 설명이다. 따라서 provenance에는 다음처럼 구분해야 한다.

```text
observed_text: OCR 결과
model_caption: local VLM이 생성한 설명
visual_embedding: image embedding vector
```

Apple Vision framework는 이미지 속 텍스트 인식과 bounding box 정보를 제공한다. macOS native integration을 고려한다면 나중에 Swift sidecar 또는 Tauri/desktop adapter와 연결할 수 있다. [Apple Vision text recognition](https://developer.apple.com/documentation/vision/locating-and-displaying-recognized-text)

Rust 이미지 처리 기본 후보는 `image` crate와 `imageproc` crate다. `image` crate는 일반 이미지 포맷 decoding/encoding과 기본 조작을 제공한다. [image crate](https://lib.rs/crates/image)

## 9.2 PDF

PDF는 두 단계로 나눠야 한다.

```text
1단계: text PDF extraction
2단계: scanned PDF OCR
```

처음부터 완벽한 layout reconstruction을 목표로 하지 말고, page number와 text span을 보존하는 것을 목표로 한다.

PDF citation은 다음 형식을 가져야 한다.

```text
paper.pdf:p13 또는 paper.pdf:p13:section=Experiment Setup
```

Rust PDF 후보는 다음과 같다.

- `pdf-extract`: PDF에서 텍스트를 추출하는 library
- `lopdf`: PDF document manipulation library

`pdf-extract`는 Rust PDF text extraction crate로 공개되어 있다. [pdf-extract crate](https://lib.rs/crates/pdf-extract)

## 9.3 음성

음성은 transcript가 핵심이다.

```text
audio file
  -> transcription
  -> timestamped segments
  -> optional speaker labels
  -> CanonicalDocument
```

음성 citation은 다음 형식을 가져야 한다.

```text
meeting-2026-04-27.m4a:00:13:42-00:14:10
```

`whisper.cpp`는 Apple Silicon, ARM NEON, Accelerate, Metal, Core ML 관련 최적화를 명시한다. 로컬 MacBook에서 음성 전사 엔진으로 적합한 후보이며, Rust에서는 binding을 감싸는 adapter crate를 둘 수 있다. [whisper.cpp README](https://github.com/ggml-org/whisper.cpp/blob/master/README.md)

# 10. 저장소 전략

추천 기본 조합은 다음과 같다.

```text
filesystem: raw assets, extracted artifacts, model cache
SQLite: metadata, job state, document/chunk table, lexical FTS
LanceDB: vector embeddings, multimodal vector search
```

SQLite FTS5는 full-text search virtual table module이며, `bm25()`, `highlight()`, `snippet()` 같은 보조 함수를 제공한다. Markdown-first MVP에서는 SQLite FTS5만으로도 유용한 검색을 만들 수 있다. [SQLite FTS5](https://sqlite.org/fts5.html)

LanceDB는 OSS embedded library로 사용할 수 있고, local filesystem path에 연결할 수 있으며, Rust SDK를 제공한다. 문서에서는 vector search, full-text search, SQL, metadata, multimodal data, table versioning 등을 언급한다. [LanceDB docs](https://docs.lancedb.com/) [LanceDB Rust crate](https://docs.rs/lancedb)

초기 선택은 다음과 같다.

| 계층 | 추천 | 이유 |
|---|---|---|
| 원본 저장 | filesystem + content hash | 단순하고 재처리 가능 |
| metadata | SQLite | 개인 로컬 앱에 충분 |
| lexical search | SQLite FTS5 | 내장, 단순, 빠른 MVP |
| vector search | LanceDB | embedded, Rust SDK, multimodal 확장 |
| model cache | filesystem | 로컬 모델 관리 용이 |

나중에 lexical search 품질이 중요해지면 Rust-native search engine인 Tantivy를 별도 adapter로 검토할 수 있다. 하지만 MVP에서는 SQLite FTS5부터 시작하는 편이 단순하다.

# 11. Local LLM과 embedding 전략

## 11.1 LLM과 embedding은 분리한다

LLM은 답변 생성용이고, embedding model은 검색용이다. 두 모델을 같은 것으로 취급하면 안 된다.

```text
Embedding model: 문서와 query를 vector로 변환
LLM: 검색된 context를 바탕으로 답변 생성
Reranker: 검색 후보를 query 기준으로 재정렬
```

## 11.2 Ollama adapter부터 시작

Ollama 문서는 macOS Sonoma 이상에서 Apple M series CPU/GPU support를 언급한다. 따라서 M4 MacBook에서 local LLM MVP를 만들기 쉽다. [Ollama macOS docs](https://docs.ollama.com/macos)

초기 adapter는 다음처럼 둔다.

```text
kb-llm-local
  - OllamaLanguageModel
  - later: LlamaCppLanguageModel
  - later: CandleLanguageModel
```

Ollama가 내부적으로 local server를 쓰더라도, 프로젝트 아키텍처 관점에서는 HTTP MSA가 아니다. `kb-llm-local` 안에 캡슐화된 model adapter일 뿐이다.

## 11.3 Local embedding

`fastembed-rs`는 Rust에서 local vector embeddings와 reranking을 생성하는 library이며, 동기 사용, ONNX inference, tokenizer 사용을 특징으로 한다. [fastembed-rs GitHub](https://github.com/Anush008/fastembed-rs)

초기 구성은 다음처럼 잡는다.

```toml
[models.embedding]
provider = "fastembed"
model = "multilingual-e5-small"
batch_size = 64

[models.llm]
provider = "ollama"
model = "qwen2.5:14b-instruct"
context_tokens = 32768
```

모델명은 예시다. 실제 선택은 당신의 문서와 golden query set으로 평가해야 한다.

# 12. M4 48GB MacBook 기준 실행 정책

M4 48GB MacBook은 개인용 local KB에 충분한 타겟이지만, indexing과 generation을 동시에 과하게 돌리면 체감 성능이 나빠질 수 있다.

권장 정책은 다음과 같다.

- embedding batch size는 config로 둔다.
- extraction, embedding, indexing은 bounded queue로 돌린다.
- LLM generation 중에는 대량 embedding job을 잠시 낮은 priority로 둔다.
- image/PDF/audio 처리는 background job으로 둔다.
- raw asset, extracted artifact, embedding cache, model cache를 분리한다.
- index rebuild는 명시적 command로 실행한다.
- 모든 job은 resume 가능해야 한다.

예시 config는 다음과 같다.

```toml
[workspace]
root = "~/KnowledgeBase"

[storage]
sqlite_path = "~/.local/share/kb/kb.sqlite"
vector_path = "~/.local/share/kb/lancedb"
raw_asset_path = "~/.local/share/kb/assets"
artifact_path = "~/.local/share/kb/artifacts"

[indexing]
max_parallel_extractors = 2
max_parallel_embeddings = 1
watch_filesystem = true

[chunking]
target_tokens = 500
overlap_tokens = 80
respect_markdown_headings = true

[models.embedding]
provider = "fastembed"
batch_size = 64

[models.llm]
provider = "ollama"
context_tokens = 32768
```

# 13. Trait 계약

컴포넌트는 trait으로 연결한다. 아래 계약을 `kb-core`에 둔다.

```rust
pub trait SourceConnector {
    fn scan(&self, scope: &SourceScope) -> anyhow::Result<Vec<RawAsset>>;
}

pub trait Extractor {
    fn supports(&self, media_type: &MediaType) -> bool;

    fn extract(
        &self,
        asset: &RawAsset,
        bytes: &[u8],
        ctx: &ExtractContext,
    ) -> anyhow::Result<CanonicalDocument>;
}

pub trait Chunker {
    fn chunk(
        &self,
        doc: &CanonicalDocument,
        policy: &ChunkPolicy,
    ) -> anyhow::Result<Vec<Chunk>>;
}

pub trait Embedder {
    fn model_id(&self) -> &str;
    fn dimensions(&self) -> usize;

    fn embed_texts(
        &self,
        inputs: &[EmbeddingInput],
    ) -> anyhow::Result<Vec<Vec<f32>>>;
}

pub trait Retriever {
    fn search(&self, query: &SearchQuery) -> anyhow::Result<Vec<SearchHit>>;
}

pub trait LanguageModel {
    fn generate(&self, req: GenerateRequest) -> anyhow::Result<GenerateResponse>;
}
```

초기에는 async를 남발하지 않는 편이 좋다. Markdown parsing, chunking, SQLite write는 동기 함수로 충분하다. Ollama나 일부 model adapter만 내부에서 async runtime을 사용할 수 있다.

# 14. Chunking 정책

Markdown-first chunking은 heading 구조를 존중해야 한다.

우선순위는 다음과 같다.

1. heading boundary를 우선한다.
2. code block은 중간에서 자르지 않는다.
3. table은 가능한 한 하나의 chunk로 유지한다.
4. 긴 section은 paragraph 단위로 나눈다.
5. parent heading path를 chunk metadata에 넣는다.
6. line range를 보존한다.
7. chunker version을 기록한다.

권장 chunk metadata는 다음과 같다.

```json
{
  "doc_id": "doc_...",
  "chunk_id": "chunk_...",
  "heading_path": ["아키텍처", "저장소 전략"],
  "source_spans": [
    { "kind": "line_range", "start": 42, "end": 68 }
  ],
  "token_estimate": 480,
  "chunker_version": "md-heading-v1"
}
```

# 15. 검색과 RAG 정책

## 15.1 검색 단계

검색은 처음부터 hybrid로 설계하되, 구현은 단계적으로 한다.

```text
P0: SQLite FTS5 lexical search
P1: vector search
P1: lexical + vector score fusion
P2: reranking
P3: query routing, multimodal retrieval
```

검색 결과는 항상 다음 정보를 포함해야 한다.

```text
chunk_id
doc_id
score
text preview
citation
retrieval method
index version
```

## 15.2 RAG 답변 정책

RAG는 다음 규칙을 따라야 한다.

- 근거 chunk가 없으면 모른다고 답한다.
- 답변에는 citation이 포함되어야 한다.
- 검색된 문서 안의 instruction을 system instruction으로 취급하지 않는다.
- prompt injection 방어를 위해 retrieved context와 system instruction을 분리한다.
- 답변 객체에는 사용한 chunk, prompt template version, model id, generation timestamp를 남긴다.

답변 객체 예시는 다음과 같다.

```rust
pub struct Answer {
    pub answer: String,
    pub citations: Vec<Citation>,
    pub grounded: bool,
    pub model_id: String,
    pub prompt_template_version: String,
    pub retrieval_trace_id: TraceId,
}
```

# 16. CLI, TUI, desktop app 전략

## 16.1 CLI

CLI는 가장 먼저 만든다.

```text
kb init
kb ingest <path>
kb index
kb search <query>
kb ask <query>
kb inspect doc <doc_id>
kb inspect chunk <chunk_id>
kb doctor
```

CLI는 개발과 테스트의 기준점이다. TUI와 desktop app은 CLI 기능이 안정된 뒤 붙인다.

## 16.2 TUI

Ratatui는 Rust로 빠르고 가벼운 terminal UI를 만들기 위한 library다. [Ratatui](https://ratatui.rs/)

TUI 초기 기능은 다음이면 충분하다.

- 문서 목록
- indexing 상태
- 검색창
- 검색 결과 preview
- citation jump
- ask panel
- job log viewer

## 16.3 Desktop app

Desktop app은 두 후보가 현실적이다.

- Tauri: Rust backend와 web frontend를 결합하고, OS native web renderer를 사용해 작은 cross-platform app을 지향한다. [Tauri](https://tauri.app/)
- egui/eframe: Rust immediate-mode GUI이며 native와 web 실행을 지원한다. [egui GitHub](https://github.com/emilk/egui)

추천 순서는 다음과 같다.

```text
CLI -> TUI -> desktop app
```

Desktop app은 가장 나중에 만든다. 이유는 UI보다 먼저 domain model, search, citation, indexing이 안정되어야 하기 때문이다.

# 17. 구현 로드맵

## Phase 0 - 계약과 뼈대

목표: compile되는 workspace와 spec 문서 만들기.

산출물:

```text
kb-core
kb-config
kb-app
kb-cli
docs/spec/*
fixtures/markdown/*
```

완료 조건:

```text
cargo check --workspace
cargo test --workspace
kb --help
```

## Phase 1 - Markdown ingestion

목표: Markdown을 읽고 canonical document와 chunk로 변환한다.

구현 crate:

```text
kb-source-fs
kb-parse-md
kb-normalize
kb-chunk
kb-store-sqlite
```

완료 조건:

```text
kb ingest ~/KnowledgeBase
kb list docs
kb inspect doc <doc_id>
```

## Phase 2 - Lexical search

목표: SQLite FTS5 기반 검색을 만든다.

완료 조건:

```text
kb search "Rust workspace 설계"
```

결과는 citation을 포함해야 한다.

```text
1. Rust workspace는 여러 package를 하나로 관리한다...
   source: notes/rust/kb.md:L12-L34
```

## Phase 3 - Vector search와 embedding

목표: local embedding과 vector store를 붙인다.

구현 crate:

```text
kb-embed
kb-embed-local
kb-store-vector
kb-search
```

완료 조건:

```text
kb index --embeddings
kb search --mode vector "비슷한 설계 원칙"
kb search --mode hybrid "Markdown chunking 규칙"
```

## Phase 4 - Local LLM RAG

목표: local LLM으로 citation 포함 답변을 생성한다.

구현 crate:

```text
kb-llm
kb-llm-local
kb-rag
```

완료 조건:

```text
kb ask "내 KB 설계에서 저장소 전략은?"
```

답변은 citation을 포함해야 하며, 근거가 없으면 거절해야 한다.

## Phase 5 - Evaluation

목표: 검색 품질과 답변 품질을 회귀 테스트한다.

구현:

```text
fixtures/golden_queries.yaml
kb-eval
```

측정값:

```text
hit@k
MRR
citation coverage
empty result rate
answer groundedness
```

## Phase 6 - 이미지 support

목표: 이미지 metadata, OCR text, optional caption을 canonical document로 만든다.

완료 조건:

```text
kb ingest ./assets/diagram.png
kb search "이미지 안의 OCR 텍스트"
```

## Phase 7 - PDF support

목표: text PDF extraction과 page citation을 제공한다.

완료 조건:

```text
kb ingest ./paper.pdf
kb search "PDF 안의 특정 개념"
```

## Phase 8 - 음성 support

목표: audio transcription과 timestamp citation을 제공한다.

완료 조건:

```text
kb ingest ./meeting.m4a
kb search "회의에서 언급한 결정사항"
```

## Phase 9 - TUI와 desktop app

목표: 사용성을 높인다.

순서:

```text
kb-tui -> kb-desktop
```

# 18. 테스트 전략

테스트는 처음부터 포함한다.

| 테스트 | 목적 |
|---|---|
| unit test | parser, chunker, ID 생성 규칙 검증 |
| snapshot test | canonical document JSON이 의도대로 유지되는지 검증 |
| contract test | trait 구현체가 같은 입력에 같은 의미의 출력을 내는지 검증 |
| integration test | ingest -> chunk -> store -> search 흐름 검증 |
| golden query test | 검색 품질 회귀 방지 |
| RAG eval | citation coverage와 groundedness 검증 |
| fixture corpus test | Markdown edge case 검증 |

가장 중요한 fixture는 Markdown edge case다.

```text
- frontmatter only
- nested headings
- long paragraph
- code block
- table
- image reference
- relative links
- malformed markdown
- Korean + English mixed text
```

# 19. AI를 이용해 컴포넌트를 만들 때의 규약

AI에게 “전체 repo를 만들어줘”라고 시키지 말고, component spec 단위로 시켜야 한다.

템플릿은 다음과 같다.

```text
Component: kb-parse-md

Responsibility:
- Markdown bytes를 CanonicalDocument로 변환한다.
- frontmatter, heading, paragraph, list, code, table, link, image ref를 보존한다.
- line range 또는 byte range를 최대한 보존한다.

Allowed dependencies:
- kb-core
- pulldown-cmark 또는 comrak
- serde
- thiserror

Forbidden dependencies:
- kb-store
- kb-llm
- kb-rag
- kb-tui
- kb-desktop

Inputs:
- RawAsset
- &[u8]
- ExtractContext

Outputs:
- CanonicalDocument

Tests:
- frontmatter parsing
- heading tree
- code block language
- image reference
- line range preservation
- malformed markdown does not panic

Non-goals:
- embedding 생성 금지
- DB write 금지
- LLM 호출 금지
```

이 규약에서 가장 중요한 것은 `Allowed dependencies`와 `Forbidden dependencies`다. AI가 편의상 parser 안에서 DB write를 하거나, search 모듈에서 직접 LLM을 호출하는 식의 경계 침범을 막아야 한다.

# 20. 피해야 할 안티패턴

다음은 피해야 한다.

1. UI에서 DB를 직접 호출한다.
2. parser에서 embedding을 만든다.
3. chunk에 source span이 없다.
4. 원본 파일을 파생물로 덮어쓴다.
5. PDF, 이미지, 음성을 별도 검색 파이프라인으로 만든다.
6. embedding model 변경 시 재색인 범위를 추적할 수 없다.
7. 검색 결과에 citation이 없다.
8. LLM 답변을 저장하면서 사용한 context와 model version을 저장하지 않는다.
9. 처음부터 desktop app에 시간을 많이 쓴다.
10. local LLM model 선택을 평가 없이 감으로 정한다.

# 21. 추천 초기 개발 순서

처음 2주를 가정하면 다음 순서가 좋다.

## 1-2일차

- workspace 생성
- `kb-core` 도메인 타입 초안
- ID 규칙 문서화
- CLI skeleton

## 3-5일차

- local folder scanner
- Markdown parser
- canonical document JSON 출력
- fixture 기반 snapshot test

## 6-8일차

- chunker 구현
- SQLite schema
- ingest command

## 9-11일차

- SQLite FTS5 검색
- citation 출력
- `kb inspect` 구현

## 12-14일차

- local embedding 실험
- LanceDB adapter 초안
- hybrid search 실험
- golden query fixture 작성

이 순서대로 가면 2주 안에 “LLM 없이도 쓸 수 있는 개인 지식 검색기”가 만들어지고, 그 다음에 RAG를 붙일 수 있다.

# 22. 최종 체크리스트

MVP 완료 조건은 다음과 같다.

- [ ] `cargo check --workspace`가 통과한다.
- [ ] `cargo test --workspace`가 통과한다.
- [ ] Markdown frontmatter를 읽는다.
- [ ] heading path를 보존한다.
- [ ] chunk마다 source span이 있다.
- [ ] SQLite에 document/chunk metadata가 저장된다.
- [ ] FTS 검색이 된다.
- [ ] 검색 결과에 citation이 있다.
- [ ] 같은 원본을 재수집해도 중복되지 않는다.
- [ ] parser/chunker version을 바꾸면 재처리 대상이 식별된다.
- [ ] local embedding을 붙일 수 있는 trait이 있다.
- [ ] local LLM을 붙일 수 있는 trait이 있다.
- [ ] `kb-app` facade를 통해 CLI가 동작한다.

P1 완료 조건은 다음과 같다.

- [ ] vector search가 된다.
- [ ] hybrid search가 된다.
- [ ] RAG 답변에 citation이 포함된다.
- [ ] 근거 없는 질문에는 답하지 않는다.
- [ ] golden query set으로 검색 품질을 추적한다.

P2 완료 조건은 다음과 같다.

- [ ] 이미지 OCR text를 검색할 수 있다.
- [ ] PDF page citation을 제공한다.
- [ ] TUI에서 검색과 citation 확인이 가능하다.

P3 완료 조건은 다음과 같다.

- [ ] 음성 transcript를 검색할 수 있다.
- [ ] timestamp citation을 제공한다.
- [ ] desktop app에서 문서, 이미지, PDF, 음성 citation을 확인할 수 있다.

# 23. 최종 권장 스택

| 영역 | 1차 추천 | 대안 |
|---|---|---|
| 언어 | Rust 2024 | Python helper는 최소화 |
| repo 구조 | Cargo workspace | 단일 crate는 비추천 |
| 원본 저장 | filesystem + blake3 | object store는 나중 |
| metadata | SQLite | PostgreSQL은 과함 |
| lexical search | SQLite FTS5 | Tantivy |
| vector store | LanceDB | sqlite-vec, Qdrant local |
| Markdown parser | pulldown-cmark | comrak |
| embedding | fastembed-rs | Ollama embedding endpoint, candle |
| LLM | Ollama adapter | llama.cpp, candle |
| TUI | Ratatui | 없음 |
| desktop | Tauri 또는 egui | Dioxus |
| audio transcription | whisper.cpp adapter | OS speech API |

# 24. 참고 자료

- Rust 2024 Edition Guide - Cargo resolver: <https://doc.rust-lang.org/edition-guide/rust-2024/cargo-resolver.html>
- Cargo Book - Workspaces: <https://rustwiki.org/en/cargo/reference/workspaces.html>
- pulldown-cmark: <https://github.com/pulldown-cmark/pulldown-cmark>
- Comrak: <https://comrak.ee/>
- SQLite FTS5: <https://sqlite.org/fts5.html>
- LanceDB documentation: <https://docs.lancedb.com/>
- LanceDB Rust crate: <https://docs.rs/lancedb>
- fastembed-rs: <https://github.com/Anush008/fastembed-rs>
- Ollama macOS documentation: <https://docs.ollama.com/macos>
- whisper.cpp: <https://github.com/ggml-org/whisper.cpp/blob/master/README.md>
- Ratatui: <https://ratatui.rs/>
- Tauri: <https://tauri.app/>
- egui: <https://github.com/emilk/egui>
- Apple Vision text recognition: <https://developer.apple.com/documentation/vision/locating-and-displaying-recognized-text>
- image crate: <https://lib.rs/crates/image>
- pdf-extract crate: <https://lib.rs/crates/pdf-extract>

