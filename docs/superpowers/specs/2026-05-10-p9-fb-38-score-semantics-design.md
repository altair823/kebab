---
title: "p9-fb-38 — Score semantics design"
phase: P9
component: kebab-core + kebab-search + kebab-cli + wire-schema + docs
task_id: p9-fb-38
status: design
target_version: 0.6.0
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§4 search, §10 UX, wire-schema search_hit.v1]
date: 2026-05-10
---

# p9-fb-38 — Score semantics

## Goal

agent / 외부 통합이 `search_hit.v1.score` 를 confidence 로 오해하지 않도록 의미를 wire + docs 에 명시. 두 axes:

- **Wire (additive minor)**: `search_hit.v1` 에 `score_kind: string` 필드 추가 — `"rrf"` (hybrid) / `"bm25"` (lexical) / `"cosine"` (vector). top-level `score` 의 의미를 hit 단위로 declarative 하게 표시.
- **Docs**: README + design §4 + SKILL 에 RRF 수식 전체 (`2/(k+rank)` per-chunk, `2/(k+1)` ceiling, normalize 과정) + "ranking signal, NOT confidence" 안내. agent 용 trust threshold 는 nested `retrieval.lexical_score` / `vector_score` 권장.

wire change additive minor — schema bump 없음, 기존 consumer 무영향.

## Behavior contract

### Wire shape

**`search_hit.v1`** — 신규 optional 필드:

```jsonc
{
  "schema_version": "search_hit.v1",
  "rank": 1,
  "score": 0.5,                 // 기존 — RRF normalized (hybrid) 또는 raw (lexical / vector)
  "score_kind": "rrf",          // p9-fb-38 신규 — "rrf" | "bm25" | "cosine"
  // 기존 필드 ...
  "retrieval": {
    "method": "hybrid",
    "fusion_score": 0.5,
    "lexical_score": 12.34,    // BM25 raw — agent 용 trust threshold
    "vector_score": 0.78,       // cosine sim — agent 용 trust threshold
    "lexical_rank": 1,
    "vector_rank": 1
  }
}
```

`score_kind` `#[serde(default)]` (옛 reader / 옛 writer 호환). schema 의 `required` 미추가.

### Score kind dispatch

| Retriever | `score_kind` | top-level `score` 의 값 |
|-----------|--------------|--------------------------|
| LexicalRetriever | `"bm25"` | raw BM25 (≥ 0, unbounded) |
| VectorRetriever | `"cosine"` | cosine similarity (`[-1, 1]`) |
| HybridRetriever (fuse) | `"rrf"` | RRF normalized (`[0, 1]`) |
| HybridRetriever (search_with_trace, mode=Lexical) | `"bm25"` | pass-through from LexicalRetriever |
| HybridRetriever (search_with_trace, mode=Vector) | `"cosine"` | pass-through from VectorRetriever |

`SearchMode` 와 `score_kind` 의 1:1 매핑은 hybrid retriever 가 mode-dispatch 시 결정. lexical/vector mode 의 hits 는 retriever 자체가 정한 kind 그대로.

### Backwards-compat

- 옛 wire reader (fb-38 이전 binary): JSON 에 `score_kind` 키 없음. ignore. 영향 없음.
- 옛 wire writer (fb-38 이전 binary 가 보낸 JSON 을 새 binary 가 읽음): `score_kind` 부재 → `default_score_kind() = ScoreKind::Rrf`. 잘못된 추정 가능 (실제 lexical / vector mode 였을 수도).
- 정확한 의미 보장은 v0.6.0 이후 binary 로 통일 시점부터.

## Allowed / forbidden dependencies

- `kebab-core`: 신규 dep 없음. enum + field 추가만.
- `kebab-search`: 신규 dep 없음. hit construction 시 score_kind 라벨링.
- `kebab-cli`: 무수정 (serde 자동 emit).
- `kebab-mcp`: 무수정 (`SearchHit` 직접 serialize → 자동 포함).
- `kebab-tui`: 무수정.

`kebab-core` 의 다른 `kebab-*` 의존 금지 룰 그대로.

## Public surface delta

### kebab-core (`search.rs`)

```rust
/// p9-fb-38: top-level `SearchHit.score` 의 의미 declaration.
/// `Rrf` (hybrid) / `Bm25` (lexical-only) / `Cosine` (vector-only).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScoreKind {
    Rrf,
    Bm25,
    Cosine,
}

impl Default for ScoreKind {
    fn default() -> Self { ScoreKind::Rrf }
}
```

`SearchHit` 확장:

```rust
pub struct SearchHit {
    // 기존 필드 ...
    /// p9-fb-38: top-level `score` 의 의미 declaration.
    /// 옛 wire (부재) → `Rrf` default (hybrid 가 기본 mode).
    #[serde(default)]
    pub score_kind: ScoreKind,
}
```

### kebab-search (lexical / vector / hybrid)

