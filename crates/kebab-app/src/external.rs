//! Helpers for the `_external/` workspace subdirectory used by
//! `ingest_file_with_config` and `ingest_stdin_with_config` (p9-fb-31).
//!
//! - `ensure_external_dir`: create `<workspace.root>/_external/` if absent.
//! - `ensure_kebabignore_entry`: append `_external/` to `<workspace.root>/.kebabignore`
//!   if missing — prevents subsequent `kebab ingest` workspace walks from
//!   re-walking files that were imported via single-file ingest.
//! - `copy_to_external`: write bytes to `_external/<blake3-12>.<ext>`, idempotent.
//! - `inject_frontmatter`: prepend a YAML frontmatter block to a markdown body
//!   string (used by `ingest_stdin_with_config`).

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

pub const EXTERNAL_DIR: &str = "_external";
const KEBABIGNORE_LINE: &str = "_external/";

/// Ensure `<workspace_root>/_external/` exists. Returns the directory path.
pub fn ensure_external_dir(workspace_root: &Path) -> Result<PathBuf> {
    let dir = workspace_root.join(EXTERNAL_DIR);
    fs::create_dir_all(&dir)
        .with_context(|| format!("create _external dir at {}", dir.display()))?;
    Ok(dir)
}

/// Append `_external/` line to `<workspace_root>/.kebabignore` if not already
/// present. Idempotent — checks for the exact line before appending.
pub fn ensure_kebabignore_entry(workspace_root: &Path) -> Result<()> {
    let path = workspace_root.join(".kebabignore");
    let existing = if path.exists() {
        fs::read_to_string(&path)
            .with_context(|| format!("read existing .kebabignore at {}", path.display()))?
    } else {
        String::new()
    };
    let already = existing
        .lines()
        .any(|line| line.trim() == KEBABIGNORE_LINE);
    if already {
        return Ok(());
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("open .kebabignore for append at {}", path.display()))?;
    if !existing.is_empty() && !existing.ends_with('\n') {
        file.write_all(b"\n")?;
    }
    writeln!(file, "{KEBABIGNORE_LINE}")?;
    Ok(())
}

/// Copy bytes to `<external_dir>/<blake3-12>.<ext>`. Idempotent — if the
/// destination file already exists with the expected hash, the existing
/// file is reused (no second write). Returns the destination path.
pub fn copy_to_external(
    external_dir: &Path,
    bytes: &[u8],
    ext: &str,
) -> Result<PathBuf> {
    let hash = blake3::hash(bytes);
    let hex = hash.to_hex();
    let prefix = &hex.as_str()[..12];
    let filename = format!("{prefix}.{ext}");
    let dest = external_dir.join(&filename);
    if !dest.exists() {
        fs::write(&dest, bytes)
            .with_context(|| format!("write external file at {}", dest.display()))?;
    }
    Ok(dest)
}

/// Prepend a YAML frontmatter block to a markdown body. Returns the wrapped
/// markdown string. Errors if `body` already starts with `---` (the user
/// should use `ingest_file_with_config` for files that already carry
/// frontmatter).
///
/// Internal `yaml_quote` always uses double-quoted YAML form with backslash
/// escapes for `"` / `\` / control chars — agent-supplied titles with
/// special characters are safe.
pub fn inject_frontmatter(
    body: &str,
    title: &str,
    source_uri: Option<&str>,
) -> Result<String> {
    let head = body.trim_start();
    if head.starts_with("---\n") || head.starts_with("---\r\n") || head.starts_with("---\r") {
        anyhow::bail!(
            "stdin already has frontmatter; use `kebab ingest-file` for files with metadata"
        );
    }
    let title_yaml = yaml_quote(title);
    let mut header = String::new();
    header.push_str("---\n");
    header.push_str(&format!("title: {title_yaml}\n"));
    if let Some(uri) = source_uri {
        let uri_yaml = yaml_quote(uri);
        header.push_str(&format!("source_uri: {uri_yaml}\n"));
    }
    header.push_str("---\n\n");
    header.push_str(body);
    Ok(header)
}

