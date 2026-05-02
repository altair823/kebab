---
phase: P9
component: kebab-search + kebab-app
task_id: p9-fb-19
title: "Search result cache (in-memory LRU + index_version invalidation)"
status: planned
depends_on: []
unblocks: []
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§7 search, §9 versioning]
source_feedback: p9-dogfooding-feedback.md item 15
---

# p9-fb-19 — Search cache

## Goal

같은 query 반복 시 SQLite FTS + Lance + RRF 재계산 회피. 우선 in-memory LRU + `index_version` bump 기반 단순 invalidation.

## Allowed dependencies

- `lru = "0.12"` (검증된 LRU crate).

## Public surface

`kebab-app::App` 에 `Mutex<LruCache<CacheKey, Vec<SearchHit>>>` 필드. 호출자 (CLI / TUI) 는 변경 없이 cached 결과 받음.

```rust
struct CacheKey {
    query_norm: String,        // NFKC + trim + lowercase
    mode: SearchMode,
    k: u32,
    snippet_chars: u32,
    embedding_version: String,
    chunker_version: String,
    index_version: u64,
}
```

CLI: `kebab search --no-cache "..."` 로 강제 bypass.

## Behavior contract

- LRU capacity: 256 entry (cfg.search.cache_capacity, default 256). 메모리 한정 — 1 entry ≈ 5KB → 1.3MB 상한.
- normalize: query 정규화 후 같은 entry. 사용자 입력 trim 차이가 redundant compute 안 만듦.
- `index_version`: SQLite `kv` 테이블의 `kv['index_version']` 단조 증가 카운터. ingest 가 1 chunk 라도 변경하면 +1. embedding 만 추가/삭제도 +1. bump 시 cache 의 모든 entry 가 stale (index_version 키 비교).
- LRU evict / stale entry 는 next miss 시 자동 garbage. 명시적 wipe API 도 제공 (`App::clear_search_cache()`).
- TTL: in-memory LRU 라 process 수명. 영속 cache (SQLite) 는 P+.

## Test plan

| kind | description |
|------|-------------|
| unit | 같은 query 2 회 → 두번째 cache hit |
| unit | ingest → index_version+1 → 같은 query stale → recompute |
| unit | NFKC 정규화: "Foo" / "FOO" / " Foo " 같은 entry |
| unit | LRU evict: capacity+1 entry 삽입 → 가장 오래된 evict |
| integration | `--no-cache` flag 가 cache bypass |

## DoD

- [ ] `cargo test -p kebab-search -p kebab-app` 통과
- [ ] `index_version` SQLite 컬럼 + ingest 가 bump
- [ ] frozen design §9 versioning 에 `index_version` 추가
- [ ] README — `--no-cache` 안내

## Out of scope

- patch-and-merge incremental (사용자가 말한 "추가만 끼워넣기") — P+ task. 우선 stale 시 전체 recompute.
- SQLite 영속 cache — P+.
- per-process 공유 cache (RwLock 다른 process 간) — P+.

## Risks / notes

- patch-and-merge 가 더 효율적이지만 RRF normalization 이 hit set 전체 기준 (`2/(k+1)`) 이라 incremental 어려움. 우선 단순 LRU 가 도그푸딩 막힘 해결.
- `index_version` 신규 — versioning cascade (§9) 에 새 차원 추가. 기존 5 개 (parser/chunker/embedding/prompt_template/index 그 자체 의미 다름) 와 구분 필요. 명확화 작업이 spec 갱신 동반.
