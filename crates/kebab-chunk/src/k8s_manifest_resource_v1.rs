//! p10-2: k8s manifest resource-aware chunker.
//!
//! Splits a multi-document YAML file on `^---\s*$` boundaries, recognises
//! documents that have both `apiVersion` and `kind` string fields as k8s
//! resources, and emits one `Chunk` per resource (with oversize >200-line
//! fallback).  Non-k8s documents are skipped; invalid YAML yields 0 chunks
//! for the entire file.

use crate::tier2_shared::{policy_hash, push_chunks_with_oversize};
use anyhow::Result;
use kebab_core::{Block, CanonicalDocument, Chunk, ChunkPolicy, ChunkerVersion, Chunker};

pub const VERSION_LABEL: &str = "k8s-manifest-resource-v1";

#[derive(Clone, Copy, Debug, Default)]
pub struct K8sManifestResourceV1Chunker;

impl Chunker for K8sManifestResourceV1Chunker {
    fn chunker_version(&self) -> ChunkerVersion {
        ChunkerVersion(VERSION_LABEL.to_string())
    }

    fn policy_hash(&self, policy: &ChunkPolicy) -> String {
        policy_hash(policy)
    }

    fn chunk(&self, doc: &CanonicalDocument, policy: &ChunkPolicy) -> Result<Vec<Chunk>> {
        // Expect a single Block::Code carrying the full YAML text.
        let text = match doc.blocks.first() {
            Some(Block::Code(cb)) => cb.code.as_str(),
            _ => return Ok(vec![]),
        };

        let slices = split_yaml_documents(text);
        let mut chunks: Vec<Chunk> = Vec::new();

        for slice in slices {
            // Invalid YAML in any document → return 0 chunks for the file.
            let value: serde_yaml::Value = match serde_yaml::from_str(slice.text) {
                Ok(v) => v,
                Err(_) => return Ok(vec![]),
            };

            let Some(mapping) = value.as_mapping() else {
                continue;
            };

            let api = mapping
                .get("apiVersion")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let kind = mapping
                .get("kind")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            // Skip non-k8s documents.
            if api.is_empty() || kind.is_empty() {
                continue;
            }

            let metadata = mapping
                .get("metadata")
                .and_then(|v| v.as_mapping());
            let name = metadata
                .and_then(|m| m.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("<unnamed>");
            let namespace = metadata
                .and_then(|m| m.get("namespace"))
                .and_then(|v| v.as_str());

            let symbol = match namespace {
                Some(ns) if !ns.is_empty() => format!("{kind}/{ns}/{name}"),
                _ => format!("{kind}/{name}"),
            };

            push_chunks_with_oversize(
                &mut chunks,
                doc,
                policy,
                slice.text,
                slice.line_start,
                slice.line_end,
                &symbol,
                "yaml",
                VERSION_LABEL,
                Some(slice.line_start),
            )?;
        }

        tracing::debug!(
            target: "kebab-chunk",
            doc_id = %doc.doc_id,
            chunks = chunks.len(),
            "k8s-manifest-resource-v1 chunked",
        );

        Ok(chunks)
    }
}

struct YamlSlice<'a> {
    text: &'a str,
    line_start: u32,
    line_end: u32,
}

/// Split raw YAML text into per-document slices on `---` separator lines.
/// Line numbers are 1-indexed.
fn split_yaml_documents(text: &str) -> Vec<YamlSlice<'_>> {
    let lines: Vec<&str> = text.lines().collect();

    // Collect indices of separator lines (0-based), then append a sentinel at
    // the end so the last slice is always terminated.
    let mut separators: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter_map(|(i, l)| {
            let trimmed = l.trim_end();
            if trimmed == "---"
                || trimmed.starts_with("--- ")
                || trimmed.starts_with("---\t")
            {
                Some(i)
            } else {
                None
            }
        })
        .collect();
    separators.push(lines.len());

    let mut slices: Vec<YamlSlice<'_>> = Vec::new();
    let mut doc_start_line: usize = 0; // 0-based index of current doc start

    for sep_line in separators {
        if sep_line > doc_start_line {
            let start_byte = byte_offset_of_line(text, doc_start_line);
            let end_byte = byte_offset_of_line(text, sep_line);
            let slice_text = &text[start_byte..end_byte];
            if !slice_text.trim().is_empty() {
                slices.push(YamlSlice {
                    text: slice_text,
                    line_start: (doc_start_line + 1) as u32,
                    line_end: sep_line as u32,
                });
            }
        }
        doc_start_line = sep_line + 1;
    }

    slices
}

/// Return the byte offset of the start of `line_idx` (0-based line index).
fn byte_offset_of_line(text: &str, line_idx: usize) -> usize {
    if line_idx == 0 {
        return 0;
    }
    let mut count = 0usize;
    for (i, c) in text.char_indices() {
        if c == '\n' {
            count += 1;
            if count == line_idx {
                return i + 1;
            }
        }
    }
    text.len()
}
