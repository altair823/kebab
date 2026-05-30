# 별칭 dense 별도 벡터 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans. Steps use checkbox (`- [ ]`).

**Goal:** `chunk.aliases`를 별도 dense 벡터(sentinel chunk_id `{orig}#alias`)로 색인해, dense(e5)가 별칭 순수 신호로 설명형 패러프레이즈를 잡게 한다. 본문 벡터 불변(회귀 안전).

**Architecture:** `ingest.expansion.embed_aliases`(default off) on 이면 별칭을 e5 passage 임베딩 → sentinel chunk_id VectorRecord upsert. VectorRetriever 가 sentinel hit 을 원본 chunk_id 로 strip + dedup(2채널 유지, wire 무변경). purge 가 sentinel 벡터도 정리.

**Tech Stack:** Rust 2024, fastembed e5, LanceVectorStore(MergeInsert keyed on chunk_id), kebab-core/config/app/search.

**빌드 규약:** `CARGO_TARGET_DIR=/build/out/cargo-target/target`, `-j 4`. 결과 redirect + `echo "EXIT=$?"` 후 커밋. `cargo|grep` 금지. 브랜치 `feat/doc-side-expansion`(같은 PR).

**참조 spec:** `docs/superpowers/specs/2026-05-30-dense-alias-vectors-design.md`

---

## File Structure

| 파일 | 역할 | Task |
|------|------|------|
| `crates/kebab-core/src/ids.rs` | `ALIAS_SUFFIX` 상수 + `strip_alias_suffix` 헬퍼 | 1 |
| `crates/kebab-config/src/lib.rs` | `IngestExpansionCfg.embed_aliases` + env | 2 |
| `crates/kebab-app/src/lib.rs` | ingest 별칭 임베딩 + sentinel VectorRecord + purge sentinel | 3 |
| `crates/kebab-search/src/vector.rs` | VectorRetriever sentinel strip + dedup + overfetch↑ | 4 |
| `docs/`, dogfood | 측정 + 문서 | 5 |

---

## Task 1: `ALIAS_SUFFIX` + `strip_alias_suffix` (kebab-core)

**Files:** Modify `crates/kebab-core/src/ids.rs` (+ `lib.rs` re-export)

- [ ] **Step 1: 실패 테스트** — `ids.rs` `#[cfg(test)] mod tests` 에:

```rust
    #[test]
    fn strip_alias_suffix_roundtrip() {
        assert_eq!(strip_alias_suffix("abc123#alias"), "abc123");
        assert_eq!(strip_alias_suffix("abc123"), "abc123"); // 접미 없으면 그대로
        assert_eq!(ALIAS_SUFFIX, "#alias");
    }
```

- [ ] **Step 2: 실패 확인** — `CARGO_TARGET_DIR=/build/out/cargo-target/target cargo test -p kebab-core strip_alias_suffix -j 4 > /tmp/dv-t1.log 2>&1; echo "EXIT=$?"` → 컴파일 실패.

- [ ] **Step 3: 구현** — `ids.rs` 상단(pub 영역)에:

```rust
/// 별칭 dense 벡터의 sentinel chunk_id 접미. 본문 벡터(원본 chunk_id)와
/// 별칭 벡터(`{orig}#alias`)를 LanceDB(chunk_id 키)에서 공존시킨다. ChunkId 는
/// blake3 hex(영숫자)라 `#` 미포함 → 충돌 없음. 설계 spec dense-alias-vectors §3.2.
pub const ALIAS_SUFFIX: &str = "#alias";

/// sentinel 별칭 chunk_id 에서 원본 chunk_id 를 복원. 접미 없으면 그대로.
pub fn strip_alias_suffix(id: &str) -> &str {
    id.strip_suffix(ALIAS_SUFFIX).unwrap_or(id)
}
```

`crates/kebab-core/src/lib.rs` 의 `ids` re-export 에 `ALIAS_SUFFIX, strip_alias_suffix` 추가
(`pub use ids::{... , ALIAS_SUFFIX, strip_alias_suffix};` — 기존 `pub use ids::{...}` 목록에 삽입).

- [ ] **Step 4: 통과** — `cargo test -p kebab-core strip_alias_suffix -j 4` EXIT=0.

- [ ] **Step 5: 커밋** — `git add crates/kebab-core && git commit -m "feat(core): ALIAS_SUFFIX + strip_alias_suffix (dense alias vectors)"`

---

## Task 2: config `embed_aliases`

**Files:** Modify `crates/kebab-config/src/lib.rs`

- [ ] **Step 1: 실패 테스트** — `#[cfg(test)] mod tests` 에:

