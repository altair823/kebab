# config 마이그레이션 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 기존 사용자 `config.toml` 을 `kebab config migrate` 로 새 스키마에 맞춰 갱신한다 — 빠진 섹션/키를 설명 주석과 함께 추가하고 deprecated 를 정리하되, 사용자가 손본 값·주석·순서는 보존한다.

**Architecture:** 순수 변환(`kebab-config::migrate`, `toml_edit` 기반 reconciliation + step 체인)과 I/O 오케스트레이션(`kebab-app` 의 백업·atomic write)을 분리한다. `init` 과 `migrate` 는 "주석 달린 default 문서"(`annotated_default_document`)를 단일 원천으로 공유한다. `kebab doctor` 가 마이그레이션 필요 여부를 ok=false 로 신호한다.

**Tech Stack:** Rust 2024, `toml_edit` 0.22(주석 보존 편집), `toml` 0.8(기존 serde 경로), clap(nested subcommand), serde_json(wire).

**Spec:** [`docs/superpowers/specs/2026-05-31-config-migration-design.md`](../specs/2026-05-31-config-migration-design.md)

---

## File Structure

| 파일 | 책임 | 신규/수정 |
|------|------|-----------|
| `crates/kebab-config/Cargo.toml` | `toml_edit` 의존성 추가 | 수정 |
| `crates/kebab-config/src/migrate.rs` | 순수 변환 엔진: 타입, 주석 카탈로그, `annotated_default_document`, `reconcile`, step 체인, `migrate_document` | 신규 |
| `crates/kebab-config/src/lib.rs` | `pub mod migrate;` 재노출, `schema_version` default 2 로 bump | 수정 |
| `crates/kebab-app/src/lib.rs` | `config_migrate_with_config_path`(I/O), `init_workspace` 가 annotated doc 사용, doctor 체크 추가 | 수정 |
| `crates/kebab-cli/src/main.rs` | `Config { Migrate }` 서브커맨드 + 사람용 출력 | 수정 |
| `crates/kebab-cli/src/wire.rs` | `wire_config_migration` | 수정 |
| `crates/kebab-app/src/schema.rs` | `config_migration.v1` 을 schema 목록에 등록 | 수정 |
| `docs/wire-schema/v1/config_migration.v1.schema.json` | wire 계약 | 신규 |
| `README.md` / `docs/SMOKE.md` / `docs/DOGFOOD.md` | surface 동기화 | 수정 |
| `tasks/HOTFIXES.md` / `HANDOFF.md` | 머지 후 dated entry | 수정 |

**빌드 명령(모든 task 공통):**
```bash
CARGO_TARGET_DIR=/build/out/cargo-target/target cargo test -p <crate> > /tmp/t.log 2>&1; echo EXIT=$?
```
절대 `cargo | grep` 금지. 실패 시 `/tmp/t.log` 확인.

---

## Task 1: toml_edit 의존성 + migrate 모듈 스캐폴딩

**Files:**
- Modify: `crates/kebab-config/Cargo.toml`
- Create: `crates/kebab-config/src/migrate.rs`
- Modify: `crates/kebab-config/src/lib.rs` (모듈 선언)

- [ ] **Step 1: `toml_edit` 의존성 추가**

`crates/kebab-config/Cargo.toml` 의 `[dependencies]` 에 추가(기존 `toml = "0.8"` 아래):
```toml
toml_edit = "0.22"
```

- [ ] **Step 2: migrate.rs 에 타입 정의 + 모듈 선언**

`crates/kebab-config/src/migrate.rs` 생성:
```rust
//! config.toml 마이그레이션 엔진 (순수 변환, I/O 없음).
//!
//! 두 메커니즘: (1) reconciliation — default 구조에 있고 사용자 파일에
//! 없는 섹션/키를 주석과 함께 추가. (2) step 체인 — schema_version 기반
//! non-additive 변환(deprecated 제거 등). 자세한 계약은 spec
//! `docs/superpowers/specs/2026-05-31-config-migration-design.md`.

use serde::Serialize;

/// 현재 바이너리가 이해하는 config 스키마 버전. 마이그레이션 완료 시
/// 사용자 파일의 `schema_version` 을 이 값으로 stamp 한다.
pub const CURRENT_SCHEMA_VERSION: u32 = 2;

/// 한 번의 마이그레이션에서 발생한 개별 변경.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MigrationChange {
    pub kind: ChangeKind,
    /// dotted path, 예: `ingest.expansion`, `workspace.include`.
    pub path: String,
    /// 사람·wire 용 한 줄 설명.
    pub detail: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    AddedSection,
    AddedKey,
    RemovedDeprecated,
}

/// 마이그레이션 결과 요약(순수 변환 단계 산출). I/O 계층이 backup_path
/// 등을 채워 wire 로 내보낸다.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MigrationOutcome {
    pub from_schema_version: u32,
    pub to_schema_version: u32,
    pub changes: Vec<MigrationChange>,
    /// 변환 후 직렬화된 새 문서 텍스트(멱등 시 입력과 동일).
    pub new_text: String,
}

impl MigrationOutcome {
    pub fn changed(&self) -> bool {
        !self.changes.is_empty()
    }
}
```

`crates/kebab-config/src/lib.rs` 에 모듈 선언 추가(상단 `mod paths;` 근처):
```rust
pub mod migrate;
```

- [ ] **Step 3: 컴파일 확인**

Run: `CARGO_TARGET_DIR=/build/out/cargo-target/target cargo build -p kebab-config > /tmp/t.log 2>&1; echo EXIT=$?`
Expected: `EXIT=0`

- [ ] **Step 4: Commit**

```bash
git add crates/kebab-config/Cargo.toml crates/kebab-config/src/migrate.rs crates/kebab-config/src/lib.rs
git commit -m "feat(config): migrate 모듈 스캐폴딩 + toml_edit 의존성"
```

---

## Task 2: schema_version default 를 2 로 bump

**Files:**
- Modify: `crates/kebab-config/src/lib.rs:672` (`Config::defaults()` 의 `schema_version: 1`)
- Modify: 테스트/fixture 의 `schema_version = 1` 리터럴

- [ ] **Step 1: 실패 테스트 추가**

`crates/kebab-config/src/lib.rs` 의 `#[cfg(test)] mod tests` 에 추가:
```rust
#[test]
fn defaults_schema_version_matches_current() {
    assert_eq!(
        Config::defaults().schema_version,
        crate::migrate::CURRENT_SCHEMA_VERSION
    );
}
```

