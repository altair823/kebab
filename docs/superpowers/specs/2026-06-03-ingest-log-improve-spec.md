# Spec: ingest 로그 개선 (파일명·phase·heartbeat·slowest 요약)

**날짜**: 2026-06-03
**유형**: feature (관측성/UX, additive wire)
**근거**: arctic 도그푸딩 중 Obsidian 볼트(이미지/PDF 혼재 + OCR/caption on)에서 ingest 가 중간부터 느려졌는데, **TTY 진행바가 파일명·현재 phase·모델·경과시간을 안 보여줘** "멈춘 것처럼" 보였다. 원인(비전 모델 스와핑)을 로그만으로 파악 불가. v0.24.0 상세 진행 로깅의 후속 — 느린 phase(특히 이미지 OCR/caption)와 병목 파일을 가시화한다.

## 현재 한계 (코드 근거)
- `kebab-cli/src/progress.rs:145` — TTY 에서 AssetStarted 는 **위치만 갱신, 파일명 메시지 미설정**(의도적; 비-TTY 줄에만 파일명). → 인터랙티브 실행 시 현재 파일 안 보임.
- 이미지 **OCR/caption 진행 이벤트 없음** — `PdfOcrStarted/Finished`(PDF 페이지)만 존재. 이미지 OCR/caption(gemma 비전, 느림)은 무이벤트 → 진행바 정지처럼 보임(`lib.rs` apply_ocr/apply_caption 호출 주변).
- 한 asset 이 오래 걸려도 **경과시간 heartbeat 없음**(완료 후 `AssetTimings` ⏱ 한 번).
- 병목 파일을 **사후 파악할 요약 없음**.

## 목표 (사용자 결정: 1+2+3+4)
1. **파일명**을 TTY 진행바 메시지에 표시.
2. 느린 **phase(OCR/caption/embed) + 모델명** 실시간 표시.
3. 현재 asset **경과시간 heartbeat**.
4. 종료 시 **가장 오래 걸린 파일 top-N 요약**.

## 작업

### A. wire 이벤트 (additive, ingest_progress.v1)
- **신규 `AssetPhase { idx, total, phase, model }`** — asset 이 느린 phase 진입 시 emit. `phase: &str` ∈ {`"ocr"`,`"caption"`,`"embed"`}; `model: Option<String>`(그 phase 를 수행하는 모델 — OCR/caption=비전 LLM 모델 id, embed=임베더 model_id). 짧은 phase(parse/chunk/store)는 emit 안 함(노이즈 방지).
- **`AssetTimings` 확장**: `ocr_ms`, `caption_ms` 필드 추가(additive, 기본 0). 기존 parse/chunk/embed/store/expansion_ms 유지. → top-N 요약의 정확한 per-asset 총시간 계산 근거.
- `PdfOcrStarted/Finished`(기존) 유지 — PDF 페이지 단위 진행은 이미 있음.
- wire schema `docs/wire-schema/v1/ingest_progress.schema.json`: `asset_phase` kind + `phase`/`model` + `ocr_ms`/`caption_ms` 필드 문서화(additive, v1 유지).

### B. emit 지점 (kebab-app)
- `ingest_one_asset` / 이미지·미디어 경로(`apply_ocr`/`apply_caption` 호출 직전, `lib.rs:~1568/1586`): 각각 `AssetPhase{phase:"ocr"|"caption", model}` emit. 임베딩 루프 진입 시 `AssetPhase{phase:"embed", model:embedder.model_id}` emit(텍스트 asset 도 적용).
- OCR/caption 소요를 측정해 `AssetTimings.ocr_ms`/`caption_ms` 채움.

### C. CLI 렌더 (kebab-cli/src/progress.rs)
1. **파일명**: AssetStarted TTY 핸들러에서 `bar.set_message(<path 축약>)`(현재 위치-only 주석 제거). 비-TTY 줄은 그대로.
2. **phase+모델**: AssetPhase 수신 시 `bar.set_message("{path} · {phase}({model})…")`.
3. **heartbeat**: AssetStarted 에서 현재 asset 시작 시각 기록 + steady-tick(예: 1s)으로 메시지 끝에 `(Ns)` 경과 갱신. asset 전환/완료 시 리셋.
4. **slowest 요약**: AssetStarted(idx→path) + AssetTimings(idx→총ms=parse+chunk+embed+store+ocr+caption) 를 누적, `Completed` 수신 시 stderr 에 `⏱ 최장 소요 top-N`(기본 N=5) 표 출력. 비-TTY/quiet 에서도 요약은 출력(유용), `--json` 모드는 미출력(ndjson 오염 방지).

### 결정 사항
- 모두 **additive wire** → `ingest_progress.v1` 유지(major bump 없음). 신규 소비자는 `asset_phase` 부재 허용.
- AssetPhase 는 **emit 스로틀 불필요**(asset·phase 당 1회, 빈도 낮음). PDF 페이지 OCR 은 기존 PdfOcrStarted 가 담당(페이지 많으면 그쪽 스로틀은 별도 — 본 spec 비범위).
- top-N 의 N: 상수 5(후속에 config 화 가능, 본 spec 비범위).
- `--quiet` 시 진행바·phase 메시지는 억제하되 **slowest 요약은 출력**(짧고 유용). `--json` 은 전부 ndjson 으로만.

## 검증 기준
- clippy 0 / 전체 test 통과(기존 진행 렌더 테스트 갱신 + 신규 이벤트 직렬화 테스트).
- TTY 스모크: 이미지/PDF 포함 폴더 ingest 시 진행바에 **파일명 + OCR/caption/embed phase + 모델 + 경과초** 표시, 종료 시 **top-N 요약**.
- 비-TTY: 기존 줄 로그 유지 + 종료 요약.
- `--json`: `asset_phase`/확장 `asset_timings` ndjson 출력, 사람용 텍스트 미혼입.
- wire schema 문서 동기화 + verbatim 일치(CI diff-check 있으면).

## 도그푸딩 (별도)
사용자 Obsidian 볼트(이미지/PDF + OCR on)로 재현 — 느린 구간에서 어떤 파일·phase·모델인지 즉시 보이는지, 종료 요약이 병목 파일을 짚는지 확인. HOTFIXES + release notes.

## 문서 동기화 (같은 PR)
- `docs/wire-schema/v1/ingest_progress.schema.json` (asset_phase, ocr_ms/caption_ms).
- README(진행 표시 설명 있으면 갱신, 명령표 영향 없음), HANDOFF 1줄, tasks/HOTFIXES dated entry, Cargo.toml version minor bump.

## 비범위
- PDF 페이지 OCR 진행 스로틀/요약(기존 이벤트 유지).
- 모델 스와핑 자체 해결(그건 Ollama 설정/OCR off — 본 작업은 가시화만).
- top-N 의 config 화.
