// crates/kebab-cli/tests/cli_help_smoke.rs
//
// Regression pin — `kebab search --help` 의 `--media` value list 가
// `code` 를 노출. Bug #7 (v0.20.0 bugfix round 2 spec §4.4).

#[test]
fn search_help_lists_code_in_media_values() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_kebab"))
        .args(["search", "--help"])
        .output()
        .expect("kebab search --help");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("`code`"),
        "search --help must list 'code' as accepted --media value; stdout = {stdout}"
    );
}
