//! Per-page text extraction. `lopdf::Document::extract_text(&[page])`
//! is the call we lean on; it has a thin history of panicking on
//! malformed pages, so we wrap it in `catch_unwind` to convert the
//! panic into a recoverable `Err` (which the caller maps to an empty
//! page + Warning).

use std::panic::{AssertUnwindSafe, catch_unwind};

pub(crate) fn extract_one(doc: &lopdf::Document, page: u32) -> anyhow::Result<String> {
    let result = catch_unwind(AssertUnwindSafe(|| doc.extract_text(&[page])))
        .map_err(|_| anyhow::anyhow!("panic during lopdf::Document::extract_text"))?;
    result.map_err(|e| anyhow::anyhow!("lopdf extract_text error: {e}"))
}
