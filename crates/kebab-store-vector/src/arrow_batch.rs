//! Arrow schema + RecordBatch builder for the per-model Lance table.
//!
//! Per design §6.3 the per-row layout is:
//!
//! ```text
//! chunk_id          : Utf8 (primary)
//! doc_id            : Utf8
//! embedding         : FixedSizeList<Float32, dim>
//! model_id          : Utf8
//! embedding_version : Utf8
//! text              : Utf8
//! heading_path      : Utf8 (JSON-encoded Vec<String>)
//! created_at        : Timestamp(Microsecond, UTC)
//! ```
//!
//! `heading_path` is encoded as a JSON string rather than a Lance
//! `List<Utf8>` to keep the `only_if` SQL filter surface clean — Lance
//! exposes scalar columns to its query DSL trivially, but list columns
//! need `array_contains`-style helpers that aren't required by the
//! current `SearchFilters` shape.

use std::sync::Arc;

use anyhow::{Context, Result};
use arrow_array::{
    ArrayRef, FixedSizeListArray, Float32Array, RecordBatch, StringArray,
    TimestampMicrosecondArray,
};
use arrow_schema::{DataType, Field, Schema, SchemaRef, TimeUnit};
use kebab_core::VectorRecord;
use time::OffsetDateTime;

/// Arrow schema for a Lance table whose vector column is FixedSizeList
/// of `dim` Float32. All non-vector columns are non-nullable; the
/// vector column itself is non-nullable but the inner Float32 slot is
/// nullable per Arrow convention (Lance ignores the inner-nullable
/// flag when the outer field is non-null).
pub(crate) fn schema_for(dim: usize) -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("chunk_id", DataType::Utf8, false),
        Field::new("doc_id", DataType::Utf8, false),
        Field::new(
            "embedding",
            DataType::FixedSizeList(
                Arc::new(Field::new("item", DataType::Float32, true)),
                dim as i32,
            ),
            false,
        ),
        Field::new("model_id", DataType::Utf8, false),
        Field::new("embedding_version", DataType::Utf8, false),
        Field::new("text", DataType::Utf8, false),
        Field::new("heading_path", DataType::Utf8, false),
        Field::new(
            "created_at",
            DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
            false,
        ),
    ]))
}

/// Build a `RecordBatch` from `recs`. All records must share `dim`;
/// callers are expected to pre-bucket per-table batches before reaching
/// here. The batch carries `recs.len()` rows; `now` is folded into
/// `created_at` for every row to match design §6.3.
pub(crate) fn build_batch(
    recs: &[VectorRecord],
    dim: usize,
    now: OffsetDateTime,
) -> Result<RecordBatch> {
    let schema = schema_for(dim);

    let chunk_ids = StringArray::from(
        recs.iter().map(|r| r.chunk_id.0.as_str()).collect::<Vec<_>>(),
    );
    let doc_ids = StringArray::from(
        recs.iter().map(|r| r.doc_id.0.as_str()).collect::<Vec<_>>(),
    );
    let model_ids = StringArray::from(
        recs.iter().map(|r| r.model_id.0.as_str()).collect::<Vec<_>>(),
    );
    let model_versions = StringArray::from(
        recs.iter()
            .map(|r| r.model_version.0.as_str())
            .collect::<Vec<_>>(),
    );
    let texts =
        StringArray::from(recs.iter().map(|r| r.text.as_str()).collect::<Vec<_>>());

    // heading_path: serde_json::Value::Array of strings, then to_string.
    let heading_paths: Vec<String> = recs
        .iter()
        .map(|r| serde_json::to_string(&r.heading_path))
        .collect::<std::result::Result<_, _>>()
        .context("serialize heading_path JSON")?;
    let heading_path_arr = StringArray::from(
        heading_paths.iter().map(String::as_str).collect::<Vec<_>>(),
    );

    // Embedding: FixedSizeList<Float32, dim>. Build from the flat
    // contiguous f32 buffer.
    let mut flat: Vec<f32> = Vec::with_capacity(recs.len() * dim);
    for r in recs {
        if r.vector.len() != dim {
            anyhow::bail!(
                "vector length {} does not match table dim {} for chunk {}",
                r.vector.len(),
                dim,
                r.chunk_id.0
            );
        }
        flat.extend_from_slice(&r.vector);
    }
    let values = Float32Array::from(flat);
    let embedding_field =
        Arc::new(Field::new("item", DataType::Float32, true));
    let embedding = FixedSizeListArray::try_new(
        embedding_field,
        dim as i32,
        Arc::new(values),
        None,
    )
    .context("build FixedSizeList embedding column")?;

    // created_at: microseconds since Unix epoch, UTC.
    let micros: Vec<i64> = std::iter::repeat_n(
        (now.unix_timestamp_nanos() / 1_000) as i64,
        recs.len(),
    )
    .collect();
    let created_at = TimestampMicrosecondArray::from(micros).with_timezone("UTC");

    let arrays: Vec<ArrayRef> = vec![
        Arc::new(chunk_ids) as ArrayRef,
        Arc::new(doc_ids),
        Arc::new(embedding),
        Arc::new(model_ids),
        Arc::new(model_versions),
        Arc::new(texts),
        Arc::new(heading_path_arr),
        Arc::new(created_at),
    ];

    RecordBatch::try_new(schema, arrays).context("assemble RecordBatch")
}

