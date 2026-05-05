# p9-fb-25 — Config `workspace.include` 제거 + 지원 형식 가시성

**Date**: 2026-05-05
**Status**: planned
**Audience**: kebab-config / kebab-app / kebab-cli / kebab-tui implementer.
**Source feedback**: 사용자 도그푸딩 2026-05-05 — config 의 `workspace.include` + `workspace.exclude` 가 동시에 있으면 case 4 (둘 다 매치 안 함) 의미 모호 + 어차피 처리 가능 형식이 정해져 있으니 사용자에게 명시 필요.

## Goal

- `WorkspaceCfg.include` 필드 제거. dead config field 제거 + denylist-only 모델 정착.
- 사용자가 ingest 결과에서 \*\*어떤 파일이 왜 skip 됐는지\*\* 즉시 파악.
- 지원 형식 (md / png / jpg / pdf) 을 README + `kebab init` config 주석에 명시.

## Non-goals

- include 의 enforce 로직 추가 (반대 방향).
- 새 extractor (txt / docx / epub 등) 도입 — 별 spec.
- `kebab doctor` 가 unsupported 파일 카운트 분석 — 별 task (간단 follow-up 가능).

## Allowed dependencies

- 기존 crate 만. 신규 crate 없음. 신규 SQLite migration 없음.

## Storage 변경

없음.

## API / Wire 변경

### `kebab-config::WorkspaceCfg`

`include: Vec<String>` 필드 제거. `exclude: Vec<String>` 만 유지.

backward-compat: serde default `deny_unknown_fields` 미사용이라 옛 config 의 `include = [...]` 은 silently deserialize 통과 + 무시. `Config::load` 가 옛 키 발견 시 `tracing::warn!` 로 deprecation 경고 emit (단발 — 같은 process 안에서 한 번만):

```
deprecated config: `workspace.include` 필드는 더 이상 사용되지 않습니다 (p9-fb-25). 처리 가능한 형식 (md / png / jpg / pdf) 은 extractor 가 자동 결정. 다음 버전부터 config 갱신 권장.
```

검출 방법: `Config::load` 가 raw TOML 파싱 후 `workspace` 테이블의 키 이름을 살펴 `include` 존재 여부 확인. `serde_ignored` crate 미도입 (YAGNI) — `toml::Value` 로 raw lookup 한 번.

### `kebab-core::IngestItem.warnings`

Skipped path 가 빈 `Vec` 대신 사유 한 줄 채움. 두 case:

- media-type filter (extractor 미지원): `format!("unsupported media type: .{ext}")` (e.g. `"unsupported media type: .docx"`). extension 이 없으면 `"unsupported media type: <no-ext>"`.
- `kb://` URI: `"kb:// URI not yet supported"`.

### `kebab-core::IngestReport.skipped_by_extension`

신규 필드:

```rust
pub skipped_by_extension: std::collections::BTreeMap<String, u32>,
```

key = lowercase extension without leading dot (`"docx"`, `"txt"`, `"epub"`). 확장자 없는 파일 = `"<no-ext>"` sentinel (꺾쇠로 일반 ext 와 시각 구분).

`BTreeMap` 사용 — wire JSON 안에서 key 정렬 안정. `HashMap` 은 매 직렬화마다 순서 바뀌어 diff / snapshot 테스트 noisy.

`AggregateCounts` 도 동일 필드 추가 — TUI / CLI 가 in-flight 와 final 모두에서 일관 표시.

### Wire schema `ingest_report.v1`

`skipped_by_extension` 필드 additive 추가:

```json
"skipped_by_extension": {
  "type": "object",
  "additionalProperties": {
    "type": "integer",
    "minimum": 0
  },
  "description": "p9-fb-25: per-extension skip count. Key = lowercase extension without leading dot (e.g. 'docx'). Files without extension key under '<no-ext>'."
}
```

CLAUDE.md 의 release 규약 (additive minor) 에 따라 release bump 트리거 안 됨.

## TUI / CLI 노출

### CLI summary

기존:

```
✓ ingest: 100 docs (5 new, 3 updated, 2 unchanged, 90 skipped), 142 chunks indexed in 12s
```

변경 (skipped > 0 + breakdown 있을 때만 괄호 안):

```
✓ ingest: 100 docs (5 new, 3 updated, 2 unchanged, 90 skipped: 80 docx, 5 txt, 5 epub), 142 chunks indexed in 12s
```

extension 카운트 desc 정렬 (큰 거 먼저). 모두 표시 (top-3 제한 없음). Line 길어질 우려가 있으나 사용자 원함 — line wrap 은 terminal 책임.

