---
title: "p9-fb-39b — Embedding model upgrade design (multilingual-e5-large)"
phase: P9
component: kebab-embed-local + kebab-store-vector + kebab-config + kebab-app
task_id: p9-fb-39b
status: design
target_version: 0.7.0
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§4 search, §5 storage, §9 versioning cascade]
date: 2026-05-10
---

# p9-fb-39b — Embedding model upgrade

## Goal

fb-39 의 lever 적용 — embedding model 을 `multilingual-e5-small` (384 dim) 에서 `multilingual-e5-large` (1024 dim) 로 업그레이드. 도그푸딩 한국어 corpus 의 retrieval precision 개선.

fb-39 가 측정 도구 (P@5 / P@10) 를 추가했으므로, 본 PR 머지 후 small vs large 비교 가능.

`bge-m3` 검토했으나 fastembed 4.9.1 의 `EmbeddingModel` enum 에 미포함 — `UserDefinedEmbeddingModel` ONNX 직접 로드 path 는 별도 작업 (fb-39c 후보). 본 PR scope = e5-large 만.

## Behavior contract

### Embedding model

- 신규 default: `multilingual-e5-large` (1024 dim).
- `kebab-embed-local::resolve_model` 에 신규 arm:

```rust
"multilingual-e5-large" => Ok(EmbeddingModel::MultilingualE5Large),
```

기존 `multilingual-e5-small` arm 그대로 (backwards-compat opt-out).

### Config defaults

- `Config::defaults().models.embedding.model`: `"multilingual-e5-small"` → `"multilingual-e5-large"`.
- `Config::defaults().models.embedding.dimensions`: `384` → `1024`.
- `kebab init` 가 생성하는 config.toml 템플릿 동일 갱신.

기존 user TOML 이 `model = "multilingual-e5-small"` 또는 `dimensions = 384` 명시한 경우 그대로 유지 — `serde` 가 user value 우선. opt-out 가능.

### Cascade

- `embedding_version`: 자동 변경 (config.models.embedding.model 값 그대로 wire 에 emit). `multilingual-e5-small` → `multilingual-e5-large`.
- fb-23 incremental ingest: 4-input match (blake3 + parser_version + chunker_version + embedding_version) 에서 embedding_version 깨짐 → 모든 chunk 재-embed. text/parse/chunk 비용 회피, embed 비용만 발생.
- `eval_runs.config_snapshot_json`: 새 version 자동 기록. 비교 시 동일 version 끼리.
- design §9 cascade rule 의 5 키 중 `embedding_version` 변경 — binary release 트리거 (CLAUDE.md `Versioning cascade` 룰).

### Migration policy

LanceDB stored vectors 의 dim 과 `config.models.embedding.dimensions` 가 mismatch 면:

- `LanceVectorStore::open` (또는 첫 호출) 가 비교 → mismatch 시 신규 `ErrorV1`:
  - `code = "embedding_dim_mismatch"`
  - `message`: `"vector index dim 384 vs config dim 1024"`
  - `hint`: `"기존 vector index 가 4-dim, config 는 N-dim. 'kebab reset --vector-only && kebab ingest' 로 재구축."`
- CLI: exit 1 + error.v1 stderr (또는 비-`--json` 모드 plain stderr).
- silent migration / auto-wipe 안 함 — 사용자 명시 동의 필요.

remediation flow:

```
$ kebab search "..."
error: vector index dim 384 vs config dim 1024

Hint: 기존 vector index 가 384-dim, config 는 1024-dim.
'kebab reset --vector-only && kebab ingest' 로 재구축.

$ kebab reset --vector-only
[wipe LanceDB + SQLite embedding_records]

$ kebab ingest
[full re-embed with new model — fastembed downloads e5-large ONNX (~1.3 GB) on first run]
```

### Wire shape

신규 wire field 없음. `error.v1.code` 의 valid value namespace 에 `"embedding_dim_mismatch"` 추가 (string, enum 아님 — additive).

## Allowed / forbidden dependencies

- `kebab-embed-local`: 신규 dep 없음. fastembed enum variant 추가만.
- `kebab-store-vector`: 신규 dep 없음. LanceDB schema reader 사용.
- `kebab-config`: 신규 dep 없음. defaults 값 변경.
- `kebab-app`: 신규 dep 없음. error propagation.

`kebab-core` 의 다른 `kebab-*` 의존 금지 룰 그대로.

## Public surface delta

### kebab-embed-local (`lib.rs`)

```rust
fn resolve_model(name: &str) -> Result<EmbeddingModel> {
    match name {
        "multilingual-e5-small" => Ok(EmbeddingModel::MultilingualE5Small),
        "multilingual-e5-large" => Ok(EmbeddingModel::MultilingualE5Large),  // 신규
        other => anyhow::bail!(/* ... */),
    }
}
```

### kebab-config (defaults + TOML 템플릿)

```rust
EmbeddingCfg {
    provider: "fastembed".to_string(),
    model: "multilingual-e5-large".to_string(),
    dimensions: 1024,
    // ... 기타 ...
}
```

generated config.toml 템플릿 도 같이 갱신.

### kebab-store-vector (`lib.rs` 또는 신규 helper)

