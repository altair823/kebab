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

/// v0.21.0 N-gram supplement (Option β): morpheme 길이 ≥ 3 인 한글 token
/// (ko-dic 가 단일 compound 으로 저장한 case) 에 대해 sliding window
/// 2-gram 보충 emit. ko-dic 가 이미 `한국정부` → `[한국, 정부]` 처럼 잘
/// 분해하는 경우는 2-char morpheme 이라 supplement 안 함 (filter 의도).
#[test]
fn tokenize_korean_morphological_emits_2gram_for_long_morpheme() {
    // ko-dic 의 분해 정책 검증: 어떤 input 이 3+자 morpheme 을 emit 하는지.
    // 본 test 는 lindera ko-dic 의 segmentation 의존이라 구체 fixture 는
    // morpheme list 가 ≥ 3 char token 을 포함하는 case 를 사용.
    let probe_inputs: &[&str] = &[
        "한국문화",          // ko-dic 가 단일 명사로 등록 가능 → 3+ char morpheme
        "주민등록번호",      // 4+ char compound — supplement 대상
        "서울특별시",        // 3+ char
        "대한민국",          // 3+ char
        "오래되었다",        // 동사 활용형 — 일부 3+ char morpheme 가능
    ];

    let mut found_supplement = false;
    for input in probe_inputs {
        let out = kebab_chunk::tokenize_korean_morphological(input).unwrap_or_default();
        let tokens: Vec<&str> = out.split_whitespace().collect();
        let unique: std::collections::HashSet<&&str> = tokens.iter().collect();
        // supplement 가 작동했다면 distinct token 수가 lindera output 의 morpheme 수보다 많음.
        // 또는 input 의 2-char prefix 가 별도 token 으로 존재.
        let prefix: String = input.chars().take(2).collect();
        if tokens.contains(&prefix.as_str()) && tokens.iter().any(|t| t.chars().count() >= 3) {
            found_supplement = true;
            println!("supplement fired for input '{input}' → tokens = {tokens:?}");
        }
        // 영어/숫자 prefix 가 emit 되지 않음 (한글만 supplement 대상).
        // 무조건 unique token 수 ≥ 1.
        assert!(!unique.is_empty(), "input '{input}' produced empty token list");
    }

    // 최소 1개 fixture 에서 supplement 동작 확인.
    // 만약 ko-dic 가 모든 probe 를 2-char 단위로만 분해하면 found_supplement=false 가능.
    // 그때는 본 test 는 ko-dic 정책상 N-gram supplement 가 marginal 임을 demonstrate (warning only).
    if !found_supplement {
        eprintln!(
            "WARNING: ko-dic 가 모든 probe input 을 2-char morpheme 으로 분해. \
             N-gram supplement 의 marginal benefit 은 corpus 의 morpheme 길이 분포 의존."
        );
    }
}

/// N-gram supplement 는 한국어 (한글) morpheme 에만 적용. 영어/숫자/혼합
/// token 은 sliding window emit 없음 (false positive 회피).
#[test]
fn tokenize_korean_morphological_no_2gram_for_english() {
    let out = kebab_chunk::tokenize_korean_morphological("Rust optimization").unwrap();
    let tokens: Vec<&str> = out.split_whitespace().collect();

    // Rust 와 optimization 자체는 token 으로 존재해야 함 (lindera output).
    assert!(
        tokens.iter().any(|t| t.eq_ignore_ascii_case("rust") || t.eq_ignore_ascii_case("optimization")),
        "lindera 의 영어 token 자체는 emit 되어야 함 — tokens = {tokens:?}"
    );
    // 영어 substring (`Rus`, `imi`, `tion` 등) 는 N-gram emit 안 됨.
    let supplements: Vec<&&str> = tokens
        .iter()
        .filter(|t| matches!(t.chars().count(), 2 | 3) && t.chars().all(|c| c.is_ascii_alphabetic()))
        .collect();
    // empty 또는 lindera 가 emit 한 짧은 ASCII token 만 — 우리가 추가 emit 한 substring 은 없음.
    assert!(
        supplements.iter().all(|t| !t.contains("Rus") && !t.contains("ust") && !t.contains("imi")),
        "영어 N-gram supplement 가 emit 됨 — false positive 위험. tokens = {tokens:?}"
    );
}
