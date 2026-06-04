//! config.toml 마이그레이션 엔진 (순수 변환, I/O 없음).
//!
//! 두 메커니즘: (1) reconciliation — default 구조에 있고 사용자 파일에
//! 없는 섹션/키를 주석과 함께 추가. (2) step 체인 — schema_version 기반
//! non-additive 변환(deprecated 제거 등). 자세한 계약은 spec
//! `docs/superpowers/specs/2026-05-31-config-migration-design.md`.

use toml_edit::DocumentMut;

/// 현재 바이너리가 이해하는 config 스키마 버전. 마이그레이션 완료 시
/// 사용자 파일의 `schema_version` 을 이 값으로 stamp 한다.
pub const CURRENT_SCHEMA_VERSION: u32 = 2;

/// 한 번의 마이그레이션에서 발생한 개별 변경.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct MigrationChange {
    pub kind: ChangeKind,
    /// dotted path, 예: `ingest.code`, `workspace.include`.
    pub path: String,
    /// 사람·wire 용 한 줄 설명.
    pub detail: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    AddedSection,
    AddedKey,
    RemovedDeprecated,
}

/// 마이그레이션 결과 요약(순수 변환 단계 산출). I/O 계층이 backup_path
/// 등을 채워 wire 로 내보낸다.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
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

/// 문서 최상단 헤더(경로 정책 등). 기존 init 헤더를 이전.
const HEADER: &str = "\
# kebab config — `~/.config/kebab/config.toml`.
#
# `workspace.root` accepts: 절대 / tilde(~) / env(${VAR}) / 상대 경로.
#   상대 경로의 base 는 cwd 가 아니라 THIS config 파일의 디렉토리.
#
# 처리 가능한 형식 (extractor 가 자동 결정 — config 에 명시할 수 없음):
#   • Markdown: .md
#   • 이미지:   .png .jpg .jpeg  (OCR + caption)
#   • PDF:      .pdf
#
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
        "models.nli" => "# NLI(groundedness) 모델.",
        "search" => "# 검색 기본 k·stale 기준·fusion.",
        "rag" => "# 답변 생성: prompt 템플릿·score gate·NLI.",
        "image" => "# 이미지 OCR + 캡션(기본 off, asset 당 모델 호출 비용).",
        "image.ocr" => "# 이미지 OCR(기본 off).",
        "image.caption" => "# 이미지 캡션(기본 off).",
        "ui" => "# TUI 팔레트·role 스타일.",
        "ingest" => "# ingest 정책(code skip 등).",
        "ingest.code" => "# code ingest skip 정책(.gitignore 자동 honor).",
        "pdf" => "# PDF ingest. scanned PDF OCR 은 기본 off(page 당 cost).",
        "pdf.ocr" => "# scanned PDF page-단위 OCR(기본 off).",
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
        first_key.leaf_decor_mut().set_prefix(format!("{HEADER}\n"));
    }

    annotate_table(doc.as_table_mut(), "");
    doc
}

