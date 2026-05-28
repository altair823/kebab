//! `LanceVectorStore` — `kebab_core::VectorStore` impl over LanceDB.
//!
//! See module-level docs in `lib.rs` for the high-level shape (two-phase
//! upsert, sync/async bridge, table layout).

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use arrow_array::{Array, Float32Array, RecordBatch, StringArray};
use arrow_schema::SchemaRef;
use futures::TryStreamExt;
use kebab_core::{
    ChunkId, DocumentId, EmbeddingModelId, IndexId, SearchFilters, VectorHit, VectorRecord,
    VectorStore,
};
use kebab_store_sqlite::{EmbeddingRecordRow, SqliteStore};
use lancedb::Connection;
use lancedb::query::{ExecutableQuery, QueryBase};
use serde_json::json;
use time::OffsetDateTime;
use tokio::runtime::{Builder as RuntimeBuilder, Runtime};

use kebab_config::expand_path;

use crate::arrow_batch::{build_batch, schema_for, schema_params_hash};
use crate::paths::lance_table_name;

/// Overfetch multiplier: when post-filtering Lance results against
/// SQLite-side filters we ask for `2 * k` candidates so a moderately
/// selective filter still returns `k` hits. P3-3 spec line 138 caps
/// the doubling at this multiplier; deeper retries are out of scope.
const OVERFETCH_MULTIPLIER: usize = 2;

/// `IndexId` collection label per design §4.2.
const INDEX_COLLECTION: &str = "chunk_embeddings";

/// `IndexId` kind label — flat cosine for v1 (§7.2 + spec line 85).
const INDEX_KIND: &str = "flat";

/// `IndexVersion` token. The schema doesn't expose IndexVersion as a
/// dimension we vary per call, but `id_for_index` requires one; pin to
/// `v1` so re-runs produce stable IDs.
const INDEX_VERSION: &str = "v1";

/// Public view of [`INDEX_VERSION`] for `kebab-app::schema_with_config`.
/// The value is the same string — exposed as `pub const` so the schema
/// facade can embed it in `SchemaV1.models.index_version` without
/// reaching into a private constant.
pub const INDEX_VERSION_STR: &str = INDEX_VERSION;

/// Lance VectorStore.
///
/// Holds a single `lancedb::Connection` opened against
/// `config.storage.vector_dir/`. The connection is cheap to clone via
/// `Arc` internally and is reused across `ensure_table` / `upsert` /
/// `search`. The `tokio::Runtime` is current-thread; multi-thread
/// would buy concurrency we don't currently exploit (kb-app job
/// scheduler serializes vector ops) at the cost of two worker
/// threads.
///
/// # Async context
///
/// `LanceVectorStore` owns a private `tokio::runtime::Runtime` and
/// drives every `VectorStore` trait method through `runtime.block_on`.
/// **Do NOT construct or call any of these methods from inside another
/// tokio runtime context** — `block_on` panics with `"Cannot start a
/// runtime from within a runtime"` in that case. `kb-app`'s job
/// scheduler is synchronous so this is safe today; if a future caller
/// wants to embed `LanceVectorStore` inside an async server they must
/// wrap calls in `tokio::task::spawn_blocking` (or move to an
/// async-native `VectorStore` impl).
pub struct LanceVectorStore {
    runtime: Runtime,
    connection: Connection,
    sqlite: Arc<SqliteStore>,
    /// Resolved absolute path to the Lance root. Kept for diagnostics
    /// only — the `Connection` already knows it.
    #[allow(dead_code)]
    vector_dir: PathBuf,
}

