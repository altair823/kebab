//! Path / string normalization helpers (§4.1, §6.6).

use std::path::{Component, Path};

use unicode_normalization::UnicodeNormalization;

use crate::asset::WorkspacePath;
use crate::errors::CoreError;

/// NFC-normalize a UTF-8 string (§4.1).
pub fn nfc(input: &str) -> String {
    input.nfc().collect()
}

/// Collapse a path to a POSIX-relative `WorkspacePath` per §6.6:
/// - convert all separators to `/`
/// - strip a leading `./`
/// - collapse repeated slashes
/// - NFC-normalize
///
/// Returns `Err(CoreError::Malformed(..))` if the resulting POSIX form
/// contains `#`, since `WorkspacePath` is forbidden from colliding with
/// the W3C-Media-Fragments separator that `Citation` URIs depend on.
pub fn to_posix(path: &Path) -> Result<WorkspacePath, CoreError> {
    let mut out = String::new();
    let mut first = true;
    for comp in path.components() {
        match comp {
            Component::CurDir => continue,
            Component::Normal(s) => {
                if !first {
                    out.push('/');
                }
                out.push_str(&s.to_string_lossy());
                first = false;
            }
            Component::ParentDir => {
                if !first {
                    out.push('/');
                }
                out.push_str("..");
                first = false;
            }
            Component::RootDir => {
                if first {
                    out.push('/');
                }
                first = false;
            }
            Component::Prefix(_) => {
                // Windows drive prefixes — `to_string_lossy` keeps form.
                out.push_str(&comp.as_os_str().to_string_lossy());
                first = false;
            }
        }
    }
    if out.is_empty() {
        out.push('.');
    }
    WorkspacePath::new(nfc(&out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collapses_curdir_and_redundant_slashes() {
        let p = Path::new("./a//b.md");
        // `Path::components` already collapses `//` on POSIX; the test
        // doc-fixed example asserts the final string is `a/b.md`.
        assert_eq!(to_posix(p).unwrap().0, "a/b.md");
    }

    #[test]
    fn nfc_normalizes_korean() {
        // U+1100 ㄱ + U+1161 ㅏ (NFD) vs U+AC00 가 (NFC). After NFC they
        // collapse to the same string; `to_posix` runs NFC after path
        // collapse, so the WorkspacePath comes out NFC regardless of input.
        let nfd = "\u{1100}\u{1161}.md";
        let nfc_str = "\u{AC00}.md";
        assert_eq!(
            to_posix(Path::new(nfd)).unwrap().0,
            to_posix(Path::new(nfc_str)).unwrap().0
        );
        assert_eq!(to_posix(Path::new(nfd)).unwrap().0, "\u{AC00}.md");
    }

    #[test]
    fn nfc_function_idempotent() {
        let s = "\u{AC00}";
        assert_eq!(nfc(s), s);
    }

    #[test]
    fn to_posix_rejects_hash_in_path() {
        // `#` collides with the W3C-Media-Fragments separator used by
        // `Citation`; the WorkspacePath invariant rejects it at construction.
        let p = Path::new("notes/has#hash.md");
        let err = to_posix(p).expect_err("# in path must be rejected");
        let msg = format!("{err}");
        assert!(msg.contains('#'), "error message should mention '#': {msg}");
    }
}