```rust
    #[test]
    fn embed_aliases_defaults_off() {
        assert!(!Config::defaults().ingest.expansion.embed_aliases);
    }

    #[test]
    fn embed_aliases_env_override() {
        let mut cfg = Config::defaults();
        let env: std::collections::HashMap<String, String> =
            [("KEBAB_INGEST_EXPANSION_EMBED_ALIASES".to_string(), "true".to_string())]
                .into_iter().collect();
        cfg.apply_env(&env);
        assert!(cfg.ingest.expansion.embed_aliases);
    }
```

- [ ] **Step 2: 실패 확인** — `cargo test -p kebab-config embed_aliases -j 4 > /tmp/dv-t2.log 2>&1; echo "EXIT=$?"` → 컴파일 실패.

- [ ] **Step 3: 구현** — `IngestExpansionCfg` struct 에 필드(기존 `prompt_version` 다음):

```rust
    /// 별칭을 dense 벡터로도 색인(별도 sentinel chunk_id). default off.
    /// `enabled`(별칭 생성)와 별개 축 — 둘 다 on 이어야 dense 별칭. 설계 spec
    /// dense-alias-vectors §3.3.
    pub embed_aliases: bool,
```

`impl Default for IngestExpansionCfg` 에 `embed_aliases: false,` 추가. `apply_env` 에:

```rust
            "KEBAB_INGEST_EXPANSION_EMBED_ALIASES" => {
                self.ingest.expansion.embed_aliases = parse_bool(v)
            }
```

- [ ] **Step 4: 통과** — `cargo test -p kebab-config -j 4` EXIT=0 (신규 2 + 기존).

- [ ] **Step 5: 커밋** — `git add crates/kebab-config && git commit -m "feat(config): ingest.expansion.embed_aliases flag (default off)"`

---

## Task 3: ingest 별칭 임베딩 + sentinel VectorRecord + purge

**Files:** Modify `crates/kebab-app/src/lib.rs` (embed 블록 ~1309, purge 함수)

- [ ] **Step 1: 구현 (embed 블록)** — `if !chunks.is_empty()` 블록(현재 body inputs/records 생성)을 확장. body records 생성 후 별칭 records 를 추가로 만들어 같은 `upsert` 에 합친다:

기존 body 임베딩(`let inputs = chunks.iter().map(|c| EmbeddingInput{text: c.text.as_str(), ...})` → `vectors` → `records`)은 **그대로**. `vec_store.upsert(&records)` **직전**에 추가:

```rust
            // dense 별칭(별도 벡터, sentinel chunk_id). embed_aliases on +
            // 별칭 있는 청크만. 본문 records 는 위에서 이미 생성됨(불변).
            let mut all_records = records;
            if app.config.ingest.expansion.embed_aliases {
                let alias_chunks: Vec<&kebab_core::Chunk> = chunks
                    .iter()
                    .filter(|c| c.aliases.as_deref().is_some_and(|a| !a.is_empty()))
                    .collect();
                if !alias_chunks.is_empty() {
                    let alias_inputs: Vec<EmbeddingInput<'_>> = alias_chunks
                        .iter()
                        .map(|c| EmbeddingInput {
                            text: c.aliases.as_deref().unwrap(),
                            kind: EmbeddingKind::Document,
                        })
                        .collect();
                    let alias_vectors = emb
                        .embed(&alias_inputs)
                        .context("Embedder::embed (alias vectors)")?;
                    for (c, v) in alias_chunks.iter().zip(alias_vectors) {
                        let alias_chunk_id = kebab_core::ChunkId(format!(
                            "{}{}",
                            c.chunk_id.0,
                            kebab_core::ALIAS_SUFFIX
                        ));
                        all_records.push(VectorRecord {
                            embedding_id: kebab_core::id_for_embedding(
                                &alias_chunk_id, &model_id, &model_version, dimensions,
                            ),
                            chunk_id: alias_chunk_id,
                            vector: v,
                            doc_id: canonical.doc_id.clone(),
                            text: c.aliases.clone().unwrap_or_default(),
                            heading_path: c.heading_path.clone(),
                            model_id: model_id.clone(),
                            model_version: model_version.clone(),
                            dimensions,
                        });
                    }
                }
            }
            vec_store.upsert(&all_records).context("VectorStore::upsert")?;
```

(기존 `vec_store.upsert(&records)` 줄은 위 `upsert(&all_records)` 로 대체 — 중복 upsert 금지.)

