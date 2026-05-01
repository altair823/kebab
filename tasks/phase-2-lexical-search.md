---
phase: P2
title: "SQLite FTS5 lexical 검색 + citation"
status: completed
depends_on: [P1]
source: kb_local_rust_report.md §10, §15, §17 Phase 2
---

# P2 — SQLite FTS5 lexical 검색 + citation

## 목표

embedding/LLM 없이 FTS5 만으로 동작하는 검색 + citation 출력. `kb search "..."` 가 chunk 와 source span 반환.

## 산출 crate

- `kb-search` (lexical 모드) — `Retriever` trait 구현 1번째.
- `kb-store-sqlite` 확장: FTS5 virtual table + trigger.

## FTS5 스키마

```sql
CREATE VIRTUAL TABLE chunks_fts USING fts5(
  chunk_id UNINDEXED,
  doc_id UNINDEXED,
  heading_path,
  text,
  tokenize = 'unicode61 remove_diacritics 2'
);

CREATE TRIGGER chunks_ai AFTER INSERT ON chunks BEGIN
  INSERT INTO chunks_fts(chunk_id, doc_id, heading_path, text)
  VALUES (new.chunk_id, new.doc_id, new.heading_path, new.text);
END;

CREATE TRIGGER chunks_ad AFTER DELETE ON chunks BEGIN
  DELETE FROM chunks_fts WHERE chunk_id = old.chunk_id;
END;

CREATE TRIGGER chunks_au AFTER UPDATE ON chunks BEGIN
  DELETE FROM chunks_fts WHERE chunk_id = old.chunk_id;
  INSERT INTO chunks_fts(chunk_id, doc_id, heading_path, text)
  VALUES (new.chunk_id, new.doc_id, new.heading_path, new.text);
END;
```

scoring: `bm25(chunks_fts)` 사용. snippet 표시는 `snippet(chunks_fts, 3, '<b>', '</b>', '…', 16)`.

한국어 토크나이저: `unicode61` 기본. CJK 향상 필요 시 `trigram` 보조 인덱스 검토 (P2 범위 밖, 후순위 노트).

## SearchQuery / SearchHit

```rust
pub struct SearchQuery {
    pub text: String,
    pub mode: SearchMode,        // P2: SearchMode::Lexical 만
    pub k: usize,                // default 10
    pub filters: SearchFilters,  // tag, lang, path glob
}

pub struct SearchHit {
    pub chunk_id: ChunkId,
    pub doc_id: DocumentId,
    pub score: f32,              // bm25 score 정규화
    pub text: String,            // snippet 또는 full chunk text
    pub citation: Citation,      // file path + line range
    pub retrieval_method: String,// "fts5-bm25"
    pub index_version: String,
}
```

`Citation` 형식: `notes/rust/kb.md:L12-L34`.

## 인덱스 라이프사이클

- ingest 시 trigger 로 자동 동기화.
- `kb index --rebuild-fts` command 로 FTS table 재구축 (chunker version bump 후 사용).
- `index_version` 은 `(schema_version, fts_config_hash)` 조합.

## kb-app facade 확장

```rust
pub fn search(query: SearchQuery) -> anyhow::Result<Vec<SearchHit>>;
```

## CLI

```text
kb search "Rust workspace 설계" [--k 10] [--tag rust] [--mode lexical]
kb index --rebuild-fts
```

출력 예:

```text
1. [0.82] Rust workspace는 여러 package를 하나로 관리한다…
   doc: notes/rust/kb.md
   citation: notes/rust/kb.md:L12-L34
   heading: 아키텍처 > Rust workspace
```

## 테스트

- fixture corpus 대상 known query → 기대 chunk 가 top-k 안에 들어오는지.
- citation 의 line range 가 원본 파일에서 실제 텍스트와 일치 (round-trip).
- 동일 query 재실행 시 결과 deterministic.
- empty corpus / 0건 hit 정상 처리 (panic 금지).

## 의존성 경계

- `kb-search` 는 `kb-store-sqlite` 와 `kb-core` 만 의존.
- LLM/embedding 호출 금지 (P2 단계).
- CLI 는 `kb-app` 통해서만 호출.

## 완료 조건

- [ ] `kb search "..."` top-k chunk 반환
- [ ] 모든 결과에 citation 포함
- [ ] citation line range 가 원본과 일치
- [ ] 한영 혼합 query 동작 (한국어 토큰화 한계는 노트로)
- [ ] golden query fixture 1차 셋 정의 (P5 에서 본격 활용)

## 리스크 / 주의

- 한국어 형태소 분석 없음 → recall 한계. P3 vector search 가 보완.
- bm25 score 절대값은 상대 비교용. UI 노출 시 정규화 필요.
- FTS trigger 가 transaction 안에서 도는지 확인. 대량 ingest 성능에 영향.
