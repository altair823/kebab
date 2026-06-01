//! Track 1 / Phase 0 feasibility SPIKE (NOT production code).
//!
//! Proves whether candle (pure Rust) can run `intfloat/multilingual-e5-large`
//! with output parity against the existing onnxruntime `FastembedEmbedder`,
//! so the NUMA double-free in fastembed 4.9.1 can be sidestepped.
//!
//! What it checks (see SPIKE_BRIEF.md):
//!   1. numeric parity   — per-sentence cosine vs FastembedEmbedder
//!   2. padding_idx      — XLM-R position ids start at pad_token_id+1
//!   3. thread control   — RAYON_NUM_THREADS caps candle's CPU threads
//!   4. CPU latency      — batch wall-clock, rough vs onnxruntime
//!
//! Run:
//!   CARGO_TARGET_DIR=/build/out/cargo-target/target \
//!   HF_HOME=/build/cache/huggingface \
//!   RAYON_NUM_THREADS=4 \
//!   cargo run -j 4 -p spike-embed-candle --release

use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result};
use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::xlm_roberta::{Config as XlmConfig, XLMRobertaModel};
use tokenizers::{PaddingParams, PaddingStrategy, Tokenizer, TruncationParams};

use kebab_embed::{Embedder, EmbeddingInput, EmbeddingKind};
use kebab_embed_local::FastembedEmbedder;

const HF_MODEL: &str = "intfloat/multilingual-e5-large";
const DOGFOOD_CONFIG: &str = "/build/dogfood/config.toml";
const MAX_LEN: usize = 512;

/// Mixed Korean / English parity set (≥ 8, brief §3).
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

fn main() -> Result<()> {
    // Touch the rayon global pool early so RAYON_NUM_THREADS is honored and
    // reportable before any candle compute spins it up.
    let rayon_threads = rayon::current_num_threads();
    let avail = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(0);
    let rayon_env = std::env::var("RAYON_NUM_THREADS").unwrap_or_else(|_| "<unset>".into());

    println!("== spike-embed-candle ==");
    println!("available_parallelism = {avail}");
    println!("RAYON_NUM_THREADS env = {rayon_env}");
    println!("rayon::current_num_threads() = {rayon_threads}");

    let device = Device::Cpu;

    // ── 1. Fetch model files (candle reads safetensors, not the ONNX cache) ──
    let cache_dir = std::env::var("HF_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/build/cache/huggingface"));
    let api = hf_hub::api::sync::ApiBuilder::new()
        .with_cache_dir(cache_dir.clone())
        .build()
        .context("build hf-hub api")?;
    let repo = api.model(HF_MODEL.to_string());
    println!("\n[load] fetching {HF_MODEL} into {} ...", cache_dir.display());
    let config_path = repo.get("config.json").context("download config.json")?;
    let tokenizer_path = repo.get("tokenizer.json").context("download tokenizer.json")?;
    let weights_path = repo
        .get("model.safetensors")
        .context("download model.safetensors")?;
    println!("[load] config     = {}", config_path.display());
    println!("[load] tokenizer  = {}", tokenizer_path.display());
    println!("[load] weights    = {}", weights_path.display());

    // ── 2. Build the candle XLM-RoBERTa model ──
    let cfg_json = std::fs::read_to_string(&config_path)?;
    let cfg: XlmConfig = serde_json::from_str(&cfg_json).context("parse XLM-R config")?;
    println!(
        "[load] config: hidden={} layers={} heads={} pad_token_id={} max_pos={} pos_emb={}",
        cfg.hidden_size,
        cfg.num_hidden_layers,
        cfg.num_attention_heads,
        cfg.pad_token_id,
        cfg.max_position_embeddings,
        cfg.position_embedding_type,
    );
    let vb = unsafe {
        VarBuilder::from_mmaped_safetensors(&[weights_path], DType::F32, &device)
            .context("mmap safetensors")?
    };
    let model = XLMRobertaModel::new(&cfg, vb).context("build XLMRobertaModel")?;

    let mut tokenizer = Tokenizer::from_file(&tokenizer_path)
        .map_err(|e| anyhow::anyhow!("load tokenizer: {e}"))?;
    tokenizer
        .with_padding(Some(PaddingParams {
            strategy: PaddingStrategy::BatchLongest,
            ..Default::default()
        }))
        .with_truncation(Some(TruncationParams {
            max_length: MAX_LEN,
            ..Default::default()
        }))
        .map_err(|e| anyhow::anyhow!("set truncation: {e}"))?;

    let pad_id = cfg.pad_token_id;

    // ── 3. candle embedding path (passage prefix, masked mean pool, L2) ──
    let candle_vecs = candle_embed(&model, &tokenizer, &device, pad_id, SENTENCES)?;
    println!("\n[candle] embedded {} sentences, dim={}", candle_vecs.len(), candle_vecs[0].len());
    // L2 norm sanity (should be ~1.0 after normalization)
    let norm0 = l2(&candle_vecs[0]);
    println!("[candle] ‖v0‖ = {norm0:.6}");

    // ── 4. FastembedEmbedder (onnxruntime) baseline ──
    println!("\n[fastembed] loading FastembedEmbedder from {DOGFOOD_CONFIG} ...");
    let config = kebab_config::Config::load(Some(std::path::Path::new(DOGFOOD_CONFIG)))
        .context("load dogfood config")?;
    let fb_t0 = Instant::now();
    let fb = FastembedEmbedder::new(&config).context("build FastembedEmbedder")?;
    println!("[fastembed] model loaded in {:.2}s", fb_t0.elapsed().as_secs_f64());
    let fb_inputs: Vec<EmbeddingInput> = SENTENCES
        .iter()
        .map(|s| EmbeddingInput { text: s, kind: EmbeddingKind::Document })
        .collect();
    let fb_vecs = fb.embed(&fb_inputs).context("fastembed embed")?;

    // ── 5. Per-sentence parity (both L2-normalized → cosine = dot) ──
    println!("\n== PARITY (candle vs fastembed, EmbeddingKind::Document / passage:) ==");
    let mut cosines = Vec::with_capacity(SENTENCES.len());
    for (i, s) in SENTENCES.iter().enumerate() {
        let c = cosine(&candle_vecs[i], &fb_vecs[i]);
        cosines.push(c);
        let preview: String = s.chars().take(40).collect();
        println!("  [{i:>2}] cos={c:.6}  {preview}");
    }
    let min = cosines.iter().cloned().fold(f32::INFINITY, f32::min);
    let mean = cosines.iter().sum::<f32>() / cosines.len() as f32;
    println!("  --> cosine min={min:.6}  mean={mean:.6}");

    // ── 6. Latency: batch of 32 (replicated) through candle ──
    let batch: Vec<&str> = SENTENCES.iter().cloned().cycle().take(32).collect();
    // warmup
    let _ = candle_embed(&model, &tokenizer, &device, pad_id, &batch[..4])?;
    let t0 = Instant::now();
    let _ = candle_embed(&model, &tokenizer, &device, pad_id, &batch)?;
    let candle_lat = t0.elapsed();

    let fb_batch: Vec<EmbeddingInput> = batch
        .iter()
        .map(|s| EmbeddingInput { text: s, kind: EmbeddingKind::Document })
        .collect();
    let t1 = Instant::now();
    let _ = fb.embed(&fb_batch)?;
    let fb_lat = t1.elapsed();

    let peak_threads = proc_threads();
    println!("\n== LATENCY (batch=32) ==");
    println!("  candle    : {:.3}s ({:.1} ms/sentence)", candle_lat.as_secs_f64(), candle_lat.as_secs_f64() * 1000.0 / 32.0);
    println!("  fastembed : {:.3}s ({:.1} ms/sentence)", fb_lat.as_secs_f64(), fb_lat.as_secs_f64() * 1000.0 / 32.0);

    println!("\n== THREAD CONTROL ==");
    println!("  RAYON_NUM_THREADS env       = {rayon_env}");
    println!("  rayon::current_num_threads  = {rayon_threads}");
    println!("  available_parallelism       = {avail}");
    println!("  peak OS threads (/proc)     = {peak_threads}");

    // ── 7. Machine verdict line for the report ──
    let verdict = if mean >= 0.99 { "PASS" } else if mean >= 0.95 { "MARGINAL" } else { "FAIL" };
    println!("\n== SUMMARY ==");
    println!("VERDICT_HINT={verdict} cosine_min={min:.6} cosine_mean={mean:.6} candle_batch32_s={:.3} fb_batch32_s={:.3} rayon_threads={rayon_threads} rayon_env={rayon_env}", candle_lat.as_secs_f64(), fb_lat.as_secs_f64());

    Ok(())
}

