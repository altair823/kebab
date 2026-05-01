---
phase: P3
title: "Local embedding + LanceDB + hybrid search"
status: planned
depends_on: [P2]
source: kb_local_rust_report.md §10, §11, §15, §17 Phase 3
---

# P3 — Local embedding + LanceDB + hybrid search

## 목표

local embedding 으로 chunk vector 화 → LanceDB 저장 → vector 검색 + lexical 융합 (hybrid). `kb search --mode {lexical,vector,hybrid}` 동작.

## 산출 crate

| crate | 역할 |
|-------|------|
| `kb-embed` | `Embedder` trait + `EmbeddingInput`/output 타입 |
| `kb-embed-local` | `fastembed-rs` adapter (1차). later: Ollama embed endpoint, candle |
| `kb-store-vector` | LanceDB 연동. table 관리, upsert, vector search |
| `kb-search` | lexical + vector 병행 + score fusion |

## Embedder

```rust
pub trait Embedder {
    fn model_id(&self) -> &str;
    fn dimensions(&self) -> usize;
    fn embed_texts(&self, inputs: &[EmbeddingInput]) -> anyhow::Result<Vec<Vec<f32>>>;
}

pub struct EmbeddingInput<'a> {
    pub text: &'a str,
    pub kind: EmbeddingKind, // Document | Query
}
```

- query 와 document 분리 prompt (e5 계열은 prefix 다름).
- batch_size config 화.
- 동기 인터페이스. 내부에서 ONNX runtime 사용.

기본 모델: `multilingual-e5-small` (config 가능). 차원/모델 ID 는 record 에 항상 같이 저장.

## LanceDB schema

table: `chunk_embeddings`

```text
chunk_id    : utf8 (primary)
doc_id      : utf8
embedding   : fixed-size-list<float32, D>
model_id    : utf8
embedding_version : utf8
text        : utf8           # 미리보기/rerank 용
heading_path: utf8
created_at  : timestamp
```

- D 는 모델 차원. 모델 변경 시 새 table (`chunk_embeddings_<model_id>`) 로 분리. mix 금지.
- index: IVF_PQ 또는 cosine flat. 코퍼스 < 100K chunk 면 flat 으로 충분.
- LanceDB Rust SDK 사용 (`lancedb` crate).

## Indexing job

```text
kb index --embeddings [--model <id>] [--batch-size N] [--resume]
```

- chunk 중 `embedding_id = chunk_id + model_id + dim` 가 vector store 에 없는 것만 처리.
- resume: 마지막 처리된 chunk_id checkpoint (`jobs` table).
- LLM generation 동시 실행 시 batch_size / 병렬도 낮춤 (config `models.embedding.batch_size`, §12).

## Hybrid search

```rust
pub enum SearchMode { Lexical, Vector, Hybrid }
```

Hybrid 점수 융합 (1차): RRF (Reciprocal Rank Fusion).

```text
score(chunk) = sum_over_methods( 1 / (k_rrf + rank_method(chunk)) )
k_rrf 기본 60.
```

이유: bm25 score 와 cosine sim 의 절대값 스케일이 다름. RRF 는 rank 기반이라 안정적.

P3 범위에선 reranker 미도입 (P+ 단계 노트).

## kb-search 구조

```rust
pub struct HybridRetriever {
    lexical: Box<dyn Retriever>,
    vector:  Box<dyn Retriever>,
    fusion:  FusionPolicy,
}
```

- 각 sub retriever 는 `Retriever` trait 구현.
- `kb-app::search` 가 mode 따라 dispatch.

## kb-app facade 확장

P3 동안 kb-app facade 의 `ingest` / `search` / `list_docs` / `inspect_doc` / `inspect_chunk` 의 stub 본체를 실제 라이브러리 호출로 대체. P0 부터 시그니처는 frozen 이므로 signature 변경 없이 body 만 swap.

```rust
pub fn ingest(scope: SourceScope, summary_only: bool) -> anyhow::Result<IngestReport>; // p3-5
pub fn search(query: SearchQuery)                  -> anyhow::Result<Vec<SearchHit>>;   // p3-5
pub fn list_docs(filter: DocFilter)                -> anyhow::Result<Vec<DocSummary>>;  // p3-5
pub fn inspect_doc(id: &DocumentId)                -> anyhow::Result<CanonicalDocument>;// p3-5
pub fn inspect_chunk(id: &ChunkId)                 -> anyhow::Result<Chunk>;            // p3-5
pub fn ask(query: &str, opts: AskOpts)             -> anyhow::Result<Answer>;           // p4-3 (stub remains)
```

p3-5 는 LLM 미관여 facade 모두 (`ask` 제외) 를 한 번에 wire. 이후 `cargo run -p kb-cli -- index` 와 `cargo run -p kb-cli -- search --mode {lexical,vector,hybrid}` 가 실 동작.

## CLI

```text
kb index --embeddings
kb search --mode vector "비슷한 설계 원칙"
kb search --mode hybrid "Markdown chunking 규칙"
```

## 테스트

- embedding determinism: 동일 입력 + 동일 모델 → 동일 vector (within fp tolerance).
- vector search smoke: fixture corpus 에서 paraphrase query 로 의도한 chunk 회수.
- hybrid 가 lexical 단독보다 hit@k 높음 (golden query 일부로 sanity check, 본격 측정은 P5).
- embedding_id collision 없음.
- 모델 교체 시 별도 table 분리 동작.

## 의존성 경계

- `kb-embed-local` 만 ONNX/모델 binding 의존. 다른 crate 는 trait 만 사용.
- `kb-store-vector` 는 `lancedb` 의존. SQLite 와 cross-write 금지 (각 store 책임 분리).
- LLM crate 와 분리 (§11.1).

## 완료 조건

- [ ] `kb index` (= `kb-app::ingest`) 로 모든 chunk 가 SQLite + LanceDB 에 저장 (p3-5)
- [ ] `kb search --mode vector` 정상 hit
- [ ] `kb search --mode hybrid` 정상 hit, citation 포함
- [ ] 모델/차원 변경 시 별도 table 로 분리 저장
- [ ] resume 시 미완료 chunk 만 처리 (P+ 로 deferred)
- [ ] hit@k 측정 가능한 형태로 결과 구조화 (P5 준비)
- [ ] `kb-app::{ingest,search,list_docs,inspect_doc,inspect_chunk}` 가 실 동작 (`ask` 는 P4-3 까지 stub) — p3-5

## 리스크 / 주의

- 모델 차원 변경 = vector index 호환 안 됨. 새 table 필수.
- M4 48GB 에서 LLM 과 embedding 동시 실행 시 thermal throttle 가능 (§12). embedding 은 background priority.
- RRF k_rrf 튜닝은 golden set 생기기 전엔 의미 없음. 기본값 고정.
- e5 query/document prefix 빠뜨리면 품질 급락. adapter 에서 강제.
