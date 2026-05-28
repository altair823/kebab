//! Shared helpers for building `kebab_core::Citation` values from a
//! chunk's first `SourceSpan`.
//!
//! Both the lexical and vector retrievers join against the same
//! `chunks.source_spans_json` column and need identical mapping logic
//! so cross-mode citation strings round-trip byte-identically (a
//! requirement for the hybrid retriever's tie-break on chunk_id and
//! for the `search --explain` output documented in design §0 Q3 and
//! §1.6). Living here means a future PDF / image / audio extractor can
//! enrich the mapping in one place rather than two.

use kebab_core::{Citation, SourceSpan, WorkspacePath};

/// Build a `Citation` from the chunk's first `SourceSpan`. P1 markdown
/// only emits `Line`, so the other variants are mostly defensive — we
/// forward them as faithfully as possible so a future PDF / image
/// extractor can flow through without churn.
///
/// `chunk_id` is taken only for diagnostic logging when the span shape
/// has no Citation mapping (`Byte`-spans, empty arrays).
pub(crate) fn citation_from_first_span(
    chunk_id: &str,
    path: WorkspacePath,
    section: Option<String>,
    first_span: Option<&SourceSpan>,
) -> Citation {
    match first_span {
        Some(SourceSpan::Line { start, end }) => Citation::Line {
            path,
            start: *start,
            end: *end,
            section,
        },
        Some(SourceSpan::Page { page, .. }) => Citation::Page {
            path,
            page: *page,
            section,
        },
        Some(SourceSpan::Region { x, y, w, h }) => Citation::Region {
            path,
            x: *x,
            y: *y,
            w: *w,
            h: *h,
        },
        Some(SourceSpan::Time { start_ms, end_ms }) => Citation::Time {
            path,
            start_ms: *start_ms,
            end_ms: *end_ms,
            speaker: None,
        },
        Some(SourceSpan::Code {
            line_start,
            line_end,
            symbol,
            lang,
        }) => Citation::Code {
            path,
            line_start: *line_start,
            line_end: *line_end,
            symbol: symbol.clone(),
            lang: lang.clone(),
        },
        // Byte-spans don't have a Citation variant. Fall back to a
        // Line citation pointing at the document head — better than
        // fabricating a position. Spans-empty falls into the same
        // branch.
        other @ (Some(SourceSpan::Byte { .. }) | None) => {
            let span_shape = match other {
                Some(_) => "Byte",
                None => "empty array",
            };
            tracing::warn!(
                chunk_id,
                span_shape,
                "kb-search: SourceSpan has no Citation mapping; falling back to Line {{1, 1}}"
            );
            Citation::Line {
                path,
                start: 1,
                end: 1,
                section,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use kebab_core::{Citation, SourceSpan, WorkspacePath};

    #[test]
    fn build_citation_code_maps_symbol_and_lang() {
        let span = SourceSpan::Code {
            line_start: 5,
            line_end: 30,
            symbol: Some("chunk::md_heading_v1::MdHeadingV1Chunker::chunk".into()),
            lang: Some("rust".into()),
        };
        let c = super::citation_from_first_span(
            "c1",
            WorkspacePath::new("crates/kebab-chunk/src/md_heading_v1.rs".to_string()).unwrap(),
            None,
            Some(&span),
        );
        match c {
            Citation::Code {
                path,
                line_start,
                line_end,
                symbol,
                lang,
            } => {
                assert_eq!(path.0, "crates/kebab-chunk/src/md_heading_v1.rs");
                assert_eq!(line_start, 5);
                assert_eq!(line_end, 30);
                assert_eq!(
                    symbol.as_deref(),
                    Some("chunk::md_heading_v1::MdHeadingV1Chunker::chunk")
                );
                assert_eq!(lang.as_deref(), Some("rust"));
            }
            other => panic!("expected Citation::Code, got {other:?}"),
        }
    }
}
