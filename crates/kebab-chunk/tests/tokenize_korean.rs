#[test]
fn tokenize_korean_morphological_splits_2char_word() {
    let out = kebab_chunk::tokenize_korean_morphological("한국 문화는 오래되었다").unwrap();
    let tokens: Vec<&str> = out.split_whitespace().collect();
    assert!(tokens.contains(&"한국"), "tokens = {tokens:?}");
}

#[test]
fn tokenize_korean_morphological_empty_returns_none() {
    assert!(kebab_chunk::tokenize_korean_morphological("").is_none());
    assert!(kebab_chunk::tokenize_korean_morphological("   ").is_none());
}