### TUI

`kebab-tui::ingest_progress::status_line` 의 final / aborted 라인 동일 포맷. in-flight 진행 중에는 breakdown 표시 안 함 (idx 진행 중 계속 변동, 불필요 noise).

## 사용자 안내 (docs)

### README

`kebab ingest` row 의 cell 끝에 추가:

```
**지원 형식** (extractor 자동 결정): Markdown (`.md`) / 이미지 (`.png`, `.jpg`, `.jpeg`, OCR + caption) / PDF (`.pdf`). 다른 확장자는 자동 skip — `--json` / TUI 의 `IngestItem.warnings` 에 사유 (`unsupported media type: .docx` 등). 카운트 분류는 `IngestReport.skipped_by_extension`.
```

### `kebab init` config.toml 주석

`[workspace]` section 위에 주석 추가:

```toml
# [workspace] — 색인 대상 디렉토리 + denylist.
#
# 지원 형식 (extractor 가 자동 결정 — config 에 명시할 수 없음):
#   - Markdown: .md
#   - 이미지:   .png .jpg .jpeg (OCR + caption)
#   - PDF:      .pdf
#
# 다른 확장자는 ingest 시 자동 skip + warning. 처리 대상 폴더의
# 일부만 ingest 하고 싶으면 `kebab ingest <path>` 로 root 명시
# 또는 `.kebabignore` 파일 / 본 `exclude` 로 denylist.
[workspace]
root = "..."
exclude = [...]
```

## Tests

### 신규 단위

- `kebab-config`: `Config::load` 가 옛 `include = [...]` 발견 시 warning emit + 정상 deserialize. snapshot test (in-memory string TOML).
- `kebab-core`: `IngestItem` JSON serde — `warnings` 가 `["unsupported media type: .docx"]` round-trip.
- `kebab-core`: `IngestReport.skipped_by_extension` JSON serde — `BTreeMap` 정렬 stable.

### 신규 통합

- `kebab-app`: 다양한 확장자 mix (`.md`, `.docx`, `.txt`, no-ext 파일) workspace 에서 ingest → `report.skipped_by_extension == {"docx": 1, "txt": 1, "<no-ext>": 1}` + 각 skipped 의 `warnings` 채워짐.
- `kebab-tui`: `status_line` 가 `90 skipped: 80 docx, 5 txt, 5 epub` 형식.
- `kebab-cli`: `kebab ingest --json` 출력에 `skipped_by_extension` 필드.

### 기존 영향

- 기존 `IngestReport` 구성 site (테스트 fixture 등) 가 새 필드 default 로 채움 (`BTreeMap::new()`).
- `WorkspaceCfg` 의 `include` 필드 제거로 컴파일 에러 → 매 site 정리 (기존 default 가 `vec!["**/*.md"]` 였으니 모두 제거).

## Spec contract impact

- design §6.2 의 `workspace.include` 항목 invalidate. frozen spec 그대로 두고 본 spec + HOTFIXES `2026-05-05 — p9-fb-25` 가 source of truth.
- design §3.x `IngestReport` 에 `skipped_by_extension` 필드 추가 (additive).
- design §2.4a `IngestEvent::AssetFinished` 에 새로 emit 되는 warnings 의미 추가 (variant 변경 없음, content 풍부화).

## Risks / notes

- **옛 config 가 `include = ["**/*.md"]` 같은 narrow 한 allowlist** 면 본 변경 후 그 이상의 확장자 (예: 파일 추가된 `.png`) 가 자동 ingest 시작. 사용자 의도와 어긋날 수 있음. 완화: deprecation warning 의 문구가 \"처리 가능 형식 자동 결정\" 명시 → 사용자가 alarm 받음. + README 변경. 경계 case 라 design accepted.
- **`skipped_by_extension` 용량**: workspace 가 1만 파일이면 dict size 작음 (extension 종류는 보통 < 50). wire 영향 무시.
- **deprecation warning 단발 vs every-load**: `Config::load` 가 매 CLI 호출마다 발생. 단발 (`std::sync::Once`) 이 깔끔. 본 spec 은 단발 채택.
- **release 트리거**: wire schema additive + serde backward-compat → CLAUDE.md release 규약 의 minor 트리거에 해당 안 됨 (additive 만으로 release 안 찍음). 사용자 explicit 도그푸딩 요청 시 bump.

## Live deviations

추후 발견되는 deviation 은 `tasks/HOTFIXES.md` `2026-05-05 — p9-fb-25` 항목에 기록.
