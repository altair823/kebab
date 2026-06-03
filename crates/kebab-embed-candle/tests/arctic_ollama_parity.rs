//! arctic-embed-l-v2.0 correctness gate (`#[ignore]` — needs the ~2GB candle
//! model + a live Ollama serving `snowflake-arctic-embed2`).
//!
//! This is the load-bearing pooling/prefix check for the arctic integration.
//! The recall measurement that justified adopting arctic (recall@10 130/132)
//! went through Ollama's `snowflake-arctic-embed2`. The candle path
//! re-implements the model (XLM-RoBERTa-large + **CLS** pooling + `query: ` on
//! queries / no prefix on documents). If candle's pooling or prefix is wrong,
//! its vectors silently diverge from the measured route and the 130 number
//! does NOT carry over. This test pins them together: per-sentence cosine
//! between the candle vector and the Ollama vector must be **> 0.99**.
//!
//! `#[ignore]` because it depends on an external Ollama daemon (CI is
//! headless/offline). The leader MUST run it once before merge.
//!
//! ## Manual run
//!
//! 1. Confirm Ollama is reachable and has the model:
//!    ```sh
//!    curl -s http://192.168.0.47:11434/api/tags        # should list snowflake-arctic-embed2
//!    ```
//! 2. Run (downloads the ~2GB candle safetensors on first run):
//!    ```sh
//!    CARGO_TARGET_DIR=/build/out/cargo-target \
//!    KEBAB_ARCTIC_OLLAMA_ENDPOINT=http://192.168.0.47:11434 \
//!    cargo test -p kebab-embed-candle --test arctic_ollama_parity -- --ignored --nocapture
//!    ```
//!    The endpoint defaults to `http://192.168.0.47:11434` if the env is unset.
//!
//! Record the printed `ARCTIC_PARITY_SUMMARY cosine_min=...` in
//! `/tmp/arctic-result.md` + `tasks/HOTFIXES.md`.

use kebab_config::Config;
use kebab_core::{Embedder, EmbeddingInput, EmbeddingKind};
use kebab_embed_candle::CandleEmbedder;
use kebab_embed_ollama::OllamaEmbedder;

const DOGFOOD_CONFIG: &str = "/build/dogfood/config.toml";
const DEFAULT_OLLAMA_ENDPOINT: &str = "http://192.168.0.47:11434";

/// Mixed Korean / English + the descriptive-recall shapes arctic was adopted
/// for (synonym / abbreviation / English term). Covers both prefix paths.
const SENTENCES: &[&str] = &[
    "스택 자료구조",
    "후입선출 방식으로 동작하는 자료구조",
    "큐는 선입선출 자료구조이다",
    "Rust ownership and the borrow checker",
    "소유권과 빌림 검사기는 메모리 안전성을 보장한다",
    "SVM 은 support vector machine 의 약자이다",
    "정렬 알고리즘의 시간 복잡도",
    "The capital of France is Paris.",
];

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    dot / (na * nb)
}

/// Base config: prefer the canonical dogfood config (for storage/cache roots),
/// fall back to `Config::defaults()` so the test still runs on a bare clone.
fn base_config() -> Config {
    Config::load(Some(std::path::Path::new(DOGFOOD_CONFIG))).unwrap_or_else(|_| Config::defaults())
}

#[test]
#[ignore = "needs ~2GB candle model + live Ollama (snowflake-arctic-embed2); run manually before merge"]
fn candle_arctic_matches_ollama_arctic() {
    let endpoint = std::env::var("KEBAB_ARCTIC_OLLAMA_ENDPOINT")
        .unwrap_or_else(|_| DEFAULT_OLLAMA_ENDPOINT.to_string());

    // candle side: the in-process arctic model.
    let mut candle_cfg = base_config();
    candle_cfg.models.embedding.provider = "candle".to_string();
    candle_cfg.models.embedding.model = "snowflake-arctic-embed-l-v2.0".to_string();
    candle_cfg.models.embedding.dimensions = 1024;

    // Ollama side: the reference route the recall numbers came from.
    let mut ollama_cfg = base_config();
    ollama_cfg.models.embedding.provider = "ollama".to_string();
    ollama_cfg.models.embedding.model = "snowflake-arctic-embed2".to_string();
    ollama_cfg.models.embedding.dimensions = 1024;
    ollama_cfg.models.embedding.endpoint = Some(endpoint.clone());

    let candle = CandleEmbedder::new(&candle_cfg).expect("build candle arctic embedder");
    let ollama = OllamaEmbedder::new(&ollama_cfg).expect("build ollama arctic embedder");

    // Exercise BOTH prefix paths so a query-side divergence can't hide.
    let inputs: Vec<EmbeddingInput> = SENTENCES
        .iter()
        .flat_map(|s| {
            [EmbeddingKind::Document, EmbeddingKind::Query]
                .into_iter()
                .map(move |kind| EmbeddingInput { text: s, kind })
        })
        .collect();

    let cv = candle.embed(&inputs).expect("candle embed");
    let ov = ollama
        .embed(&inputs)
        .expect("ollama embed (is snowflake-arctic-embed2 pulled @ the endpoint?)");

    assert_eq!(cv.len(), ov.len(), "embedding counts must match");
    assert_eq!(cv.len(), inputs.len(), "one vector per input");
    assert_eq!(candle.dimensions(), 1024);

    let mut min_cos = f32::INFINITY;
    for (i, inp) in inputs.iter().enumerate() {
        assert_eq!(cv[i].len(), 1024, "candle dim");
        assert_eq!(ov[i].len(), 1024, "ollama dim");
        let c = cosine(&cv[i], &ov[i]);
        min_cos = min_cos.min(c);
        let kind = match inp.kind {
            EmbeddingKind::Document => "doc",
            EmbeddingKind::Query => "qry",
        };
        let preview: String = inp.text.chars().take(36).collect();
        println!("[{i:>2}] {kind} cos={c:.6}  {preview}");
    }

    println!("ARCTIC_PARITY_SUMMARY cosine_min={min_cos:.6} endpoint={endpoint}");
    assert!(
        min_cos > 0.99,
        "candle arctic vs Ollama arctic cosine_min={min_cos:.6} ≤ 0.99 — \
         pooling/prefix mismatch; the recall=130 measurement will NOT reproduce"
    );
}
