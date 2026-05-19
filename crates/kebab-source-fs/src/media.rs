//! Media-type detection by extension. Per P1-1 task spec we do NOT do
//! libmagic-style sniffing; extension is enough for P1. Unknown / missing
//! extensions fall through to `MediaType::Other(ext.to_string())` (empty
//! string when the file has no extension at all).

use std::path::Path;

use kebab_core::{AudioType, ImageType, MediaType};

/// Return `MediaType` for `path` based purely on its lowercased extension.
/// `.md` → Markdown, `.pdf` → Pdf, image and audio extensions map onto
/// `MediaType::Image(_)` / `MediaType::Audio(_)`. Anything else (including
/// missing extension) → `MediaType::Other(ext)`.
pub(crate) fn media_type_for(path: &Path) -> MediaType {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();

    match ext.as_str() {
        "md" => MediaType::Markdown,
        "pdf" => MediaType::Pdf,

        "png" => MediaType::Image(ImageType::Png),
        "jpg" | "jpeg" => MediaType::Image(ImageType::Jpeg),
        "webp" => MediaType::Image(ImageType::Webp),
        "gif" => MediaType::Image(ImageType::Gif),
        "tiff" | "tif" => MediaType::Image(ImageType::Tiff),

        "m4a" => MediaType::Audio(AudioType::M4a),
        "mp3" => MediaType::Audio(AudioType::Mp3),
        "wav" => MediaType::Audio(AudioType::Wav),
        "flac" => MediaType::Audio(AudioType::Flac),
        "ogg" => MediaType::Audio(AudioType::Ogg),

        // p10-1A-2: Rust is the only code lang activated in 1A. Other
        // recognized code langs stay Other until their phase (1B+).
        "rs" => MediaType::Code("rust".to_string()),

        // Empty string (no extension) and any other extension: bucket as
        // Other and let downstream extractors decide if they support it.
        _ => MediaType::Other(ext),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_and_pdf() {
        assert_eq!(media_type_for(Path::new("a/b.md")), MediaType::Markdown);
        assert_eq!(media_type_for(Path::new("a/b.MD")), MediaType::Markdown);
        assert_eq!(media_type_for(Path::new("a/b.pdf")), MediaType::Pdf);
    }

    #[test]
    fn images_and_audio() {
        assert_eq!(
            media_type_for(Path::new("p.jpg")),
            MediaType::Image(ImageType::Jpeg)
        );
        assert_eq!(
            media_type_for(Path::new("p.JPEG")),
            MediaType::Image(ImageType::Jpeg)
        );
        assert_eq!(
            media_type_for(Path::new("a.M4A")),
            MediaType::Audio(AudioType::M4a)
        );
        assert_eq!(
            media_type_for(Path::new("a.flac")),
            MediaType::Audio(AudioType::Flac)
        );
    }

    #[test]
    fn rust_files_map_to_media_code_rust() {
        assert_eq!(
            media_type_for(Path::new("crates/kebab-core/src/lib.rs")),
            MediaType::Code("rust".to_string())
        );
        // non-Rust code extensions stay Other in 1A
        assert_eq!(media_type_for(Path::new("a/b.py")), MediaType::Other("py".to_string()));
        assert_eq!(media_type_for(Path::new("Cargo.toml")), MediaType::Other("toml".to_string()));
    }

    #[test]
    fn unknown_and_missing_extension() {
        assert_eq!(
            media_type_for(Path::new("notes/x.weird")),
            MediaType::Other("weird".to_string())
        );
        assert_eq!(
            media_type_for(Path::new("README")),
            MediaType::Other(String::new())
        );
    }
}
