# IMPL REPORT — 상세 ingest 진행 로깅 (feat/ingest-progress-detail, v0.24.0)

## 요약

asset(문서) 단위뿐이던 ingest 진행 이벤트에 **asset 내부 phase 가시성**을
추가했다. 큰 문서 하나가 expansion(별칭 LLM, 청크당 순차)으로 수십 분 걸려도
진행바가 `1/N`에 멈춘 듯 보이던 문제를 해결 — 청크 수 · expansion 라이브
카운터 · phase별 소요시간을 노출한다. wire `ingest_progress.v1`은
**additive(backward-compat)** 유지.

## 추가한 이벤트 (`crates/kebab-app/src/ingest_progress.rs` `IngestEvent`)

`#[serde(tag="kind", rename_all="snake_case")]` 이므로 신규 변이 추가 = wire v1
호환.

| 이벤트 | 필드 | emit 시점 / 경로 |
|--------|------|------------------|
| `asset_chunked` | `idx, total, chunks` | 청킹 직후(expansion/embed 전). markdown / image / pdf 세 경로 모두 |
| `expansion_progress` | `idx, total, done, chunks` | expansion 루프 중 **스로틀**(매 25청크 또는 ≥1s, 종료 시 `done==chunks` 1프레임 더). markdown, expansion enabled 시 |
| `asset_timings` | `idx, total, parse_ms, chunk_ms, expansion_ms, embed_ms, store_ms` | asset markdown 파이프라인 종료 시 1회. **markdown 경로만** |

### 설계 결정 — `AssetTimings` 이벤트 (vs `AssetFinished` 필드)

IMPL_BRIEF §1은 `AssetFinished`에 optional phase-timing 필드를, §2는 대안으로
신규 `AssetTimings` 이벤트(권장)를 제시했다. **후자를 택함**:

- `AssetFinished`는 호출부(`ingest_with_config_progress` 루프, lib.rs)에서
  생성되는데 timing 데이터는 `ingest_one_asset` **내부**에만 존재한다. 필드를
  채우려면 `kebab_core::IngestItem`(wire-stable struct) 변경 또는 별도 plumbing
  이 필요.
- `ingest_one_asset`는 이미 `progress` 핸들을 들고 있어 새 이벤트를 직접 emit
  하는 쪽이 crate 경계(`kebab-core` 불변, CLAUDE.md §Allowed/forbidden deps)도
  지키고 더 깔끔.
- `AssetFinished`는 손대지 않음 → 기존 consumer 전부 무변경.

`AssetTimings`의 5개 phase 필드는 `u64`(이벤트 emit 시 항상 존재). markdown
경로만 emit하므로 optional 처리 불필요.

## 변경 파일

| 파일 | 변경 |
|------|------|
| `crates/kebab-app/src/ingest_progress.rs` | 3개 신규 변이 + ordering-invariant doc comment 갱신 + 직렬화 단위 테스트 3개 |
| `crates/kebab-app/src/lib.rs` | `ingest_one_asset`/`ingest_one_image_asset`/`ingest_one_pdf_asset` 시그니처에 `idx,total` 스레딩(image엔 `progress`도). markdown 경로: `Instant` phase 타이머(parse/chunk/expansion/embed/store) + `AssetChunked` 즉시 emit + expansion 루프 스로틀 `ExpansionProgress` + `AssetTimings` emit. image/pdf 경로: `AssetChunked` emit |
| `crates/kebab-cli/src/progress.rs` | `handle_human`에 3개 arm — `asset_chunked`→진행바 message `→ N chunks`, `expansion_progress`→message `별칭 확장 {done}/{chunks}`, `asset_timings`→`⏱ parse … · store …` 한 줄. `fmt_ms` 헬퍼(<1s=ms, ≥1s=1-decimal 초) + 단위 테스트. `--json`은 `emit_json` 자동 처리 |
| `crates/kebab-tui/src/ingest_progress.rs` | reducer match에 3개 신규 변이 no-op arm(상태바는 per-asset 카운터만 추적) — 컴파일 유지 |
| `crates/kebab-app/tests/ingest_progress.rs` | `progress_event_sequence_matches_design_section_2_4a` 를 v0.24.0 순서 불변식(`AssetStarted < AssetChunked < [ExpansionProgress*] < AssetTimings < AssetFinished`)을 견고하게 검증하도록 재작성 |
| `docs/wire-schema/v1/ingest_progress.schema.json` | 신규 kind 3개 + 필드(`done, parse_ms, chunk_ms, expansion_ms, embed_ms, store_ms`) additive 기재. `chunks` description 확장 |
| `Cargo.toml` (+ `Cargo.lock`) | workspace version 0.23.1 → **0.24.0** (additive minor) |
| `tasks/HOTFIXES.md` | 2026-06-02 dated entry |
| `README.md` | ingest 명령 row에 진행 표시 한 줄 |

