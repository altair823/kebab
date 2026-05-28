//! Unit + snapshot tests for the Inspect pane (P9-4).
//!
//! Tests bypass the facade fetch by hand-populating `InspectState.doc`
//! / `state.chunk`. The fetch path itself is exercised end-to-end by
//! manual smoke (TempDir KB).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use kebab_config::Config;
use kebab_core::{
    AssetId, Block, BlockId, CanonicalDocument, Chunk, ChunkId, ChunkerVersion, CommonBlock,
    DocumentId, HeadingBlock, Inline, Lang, Metadata, ParserVersion, Provenance, ProvenanceEvent,
    ProvenanceKind, SourceSpan, SourceType, TextBlock, TrustLevel, WorkspacePath,
};
use kebab_tui::{
    App, InspectState, InspectTarget, KeyOutcome, Pane, handle_key_inspect, render_inspect,
};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use std::path::PathBuf;
use time::OffsetDateTime;

fn fresh_app() -> App {
    let mut config = Config::defaults();
    config.storage.data_dir = "/tmp/kebab-tui-inspect-tests-noop".to_string();
    config.workspace.root = "/tmp/kebab-tui-inspect-tests-noop/workspace".to_string();
    let mut app = App::new(config).expect("App::new");
    app.focus = Pane::Inspect;
    app.inspect = Some(InspectState::default());
    app
}

fn make_doc() -> CanonicalDocument {
    let doc_id = DocumentId("d".repeat(32));
    let asset_id = AssetId("a".repeat(32));
    let span1 = SourceSpan::Line { start: 1, end: 1 };
    let span2 = SourceSpan::Line { start: 2, end: 5 };
    let common1 = CommonBlock {
        block_id: BlockId("b".repeat(32)),
        heading_path: vec![],
        source_span: span1,
    };
    let common2 = CommonBlock {
        block_id: BlockId("c".repeat(32)),
        heading_path: vec!["Top".into()],
        source_span: span2,
    };
    let blocks = vec![
        Block::Heading(HeadingBlock {
            common: common1,
            level: 1,
            text: "Top".into(),
        }),
        Block::Paragraph(TextBlock {
            common: common2,
            text: "first paragraph body line.".into(),
            inlines: vec![Inline::Text {
                text: "first paragraph body line.".into(),
            }],
        }),
    ];
    let mut user = serde_json::Map::new();
    user.insert(
        "custom_key".into(),
        serde_json::Value::String("custom_val".into()),
    );

    CanonicalDocument {
        doc_id,
        source_asset_id: asset_id,
        workspace_path: WorkspacePath::new("notes/test.md".into()).unwrap(),
        title: "Test Doc".into(),
        lang: Lang("en".into()),
        blocks,
        metadata: Metadata {
            aliases: vec!["alias1".into()],
            tags: vec!["tag-a".into(), "tag-b".into()],
            created_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
            updated_at: OffsetDateTime::from_unix_timestamp(1_700_000_500).unwrap(),
            source_type: SourceType::Note,
            trust_level: TrustLevel::Primary,
            user_id_alias: None,
            user,
            repo: None,
            git_branch: None,
            git_commit: None,
            code_lang: None,
        },
        provenance: Provenance {
            events: vec![ProvenanceEvent {
                at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
                agent: "kb-source-fs".into(),
                kind: ProvenanceKind::Discovered,
                note: None,
            }],
        },
        parser_version: ParserVersion("test-parser".into()),
        schema_version: 1,
        doc_version: 1,
        last_chunker_version: None,
        last_embedding_version: None,
    }
}

fn make_chunk() -> Chunk {
    Chunk {
        chunk_id: ChunkId("e".repeat(32)),
        doc_id: DocumentId("d".repeat(32)),
        block_ids: vec![BlockId("b".repeat(32)), BlockId("c".repeat(32))],
        text: "chunk body line one.\nchunk body line two.".into(),
        heading_path: vec!["Top".into(), "Sub".into()],
        source_spans: vec![SourceSpan::Line { start: 1, end: 5 }],
        token_estimate: 12,
        chunker_version: ChunkerVersion("md-heading-v1".into()),
        policy_hash: "deadbeefdeadbeef".into(),
        tokenized_korean_text: None,
    }
}

fn render_to_string(app: &App, w: u16, h: u16) -> String {
    let backend = TestBackend::new(w, h);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            let area = Rect::new(0, 0, w, h);
            render_inspect(f, area, app);
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn esc_returns_to_recorded_pane() {
    let mut app = fresh_app();
    {
        let s = app.inspect.as_mut().unwrap();
        s.return_to = Pane::Search;
    }
    let outcome = handle_key_inspect(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(outcome, KeyOutcome::SwitchPane(Pane::Search));
}

#[test]
fn q_also_returns() {
    let mut app = fresh_app();
    let outcome = handle_key_inspect(
        &mut app,
        KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
    );
    assert_eq!(outcome, KeyOutcome::SwitchPane(Pane::Library));
}

#[test]
fn j_k_scroll_within_bounds_no_panic() {
    let mut app = fresh_app();
    handle_key_inspect(
        &mut app,
        KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
    );
    assert_eq!(app.inspect.as_ref().unwrap().scroll, 1);
    handle_key_inspect(
        &mut app,
        KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE),
    );
    assert_eq!(app.inspect.as_ref().unwrap().scroll, 0);
    // Underflow saturates at 0
    handle_key_inspect(
        &mut app,
        KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE),
    );
    assert_eq!(app.inspect.as_ref().unwrap().scroll, 0);
}

