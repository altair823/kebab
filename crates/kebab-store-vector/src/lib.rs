//! `kb-store-vector` — LanceDB-backed [`kebab_core::VectorStore`] for kb.
//!
//! Stores per-model Lance tables under `config.storage.vector_dir/`
//! (`chunk_embeddings_<model>_<dim>.lance/`). `upsert` runs the
//! SQLite-first / Lance-second two-phase write described in design
//! §5.6: phase 1 stages `embedding_records` rows at `status='pending'`,
//! phase 2 issues a Lance `MergeInsert` keyed on `chunk_id`, phase 3
//! flips the rows to `status='committed'`. `search` joins against
//! `embedding_records WHERE status='committed'` so partial-write Lance
//! rows never surface to callers; if the process crashes between phase
//! 2 and phase 3 (or phase 2 itself fails), the next `upsert` call
//! retries the still-pending rows idempotently because Lance MergeInsert
//! dedupes on `chunk_id`.
//!
//! Sync / async bridge: `VectorStore` is a sync trait (§7.2) and
//! LanceDB's Rust API is async-only. We own a private current-thread
//! `tokio::runtime::Runtime` and `block_on` per trait method. The
//! tradeoff is documented inline; multi-thread runtime would let two
//! upserts run concurrently but kb-app's job scheduler already
//! serializes vector ops, and current-thread saves the two worker
//! threads a multi-thread runtime spawns by default.
//!
//! See `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md`
//! §5.6 (embedding_records DDL), §6.3 (lancedb table naming),
//! §7.2 (VectorStore), §9 (versioning).

mod arrow_batch;
mod paths;
mod store;

pub use store::{INDEX_VERSION_STR, LanceVectorStore};