- [ ] **Step 2: 구현 (purge sentinel)** — `purge_vector_orphans_for_workspace_path` 의 `delete_by_chunk_ids(&stale)` 를, stale + sentinel 을 함께 지우도록:

```rust
    let mut to_delete = stale.clone();
    to_delete.extend(stale.iter().map(|id| format!("{}{}", id, kebab_core::ALIAS_SUFFIX)));
    vec_store
        .delete_by_chunk_ids(&to_delete)
        .context("VectorStore::delete_by_chunk_ids (orphan vector cleanup)")?;
```

그리고 `sweep_deleted_files` 의 `purge_deleted_workspace_path` 후 `vec.delete_by_chunk_ids(&chunk_ids)`(있는 곳)도 동일하게 `{id}#alias` 를 포함하도록 확장(해당 위치 `grep -n "delete_by_chunk_ids" crates/kebab-app/src/lib.rs` 로 모두 찾아 sentinel 추가).

- [ ] **Step 3: 빌드 + 회귀** — `CARGO_TARGET_DIR=/build/out/cargo-target/target cargo build -p kebab-app -j 4 > /tmp/dv-t3.log 2>&1; echo "EXIT=$?"` EXIT=0. `cargo test -p kebab-app -j 4` EXIT=0(embed_aliases off 라 기존 무영향).

- [ ] **Step 4: 커밋** — `git add crates/kebab-app/src/lib.rs && git commit -m "feat(app): 별칭 dense 별도 벡터 색인 + purge (sentinel)"`

---

## Task 4: VectorRetriever sentinel strip + dedup

**Files:** Modify `crates/kebab-search/src/vector.rs`

- [ ] **Step 1: 실패 테스트** — `crates/kebab-search/tests/` 의 기존 vector 테스트 패턴 확인(`ls crates/kebab-search/tests/ && grep -rln "VectorRetriever" crates/kebab-search/tests/`). store 에 body + `{orig}#alias` 벡터를 넣고, 별칭 벡터에 가까운 쿼리로 검색 시 결과가 **원본 chunk_id** 1개(중복 없음)인지 검증:

```rust
#[test]
fn alias_vector_hit_strips_to_original_and_dedupes() {
    // store 에 chunk "c1" body 벡터 + "c1#alias" 별칭 벡터. 쿼리가 둘 다 매칭.
    // 결과: 원본 "c1" 1개 (sentinel strip + dedup).
    // (기존 vector 테스트 헬퍼로 store fixture 구성 — 벡터/임베딩 mock 패턴 따름.)
    let hits = retr.search(&q).unwrap();
    let c1 = hits.iter().filter(|h| h.chunk_id.0 == "c1").count();
    assert_eq!(c1, 1, "body+alias 둘 다 매칭해도 원본 chunk_id 1개로 dedup");
    assert!(!hits.iter().any(|h| h.chunk_id.0.ends_with("#alias")),
        "sentinel chunk_id 가 결과에 노출되면 안 된다");
}
```

> 정확한 store fixture(벡터 upsert + embed mock)는 기존 `tests/` 의 VectorRetriever 테스트 패턴을 따른다.

- [ ] **Step 2: 실패 확인** — `cargo test -p kebab-search alias_vector_hit -j 4 > /tmp/dv-t4.log 2>&1; echo "EXIT=$?"` → 실패(현재 sentinel 노출 + 중복).