/// 재귀적으로 하위 테이블에 헤더 주석 부착. `prefix_path` 는 dotted 누적 경로.
/// annotated_default_document 는 항상 주석 없는 defaults 에서 새로 만들므로
/// 무조건 부착해도 중복되지 않는다.
fn annotate_table(table: &mut toml_edit::Table, prefix_path: &str) {
    let keys: Vec<String> = table.iter().map(|(k, _)| k.to_string()).collect();
    for key in keys {
        let path = if prefix_path.is_empty() {
            key.clone()
        } else {
            format!("{prefix_path}.{key}")
        };
        if let Some(item) = table.get_mut(&key) {
            if let Some(sub) = item.as_table_mut() {
                if let Some(c) = section_comment(&path) {
                    sub.decor_mut().set_prefix(format!("\n{c}\n"));
                }
                annotate_table(sub, &path);
            }
        }
    }
}

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
                // schema_version 키는 stamp 단계가 다룬다(change 기록 X).
                if path == "schema_version" {
                    user.insert(key, ref_item.clone());
                    continue;
                }
                let kind = if ref_item.is_table() {
                    ChangeKind::AddedSection
                } else {
                    ChangeKind::AddedKey
                };
                user.insert(key, ref_item.clone());
                changes.push(MigrationChange {
                    kind,
                    path: path.clone(),
                    detail: section_comment(&path).map_or_else(
                        || format!("{key} 추가"),
                        |c| c.trim_start_matches("# ").to_string(),
                    ),
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
        .and_then(toml_edit::Item::as_integer)
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
        .and_then(toml_edit::Item::as_integer)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn annotated_default_has_all_sections_and_parses_back_to_defaults() {
        let doc = annotated_default_document();
        let text = doc.to_string();
        // v3: 미디어 형식 섹션이 전부 `[ingest.*]` 하위로 통합됐다. IngestCfg
        // 는 스칼라(병렬도) 필드가 있어 bare `[ingest]` + 하위 테이블이 함께
        // 직렬화된다.
        for section in [
            "[workspace]",
            "[ingest]",
            "[ingest.chunking]",
            "[ingest.code]",
            "[ingest.image.ocr]",
            "[ingest.pdf.ocr]",
            "[logging]",
            "[ui]",
        ] {
            assert!(text.contains(section), "missing {section}:\n{text}");
        }
        assert!(text.contains("# "), "no comments attached");
        let back: crate::Config = toml::from_str(&text).expect("parse annotated default");
        assert_eq!(back, crate::Config::defaults());
    }

    #[test]
    fn reconcile_adds_missing_section_preserving_user_values_and_comments() {
        // ingest 통째 누락(→ ingest.code 추가), logging 통째 누락,
        // default_k 는 사용자가 바꿈, 주석 보유.
        let user_text = "\
schema_version = 1

[workspace]
root = \"/my/notes\"   # 내 워크스페이스

[search]
default_k = 25
";
        let mut user: DocumentMut = user_text.parse().unwrap();
        let reference = annotated_default_document();
        let ref_tbl = reference.as_table().clone();
        let mut changes = Vec::new();
        reconcile(&ref_tbl, user.as_table_mut(), "", &mut changes);
        let out = user.to_string();

        // 누락된 [ingest.code] 가 주석과 함께 추가.
        assert!(out.contains("[ingest.code]"), "ingest.code not added:\n{out}");
        // 통째 누락된 logging 추가.
        assert!(out.contains("[logging]"), "logging not added");
        // 사용자 값/주석/기존 섹션 보존.
        assert!(out.contains("root = \"/my/notes\""));
        assert!(out.contains("# 내 워크스페이스"));
        assert!(out.contains("default_k = 25"));
        // 새 섹션 주석 부착.
        assert!(out.contains("code ingest skip 정책"));
        // 통째 누락 부모는 부모 경로로 한 번 기록.
        assert!(
            changes
                .iter()
                .any(|c| c.kind == ChangeKind::AddedSection && c.path == "ingest")
        );
        assert!(
            changes
                .iter()
                .any(|c| c.kind == ChangeKind::AddedSection && c.path == "logging")
        );
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
        let ref_tbl = reference.as_table().clone();
        let mut changes = Vec::new();
        reconcile(&ref_tbl, user.as_table_mut(), "", &mut changes);
        let out = user.to_string();
        assert!(out.contains("score_gate = 0.8"), "user value clobbered:\n{out}");
        assert!(!changes.iter().any(|c| c.path == "rag.score_gate"));
    }

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
        assert!(
            changes
                .iter()
                .any(|c| c.kind == ChangeKind::RemovedDeprecated && c.path == "workspace.include")
        );
        let mut changes2 = Vec::new();
        step_1_to_2(&mut user, &mut changes2);
        assert!(changes2.is_empty());
    }

    fn read_schema_version(text: &str) -> u32 {
        let doc: DocumentMut = text.parse().unwrap();
        doc.get("schema_version")
            .and_then(toml_edit::Item::as_integer)
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
        assert!(outcome.new_text.contains("[ingest.code]"));
        assert_eq!(read_schema_version(&outcome.new_text), CURRENT_SCHEMA_VERSION);

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
}
