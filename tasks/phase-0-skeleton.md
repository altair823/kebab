---
phase: P0
title: "Workspace 뼈대 + 도메인 계약"
status: completed
depends_on: []
source: kebab_local_rust_report.md §3, §4, §6, §7, §13
---

# P0 — Workspace 뼈대 + 도메인 계약

## 목표

compile 되는 Rust 2024 workspace 와 domain spec 확정. 이후 모든 phase 가 이 계약 위에서 동작.

## 산출 crate

| crate | 역할 |
|-------|------|
| `kebab-core` | domain type, trait, error, ID 규칙 |
| `kebab-parse-types` | parser 중간 표현 (`ParsedBlock` 등) — `kebab-core` 와 parsers/normalize 사이 thin layer (design §3.7b) |
| `kebab-config` | config 로딩, 기본값, 경로 확장 |
| `kebab-app` | facade. CLI/TUI/desktop 공통 진입점 |
| `kebab-cli` | `kebab` 바이너리 skeleton (`--help`만 동작) |

## Workspace 설정

`Cargo.toml` root:

```toml
[workspace]
resolver = "3"
members = ["crates/kebab-core", "crates/kebab-parse-types", "crates/kebab-config", "crates/kebab-app", "crates/kebab-cli"]

[workspace.package]
edition = "2024"
rust-version = "1.85"

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

추가 멤버 crate 는 후속 phase 에서 합류.

## kebab-core 도메인 타입

`RawAsset`, `CanonicalDocument`, `Block`, `Chunk`, `SearchHit`, `Citation`, `SourceSpan`, `Provenance`, `Metadata`. report §6 정의 그대로.

## kebab-core trait

```rust
pub trait SourceConnector { fn scan(&self, scope: &SourceScope) -> anyhow::Result<Vec<RawAsset>>; }
pub trait Extractor { fn supports(&self, m: &MediaType) -> bool; fn extract(&self, asset: &RawAsset, bytes: &[u8], ctx: &ExtractContext) -> anyhow::Result<CanonicalDocument>; }
pub trait Chunker { fn chunk(&self, doc: &CanonicalDocument, policy: &ChunkPolicy) -> anyhow::Result<Vec<Chunk>>; }
pub trait Embedder { fn model_id(&self) -> &str; fn dimensions(&self) -> usize; fn embed_texts(&self, inputs: &[EmbeddingInput]) -> anyhow::Result<Vec<Vec<f32>>>; }
pub trait Retriever { fn search(&self, query: &SearchQuery) -> anyhow::Result<Vec<SearchHit>>; }
pub trait LanguageModel { fn generate(&self, req: GenerateRequest) -> anyhow::Result<GenerateResponse>; }
```

초기엔 동기. async 도입은 LLM/embedding adapter 내부에 한정.

## ID 규칙 (deterministic)

```text
asset_id     = blake3(raw bytes)
doc_id       = stable source path + asset hash + parser version
block_id     = doc_id + block path + source span
chunk_id     = doc_id + chunker version + block ids
embedding_id = chunk_id + embedding model id + dimension
index_id     = collection name + index version + embedding model id
```

각 record 에 다음 version 필드 보존: `doc_version`, `schema_version`, `parser_version`, `chunker_version`, `embedding_model`, `embedding_version`, `index_version`, `prompt_template_version`.

## kebab-app facade

CLI/TUI/desktop 모두 facade 함수 호출. parser/DB/LLM adapter 직접 호출 금지. 초기 facade 메서드 stub:

```rust
pub fn ingest(path: &Path) -> anyhow::Result<IngestReport>;
pub fn search(query: &str, mode: SearchMode) -> anyhow::Result<Vec<SearchHit>>;
pub fn ask(query: &str) -> anyhow::Result<Answer>;
pub fn inspect_doc(id: &DocumentId) -> anyhow::Result<CanonicalDocument>;
pub fn inspect_chunk(id: &ChunkId) -> anyhow::Result<Chunk>;
pub fn doctor() -> anyhow::Result<DoctorReport>;
```

## kebab-cli skeleton

`clap` derive. subcommand: `init`, `ingest`, `index`, `search`, `ask`, `inspect doc|chunk`, `doctor`. 본체는 `kebab-app` 호출만. P0 에선 `--help` 만 동작.

## spec 문서

`docs/spec/` 에 다음 작성:
- `domain-model.md`
- `ids.md`
- `canonical-document.md`
- `chunk-policy.md`
- `citation-policy.md`
- `module-boundaries.md`
- `ai-generation-guidelines.md`

## fixture

`fixtures/markdown/` 에 최소 3개: `simple-note.md`, `nested-headings.md`, `code-and-table.md`.

## 의존성 경계

- `kebab-core`: 외부 의존 최소 (serde, time, uuid, blake3, thiserror, tracing).
- `kebab-cli` → `kebab-app` 만 의존. parser/DB/LLM 직접 의존 금지.
- `kebab-app` 은 trait 만 보고 동작. 구현체는 dyn injection.

## 완료 조건

- [ ] `cargo check --workspace` 통과
- [ ] `cargo test --workspace` 통과 (단위 테스트는 ID 생성/도메인 직렬화 round-trip)
- [ ] `kebab --help` 출력
- [ ] `docs/spec/*` 7개 문서 존재
- [ ] `fixtures/markdown/*` 3개 존재
- [ ] domain type serde JSON snapshot test 1개 이상

## 리스크 / 주의

- ID 규칙 변경은 모든 후속 phase 의 record 무효화. P0 에서 못 박을 것.
- async 남발 금지. 동기로 충분.
- crate 경계 침범 (특히 facade 우회) 1건이라도 들어오면 후속 phase 전체가 흔들림.
