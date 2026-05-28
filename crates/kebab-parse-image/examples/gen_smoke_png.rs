//! `cargo run --example gen_smoke_png -p kebab-parse-image -- <out.png>`
//!
//! 100×50 solid-red PNG used by the SMOKE runbook for the image
//! filename-indexing path (OCR / caption disabled).

use image::{ImageBuffer, Rgb};
use std::io::Cursor;

fn main() {
    let out = std::env::args()
        .nth(1)
        .expect("usage: gen_smoke_png <out.png>");
    let img: ImageBuffer<Rgb<u8>, _> = ImageBuffer::from_fn(100, 50, |_, _| Rgb([255, 0, 0]));
    let mut buf = Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Png)
        .expect("encode PNG");
    let bytes = buf.into_inner();
    std::fs::write(&out, &bytes).expect("write");
    println!("wrote {} ({} bytes)", out, bytes.len());
}