impl LanceVectorStore {
    /// Open (or create) the Lance directory under
    /// `config.storage.vector_dir`, build a current-thread tokio
    /// runtime, and return a ready-to-use store. Migrations on the
    /// SQLite side must already have been applied (`run_migrations`)
    /// — this constructor does not touch the SQLite schema.
    ///
    /// **Caveat:** internally calls `runtime.block_on` to open the
    /// Lance connection. Calling this from inside another tokio
    /// runtime context will panic with `"Cannot start a runtime from
    /// within a runtime"`. See the struct-level `# Async context`
    /// section.
    pub fn new(config: &kebab_config::Config, sqlite: Arc<SqliteStore>) -> Result<Self> {
        let data_dir = expand_path(&config.storage.data_dir, "");
        let vector_dir = expand_path(&config.storage.vector_dir, &data_dir.to_string_lossy());
        std::fs::create_dir_all(&vector_dir)
            .with_context(|| format!("create vector_dir {}", vector_dir.display()))?;

        // current-thread runtime: see module docs. Multi-thread would
        // spawn two worker threads we don't use.
        let runtime = RuntimeBuilder::new_current_thread()
            .enable_all()
            .build()
            .context("build tokio runtime for kb-store-vector")?;

        let uri = vector_dir.to_string_lossy().into_owned();
        let connection = runtime.block_on(async {
            lancedb::connect(&uri)
                .execute()
                .await
                .context("lancedb::connect")
        })?;

        tracing::debug!(
            target: "kebab-store-vector",
            vector_dir = %vector_dir.display(),
            "opened LanceVectorStore"
        );

        Ok(Self {
            runtime,
            connection,
            sqlite,
            vector_dir,
        })
    }

    /// Open or create the Lance table with the current schema. Returns
    /// a handle the caller can use for queries.
    async fn ensure_table_async(
        connection: &Connection,
        table_name: &str,
        dim: usize,
    ) -> Result<lancedb::Table> {
        match connection.open_table(table_name).execute().await {
            Ok(t) => Ok(t),
            Err(lancedb::Error::TableNotFound { .. }) => {
                let schema = schema_for(dim);
                let table = connection
                    .create_empty_table(table_name, schema)
                    .execute()
                    .await
                    .context("create_empty_table")?;
                tracing::info!(
                    target: "kebab-store-vector",
                    table = table_name,
                    dim,
                    "created Lance table"
                );
                Ok(table)
            }
            Err(e) => Err(anyhow::Error::from(e)).context("open_table"),
        }
    }

    /// Validate that the on-disk Lance table's schema matches what
    /// `schema_for(dim)` produces. Used by `upsert` to fail fast on a
    /// dim mismatch BEFORE any phase-1 SQLite write lands.
    fn check_dim(table_schema: &SchemaRef, dim: usize) -> Result<()> {
        let field = table_schema
            .field_with_name("embedding")
            .context("table missing 'embedding' column")?;
        match field.data_type() {
            arrow_schema::DataType::FixedSizeList(_, table_dim) => {
                if (*table_dim as usize) != dim {
                    anyhow::bail!(
                        "dimension mismatch: table has dim {table_dim}, records have dim {dim}"
                    );
                }
                Ok(())
            }
            other => anyhow::bail!("embedding column has unexpected Arrow type {other:?}"),
        }
    }
}

impl VectorStore for LanceVectorStore {
    fn ensure_table(&self, model: &EmbeddingModelId, dim: usize) -> Result<IndexId> {
        let table_name = lance_table_name(&model.0, dim);
        // The trait method only needs the IndexId — we don't return the
        // Lance handle. Open (or create) the table to enforce idempotence
        // (a second call with the same params must succeed and yield
        // the same IndexId).
        self.runtime.block_on(async {
            Self::ensure_table_async(&self.connection, &table_name, dim).await
        })?;

        let params_hash = schema_params_hash(dim);
        let id = kebab_core::id_for_index(
            INDEX_COLLECTION,
            model,
            dim,
            &kebab_core::IndexVersion(INDEX_VERSION.to_string()),
            INDEX_KIND,
            &params_hash,
        );
        Ok(id)
    }

