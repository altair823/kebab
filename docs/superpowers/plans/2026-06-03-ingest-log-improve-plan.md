# Plan: ingest 로그 개선 구현

spec: `docs/superpowers/specs/2026-06-03-ingest-log-improve-spec.md`. 브랜치 `feat/ingest-log-improve`. 빌드 `CARGO_TARGET_DIR=/build/out/cargo-target -j 4`(전체 test `-j 1`). cli 통합테스트용 `target` 심링크 후 정리.

## Task 1 — wire 이벤트 (kebab-app/src/ingest_progress.rs)
- `IngestEvent` 에 `AssetPhase { idx: u32, total: u32, phase: String, model: Option<String> }` variant 추가(serde tag 규약 기존과 동일, snake `asset_phase`).
- `AssetTimings` 에 `ocr_ms: u64`, `caption_ms: u64` 필드 추가(기존 필드 뒤, serde default 0 → 구 소비자 호환).
- 직렬화 테스트 추가(asset_phase, 확장 timings).

## Task 2 — emit 지점 (kebab-app/src/lib.rs)
- 이미지 경로: `apply_ocr` 직전 `AssetPhase{phase:"ocr", model: <ocr model>}`, `apply_caption` 직전 `AssetPhase{phase:"caption", model: <llm model>}` emit. 각 호출 시간 측정 → `ocr_ms`/`caption_ms`.
- 임베딩 루프 진입 직전 `AssetPhase{phase:"embed", model: embedder.model_id}` emit(텍스트 포함 전 asset).
- `AssetTimings` 생성부에 ocr_ms/caption_ms 전달.
- 짧은 phase(parse/chunk/store)는 emit 안 함.

## Task 3 — CLI 렌더 (kebab-cli/src/progress.rs)
- **파일명**: AssetStarted TTY 핸들러 `bar.set_message(<path>)` (현재 위치-only 주석/로직 교체; path 길면 말미 축약). 비-TTY 줄 유지.
- **phase+모델**: AssetPhase 수신 → `bar.set_message(format!("{path} · {phase}({model})…"))`. 현재 path 를 핸들러 상태로 보관(AssetStarted 에서 저장).
- **heartbeat**: AssetStarted 에서 `Instant::now()` 보관 + `bar.enable_steady_tick(1s)` + 메시지 렌더에 경과초 `(Ns)`. AssetFinished/다음 AssetStarted 에서 리셋. (indicatif steady-tick + 커스텀 메시지.)
- **slowest 요약**: 핸들러에 `Vec<(path, total_ms)>` 누적 — AssetStarted 로 idx→path, AssetTimings 로 idx→sum(parse+chunk+embed+store+ocr+caption). `Completed` 수신 시 상위 5개 stderr 표 출력(`⏱ 최장 소요:`). `--json` 모드 미출력, quiet 여도 요약은 출력.
- `fmt_ms`(기존) 재사용.

## Task 4 — wire schema + 문서
- `docs/wire-schema/v1/ingest_progress.schema.json`: `asset_phase` kind(phase enum, model) + `ocr_ms`/`caption_ms` 필드 추가(additive). verbatim 일치.
- README(있으면 진행 표시 한 줄), HANDOFF 1줄, tasks/HOTFIXES dated entry, Cargo.toml version minor bump(+Cargo.lock).

## Task 5 — 검증
- clippy 0, 전체 test 통과(기존 progress 테스트 갱신).
- 스모크: 이미지/PDF 포함 임시 폴더 ingest → TTY 파일명+phase+모델+경과, 종료 top-N. 비-TTY 줄+요약. `--json` ndjson(asset_phase/ocr_ms) 확인, 사람텍스트 미혼입.
- 결과 요약 `/tmp/ingestlog-result.md`(게이트 + 스모크 캡처).

## 리뷰 루프
완료 → 리더 clippy/test 독립 재확인 → `gitea-pr`(title `feat(ingest): 진행 로그 개선 — 파일명/phase/heartbeat/slowest 요약`) → 리뷰 루프 → 사용자 머지. 머지 후 Obsidian 볼트 도그푸딩.
