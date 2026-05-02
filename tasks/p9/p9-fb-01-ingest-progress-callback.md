---
phase: P9
component: kebab-app + kebab-core
task_id: p9-fb-01
title: "Ingest progress callback / event channel"
status: completed
depends_on: []
unblocks: [p9-fb-02, p9-fb-03]
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§7 ingest, §10 UX]
source_feedback: p9-dogfooding-feedback.md item 1
---

# p9-fb-01 — Ingest progress callback

## Goal

`kebab_app::ingest_with_config` 가 진행 상황을 caller 에게 흘려보낼 수 있도록 progress callback (또는 mpsc Sender) 주입 surface 추가. CLI / TUI / desktop 셋 모두 같은 이벤트 stream 소비.

## Why now

도그푸딩 시 ingest 가 1.8 초 묵음 후 결과만 출력 — hung 인지 빈 워크스페이스인지 구분 불가. progress event 가 모든 UI surface 의 prerequisite.

## Allowed dependencies

- 기존 kebab-app deps. 신규 X.
- `std::sync::mpsc` 또는 `crossbeam_channel`.

## Public surface

```rust
#[derive(Debug, Clone)]
pub enum IngestEvent {
    ScanStarted { root: PathBuf },
    ScanCompleted { total: u32 },
    AssetStarted { idx: u32, total: u32, path: String, media: MediaKind },
    AssetFinished { idx: u32, kind: IngestItemKind, chunks: u32 },
    EmbedBatchStarted { n_chunks: u32 },
    EmbedBatchFinished { n_chunks: u32, ms: u64 },
    Aborted { partial_counts: AggregateCounts },
    Completed { counts: AggregateCounts },
}

#[doc(hidden)]
pub fn ingest_with_config_progress(
    config: kebab_config::Config,
    scope: SourceScope,
    summary_only: bool,
    progress: Option<Sender<IngestEvent>>,
) -> anyhow::Result<IngestReport>;
```

기존 `ingest_with_config` 는 `progress=None` 으로 forwarding wrapper.

## Behavior contract

- progress event 발신은 best-effort. receiver drop 되면 이후 send 무시 (panic 금지).
- 이벤트 ordering: `ScanStarted < ScanCompleted < (AssetStarted < AssetFinished)* < Completed|Aborted`. embed batch 는 asset 사이 임의 위치.
- `Aborted` 이벤트는 cancellation token (p9-fb-04) trigger 시에만 발신. CLI / TUI 의 cancel 신호 wiring 은 각각 p9-fb-04, p9-fb-03 에서 구현.
- `--json` CLI 는 line-delimited 형태로 dump (`schema_version=ingest_progress.v1`) — 별도 task (p9-fb-02).

## Test plan

| kind | description |
|------|-------------|
| unit | `Sender<IngestEvent>` 가 ScanStarted → ScanCompleted → Asset* → Completed 순서로 받는다 |
| integration | tmp workspace 3 md → 받은 이벤트 sequence 가 monotonic idx |

## DoD

- [ ] `cargo test -p kebab-app` 통과
- [ ] 기존 `ingest_with_config` 호출자 (CLI 단발 호출) 변경 없음
- [ ] HOTFIXES 항목 X — 신기능, deviation 아님

## Out of scope

- progress event JSON 직렬화 (별도 wire schema task)
- TUI 가 이벤트 소비해서 status bar 그리기 (p9-fb-03)
