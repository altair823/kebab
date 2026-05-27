// crates/kebab-parse-pdf/src/page_image.rs (신규)
//
// PDF page → DCTDecode JPEG bytes extract. lopdf 의 page 의 Resources/XObject
// 를 traverse, 첫 image XObject 의 /Filter 검사, DCTDecode + JPEG magic
// 검증 통과 시 raw bytes 반환. 다른 encoding (FlateDecode / CCITTFax /
// JPXDecode) 또는 image XObject 없음 시 Ok(None).
//
// v1 scope = DCTDecode passthrough only (H-3 resolution 갈래 A). image
// crate 도입 0 → single binary 원칙 보존.

use anyhow::{Context, Result};
use lopdf::{Document, Object};

pub fn extract_dctdecode_page_image(
    pdf_doc: &Document,
    page_num: u32,
) -> Result<Option<Vec<u8>>> {
    let pages = pdf_doc.get_pages();
    let &page_oid = pages.get(&page_num)
        .with_context(|| format!("page {page_num} not in get_pages()"))?;

    // page → /Resources → /XObject → traverse for first /Subtype /Image with /Filter == /DCTDecode.
    let page = pdf_doc.get_dictionary(page_oid)?;
    let resources_obj = page.get(b"Resources").ok();
    let resources = match resources_obj {
        Some(Object::Dictionary(d)) => Some(d.clone()),
        Some(Object::Reference(r)) => pdf_doc.get_dictionary(*r).ok().cloned(),
        _ => None,
    };
    let resources = match resources { Some(r) => r, None => return Ok(None) };

    let xobject_obj = resources.get(b"XObject").ok();
    let xobject = match xobject_obj {
        Some(Object::Dictionary(d)) => d.clone(),
        Some(Object::Reference(r)) => match pdf_doc.get_dictionary(*r) { Ok(d) => d.clone(), Err(_) => return Ok(None) },
        _ => return Ok(None),
    };

    for (_name, obj) in xobject.iter() {
        let stream_oid = match obj {
            Object::Reference(r) => *r,
            _ => continue,
        };
        let stream = match pdf_doc.get_object(stream_oid) {
            Ok(Object::Stream(s)) => s.clone(),
            _ => continue,
        };
        let subtype_is_image = stream.dict.get(b"Subtype")
            .ok()
            .and_then(|o| match o { Object::Name(n) => Some(n.as_slice()), _ => None })
            .is_some_and(|n| n == b"Image");
        if !subtype_is_image { continue; }

        let filter_obj = stream.dict.get(b"Filter").ok();
        let is_dct_only = match filter_obj {
            Some(Object::Name(n)) => n.as_slice() == b"DCTDecode",
            Some(Object::Array(arr)) => arr.len() == 1
                && matches!(arr.first(), Some(Object::Name(n)) if n.as_slice() == b"DCTDecode"),
            _ => false,
        };
        if !is_dct_only { continue; }

        // raw bytes — lopdf 의 stream.content 는 already-encoded (filter 적용
        // 후). DCTDecode 의 경우 raw JPEG bytes.
        let bytes = stream.content.clone();
        if bytes.len() < 4 || &bytes[0..2] != b"\xFF\xD8" {
            tracing::warn!(
                target: "kebab-parse-pdf",
                "page={} DCTDecode stream missing JPEG magic byte (\\xFF\\xD8), skip", page_num
            );
            return Ok(None);
        }
        return Ok(Some(bytes));
    }
    Ok(None)
}