    fn upsert(&self, recs: &[VectorRecord]) -> Result<()> {
        if recs.is_empty() {
            return Ok(());
        }

        // All records in a single upsert call must share (model_id,
        // model_version, dimensions). Callers (kb-app indexer) already
        // batch by model; we enforce here so a misuse fails loudly.
        let model_id = recs[0].model_id.clone();
        let model_version = recs[0].model_version.clone();
        let dim = recs[0].dimensions;
        for r in recs {
            if r.model_id != model_id || r.model_version != model_version || r.dimensions != dim {
                anyhow::bail!(
                    "kb-store-vector::upsert called with mixed (model_id, model_version, dim) — caller must bucket per table"
                );
            }
        }

        let table_name = lance_table_name(&model_id.0, dim);

        // Open (or create) the Lance table FIRST and check its on-disk
        // dim against what the records claim. A mismatch must error
        // before any phase-1 SQLite write — spec line 94: "Dimension
        // mismatch returns Error from upsert and writes nothing."
        let table = self.runtime.block_on(async {
            Self::ensure_table_async(&self.connection, &table_name, dim).await
        })?;
        let table_schema = self
            .runtime
            .block_on(async { table.schema().await.context("read table schema") })?;
        Self::check_dim(&table_schema, dim)?;

        // Phase 1: stage embedding_records rows at status='pending'.
        let now = OffsetDateTime::now_utc();
        let pending_rows: Vec<EmbeddingRecordRow> = recs
            .iter()
            .map(|r| EmbeddingRecordRow {
                embedding_id: r.embedding_id.0.clone(),
                chunk_id: r.chunk_id.0.clone(),
                model_id: r.model_id.0.clone(),
                model_version: r.model_version.0.clone(),
                dimensions: r.dimensions,
                lance_table: table_name.clone(),
                created_at: now,
            })
            .collect();
        self.sqlite
            .put_embedding_records_pending(&pending_rows)
            .context("phase 1: stage pending embedding_records")?;

        // Phase 2: Lance MergeInsert keyed on chunk_id.
        let batch = build_batch(recs, dim, now)?;
        merge_insert_batch(&self.runtime, &table, batch).context("phase 2: Lance MergeInsert")?;

        // Phase 3: flip rows to status='committed'. If we crashed
        // between phase 2 and phase 3, the rows stay 'pending' and a
        // future upsert call retries them (Lance MergeInsert dedupes
        // on chunk_id, so the retry is a no-op on the Lance side).
        let embedding_ids: Vec<String> = recs.iter().map(|r| r.embedding_id.0.clone()).collect();
        self.sqlite
            .mark_embedding_records_committed(&embedding_ids)
            .context("phase 3: mark embedding_records committed")?;

        tracing::info!(
            target: "kebab-store-vector",
            table = %table_name,
            rows = recs.len(),
            "upsert committed"
        );
        Ok(())
    }

    /// Delete every Lance row whose `chunk_id` matches one of the
    /// supplied IDs. Iterates *all* `chunk_embeddings_*` tables in the
    /// connection — a single chunk_id only ever lives in one table
    /// (one-model-per-workspace today, see `INDEX_VERSION` in
    /// `paths.rs`), but the loop keeps the helper correct should the
    /// workspace ever maintain multiple tables (e.g. mid-migration
    /// between embedding models).
    ///
    /// Wired in by `kebab-app::ingest_one_*_asset` after the SQLite
    /// side has been swept by `purge_orphan_at_workspace_path` —
    /// closes the "vector store orphan" caveat from HOTFIXES
    /// 2026-05-02 P7-3.
    fn delete_by_chunk_ids(&self, chunk_ids: &[kebab_core::ChunkId]) -> Result<()> {
        if chunk_ids.is_empty() {
            return Ok(());
        }
        // SQL IN() list. chunk_ids are 32-hex-char blake3 prefixes
        // (validated upstream), so SQL injection is structurally
        // impossible — we still quote to keep the predicate
        // syntactically valid. We chunk into batches of 200 to keep the
        // WHERE clause within typical SQL parser limits.
        const BATCH: usize = 200;
        self.runtime.block_on(async {
            let names = self
                .connection
                .table_names()
                .execute()
                .await
                .context("table_names")?;
            for name in names {
                if !name.starts_with("chunk_embeddings_") {
                    continue;
                }
                let table = match self.connection.open_table(&name).execute().await {
                    Ok(t) => t,
                    Err(e) => {
                        tracing::warn!(
                            target: "kebab-store-vector",
                            table = %name,
                            error = %e,
                            "delete_by_chunk_ids: skipped unopenable table"
                        );
                        continue;
                    }
                };
                for batch in chunk_ids.chunks(BATCH) {
                    // chunk_ids in production come from `id_for_chunk`
                    // which always emits 32 ASCII hex chars. The
                    // `ChunkId(pub String)` newtype permits hand-
                    // construction that bypasses that invariant; assert
                    // it here so a misuse fails loudly in dev rather
                    // than slipping a tainted string into Lance's SQL
                    // parser.
                    debug_assert!(
                        batch
                            .iter()
                            .all(|id| id.0.bytes().all(|b| b.is_ascii_hexdigit())),
                        "ChunkId must be ASCII hex (id_for_chunk invariant) — \
                         hand-constructed IDs that bypass this would let \
                         Lance's SQL parser see arbitrary text"
                    );
                    let list = batch
                        .iter()
                        .map(|id| format!("'{}'", id.0))
                        .collect::<Vec<_>>()
                        .join(",");
                    let predicate = format!("chunk_id IN ({list})");
                    table
                        .delete(&predicate)
                        .await
                        .with_context(|| format!("Lance delete on {name} ({} ids)", batch.len()))?;
                }
            }
            anyhow::Ok(())
        })?;
        tracing::debug!(
            target: "kebab-store-vector",
            count = chunk_ids.len(),
            "deleted vector rows by chunk_id"
        );
        Ok(())
    }

