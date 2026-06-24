//! Test fixture builders for `kebab-parse-pdf`.
//!
//! PDFs are constructed in-memory at test time via `lopdf` rather than
//! committed as binary fixtures. Same rationale as
//! `kebab-parse-image::tests::common`: fixture provenance is auditable
//! from source, no `include_bytes!` paths to keep in sync, and the test
//! binary stays self-contained.

#![allow(dead_code)]

use std::path::PathBuf;

use kebab_core::{
    AssetStorage, Checksum, ExtractConfig, ExtractContext, MediaType, RawAsset, SourceUri,
    WorkspacePath,
};
use lopdf::content::{Content, Operation};
use lopdf::{Document, Object, Stream, dictionary};
use time::OffsetDateTime;

/// `/Info` dict fields a fixture wants to surface (all optional).
#[derive(Default, Clone)]
pub struct InfoDict {
    pub title: Option<Vec<u8>>, // raw bytes — caller controls PDFDocEncoding vs UTF-16BE
    pub producer: Option<&'static str>,
    pub creator: Option<&'static str>,
}

/// Build a Helvetica-text PDF. `pages` is one entry per page; `None`
/// means the page exists in `/Pages` but has no `/Contents` stream
/// (the "scanned candidate" shape — `extract_text` returns empty).
pub fn build_text_pdf(pages: &[Option<&str>]) -> Vec<u8> {
    build_text_pdf_with_info(pages, &InfoDict::default())
}

pub fn build_text_pdf_with_info(pages: &[Option<&str>], info: &InfoDict) -> Vec<u8> {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
    });
    let resources_id = doc.add_object(dictionary! {
        "Font" => dictionary! { "F1" => font_id },
    });

    let mut page_refs: Vec<Object> = Vec::new();
    for page in pages {
        let mut page_dict = dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
        };
        if let Some(text) = page {
            let content = Content {
                operations: vec![
                    Operation::new("BT", vec![]),
                    Operation::new("Tf", vec!["F1".into(), 24.into()]),
                    Operation::new("Td", vec![Object::Integer(100), Object::Integer(700)]),
                    Operation::new("Tj", vec![Object::string_literal(*text)]),
                    Operation::new("ET", vec![]),
                ],
            };
            let stream_data = content.encode().expect("content encode");
            let content_id = doc.add_object(Stream::new(dictionary! {}, stream_data));
            page_dict.set("Contents", content_id);
        }
        let page_id = doc.add_object(page_dict);
        page_refs.push(page_id.into());
    }

    let count = page_refs.len() as i64;
    let pages_dict = dictionary! {
        "Type" => "Pages",
        "Kids" => page_refs,
        "Count" => count,
        "Resources" => resources_id,
        "MediaBox" => vec![
            Object::Integer(0),
            Object::Integer(0),
            Object::Integer(595),
            Object::Integer(842),
        ],
    };
    doc.objects.insert(pages_id, Object::Dictionary(pages_dict));

    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    doc.trailer.set("Root", catalog_id);

    if info.title.is_some() || info.producer.is_some() || info.creator.is_some() {
        let mut info_dict = lopdf::Dictionary::new();
        if let Some(title) = &info.title {
            info_dict.set(
                "Title",
                Object::String(title.clone(), lopdf::StringFormat::Literal),
            );
        }
        if let Some(p) = info.producer {
            info_dict.set(
                "Producer",
                Object::String(p.as_bytes().to_vec(), lopdf::StringFormat::Literal),
            );
        }
        if let Some(c) = info.creator {
            info_dict.set(
                "Creator",
                Object::String(c.as_bytes().to_vec(), lopdf::StringFormat::Literal),
            );
        }
        let info_id = doc.add_object(Object::Dictionary(info_dict));
        doc.trailer.set("Info", info_id);
    }

    let mut out: Vec<u8> = Vec::new();
    doc.save_to(&mut out).expect("save PDF to memory");
    out
}

/// Wrap any valid PDF byte buffer with a fake `/Encrypt` trailer entry
/// so `Document::is_encrypted()` flips to true. We don't actually
/// encrypt anything — the extractor refuses encrypted PDFs **before**
/// touching streams, so the marker is sufficient.
pub fn make_encrypted_pdf() -> Vec<u8> {
    let bytes = build_text_pdf(&[Some("placeholder")]);
    let mut doc = Document::load_mem(&bytes).expect("load round-tripped PDF");
    let enc_id = doc.add_object(dictionary! {
        "Filter" => "Standard",
        "V" => 1,
        "R" => 2,
        "Length" => 40,
        "P" => -4,
    });
    doc.trailer.set("Encrypt", enc_id);
    let mut out = Vec::new();
    doc.save_to(&mut out).expect("save encrypted PDF");
    out
}

/// 27-byte garbage with no `%PDF-` header — `Document::load_mem` errors.
pub fn corrupt_pdf() -> Vec<u8> {
    b"NOT A PDF; just plain bytes".to_vec()
}

/// Encode a Rust `&str` as the PDF UTF-16BE-with-BOM string format.
/// Used to verify `info::pdf_string` decodes the multilingual Title
/// path correctly.
pub fn utf16be_bom(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(2 + s.encode_utf16().count() * 2);
    out.extend_from_slice(&[0xFE, 0xFF]);
    for unit in s.encode_utf16() {
        out.extend_from_slice(&unit.to_be_bytes());
    }
    out
}

/// Asset + ExtractContext fixture, mirroring `kebab-parse-image::tests::common`.
pub struct PdfFixture {
    pub asset: RawAsset,
    workspace_root: PathBuf,
    config: ExtractConfig,
}

impl PdfFixture {
    pub fn ctx(&self) -> ExtractContext<'_> {
        ExtractContext {
            asset: &self.asset,
            workspace_root: &self.workspace_root,
            config: &self.config,
            source_id: None,
            source_trust: None,
        }
    }
}

pub fn fixture_for(workspace_path: &str, bytes: &[u8]) -> PdfFixture {
    let blake = blake3::hash(bytes);
    let full_hex = blake.to_hex().to_string();
    let asset_id = kebab_core::id_for_asset(&full_hex);
    let workspace_path = WorkspacePath::new(workspace_path.to_string()).unwrap();
    let discovered_at = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
    let asset = RawAsset {
        asset_id,
        source_uri: SourceUri::File(PathBuf::from(format!("/tmp/{}", workspace_path.0))),
        workspace_path,
        media_type: MediaType::Pdf,
        byte_len: bytes.len() as u64,
        checksum: Checksum(full_hex),
        discovered_at,
        stored: AssetStorage::Reference {
            path: PathBuf::from("/tmp/fake"),
            sha: Checksum("0".repeat(64)),
        },
    };
    PdfFixture {
        asset,
        workspace_root: PathBuf::from("/tmp/fake-root"),
        config: ExtractConfig::default(),
    }
}

/// Replace every provenance event timestamp after index 0 (Discovered)
/// with `<stripped>` so determinism / snapshot tests can compare JSON
/// across runs. Same shape as `kebab-parse-image::tests::common::strip_dynamic_at`.
pub fn strip_dynamic_at(json: &mut serde_json::Value) {
    if let Some(events) = json
        .get_mut("provenance")
        .and_then(|p| p.get_mut("events"))
        .and_then(|e| e.as_array_mut())
    {
        for (i, ev) in events.iter_mut().enumerate() {
            if i > 0
                && let Some(obj) = ev.as_object_mut()
            {
                obj.insert("at".into(), serde_json::Value::String("<stripped>".into()));
            }
        }
    }
}
