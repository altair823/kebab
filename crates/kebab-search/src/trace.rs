//! p9-fb-37: trace capture helpers for `HybridRetriever::search_with_trace`.

use std::collections::BTreeMap;

use kebab_core::{
    SearchHit, SearchTrace, TraceCandidate, TraceFusionInput, TraceTiming,
};

/// Build a `TraceCandidate` from a `SearchHit`. The score field reflects
/// each side's score (lexical / vector / fusion) — caller selects which
/// retriever's hit list this is.
pub fn candidates_from_hits(hits: &[SearchHit], score_kind: ScoreKind) -> Vec<TraceCandidate> {
    hits.iter()
        .map(|h| TraceCandidate {
            chunk_id: h.chunk_id.clone(),
            doc_id: h.doc_id.clone(),
            doc_path: h.doc_path.clone(),
            rank: h.rank,
            score: match score_kind {
                ScoreKind::Lexical => h.retrieval.lexical_score.unwrap_or(0.0),
                ScoreKind::Vector => h.retrieval.vector_score.unwrap_or(0.0),
            },
        })
        .collect()
}

#[derive(Clone, Copy, Debug)]
pub enum ScoreKind {
    Lexical,
    Vector,
}

/// Build the union of (chunk_id) across lex and vec hit lists, with
/// each side's rank captured. `fusion_score` is filled by the caller
/// (RRF computes it during fusion, this helper just pre-builds the
/// rank table — caller overwrites fusion_score in a second pass).
pub fn build_fusion_input_skeleton(
    lex: &[SearchHit],
    vec: &[SearchHit],
) -> Vec<TraceFusionInput> {
    let mut by_chunk: BTreeMap<String, TraceFusionInput> = BTreeMap::new();
    for h in lex {
        by_chunk
            .entry(h.chunk_id.0.clone())
            .or_insert(TraceFusionInput {
                chunk_id: h.chunk_id.clone(),
                lexical_rank: None,
                vector_rank: None,
                fusion_score: 0.0,
            })
            .lexical_rank = Some(h.rank);
    }
    for h in vec {
        by_chunk
            .entry(h.chunk_id.0.clone())
            .or_insert(TraceFusionInput {
                chunk_id: h.chunk_id.clone(),
                lexical_rank: None,
                vector_rank: None,
                fusion_score: 0.0,
            })
            .vector_rank = Some(h.rank);
    }
    by_chunk.into_values().collect()
}

/// Container the hybrid retriever fills during a traced run.
#[derive(Default)]
pub struct TraceBuilder {
    pub lexical: Vec<TraceCandidate>,
    pub vector: Vec<TraceCandidate>,
    pub rrf_inputs: Vec<TraceFusionInput>,
    pub timing: TraceTiming,
}

impl TraceBuilder {
    pub fn into_trace(self) -> SearchTrace {
        SearchTrace {
            lexical: self.lexical,
            vector: self.vector,
            rrf_inputs: self.rrf_inputs,
            timing: self.timing,
        }
    }
}
