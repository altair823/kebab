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
    assert!(new.contains("[ingest.code]"));

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
    assert!(
        text.contains("code ingest skip 정책"),
        "section comment missing:\n{text}"
    );
    assert!(text.contains("[ingest.code]"));
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

/// `doctor` is a diagnostic and must not manufacture the store it is
/// asked about. The `fts_shadow` check (issue #229 / V016) reads SQLite,
/// and `SqliteStore::open` creates the file — so on a machine with no KB
/// yet, running doctor once used to leave an empty `kebab.sqlite` behind.
#[test]
fn doctor_does_not_create_a_store_where_none_exists() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let cfg = dir.path().join("config.toml");
    fs::write(
        &cfg,
        format!(
            "schema_version = 1\n\n[workspace]\nroot = \"/n\"\ninclude=[\"*.md\"]\n\n\
             [storage]\ndata_dir = \"{}\"\n",
            data.display()
        ),
    )
    .unwrap();

    let report = kebab_app::doctor_with_config_path(Some(&cfg)).unwrap();
    let check = report
        .checks
        .iter()
        .find(|c| c.name == "fts_shadow")
        .expect("doctor must report fts_shadow even with no store");
    assert!(check.ok, "a missing store is not a drifted one");

    assert!(
        !data.join(kebab_store_sqlite::SQLITE_FILE).exists(),
        "doctor must not create {} — it only reports",
        kebab_store_sqlite::SQLITE_FILE
    );
}

/// A store that predates V016 can be misaligned already — V009's
/// backfill inserted without an explicit rowid — but there the delete
/// trigger still addresses rows by chunk_id, so the drift is harmless.
/// Telling that user to `kebab reset` would destroy a healthy KB over a
/// condition that applying the migration fixes by itself.
#[test]
fn doctor_does_not_call_a_pre_v016_store_broken() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    let cfg = dir.path().join("config.toml");
    fs::write(
        &cfg,
        format!(
            "schema_version = 1\n\n[workspace]\nroot = \"/n\"\ninclude=[\"*.md\"]\n\n\
             [storage]\ndata_dir = \"{}\"\n",
            data.display()
        ),
    )
    .unwrap();

    // A store stamped at V015 with a shadow deliberately misaligned:
    // exactly what a pre-V016 upgrade can look like.
    let db = data.join(kebab_store_sqlite::SQLITE_FILE);
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch(
        "CREATE TABLE refinery_schema_history (version INTEGER);
         INSERT INTO refinery_schema_history VALUES (15);
         CREATE TABLE chunks (chunk_id TEXT PRIMARY KEY, text TEXT);
         CREATE VIRTUAL TABLE chunks_fts USING fts5(chunk_id UNINDEXED, text);
         INSERT INTO chunks VALUES ('a', 'x'), ('b', 'y');
         INSERT INTO chunks_fts(rowid, chunk_id, text) VALUES (77, 'a', 'x');",
    )
    .unwrap();
    drop(conn);

    let report = kebab_app::doctor_with_config_path(Some(&cfg)).unwrap();
    let check = report
        .checks
        .iter()
        .find(|c| c.name == "fts_shadow")
        .unwrap();
    assert!(
        check.ok,
        "a pre-V016 store must not fail the check: {}",
        check.detail
    );
    assert!(
        check.detail.contains("V016"),
        "the detail should say the migration has not been applied, got {:?}",
        check.detail
    );
    assert!(
        check.hint.as_deref().unwrap_or_default().is_empty(),
        "and must not tell the user to reset: {:?}",
        check.hint
    );
}
