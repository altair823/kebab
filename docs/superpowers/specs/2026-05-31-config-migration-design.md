# config 마이그레이션 — 설계 (spec)

> 2026-05-31. config.toml **스키마 진화 시 기존 사용자 파일을 자동 갱신**하는 기능의
> 설계 계약. kickoff 인계 문서
> [`2026-05-31-config-migration-kickoff.md`](../handoffs/2026-05-31-config-migration-kickoff.md)
> 의 brainstorm 결과를 확정한 spec 이다. 본 문서를 기준으로 plan → 구현.

## 0. 결정 요약 (brainstorm 게이트)

| 축 | 결정 | 근거 |
|----|------|------|
| **트리거** | 명시 명령 `kebab config migrate` + `kebab doctor` 안내 | 예측 가능성·안전. load 시 자동 쓰기는 쓰기 권한/동시 실행/손상 위험. |
| **주석 보존** | `toml_edit` 부분 편집 | 사용자가 손본 값·주석·순서·정렬 100% 보존. 빠진 것만 추가. |
| **버전 메커니즘** | reconciliation(additive) + step 체인(non-additive) 하이브리드 | kebab config 는 `schema_version` 이 줄곧 `1` 인 채로 섹션이 누적돼 버전 번호만으로 "무엇이 빠졌는지" 구분 불가 → 구조 비교가 본질. |

## 1. 동기 (kickoff §1 재확인)

v0.21.0 에서 `[ingest.expansion]` 등 섹션이 늘었지만, 기존 사용자 config.toml 은
serde default 로 **동작은 호환**(off 로 로드)되나 그 섹션이 **파일에 써지지 않아**
사용자가 파일을 열어도 새 기능의 존재·노브를 알 수 없다. DB 는 V00X refinery
마이그레이션이 있는데 config 는 없다 — 이걸 만든다.

핵심: **데이터 무효화가 아니라 *파일 가시성* 문제**. 읽기 호환성은 이미 확보돼 있으므로
(`#[serde(default)]`), 만들 것은 *사용자 파일을 새 스키마에 맞춰 갱신*하는 것이다.

## 2. 비목표 (YAGNI)

- config 값의 **의미적 검증**(예: score_gate 범위 체크) — 별개 기능. 본 작업 범위 아님.
- **load 시 자동 마이그레이션** — 명시적으로 제외(트리거 결정). 추후 필요 시 별 작업.
- **다운그레이드**(새 → 옛 스키마) — 단방향만.
- 기존 사용자 **값의 재조정**(default 가 바뀌었다고 사용자 값 덮어쓰기) — 절대 안 함.
  마이그레이션은 *없는 것 추가* + *deprecated 정리*만. 사용자가 명시한 값은 불가침.

## 3. 아키텍처 — 두 메커니즘

마이그레이션은 사용자 파일(`toml_edit::DocumentMut`)에 다음 순서로 적용한다.

```
원본 파일 → [1. step 체인(non-additive)] → [2. reconciliation(additive)] → [3. schema_version stamp] → 결과
```

### 3.1 Reconciliation (additive — 핵심 메커니즘)

**정의**: "default Config 구조에는 있지만 사용자 파일에 없는 테이블/키를, 설명 주석과
함께 사용자 파일에 추가한다." 버전과 무관하게 동작하며 멱등이다.

**참조 문서 = 주석 달린 default**: `annotated_default_document()` 가 단일 진실 원천이다.

```
fn annotated_default_document() -> toml_edit::DocumentMut
//  Config::defaults() 를 toml_edit Document 로 직렬화한 뒤,
//  주석 카탈로그(§3.3)의 설명을 각 테이블/키의 decor(prefix)에 부착.
//  → 이 문서가 "완전체 config.toml" 의 정의.
```

`kebab init` 도 이 함수의 출력을 그대로 파일로 쓴다(§5.2). 즉 **init 과 migrate 가
동일한 참조 문서를 공유** → 주석·구조의 단일 원천.

**reconcile 알고리즘** (참조 문서 `ref` → 사용자 문서 `user`, 재귀):

```
for each (key, ref_item) in ref (문서 순서 유지):
    if key 가 user 에 없음:
        user 에 ref_item 을 통째 복사 (decor=주석 포함).  → change: added_section / added_key
    else if ref_item 과 user[key] 가 둘 다 테이블:
        recurse(ref_item, user[key])    # 하위만 비교
    else:
        # 키가 이미 존재(값이 default 와 달라도) → 건드리지 않음. (값 불가침)
```

- **삽입 위치**: 누락 키는 해당 테이블 **끝에 append**(결정적·단순). 사용자가 짜둔 기존
  순서는 보존되고 새 항목만 뒤에 붙는다.
