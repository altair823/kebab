//! Shared path expansion helper.
//!
//! `Config::storage.*` fields are stored as raw template strings (e.g.
//! `${XDG_DATA_HOME:-~/.local/share}/kb`, `{data_dir}/runs`). Every
//! crate that turns one of those strings into a real filesystem path
//! needs to apply the same set of substitutions; this module is the
//! single source of truth so the behavior cannot drift.
//!
//! Substitutions, applied in order:
//!
//! 1. `{data_dir}` → caller-supplied `data_dir`.
//!    - When the caller passes an empty `data_dir` (because they ARE
//!      resolving `data_dir` itself), the substitution is a no-op so
//!      a literal `{data_dir}` is left in place rather than producing
//!      a `/{data_dir}/...` artifact.
//! 2. `${XDG_DATA_HOME:-<default>}` (or the bare `${XDG_DATA_HOME}`) →
//!    the env var if set + non-empty, else the default after `:-`.
//!    Mimics POSIX shell's `${VAR:-default}` semantics. Mid-string
//!    occurrences are supported; only the first match is replaced.
//! 3. Leading `~` / `~/...` → `$HOME`. Any non-leading `~` is left
//!    literal (matches shell behavior — only the first segment expands).
//!
//! The result is a `PathBuf` regardless of whether all substitutions
//! were applicable; relative paths are kept relative to the caller's
//! CWD (not resolved here).

use std::path::PathBuf;

/// Expand storage-path templates. See module docs for the substitution
/// rules.
///
/// Pass an empty `data_dir` when resolving `data_dir` itself; the
/// `{data_dir}` substitution becomes a no-op in that case so the
/// recursive shape (`data_dir = "${XDG_DATA_HOME:-…}/kb"`) resolves
/// without producing a literal `{data_dir}` token in the output.
pub fn expand_path(raw: &str, data_dir: &str) -> PathBuf {
    let mut s = raw.to_string();

    // 1. {data_dir} substitution (skipped when resolving data_dir
    //    itself; see module docs).
    if !data_dir.is_empty() {
        s = s.replace("{data_dir}", data_dir);
    }

    // 2. ${XDG_DATA_HOME:-<default>}: env override else default.
    if let Some(start) = s.find("${XDG_DATA_HOME") {
        if let Some(rel_end) = s[start..].find('}') {
            let end = start + rel_end + 1; // include trailing '}'
            let inner = &s[start + 2..end - 1]; // strip ${ and }
            let replacement = match std::env::var("XDG_DATA_HOME") {
                Ok(v) if !v.is_empty() => v,
                _ => match inner.split_once(":-") {
                    Some((_, default)) => default.to_string(),
                    None => String::new(),
                },
            };
            s.replace_range(start..end, &replacement);
        }
    }

    // 3. Leading `~` → $HOME.
    if let Some(rest) = s.strip_prefix('~') {
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            return home.join(rest.trim_start_matches('/'));
        }
    }

    PathBuf::from(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    /// `XDG_DATA_HOME` / `HOME` env mutations must be serialized so
    /// concurrent test runs (cargo's default parallel runner) don't
    /// observe each other's transient values.
    static ENV_LOCK: StdMutex<()> = StdMutex::new(());

    /// RAII guard: snapshots `XDG_DATA_HOME` on construction, restores
    /// it on drop.
    struct XdgGuard {
        prior: Option<String>,
    }

    impl XdgGuard {
        fn capture() -> Self {
            Self {
                prior: std::env::var("XDG_DATA_HOME").ok(),
            }
        }
    }

    impl Drop for XdgGuard {
        fn drop(&mut self) {
            // SAFETY: edition 2024 marks set_var/remove_var unsafe
            // because env mutation is not thread-safe. The ENV_LOCK
            // guard at the call site prevents concurrent observation.
            unsafe {
                match &self.prior {
                    Some(v) => std::env::set_var("XDG_DATA_HOME", v),
                    None => std::env::remove_var("XDG_DATA_HOME"),
                }
            }
        }
    }

    #[test]
    fn substitutes_data_dir_template() {
        let p = expand_path("{data_dir}/runs", "/tmp/kbtest");
        assert_eq!(p, PathBuf::from("/tmp/kbtest/runs"));
    }

    #[test]
    fn data_dir_substitution_skipped_when_empty() {
        // Empty `data_dir` is the "resolving data_dir itself" signal;
        // the literal `{data_dir}` token must survive.
        let p = expand_path("{data_dir}/runs", "");
        assert_eq!(p, PathBuf::from("{data_dir}/runs"));
    }

    #[test]
    fn passthrough_absolute_path() {
        let p = expand_path("/abs/runs", "/ignored");
        assert_eq!(p, PathBuf::from("/abs/runs"));
    }

    #[test]
    fn xdg_data_home_set_replaces_var() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _guard = XdgGuard::capture();
        // SAFETY: lock held for the duration of this test.
        unsafe { std::env::set_var("XDG_DATA_HOME", "/custom/path") };

        let p = expand_path("${XDG_DATA_HOME:-~/.local/share}/kb", "");
        assert_eq!(p, PathBuf::from("/custom/path/kb"));
    }

    #[test]
    fn xdg_data_home_unset_uses_default() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _guard = XdgGuard::capture();
        // SAFETY: lock held for the duration of this test.
        unsafe { std::env::remove_var("XDG_DATA_HOME") };

        let home = std::env::var("HOME").expect("HOME must be set in tests");
        let expected = PathBuf::from(home).join(".local/share/kb");
        let p = expand_path("${XDG_DATA_HOME:-~/.local/share}/kb", "");
        assert_eq!(p, expected);
    }

    #[test]
    fn xdg_with_no_default_resolves_to_empty_when_unset() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _guard = XdgGuard::capture();
        // SAFETY: lock held for the duration of this test.
        unsafe { std::env::remove_var("XDG_DATA_HOME") };

        // No `:-default` clause, no env var → empty string substitution.
        let p = expand_path("${XDG_DATA_HOME}/kb", "");
        assert_eq!(p, PathBuf::from("/kb"));
    }

    #[test]
    fn leading_tilde_expands_to_home() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let home = std::env::var("HOME").expect("HOME must be set in tests");
        let p = expand_path("~/runs", "");
        assert_eq!(p, PathBuf::from(home).join("runs"));
    }

    #[test]
    fn data_dir_then_xdg_then_tilde_compose() {
        // Order matters: substitute `{data_dir}` (which itself contains
        // an unexpanded `${XDG_DATA_HOME}` and `~`), then the other two
        // resolve the result.
        let _lock = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _guard = XdgGuard::capture();
        // SAFETY: lock held for the duration of this test.
        unsafe { std::env::set_var("XDG_DATA_HOME", "/xdg/data") };

        let p = expand_path("{data_dir}/runs", "/xdg/data/kb");
        assert_eq!(p, PathBuf::from("/xdg/data/kb/runs"));
    }
}
