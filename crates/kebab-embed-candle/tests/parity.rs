//! Parity test (spec §7, `#[ignore]` — needs the ~2GB model + network).
//!
//! Confirms the candle backend reproduces the onnxruntime `FastembedEmbedder`
//! vectors closely enough that no re-index is required (spec D-reindex):
//! per-sentence cosine ≥ 0.9999, and reports the dimension-wise max absolute
//! difference (the number the re-index decision hangs on).
//!
//! Run manually:
//!   CARGO_TARGET_DIR=/build/out/cargo-target/target \
//!   cargo test -p kebab-embed-candle --release -- --ignored --nocapture
//!
//! Uses the canonical dogfood config so both backends resolve the same model
//! identifiers and cache roots.

use kebab_config::Config;
use kebab_core::{Embedder, EmbeddingInput, EmbeddingKind};
use kebab_embed_candle::CandleEmbedder;
use kebab_embed_local::FastembedEmbedder;

const DOGFOOD_CONFIG: &str = "/build/dogfood/config.toml";

/// Mixed Korean / English parity set (≥ 8 sentences, mirrors the Phase 0 spike).
const SENTENCES: &[&str] = &[
    "The quick brown fox jumps over the lazy dog.",
    "오늘 날씨가 정말 좋아서 산책을 나가고 싶다.",
    "Rust is a systems programming language focused on safety and performance.",
    "벡터 검색은 임베딩 사이의 코사인 유사도를 이용한다.",
    "Machine learning models require large amounts of training data.",
    "한국어와 영어가 섞인 문장도 멀티링구얼 모델은 잘 처리한다.",
    "The capital of France is Paris, a city known for its art and culture.",
    "이 프로젝트는 로컬 우선 지식 베이스와 검색 증강 생성을 목표로 한다.",
    "Database indexing dramatically speeds up query performance.",
    "임베딩 모델을 candle 로 옮기면 NUMA 서버에서 안전하게 돌릴 수 있다.",
];

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    dot / (na * nb)
}

#[test]
#[ignore = "needs ~2GB model + network; run manually for the re-index decision"]
fn candle_matches_fastembed() {
    let config = Config::load(Some(std::path::Path::new(DOGFOOD_CONFIG)))
        .expect("load dogfood config for parity baseline");

    let candle = CandleEmbedder::new(&config).expect("build CandleEmbedder");
    let fastembed = FastembedEmbedder::new(&config).expect("build FastembedEmbedder");

    // Cover BOTH prefix paths (`passage:` for Document, `query:` for Query) so
    // a query-side prefix/pooling divergence can't slip through (reviewer note).
    let inputs: Vec<EmbeddingInput> = SENTENCES
        .iter()
        .flat_map(|s| {
            [EmbeddingKind::Document, EmbeddingKind::Query]
                .into_iter()
                .map(move |kind| EmbeddingInput { text: s, kind })
        })
        .collect();

    let cv = candle.embed(&inputs).expect("candle embed");
    let fv = fastembed.embed(&inputs).expect("fastembed embed");

    assert_eq!(cv.len(), fv.len(), "embedding counts must match");
    assert_eq!(cv.len(), inputs.len(), "one vector per input");
    assert_eq!(candle.dimensions(), 1024);

    let mut min_cos = f32::INFINITY;
    let mut max_abs_diff = 0f32;
    for (i, inp) in inputs.iter().enumerate() {
        assert_eq!(cv[i].len(), 1024, "candle dim");
        assert_eq!(fv[i].len(), 1024, "fastembed dim");
        let c = cosine(&cv[i], &fv[i]);
        min_cos = min_cos.min(c);
        let diff = cv[i]
            .iter()
            .zip(&fv[i])
            .map(|(a, b)| (a - b).abs())
            .fold(0f32, f32::max);
        max_abs_diff = max_abs_diff.max(diff);
        let kind = match inp.kind {
            EmbeddingKind::Document => "doc",
            EmbeddingKind::Query => "qry",
        };
        let preview: String = inp.text.chars().take(36).collect();
        println!("[{i:>2}] {kind} cos={c:.6} max_abs_diff={diff:.6e}  {preview}");
    }

    println!("PARITY_SUMMARY cosine_min={min_cos:.6} max_abs_diff={max_abs_diff:.6e}");
    assert!(
        min_cos >= 0.9999,
        "candle vs fastembed cosine_min={min_cos:.6} < 0.9999 — investigate before merge"
    );
}
