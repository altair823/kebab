//! `cargo run --example gen_smoke_pdf -p kebab-parse-pdf -- <out.pdf> <text-page-1> [<text-page-N> ...]`
//!
//! Tiny generator used by the SMOKE runbook to produce text PDFs without
//! adding `reportlab` / `qpdf` to the dev-machine prerequisites. Mirrors
//! the `tests/common::build_text_pdf` builder.

use lopdf::content::{Content, Operation};
use lopdf::{Document, Object, Stream, dictionary};
use std::fs::File;
use std::io::Write;

fn main() {
    let mut args = std::env::args().skip(1);
    let out = args.next().expect("usage: gen_smoke_pdf <out.pdf> <text...>");
    let pages: Vec<String> = args.collect();
    if pages.is_empty() {
        eprintln!("at least one page text required");
        std::process::exit(2);
    }

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
    for text in &pages {
        let content = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 14.into()]),
                Operation::new(
                    "Td",
                    vec![Object::Integer(72), Object::Integer(720)],
                ),
                Operation::new("Tj", vec![Object::string_literal(text.as_str())]),
                Operation::new("ET", vec![]),
            ],
        };
        let stream_data = content.encode().expect("content encode");
        let content_id = doc.add_object(Stream::new(dictionary! {}, stream_data));
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
        });
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
    doc.objects
        .insert(pages_id, Object::Dictionary(pages_dict));
    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    doc.trailer.set("Root", catalog_id);

    let mut buf: Vec<u8> = Vec::new();
    doc.save_to(&mut buf).expect("save PDF");
    File::create(&out)
        .expect("create out")
        .write_all(&buf)
        .expect("write");
    println!("wrote {} ({} bytes, {} pages)", out, buf.len(), pages.len());
}
