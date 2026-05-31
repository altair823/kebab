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