/// p9-fb-24 task 2: PageDown advances scroll by `PAGE_STEP` (= 10).
/// Pins the constant so a future viewport-aware refactor surfaces
/// here, not silently in user-visible behaviour. Replaces the
/// pre-fb-24 `page_keys_scroll_by_ten` (deleted as duplicate).
#[test]
fn page_down_scrolls_by_ten_in_inspect() {
    let mut app = fresh_app();
    let outcome = handle_key_inspect(
        &mut app,
        KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
    );
    assert_eq!(outcome, KeyOutcome::Continue);
    assert_eq!(app.inspect.as_ref().unwrap().scroll, 10);
}

/// p9-fb-24 task 2: PageUp rewinds scroll by `PAGE_STEP`, saturating
/// at 0 (no underflow).
#[test]
fn page_up_rewinds_by_ten_saturating_in_inspect() {
    let mut app = fresh_app();
    app.inspect.as_mut().unwrap().scroll = 25;
    handle_key_inspect(&mut app, KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE));
    assert_eq!(app.inspect.as_ref().unwrap().scroll, 15);
    app.inspect.as_mut().unwrap().scroll = 3;
    handle_key_inspect(&mut app, KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE));
    assert_eq!(app.inspect.as_ref().unwrap().scroll, 0);
}

#[test]
fn c_toggles_collapse_state() {
    let mut app = fresh_app();
    // First press: nothing collapsed → collapse all.
    handle_key_inspect(
        &mut app,
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE),
    );
    let s = app.inspect.as_ref().unwrap();
    assert!(!s.collapsed.is_empty(), "first c collapses all");
    // Second press: some collapsed → expand all.
    handle_key_inspect(
        &mut app,
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE),
    );
    let s = app.inspect.as_ref().unwrap();
    assert!(s.collapsed.is_empty(), "second c expands all");
}

#[test]
fn no_target_renders_hint_without_panic() {
    let app = fresh_app();
    let rendered = render_to_string(&app, 80, 20);
    assert!(rendered.contains("Inspect"), "header visible");
    assert!(
        rendered.contains("no target") || rendered.contains("press Enter"),
        "hint visible: {rendered}"
    );
}

#[test]
fn loading_state_renders_loading_message() {
    let mut app = fresh_app();
    {
        let s = app.inspect.as_mut().unwrap();
        s.target = Some(InspectTarget::Doc(DocumentId("d".repeat(32))));
        s.loading = true;
    }
    let rendered = render_to_string(&app, 80, 10);
    assert!(rendered.contains("loading"), "loading hint: {rendered}");
}

#[test]
fn doc_view_renders_header_and_metadata() {
    let mut app = fresh_app();
    {
        let s = app.inspect.as_mut().unwrap();
        s.target = Some(InspectTarget::Doc(DocumentId("d".repeat(32))));
        s.doc = Some(make_doc());
    }
    let rendered = render_to_string(&app, 100, 40);
    assert!(rendered.contains("Test Doc"), "title rendered");
    assert!(rendered.contains("notes/test.md"), "doc_path rendered");
    assert!(rendered.contains("test-parser"), "parser_version rendered");
    assert!(rendered.contains("metadata"), "metadata section visible");
    assert!(rendered.contains("tag-a"), "tags rendered");
    assert!(
        rendered.contains("custom_key") || rendered.contains("custom_val"),
        "user metadata pretty-printed"
    );
    assert!(
        rendered.contains("provenance"),
        "provenance section visible"
    );
    assert!(rendered.contains("kb-source-fs"), "agent rendered");
    assert!(rendered.contains("blocks"), "blocks section visible");
    assert!(rendered.contains("Heading L1"), "block describe rendered");
}

#[test]
fn doc_view_collapse_hides_section_body() {
    let mut app = fresh_app();
    {
        let s = app.inspect.as_mut().unwrap();
        s.target = Some(InspectTarget::Doc(DocumentId("d".repeat(32))));
        s.doc = Some(make_doc());
    }
    let pre = render_to_string(&app, 100, 30);
    assert!(pre.contains("kb-source-fs"), "before collapse");
    assert!(pre.contains("Heading L1"), "blocks body before collapse");
    handle_key_inspect(
        &mut app,
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE),
    );
    let post = render_to_string(&app, 100, 30);
    assert!(post.contains("metadata"), "section header still visible");
    assert!(
        post.contains("blocks (2)"),
        "blocks count visible inline on collapsed header: {post}"
    );
    assert!(
        !post.contains("kb-source-fs"),
        "provenance body hidden after collapse: {post}"
    );
    assert!(
        !post.contains("Heading L1"),
        "blocks body hidden after collapse (count must collapse with body): {post}"
    );
}