- LexicalRetriever hit construction 에 `score_kind: ScoreKind::Bm25`.
- VectorRetriever hit construction 에 `score_kind: ScoreKind::Cosine`.
- HybridRetriever fuse 결과 hit 에 `score_kind: ScoreKind::Rrf`.
- HybridRetriever `search_with_trace` (fb-37) 의 Lexical/Vector branch 는 underlying retriever 의 hit 그대로 반환 — score_kind 는 그 retriever 의 라벨 (Bm25 / Cosine).

### kebab-cli + kebab-mcp

무수정. `serde_json::to_value(&hit)` 가 `score_kind` 를 자동 emit.

## Test plan

| kind | description |
|------|-------------|
| unit (kebab-core) | `ScoreKind` serde — Rrf↔"rrf", Bm25↔"bm25", Cosine↔"cosine" |
| unit (kebab-core) | `SearchHit` deserialization 시 `score_kind` 부재 → `Rrf` default |
| unit (kebab-core) | `ScoreKind::default() == Rrf` |
| unit (kebab-search/lexical) | LexicalRetriever hit 의 `score_kind == Bm25` |
| unit (kebab-search/vector) | VectorRetriever hit 의 `score_kind == Cosine` |
| unit (kebab-search/hybrid) | HybridRetriever fuse → all hits `Rrf` |
| unit (kebab-search/hybrid) | search_with_trace mode=Lexical → hits `Bm25` |
| 통합 (kebab-cli) | `kebab search Q --mode lexical --json` → `hits[0].score_kind == "bm25"` |
| 통합 (kebab-cli) | `kebab search Q --json` (default hybrid) → `hits[0].score_kind == "rrf"` |

vector mode 통합 테스트는 embeddings 의존 — unit (search_with_trace mode=Vector 시 hits Cosine) 으로 대체.

## Implementation steps (high-level)

1. `kebab-core::ScoreKind` enum + `SearchHit.score_kind` field + 단위 테스트.
2. `kebab-search/lexical.rs` LexicalRetriever hit construction 에 `Bm25` 라벨 + 단위 테스트.
3. `kebab-search/vector.rs` VectorRetriever hit construction 에 `Cosine` + 단위 테스트.
4. `kebab-search/hybrid.rs` fuse + search_with_trace 에 `Rrf` / pass-through + 단위 테스트.
5. `kebab-cli` 통합 테스트 (lexical-only + hybrid).
6. `docs/wire-schema/v1/search_hit.schema.json` — `score_kind` 필드 추가.
7. README — "Score interpretation" 섹션 (RRF 수식 + score_kind 표 + agent guidance).
8. design §4 search — RRF 수식 + normalize 정의 + score_kind 필드 등록.
9. SKILL.md — `mcp__kebab__search` 응답에 `score_kind` 안내.
10. tasks/INDEX.md / spec status flip.

## Risks / notes

- **RRF normalizer 변경 시**: k_rrf default 변경 또는 retriever 수 > 2 확장 시 ceiling 재계산. design §4 RRF 수식 + README Score interpretation 갱신 필요.
- **vector mode 통합 테스트 부재**: 통합 테스트 fixture 가 embeddings 없음 (`provider = "none"`). 통합은 lexical / hybrid 만, vector 는 단위 테스트로 cover.
- **fb-37 search_with_trace 와 정합성**: search_with_trace 는 underlying retriever 가 만든 hit 을 그대로 trace 의 lex/vec list 에 채움 — score_kind 도 자동 보존. 추가 작업 없음.
- **`#[serde(default)]` 의미**: 옛 wire reader 가 `score_kind` 키 발견 시 unknown field 거절 안 함 (serde 기본 동작 — `deny_unknown_fields` 없음, 확인 완료). 안전.

## Out of scope

- top-level `score` rename 또는 deprecation (v0.7.0+ 검토).
- channel score 의 추가 노출 (이미 `retrieval` block 에 있음).
- score gate threshold 변경 (config.rag.score_gate).
- TUI score badge / color hint.
- per-channel score normalization (BM25/cosine 둘 다 raw 유지).
- `RetrievalDetail.method` 와 `score_kind` 의 정합성 검증 (둘 다 같은 정보 source 지만 별도 declarative).

## Documentation updates (implementation PR 동시)

- `README.md` — "Score interpretation" 섹션 (RRF 수식 + score_kind 표 + agent guidance).
- `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` §4 — RRF 수식 block + score_kind field 등록.
- `docs/wire-schema/v1/search_hit.schema.json` — `score_kind` enum 필드.
- `integrations/claude-code/kebab/SKILL.md` — `mcp__kebab__search` 응답 안내 (score_kind + "ranking signal, NOT confidence" + raw threshold guidance).
- `tasks/p9/p9-fb-38-score-semantics.md` — `status: open → completed`, design + plan 링크.
- `tasks/INDEX.md` — fb-38 행 ✅.