    fn search(
        &self,
        query_vec: &[f32],
        k: usize,
        filters: &SearchFilters,
    ) -> Result<Vec<VectorHit>> {
        if k == 0 {
            return Ok(Vec::new());
        }

        // We need to know which table to query. SearchFilters doesn't
        // carry a model_id (the trait doesn't expose one to the
        // caller), so we scan known tables on disk and pick the one
        // matching `query_vec.len()`. In v1 there's typically one
        // model in play; if there are several we pick the first match.
        let dim = query_vec.len();
        let table_name = if let Some(name) = self
            .runtime
            .block_on(async { find_matching_table(&self.connection, dim).await })?
        {
            name
        } else {
            tracing::debug!(
                target: "kebab-store-vector",
                dim,
                "search: no Lance table matches query dim — returning empty"
            );
            return Ok(Vec::new());
        };

        // Pre-fetch 2*k Lance rows; we'll filter against SQLite
        // afterwards. If filters are empty we still over-fetch to
        // exclude tombstoned / pending rows.
        let overfetch = k.saturating_mul(OVERFETCH_MULTIPLIER).max(k);
        let raw_hits = self.runtime.block_on(async {
            let table = match self.connection.open_table(&table_name).execute().await {
                Ok(t) => t,
                Err(lancedb::Error::TableNotFound { .. }) => return Ok(Vec::new()),
                Err(e) => return Err(anyhow::Error::from(e)),
            };

            let stream = table
                .vector_search(query_vec)
                .context("vector_search")?
                .distance_type(lancedb::DistanceType::Cosine)
                .limit(overfetch)
                .execute()
                .await
                .context("execute vector query")?;
            let batches: Vec<RecordBatch> =
                stream.try_collect().await.context("collect batches")?;
            Result::<Vec<RecordBatch>>::Ok(batches)
        })?;

        let candidates = decode_lance_hits(&raw_hits)?;

        // Filter against embedding_records (status='committed') and
        // documents (tags / lang / path / trust). For the empty filter
        // case the join still excludes tombstoned / pending rows.
        // The `filter_chunks` helper lives in kb-store-sqlite (the
        // crate that owns the schema), so this crate doesn't need its
        // own rusqlite / globset direct deps.
        let candidate_ids: Vec<ChunkId> = {
            // Deduplicate — Lance result batches can in principle
            // repeat a chunk_id across batches; the JOIN is most
            // efficient if we ask once per id.
            let mut seen = HashSet::new();
            candidates
                .iter()
                .filter(|c| seen.insert(c.chunk_id.0.clone()))
                .map(|c| c.chunk_id.clone())
                .collect()
        };
        let allowed_set: HashSet<String> = self
            .sqlite
            .filter_chunks(&candidate_ids, filters)
            .context("post-filter chunks via kb-store-sqlite")?
            .into_iter()
            .map(|c| c.0)
            .collect();

        let mut hits: Vec<VectorHit> = candidates
            .into_iter()
            .filter(|c| allowed_set.contains(&c.chunk_id.0))
            .take(k)
            .map(LanceCandidate::into_hit)
            .collect();
        // Re-rank by score desc to give callers a consistent ordering
        // regardless of post-filter shuffling.
        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(hits)
    }
}

/// One Lance row decoded from a query batch, paired with the converted
/// score and pre-built JSON payload. We keep `chunk_id` separately so
/// the SQLite filter pass can JOIN against it without re-parsing the
/// payload.
struct LanceCandidate {
    chunk_id: ChunkId,
    doc_id: DocumentId,
    text: String,
    heading_path: Vec<String>,
    score: f32,
}

impl LanceCandidate {
    fn into_hit(self) -> VectorHit {
        let payload = json!({
            "doc_id": self.doc_id.0,
            "text": self.text,
            "heading_path": self.heading_path,
        });
        VectorHit {
            chunk_id: self.chunk_id,
            score: self.score,
            payload,
        }
    }
}

