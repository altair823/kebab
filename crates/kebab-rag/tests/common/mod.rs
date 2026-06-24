//! Shared scaffolding for kb-rag tests.
//!
//! Provides:
//! - [`RagEnv`] — a tempdir-backed `SqliteStore` with helpers to seed
//!   asset/document/chunk rows directly via SQL (so the test crate's
//!   deps stay inside the allowed list).
//! - [`MockRetriever`] — returns canned `Vec<SearchHit>` regardless of
//!   the query, so the pipeline exercise is independent of any real
//!   indexer.
//! - small helpers to build `Citation` / `SearchHit` / canned LM
//!   responses without rewriting boilerplate in every test.

#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use kebab_config::Config;
use kebab_core::{
    ChunkId, ChunkerVersion, Citation, DocumentId, IndexVersion, RetrievalDetail, Retriever,
    SearchHit, SearchMode, SearchQuery, WorkspacePath,
};
use kebab_nli::{NliScores, NliVerifier};
use kebab_store_sqlite::SqliteStore;
use rusqlite::params;
use tempfile::TempDir;

/// Tempdir-backed test environment. Holds an open `SqliteStore` with
/// V001 + V002 + V003 migrations applied so chunk reads work end-to-end.
pub struct RagEnv {
    pub temp: TempDir,
    pub config: Config,
    pub sqlite: Arc<SqliteStore>,
}

impl RagEnv {
    pub fn new() -> Self {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut config = Config::defaults();
        config.storage.data_dir = temp.path().to_string_lossy().into_owned();
        let sqlite = SqliteStore::open(&config.storage).unwrap();
        sqlite.run_migrations().unwrap();
        Self {
            temp,
            config,
            sqlite: Arc::new(sqlite),
        }
    }

    /// Seed the minimal (assets, documents, chunks) row triple needed
    /// for `DocumentStore::get_chunk` to round-trip in tests.
    /// `chunk_id` / `doc_id` must already be 32-hex-char shaped (use
    /// [`id32`] to pad short prefixes).
    pub fn seed_chunk(
        &self,
        chunk_id: &str,
        doc_id: &str,
        workspace_path: &str,
        text: &str,
        heading_path: &[&str],
    ) {
        let asset_id = format!("a{}", &doc_id[..31]);
        let conn = self.sqlite.read_conn();
        conn.execute(
            "INSERT OR IGNORE INTO assets (
                asset_id, source_uri, workspace_path, media_type, byte_len,
                checksum, storage_kind, storage_path, discovered_at
             ) VALUES (?, ?, ?, '\"markdown\"', 0,
                       'deadbeefdeadbeefdeadbeefdeadbeef',
                       'reference', ?, '1970-01-01T00:00:00Z')",
            params![
                asset_id,
                format!("file://{workspace_path}"),
                workspace_path,
                workspace_path,
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO documents (
                doc_id, asset_id, workspace_path, title, lang, source_type,
                trust_level, parser_version, doc_version, schema_version,
                metadata_json, provenance_json, created_at, updated_at
             ) VALUES (?, ?, ?, NULL, 'en', 'markdown', 'primary', 'v1', 1, 1,
                       '{}', '{}', '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z')",
            params![doc_id, asset_id, workspace_path],
        )
        .unwrap();
        let heading_json = serde_json::to_string(heading_path).unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO chunks (
                chunk_id, doc_id, text, heading_path_json, section_label,
                source_spans_json, token_estimate, chunker_version,
                policy_hash, block_ids_json, created_at
             ) VALUES (?, ?, ?, ?, NULL,
                       '[{\"kind\":\"line\",\"start\":1,\"end\":3}]',
                       1, 'v1', 'h', '[]', '1970-01-01T00:00:00Z')",
            params![chunk_id, doc_id, text, heading_json],
        )
        .unwrap();
    }

    /// Count rows in `answers`. Tests use this to assert that every
    /// `ask` (incl. refusals) writes exactly one row.
    pub fn count_answers(&self) -> i64 {
        let conn = self.sqlite.read_conn();
        conn.query_row("SELECT COUNT(*) FROM answers", [], |r| r.get(0))
            .unwrap()
    }
}

/// Build a `SearchHit` with canned scores. Citation defaults to a
/// `Line { 1..=3 }` over `workspace_path`.
pub fn mk_hit(
    rank: u32,
    chunk_id: &str,
    doc_id: &str,
    workspace_path: &str,
    fusion_score: f32,
    heading: &[&str],
) -> SearchHit {
    mk_hit_with_indexed_at(
        rank,
        chunk_id,
        doc_id,
        workspace_path,
        fusion_score,
        heading,
        time::OffsetDateTime::UNIX_EPOCH,
    )
}