#[test]
fn chunk_view_renders_text_and_block_ids() {
    let mut app = fresh_app();
    {
        let s = app.inspect.as_mut().unwrap();
        s.target = Some(InspectTarget::Chunk(ChunkId("e".repeat(32))));
        s.chunk = Some(make_chunk());
    }
    let rendered = render_to_string(&app, 100, 40);
    assert!(
        rendered.contains("md-heading-v1"),
        "chunker_version rendered"
    );
    assert!(rendered.contains("Top / Sub"), "heading_path joined");
    assert!(rendered.contains("Line 1-5"), "source span described");
    assert!(
        rendered.contains("chunk body line one"),
        "text body rendered"
    );
    assert!(
        rendered.contains("embeddings (2)"),
        "block_id count rendered inline on embeddings header"
    );
}

/// p9-fb-32: when a doc's `metadata.updated_at` is older than the
/// configured `stale_threshold_days`, the Inspect pane prefixes the
/// `doc_path` value with a Warning-styled `[STALE] ` Span. Threshold
/// 0 (the staleness feature off) must NOT render the badge.
#[test]
fn inspect_doc_header_shows_stale_badge_when_threshold_exceeded() {
    let mut app = fresh_app();
    // Force a non-zero threshold so the staleness post-process can fire.
    app.config.search.stale_threshold_days = 30;
    {
        let s = app.inspect.as_mut().unwrap();
        s.target = Some(InspectTarget::Doc(DocumentId("d".repeat(32))));
        let mut doc = make_doc();
        // Backdate updated_at by 60 days so 60d > 30d threshold.
        doc.metadata.updated_at = OffsetDateTime::now_utc() - time::Duration::days(60);
        s.doc = Some(doc);
    }
    let rendered = render_to_string(&app, 100, 40);
    assert!(
        rendered.contains("[STALE]"),
        "[STALE] badge must render on stale doc header: {rendered}"
    );
    // Same line carrying the doc_path value must show the badge.
    let path_line = rendered
        .lines()
        .find(|l| l.contains("notes/test.md"))
        .expect("doc_path line must render");
    assert!(
        path_line.contains("[STALE]"),
        "doc_path row must carry [STALE] badge: {path_line}"
    );
}

#[test]
fn inspect_doc_header_omits_stale_badge_when_fresh() {
    let mut app = fresh_app();
    app.config.search.stale_threshold_days = 30;
    {
        let s = app.inspect.as_mut().unwrap();
        s.target = Some(InspectTarget::Doc(DocumentId("d".repeat(32))));
        let mut doc = make_doc();
        // 1 day old — under the 30d threshold.
        doc.metadata.updated_at = OffsetDateTime::now_utc() - time::Duration::days(1);
        s.doc = Some(doc);
    }
    let rendered = render_to_string(&app, 100, 40);
    assert!(
        !rendered.contains("[STALE]"),
        "fresh doc must NOT carry [STALE] badge: {rendered}"
    );
}

#[test]
fn inspect_doc_header_omits_stale_badge_when_threshold_zero() {
    let mut app = fresh_app();
    // Threshold 0 = staleness feature disabled.
    app.config.search.stale_threshold_days = 0;
    {
        let s = app.inspect.as_mut().unwrap();
        s.target = Some(InspectTarget::Doc(DocumentId("d".repeat(32))));
        let mut doc = make_doc();
        // Even a year-old doc must not get [STALE] when threshold = 0.
        doc.metadata.updated_at = OffsetDateTime::now_utc() - time::Duration::days(365);
        s.doc = Some(doc);
    }
    let rendered = render_to_string(&app, 100, 40);
    assert!(
        !rendered.contains("[STALE]"),
        "threshold = 0 must disable [STALE] badge regardless of age: {rendered}"
    );
}

#[test]
fn no_inspect_state_returns_to_library() {
    let mut config = Config::defaults();
    config.storage.data_dir = "/tmp/kebab-tui-inspect-tests-noop".into();
    let mut app = App::new(config).unwrap();
    app.focus = Pane::Inspect;
    let outcome = handle_key_inspect(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(outcome, KeyOutcome::SwitchPane(Pane::Library));
}

#[test]
fn enter_inspect_helper_sets_target_and_marks_fetch() {
    let mut app = fresh_app();
    app.inspect = None; // simulate cold state
    kebab_tui::enter_inspect(
        &mut app,
        InspectTarget::Doc(DocumentId("d".repeat(32))),
        Pane::Library,
    );
    let s = app.inspect.as_ref().unwrap();
    assert!(matches!(s.target, Some(InspectTarget::Doc(_))));
    assert_eq!(s.return_to, Pane::Library);
    assert!(s.needs_fetch);
    assert!(s.doc.is_none());
    let _ = PathBuf::from(""); // silence unused-import in some configs
}
