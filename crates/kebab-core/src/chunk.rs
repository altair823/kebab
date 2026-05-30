//! Chunk (§3.5).

use serde::{Deserialize, Serialize};

use crate::document::SourceSpan;
use crate::ids::{BlockId, ChunkId, DocumentId};
use crate::versions::ChunkerVersion;

/// A unit of retrievable text per design §3.5 + §5.5.
///
/// `policy_hash` is the chunker's hex digest of the active `ChunkPolicy`
/// (e.g. `target_tokens`, `overlap_tokens`). It mirrors the §5.5 SQLite
/// schema column so persistence is a straight copy, and feeds the
/// `chunk_id` recipe (§4.2) so policy edits invalidate downstream IDs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Chunk {
    pub chunk_id: ChunkId,
    pub doc_id: DocumentId,
    pub block_ids: Vec<BlockId>,
    pub text: String,
    pub heading_path: Vec<String>,
    pub source_spans: Vec<SourceSpan>,
    pub token_estimate: usize,
    pub chunker_version: ChunkerVersion,
    pub policy_hash: String,
    /// 한국어 형태소 분해된 token 시퀀스 (공백 join). lindera ko-dic
    /// 으로 chunker 가 pre-fill. None 시 raw text 만 FTS5 index.
    /// Bug #8 (한국어 2자 query) 해결을 위한 V009 cascade.
    #[serde(default)]
    pub tokenized_korean_text: Option<String>,
    /// 색인시 doc-side expansion (Phase 2) 으로 생성된 "검색용 별칭"
    /// (같은언어 paraphrase + 한↔영 번역, 개행 join). `[ingest.expansion]`
    /// flag off 또는 미생성이면 None — 별도 FTS5 테이블 `chunk_aliases_fts`
    /// 에만 색인되고 본문 매칭/dense 임베딩에는 영향 없음. 설계 spec
    /// `2026-05-30-doc-side-expansion-design.md` §3.3.
    #[serde(default)]
    pub aliases: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aliases_defaults_to_none_on_deserialize() {
        // aliases 필드가 없는 과거 JSON 도 파싱되어야 한다 (#[serde(default)]).
        let json = r#"{
            "chunk_id": "c1",
            "doc_id": "d1",
            "block_ids": [],
            "text": "hello",
            "heading_path": [],
            "source_spans": [],
            "token_estimate": 1,
            "chunker_version": "md-heading-v1",
            "policy_hash": "abc"
        }"#;
        let c: Chunk = serde_json::from_str(json).unwrap();
        assert_eq!(c.aliases, None);
        assert_eq!(c.tokenized_korean_text, None);
    }
}