- [ ] **Step 2: 실패 확인**

Run: `CARGO_TARGET_DIR=/build/out/cargo-target/target cargo test -p kebab-config defaults_schema_version_matches_current > /tmp/t.log 2>&1; echo EXIT=$?`
Expected: FAIL (`left: 1, right: 2`)

- [ ] **Step 3: default bump**

`crates/kebab-config/src/lib.rs` 의 `Config::defaults()` 본문(약 672행):
```rust
            schema_version: crate::migrate::CURRENT_SCHEMA_VERSION,
```
(`schema_version: 1,` 을 위 줄로 교체.)

- [ ] **Step 4: 인라인 fixture 갱신**

같은 파일 테스트 영역의 하드코딩 `schema_version = 1` 리터럴(약 1276행, 1704행의 `const *_TOML` 문자열)은 **그대로 둔다** — 그것들은 "옛 파일도 로드된다"는 forward-compat 테스트의 입력이므로 1 이어야 한다. `defaults_are_serde_roundtrip_stable`(1347행 부근)은 `Config::defaults()` 를 직렬화→역직렬화하므로 자동으로 2 가 되어 통과. 추가 수정 불필요. (혹시 `assert!(toml.contains("schema_version = 1"))` 형태가 있으면 2 로 갱신 — grep `schema_version = 1` 로 확인.)

Run: `CARGO_TARGET_DIR=/build/out/cargo-target/target cargo test -p kebab-config > /tmp/t.log 2>&1; echo EXIT=$?`
Expected: `EXIT=0`

- [ ] **Step 5: Commit**

```bash
git add crates/kebab-config/src/lib.rs
git commit -m "feat(config): schema_version default 1 → 2 (마이그레이션 축)"
```

---

## Task 3: 주석 카탈로그 + annotated_default_document

**Files:**
- Modify: `crates/kebab-config/src/migrate.rs`

`toml_edit` 0.22 API 메모: 테이블 헤더 위 주석은 `table.decor_mut().set_prefix("# ...\n")`. 테이블 안 key 위 주석은 `table.key_mut("name").unwrap().leaf_decor_mut().set_prefix("# ...\n")` (KeyMut). 값이 dotted/inline 인 경우도 동일하게 key 의 leaf decor 사용. 문서 전체 상단 주석은 `doc.decor_mut().set_prefix(...)` 가 아니라 첫 항목의 prefix 또는 `doc.set_trailing`/직접 문자열 prepend — 여기서는 첫 key 의 prefix 에 헤더를 붙인다.

- [ ] **Step 1: 실패 테스트 추가**

`migrate.rs` 하단:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn annotated_default_has_all_sections_and_parses_back_to_defaults() {
        let doc = annotated_default_document();
        let text = doc.to_string();
        // 핵심 섹션이 텍스트에 존재
        for section in ["[workspace]", "[ingest.expansion]", "[pdf]", "[logging]", "[ui]"] {
            assert!(text.contains(section), "missing {section}:\n{text}");
        }
        // 주석이 적어도 하나 부착됨
        assert!(text.contains("# "), "no comments attached");
        // 역파싱하면 Config::defaults() 와 동일(주석은 serde 가 무시)
        let back: crate::Config = toml::from_str(&text).expect("parse annotated default");
        assert_eq!(back, crate::Config::defaults());
    }
}
```

- [ ] **Step 2: 실패 확인**

Run: `CARGO_TARGET_DIR=/build/out/cargo-target/target cargo test -p kebab-config annotated_default_has_all_sections > /tmp/t.log 2>&1; echo EXIT=$?`
Expected: FAIL (`annotated_default_document` 미정의 → 컴파일 에러)

- [ ] **Step 3: 카탈로그 + 함수 구현**

`migrate.rs` 에 추가:
```rust
use toml_edit::DocumentMut;

/// 문서 최상단 헤더(경로 정책 등). 기존 init 헤더를 이전.
const HEADER: &str = "\
# kebab config — `~/.config/kebab/config.toml`.
#
# `workspace.root` accepts: 절대 / tilde(~) / env(${VAR}) / 상대 경로.
#   상대 경로의 base 는 cwd 가 아니라 THIS config 파일의 디렉토리.
# 처리 형식(extractor 자동 결정): Markdown(.md) / 이미지(.png .jpg) / PDF(.pdf).
# 런타임 override: `KEBAB_*` env (예: KEBAB_WORKSPACE_ROOT=/tmp kebab ingest).
#
# 이 파일은 `kebab config migrate` 로 새 스키마에 맞춰 갱신할 수 있다
# (빠진 섹션 추가 + 손본 값·주석 보존).
";

/// 테이블 헤더(`[section]`) 위에 붙일 주석. dotted path → 한 줄(들).
fn section_comment(path: &str) -> Option<&'static str> {
    Some(match path {
        "workspace" => "# 색인 대상 워크스페이스.",
        "storage" => "# XDG 저장 경로(데이터/sqlite/벡터/에셋/모델).",
        "indexing" => "# 병렬도 + 파일시스템 watch.",
        "chunking" => "# 청크 크기·오버랩·heading 존중.",
        "models" => "# embedding / llm / nli 모델.",
        "models.embedding" => "# 다국어 sentence embedding. dim 불일치 시 검색 0건.",
        "models.llm" => "# Ollama host:port + 모델.",
        "search" => "# 검색 기본 k·stale 기준·fusion.",
        "rag" => "# 답변 생성: prompt 템플릿·score gate·NLI.",
        "image" => "# 이미지 OCR + 캡션(기본 off, asset 당 모델 호출 비용).",
        "ui" => "# TUI 팔레트·role 스타일.",
        "ingest" => "# ingest 정책(code skip 등).",
        "ingest.code" => "# code ingest skip 정책(.gitignore 자동 honor).",
        "ingest.expansion" => "# doc-side 별칭 확장(기본 off). 패러프레이즈 강건성↑, LLM 비용 큼.",
        "pdf" => "# PDF ingest. scanned PDF OCR 은 기본 off(page 당 cost).",
        "logging" => "# ingest 로그(기본 on, ~/.local/state/kebab/logs).",
        _ => return None,
    })
}