/// YAML-quote a string. Always uses double-quoted form with backslash-escape
/// for `"` and `\`. Defensive against agent-supplied titles that contain
/// quotes / control chars.
fn yaml_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn ensure_external_dir_creates_dir() {
        let dir = tempdir().unwrap();
        let result = ensure_external_dir(dir.path()).unwrap();
        assert_eq!(result, dir.path().join("_external"));
        assert!(result.is_dir());
    }

    #[test]
    fn ensure_external_dir_is_idempotent() {
        let dir = tempdir().unwrap();
        let _ = ensure_external_dir(dir.path()).unwrap();
        let result = ensure_external_dir(dir.path()).unwrap();
        assert!(result.is_dir());
    }

    #[test]
    fn ensure_kebabignore_entry_creates_file_with_line() {
        let dir = tempdir().unwrap();
        ensure_kebabignore_entry(dir.path()).unwrap();
        let content = fs::read_to_string(dir.path().join(".kebabignore")).unwrap();
        assert!(content.lines().any(|l| l.trim() == "_external/"));
    }

    #[test]
    fn ensure_kebabignore_entry_appends_to_existing() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join(".kebabignore"), "*.tmp\n").unwrap();
        ensure_kebabignore_entry(dir.path()).unwrap();
        let content = fs::read_to_string(dir.path().join(".kebabignore")).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert!(lines.contains(&"*.tmp"));
        assert!(lines.contains(&"_external/"));
    }

    #[test]
    fn ensure_kebabignore_entry_idempotent() {
        let dir = tempdir().unwrap();
        ensure_kebabignore_entry(dir.path()).unwrap();
        ensure_kebabignore_entry(dir.path()).unwrap();
        let content = fs::read_to_string(dir.path().join(".kebabignore")).unwrap();
        let count = content.lines().filter(|l| l.trim() == "_external/").count();
        assert_eq!(count, 1, "should not duplicate");
    }

    #[test]
    fn ensure_kebabignore_entry_handles_missing_trailing_newline() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join(".kebabignore"), "*.tmp").unwrap(); // no \n
        ensure_kebabignore_entry(dir.path()).unwrap();
        let content = fs::read_to_string(dir.path().join(".kebabignore")).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert!(lines.contains(&"*.tmp"));
        assert!(lines.contains(&"_external/"));
    }

    #[test]
    fn copy_to_external_writes_with_hash_prefix_filename() {
        let dir = tempdir().unwrap();
        let ext_dir = ensure_external_dir(dir.path()).unwrap();
        let path = copy_to_external(&ext_dir, b"hello", "md").unwrap();
        assert!(path.exists());
        assert!(path.file_name().unwrap().to_string_lossy().ends_with(".md"));
        let stem = path.file_stem().unwrap().to_string_lossy();
        assert_eq!(stem.len(), 12);
    }

    #[test]
    fn copy_to_external_is_idempotent_for_same_bytes() {
        let dir = tempdir().unwrap();
        let ext_dir = ensure_external_dir(dir.path()).unwrap();
        let p1 = copy_to_external(&ext_dir, b"hello", "md").unwrap();
        let p2 = copy_to_external(&ext_dir, b"hello", "md").unwrap();
        assert_eq!(p1, p2);
    }

    #[test]
    fn copy_to_external_different_bytes_produce_different_filenames() {
        let dir = tempdir().unwrap();
        let ext_dir = ensure_external_dir(dir.path()).unwrap();
        let p1 = copy_to_external(&ext_dir, b"hello", "md").unwrap();
        let p2 = copy_to_external(&ext_dir, b"world", "md").unwrap();
        assert_ne!(p1, p2);
    }

    #[test]
    fn inject_frontmatter_basic() {
        let out = inject_frontmatter("## Body", "Article X", None).unwrap();
        assert!(out.starts_with("---\ntitle: \"Article X\"\n---\n\n## Body"));
    }

    #[test]
    fn inject_frontmatter_with_source_uri() {
        let out = inject_frontmatter("## Body", "X", Some("https://example.com/x")).unwrap();
        assert!(out.contains("title: \"X\""));
        assert!(out.contains("source_uri: \"https://example.com/x\""));
        assert!(out.contains("\n## Body"));
    }

    #[test]
    fn inject_frontmatter_errors_on_existing_frontmatter() {
        let body = "---\ntitle: Existing\n---\n\n## Body";
        let err = inject_frontmatter(body, "New", None).unwrap_err();
        assert!(err.to_string().contains("already has frontmatter"));
    }

    #[test]
    fn inject_frontmatter_errors_on_existing_frontmatter_crlf() {
        let body = "---\r\ntitle: Existing\r\n---\r\n\r\n## Body";
        let err = inject_frontmatter(body, "New", None).unwrap_err();
        assert!(err.to_string().contains("already has frontmatter"));
    }

    #[test]
    fn yaml_quote_escapes_quotes_and_backslashes() {
        assert_eq!(yaml_quote("hello \"world\""), "\"hello \\\"world\\\"\"");
        assert_eq!(yaml_quote("path\\to"), "\"path\\\\to\"");
        assert_eq!(yaml_quote("line\nbreak"), "\"line\\nbreak\"");
    }
}
