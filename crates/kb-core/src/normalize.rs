//! Path / string normalization helpers (§4.1, §6.6).

use std::path::{Component, Path};

use unicode_normalization::UnicodeNormalization;

use crate::asset::WorkspacePath;

/// NFC-normalize a UTF-8 string (§4.1).
pub fn nfc(input: &str) -> String {
    input.nfc().collect()
}

/// Collapse a path to a POSIX-relative `WorkspacePath` per §6.6:
/// - convert all separators to `/`
/// - strip a leading `./`
/// - collapse repeated slashes
/// - NFC-normalize
pub fn to_posix(path: &Path) -> WorkspacePath {
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
    WorkspacePath(nfc(&out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collapses_curdir_and_redundant_slashes() {
        let p = Path::new("./a//b.md");
        // `Path::components` already collapses `//` on POSIX; the test
        // doc-fixed example asserts the final string is `a/b.md`.
        assert_eq!(to_posix(p).0, "a/b.md");
    }

    #[test]
    fn nfc_normalizes_korean() {
        // U+1100 ㄱ + U+1161 ㅏ (NFD) vs U+AC00 가 (NFC). After NFC they
        // collapse to the same string; `to_posix` runs NFC after path
        // collapse, so the WorkspacePath comes out NFC regardless of input.
        let nfd = "\u{1100}\u{1161}.md";
        let nfc_str = "\u{AC00}.md";
        assert_eq!(to_posix(Path::new(nfd)).0, to_posix(Path::new(nfc_str)).0);
        assert_eq!(to_posix(Path::new(nfd)).0, "\u{AC00}.md");
    }

    #[test]
    fn nfc_function_idempotent() {
        let s = "\u{AC00}";
        assert_eq!(nfc(s), s);
    }
}