/// Config::defaults() 를 직렬화 + 주석 부착한 "완전체" 문서.
/// init 과 migrate reconciliation 의 단일 참조 원천.
pub fn annotated_default_document() -> DocumentMut {
    let defaults = crate::Config::defaults();
    let pretty = toml::to_string_pretty(&defaults).expect("defaults serialize");
    let mut doc: DocumentMut = pretty.parse().expect("defaults parse as toml_edit");

    // 헤더: 첫 최상위 항목의 prefix 로.
    if let Some((mut first_key, _)) = doc.as_table_mut().iter_mut().next() {
        let prefix = format!("{HEADER}\n");
        first_key.leaf_decor_mut().set_prefix(prefix);
    }

    annotate_table(doc.as_table_mut(), "");
    doc
}

/// 재귀적으로 테이블/키에 주석 부착. `prefix_path` 는 dotted 누적 경로.
fn annotate_table(table: &mut toml_edit::Table, prefix_path: &str) {
    // 키 이름 목록을 먼저 수집(차용 충돌 회피).
    let keys: Vec<String> = table.iter().map(|(k, _)| k.to_string()).collect();
    for key in keys {
        let path = if prefix_path.is_empty() {
            key.clone()
        } else {
            format!("{prefix_path}.{key}")
        };
        // 하위 테이블이면 헤더 주석 + 재귀.
        if let Some(item) = table.get_mut(&key) {
            if let Some(sub) = item.as_table_mut() {
                if let Some(c) = section_comment(&path) {
                    let existing = sub.decor().prefix().and_then(|p| p.as_str()).unwrap_or("");
                    if !existing.contains(c) {
                        sub.decor_mut().set_prefix(format!("\n{c}\n"));
                    }
                }
                annotate_table(sub, &path);
            }
        }
    }
}
```

> 주: `leaf_decor_mut` / `decor_mut` 의 정확한 시그니처는 toml_edit 0.22 `cargo doc` 으로 확인. 위 코드가 컴파일 안 되면 동등 API(`key_decor_mut`, `Table::decor_mut`)로 조정 — 테스트가 가드한다.

- [ ] **Step 4: 통과 확인**

Run: `CARGO_TARGET_DIR=/build/out/cargo-target/target cargo test -p kebab-config annotated_default > /tmp/t.log 2>&1; echo EXIT=$?`
Expected: `EXIT=0`

- [ ] **Step 5: Commit**

```bash
git add crates/kebab-config/src/migrate.rs
git commit -m "feat(config): 주석 카탈로그 + annotated_default_document"
```

---

## Task 4: reconcile — 빠진 섹션/키를 주석과 함께 추가

**Files:**
- Modify: `crates/kebab-config/src/migrate.rs`

- [ ] **Step 1: 실패 테스트 추가**

`migrate.rs` tests:
```rust
    #[test]
    fn reconcile_adds_missing_section_preserving_user_values_and_comments() {
        // 옛 파일: expansion/logging 없음, score 는 사용자가 바꿈, 주석 보유.
        let user_text = "\
schema_version = 1

[workspace]
root = \"/my/notes\"   # 내 워크스페이스

[search]
default_k = 25
";
        let mut user: DocumentMut = user_text.parse().unwrap();
        let reference = annotated_default_document();
        let mut changes = Vec::new();
        reconcile(reference.as_table(), user.as_table_mut(), "", &mut changes);
        let out = user.to_string();

        // 빠진 섹션 추가됨
        assert!(out.contains("[ingest.expansion]"), "expansion not added:\n{out}");
        assert!(out.contains("[logging]"), "logging not added");
        // 사용자 값/주석 보존
        assert!(out.contains("root = \"/my/notes\""));
        assert!(out.contains("# 내 워크스페이스"));
        assert!(out.contains("default_k = 25"));
        // 새 섹션엔 주석 부착
        assert!(out.contains("doc-side 별칭"));
        // change 기록
        assert!(changes.iter().any(|c| c.kind == ChangeKind::AddedSection
            && c.path == "ingest.expansion"));
    }

    #[test]
    fn reconcile_does_not_overwrite_user_value_differing_from_default() {
        let user_text = "\
schema_version = 2

[rag]
score_gate = 0.8
";
        let mut user: DocumentMut = user_text.parse().unwrap();
        let reference = annotated_default_document();
        let mut changes = Vec::new();
        reconcile(reference.as_table(), user.as_table_mut(), "", &mut changes);
        let out = user.to_string();
        assert!(out.contains("score_gate = 0.8"), "user value clobbered:\n{out}");
        // rag 의 다른 키들은 추가됐을 수 있으나 score_gate 변경 change 는 없어야 함
        assert!(!changes.iter().any(|c| c.path == "rag.score_gate"));
    }