- **중첩 테이블**: `[ingest]` 는 있는데 `[ingest.expansion]` 이 없으면 `expansion`
  하위 테이블만 추가. `[ingest]` 자체가 없으면 `[ingest]` + 그 안의 모든 하위를 추가.
- **값 불가침 예시**: 사용자가 `score_gate = 0.8` 로 바꿔뒀고 default 가 0.6 이어도,
  키가 존재하므로 **0.8 유지**. 마이그레이션은 0.6 으로 되돌리지 않는다.

### 3.2 Step 체인 (non-additive)

`schema_version` 기반 버전별 변환 함수. additive 가 아닌 변경(deprecated 제거, rename,
형식 변환)을 담당한다. DB refinery 패턴 차용.

```
const CURRENT_SCHEMA_VERSION: u32 = 2;   // 이번 작업에서 1 → 2

fn step_1_to_2(doc: &mut DocumentMut, changes: &mut Vec<MigrationChange>)
//  v1 → v2 변환: 옛 `workspace.include` 키 제거 (p9-fb-25 deprecated).
//   - doc["workspace"]["include"] 존재 시 remove → change: removed_deprecated.
//   - 없으면 noop (멱등).
```

- **실행 범위**: 파일의 `schema_version`(없으면 1 로 간주) 부터 `CURRENT` 까지 순차 적용.
  이미 `CURRENT` 이상이면 step 없음.
- 각 step 은 **개별적으로 멱등**(이미 적용된 상태에서 재실행해도 noop).
- 이번 작업의 유일한 step 은 `1→2`(workspace.include 제거). 누적된 섹션 추가
  (image/ui/ingest/pdf/logging/expansion)는 **전부 reconciliation 이 처리**하므로
  step 으로 만들지 않는다. step 체인은 "구조로 표현 못 하는 변환"만 담는다.

### 3.3 주석 카탈로그

"섹션/키 → 한국어 설명 주석" 매핑을 kebab-config 의 마이그레이션 모듈 한 곳에 정적
정의한다. 단일 원천 — README/SMOKE 와 중복하지 않고 여기를 정본으로.

- 기존 `init_workspace` 의 헤더(경로 정책 설명, `kebab-app/src/lib.rs:147~`)는
  **문서 레벨 prefix** 로 이전한다(`annotated_default_document` 가 부착).
- 섹션별 주석은 README Configuration §의 노브 설명을 차용해 **간결**하게(1~2줄).
  예: `[ingest.expansion]` → `# doc-side 별칭 확장 (기본 off). 검색 패러프레이즈 강건성↑.`
- 주석 문구는 짧게, 과하지 않게. 전체 문서는 생성된 파일·README·SMOKE 참고로 유도.

### 3.4 멱등성 보장 (안전 1축)

- reconciliation: 이미 있는 키는 skip → 두 번째 실행 시 changes 비어 있음.
- step: 각 step 이 noop-safe.
- 결과: **마이그레이션 후 재실행하면 `changed=false`, 파일 미변경.** 이것이 doctor
  체크(§5.3)와 멱등 테스트의 핵심 단언.

## 4. 안전 3축 (kickoff §4.4)

1. **멱등** — §3.4.
2. **백업** — 파일 수정 직전 `<config>.bak` 생성(원본 복사). 기존 `.bak` 있으면 덮어씀
   (단순화; 변경 내용은 dry-run 으로 사전 확인 가능). dry-run 시 백업도 안 만듦.
3. **dry-run** — `--dry-run` 은 changes 만 계산·출력하고 **파일·백업 모두 미수정**.

**실패 시 원본 보존(atomic write)**: 편집 결과는 `<config>.tmp` 에 먼저 쓰고
`rename(tmp, config)` 로 교체. rename 이전 어느 단계에서 실패해도 원본 불변. 순서:
`백업 생성 → tmp 쓰기 → tmp 검증(재파싱 round-trip) → atomic rename`.

## 5. 표면 (surface)

### 5.1 CLI — `kebab config migrate`

신규 top-level `Config` 서브커맨드 그룹(clap nested, `Inspect`/`List` 패턴 차용):

```
kebab config migrate [--dry-run] [--json]
```

- 전역 `--config <path>` 존중 (facade rule). 미지정 시 XDG 기본 경로.
- 대상 파일이 없으면 에러: `config 파일이 없습니다. 먼저 kebab init 을 실행하세요.`
  (`--json` 시 `error.v1`, code `config_not_found`).
- 사람용 출력: 변경 목록(추가된 섹션/키, 제거된 deprecated) + 백업 경로 + "N changes
  applied" 또는 "already up to date".
- `--json`: `config_migration.v1` (§5.4).

**facade**: kebab-cli 는 kebab-app 의
`config_migrate_with_config_path(config_path: Option<&Path>, dry_run: bool)
-> anyhow::Result<ConfigMigrationReport>` 를 호출(파일 read/백업/atomic write
오케스트레이션은 app 계층, 순수 변환은 config 계층 — §6).

