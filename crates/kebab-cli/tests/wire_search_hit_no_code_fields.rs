//! p10-1A-1 Task 13: regression — markdown SearchHit omits `repo` and
//! `code_lang` from JSON when both are `None`.
//!
//! Proves that adding optional fields to SearchHit does not silently
//! inject spurious keys into the existing markdown corpus wire shape.

use kebab_core::{
    ChunkId, ChunkerVersion, Citation, DocumentId, IndexVersion, RetrievalDetail, ScoreKind,
    SearchHit, WorkspacePath,
};

#[test]
fn markdown_hit_omits_repo_and_code_lang() {
    let hit = SearchHit {
        rank: 1,
        chunk_id: ChunkId("c1".into()),
        doc_id: DocumentId("d1".into()),
        doc_path: WorkspacePath::new("notes/foo.md".into()).unwrap(),
        heading_path: vec!["A".into(), "B".into()],
        section_label: Some("B".into()),
        snippet: "hi".into(),
        citation: Citation::Line {
            path: WorkspacePath::new("notes/foo.md".into()).unwrap(),
            start: 1,
            end: 2,
            section: None,
        },
        retrieval: RetrievalDetail::default(),
        index_version: IndexVersion("v1".into()),
        embedding_model: None,
        chunker_version: ChunkerVersion("md-heading-v1".into()),
        indexed_at: time::OffsetDateTime::UNIX_EPOCH,
        stale: false,
        score_kind: ScoreKind::Rrf,
        repo: None,
        code_lang: None,
        source_id: None,
        trust_level: None,
    };
    let s = serde_json::to_string(&hit).unwrap();
    assert!(
        !s.contains("\"repo\""),
        "repo should be absent from markdown hit JSON: {s}"
    );
    assert!(
        !s.contains("\"code_lang\""),
        "code_lang should be absent from markdown hit JSON: {s}"
    );
}