```

- [ ] **Step 2: 실패 확인**

Run: `CARGO_TARGET_DIR=/build/out/cargo-target/target cargo test -p kebab-config reconcile_ > /tmp/t.log 2>&1; echo EXIT=$?`
Expected: FAIL (`reconcile` 미정의)

- [ ] **Step 3: reconcile 구현**

`migrate.rs`:
```rust
/// 참조(주석 달린 default) 테이블 `reference` 를 기준으로, 사용자 테이블
/// `user` 에 없는 항목을 decor(주석) 포함 통째 복사한다. 이미 있는 키는
/// 건드리지 않는다(값 불가침). 양쪽이 테이블이면 하위로 재귀.
pub fn reconcile(
    reference: &toml_edit::Table,
    user: &mut toml_edit::Table,
    prefix_path: &str,
    changes: &mut Vec<MigrationChange>,
) {
    for (key, ref_item) in reference.iter() {
        let path = if prefix_path.is_empty() {
            key.to_string()
        } else {
            format!("{prefix_path}.{key}")
        };
        match user.get_mut(key) {
            None => {
                // 통째 삽입(decor 보존). 테이블이면 added_section, 아니면 added_key.
                let kind = if ref_item.is_table() {
                    ChangeKind::AddedSection
                } else {
                    ChangeKind::AddedKey
                };
                // schema_version 키는 reconcile 가 아니라 stamp 단계가 다룬다.
                if path == "schema_version" {
                    user.insert(key, ref_item.clone());
                    continue;
                }
                user.insert(key, ref_item.clone());
                changes.push(MigrationChange {
                    kind,
                    path: path.clone(),
                    detail: section_comment(&path)
                        .map(|c| c.trim_start_matches("# ").to_string())
                        .unwrap_or_else(|| format!("{key} 추가")),
                });
            }
            Some(existing) => {
                if let (Some(ref_tbl), Some(user_tbl)) =
                    (ref_item.as_table(), existing.as_table_mut())
                {
                    reconcile(ref_tbl, user_tbl, &path, changes);
                }
                // 둘 다 테이블이 아니면(스칼라 등) 값 불가침 → 무시.
            }
        }
    }
}
```

- [ ] **Step 4: 통과 확인**

Run: `CARGO_TARGET_DIR=/build/out/cargo-target/target cargo test -p kebab-config reconcile_ > /tmp/t.log 2>&1; echo EXIT=$?`
Expected: `EXIT=0`

- [ ] **Step 5: Commit**

```bash
git add crates/kebab-config/src/migrate.rs
git commit -m "feat(config): reconcile — 빠진 섹션/키 주석과 함께 추가(값 불가침)"
```

---

## Task 5: step 체인 — workspace.include 제거 (v1→v2)

**Files:**
- Modify: `crates/kebab-config/src/migrate.rs`

- [ ] **Step 1: 실패 테스트 추가**

```rust
    #[test]
    fn step_1_to_2_removes_deprecated_workspace_include() {
        let user_text = "\
[workspace]
root = \"/n\"
include = [\"*.md\"]
";
        let mut user: DocumentMut = user_text.parse().unwrap();
        let mut changes = Vec::new();
        step_1_to_2(&mut user, &mut changes);
        let out = user.to_string();
        assert!(!out.contains("include"), "include not removed:\n{out}");
        assert!(changes.iter().any(|c| c.kind == ChangeKind::RemovedDeprecated
            && c.path == "workspace.include"));
        // 멱등: 재실행 시 noop
        let mut changes2 = Vec::new();
        step_1_to_2(&mut user, &mut changes2);
        assert!(changes2.is_empty());
    }
```

- [ ] **Step 2: 실패 확인**

Run: `CARGO_TARGET_DIR=/build/out/cargo-target/target cargo test -p kebab-config step_1_to_2 > /tmp/t.log 2>&1; echo EXIT=$?`
Expected: FAIL (미정의)

- [ ] **Step 3: 구현**

```rust
/// v1 → v2: deprecated `workspace.include` 제거(p9-fb-25). 멱등.
pub fn step_1_to_2(doc: &mut DocumentMut, changes: &mut Vec<MigrationChange>) {
    if let Some(ws) = doc.get_mut("workspace").and_then(|i| i.as_table_mut()) {
        if ws.remove("include").is_some() {
            changes.push(MigrationChange {
                kind: ChangeKind::RemovedDeprecated,
                path: "workspace.include".to_string(),
                detail: "p9-fb-25: 처리 형식은 extractor 가 자동 결정 — 더 이상 사용 안 함."
                    .to_string(),
            });
        }
    }
}

/// 파일의 schema_version(없으면 1) 부터 CURRENT 까지 step 적용.
fn run_steps(doc: &mut DocumentMut, from: u32, changes: &mut Vec<MigrationChange>) {
    if from < 2 {
        step_1_to_2(doc, changes);
    }
    // 미래 step: if from < 3 { step_2_to_3(...) } ...
}
```

- [ ] **Step 4: 통과 확인**

Run: `CARGO_TARGET_DIR=/build/out/cargo-target/target cargo test -p kebab-config step_1_to_2 > /tmp/t.log 2>&1; echo EXIT=$?`
Expected: `EXIT=0`

- [ ] **Step 5: Commit**

```bash
git add crates/kebab-config/src/migrate.rs
git commit -m "feat(config): step 체인 v1→v2(workspace.include 제거) + run_steps"
```

---

## Task 6: migrate_document — 오케스트레이션 + 멱등 + schema_version stamp

**Files:**
- Modify: `crates/kebab-config/src/migrate.rs`

- [ ] **Step 1: 실패 테스트 추가**

```rust
    fn read_schema_version(text: &str) -> u32 {
        let doc: DocumentMut = text.parse().unwrap();
        doc.get("schema_version")
            .and_then(|i| i.as_integer())
            .unwrap_or(1) as u32
    }

    #[test]
    fn migrate_document_stamps_version_and_is_idempotent() {
        let old = "\
schema_version = 1

[workspace]
root = \"/n\"
include = [\"*.md\"]
";
        let outcome = migrate_document(old);
        assert_eq!(outcome.from_schema_version, 1);
        assert_eq!(outcome.to_schema_version, CURRENT_SCHEMA_VERSION);
        assert!(outcome.changed());
        assert!(!outcome.new_text.contains("include"));
        assert!(outcome.new_text.contains("[ingest.expansion]"));
        assert_eq!(read_schema_version(&outcome.new_text), CURRENT_SCHEMA_VERSION);

        // 멱등: migrate 결과를 다시 migrate → 변경 없음, 텍스트 동일.
        let again = migrate_document(&outcome.new_text);
        assert!(!again.changed(), "not idempotent: {:?}", again.changes);
        assert_eq!(again.new_text, outcome.new_text);
    }

    #[test]
    fn migrate_document_missing_schema_version_treated_as_v1() {
        let old = "[workspace]\nroot = \"/n\"\n";
        let outcome = migrate_document(old);
        assert_eq!(outcome.from_schema_version, 1);
        assert_eq!(read_schema_version(&outcome.new_text), CURRENT_SCHEMA_VERSION);
    }
```

- [ ] **Step 2: 실패 확인**

Run: `CARGO_TARGET_DIR=/build/out/cargo-target/target cargo test -p kebab-config migrate_document > /tmp/t.log 2>&1; echo EXIT=$?`
Expected: FAIL (미정의)

- [ ] **Step 3: 구현**

```rust
/// 사용자 config.toml 텍스트를 받아 step 체인 + reconciliation + version
/// stamp 를 적용하고 결과를 반환한다. 순수 함수(I/O 없음). 파싱 실패 시
/// from=1, 변경 없음, new_text=입력 그대로(상위에서 파싱 에러를 따로 처리).
pub fn migrate_document(text: &str) -> MigrationOutcome {
    let mut doc: DocumentMut = match text.parse() {
        Ok(d) => d,
        Err(_) => {
            return MigrationOutcome {
                from_schema_version: 1,
                to_schema_version: CURRENT_SCHEMA_VERSION,
                changes: Vec::new(),
                new_text: text.to_string(),
            };
        }
    };
    let from = doc
        .get("schema_version")
        .and_then(|i| i.as_integer())
        .unwrap_or(1) as u32;

    let mut changes = Vec::new();

    // 1. non-additive step 체인.
    run_steps(&mut doc, from, &mut changes);

    // 2. additive reconciliation(버전 무관).
    let reference = annotated_default_document();
    let ref_table = reference.as_table().clone();
    reconcile(&ref_table, doc.as_table_mut(), "", &mut changes);

    // 3. schema_version stamp.
    let current_in_file = doc
        .get("schema_version")
        .and_then(|i| i.as_integer())
        .unwrap_or(0) as u32;
    if current_in_file != CURRENT_SCHEMA_VERSION {
        doc["schema_version"] = toml_edit::value(i64::from(CURRENT_SCHEMA_VERSION));
    }

    MigrationOutcome {
        from_schema_version: from,
        to_schema_version: CURRENT_SCHEMA_VERSION,
        changes,
        new_text: doc.to_string(),
    }
}
```

> 멱등 주의: reconcile 가 `schema_version` 을 추가할 때 change 를 기록하지 않도록 Task 4 에서 `path == "schema_version"` 분기를 두었다. version stamp 도 값이 같으면 건드리지 않는다. 따라서 두 번째 실행은 changes 빈 배열.

- [ ] **Step 4: 통과 확인**

Run: `CARGO_TARGET_DIR=/build/out/cargo-target/target cargo test -p kebab-config migrate_document > /tmp/t.log 2>&1; echo EXIT=$?`
Expected: `EXIT=0`. 전체: `cargo test -p kebab-config` 도 `EXIT=0`.

- [ ] **Step 5: Commit**

```bash
git add crates/kebab-config/src/migrate.rs
git commit -m "feat(config): migrate_document — step+reconcile+stamp, 멱등"
```

---

## Task 7: kebab-app facade — 파일 read/백업/atomic write + dry-run

**Files:**
- Modify: `crates/kebab-app/src/lib.rs`
- Test: `crates/kebab-app/src/lib.rs` (`#[cfg(test)]`) 또는 `crates/kebab-app/tests/config_migrate.rs`

- [ ] **Step 1: 실패 테스트 추가**

`crates/kebab-app/tests/config_migrate.rs` 생성:
```rust
use std::fs;

#[test]
fn migrate_writes_backup_and_atomic_with_dry_run_noop() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("config.toml");
    fs::write(&cfg, "schema_version = 1\n\n[workspace]\nroot = \"/n\"\ninclude = [\"*.md\"]\n").unwrap();

    // dry-run: 파일·백업 미변경.
    let report = kebab_app::config_migrate_with_config_path(Some(&cfg), true).unwrap();
    assert!(report.changed);
    assert!(report.dry_run);
    assert!(report.backup_path.is_none());
    assert!(!dir.path().join("config.toml.bak").exists());
    assert!(fs::read_to_string(&cfg).unwrap().contains("include"), "dry-run modified file");

    // 실제 적용: 백업 생성 + 파일 갱신.
    let report = kebab_app::config_migrate_with_config_path(Some(&cfg), false).unwrap();
    assert!(report.changed);
    assert!(!report.dry_run);
    assert!(report.backup_path.is_some());
    assert!(dir.path().join("config.toml.bak").exists());
    let new = fs::read_to_string(&cfg).unwrap();
    assert!(!new.contains("include"));
    assert!(new.contains("[ingest.expansion]"));

    // 멱등: 재실행 changed=false.
    let report = kebab_app::config_migrate_with_config_path(Some(&cfg), false).unwrap();
    assert!(!report.changed);
}

#[test]
fn migrate_missing_file_errors() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("nope.toml");
    assert!(kebab_app::config_migrate_with_config_path(Some(&cfg), false).is_err());
}
```

`crates/kebab-app/Cargo.toml` 의 `[dev-dependencies]` 에 `tempfile` 이 없으면 추가(`tempfile = "3"`). 먼저 grep 으로 확인.

- [ ] **Step 2: 실패 확인**

Run: `CARGO_TARGET_DIR=/build/out/cargo-target/target cargo test -p kebab-app --test config_migrate > /tmp/t.log 2>&1; echo EXIT=$?`
Expected: FAIL (`config_migrate_with_config_path` 미정의)

- [ ] **Step 3: facade + 리포트 타입 구현**

`crates/kebab-app/src/lib.rs` 에 추가(doctor 함수 근처):
```rust
/// `kebab config migrate` 의 결과(wire `config_migration.v1` 소스).
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct ConfigMigrationReport {
    pub schema_version: String, // 항상 "config_migration.v1"
    pub config_path: String,
    pub dry_run: bool,
    pub from_schema_version: u32,
    pub to_schema_version: u32,
    pub changed: bool,
    pub backup_path: Option<String>,
    pub changes: Vec<kebab_config::migrate::MigrationChange>,
}

/// 사용자 config.toml 을 새 스키마로 마이그레이션한다(facade).
/// `config_path` 미지정 시 XDG 기본. `dry_run=true` 면 파일·백업 미변경.
pub fn config_migrate_with_config_path(
    config_path: Option<&std::path::Path>,
    dry_run: bool,
) -> anyhow::Result<ConfigMigrationReport> {
    let path: PathBuf = match config_path {
        Some(p) => p.to_path_buf(),
        None => kebab_config::Config::xdg_config_path(),
    };
    if !path.exists() {
        anyhow::bail!(
            "config 파일이 없습니다: {} — 먼저 `kebab init` 을 실행하세요.",
            path.display()
        );
    }
    let text = std::fs::read_to_string(&path)?;
    let outcome = kebab_config::migrate::migrate_document(&text);

    let mut backup_path = None;
    if !dry_run && outcome.changed() {
        // 백업.
        let bak = path.with_extension("toml.bak");
        std::fs::copy(&path, &bak)?;
        backup_path = Some(bak.display().to_string());
        // atomic: tmp 쓰기 → 재파싱 검증 → rename.
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, &outcome.new_text)?;
        // round-trip 검증(손상 방지).
        if kebab_config::Config::from_file(&tmp).is_err() {
            std::fs::remove_file(&tmp).ok();
            anyhow::bail!("마이그레이션 결과가 유효하지 않아 원본을 보존합니다.");
        }
        std::fs::rename(&tmp, &path)?;
    }

    Ok(ConfigMigrationReport {
        schema_version: "config_migration.v1".to_string(),
        config_path: path.display().to_string(),
        dry_run,
        from_schema_version: outcome.from_schema_version,
        to_schema_version: outcome.to_schema_version,
        changed: outcome.changed(),
        backup_path,
        changes: outcome.changes,
    })
}
```

`MigrationChange` 가 `Serialize` 인지 확인(Task 1 에서 derive 함). `kebab-app/Cargo.toml` 에 `kebab-config` 의존성은 이미 있음.

- [ ] **Step 4: 통과 확인**

Run: `CARGO_TARGET_DIR=/build/out/cargo-target/target cargo test -p kebab-app --test config_migrate > /tmp/t.log 2>&1; echo EXIT=$?`
Expected: `EXIT=0`

- [ ] **Step 5: Commit**

```bash
git add crates/kebab-app/src/lib.rs crates/kebab-app/Cargo.toml crates/kebab-app/tests/config_migrate.rs
git commit -m "feat(app): config_migrate facade — 백업+atomic write+dry-run"
```

---

## Task 8: init_workspace 가 annotated_default_document 사용

**Files:**
- Modify: `crates/kebab-app/src/lib.rs:145-180` (`init_workspace` 의 config 쓰기 블록)

- [ ] **Step 1: 실패 테스트 추가**

`crates/kebab-app/tests/config_migrate.rs` 에 추가(init 은 XDG 경로를 쓰므로 직접 함수가 아닌, annotated doc 의 산출을 검증 — kebab-config 차원에서 이미 Task3 가 커버. 여기선 init 산출이 섹션 주석을 포함하는지 단위로):
```rust
#[test]
fn annotated_default_serialization_contains_section_comments() {
    let doc = kebab_config::migrate::annotated_default_document();
    let text = doc.to_string();
    assert!(text.contains("doc-side 별칭"), "section comment missing:\n{text}");
    assert!(text.contains("[ingest.expansion]"));
}
```

- [ ] **Step 2: 실패 확인 / 현황 확인**

Run: `CARGO_TARGET_DIR=/build/out/cargo-target/target cargo test -p kebab-app annotated_default_serialization > /tmp/t.log 2>&1; echo EXIT=$?`
Expected: PASS (Task3 가 이미 구현) — 이 테스트는 회귀 가드. init 본문 교체가 목적.

- [ ] **Step 3: init_workspace 본문 교체**

`crates/kebab-app/src/lib.rs` 의 config 쓰기 블록(약 145~180행, `let toml_text = toml::to_string_pretty(&cfg)?;` ~ `std::fs::write(&cfg_path, combined)?;`)을 다음으로 교체:
```rust
    if !cfg_path.exists() || force {
        // init 과 migrate 가 동일한 "주석 달린 default" 문서를 공유한다.
        let doc = kebab_config::migrate::annotated_default_document();
        std::fs::write(&cfg_path, doc.to_string())?;
    }
```
(기존 `let cfg = ...defaults()`, `toml::to_string_pretty`, `header` 문자열, `combined` 조립은 모두 삭제 — 헤더는 annotated_default_document 의 HEADER 로 이전됨.)

- [ ] **Step 4: 통과 확인**

Run: `CARGO_TARGET_DIR=/build/out/cargo-target/target cargo test -p kebab-app > /tmp/t.log 2>&1; echo EXIT=$?`
Expected: `EXIT=0`

- [ ] **Step 5: Commit**

```bash
git add crates/kebab-app/src/lib.rs crates/kebab-app/tests/config_migrate.rs
git commit -m "feat(app): init 이 주석 달린 default 문서 사용(섹션 주석 포함)"
```

---

## Task 9: doctor 에 config_migration 체크 추가

**Files:**
- Modify: `crates/kebab-app/src/lib.rs` (`doctor_with_config_path`, 약 3214행 `let ok = ...` 직전)

- [ ] **Step 1: 실패 테스트 추가**

`crates/kebab-app/tests/config_migrate.rs`:
```rust
#[test]
fn doctor_flags_outdated_config() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("config.toml");
    // 옛 파일(섹션 누락 + deprecated).
    fs::write(&cfg, "schema_version = 1\n\n[workspace]\nroot = \"/n\"\ninclude=[\"*.md\"]\n").unwrap();
    let report = kebab_app::doctor_with_config_path(Some(&cfg)).unwrap();
    let check = report.checks.iter().find(|c| c.name == "config_migration").unwrap();
    assert!(!check.ok, "outdated config should fail check");
    assert!(check.hint.as_deref().unwrap().contains("config migrate"));
    assert!(!report.ok, "overall doctor should be false");

    // migrate 후엔 통과.
    kebab_app::config_migrate_with_config_path(Some(&cfg), false).unwrap();
    let report = kebab_app::doctor_with_config_path(Some(&cfg)).unwrap();
    let check = report.checks.iter().find(|c| c.name == "config_migration").unwrap();
    assert!(check.ok, "after migrate should pass");
}
```

- [ ] **Step 2: 실패 확인**

Run: `CARGO_TARGET_DIR=/build/out/cargo-target/target cargo test -p kebab-app doctor_flags_outdated > /tmp/t.log 2>&1; echo EXIT=$?`
Expected: FAIL (config_migration 체크 없음)

- [ ] **Step 3: 체크 추가**

`crates/kebab-app/src/lib.rs` 의 `doctor_with_config_path` 에서 `let ok = checks.iter().all(...)` 직전에 추가:
```rust
    // config_migration — 사용자 파일이 새 스키마와 동기인지(dry-run 마이그레이션).
    // 파일이 존재할 때만 점검(없으면 defaults 사용 중이라 마이그레이션 무의미).
    if cfg_path.exists() {
        if let Ok(text) = std::fs::read_to_string(&cfg_path) {
            let outcome = kebab_config::migrate::migrate_document(&text);
            let (mok, detail, hint) = if outcome.changed() {
                let added = outcome
                    .changes
                    .iter()
                    .filter(|c| {
                        matches!(
                            c.kind,
                            kebab_config::migrate::ChangeKind::AddedSection
                                | kebab_config::migrate::ChangeKind::AddedKey
                        )
                    })
                    .count();
                let removed = outcome.changes.len() - added;
                (
                    false,
                    format!(
                        "{} pending changes (added {added}, removed {removed} deprecated)",
                        outcome.changes.len()
                    ),
                    Some("run `kebab config migrate` to update your config.toml".to_string()),
                )
            } else {
                (
                    true,
                    format!("config up to date (schema v{})", outcome.to_schema_version),
                    None,
                )
            };
            checks.push(DoctorCheck {
                name: "config_migration".to_string(),
                ok: mok,
                detail,
                hint,
            });
        }
    }
```

- [ ] **Step 4: 통과 확인**

Run: `CARGO_TARGET_DIR=/build/out/cargo-target/target cargo test -p kebab-app doctor_flags_outdated > /tmp/t.log 2>&1; echo EXIT=$?`
Expected: `EXIT=0`. 기존 doctor 테스트(`cargo test -p kebab-app`)도 깨지지 않는지 확인(default config 테스트가 ok=true 가정한다면 영향 점검).

- [ ] **Step 5: Commit**

```bash
git add crates/kebab-app/src/lib.rs crates/kebab-app/tests/config_migrate.rs
git commit -m "feat(app): doctor 가 config 마이그레이션 필요 시 ok=false 로 안내"
```

---

## Task 10: CLI `kebab config migrate` 서브커맨드

**Files:**
- Modify: `crates/kebab-cli/src/main.rs` (Cmd enum + match 핸들러)
- Modify: `crates/kebab-cli/src/wire.rs` (wire_config_migration)

- [ ] **Step 1: wire 함수 + 실패 테스트(wire.rs)**

`crates/kebab-cli/src/wire.rs` 에 추가:
```rust
/// `config_migration.v1` wire 직렬화.
pub fn wire_config_migration(r: &kebab_app::ConfigMigrationReport) -> serde_json::Value {
    serde_json::json!({
        "schema_version": r.schema_version,
        "config_path": r.config_path,
        "dry_run": r.dry_run,
        "from_schema_version": r.from_schema_version,
        "to_schema_version": r.to_schema_version,
        "changed": r.changed,
        "backup_path": r.backup_path,
        "changes": r.changes.iter().map(|c| serde_json::json!({
            "kind": c.kind,
            "path": c.path,
            "detail": c.detail,
        })).collect::<Vec<_>>(),
    })
}
```
(테스트는 통합 단계에서 CLI 스냅샷으로 — 단위 테스트가 wire.rs 에 있으면 패턴 따라 추가.)

- [ ] **Step 2: Cmd enum 에 Config 그룹 추가**

`crates/kebab-cli/src/main.rs` 의 `enum Cmd` 에 variant 추가(`Doctor,` 근처):
```rust
    /// config.toml 관리.
    Config {
        #[command(subcommand)]
        what: ConfigWhat,
    },
```
같은 파일에 서브커맨드 enum 추가(다른 `*What` enum 들 근처):
```rust
#[derive(Subcommand, Debug)]
enum ConfigWhat {
    /// 기존 config.toml 을 새 스키마로 마이그레이션(빠진 섹션 추가 + 멱등).
    Migrate {
        /// 변경만 출력하고 파일은 수정하지 않는다.
        #[arg(long)]
        dry_run: bool,
    },
}
```

- [ ] **Step 3: match 핸들러 추가**

`main()` 의 `match cli.command` 에 arm 추가(`Cmd::Doctor =>` 근처 패턴 차용 — `cli.json` 플래그명은 기존 코드 확인 후 맞춤):
```rust
        Cmd::Config { what } => match what {
            ConfigWhat::Migrate { dry_run } => {
                let report = kebab_app::config_migrate_with_config_path(
                    cli.config.as_deref(),
                    dry_run,
                )?;
                if cli.json {
                    println!("{}", wire::wire_config_migration(&report));
                } else if !report.changed {
                    println!("config 이미 최신입니다 (schema v{}).", report.to_schema_version);
                } else {
                    let verb = if report.dry_run { "변경 예정" } else { "적용됨" };
                    println!(
                        "config 마이그레이션 {verb}: v{} → v{} ({} changes)",
                        report.from_schema_version,
                        report.to_schema_version,
                        report.changes.len()
                    );
                    for c in &report.changes {
                        println!("  - [{:?}] {} — {}", c.kind, c.path, c.detail);
                    }
                    if let Some(bak) = &report.backup_path {
                        println!("백업: {bak}");
                    }
                    if report.dry_run {
                        println!("(--dry-run: 파일 미수정. 적용하려면 --dry-run 없이 재실행)");
                    }
                }
            }
        },
```
> `cli.json` / `cli.config` 의 정확한 필드명은 같은 파일 다른 arm(예: Doctor, Search)에서 확인해 맞춘다. `--json` 이 전역 플래그인지 서브커맨드별인지도 기존 패턴을 따른다.

- [ ] **Step 4: 빌드 + smoke**

Run: `CARGO_TARGET_DIR=/build/out/cargo-target/target cargo build -p kebab-cli > /tmp/t.log 2>&1; echo EXIT=$?`
Expected: `EXIT=0`

수동 smoke:
```bash
BIN=/build/out/cargo-target/target/debug/kebab
T=$(mktemp -d)
printf 'schema_version = 1\n\n[workspace]\nroot = "/n"\ninclude=["*.md"]\n' > $T/config.toml
$BIN --config $T/config.toml config migrate --dry-run
$BIN --config $T/config.toml config migrate
$BIN --config $T/config.toml config migrate   # 멱등 → "이미 최신"
$BIN --config $T/config.toml --json config migrate --dry-run
ls $T   # config.toml + config.toml.bak
```
Expected: dry-run 은 변경 목록 + 파일 미수정, 실제 적용은 .bak 생성 + 섹션 추가, 재실행은 "이미 최신", --json 은 `config_migration.v1`.

- [ ] **Step 5: Commit**

```bash
git add crates/kebab-cli/src/main.rs crates/kebab-cli/src/wire.rs
git commit -m "feat(cli): kebab config migrate 서브커맨드(+--dry-run/--json)"
```

---

## Task 11: wire schema 파일 + schema 목록 등록

**Files:**
- Create: `docs/wire-schema/v1/config_migration.v1.schema.json`
- Modify: `crates/kebab-app/src/schema.rs` (약 110행, schema 라벨 목록)

- [ ] **Step 1: schema 목록 실패 테스트**

`crates/kebab-app/src/schema.rs` 의 schema 목록(`"doctor.v1",` 가 있는 배열)에 `"config_migration.v1",` 가 포함되는지 검증하는 테스트가 있으면 그걸 갱신; 없으면 추가:
```rust
#[test]
fn schema_list_includes_config_migration() {
    assert!(SCHEMAS.contains(&"config_migration.v1"));
}
```
(배열 상수명은 실제 코드 확인 — `SCHEMAS` 또는 유사.)

- [ ] **Step 2: 실패 확인**

Run: `CARGO_TARGET_DIR=/build/out/cargo-target/target cargo test -p kebab-app schema_list_includes_config_migration > /tmp/t.log 2>&1; echo EXIT=$?`
Expected: FAIL

- [ ] **Step 3: 목록 등록 + JSON 스키마 파일**

`schema.rs` 의 라벨 배열에 `"config_migration.v1",` 추가(알파벳/논리 순서 맞춤).

`docs/wire-schema/v1/config_migration.v1.schema.json` 생성:
```json
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "title": "config_migration.v1",
  "type": "object",
  "required": ["schema_version", "config_path", "dry_run", "from_schema_version", "to_schema_version", "changed", "changes"],
  "properties": {
    "schema_version": { "const": "config_migration.v1" },
    "config_path": { "type": "string" },
    "dry_run": { "type": "boolean" },
    "from_schema_version": { "type": "integer" },
    "to_schema_version": { "type": "integer" },
    "changed": { "type": "boolean" },
    "backup_path": { "type": ["string", "null"] },
    "changes": {
      "type": "array",
      "items": {
        "type": "object",
        "required": ["kind", "path", "detail"],
        "properties": {
          "kind": { "enum": ["added_section", "added_key", "removed_deprecated"] },
          "path": { "type": "string" },
          "detail": { "type": "string" }
        }
      }
    }
  }
}
```

- [ ] **Step 4: 통과 확인**

Run: `CARGO_TARGET_DIR=/build/out/cargo-target/target cargo test -p kebab-app > /tmp/t.log 2>&1; echo EXIT=$?`
Expected: `EXIT=0`. wire schema 디렉토리와 코드 목록 일치를 검사하는 테스트가 있으면 함께 통과.

- [ ] **Step 5: Commit**

```bash
git add docs/wire-schema/v1/config_migration.v1.schema.json crates/kebab-app/src/schema.rs
git commit -m "feat(wire): config_migration.v1 스키마 + schema 목록 등록"
```

---

## Task 12: 전체 게이트 + 문서 동기화

**Files:**
- Modify: `README.md` (Configuration §), `docs/SMOKE.md`, `docs/DOGFOOD.md`

- [ ] **Step 1: clippy + 전체 테스트 게이트**

```bash
CARGO_TARGET_DIR=/build/out/cargo-target/target cargo clippy --workspace --all-targets -- -D warnings > /tmp/clippy.log 2>&1; echo EXIT=$?
CARGO_TARGET_DIR=/build/out/cargo-target/target cargo test --workspace --no-fail-fast -j 1 > /tmp/test.log 2>&1; echo EXIT=$?
```
Expected: 둘 다 `EXIT=0`. 실패 시 로그 확인 후 수정.

- [ ] **Step 2: README Configuration § 갱신**

`README.md` Configuration 절(약 90~127행)에 `kebab config migrate` 한 줄 추가(불릿 목록):
```markdown
- **`kebab config migrate`** — 새 버전에서 추가된 config 섹션을 기존 `config.toml` 에
  설명 주석과 함께 채워 넣는다(사용자가 손본 값·주석·순서는 보존, 멱등, 자동 `.bak` 백업).
  `--dry-run` 으로 변경 미리보기. `kebab doctor` 가 갱신 필요 시 안내.
```
config 예시 블록은 `annotated_default_document` 산출과 큰 괴리가 없으면 유지(섹션 주석이 추가됐다는 점만 위 불릿이 설명).

- [ ] **Step 3: docs/SMOKE.md 에 migrate 단계**

config 예시 블록 뒤에 `config migrate --dry-run` smoke 한 단계 추가(기존 SMOKE 흐름 패턴 따라).

- [ ] **Step 4: docs/DOGFOOD.md 시나리오 추가**

config 관련 section 에 "옛 config(섹션 누락) → `config migrate` → 섹션 가시성 + 멱등 확인" 시나리오 추가.

- [ ] **Step 5: Commit**

```bash
git add README.md docs/SMOKE.md docs/DOGFOOD.md
git commit -m "docs: config migrate surface 동기화(README/SMOKE/DOGFOOD)"
```

---

## Task 13: 도그푸딩 + HOTFIXES/HANDOFF (머지 직전/직후)

**Files:**
- Modify: `tasks/HOTFIXES.md`, `HANDOFF.md`

- [ ] **Step 1: 실제 도그푸딩**

릴리스 binary 로 실제 옛 config(예: v0.20 시절 `.bak` 또는 수동 축약본)에 대해 migrate 실행:
```bash
CARGO_TARGET_DIR=/build/out/cargo-target/target cargo build --release > /tmp/rel.log 2>&1; echo EXIT=$?
BIN=/build/out/cargo-target/target/release/kebab
# /build/dogfood/ 에 옛 config 준비 후 migrate dry-run → 적용 → 멱등 + doctor 확인.
```
추가된 섹션 수, 제거된 deprecated, 멱등(2회차 "이미 최신") evidence 수집.

- [ ] **Step 2: HOTFIXES dated entry**

`tasks/HOTFIXES.md` 상단에 `## 2026-05-31 — config 마이그레이션` 추가: trigger, 메커니즘(reconciliation+step), 도그푸딩 evidence(추가 섹션 N개, workspace.include 제거, 멱등), known limitation(append 순서, doctor ok=false 의미).

- [ ] **Step 3: HANDOFF 한 줄**

`HANDOFF.md` "머지 후 발견된 버그 / 결정 (요약)" 에 config 마이그레이션 한 줄.

- [ ] **Step 4: Commit**

```bash
git add tasks/HOTFIXES.md HANDOFF.md
git commit -m "docs: config 마이그레이션 도그푸딩 evidence + HANDOFF"
```

- [ ] **Step 5: PR (gitea REST)**

gitea-ops skill 로 `feat/config-migration` → `main` PR 생성. 리뷰 루프(round1 opus, closure verify sonnet) → 머지.

---

## 마이그레이션 노트 (실행자용)

- **버전 bump**: schema_version 은 additive(데이터 무효화 아님) → 읽기 호환 유지. workspace `Cargo.toml` binary version bump 는 surface 누적 기준 사용자 판단(CLAUDE.md §Versioning). 본 plan 은 binary version 을 건드리지 않음.
- **facade rule**: kebab-cli 는 kebab-app facade(`config_migrate_with_config_path`)만 호출. 순수 변환은 kebab-config. 위반 금지.
- **toml_edit 0.22 decor API**: `leaf_decor_mut`/`decor_mut`/`key_mut` 시그니처가 안 맞으면 `cargo doc -p toml_edit --open` 으로 확인 후 동등 API 로 조정. TDD 테스트가 회귀 가드.
- **빌드**: 항상 `CARGO_TARGET_DIR=/build/out/cargo-target/target ... > /tmp/x.log 2>&1; echo EXIT=$?`. `cargo | grep` 금지.
- **무관 변경**: `fixtures/markdown/long-section.chunks.snapshot.json` 의 기존 WIP 변경은 이 작업과 무관 — 스테이징하지 말 것.