## 검증 (exit code)

전부 `CARGO_TARGET_DIR=/build/out/cargo-target/target`, `-j 4`.

| 게이트 | 명령 | 결과 |
|--------|------|------|
| clippy | `cargo clippy --workspace --all-targets -j 4 -- -D warnings` | **exit 0** ✅ |
| test | `cargo test -p kebab-app -p kebab-cli -j 4 --no-fail-fast` | **exit 0** ✅ — 312 passed, 0 failed |

신규 테스트 전부 통과:
`asset_chunked_serializes_with_discriminator`,
`expansion_progress_serializes_with_discriminator`,
`asset_timings_serializes_all_phase_fields`,
`progress_event_sequence_matches_design_section_2_4a`(재작성),
`fmt_ms_switches_unit_at_one_second`, 그리고 CLI 통합
(`ingest_json_emits_line_delimited_progress_then_report` 등 4개, 실제 바이너리로
새 이벤트가 `--json`/human stderr에 흐르는지 검증).

### 테스트 인프라 주의 (내 변경과 무관)

`cargo test`(fail-fast default)의 첫 실행은 재작성 전 `progress_event_sequence`
실패에서 멈췄다. 재작성 후 두 번째 실행에서 cargo가 더 진행하다
`crates/kebab-cli/tests/cli_error_wire.rs`의 2개 테스트가 **spawn 단계**
(`cmd.output().unwrap()`)에서 실패. 원인: 이 테스트(와 `ingest_progress_cli.rs`)
가 바이너리를 **하드코딩된 `<worktree>/target/debug/kebab`** 로 찾는데, 브리프가
강제한 `CARGO_TARGET_DIR=/build/out/cargo-target/target` 리다이렉트 때문에 실제
바이너리는 `/build/out/cargo-target/target/debug/kebab`에 빌드됨. 즉 **사전
존재하던 테스트-인프라 경로 가정 문제이지 내 코드 회귀가 아님**.

검증: gitignore된 `target` 심링크(`→ /build/out/cargo-target/target`)로 경로를
해결한 뒤 전체 재실행 → `cli_error_wire` 포함 **312 passed / 0 failed** (위 표).
심링크는 검증 후 제거(deliverable 아님, gitignore라 미커밋).

> 후속 권장: 해당 테스트들의 `kebab_bin()`을 `env!("CARGO_BIN_EXE_kebab")`로
> 교체하면 `CARGO_TARGET_DIR` 리다이렉트에 무관해진다. 본 작업 범위 밖이라
> 미수정.

### 실제 동작 확인 (smoke, provider 기본=embedder on / expansion off)

작은 2-문서 markdown corpus ingest:

`--json` stream kind counts:
`scan_started:1, scan_completed:1, asset_started:2, asset_chunked:2,
asset_timings:2, asset_finished:2, completed:1, ingest_report.v1:1`

`asset_chunked` 샘플:
```json
{"chunks":2,"idx":1,"kind":"asset_chunked","schema_version":"ingest_progress.v1","total":2,"ts":"2026-06-02T13:56:11Z"}
```
`asset_timings` 샘플:
```json
{"chunk_ms":676,"embed_ms":109,"expansion_ms":0,"idx":1,"kind":"asset_timings","parse_ms":6,"schema_version":"ingest_progress.v1","store_ms":920,"total":2,"ts":"2026-06-02T13:56:12Z"}
```

human-mode(`KEBAB_PROGRESS=plain`) stderr:
```
ingest: 1/2 → 2 chunks
  ⏱ parse 3ms · chunk 673ms · expand 0ms · embed 96ms · store 33ms
```

`expansion_progress`는 expansion 비활성(원격 LLM 미연결)이라 미발신 — 단위/통합
테스트로 충분히 커버(브리프 §검증 명시).

## 잔여 리스크 / 한계

- **image/pdf phase timing 없음**: 두 경로는 `asset_chunked`만 emit, `asset_timings`
  미발신(phase shape가 OCR/caption이라 다름). 브리프가 허용한 범위.
- **expansion_progress 비-TTY 억제**: 스로틀에도 비-TTY human 모드에선 로그 폭주
  방지로 기본 억제(진행바 message로 커버, `--json`은 전량 발신).
- **expansion_progress 실측 미검증**: 원격 GPU Ollama 필요. 코드 경로는 단위/통합
  으로 검증했으나 라이브 카운터의 실제 rate/ETA 체감은 도그푸딩(원격 LLM 연결)
  에서 별도 확인 권장.
- 커밋만 수행, push/PR 미수행(메인 세션 처리).
