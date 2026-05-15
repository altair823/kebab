use kebab_parse_code::code_lang_for_path;
use std::path::Path;

#[test]
fn known_extensions_map_to_canonical_identifiers() {
    let cases = [
        ("foo.rs", Some("rust")),
        ("foo.py", Some("python")),
        ("foo.pyi", Some("python")),
        ("foo.ts", Some("typescript")),
        ("foo.tsx", Some("typescript")),
        ("foo.js", Some("javascript")),
        ("foo.mjs", Some("javascript")),
        ("foo.cjs", Some("javascript")),
        ("foo.jsx", Some("javascript")),
        ("foo.go", Some("go")),
        ("foo.java", Some("java")),
        ("foo.kt", Some("kotlin")),
        ("foo.kts", Some("kotlin")),
        ("foo.c", Some("c")),
        ("foo.h", Some("c")),
        ("foo.cpp", Some("cpp")),
        ("foo.cc", Some("cpp")),
        ("foo.cxx", Some("cpp")),
        ("foo.hpp", Some("cpp")),
        ("foo.hh", Some("cpp")),
        ("foo.hxx", Some("cpp")),
        ("foo.yaml", Some("yaml")),
        ("foo.yml", Some("yaml")),
        ("foo.toml", Some("toml")),
        ("foo.json", Some("json")),
        ("foo.sh", Some("shell")),
        ("foo.bash", Some("shell")),
        ("foo.zsh", Some("shell")),
        ("foo.mk", Some("make")),
    ];
    for (path, expected) in cases {
        assert_eq!(
            code_lang_for_path(Path::new(path)),
            expected,
            "path = {path}"
        );
    }
}

#[test]
fn special_filenames_map_to_identifiers() {
    assert_eq!(code_lang_for_path(Path::new("Dockerfile")), Some("dockerfile"));
    assert_eq!(code_lang_for_path(Path::new("foo.dockerfile")), Some("dockerfile"));
    assert_eq!(code_lang_for_path(Path::new("Makefile")), Some("make"));
}

#[test]
fn unknown_extension_returns_none() {
    assert_eq!(code_lang_for_path(Path::new("foo.docx")), None);
    assert_eq!(code_lang_for_path(Path::new("foo")), None);
    assert_eq!(code_lang_for_path(Path::new("foo.unknown")), None);
}

#[test]
fn case_insensitive() {
    assert_eq!(code_lang_for_path(Path::new("Foo.RS")), Some("rust"));
    assert_eq!(code_lang_for_path(Path::new("FOO.YAML")), Some("yaml"));
}
