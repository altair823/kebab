# 상세 ingest 진행 로깅 (IMPL BRIEF)

너는 worktree `/build/out/kebab-worktrees/progress-detail` (브랜치 `feat/ingest-progress-detail`)의 executor 다.

## 동기 (왜)

현재 ingest 진행 이벤트는 **asset(문서) 단위**뿐이라(`AssetStarted`/`AssetFinished`), 한 문서 내부의 parse/chunk/**expansion(별칭 LLM, 청크당 순차 호출)**/embed/store 가 전부 깜깜하다. 그래서 큰 문서 하나가 expansion 으로 30분 걸려도 진행바는 `1/5150` 에 멈춘 듯 보이고, 사용자가 **병목을 못 본다**. 측정상 expansion 은 청크당 ~1~4s(원격 GPU Ollama), 큰 문서 = 청크 수백~천 개 → 그 한 문서에서 수십 분. embed(candle CPU)도 느릴 수 있다.

목표: **asset 내부 phase 를 노출**해 사용자가 어디서 시간이 가는지 즉시 보게 한다. 특히 expansion 라이브 카운터 + phase 별 소요시간.

## 구현 (정확히)

### 1) 신규 진행 이벤트 — `crates/kebab-app/src/ingest_progress.rs` `IngestEvent` enum

`#[serde(tag="kind", rename_all="snake_case")]` 이라 **변이 추가는 wire v1 호환(additive)**. 추가:

- `AssetChunked { idx: u32, total: u32, chunks: u32 }` — 청킹 직후, expansion/embed 전. "이 문서가 N청크" 를 즉시 노출.
- `ExpansionProgress { idx: u32, total: u32, done: u32, chunks: u32 }` — expansion 루프 중 **스로틀**해서 발신 (아래 §3). `done`=처리한 청크, `chunks`=전체 청크.

그리고 `AssetFinished` 에 **optional phase-timing 필드 추가** (additive, `#[serde(skip_serializing_if="Option::is_none")] Option<u64>`): `parse_ms, chunk_ms, expansion_ms, embed_ms, store_ms`. 기존 호출부가 깨지지 않게 — `AssetFinished` 생성 지점(검색해서 전부)에서 새 필드를 채우거나 `None`.

새 변이 + 새 필드에 대한 단위 테스트(직렬화 `kind` 판별 + skip_serializing) 추가 (기존 `ingest_progress.rs` 테스트 스타일 따라).

### 2) idx/total 스레딩 + phase 계측 — `crates/kebab-app/src/lib.rs` `ingest_one_asset`

- `ingest_one_asset` 시그니처에 `idx: u32, total: u32` 추가. 호출부(asset 루프, ~line 461-497)에서 `idx`(=`u32::try_from(zero_idx+1)`), `total`(=`scanned_count`) 전달. image/pdf 서브함수(`ingest_one_image_asset`/`ingest_one_pdf_asset`)에도 idx/total 전달(시그니처 추가) — 최소한 그들도 `AssetChunked` 는 emit (없으면 markdown 경로만 emit 하고 나머지는 phase timing 생략해도 됨; 단 idx/total 은 일관되게 전달).
- **markdown 경로**(`ingest_one_asset` 본문, ~1247-1510)에 `std::time::Instant` 타이머:
  - parse_ms: 진입~chunk 직전.
  - chunk_ms: `MdHeadingV1Chunker.chunk`(1289) 직후 측정 → **즉시 `AssetChunked{idx,total,chunks:chunks.len()}` emit** (`crate::ingest_progress::emit(progress, ...)`).
  - expansion_ms: expansion 블록(1299-1357) 전체.
  - embed_ms: embed+upsert 블록(1387~) 전체.
  - store_ms: `put_chunks`(1381) 등 저장.
  - `AssetFinished` 는 호출부에서 만들어진다(현 코드 확인) — phase timing 을 거기로 넘기려면 `IngestItem` 에 timing 을 실어 보내거나, **간단히: ingest_one_asset 가 AssetFinished 의 timing 을 직접 emit 하지 말고**, 호출부 AssetFinished emit 지점에서 쓸 수 있도록 `IngestItem` 에 optional timing 필드를 추가하는 대신 — **더 단순한 길**: phase timing 을 `ingest_one_asset` 가 자체적으로 `tracing::info!` + 새 이벤트로 넘기기 부담되면, `AssetFinished` 의 timing 은 **호출부에서 측정** (ingest_one_asset 호출 전후 Instant 로 total 만) + 내부 세부(parse/chunk/expansion/embed)는 ingest_one_asset 가 `AssetChunked`/`ExpansionProgress` 로 노출. **결정: phase 별 ms 는 ingest_one_asset 가 `IngestItem` 에 optional 필드로 실어 반환 → 호출부가 AssetFinished 에 채운다.** `kebab_core::IngestItem` 에 optional timing 필드 추가가 부담되면, 차선으로 ingest_one_asset 내부에서 직접 phase timing 을 담은 `AssetFinished`-호환 정보를 progress 로 별도 이벤트(`AssetTimings{idx, parse_ms,...}`)로 emit. **둘 중 더 깔끔한 쪽을 택하되, 기존 wire/contract 깨지 말 것.** (권장: 신규 `AssetTimings` 이벤트 — IngestItem/wire 변경 회피.)