/// blake3-hex of the canonical JSON of the schema. Used as
/// `params_hash` for `id_for_index` so the `IndexId` stays stable
/// across invocations with the same `dim`.
pub(crate) fn schema_params_hash(dim: usize) -> String {
    // Keep the hash input shape self-describing so a future schema
    // tweak (extra column, type change, …) bumps the hash and produces
    // a different `IndexId` automatically.
    let descriptor = serde_json::json!({
        "version": 1,
        "dim": dim,
        "columns": [
            {"name": "chunk_id", "type": "Utf8"},
            {"name": "doc_id", "type": "Utf8"},
            {"name": "embedding", "type": "FixedSizeList<Float32>", "size": dim},
            {"name": "model_id", "type": "Utf8"},
            {"name": "embedding_version", "type": "Utf8"},
            {"name": "text", "type": "Utf8"},
            {"name": "heading_path", "type": "Utf8"},
            {"name": "created_at", "type": "Timestamp<us, UTC>"},
        ],
    });
    let bytes = descriptor_bytes(&descriptor);
    blake3::hash(&bytes).to_hex().to_string()
}

/// Serialize the schema descriptor to bytes for hashing. Plain
/// `serde_json::to_vec` rather than a canonical-JSON crate is fine
/// here because the descriptor is built from a fixed `serde_json::json!`
/// literal in `schema_params_hash` — `serde_json` walks the object's
/// key order deterministically (insertion order, since `Value::Object`
/// uses `Map`), so the byte output is stable across runs without a
/// canonicalizer. The empty-vec fallback on the (unreachable, given
/// our literal input) error path keeps the function infallible.
fn descriptor_bytes(v: &serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(v).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use kebab_core::{ChunkId, DocumentId, EmbeddingId, EmbeddingModelId, EmbeddingVersion};
    use time::OffsetDateTime;

    fn make_rec(chunk_idx: u8, dim: usize) -> VectorRecord {
        VectorRecord {
            chunk_id: ChunkId(format!("{chunk_idx:032x}")),
            embedding_id: EmbeddingId(format!("{:032x}", 0xeeeeu16 + u16::from(chunk_idx))),
            vector: vec![0.1_f32; dim],
            doc_id: DocumentId("aaaa".repeat(8)),
            text: format!("text-{chunk_idx}"),
            heading_path: vec!["A".to_string(), "B".to_string()],
            model_id: EmbeddingModelId("test".to_string()),
            model_version: EmbeddingVersion("v1".to_string()),
            dimensions: dim,
        }
    }

    #[test]
    fn build_batch_round_trip_basic() {
        let recs = vec![make_rec(1, 4), make_rec(2, 4)];
        let batch = build_batch(&recs, 4, OffsetDateTime::UNIX_EPOCH).unwrap();
        assert_eq!(batch.num_rows(), 2);
        assert_eq!(batch.num_columns(), 8);
        let schema = batch.schema();
        assert_eq!(schema.field(0).name(), "chunk_id");
        assert_eq!(schema.field(2).name(), "embedding");
    }

    #[test]
    fn build_batch_dim_mismatch_errors() {
        let mut rec = make_rec(1, 4);
        rec.vector = vec![0.0_f32; 3];
        let err = build_batch(&[rec], 4, OffsetDateTime::UNIX_EPOCH).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("does not match table dim"), "msg={msg}");
    }

    #[test]
    fn schema_params_hash_is_stable_for_dim() {
        let h1 = schema_params_hash(384);
        let h2 = schema_params_hash(384);
        assert_eq!(h1, h2);
        let h3 = schema_params_hash(512);
        assert_ne!(h1, h3);
    }
}
