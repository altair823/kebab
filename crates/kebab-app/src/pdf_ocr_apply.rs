// crates/kebab-app/src/pdf_ocr_apply.rs
//
// PDF post-extract OCR enrichment. parser isolation 보존 — kebab-parse-pdf 가
// kebab-parse-image::OcrEngine 을 import 하지 않도록, helper 는 kebab-app 에 둠.
// image path 의 apply_ocr (kebab-parse-image::ocr::apply_ocr) 의
// PDF page 변형 — image 는 ImageRefBlock.ocr 를 mutate, PDF 는
// Block::Paragraph.text / inlines 를 in-place mutate (단일 OCR fallback) 또는
// 새 Block::Paragraph 를 push (always_on dual-block).

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use anyhow::{Context, Result};
use kebab_core::{
    Block, CanonicalDocument, CommonBlock, Inline, Lang, ProvenanceEvent,
    ProvenanceKind, SourceSpan, TextBlock, id_for_block,
};
use kebab_parse_image::OcrEngine;
use kebab_parse_pdf::{compute_valid_char_ratio, extract_dctdecode_page_image};
use lopdf::Document as LopdfDocument;
use time::OffsetDateTime;
use tracing::warn;

pub struct PdfOcrOpts {
    pub enabled: bool,
    pub always_on: bool,
    pub valid_ratio_threshold: f32,
    pub min_char_count: u32,
    pub lang_hint: Option<Lang>,
    /// Optional per-page cancellation handle. checked at start of each page
    /// loop iteration; set→true 시 `cancelled mid-PDF` error 반환. plan §6 E4
    /// + verifier LOW L-1 resolution + spec §4.8 line 1159 명시.
    pub cancel: Option<Arc<AtomicBool>>,
}

#[derive(Debug)]
pub struct PdfOcrSummary {
    pub pages_ocrd: u32,
    pub ms_total: u64,
}

