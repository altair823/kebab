//! p9-fb-25 task 3: `kebab init` produces config.toml with a header
//! comment listing the four supported extensions (md / png / jpg+jpeg
//! / pdf) so a user editing the config knows what's processable.

#[test]
fn init_workspace_header_lists_supported_extensions() {
    let tmp = tempfile::tempdir().expect("tempdir");
    // SAFETY: Rust 2024 marks set_var as unsafe — wrap in unsafe block.
    // Each test sets process-wide XDG_CONFIG_HOME to point at the
    // tempdir; init_workspace writes config.toml relative to it.
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", tmp.path());
        // Same dir for data + cache to avoid touching real user paths.
        std::env::set_var("XDG_DATA_HOME", tmp.path().join("data"));
        std::env::set_var("XDG_CACHE_HOME", tmp.path().join("cache"));
        std::env::set_var("XDG_STATE_HOME", tmp.path().join("state"));
    }
    kebab_app::init_workspace(true).expect("init_workspace");
    let cfg_path = kebab_config::Config::xdg_config_path();
    let body = std::fs::read_to_string(&cfg_path)
        .unwrap_or_else(|e| panic!("read config at {}: {e}", cfg_path.display()));
    assert!(
        body.contains("처리 가능한 형식"),
        "header lists supported types section: body=\n{body}"
    );
    assert!(body.contains("Markdown: .md"), "md listed");
    assert!(body.contains(".png .jpg .jpeg"), "image extensions listed");
    assert!(body.contains("PDF:      .pdf"), "pdf listed");
    assert!(
        !body.contains("workspace.include"),
        "no leftover include reference"
    );
}
