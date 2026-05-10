---
phase: P9
component: kebab-embed-local + kebab-config + kebab-store-vector + docs
task_id: p9-fb-39b
title: "Embedding model upgrade (multilingual-e5-large)"
status: completed
target_version: 0.7.0
depends_on: [p9-fb-39]
unblocks: []
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§4 search, §5 storage, §9 versioning cascade]
source_feedback: 사용자 도그푸딩 2026-05-06 — Claude Code 가 kebab CLI 사용 후 "rank 5+ 노이즈 섞임" 지적 (fb-39 의 lever 적용 측면).
---

# p9-fb-39b — Embedding model upgrade

> ✅ **구현 완료.** fb-39 의 lever 후보 4개 중 embedding model 업그레이드 lever 적용. P@k metric (fb-39) 으로 small vs large 비교 가능.
>
> - Design update: [`docs/superpowers/specs/2026-04-27-kebab-final-form-design.md`](../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md) §5 / §9
> - Plan: [`docs/superpowers/plans/2026-05-10-p9-fb-39b-embedding-upgrade.md`](../../docs/superpowers/plans/2026-05-10-p9-fb-39b-embedding-upgrade.md)

## 요약

- `multilingual-e5-small` (384 dim) → `multilingual-e5-large` (1024 dim) default flip.
- 기존 user TOML 이 small 명시 시 그대로 유지 (backwards-compat).
- fb-23 incremental ingest 가 embedding_version mismatch 감지 → 자동 re-embed.
- 0.6 → 0.7 minor bump 트리거 (design §9 cascade rule).

## 구현 항목

1. **config defaults flip** — `[models.embedding] model = "multilingual-e5-large"`, `dimensions = 1024`.
2. **fastembed e5-large resolution** — `kebab-embed-local` 의 `resolve_model()` 에 e5-large arm 추가.
3. **fixture sweep** — 모든 unit/integration 테스트의 default embedding 모델 확인. Config 에서 명시하지 않으면 새 default 따름 (`provider = "none"` 테스트 제외).
4. **design contract update** — design §5 (storage example) + §9 (versioning table) 의 embedding_model.id + dimensions 갱신.
5. **HOTFIXES entry** — 사용자 재 ingest 절차 + backwards-compat 동작 명시.
6. **README update** — `[models.embedding]` 섹션의 기본값 + `dimensions` 필드 설명 갱신.
7. **SMOKE.md append** — 스모크 테스트 중 embedding 업그레이드 검증 절차 (reset → config 갱신 → ingest → eval).
8. **tasks/INDEX.md append** — p9-fb-39b row 추가 (p9-fb-39 sibling).

## Allowed dependencies

- `kebab-embed-local` — fastembed crate + `kebab-core`
- `kebab-config` — toml crate
- `kebab-store-vector` — lancedb crate (table naming 로직만 영향)
- `kebab-app` — 와이어링만 (API 변경 없음)

## Forbidden dependencies

- parse-* crate (parser 무관)
- llm-* crate (embedding 과 무관)
- search crate (검색 로직은 adapter pattern 으로 이미 generic)

## Test

- `cargo test -p kebab-embed-local -- e5_large` (새 arm 테스트)
- `cargo test -p kebab-config -- embedding_defaults` (config defaults)
- `cargo test --workspace --no-fail-fast -j 1` (full regression)
- Smoke: `kebab --config /tmp/smoke.toml doctor | grep embedding` → `multilingual-e5-large (1024d)`
- Smoke: `kebab --config /tmp/smoke.toml ingest` → embedding 진행 표시 + dimension check
- P@k eval: `kebab eval run` (fb-39 의 golden set) small vs large 비교

## Backward compat notes

- Pre-fb-39b user 가 config 에서 명시하지 않은 embedding → new default (large) 자동 적용. TOML 에 `model = "multilingual-e5-small"` 명시하면 유지.
- `kebab_app::config::Config` 의 `embedding_model` field 는 Optional 이므로 old config (small) 도 parse 성공 (v1 설계 §9 cascade 규칙).
- Orphan LanceDB table (`chunk_embeddings_multilingual-e5-small_384`) 은 다음 `kebab ingest` 실행 후 stale 취급 — 사용자가 수동 `kebab reset --vector-only` 로 정리 가능.

## Binary version bump

- 0.6.0 → 0.7.0 (design §9 cascade rule: embedding_model change = minor bump).
- Release notes: `embedding default: multilingual-e5-small (384d) → multilingual-e5-large (1024d), P@k metric ↑`.

## Post-merge deviation

None — 설계 contract 대로 구현 완료.
