---
phase: P9
component: kebab-store-sqlite
task_id: p9-fb-17
title: "SQLite V004 — chat_sessions / chat_turns"
status: planned
depends_on: [p9-fb-15]
unblocks: [p9-fb-18]
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§5 storage]
source_feedback: p9-dogfooding-feedback.md item 13, 14
---

# p9-fb-17 — Chat session storage

## Goal

multi-turn 대화 영속화. session 단위로 turn 저장 / 조회. TUI 의 "이전 대화 이어가기", CLI `--session <id>` (p9-fb-18) 의 backing store.

## Allowed dependencies

- `kebab-store-sqlite` 기존 deps (rusqlite, refinery).

## Public surface

마이그레이션 `migrations/V004__chat_sessions.sql`:

```sql
CREATE TABLE chat_sessions (
    session_id   TEXT PRIMARY KEY,           -- 사용자 지정 또는 blake3 해시
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL,
    title        TEXT,                        -- 첫 question 의 첫 N 자
    config_snapshot_json TEXT NOT NULL        -- prompt_template_version, llm model 등
);

CREATE TABLE chat_turns (
    turn_id      TEXT PRIMARY KEY,           -- blake3(session_id || index)
    session_id   TEXT NOT NULL REFERENCES chat_sessions(session_id) ON DELETE CASCADE,
    turn_index   INTEGER NOT NULL,
    question     TEXT NOT NULL,
    answer       TEXT NOT NULL,
    citations_json TEXT NOT NULL,             -- Vec<Citation> 직렬화
    created_at   INTEGER NOT NULL,
    UNIQUE(session_id, turn_index)
);
CREATE INDEX idx_chat_turns_session ON chat_turns(session_id, turn_index);
```

`kebab_core::ChatSessionRepo` trait + `SqliteStore` impl.

## Behavior contract

- session_id 사용자 명시 (`kebab ask --session foo`) 또는 자동 생성 (blake3 of first_question + ts).
- turn_index monotonic per session.
- `ON DELETE CASCADE` — `kebab reset --data-only` (p9-fb-06) 가 wipe.
- `config_snapshot_json` 는 prompt_template_version + llm.model + max_context_tokens 등 — 후일 retrospective 분석 가능.

## Test plan

| kind | description |
|------|-------------|
| unit | session 생성, 3 turn append, list_turns sequence |
| unit | session delete → CASCADE turns |
| migration | V004 apply 후 schema_version table 갱신 |

## DoD

- [ ] `cargo test -p kebab-store-sqlite` 통과
- [ ] `migrations/V004__chat_sessions.sql` 추가
- [ ] `kebab_core::ChatSessionRepo` trait 정의
- [ ] frozen design §5 storage 절에 chat_sessions / chat_turns 추가

## Out of scope

- session 검색 / 필터 UI (P+)
- 다른 store backend (postgres 등)