### 5.2 `kebab init` 영향 (user-visible)

`init_workspace` 가 `annotated_default_document()` 출력을 쓰도록 변경. 결과: init 이
생성하는 config.toml 이 **섹션별 주석을 포함**(기존엔 헤더만). 이는 user-visible surface
변경이므로 README Configuration §·docs/SMOKE.md 의 config 예시 블록 동기화 필요.

### 5.3 `kebab doctor` 체크 추가 (additive)

config load 체크 직후 `config_migration` 체크 1개 추가:

- 내부적으로 dry-run 마이그레이션 실행 → changes 비었으면 `ok=true`,
  detail `config up to date (schema v2)`, hint=None.
- changes 있으면 `ok=false`, detail `N pending changes (added M sections, removed K
  deprecated)`, hint `run kebab config migrate to update your config.toml`.
- **trade-off (확정)**: `DoctorCheck` 는 `ok: bool` 뿐이고 hint 는 `ok==false` 일 때
  표시되는 규약이므로, "마이그레이션 필요"는 `ok=false` 로 신호한다. 이는 전체
  `DoctorReport.ok`(모든 체크의 AND)를 false 로 만든다 — 즉 *완전히 동작하지만
  config 가 옛 스키마인* 환경에서 `kebab doctor` 가 "비정상"으로 보고된다. 이를
  의도된 동작으로 받아들인다(doctor = "정리할 것이 있는가"의 점검이고, hint 가 정확한
  교정 명령을 제시). 새 키만 추가하는 additive 변경을 "건강 실패"로 과하게 보는 면이
  있으나, 별도 warn 상태를 도입하는 것(스키마·표면 변경)보다 단순함을 택한다.
- `doctor.v1` 스키마는 변경 없음(checks 배열에 행 1개 추가 — additive, backward-compat).

### 5.4 wire schema `config_migration.v1` (신규)

`docs/wire-schema/v1/config_migration.schema.json` 신설. `--json` 출력:

```json
{
  "schema_version": "config_migration.v1",
  "dry_run": true,
  "config_path": "/home/me/.config/kebab/config.toml",
  "from_schema_version": 1,
  "to_schema_version": 2,
  "changed": true,
  "backup_path": null,
  "changes": [
    { "kind": "added_section",     "path": "ingest.expansion", "detail": "doc-side 별칭 확장 (기본 off)" },
    { "kind": "added_key",         "path": "logging.enabled",  "detail": "ingest 로그 활성화" },
    { "kind": "removed_deprecated","path": "workspace.include","detail": "p9-fb-25: extractor 자동 결정" }
  ]
}
```

- `changed`: 실제(또는 dry-run 시 가정) 변경 발생 여부. false 면 changes=[].
- `backup_path`: 실제 적용 시 `.bak` 경로, dry-run 시 `null`.
- `kind` enum: `added_section | added_key | removed_deprecated`. (향후 `renamed`,
  `reformatted` 확장 여지 — 본 작업은 3종.)
- additive 신규 스키마 → 기존 통합 영향 없음. wire major bump 아님(v1 추가).

## 6. 코드 배치 (crate 경계)

| 위치 | 책임 | 비고 |
|------|------|------|
| `crates/kebab-config/src/migrate.rs` (신규) | **순수 변환**: `annotated_default_document`, `reconcile`, step 체인, `CURRENT_SCHEMA_VERSION`, 주석 카탈로그, `MigrationChange`/`ConfigMigrationReport` 타입, `migrate_document(doc) -> Vec<MigrationChange>` | I/O 없음. 문자열 in → 문자열 out 로 테스트 가능. |
| `crates/kebab-config/Cargo.toml` | `toml_edit = "0.22"` 의존성 추가 | 주석 보존 편집 핵심. |
| `crates/kebab-app/src/lib.rs` | **I/O 오케스트레이션**: `config_migrate_with_config_path`(read → migrate_document → 백업 → tmp write → atomic rename), `init_workspace` 가 `annotated_default_document` 사용하도록 수정, doctor 에 체크 추가 | facade. fs 부작용은 app 계층. |
| `crates/kebab-cli/src/main.rs` | `Config { Migrate { dry_run } }` 서브커맨드, 사람용 출력 | kebab-app facade 만 호출. |
| `crates/kebab-cli/src/wire.rs` | `wire_config_migration(report) -> Value` | `config_migration.v1` 직렬화. |
| `docs/wire-schema/v1/config_migration.schema.json` (신규) | wire 계약 | |

**경계 근거**: kebab-config 는 이미 파일 *읽기*(`from_file`)를 하지만, *쓰기*는
`init_workspace`(app)에 있다. 일관성·테스트성 위해 순수 변환은 config, 부작용(백업·쓰기)
은 app. doctor(app)·cli 모두 동일 순수 변환을 재사용.

