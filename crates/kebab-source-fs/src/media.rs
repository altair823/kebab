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
    // p10-2: code_lang_for_path is the single source of truth for code lang
    // (design §3.5). Delegate before falling back to extension branches.
    if let Some(lang) = crate::code_meta::code_lang_for_path(path) {
        return MediaType::Code(lang.to_string());
    }

    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();

    match ext.as_str() {
        // Markdown + MDX (markdown + JSX, treated as plain markdown — the
        // JSX islands are folded into raw passthrough by the md parser).
        "md" | "mdx" => MediaType::Markdown,
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
        // Cargo.toml is a Tier 2 code manifest (p10-2), handled by code_lang_for_path
        assert_eq!(
            media_type_for(Path::new("Cargo.toml")),
            MediaType::Code("toml".to_string())
        );
    }

    #[test]
    fn py_ts_js_files_map_to_media_code() {
        assert_eq!(
            media_type_for(Path::new("a/b.py")),
            MediaType::Code("python".into())
        );
        assert_eq!(
            media_type_for(Path::new("a/b.pyi")),
            MediaType::Code("python".into())
        );
        assert_eq!(
            media_type_for(Path::new("a/b.ts")),
            MediaType::Code("typescript".into())
        );
        assert_eq!(
            media_type_for(Path::new("a/b.tsx")),
            MediaType::Code("typescript".into())
        );
        assert_eq!(
            media_type_for(Path::new("a/b.js")),
            MediaType::Code("javascript".into())
        );
        assert_eq!(
            media_type_for(Path::new("a/b.mjs")),
            MediaType::Code("javascript".into())
        );
        assert_eq!(
            media_type_for(Path::new("a/b.cjs")),
            MediaType::Code("javascript".into())
        );
        assert_eq!(
            media_type_for(Path::new("a/b.jsx")),
            MediaType::Code("javascript".into())
        );
        assert_eq!(
            media_type_for(Path::new("a/b.rs")),
            MediaType::Code("rust".into())
        );
    }

    #[test]
    fn ts_variants_mts_cts() {
        // .mts / .cts are TypeScript ESM / CommonJS — same grammar as .ts.
        assert_eq!(
            media_type_for(Path::new("a/b.mts")),
            MediaType::Code("typescript".into())
        );
        assert_eq!(
            media_type_for(Path::new("a/b.cts")),
            MediaType::Code("typescript".into())
        );
    }

    #[test]
    fn mdx_routes_to_markdown() {
        // MDX is markdown with JSX islands; the md parser folds the JSX
        // through as raw passthrough.
        assert_eq!(
            media_type_for(Path::new("docs/page.mdx")),
            MediaType::Markdown
        );
    }

    #[test]
    fn go_files_map_to_media_code_go() {
        assert_eq!(
            media_type_for(Path::new("a/b.go")),
            MediaType::Code("go".into())
        );
    }

    #[test]
    fn java_kotlin_files_map_to_media_code() {
        assert_eq!(
            media_type_for(Path::new("a/b.java")),
            MediaType::Code("java".into())
        );
        assert_eq!(
            media_type_for(Path::new("a/b.kt")),
            MediaType::Code("kotlin".into())
        );
        assert_eq!(
            media_type_for(Path::new("a/b.kts")),
            MediaType::Code("kotlin".into())
        );
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

    #[test]
    fn tier2_files_map_to_media_code() {
        assert_eq!(
            media_type_for(Path::new("a/deploy.yaml")),
            MediaType::Code("yaml".into())
        );
        assert_eq!(
            media_type_for(Path::new("a/Dockerfile")),
            MediaType::Code("dockerfile".into())
        );
        assert_eq!(
            media_type_for(Path::new("a/Cargo.toml")),
            MediaType::Code("toml".into())
        );
        assert_eq!(
            media_type_for(Path::new("a/pom.xml")),
            MediaType::Code("xml".into())
        );
        assert_eq!(
            media_type_for(Path::new("a/build.gradle")),
            MediaType::Code("groovy".into())
        );
        assert_eq!(
            media_type_for(Path::new("a/go.mod")),
            MediaType::Code("go-mod".into())
        );
    }
}