/// candle embedding: apply e5 `passage:` prefix, tokenize (batch-padded),
/// forward through XLM-R, attention-mask-weighted mean pool, L2 normalize.
fn candle_embed(
    model: &XLMRobertaModel,
    tokenizer: &Tokenizer,
    device: &Device,
    _pad_id: u32,
    sentences: &[&str],
) -> Result<Vec<Vec<f32>>> {
    let prefixed: Vec<String> = sentences.iter().map(|s| format!("passage: {s}")).collect();
    let encodings = tokenizer
        .encode_batch(prefixed, true)
        .map_err(|e| anyhow::anyhow!("encode_batch: {e}"))?;

    let bsz = encodings.len();
    let seq = encodings[0].get_ids().len();

    let mut ids = Vec::with_capacity(bsz * seq);
    let mut mask = Vec::with_capacity(bsz * seq);
    for enc in &encodings {
        ids.extend(enc.get_ids().iter().copied());
        mask.extend(enc.get_attention_mask().iter().map(|&m| m as f32));
    }

    let input_ids = Tensor::from_vec(ids, (bsz, seq), device)?;
    let attn_f32 = Tensor::from_vec(mask, (bsz, seq), device)?;
    let token_type_ids = input_ids.zeros_like()?;

    // forward: (input_ids, attention_mask, token_type_ids, past, enc_hidden, enc_mask)
    let hidden = model.forward(&input_ids, &attn_f32, &token_type_ids, None, None, None)?;

    // masked mean pool
    let mask3 = attn_f32.unsqueeze(2)?; // (b, seq, 1)
    let summed = hidden.broadcast_mul(&mask3)?.sum(1)?; // (b, hidden)
    let counts = mask3.sum(1)?; // (b, 1)
    let mean = summed.broadcast_div(&counts)?;

    // L2 normalize
    let norm = mean.sqr()?.sum_keepdim(1)?.sqrt()?;
    let normalized = mean.broadcast_div(&norm)?;

    Ok(normalized.to_vec2::<f32>()?)
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na = l2(a);
    let nb = l2(b);
    dot / (na * nb)
}

fn l2(v: &[f32]) -> f32 {
    v.iter().map(|x| x * x).sum::<f32>().sqrt()
}

/// Peak OS thread count for this process from /proc/self/status.
fn proc_threads() -> usize {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("Threads:"))
                .and_then(|l| l.split_whitespace().nth(1).map(str::to_string))
        })
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}