### 3) expansion 루프 스로틀 emit — lib.rs:1316 `for chunk in &mut chunks`

- 루프에 `done` 카운터 + 마지막 emit 시각(`Instant`). **매 청크마다 emit 금지**(채널 폭주) — `done % 25 == 0 || last_emit.elapsed() >= Duration::from_secs(1)` 일 때 `ExpansionProgress{idx,total,done,chunks:chunks.len()}` emit. 루프 종료 후 마지막 한 번 더(done==total) emit.
- 캐시 히트 청크도 done 에 포함(빠르게 지나감을 보여줌).

### 4) CLI 렌더링 — `crates/kebab-cli/src/progress.rs` `handle_human`

- `AssetChunked` → 현재 asset 라인에 `→ N chunks` 표시(또는 메시지 업데이트). expansion 서브-진행의 total 설정.
- `ExpansionProgress` → 라이브 메시지 `별칭 확장 {done}/{chunks}` (가능하면 rate/eta). indicatif 메시지 업데이트(기존 bar 활용; per-asset position 은 AssetStarted 에서 이미 advance 됨).
- `AssetTimings`(또는 AssetFinished timing) → asset 종료 시 한 줄 `parse Xs · chunk Ys · expand Zs · embed Ws · store Vs`.
- `ProgressMode::Json` 은 `emit_json` 이 임의 이벤트 직렬화하므로 자동 처리(확인만).
- `quiet` 모드는 기존대로 억제.

### 5) wire 스키마 문서 — `docs/wire-schema/v1/ingest_progress.v1.*`

- 신규 이벤트/필드를 additive 로 기재 (기존 파일 형식 따라). v1 유지(additive minor).

### 6) 버전 + 문서

- 워크스페이스 `Cargo.toml` version 0.23.1 → **0.24.0** (신규 wire 이벤트 + CLI 진행 surface = additive minor).
- `tasks/HOTFIXES.md` dated entry, README 진행 표시 관련 한 줄(있으면).

## 제약 / 검증

- `CARGO_TARGET_DIR=/build/out/cargo-target/target`, 빌드/테스트 직렬 `-j 4`, 무거운 빌드 `run_in_background`.
- wire v1 **호환 유지**(기존 consumer 가 깨지면 안 됨 — 새 변이/필드만 추가, 기존 필드 rename/삭제 금지).
- 채널 폭주 방지(스로틀 필수). best-effort emit 규약(드롭된 receiver 무시) 유지.
- **검증 게이트**: `cargo clippy --workspace --all-targets -j 4 -- -D warnings` exit 0; `cargo test -p kebab-app -p kebab-cli -j 4` exit 0(특히 ingest_progress 테스트 + progress.rs 테스트); 각 결과 exit code 로 검증(주장 금지).
- 실제 동작 확인: 작은 corpus + `provider=none`(빠름)로 ingest 해 `AssetChunked`/timing 라인이 뜨는지 + `--json` 에 새 `kind` 가 나오는지 확인. (expansion 라이브 카운터는 expansion 켜야 보이나, 원격 LLM 필요하니 단위/통합으로 충분.)

## 산출물

`/build/out/kebab-worktrees/progress-detail/IMPL_REPORT.md`: 추가한 이벤트/필드, 변경 파일, 빌드/clippy/test exit code, --json 새 kind 샘플, 잔여 리스크. `feat/ingest-progress-detail` 에 커밋(push/PR 금지 — 메인 세션이 처리).

막히면 IMPL_REPORT 에 적고 멈춰라. wire v1 호환만은 절대 깨지 말 것.
