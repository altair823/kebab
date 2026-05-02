//! Integration tests for `MockLanguageModel`. Gated behind the `mock` feature.
//!
//! Canonical invocation: `cargo test -p kb-llm --features mock`.

#![cfg(feature = "mock")]

use kebab_llm::{
    FinishReason, GenerateRequest, LanguageModel, MockLanguageModel, TokenChunk, TokenUsage,
    assert_finish_chunk,
};
use proptest::prelude::*;

fn usage() -> TokenUsage {
    TokenUsage {
        prompt_tokens: 10,
        completion_tokens: 20,
        latency_ms: 30,
    }
}

fn req_with_stop(stop: Vec<&str>) -> GenerateRequest {
    GenerateRequest {
        system: "sys".into(),
        user: "usr".into(),
        stop: stop.into_iter().map(String::from).collect(),
        max_tokens: 64,
        temperature: 0.0,
        seed: None,
        images: Vec::new(),
    }
}

fn mk(canned: &str, finish: FinishReason) -> MockLanguageModel {
    MockLanguageModel {
        model_id: "mock-test".into(),
        provider: "mock".into(),
        context_tokens: 4096,
        canned_response: canned.into(),
        canned_finish: finish,
        canned_usage: usage(),
    }
}

fn drain(m: &dyn LanguageModel, req: GenerateRequest) -> Vec<TokenChunk> {
    m.generate_stream(req)
        .expect("generate_stream")
        .map(|r| r.expect("ok chunk"))
        .collect()
}

#[test]
fn streams_then_done() {
    let m = mk("hello", FinishReason::Stop);
    let chunks = drain(&m, req_with_stop(vec![]));

    // 5 Token chunks ("h", "e", "l", "l", "o") + Done.
    assert_eq!(chunks.len(), 6);
    assert_finish_chunk(&chunks);

    let tokens: Vec<&str> = chunks
        .iter()
        .filter_map(|c| match c {
            TokenChunk::Token(s) => Some(s.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(tokens, vec!["h", "e", "l", "l", "o"]);

    match chunks.last().unwrap() {
        TokenChunk::Done {
            finish_reason,
            usage: u,
        } => {
            assert_eq!(*finish_reason, FinishReason::Stop);
            assert_eq!(*u, usage());
        }
        _ => unreachable!(),
    }
}

#[test]
fn honors_stop_strings() {
    // canned has "STOP" embedded; req.stop=["STOP"] truncates before it.
    let m = mk("abc STOP defg", FinishReason::Length);
    let chunks = drain(&m, req_with_stop(vec!["STOP"]));

    let concat: String = chunks
        .iter()
        .filter_map(|c| match c {
            TokenChunk::Token(s) => Some(s.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(concat, "abc ");

    // Stop-string truncation forces FinishReason::Stop, overriding the
    // configured `canned_finish` (Length here).
    match chunks.last().unwrap() {
        TokenChunk::Done { finish_reason, .. } => {
            assert_eq!(*finish_reason, FinishReason::Stop);
        }
        _ => panic!("last chunk must be Done"),
    }
}

#[test]
fn honors_first_stop_match() {
    // Two stop strings; "BAR" appears at byte 4, "FOO" at byte 12. Earliest
    // wins regardless of order in req.stop.
    let m = mk("abc BAR xyz FOO end", FinishReason::Stop);
    let chunks = drain(&m, req_with_stop(vec!["FOO", "BAR"]));

    let concat: String = chunks
        .iter()
        .filter_map(|c| match c {
            TokenChunk::Token(s) => Some(s.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(concat, "abc ");
}

#[test]
fn dyn_dispatch_via_box() {
    let m: Box<dyn LanguageModel> = Box::new(mk("xy", FinishReason::Stop));
    assert_eq!(m.model_ref().id, "mock-test");
    assert_eq!(m.model_ref().provider, "mock");
    assert!(m.model_ref().dimensions.is_none());
    assert_eq!(m.context_tokens(), 4096);

    let chunks: Vec<TokenChunk> = m
        .generate_stream(req_with_stop(vec![]))
        .expect("stream")
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(chunks.len(), 3); // x, y, Done
    assert_finish_chunk(&chunks);
}

#[test]
fn concat_equals_canned() {
    let canned = "the quick brown fox";
    let m = mk(canned, FinishReason::Stop);
    let chunks = drain(&m, req_with_stop(vec![]));
    let concat: String = chunks
        .iter()
        .filter_map(|c| match c {
            TokenChunk::Token(s) => Some(s.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(concat, canned);
}

#[test]
fn model_ref_has_no_dimensions() {
    let m = mk("anything", FinishReason::Stop);
    let r = m.model_ref();
    assert_eq!(r.id, "mock-test");
    assert_eq!(r.provider, "mock");
    assert!(r.dimensions.is_none());
}

#[test]
fn finish_reason_passes_through_when_no_stop_match() {
    // No stop hit → `canned_finish` is preserved verbatim.
    let m = mk("hi", FinishReason::Length);
    let chunks = drain(&m, req_with_stop(vec!["NEVER_MATCHES"]));
    match chunks.last().unwrap() {
        TokenChunk::Done { finish_reason, .. } => {
            assert_eq!(*finish_reason, FinishReason::Length);
        }
        _ => panic!("last chunk must be Done"),
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 100,
        ..ProptestConfig::default()
    })]

    /// 100 random Unicode canned strings: with no stop strings configured,
    /// the stream MUST end in Done, contain exactly `canned.chars().count()`
    /// Token chunks, and concatenate back to the canned text byte-equal.
    #[test]
    fn proptest_random_canned_strings(canned in ".{0,256}") {
        let m = mk(&canned, FinishReason::Stop);
        let chunks = drain(&m, req_with_stop(vec![]));

        // Last chunk must be Done.
        assert_finish_chunk(&chunks);

        // Token-chunk count == canned.chars().count().
        let token_count = chunks
            .iter()
            .filter(|c| matches!(c, TokenChunk::Token(_)))
            .count();
        prop_assert_eq!(token_count, canned.chars().count());

        // Concatenation == canned (byte-equal).
        let concat: String = chunks
            .iter()
            .filter_map(|c| match c {
                TokenChunk::Token(s) => Some(s.as_str()),
                _ => None,
            })
            .collect();
        prop_assert_eq!(concat, canned);
    }
}