/// Build a `SearchHit` with an explicit `indexed_at` timestamp. Used by
/// p9-fb-32 staleness tests so the pipeline sees realistic per-hit
/// indexed_at values flowing through to `AnswerCitation`.
pub fn mk_hit_with_indexed_at(
    rank: u32,
    chunk_id: &str,
    doc_id: &str,
    workspace_path: &str,
    fusion_score: f32,
    heading: &[&str],
    indexed_at: time::OffsetDateTime,
) -> SearchHit {
    let p = WorkspacePath::new(workspace_path.to_string()).expect("workspace path valid");
    SearchHit {
        rank,
        chunk_id: ChunkId(chunk_id.to_string()),
        doc_id: DocumentId(doc_id.to_string()),
        doc_path: p.clone(),
        heading_path: heading
            .iter()
            .map(std::string::ToString::to_string)
            .collect(),
        section_label: None,
        snippet: "snippet".to_string(),
        citation: Citation::Line {
            path: p,
            start: 1,
            end: 3,
            section: None,
        },
        retrieval: RetrievalDetail {
            method: SearchMode::Lexical,
            fusion_score,
            lexical_score: Some(fusion_score),
            vector_score: None,
            lexical_rank: Some(rank),
            vector_rank: None,
        },
        index_version: IndexVersion("test-iv".to_string()),
        embedding_model: None,
        chunker_version: ChunkerVersion("v1".to_string()),
        // p9-fb-32: pipeline post-processes `stale` from `indexed_at`
        // + cfg threshold; tests configure both via this helper.
        indexed_at,
        stale: false,
        score_kind: kebab_core::ScoreKind::Rrf,
        repo: None,
        code_lang: None,
        source_id: None,
        trust_level: None,
    }
}

/// Mock retriever that returns a fixed `Vec<SearchHit>` regardless of
/// the query / k / filters. Captures the invocation count for assertions.
pub struct MockRetriever {
    pub hits: Vec<SearchHit>,
    pub calls: std::sync::atomic::AtomicUsize,
}

