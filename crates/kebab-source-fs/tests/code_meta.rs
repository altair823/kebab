use kebab_source_fs::BUILTIN_BLACKLIST;

#[test]
fn builtin_blacklist_has_exactly_six_entries() {
    assert_eq!(BUILTIN_BLACKLIST.len(), 6);
    let expected = [
        "**/node_modules/**",
        "**/target/**",
        "**/__pycache__/**",
        "**/.venv/**",
        "**/venv/**",
        "**/env/**",
    ];
    for pat in expected {
        assert!(BUILTIN_BLACKLIST.contains(&pat), "missing pattern: {pat}");
    }
}