- [ ] **Step 3: 구현** — `vector.rs` `search()`:
  (a) `VECTOR_OVERFETCH_MULTIPLIER` 를 `2` → `3` (별칭 벡터로 dedup 후 k 미달 방지).
  (b) raw_hits 순회 루프에서 strip + dedup. 기존:
  ```rust
        let candidate_ids: Vec<&str> = raw_hits.iter().map(|h| h.chunk_id.0.as_str()).collect();
        let hydration = hydrate_chunks(&self.sqlite, &candidate_ids)...;
        ...
        for hit in raw_hits {
            let Some(meta) = hydration.get(hit.chunk_id.0.as_str()) else { continue; };
            rank = rank.saturating_add(1);
            hits.push(build_hit(hit, meta, rank, ...)?);
            if hits.len() >= k { break; }
        }
  ```
  를 다음으로(원본 id 로 hydrate + seen dedup, build_hit 에 strip 된 chunk_id 반영):
  ```rust
        // sentinel 별칭 hit 을 원본 chunk_id 로 strip 해 hydrate.
        let candidate_ids: Vec<&str> = raw_hits
            .iter()
            .map(|h| kebab_core::strip_alias_suffix(h.chunk_id.0.as_str()))
            .collect();
        let hydration = hydrate_chunks(&self.sqlite, &candidate_ids)
            .context("kb-search vector: hydrate chunk metadata")?;
        ...
        let model_id = self.embed.model_id();
        let mut hits: Vec<SearchHit> = Vec::with_capacity(k.min(raw_hits.len()));
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut rank: u32 = 0;
        for mut hit in raw_hits {
            let orig = kebab_core::strip_alias_suffix(hit.chunk_id.0.as_str()).to_string();
            if !seen.insert(orig.clone()) {
                continue; // 같은 원본이 body+alias 둘 다 → 첫(높은 score) 유지
            }
            let Some(meta) = hydration.get(orig.as_str()) else { continue; };
            // build_hit 이 원본 chunk_id 를 쓰도록 hit 의 chunk_id 를 strip 본으로 교체.
            hit.chunk_id = kebab_core::ChunkId(orig);
            rank = rank.saturating_add(1);
            hits.push(build_hit(hit, meta, rank, &self.index_version, &model_id, self.snippet_chars)?);
            if hits.len() >= k { break; }
        }
  ```
  (`raw_hits` 가 `Vec<VectorHit>` 라 `for mut hit` 가능. `VectorHit.chunk_id` 가 `pub` 인지 확인 — `crates/kebab-core/src/vector.rs:24`. pub 아니면 build_hit 시그니처에 override chunk_id 인자 추가.)

- [ ] **Step 4: 통과 + 회귀** — `cargo test -p kebab-search -j 4 > /tmp/dv-t4.log 2>&1; echo "EXIT=$?"` EXIT=0 (신규 + 기존 vector/hybrid).

- [ ] **Step 5: 커밋** — `git add crates/kebab-search/src/vector.rs crates/kebab-search/tests && git commit -m "feat(search): VectorRetriever sentinel 별칭 strip + dedup"`

---

## Task 5: 측정 + 문서

- [ ] **Step 1: clippy** — `cargo clippy --workspace --all-targets -j 4 -- -D warnings > /tmp/dv-clippy.log 2>&1; echo "EXIT=$?"` EXIT=0.

- [ ] **Step 2: 측정** — `.kebabignore`(topics 만) 재작성 → release 빌드 → `KEBAB_INGEST_EXPANSION_ENABLED=true KEBAB_INGEST_EXPANSION_EMBED_ALIASES=true kebab ingest --force-reingest`(topics 재임베딩, 별칭 벡터 생성, ~32분) → `KEBAB_EVAL_GOLDEN=... kebab eval run --mode hybrid --k 50` → `eval variants`. **Read 로 값 확인(추측 금지).**
  - **효과**: 영어 설명형(mvcc/raft) `recall@50` 0→양수 회복? concat PoC(6/0/2/0.25) 대비 개선?
  - **회귀**: body 벡터 불변이라 명사형/단일쿼리 회귀 0 확인. 측정 후 `.kebabignore` 삭제.

- [ ] **Step 3: 문서** — `tasks/HOTFIXES.md` dated entry(lexical 별칭 + dense 별칭 측정 표), README Configuration(`embed_aliases` off 기본), ARCHITECTURE(별칭 dense sentinel 벡터), HANDOFF.

- [ ] **Step 4: 커밋** — `git add tasks/HOTFIXES.md README.md docs/ARCHITECTURE.md HANDOFF.md && git commit -m "docs: dense 별칭 측정 결과 + 문서 동기화"`

---

## Self-Review

- **Spec 커버리지**: §3.2 sentinel→Task1. §3.3 config→Task2, ingest embed→Task3, retriever dedup→Task4, purge→Task3. §5 측정→Task5. §7 테스트→각 Task. ✅
- **Placeholder**: Task4 Step1 store fixture 는 "기존 패턴 따름"으로 위임(단언 핵심 명시). VectorHit.chunk_id pub 여부는 "확인 후 분기" 지시. 나머지 완성 코드. ✅
- **타입 일관성**: `ALIAS_SUFFIX`/`strip_alias_suffix`(Task1, kebab_core) ↔ ingest(Task3)·retriever(Task4) 사용. `embed_aliases`(Task2 config) ↔ ingest(Task3). VectorRecord 필드(Task3) = 기존 body records 와 동일 구조. ✅

---

## Execution Handoff

OMC teammate(sequential single-team). Task1·2=sonnet(작은), Task3·4=opus(임베딩/retriever 핵심). Task3/4 후 code-reviewer(opus, sentinel dedup·purge 정확성·회귀). Task5 측정은 main 세션 직접.
