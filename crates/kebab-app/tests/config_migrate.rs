use std::fs;

#[test]
fn migrate_writes_backup_and_atomic_with_dry_run_noop() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("config.toml");
    fs::write(
        &cfg,
        "schema_version = 1\n\n[workspace]\nroot = \"/n\"\ninclude = [\"*.md\"]\n",
    )
    .unwrap();

    // dry-run: 파일·백업 미변경.
    let report = kebab_app::config_migrate_with_config_path(Some(&cfg), true).unwrap();
    assert!(report.changed);
    assert!(report.dry_run);
    assert!(report.backup_path.is_none());
    assert!(!dir.path().join("config.toml.bak").exists());
    assert!(
        fs::read_to_string(&cfg).unwrap().contains("include"),
        "dry-run modified file"
    );

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

#[test]
fn annotated_default_serialization_contains_section_comments() {
    let doc = kebab_config::migrate::annotated_default_document();
    let text = doc.to_string();
    assert!(text.contains("doc-side 별칭"), "section comment missing:\n{text}");
    assert!(text.contains("[ingest.expansion]"));
}

#[test]
fn doctor_flags_outdated_config() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("config.toml");
    fs::write(
        &cfg,
        "schema_version = 1\n\n[workspace]\nroot = \"/n\"\ninclude=[\"*.md\"]\n",
    )
    .unwrap();
    let report = kebab_app::doctor_with_config_path(Some(&cfg)).unwrap();
    let check = report
        .checks
        .iter()
        .find(|c| c.name == "config_migration")
        .unwrap();
    assert!(!check.ok, "outdated config should fail check");
    assert!(check.hint.as_deref().unwrap().contains("config migrate"));
    assert!(!report.ok, "overall doctor should be false");

    // migrate 후엔 통과.
    kebab_app::config_migrate_with_config_path(Some(&cfg), false).unwrap();
    let report = kebab_app::doctor_with_config_path(Some(&cfg)).unwrap();
    let check = report
        .checks
        .iter()
        .find(|c| c.name == "config_migration")
        .unwrap();
    assert!(check.ok, "after migrate should pass");
}