pub fn apply_ocr_to_pdf_pages<F>(
    canonical: &mut CanonicalDocument,
    engine: &dyn OcrEngine,
    pdf_bytes: &[u8],
    opts: &PdfOcrOpts,
    mut emit_progress: F,
) -> Result<PdfOcrSummary>
where
    F: FnMut(PdfOcrProgress),
{
    if !opts.enabled {
        return Ok(PdfOcrSummary { pages_ocrd: 0, ms_total: 0 });
    }
    let pdf_doc = LopdfDocument::load_mem(pdf_bytes)
        .context("kb-app::pdf_ocr_apply: re-parse PDF for image extract")?;
    let page_count = pdf_doc.get_pages().len() as u32;

    let mut new_events: Vec<ProvenanceEvent> = Vec::new();
    let mut ocr_blocks: Vec<Block> = Vec::new();
    let mut pages_ocrd: u32 = 0;
    let mut ms_total: u64 = 0;

    // canonical.blocks 의 page → block index map (text-detect block 의 in-place
    // mutate 또는 dual-block push 결정용).
    // PdfTextExtractor 가 page 마다 1 Block::Paragraph + SourceSpan::Page 를
    // 생성 (§1.4) — 그 invariant 사용.
    for page_num in 1..=page_count {
        if let Some(cancel) = &opts.cancel {
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                anyhow::bail!("PDF OCR cancelled mid-PDF at page {page_num}");
            }
        }

        let text_block_idx = find_paragraph_block_idx(&canonical.blocks, page_num);
        let text = match &canonical.blocks[text_block_idx] {
            Block::Paragraph(tb) => tb.text.clone(),
            _ => String::new(),
        };
        let chars = text.chars().count() as u32;
        let valid_ratio = compute_valid_char_ratio(&text);
        let needs_ocr =
            chars < opts.min_char_count || valid_ratio < opts.valid_ratio_threshold;

        // 결정 matrix:
        //   always_on=true → 모든 page OCR (dual-block).
        //   always_on=false + needs_ocr → in-place OCR (text-detect block mutate).
        //   needs_ocr=false → skip.
        let do_ocr = opts.always_on || needs_ocr;
        if !do_ocr {
            continue;
        }

        emit_progress(PdfOcrProgress::Started { page: page_num });

        let page_image_bytes = if let Some(b) = extract_dctdecode_page_image(&pdf_doc, page_num)? { b } else {
            let note = format!(
                "page={page_num} skipped: no DCTDecode image XObject (vector PDF page or unsupported /Filter — v1 supports DCTDecode passthrough only; see release notes for normalization guidance)"
            );
            warn!(target: "kebab-app", "{}", note);
            new_events.push(ProvenanceEvent {
                at: OffsetDateTime::now_utc(),
                agent: "kb-parse-pdf".to_string(),
                kind: ProvenanceKind::Warning,
                note: Some(note),
            });
            emit_progress(PdfOcrProgress::Finished {
                page: page_num,
                ms: 0,
                chars: 0,
                skipped: true,
            });
            continue;
        };

        let start = Instant::now();
        let ocr = match engine.recognize(&page_image_bytes, opts.lang_hint.as_ref()) {
            Ok(t) => t,
            Err(e) => {
                // OCR failure: warning event + skip (text-detect block 그대로).
                let note = format!(
                    "page={} OCR failed engine={} version={} err={}",
                    page_num,
                    engine.engine_name(),
                    engine.engine_version(),
                    e
                );
                warn!(target: "kebab-app", "{}", note);
                new_events.push(ProvenanceEvent {
                    at: OffsetDateTime::now_utc(),
                    agent: "kb-parse-pdf".to_string(),
                    kind: ProvenanceKind::Warning,
                    note: Some(note),
                });
                emit_progress(PdfOcrProgress::Finished {
                    page: page_num,
                    ms: start.elapsed().as_millis() as u64,
                    chars: 0,
                    skipped: true,
                });
                continue;
            }
        };
        let elapsed_ms = start.elapsed().as_millis() as u64;
        let chars_ocr = ocr.joined.chars().count() as u32;

        pages_ocrd = pages_ocrd.saturating_add(1);
        ms_total = ms_total.saturating_add(elapsed_ms);

        if opts.always_on && !needs_ocr {
            // dual-block path: 새 Block::Paragraph push, ordinal = page-1 + page_count.
            let ocr_ordinal = (page_num - 1) + page_count;
            let span_ocr = SourceSpan::Page {
                page: page_num,
                char_start: Some(0),
                char_end: Some(chars_ocr),
            };
            let block_id =
                id_for_block(&canonical.doc_id, "paragraph", &[], ocr_ordinal, &span_ocr);
            let common = CommonBlock {
                block_id,
                heading_path: Vec::new(),
                source_span: span_ocr,
            };
            ocr_blocks.push(Block::Paragraph(TextBlock {
                common,
                text: ocr.joined.clone(),
                inlines: if ocr.joined.is_empty() {
                    Vec::new()
                } else {
                    vec![Inline::Text {
                        text: ocr.joined.clone(),
                    }]
                },
            }));
        } else {
            // in-place mutate: text-detect block (빈 또는 low-valid) 의 text/inlines 교체.
            // block_id / ordinal 보존 — span 의 char_end 만 갱신.
            if let Block::Paragraph(tb) = &mut canonical.blocks[text_block_idx] {
                tb.text = ocr.joined.clone();
                tb.inlines = if ocr.joined.is_empty() {
                    Vec::new()
                } else {
                    vec![Inline::Text {
                        text: ocr.joined.clone(),
                    }]
                };
                if let SourceSpan::Page { char_end, .. } = &mut tb.common.source_span {
                    *char_end = Some(chars_ocr);
                }
            }
        }

        new_events.push(ProvenanceEvent {
            at: OffsetDateTime::now_utc(),
            agent: "kb-parse-pdf".to_string(),
            kind: ProvenanceKind::OcrApplied,
            note: Some(format!(
                "page={} engine={} version={} regions={} ms={} chars={}",
                page_num,
                engine.engine_name(),
                engine.engine_version(),
                ocr.regions.len(),
                elapsed_ms,
                chars_ocr
            )),
        });

        emit_progress(PdfOcrProgress::Finished {
            page: page_num,
            ms: elapsed_ms,
            chars: chars_ocr,
            skipped: false,
        });
    }

    canonical.blocks.extend(ocr_blocks);
    canonical.provenance.events.extend(new_events);
    Ok(PdfOcrSummary { pages_ocrd, ms_total })
}

fn find_paragraph_block_idx(blocks: &[Block], page_num: u32) -> usize {
    blocks
        .iter()
        .position(|b| match b {
            Block::Paragraph(tb) => matches!(
                tb.common.source_span,
                SourceSpan::Page { page, .. } if page == page_num
            ),
            _ => false,
        })
        .expect("PdfTextExtractor emits 1 Block::Paragraph per page (invariant)")
}

pub enum PdfOcrProgress {
    Started { page: u32 },
    Finished {
        page: u32,
        ms: u64,
        chars: u32,
        skipped: bool,
    },
}
