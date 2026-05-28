//! `kb-rag` — RAG pipeline (P4-3).
//!
//! End-to-end orchestration of `retrieve → gate → pack → generate →
//! cite-validate → persist` per design §0 Q4 / §1 / §2.3 / §3.8 / §6.4.
//!
//! Allowed deps per the P4-3 task spec:
//! - `kb-core` (Answer / Retriever / LanguageModel / DocumentStore types)
//! - `kb-config` (RagCfg + LlmCfg + EmbeddingModelCfg)
//! - `kb-search` (Retriever trait object — concrete adapters injected)
//! - `kb-llm` (LanguageModel trait re-export)
//! - `kb-store-sqlite` (read chunk text via DocumentStore + write
//!   `answers` row via the new `put_answer` helper)
//! - `serde`, `serde_json`, `regex`, `time`, `tracing`, `thiserror`,
//!   `anyhow`, `blake3` (TraceId minting).
//!
//! Forbidden (per spec §Forbidden dependencies): `kb-source-fs`,
//! `kb-parse-md`, `kb-normalize`, `kb-chunk`, `kb-store-vector` (only
//! reachable via `Retriever`), `kb-embed*` (only via `Retriever`),
//! `kb-llm-local` (only via `LanguageModel`), `kb-tui`, `kb-desktop`.

pub use kebab_core::{Answer, AnswerCitation, AnswerRetrievalSummary, RefusalReason};

mod pipeline;

pub use pipeline::{
    AskOpts, MAX_NLI_HYPOTHESIS_CHARS_INITIAL, MAX_NLI_HYPOTHESIS_CHARS_MIN, MAX_NLI_PREMISE_CHARS,
    RagPipeline, StreamEvent, truncate_for_nli, truncate_hypothesis_for_nli_with_budget,
};
