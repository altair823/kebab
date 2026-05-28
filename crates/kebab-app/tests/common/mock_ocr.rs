use std::sync::Mutex;

use anyhow::Result;
use kebab_core::{Lang, OcrText};
use kebab_parse_image::OcrEngine;

pub struct MockOcrEngine {
    expected_texts: Vec<String>,
    call_index: Mutex<usize>,
    fail: bool,
}

impl MockOcrEngine {
    /// Single text (backward-compat ctor for pdf_ocr_apply.rs 10 sites).
    pub fn single(text: impl Into<String>, fail: bool) -> Self {
        Self {
            expected_texts: vec![text.into()],
            call_index: Mutex::new(0),
            fail,
        }
    }

    /// Per-page texts (cursor advances per recognize call).
    pub fn per_page(texts: Vec<String>, fail: bool) -> Self {
        Self {
            expected_texts: texts,
            call_index: Mutex::new(0),
            fail,
        }
    }
}

impl OcrEngine for MockOcrEngine {
    fn engine_name(&self) -> &'static str {
        "mock-ocr"
    }

    fn engine_version(&self) -> String {
        "mock-v1".to_string()
    }

    fn recognize(&self, _img: &[u8], _hint: Option<&Lang>) -> Result<OcrText> {
        if self.fail {
            anyhow::bail!("mock failure");
        }
        let mut idx = self.call_index.lock().unwrap();
        let text = self
            .expected_texts
            .get(*idx)
            .cloned()
            .unwrap_or_else(|| self.expected_texts.last().cloned().unwrap_or_default());
        *idx += 1;
        Ok(OcrText {
            joined: text,
            regions: vec![],
            engine: "mock-ocr".to_string(),
            engine_version: "mock-v1".to_string(),
        })
    }
}