impl MockRetriever {
    pub fn new(hits: Vec<SearchHit>) -> Self {
        Self {
            hits,
            calls: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    pub fn calls(&self) -> usize {
        self.calls.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl Retriever for MockRetriever {
    fn search(&self, _q: &SearchQuery) -> anyhow::Result<Vec<SearchHit>> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(self.hits.clone())
    }
    fn index_version(&self) -> IndexVersion {
        IndexVersion("test-iv".to_string())
    }
}

/// p9-fb-41 PR-3b-ii: scripted retriever. Returns a different
/// `Vec<SearchHit>` per `search` call from a pre-supplied sequence,
/// so a multi-hop test can simulate "iter 1 returns chunk A, iter 2
/// returns chunks A+B" (pool dedup) or "different sub-queries hit
/// different docs". Exhaustion returns an empty `Vec` (no panic) —
/// the pipeline already handles "no hits this round" gracefully via
/// the dedup loop, and a panic would conflate "test forgot a row"
/// with "pipeline made an unexpected extra retrieval call".
///
/// Use [`ScriptedRetriever::calls`] to assert the expected number
/// of retrievals occurred.
pub struct ScriptedRetriever {
    hits_per_call: Vec<Vec<SearchHit>>,
    next: std::sync::atomic::AtomicUsize,
}

impl ScriptedRetriever {
    pub fn new(hits_per_call: Vec<Vec<SearchHit>>) -> Self {
        Self {
            hits_per_call,
            next: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    pub fn calls(&self) -> usize {
        self.next.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl Retriever for ScriptedRetriever {
    fn search(&self, _q: &SearchQuery) -> anyhow::Result<Vec<SearchHit>> {
        let idx = self.next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(self.hits_per_call.get(idx).cloned().unwrap_or_default())
    }
    fn index_version(&self) -> IndexVersion {
        IndexVersion("test-iv".to_string())
    }
}

/// Pad a short prefix to the 32-hex shape `kebab_core` newtypes expect.
pub fn id32(prefix: &str) -> String {
    let mut s = prefix.to_string();
    while s.len() < 32 {
        s.push('0');
    }
    s.truncate(32);
    s
}

/// p9-fb-41 PR-3b-ii: scripted language model. Returns a different
/// canned response per `generate_stream` call from a pre-supplied
/// `Vec<String>`. Mirrors `MockLanguageModel`'s streaming contract
/// (one `TokenChunk::Token` per Unicode scalar, terminal `Done` with
/// `canned_usage`, stop-string truncation honoured) but lets a test
/// distinguish the decompose / per-iter decide / synthesize LLM calls
/// of `RagPipeline::ask_multi_hop` — each can return a different
/// payload (`["q1","q2"]`, `[]`, `"final answer [#1]"`, etc.).
///
/// Internally `Vec<String>` (immutable after construction) plus an
/// `AtomicUsize` index counter, so the type is `Send + Sync` and
/// tests wrap it in `Arc::new(ScriptedLm::new(...))` to share with
/// the pipeline. Tests can read `calls()` for an assertion on the
/// expected LLM call count.
///
/// Exhaustion (more calls than scripted responses) panics — tests
/// that need an "infinite" final response can supply a longer
/// script; the panic message names the call index so the test
/// failure points at the missing entry.
pub struct ScriptedLm {
    model_id: String,
    provider: String,
    context_tokens: usize,
    /// Canned responses in call order. Index `i` is returned on the
    /// `i`-th `generate_stream` call (0-based).
    responses: Vec<String>,
    /// 0-based index of the next response to return on `generate_stream`.
    next: std::sync::atomic::AtomicUsize,
    canned_finish: kebab_core::FinishReason,
    canned_usage: kebab_core::TokenUsage,
}

impl ScriptedLm {
    /// Build a scripted LM with the default model_id/provider used by
    /// the rest of the test suite (`mock-lm` / `mock`) and the
    /// MockLanguageModel-equivalent canned usage. No knobs are
    /// exposed today — every multi-hop test exercises the pipeline
    /// flow, not the LM identity. Add builders only when a test
    /// genuinely needs to override defaults.
    pub fn new(responses: Vec<&str>) -> Self {
        Self {
            model_id: "mock-lm".to_string(),
            provider: "mock".to_string(),
            context_tokens: 32_768,
            responses: responses.into_iter().map(str::to_string).collect(),
            next: std::sync::atomic::AtomicUsize::new(0),
            canned_finish: kebab_core::FinishReason::Stop,
            canned_usage: kebab_core::TokenUsage {
                prompt_tokens: 10,
                completion_tokens: 5,
                latency_ms: 7,
            },
        }
    }

    /// Total `generate_stream` invocations so far. Tests use this to
    /// assert "exactly N LLM calls happened" without scanning the
    /// HopRecord trace (the trace is the user-visible signal; this
    /// is the lower-level call counter).
    pub fn calls(&self) -> usize {
        self.next.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Earliest byte position of any non-empty stop string in
    /// `canned`. Same precedence rule as `MockLanguageModel`:
    /// `Iterator::min` returns the first equal element, so ties
    /// break by `stop` declaration order. `str::find` returns a
    /// UTF-8 char boundary by contract, so the resulting prefix
    /// slice is sound.
    fn apply_stop<'a>(canned: &'a str, stop: &[String]) -> (&'a str, bool) {
        let earliest = stop
            .iter()
            .filter(|s| !s.is_empty())
            .filter_map(|s| canned.find(s.as_str()))
            .min();
        match earliest {
            Some(idx) => (&canned[..idx], true),
            None => (canned, false),
        }
    }
}

impl kebab_core::LanguageModel for ScriptedLm {
    fn model_ref(&self) -> kebab_core::ModelRef {
        kebab_core::ModelRef {
            id: self.model_id.clone(),
            provider: self.provider.clone(),
            dimensions: None,
        }
    }

    fn context_tokens(&self) -> usize {
        self.context_tokens
    }

    fn generate_stream(
        &self,
        req: kebab_core::GenerateRequest,
    ) -> anyhow::Result<Box<dyn Iterator<Item = anyhow::Result<kebab_core::TokenChunk>> + Send>>
    {
        let idx = self.next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let canned = self.responses.get(idx).unwrap_or_else(|| {
            panic!(
                "ScriptedLm exhausted: call #{idx} requested but only {} responses scripted",
                self.responses.len()
            )
        });
        let (truncated, stop_hit) = Self::apply_stop(canned, &req.stop);
        let mut chunks: Vec<kebab_core::TokenChunk> = truncated
            .chars()
            .map(|c| kebab_core::TokenChunk::Token(c.to_string()))
            .collect();
        let finish_reason = if stop_hit {
            kebab_core::FinishReason::Stop
        } else {
            self.canned_finish.clone()
        };
        chunks.push(kebab_core::TokenChunk::Done {
            finish_reason,
            usage: self.canned_usage.clone(),
        });
        Ok(Box::new(chunks.into_iter().map(Ok)))
    }
}

/// p9-fb-41 PR-9c-2: mock NLI verifier for multi-hop step 8.5 tests.
/// Three constructors mirror the test scenarios:
/// - [`MockNliVerifier::pass`] — high entailment score (0.9), `nli_passed`
///   is true at the production default threshold (0.5).
/// - [`MockNliVerifier::fail`] — low entailment (0.1), refuses at any
///   threshold > 0.1.
/// - [`MockNliVerifier::err`] — returns an `anyhow::Error` so the pipeline
///   surfaces `RefusalReason::NliModelUnavailable`.
///
/// `call_count` instrumented (Mutex-wrapped usize) so a test can assert
/// the verifier ran the expected number of times — useful for pinning
/// the "threshold = 0 skips verify" invariant when the verifier is
/// nonetheless attached to the pipeline.
pub struct MockNliVerifier {
    pub mode: MockMode,
    pub call_count: Mutex<usize>,
}

pub enum MockMode {
    /// Return these scores. Used by pass / fail variants.
    Scores(NliScores),
    /// Return an `anyhow::Error`. Used by the err variant.
    Err(String),
}

impl MockNliVerifier {
    pub fn pass() -> Arc<Self> {
        Arc::new(Self {
            mode: MockMode::Scores(NliScores {
                entailment: 0.9,
                neutral: 0.07,
                contradiction: 0.03,
            }),
            call_count: Mutex::new(0),
        })
    }

    pub fn fail() -> Arc<Self> {
        Arc::new(Self {
            mode: MockMode::Scores(NliScores {
                entailment: 0.1,
                neutral: 0.4,
                contradiction: 0.5,
            }),
            call_count: Mutex::new(0),
        })
    }

    pub fn err() -> Arc<Self> {
        Arc::new(Self {
            mode: MockMode::Err("mock NLI unavailable".into()),
            call_count: Mutex::new(0),
        })
    }

    pub fn calls(&self) -> usize {
        *self.call_count.lock().unwrap()
    }
}

impl NliVerifier for MockNliVerifier {
    fn score(&self, _premise: &str, _hypothesis: &str) -> anyhow::Result<NliScores> {
        *self.call_count.lock().unwrap() += 1;
        match &self.mode {
            MockMode::Scores(s) => Ok(*s),
            MockMode::Err(e) => anyhow::bail!("{e}"),
        }
    }
}

/// S3 follow-up (2026-05-26): closure type aliases for `SpyNliVerifier`
/// fields — clippy `type_complexity` 회피.
#[allow(clippy::type_complexity)]
pub type ScoreFn = Arc<dyn Fn(&str, &str) -> anyhow::Result<NliScores> + Send + Sync>;
#[allow(clippy::type_complexity)]
pub type HypothesisTokenCountFn = Arc<dyn Fn(&str) -> anyhow::Result<usize> + Send + Sync>;

/// S3 follow-up (2026-05-26): closure-based NLI verifier — caller 가
/// `(premise, hypothesis) -> Result<NliScores>` + `(hypothesis) ->
/// Result<usize>` 두 closures 정의 가능 + spy 로 입력 capture. 기존
/// `MockNliVerifier` (고정 mode) 와 sibling. truncate_hypothesis_for_nli_with_budget
/// retry loop 의 token-count 시뮬레이션 + final score 동작을 한 verifier
/// 안에서 inject 하기 위함.
pub struct SpyNliVerifier {
    pub score_fn: ScoreFn,
    pub hypothesis_token_count_fn: HypothesisTokenCountFn,
    pub received_premises: Mutex<Vec<String>>,
    pub received_hypotheses: Mutex<Vec<String>>,
}

impl SpyNliVerifier {
    /// 2-arg constructor — score + hypothesis_token_count 둘 다 closure
    /// 로 주입. `Arc<T>` field mutation 불가 회피를 위해 한 번에 전달.
    pub fn new<F, G>(score_fn: F, token_count_fn: G) -> Arc<Self>
    where
        F: Fn(&str, &str) -> anyhow::Result<NliScores> + Send + Sync + 'static,
        G: Fn(&str) -> anyhow::Result<usize> + Send + Sync + 'static,
    {
        Arc::new(Self {
            score_fn: Arc::new(score_fn),
            hypothesis_token_count_fn: Arc::new(token_count_fn),
            received_premises: Mutex::new(Vec::new()),
            received_hypotheses: Mutex::new(Vec::new()),
        })
    }
}

impl NliVerifier for SpyNliVerifier {
    fn score(&self, premise: &str, hypothesis: &str) -> anyhow::Result<NliScores> {
        self.received_premises
            .lock()
            .unwrap()
            .push(premise.to_string());
        self.received_hypotheses
            .lock()
            .unwrap()
            .push(hypothesis.to_string());
        (self.score_fn)(premise, hypothesis)
    }

    fn hypothesis_token_count(&self, hypothesis: &str) -> anyhow::Result<usize> {
        (self.hypothesis_token_count_fn)(hypothesis)
    }
}
