//! `/api/embed` behavior against a `wiremock`-hosted mock server.
//!
//! `wiremock` is async, so the tests are `#[tokio::test]`; the sync
//! [`OllamaEmbedder`] is driven from `spawn_blocking` to keep `reqwest::blocking`
//! off the async runtime (same pattern as `kebab-llm-local`'s streaming tests).
//! tokio is a `dev-dependency` only.

use kebab_config::Config;
use kebab_core::{Embedder, EmbeddingInput, EmbeddingKind};
use kebab_embed_ollama::OllamaEmbedder;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Config pointing at the mock server, with a small dim so the mock body is
/// tiny. `model` is an arctic tag so prefix resolution is exercised.
fn cfg_for(endpoint: &str, dim: usize) -> Config {
    let mut cfg = Config::defaults();
    cfg.models.embedding.provider = "ollama".to_string();
    cfg.models.embedding.model = "snowflake-arctic-embed2".to_string();
    cfg.models.embedding.dimensions = dim;
    cfg.models.embedding.endpoint = Some(endpoint.to_string());
    cfg
}

async fn embed_blocking(
    cfg: Config,
    inputs: Vec<(String, EmbeddingKind)>,
) -> anyhow::Result<Vec<Vec<f32>>> {
    tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<Vec<f32>>> {
        let emb = OllamaEmbedder::new(&cfg)?;
        let refs: Vec<EmbeddingInput<'_>> = inputs
            .iter()
            .map(|(t, k)| EmbeddingInput { text: t, kind: *k })
            .collect();
        emb.embed(&refs)
    })
    .await
    .expect("blocking task panicked")
}

#[tokio::test]
async fn embed_returns_l2_normalized_vectors() {
    let server = MockServer::start().await;
    // Two raw (un-normalized) vectors of dim 2; the adapter must L2-normalize.
    let body = r#"{"model":"snowflake-arctic-embed2","embeddings":[[3.0,4.0],[0.0,5.0]]}"#;
    Mock::given(method("POST"))
        .and(path("/api/embed"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(&server)
        .await;

    let out = embed_blocking(
        cfg_for(&server.uri(), 2),
        vec![
            ("스택 자료구조".to_string(), EmbeddingKind::Query),
            ("후입선출".to_string(), EmbeddingKind::Document),
        ],
    )
    .await
    .expect("embed should succeed");

    assert_eq!(out.len(), 2);
    for v in &out {
        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5, "expected unit norm, got {norm}");
    }
    // [3,4] → [0.6, 0.8].
    assert!((out[0][0] - 0.6).abs() < 1e-5 && (out[0][1] - 0.8).abs() < 1e-5);
}

#[tokio::test]
async fn embed_rejects_dim_mismatch() {
    let server = MockServer::start().await;
    // Server returns dim 3, config expects dim 2 → hard error.
    let body = r#"{"model":"snowflake-arctic-embed2","embeddings":[[1.0,2.0,3.0]]}"#;
    Mock::given(method("POST"))
        .and(path("/api/embed"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(&server)
        .await;

    let err = embed_blocking(
        cfg_for(&server.uri(), 2),
        vec![("q".to_string(), EmbeddingKind::Query)],
    )
    .await
    .expect_err("dim mismatch must error");
    let msg = format!("{err:#}");
    assert!(msg.contains("dim"), "expected dim error, got: {msg}");
}

#[tokio::test]
async fn embed_empty_input_is_noop() {
    // No mock needed — empty input must never hit the network.
    let out = embed_blocking(cfg_for("http://127.0.0.1:1", 2), vec![])
        .await
        .expect("empty embed should be Ok(empty)");
    assert!(out.is_empty());
}