## 7. schema_version 의 새 의미

- 기존: 항상 `1`, 검증·로직에 안 쓰이는 장식.
- 신규: "이 파일이 sync 된 스키마 버전" 마커 + step 체인의 축.
- `Config::defaults().schema_version` 및 `CURRENT_SCHEMA_VERSION` 을 **2** 로 bump.
  마이그레이션 완료 시 사용자 파일의 `schema_version` 을 `CURRENT` 로 stamp.
- 읽기 경로(`from_file`)는 여전히 `schema_version` 으로 **거부하지 않음**(forward-compat
  유지). 즉 옛 바이너리로 새 파일을, 새 바이너리로 옛 파일을 읽어도 동작.

## 8. 문서 동기화 (user-facing surface)

- **README.md Configuration §**: `kebab config migrate` 한 줄 + init config 가 섹션
  주석을 갖는다는 설명. config 예시 블록을 `annotated_default_document` 산출과 일치.
- **docs/SMOKE.md**: config 예시 블록 동기화. migrate dry-run smoke 단계 추가.
- **docs/DOGFOOD.md**: config 관련 section 에 migrate 시나리오(옛 파일 → migrate →
  섹션 가시성 확인) 추가.
- **tasks/HOTFIXES.md**: 머지 후 dated entry(`## YYYY-MM-DD — config 마이그레이션`),
  도그푸딩 evidence(옛 config 에 빠진 섹션 N개 추가 + workspace.include 제거 멱등 확인).
- **HANDOFF.md**: 해당되면 한 줄.

## 9. 릴리스 트리거 판단

- 신규 CLI 서브커맨드(`config migrate`) + doctor 체크 + init 출력 변경 = **user-visible
  surface 변경** → 도그푸딩 필수, README 동기화 필수.
- `schema_version` bump(1→2)는 **additive**(데이터 무효화 아님, 읽기 호환 유지) →
  CLAUDE.md §Versioning 의 DB/wire breaking 기준엔 해당 안 됨. 다만 surface 누적이
  있으므로 **minor bump** 대상일 수 있음. 실제 bump/release 컷 시점은 사용자 판단.

## 10. 테스트 전략 (plan 의 TDD 근거)

순수 변환(kebab-config)이 테스트의 중심 — 문자열 in/out, fs 불필요:

1. **reconciliation 추가**: 옛 config 문자열(섹션 누락) → migrate → 누락 섹션이 주석과
   함께 추가됐고, 기존 키·주석·순서는 보존.
2. **값 불가침**: 사용자가 바꾼 값(예: `score_gate = 0.8`)이 migrate 후에도 유지.
3. **멱등**: migrate 출력을 다시 migrate → `changed=false`, 동일 문자열.
4. **step (workspace.include 제거)**: 옛 키 있는 문자열 → 제거됨 + change 기록. 없으면 noop.
5. **schema_version stamp**: 결과의 `schema_version = 2`. 없던 파일엔 추가됨.
6. **주석 보존**: 사용자가 임의 키에 단 주석이 migrate 후에도 그대로.
7. (app) **백업·atomic·실패 보존**: 백업 파일 생성, tmp rename, 손상 입력 시 원본 불변.
8. (app) **dry-run**: 파일·백업 미생성, report.changed 정확.
9. (cli/wire) `config_migration.v1` 직렬화 형태.

## 11. Risks / notes

- `toml_edit` 신규 의존성 — kebab-config 에 추가. `toml`(0.8)과 공존(serde 경로는
  여전히 `toml`, 편집 경로만 `toml_edit`). 버전은 구현 시 최신 0.22.x 확인.
- reconciliation 의 "끝에 append" 는 사용자가 짠 미적 순서를 흩뜨릴 수 있으나(새 섹션이
  뒤로 몰림), 값·주석·기존 순서 보존이 우선이며 단순·결정적이라 채택.
- 첫 step(`1→2`)은 사실상 이미 무시되는 `workspace.include` 청소뿐 — step 체인은 주로
  *프레임워크*로서 미래 non-additive 변경을 위해 깔아둔다.
- kickoff 인계 문서와의 차이: kickoff §4.2 는 "버전별 변환 함수 체인"만 제안했으나,
  kebab 의 serde-default 특성상 additive 변경은 step 으로 표현하기 부적절(버전 무관) →
  **reconciliation 을 1급 메커니즘으로 승격**하고 step 은 non-additive 전용으로 한정.
- 2026-06-04 v3 재편(첫 non-additive rename)에서 `step_2_to_3`(미디어 테이블
  `[ingest.*]` relocation) + `Config::from_file` load 시 메모리 자동변환 추가 —
  `docs/superpowers/specs/2026-06-04-config-schema-reorg-design.md`.