/// Decode a list of Lance result batches into typed candidates.
/// Lance's vector query attaches a `_distance: Float32` column; we
/// convert to similarity via `1 - distance` then shift to `[0, 1]`
/// via `(sim + 1) / 2` per spec line 96. NaN distances get score 0
/// (with a warn log).
fn decode_lance_hits(batches: &[RecordBatch]) -> Result<Vec<LanceCandidate>> {
    let mut out = Vec::new();
    for batch in batches {
        let chunk_ids = batch
            .column_by_name("chunk_id")
            .context("missing chunk_id col")?
            .as_any()
            .downcast_ref::<StringArray>()
            .context("chunk_id wrong type")?;
        let doc_ids = batch
            .column_by_name("doc_id")
            .context("missing doc_id col")?
            .as_any()
            .downcast_ref::<StringArray>()
            .context("doc_id wrong type")?;
        let texts = batch
            .column_by_name("text")
            .context("missing text col")?
            .as_any()
            .downcast_ref::<StringArray>()
            .context("text wrong type")?;
        let heading_path_str = batch
            .column_by_name("heading_path")
            .context("missing heading_path col")?
            .as_any()
            .downcast_ref::<StringArray>()
            .context("heading_path wrong type")?;
        let distances = batch
            .column_by_name("_distance")
            .context("missing _distance col")?
            .as_any()
            .downcast_ref::<Float32Array>()
            .context("_distance wrong type")?;

        for i in 0..batch.num_rows() {
            let dist = distances.value(i);
            let score = score_from_distance(dist);
            let heading_path: Vec<String> =
                serde_json::from_str(heading_path_str.value(i)).unwrap_or_default();
            out.push(LanceCandidate {
                chunk_id: ChunkId(chunk_ids.value(i).to_string()),
                doc_id: DocumentId(doc_ids.value(i).to_string()),
                text: texts.value(i).to_string(),
                heading_path,
                score,
            });
        }
    }
    Ok(out)
}

/// Convert a cosine distance (LanceDB returns `1 - cosine_similarity`
/// in `[0, 2]` for L2-normalized vectors) to a `[0, 1]` score via
/// `score = ((1 - distance) + 1) / 2`. Per spec line 96 the shift
/// (rather than clamp) preserves ordering between unrelated and
/// opposite vectors. NaN — which Lance can produce when one side is
/// the all-zero vector — collapses to 0 with a warn.
fn score_from_distance(distance: f32) -> f32 {
    if distance.is_nan() {
        tracing::warn!(
            target: "kebab-store-vector",
            "NaN cosine distance from Lance — coercing to score 0"
        );
        return 0.0;
    }
    let sim = 1.0 - distance;
    f32::midpoint(sim, 1.0)
}

/// Find a Lance table whose embedding column is FixedSizeList<Float32, dim>.
async fn find_matching_table(connection: &Connection, dim: usize) -> Result<Option<String>> {
    let names = connection
        .table_names()
        .execute()
        .await
        .context("table_names")?;
    for name in names {
        if !name.starts_with("chunk_embeddings_") {
            continue;
        }
        match connection.open_table(&name).execute().await {
            Ok(t) => {
                let schema = t.schema().await.context("schema for table")?;
                if let Ok(field) = schema.field_with_name("embedding") {
                    if let arrow_schema::DataType::FixedSizeList(_, table_dim) = field.data_type() {
                        if (*table_dim as usize) == dim {
                            return Ok(Some(name));
                        }
                    }
                }
            }
            Err(e) => {
                tracing::warn!(
                    target: "kebab-store-vector",
                    table = %name,
                    error = %e,
                    "search: skipped unopenable table"
                );
            }
        }
    }
    Ok(None)
}

/// Run the Lance MergeInsert under our embedded runtime. Pulled out
/// of `upsert` so the trait method stays compact.
fn merge_insert_batch(runtime: &Runtime, table: &lancedb::Table, batch: RecordBatch) -> Result<()> {
    let schema = batch.schema();
    runtime.block_on(async move {
        let reader = arrow_array::RecordBatchIterator::new(vec![Ok(batch)].into_iter(), schema);
        let mut builder = table.merge_insert(&["chunk_id"]);
        builder
            .when_matched_update_all(None)
            .when_not_matched_insert_all();
        builder
            .execute(Box::new(reader))
            .await
            .context("MergeInsert execute")?;
        Result::<()>::Ok(())
    })
}