```rust
impl LanceVectorStore {
    pub fn open(...) -> Result<Self> {
        // 기존 open 로직 ...
        let stored_dim = read_schema_vector_dim(&table)?;
        if stored_dim != config_dim {
            anyhow::bail!(StructuredError(ErrorV1 {
                code: "embedding_dim_mismatch".to_string(),
                message: format!("vector index dim {stored_dim} vs config dim {config_dim}"),
                hint: Some(format!(
                    "기존 vector index 가 {stored_dim}-dim, config 는 {config_dim}-dim. \
                     'kebab reset --vector-only && kebab ingest' 로 재구축."
                )),
                // ...
            }));
        }
        Ok(...)
    }
}
```

(정확한 LanceDB schema reading API 는 구현 시 확인 — `Table::schema()` 또는 `arrow_schema::Schema` 직접 inspect.)

## Test plan

| kind | description |
|------|-------------|
| unit (kebab-embed-local) | `resolve_model("multilingual-e5-large")` returns Ok |
| unit (kebab-embed-local) | `check_dim(1024, 1024)` ok |
| unit (kebab-embed-local) | `check_dim(384, 1024)` Err — message mentions both dims |
| unit (kebab-config) | `Config::defaults().models.embedding.model == "multilingual-e5-large"` |
| unit (kebab-config) | `Config::defaults().models.embedding.dimensions == 1024` |
| unit (kebab-config) | TOML `model = "multilingual-e5-small"` deserialize 정상 (backwards-compat) |
| unit (kebab-config) | 생성된 config.toml 템플릿 안 `model = "multilingual-e5-large"`, `dimensions = 1024` |
| unit (kebab-store-vector) | mismatch fixture (384-dim stored + 1024 cfg) → `embedding_dim_mismatch` ErrorV1 |
| 통합 (kebab-cli) | mismatch scenario — pre-existing 384-dim DB + new config → exit 1 + error.v1 stderr (`code = embedding_dim_mismatch`) + hint mentions reset --vector-only |
| 통합 (kebab-cli) | small config 로 fresh ingest + search → 정상 (backwards-compat path 검증) |

`multilingual-e5-large` 모델 다운로드 회피 위해 unit/integration 테스트는 fixture 또는 mock — 실 모델 호출 안 함. 첫 도그푸딩 시 사용자가 fastembed cache 다운로드.

## Implementation steps (high-level)

1. `kebab-embed-local::resolve_model` arm + check_dim 단위 테스트.
2. `kebab-store-vector` dim mismatch detection + ErrorV1 + 단위 테스트.
3. `kebab-config` defaults flip + TOML 템플릿 + 단위 테스트.
4. `kebab-cli` integration: mismatch error.v1 wire + backwards-compat path 통합 테스트.
5. README + SMOKE + design + HOTFIXES + status flip.

5 task. 단일 PR, single 세션 가능.

## Risks / notes

- **첫 실행 모델 다운로드**: e5-large ONNX ~1.3 GB. fastembed cache (`config.storage.model_dir/fastembed/`) 에 자동 다운로드 (첫 호출 시). progress 표시 없음 — 사용자 침묵 latency. `kebab doctor` 또는 README 에 경고 안내.
- **Search/ingest latency**: e5-large 가 e5-small 대비 ~3-4× embedding 시간. ingest 비용 증가 (one-time + 신규 docs). search 시 query embed per-call 증가.
- **Disk usage**: vector dim 2.6× → LanceDB 약 2.7× 증가.
- **HOTFIXES entry**: dim mismatch UX (error.v1 + reset --vector-only flow) 가 frozen design 안 명시 안 된 신규 동작 — HOTFIXES 한 항목 추가.
- **eval comparison**: fb-39 P@k 가 측정 도구. 도그푸딩 corpus + golden 의 expected_chunk_ids 채워서 small vs large 정량 비교 별도 (PR 안 의무 아님).
- **fb-23 incremental ingest 와의 상호작용**: embedding_version 변경 → 모든 doc 재-embed. fb-23 의 unchanged path 는 한 번도 hit 안 함 (예상 동작).
- **release trigger**: design §9 cascade rule 의 `embedding_version` 변경 → CLAUDE.md `Versioning cascade` 룰에 따라 binary 0.6 → 0.7 minor bump 필요.

## Out of scope

- bge-m3 또는 user-defined ONNX path (fb-39c 후보).
- Other lever (RRF / cross-encoder / chunk policy).
- Auto-migration / background re-vector.
- LanceDB schema migration tooling (별도 wipe + re-ingest).
- multi-model coexistence (한 KB 안 small + large 동시).
- precision 정량 비교 의무 (별도 도그푸딩).

## Documentation updates (implementation PR 동시)

- `README.md` `[models.embedding]` config 섹션 — default 변경 + small opt-out 안내 + dim mismatch 시 reset 명령 안내.
- `docs/SMOKE.md` — upgrade walkthrough (`kebab reset --vector-only && kebab ingest` 시퀀스 + 첫 ONNX 다운로드 latency 경고).
- `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` §5 storage / §9 versioning 적절 절 — 새 default + dim 1024 명시.
- `tasks/HOTFIXES.md` — dim mismatch UX entry.
- `tasks/p9/p9-fb-39-retrieval-precision-tuning.md` banner — fb-39b lever 적용 (embedding upgrade) ✅ 추가 (단 spec status 는 fb-39 frozen).
- `tasks/p9/p9-fb-39b-embedding-upgrade.md` 신규 task spec (만들거나, fb-39 sub-task 로 frontmatter 처리).
- `tasks/INDEX.md` — fb-39b 행 추가 ✅.
- 본 PR 머지 후 `chore: bump version 0.6 → 0.7` + tag (CLAUDE.md release 절차).
