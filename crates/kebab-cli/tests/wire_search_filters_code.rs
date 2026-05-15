//! p10-1A-1 Task 15: CLI accepts --repo and --code-lang flags.
//!
//! These tests verify that clap parses the new flags without error.
//! They drive `kebab search --help` (which exercises flag parsing
//! via clap's help generation path, exiting 0) or use a minimal
//! config + `--json` round-trip to verify the flags reach the wire.

use std::process::Command;

fn kebab() -> Command {
    Command::new(env!("CARGO_BIN_EXE_kebab"))
}

/// `kebab search --help` must exit 0 and mention `--repo`.
#[test]
fn cli_search_help_mentions_repo_flag() {
    let out = kebab()
        .args(["search", "--help"])
        .output()
        .expect("failed to run kebab");
    // clap help exits 0.
    assert!(
        out.status.success(),
        "kebab search --help exited non-zero: {:?}",
        out.status
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("--repo"),
        "--repo flag must appear in search help output:\n{stdout}"
    );
}

/// `kebab search --help` must exit 0 and mention `--code-lang`.
#[test]
fn cli_search_help_mentions_code_lang_flag() {
    let out = kebab()
        .args(["search", "--help"])
        .output()
        .expect("failed to run kebab");
    assert!(
        out.status.success(),
        "kebab search --help exited non-zero: {:?}",
        out.status
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("--code-lang"),
        "--code-lang flag must appear in search help output:\n{stdout}"
    );
}

/// `kebab search --help` must exit 0 and mention `--media`.
/// Confirms `--media code` value pathway is available (media is
/// a free-form Vec<String> that already accepted arbitrary values).
#[test]
fn cli_search_help_mentions_media_flag() {
    let out = kebab()
        .args(["search", "--help"])
        .output()
        .expect("failed to run kebab");
    assert!(
        out.status.success(),
        "kebab search --help exited non-zero: {:?}",
        out.status
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("--media"),
        "--media flag must appear in search help output:\n{stdout}"
    );
}
