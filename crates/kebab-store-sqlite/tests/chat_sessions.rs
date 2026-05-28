//! p9-fb-17: `ChatSessionRepo` impl for `SqliteStore`. Verifies the
//! V005 schema, insert/list/delete, monotonic turn_index, and
//! ON DELETE CASCADE.

use kebab_config::Config;
use kebab_core::traits::{ChatSessionRepo, ChatSessionRow, ChatTurnRow};
use kebab_store_sqlite::SqliteStore;
use tempfile::TempDir;

fn config_for(tmp: &TempDir) -> Config {
    let mut c = Config::defaults();
    c.storage.data_dir = tmp.path().to_string_lossy().into_owned();
    c
}

fn open_store(tmp: &TempDir) -> SqliteStore {
    let cfg = config_for(tmp);
    let store = SqliteStore::open(&cfg).unwrap();
    store.run_migrations().unwrap();
    store
}

fn make_session(id: &str) -> ChatSessionRow {
    ChatSessionRow {
        session_id: id.to_string(),
        created_at: 1_700_000_000,
        updated_at: 1_700_000_000,
        title: Some(format!("Title for {id}")),
        config_snapshot_json: r#"{"prompt_template_version":"rag-v2","llm.model":"gemma4:e4b"}"#
            .to_string(),
    }
}

fn make_turn(session_id: &str, index: u32) -> ChatTurnRow {
    ChatTurnRow {
        turn_id: format!("turn-{session_id}-{index:08x}"),
        session_id: session_id.to_string(),
        turn_index: index,
        question: format!("Q{index} for {session_id}?"),
        answer: format!("A{index} for {session_id}."),
        citations_json: "[]".to_string(),
        created_at: 1_700_000_000 + i64::from(index),
    }
}

#[test]
fn create_get_roundtrip() {
    let tmp = TempDir::new().unwrap();
    let store = open_store(&tmp);
    let session = make_session("sess-1");
    store.create_session(&session).unwrap();
    let fetched = store
        .get_session("sess-1")
        .unwrap()
        .expect("session present");
    assert_eq!(fetched, session);
}

#[test]
fn get_missing_session_returns_none() {
    let tmp = TempDir::new().unwrap();
    let store = open_store(&tmp);
    assert!(store.get_session("nope").unwrap().is_none());
}

#[test]
fn create_session_pk_collision_errors() {
    let tmp = TempDir::new().unwrap();
    let store = open_store(&tmp);
    let session = make_session("dup");
    store.create_session(&session).unwrap();
    let err = store.create_session(&session).unwrap_err();
    assert!(
        format!("{err:#}").contains("UNIQUE")
            || format!("{err:#}").contains("constraint")
            || format!("{err:#}").to_lowercase().contains("primary key"),
        "expected PK collision error: {err:#}"
    );
}

#[test]
fn append_turn_then_list_in_order() {
    let tmp = TempDir::new().unwrap();
    let store = open_store(&tmp);
    store.create_session(&make_session("multi")).unwrap();
    for i in 0..3 {
        store.append_turn(&make_turn("multi", i)).unwrap();
    }
    let turns = store.list_turns("multi").unwrap();
    assert_eq!(turns.len(), 3);
    for (i, t) in turns.iter().enumerate() {
        assert_eq!(t.turn_index as usize, i);
        assert_eq!(t.question, format!("Q{i} for multi?"));
    }
}

#[test]
fn append_turn_collides_on_same_index() {
    let tmp = TempDir::new().unwrap();
    let store = open_store(&tmp);
    store.create_session(&make_session("dup-turn")).unwrap();
    store.append_turn(&make_turn("dup-turn", 0)).unwrap();
    let err = store.append_turn(&make_turn("dup-turn", 0)).unwrap_err();
    assert!(
        format!("{err:#}").to_lowercase().contains("unique")
            || format!("{err:#}").to_lowercase().contains("constraint")
            || format!("{err:#}").to_lowercase().contains("primary key"),
        "expected unique constraint: {err:#}"
    );
}

#[test]
fn append_turn_bumps_session_updated_at() {
    let tmp = TempDir::new().unwrap();
    let store = open_store(&tmp);
    let session = make_session("bump");
    store.create_session(&session).unwrap();
    let pre = store.get_session("bump").unwrap().unwrap().updated_at;
    let mut t = make_turn("bump", 0);
    t.created_at = pre + 100;
    store.append_turn(&t).unwrap();
    let post = store.get_session("bump").unwrap().unwrap().updated_at;
    assert_eq!(
        post,
        pre + 100,
        "updated_at must follow latest turn's created_at"
    );
}

#[test]
fn delete_session_cascades_to_turns() {
    let tmp = TempDir::new().unwrap();
    let store = open_store(&tmp);
    store.create_session(&make_session("cascade")).unwrap();
    for i in 0..2 {
        store.append_turn(&make_turn("cascade", i)).unwrap();
    }
    store.delete_session("cascade").unwrap();
    assert!(store.get_session("cascade").unwrap().is_none());
    assert_eq!(
        store.list_turns("cascade").unwrap().len(),
        0,
        "ON DELETE CASCADE must wipe orphan turns"
    );
}

#[test]
fn list_sessions_orders_by_updated_at_desc() {
    let tmp = TempDir::new().unwrap();
    let store = open_store(&tmp);
    let mut a = make_session("a");
    a.updated_at = 100;
    let mut b = make_session("b");
    b.updated_at = 300;
    let mut c = make_session("c");
    c.updated_at = 200;
    store.create_session(&a).unwrap();
    store.create_session(&b).unwrap();
    store.create_session(&c).unwrap();
    let listed = store.list_sessions(10).unwrap();
    let ids: Vec<_> = listed.iter().map(|s| s.session_id.clone()).collect();
    assert_eq!(ids, vec!["b", "c", "a"]);
}

#[test]
fn list_sessions_respects_limit() {
    let tmp = TempDir::new().unwrap();
    let store = open_store(&tmp);
    for i in 0..5 {
        store
            .create_session(&make_session(&format!("s{i}")))
            .unwrap();
    }
    assert_eq!(store.list_sessions(2).unwrap().len(), 2);
    assert_eq!(store.list_sessions(100).unwrap().len(), 5);
}
