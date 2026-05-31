# config 마이그레이션 — 작업 인계 (kickoff)

> 2026-05-31. config.toml **스키마 진화 시 기존 사용자 파일을 자동 마이그레이션**하는
> 기능. 새 세션은 이 문서 + 메모리 [[project_paraphrase_robustness]] 로 이어받는다.
> 본격 진행은 brainstorm → spec → plan → 구현 (방법론 §5).

## 1. 동기

v0.21.0 에서 `[ingest.expansion]`(별칭) 섹션을 추가했다. 기존 사용자 config.toml 은
serde default 로 **동작은 호환**(off 로 로드)되지만, 그 섹션이 **파일에 써지지 않아**
사용자가 파일을 열어도 새 기능의 존재·노브를 알 수 없다. DB 는 V00X refinery
마이그레이션이 있는데 **config 는 마이그레이션 메커니즘이 없다** — 이걸 만든다.

## 2. 현황 (코드, 현재 main = v0.21.0)

- **읽기는 이미 forward-compatible**: `crates/kebab-config/src/lib.rs` 의 모든 새
  섹션/필드가 `#[serde(default)]` (예: ImageCfg L50, UiCfg L55, ingest.code L60,
  PdfCfg L65, logging L70, nli L132 …). missing 필드는 default 로 로드돼 **기존
  config 가 깨지지 않는다**. → 동작 호환성은 확보돼 있고, 만들 것은 *파일 갱신*이다.
- **`schema_version: u32`** (lib.rs:38, 현재 `1`) — **검증·마이그레이션에 안 쓰이는
  장식**. 마이그레이션의 버전 축으로 활용할 자리.
- **파일 쓰기는 init 뿐**: `kebab init` 이 `toml::to_string(&Config::defaults())`
  로 default config 생성(lib.rs:1349 부근). **기존 파일을 갱신하는 경로는 없다.**
- **deprecated 선례**: 옛 `workspace.include` 는 로드 시 무시 + 1회 deprecation
  warning (p9-fb-25). 마이그레이션의 "deprecated 정리" 참고 패턴.

## 3. 풀어야 할 핵심 — 주석/순서 보존

`toml::to_string` 으로 통째 재작성하면 **사용자가 손본 주석·정렬·순서가 전부
날아간다**. 이게 config 마이그레이션의 본질적 난점. 접근 3안:

| 방식 | 주석 보존 | 복잡도 | 비고 |
|------|-----------|--------|------|
| A. 전체 재작성(로드→재직렬화) | ✗ | 낮음 | 사용자 값은 보존되나 주석 손실 |
| B. `toml_edit` 로 missing 섹션만 주석과 함께 append/수정 | ✓ | 중간 | 의존성 추가, 가장 사용자 친화적 |
| C. 백업(.bak) 후 재생성 + diff 안내 | △ | 낮음 | 안전하나 사용자가 주석 수동 복원 |

→ **B(`toml_edit`)** 가 사용자 손본 config 보존엔 최선. 의존성·복잡도 trade-off 를
brainstorm 에서 결정.

## 4. 설계 결정 (brainstorm 시작점)

1. **트리거**: `kebab config migrate` 명시 명령 vs `load` 시 자동(+백업). 자동은
   편하나 예측 가능성/안전(쓰기 권한·손상)이 걸린다. 명시 명령 + `kebab doctor`
   에서 "마이그레이션 필요" 안내가 무난할 수 있음.
2. **버전 축**: `schema_version` 기반 버전별 변환 함수 체인 (v1→v2→…, DB refinery
   패턴 차용). 각 step 은 "이 버전에서 추가된 섹션/바뀐 형식/제거된 deprecated".
3. **동작**: (a) 새 섹션을 주석과 함께 추가 (b) deprecated 필드 정리/이동
   (c) 형식 변경 변환. 모두 **멱등**(재실행 안전).
4. **안전**: 사용자 손본 config 손상 절대 금지 → **백업(.bak) 필수**, dry-run 옵션,
   실패 시 원본 보존.

## 5. 방법론 (v0.21.0 작업과 동일 — PR #195/#196 참고)

brainstorm(사용자 컨펌 게이트 skip, self-review) → spec(self-review) → plan(TDD,
bite-sized) → executor(opus) 또는 OMC teammate 구현 → **gitea-pr 리뷰 루프**
(round1 리뷰 opus, closure verify sonnet) → 머지. 빌드는 항상
`CARGO_TARGET_DIR=/build/out/cargo-target/target cargo … -j 4 > /tmp/x.log 2>&1; echo EXIT=$?`
(절대 `cargo | grep` 금지). PR 은 gitea REST(`~/.netrc`), gh 안 됨.

## 6. 관련 파일

- `crates/kebab-config/src/lib.rs` — `Config` struct, `schema_version`, serde default
  패턴, `load`/`defaults`/`to_string`. 마이그레이션 모듈을 여기 or 신규 `migrate.rs`.
- `crates/kebab-cli/src/*` — `init` 명령 옆에 `config migrate`(또는 `config`) 서브커맨드.
- `migrations/V0XX__*.sql` — DB 마이그레이션의 버전 체인 패턴 차용 참고.
- `toml_edit` 크레이트(주석 보존 편집) — B안 시 의존성 후보.

## 7. 주의

- config 마이그레이션은 **user-facing surface** → README(Configuration)/HOTFIXES 동기화
  (이번 세션 패턴 [[feedback_readme_sync_rule]]). 마이그레이션 *동작 디테일*은 spec 에
  충실히([[feedback_design_detail_docs]]).
- `schema_version` bump 가 release 트리거인지는 별도 판단 — DB schema(V00X)와 달리
  config 버전은 데이터 무효화가 아니므로, additive 면 release 트리거 아닐 수 있음
  (CLAUDE.md §Versioning 의 DB/wire 기준과 구분).
- 멱등 + 백업 + dry-run 이 안전의 3축.
