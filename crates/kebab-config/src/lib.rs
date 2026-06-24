//! `kb-config` — `Config` schema and XDG path resolution (§6).
//!
//! Layer order (`Config::load`): defaults → file → env (`KEBAB_<SECTION>_<KEY>`).
//! CLI overrides land later, applied by `kb-cli` after `Config::load`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

mod paths;
pub mod migrate;
pub use paths::{expand_path, expand_path_with_base};

/// Implicit source id used when a single-root `[workspace]` config (no
/// `[[workspace.sources]]`) is normalized into the multi-source model, and
/// the `DEFAULT` value of the `documents.source_id` column. Kept in sync
/// with the migration default in `migrations/V0XX__documents_source_id.sql`.
pub const DEFAULT_SOURCE_ID: &str = "default";

/// f32 의 shortest round-trip(Display)을 f64 로 재파싱해 직렬화한다.
/// `0.3_f32` 가 `0.30000001192092896` 으로 새지 않고 `0.3` 으로 출력되게 한다.
/// 마이그레이션 시 toml_edit relocation 의 무손실 비교를 깨지 않도록, 그리고
/// `kebab config migrate` 산출물이 사람이 읽기 좋게.
fn ser_f32_clean<S>(v: &f32, s: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let clean: f64 = format!("{v}").parse().unwrap_or(f64::from(*v));
    s.serialize_f64(clean)
}

/// Signal: `Config::from_file` / `Config::load` failed due to missing path,
/// I/O failure, TOML parse failure, or post-parse validation failure.
///
/// Wrapped into `anyhow::Error` at the API boundary so callers that need
/// structured details (e.g. kebab-cli's `error_classify`) can
/// `downcast_ref::<ConfigInvalid>()` for the wire record.
#[derive(Debug, thiserror::Error)]
#[error("config invalid at {path}: {cause}")]
pub struct ConfigInvalid {
    pub path: PathBuf,
    pub cause: String,
}

/// p20-bugfix3 Bug #10: explicit `--config <path>` was missing → silent
/// fallback to defaults instead of fail-fast. `kebab-app::error_wire::classify`
/// downcasts → `code: "config_not_found"` ErrorV1.
#[derive(Debug, thiserror::Error)]
#[error("config file does not exist: {path}")]
pub struct ConfigNotFound {
    pub path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Config {
    pub schema_version: u32,
    pub workspace: WorkspaceCfg,
    pub storage: StorageCfg,
    pub models: ModelsCfg,
    /// v3: 모든 미디어 형식 ingest 설정의 우산 — 병렬도(← 옛 `[indexing]`),
    /// chunking, code, image, pdf 가 전부 `[ingest.*]` 하위로 통합됐다.
    /// `#[serde(default)]` 로 두어 미변환 / 부분 config 도 로드된다(자동
    /// 변환은 `Config::from_file` 가 메모리에서 수행 — T6).
    #[serde(default)]
    pub ingest: IngestCfg,
    pub search: SearchCfg,
    pub rag: RagCfg,
    /// p9-fb-14: TUI palette + role-style mapping. `#[serde(default)]`
    /// so configs that predate this section still load (defaults to
    /// `dark`).
    #[serde(default = "UiCfg::defaults")]
    pub ui: UiCfg,
    /// v0.20.x ingest log surface. `#[serde(default)]` so pre-v0.20
    /// config files without a `[logging]` section load with built-in
    /// defaults (enabled=true, dir=~/.local/state/kebab/logs).
    #[serde(default)]
    pub logging: LoggingCfg,
    /// p9-fb-05: directory of the on-disk config file this `Config`
    /// was loaded from, if any. Populated by `Config::from_file` /
    /// `Config::load` — never serialized (`#[serde(skip)]`). Used by
    /// `expand_path_with_base` to resolve relative `workspace.root`
    /// against the config file's location instead of the user's
    /// `cwd` (so `--config /tmp/cfg.toml` + `root = "kb"` reads
    /// `/tmp/kb` no matter where the user invoked from).
    ///
    /// `pub(crate)` so external callers can't break the
    /// "stamped only by from_file/load" invariant by hand. Use
    /// [`Config::with_source_dir`] for tests / programmatic
    /// construction that need a specific `source_dir`.
    #[serde(skip)]
    pub(crate) source_dir: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceCfg {
    /// Single-root workspace (legacy / common case). `Option` so that a
    /// config that declares only `[[workspace.sources]]` (no bare `root`)
    /// parses — and, symmetrically, a legacy single-`root` config (no
    /// `sources`) still parses unchanged. The load-time normalizer
    /// ([`Config::normalize_sources`]) reconciles the two into a single
    /// non-empty `sources` list (`id = "default"` synthesized from `root`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    pub exclude: Vec<String>,
    /// `[[workspace.sources]]`: named multi-source declaration. When empty
    /// and `root` is set, the load path normalizes to a single implicit
    /// `default` source. Each entry stamps its `id` onto every document it
    /// ingests and supplies per-source `trust_level` / `source_type`
    /// defaults (frontmatter still wins per the §0 Q9 derive table).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<SourceCfg>,
}

/// One named source under `[[workspace.sources]]`.
///
/// `trust_level` / `source_type` are the **source-level defaults**: they
/// apply when a document's frontmatter does not specify the field. The
/// precedence is `frontmatter > source default > hardcoded`
/// (`TrustLevel::Primary` / `SourceType::Markdown`) — implemented in the
/// markdown derive via `BodyHints::fallback_trust_level`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SourceCfg {
    /// Stable identifier stamped onto `documents.source_id` for every
    /// document ingested from this source. Must be unique and non-empty
    /// across the workspace (enforced in [`Config::validate`]).
    pub id: String,
    /// Root directory to walk for this source. Accepts the same
    /// absolute / `~` / `${VAR}` / relative(=config-dir-based) forms as
    /// the legacy `workspace.root`.
    pub root: String,
    /// Per-source denylist globs, merged on top of `workspace.exclude`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,
    /// Per-source default `trust_level` (frontmatter overrides it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trust_level: Option<kebab_core::TrustLevel>,
    /// Per-source default `source_type` (frontmatter overrides it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_type: Option<kebab_core::SourceType>,
}

/// A source with its `root` resolved to an absolute path and its `exclude`
/// merged with `workspace.exclude`. Produced by [`Config::resolved_sources`]
/// — the single entry point the ingest pipeline iterates over.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedSource {
    /// Stamped onto `documents.source_id`.
    pub id: String,
    /// Absolute walk root (tilde / `${VAR}` / relative-to-config resolved).
    pub root: PathBuf,
    /// `workspace.exclude` ∪ per-source `exclude`.
    pub exclude: Vec<String>,
    /// Per-source default trust level (None → fall back to `Primary`).
    pub trust_level: Option<kebab_core::TrustLevel>,
    /// Per-source default source type (None → fall back to `Markdown`).
    pub source_type: Option<kebab_core::SourceType>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StorageCfg {
    pub data_dir: String,
    pub sqlite: String,
    pub vector_dir: String,
    pub asset_dir: String,
    pub artifact_dir: String,
    pub model_dir: String,
    pub runs_dir: String,
    pub copy_threshold_mb: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChunkingCfg {
    pub target_tokens: usize,
    pub overlap_tokens: usize,
    pub respect_markdown_headings: bool,
    pub chunker_version: String,
    /// Max byte/3 token estimate per emitted chunk (md-heading-v2).
    /// After the v1-equivalent chunking pass, any chunk whose estimate
    /// exceeds this value is split at line (then UTF-8 char) boundaries
    /// into sub-pieces each ≤ budget. Covers all block kinds: list, code,
    /// paragraph, table. The default (4000) is large enough to keep normal
    /// content atomic while splitting pathological log / stacktrace / Jira
    /// list dumps that would otherwise overflow an embedder context window.
    /// `#[serde(default)]` so pre-v2 config files that predate the key
    /// still load (migration injects it additively).
    #[serde(default = "default_max_chunk_tokens")]
    pub max_chunk_tokens: usize,
}

/// Default md-heading-v2 chunk split budget. 4000 byte/3 tokens
/// (~12 KB) keeps ordinary source files and prose atomic while
/// splitting the pathological 20k–76k token Jira log blocks that fail
/// to embed on strict servers.
fn default_max_chunk_tokens() -> usize {
    4000
}

/// Floor for `[ingest.chunking].target_tokens` (and, transitively, the
/// per-chunk cap `max_chunk_tokens ≥ target_tokens`). A chunk target
/// below this is never a real config — it only ever appears as a typo /
/// zero that would shatter the corpus into useless fragments. Enforced
/// by [`Config::validate_chunking`].
const MIN_CHUNK_TOKENS: usize = 16;

impl ChunkingCfg {
    pub fn defaults() -> Self {
        Self {
            target_tokens: 500,
            overlap_tokens: 80,
            respect_markdown_headings: true,
            // md-heading-v2 is the hardcoded markdown default (it splits
            // oversize chunks of any block kind; v1 never did). Stamping it
            // here means the lib.rs skip-check re-chunks md docs on next
            // ingest via the version cascade (design §9).
            chunker_version: "md-heading-v2".to_string(),
            max_chunk_tokens: default_max_chunk_tokens(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelsCfg {
    pub embedding: EmbeddingModelCfg,
    pub llm: LlmCfg,
    /// p9-fb-41 PR-9c-1: NLI verifier model + provider knob.
    /// `#[serde(default)]` so pre-v0.18 config files that predate the
    /// `[models.nli]` section still load with built-in defaults
    /// (`Xenova/mDeBERTa-v3-base-xnli-multilingual-nli-2mil7` / `onnx`).
    /// The verifier itself is gated by `[rag].nli_threshold` — even
    /// with a model configured here, threshold `0.0` (the default)
    /// skips the verification step entirely.
    #[serde(default = "NliCfg::defaults")]
    pub nli: NliCfg,
}

/// p9-fb-41 PR-9c-1: NLI verifier configuration. The model id flows to
/// `OnnxNliVerifier::new` via `kebab-nli` (PR-9c-2 wiring); the provider
/// is reserved for future verifier swap-in (currently only `"onnx"` is
/// recognized — anything else falls back to the same path).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NliCfg {
    pub model: String,
    pub provider: String,
}

impl NliCfg {
    pub fn defaults() -> Self {
        Self {
            model: "Xenova/mDeBERTa-v3-base-xnli-multilingual-nli-2mil7".to_string(),
            provider: "onnx".to_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EmbeddingModelCfg {
    /// `fastembed` (default, onnxruntime), `candle` (pure-Rust, NUMA-safe),
    /// or `ollama` (remote HTTP embedding endpoint). `none` disables
    /// embeddings (lexical-only). Unknown values error at embedder
    /// construction.
    pub provider: String,
    pub model: String,
    pub version: String,
    pub dimensions: usize,
    pub batch_size: usize,
    /// Cap on the CPU worker threads the `candle` provider spins up
    /// (sizes the global rayon pool; env `KEBAB_EMBED_THREADS` overrides).
    /// `0` = auto (rayon default = #cores). Lever to sidestep the
    /// onnxruntime 48-thread NUMA double-free; ignored by the `fastembed`
    /// provider. Defaulted on load so pre-0.22 config files still parse.
    #[serde(default)]
    pub num_threads: u32,
    /// HTTP endpoint for the `ollama` embedding provider (e.g.
    /// `"http://127.0.0.1:11434"`). `None` (or a missing key in TOML) means
    /// "fall back to `models.llm.endpoint`" — same convention as the OCR /
    /// vision endpoints. Ignored by the `fastembed` / `candle` providers.
    /// Defaulted on load so pre-0.26 config files still parse.
    #[serde(default)]
    pub endpoint: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LlmCfg {
    pub provider: String,
    pub model: String,
    pub context_tokens: usize,
    pub endpoint: String,
    #[serde(serialize_with = "ser_f32_clean")]
    pub temperature: f32,
    pub seed: u64,
    /// v0.17.0 post-dogfood: Hard ceiling on a single HTTP exchange to
    /// the LLM endpoint (Ollama, etc.). Cold-loading an 8B+ model on
    /// CPU-only hosts can spend 60-90s on model load + several minutes
    /// on a first inference, blowing past the old hard-coded 300s cap
    /// and surfacing as `error: kb-rag: llm.generate_stream` to the
    /// user. Config-driven so 16-GB / CPU-only deployments using small
    /// (≤4B) models can keep the original 300s and large-model dogfood
    /// can dial it up (e.g. 1200s) without rebuilding.
    ///
    /// **Edge case — `0` is NOT a disable sentinel.**
    /// `reqwest::ClientBuilder::timeout(Duration::from_secs(0))` sets a
    /// 0-second read timeout, so every request fails *immediately* with
    /// `error: kb-rag: ollama timeout`. To approximate "no cap", use a
    /// large finite value (e.g. `u64::MAX` ≈ 5.8 × 10¹¹ years, or
    /// just a generous number like `86400`).
    #[serde(default = "default_llm_request_timeout_secs")]
    pub request_timeout_secs: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SearchCfg {
    pub default_k: usize,
    pub hybrid_fusion: String,
    pub rrf_k: u32,
    pub snippet_chars: usize,
    /// p9-fb-19: in-memory LRU cache capacity for `App::search`.
    /// One entry ≈ 5 KB → default 256 caps memory at ~1.3 MB. Set
    /// to `0` to disable the cache entirely. Stale entries
    /// (corpus_revision mismatch) are evicted on next access.
    #[serde(default = "default_cache_capacity")]
    pub cache_capacity: usize,
    /// p9-fb-32: hits and citations whose source doc was last
    /// re-processed more than this many days ago are marked
    /// `stale: true` in wire / TUI / CLI surfaces. `0` disables.
    #[serde(default = "default_stale_threshold_days")]
    pub stale_threshold_days: u32,
}

fn default_cache_capacity() -> usize {
    256
}

/// v0.17.0 post-dogfood: matches the legacy hard-coded ceiling so
/// existing configs that omit the field keep behaving identically.
/// Overridable per config / `KEBAB_MODELS_LLM_REQUEST_TIMEOUT_SECS`.
fn default_llm_request_timeout_secs() -> u64 {
    300
}

fn default_stale_threshold_days() -> u32 {
    30
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RagCfg {
    pub prompt_template_version: String,
    #[serde(serialize_with = "ser_f32_clean")]
    pub score_gate: f32,
    pub explain_default: bool,
    pub max_context_tokens: usize,
    /// p9-fb-41: hard ceiling on the number of multi-hop iterations
    /// (decompose iter + decide iters). When the LLM keeps returning
    /// `continue` past this depth the pipeline cuts to `synthesize`
    /// with `HopRecord.forced_stop = true`. Default `3` — enough for
    /// most cross-doc reasoning, low enough to bound LLM cost.
    #[serde(default = "default_multi_hop_max_depth")]
    pub multi_hop_max_depth: u32,
    /// p9-fb-41: cap on how many sub-queries the LLM may emit in a
    /// single decompose / decide call. This is the *prompt-side
    /// soft hint* — the value the pipeline injects into the
    /// decompose / decide prompts so the LLM knows what to aim for.
    /// kebab-rag enforces a separate compile-time hard ceiling
    /// (`MULTI_HOP_MAX_SUB_QUERIES_HARD_CAP`, currently 10) as a
    /// safety net against misbehaving models — if you raise this
    /// knob above the hard cap, bump the const in the same PR.
    /// Default `5`.
    #[serde(default = "default_multi_hop_max_sub_queries_per_iter")]
    pub multi_hop_max_sub_queries_per_iter: u32,
    /// p9-fb-41: hard ceiling on the deduped chunk pool. When the
    /// accumulated pool would exceed this many chunks the pipeline
    /// stops accepting new retrieval results and forces synthesize
    /// with `forced_stop = true`.
    ///
    /// Default `15` — tuned down from the original 30 in the v0.18
    /// pre-cut dogfood (`tasks/HOTFIXES.md` 2026-05-25 fb-41 entry,
    /// "post-PR-7 dogfood retest + PR-8 partial mitigation" sub-section).
    /// With 30 chunks the synthesize prompt was large enough for
    /// gemma3:4b to lose the citation rule + drift into unrelated
    /// chunks; 15 keeps the prompt tight while still allowing 3-iter
    /// cross-doc reasoning over ~5 chunks per iter.
    #[serde(default = "default_multi_hop_max_pool_chunks")]
    pub multi_hop_max_pool_chunks: u32,
    /// p9-fb-41 PR-9c-1: minimum NLI entailment score required for the
    /// multi-hop synthesize answer to be returned as `grounded=true`
    /// (spec §2.6 single gate). When the post-synthesize NLI verifier
    /// returns `NliScores::faithfulness() < nli_threshold` the
    /// pipeline refuses with `RefusalReason::NliVerificationFailed`.
    ///
    /// Default `0.0` = verification disabled — no NLI call, multi-hop
    /// matches its PR-3b behavior exactly. Set to e.g. `0.5` to
    /// activate the gate. Knob lives on `[rag]` (the gate is a RAG
    /// policy, not a model property); the model itself comes from
    /// `[models.nli].model`.
    ///
    /// Single-pass `ask` ignores this knob entirely — only multi-hop
    /// runs through the verification step (PR-9c-2 wires it).
    #[serde(default = "default_nli_threshold", serialize_with = "ser_f32_clean")]
    pub nli_threshold: f32,
}

fn default_multi_hop_max_depth() -> u32 {
    3
}

fn default_multi_hop_max_sub_queries_per_iter() -> u32 {
    5
}

fn default_multi_hop_max_pool_chunks() -> u32 {
    15
}

/// p9-fb-41 PR-9c-1: NLI gate disabled by default per spec §2.6
/// (verification opt-in — users explicitly raise the threshold once
/// they're ready to trade refusal-rate for groundedness).
fn default_nli_threshold() -> f32 {
    0.0
}

/// Settings for the image ingest pipeline (P6). `ocr` controls OCR
/// behaviour (P6-2); `caption` controls vision-LM captioning (P6-3).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImageCfg {
    #[serde(default = "OcrCfg::defaults")]
    pub ocr: OcrCfg,
    #[serde(default = "CaptionCfg::defaults")]
    pub caption: CaptionCfg,
}

impl ImageCfg {
    pub fn defaults() -> Self {
        Self {
            ocr: OcrCfg::defaults(),
            caption: CaptionCfg::defaults(),
        }
    }
}

/// OCR settings (P6-2). v1 ships a single Ollama-vision adapter; the
/// `OcrEngine` trait in `kebab-parse-image` keeps the door open for
/// Tesseract / Apple Vision / PaddleOCR engines as feature-gated
/// alternatives in P+. See `tasks/HOTFIXES.md` (2026-05-02) for the
/// rationale on dropping the original Tesseract default.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OcrCfg {
    /// Run OCR on every image during ingest. Default `false` because
    /// OCR adds one model call per asset.
    pub enabled: bool,
    /// Engine identifier. v1 only ships `"ollama-vision"`.
    pub engine: String,
    /// Model id passed to the engine (e.g. `"gemma4:e4b"` for
    /// Ollama-vision).
    pub model: String,
    /// HTTP endpoint for the OCR engine. `None` (or a missing key in
    /// TOML) means "fall back to `models.llm.endpoint`" — convenient
    /// when the same Ollama host serves both LLM and vision.
    #[serde(default)]
    pub endpoint: Option<String>,
    /// BCP-47 language hints (e.g. `["eng", "kor"]`). The adapter
    /// renders them into the prompt; the LLM honours them probabilistically.
    pub languages: Vec<String>,
    /// Cap the long edge of the image (in pixels) before sending. Larger
    /// images bloat prompt cost. Default `1600`.
    pub max_pixels: u32,
    /// v0.17.2 post-dogfood: Hard ceiling on a single HTTP exchange to
    /// the OCR endpoint. Sister knob to [`LlmCfg::request_timeout_secs`]
    /// — kept separate because OCR latency is typically shorter than
    /// chat-LLM cold start, and large vision models on CPU-only hosts
    /// occasionally need a different budget. See HOTFIXES 2026-05-25
    /// for the rationale.
    ///
    /// **Edge case — `0` is NOT a disable sentinel.** Same semantics as
    /// [`LlmCfg::request_timeout_secs`]: `Duration::from_secs(0)` means
    /// "every request fails immediately" (reqwest 0.12.x — the read
    /// timeout is applied as a 0-second deadline), not "no timeout".
    /// To approximate "no cap", use a large finite value (e.g.
    /// `u64::MAX` ≈ 5.8 × 10¹¹ years, or just a generous number like
    /// `86400`).
    #[serde(default = "default_ocr_request_timeout_secs")]
    pub request_timeout_secs: u64,

    // ── paddle-onnx engine overrides (v0.27.0) ──────────────────────────
    // Only consulted when `engine == "paddle-onnx"`; the ollama-vision
    // engine ignores them. All `#[serde(default)]` so pre-v0.27 config
    // files load unchanged.
    /// Override path to the detection ONNX model. `None` → bundled
    /// `assets/paddleocr-onnx/ppocrv5_mobile_det.onnx` (or the directory
    /// named by `KEBAB_IMAGE_OCR_MODEL_DIR`).
    #[serde(default)]
    pub det_model: Option<String>,
    /// Override path to the recognition ONNX model. `None` → bundled
    /// `assets/paddleocr-onnx/korean_ppocrv5_mobile_rec.onnx`.
    #[serde(default)]
    pub rec_model: Option<String>,
    /// Override path to the character dictionary. `None` → bundled
    /// `assets/paddleocr-onnx/korean_dict.txt`.
    #[serde(default)]
    pub dict: Option<String>,
    /// DBNet detection box score threshold (0.0..=1.0). Boxes whose mean
    /// probability is below this are dropped. Default `0.3`.
    #[serde(default = "default_ocr_score_thresh", serialize_with = "ser_f32_clean")]
    pub score_thresh: f32,
    /// Polygon unclip ratio applied to each detected box before crop.
    /// Larger = more padding around the text. Default `1.5`.
    #[serde(default = "default_ocr_unclip_ratio", serialize_with = "ser_f32_clean")]
    pub unclip_ratio: f32,
    /// Hard cap on detected boxes per image (runaway guard). Extra boxes
    /// past this count are truncated with a warning. Default `1000`.
    #[serde(default = "default_ocr_max_boxes")]
    pub max_boxes: usize,
}

impl OcrCfg {
    pub fn defaults() -> Self {
        Self {
            enabled: false,
            engine: "ollama-vision".to_string(),
            model: "gemma4:e4b".to_string(),
            endpoint: None,
            languages: vec!["eng".to_string(), "kor".to_string()],
            max_pixels: 1600,
            request_timeout_secs: default_ocr_request_timeout_secs(),
            det_model: None,
            rec_model: None,
            dict: None,
            score_thresh: default_ocr_score_thresh(),
            unclip_ratio: default_ocr_unclip_ratio(),
            max_boxes: default_ocr_max_boxes(),
        }
    }
}

/// paddle-onnx DBNet box score threshold default. See [`OcrCfg::score_thresh`].
fn default_ocr_score_thresh() -> f32 {
    0.3
}
/// paddle-onnx unclip ratio default. See [`OcrCfg::unclip_ratio`].
fn default_ocr_unclip_ratio() -> f32 {
    1.5
}
/// paddle-onnx box-count cap default. See [`OcrCfg::max_boxes`].
fn default_ocr_max_boxes() -> usize {
    1000
}

/// v0.17.2 post-dogfood: matches the legacy hard-coded ceiling so
/// existing configs that omit the field keep behaving identically.
/// Overridable per config / `KEBAB_IMAGE_OCR_REQUEST_TIMEOUT_SECS`.
fn default_ocr_request_timeout_secs() -> u64 {
    300
}

/// Caption settings (P6-3). Caption uses the same Ollama-vision /
/// `LanguageModel` pipeline as the rest of the workspace; the trait
/// abstraction is the part the spec demands. `enabled` defaults to
/// `false` because captioning costs one model call per asset and the
/// output is model-generated (low trust).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CaptionCfg {
    /// Run captioning on every image during ingest. Default `false`.
    pub enabled: bool,
    /// Cap the long edge of the image (in pixels) before sending. The
    /// spec recommends an aggressive 768×768 cap because larger
    /// vision-LM inputs translate directly into prompt cost. Default
    /// `768`.
    pub max_pixels: u32,
    /// Caption prompt template version pinned into wire output via
    /// `ModelCaption.model_version`. Bump when the prompt changes so
    /// downstream eval can detect regressions.
    pub prompt_template_version: String,
}

impl CaptionCfg {
    pub fn defaults() -> Self {
        Self {
            enabled: false,
            max_pixels: 768,
            prompt_template_version: "caption-v1".to_string(),
        }
    }
}

/// Settings for the PDF ingest pipeline (P7 + v0.20.0 sub-item 1).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PdfCfg {
    #[serde(default = "PdfOcrCfg::defaults")]
    pub ocr: PdfOcrCfg,
}

impl PdfCfg {
    pub fn defaults() -> Self {
        Self {
            ocr: PdfOcrCfg::defaults(),
        }
    }
}

impl Default for PdfCfg {
    fn default() -> Self {
        Self::defaults()
    }
}

/// v0.20.x ingest log surface: structured ndjson log written per ingest run.
/// `#[serde(default)]` on Config.logging ensures pre-v0.20 config files load cleanly.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LoggingCfg {
    /// Write structured ndjson log for each ingest run. Default `true`.
    /// Set `false` to suppress log file creation entirely (AC-6).
    #[serde(default = "default_ingest_log_enabled")]
    pub ingest_log_enabled: bool,

    /// Directory for per-run log files. Default `{state_dir}/logs`.
    /// `{state_dir}` expands to the XDG state dir (e.g. `~/.local/state/kebab`).
    #[serde(default = "default_ingest_log_dir")]
    pub ingest_log_dir: PathBuf,

    /// v0.20.x r2 Enhancement 4: keep the most recent N ingest log files.
    /// Older files (beyond this count) are deleted at ingest start.
    /// Default 100. AC-9: #[serde(default)] ensures backward compat.
    #[serde(default = "default_keep_recent_runs")]
    pub keep_recent_runs: u32,

    /// v0.20.x r2 Enhancement 4: delete log files older than N days.
    /// Also applied to `pdf_ocr_events` SQLite rows. Default 30.
    #[serde(default = "default_retention_days")]
    pub retention_days: u32,
}

fn default_ingest_log_enabled() -> bool {
    true
}
fn default_ingest_log_dir() -> PathBuf {
    PathBuf::from("{state_dir}/logs")
}
fn default_keep_recent_runs() -> u32 {
    100
}
fn default_retention_days() -> u32 {
    30
}

impl Default for LoggingCfg {
    fn default() -> Self {
        Self {
            ingest_log_enabled: default_ingest_log_enabled(),
            ingest_log_dir: default_ingest_log_dir(),
            keep_recent_runs: default_keep_recent_runs(),
            retention_days: default_retention_days(),
        }
    }
}

/// v0.20.0 sub-item 1: scanned PDF OCR via Ollama vision LLM. Default
/// disabled — opt-in because OCR adds ~45-100s per scanned page on CPU
/// (qwen2.5vl:3b, remote). Enable for book / paper scan KB.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PdfOcrCfg {
    /// Run OCR on scanned PDF pages. Default `false` (opt-in).
    pub enabled: bool,
    /// `false` (default) — text-detect first + vision fallback on
    /// scanned pages only. `true` — vision LLM 호출 on every page
    /// (vector PDF 의 dual-text confidence boost — doubles chunk count).
    pub always_on: bool,
    /// Engine identifier: `"ollama-vision"` or `"paddle-onnx"`. When set to
    /// `"paddle-onnx"`, model paths and tuning knobs are read from
    /// `[image.ocr]`, not `[pdf.ocr]` — PaddleOCR has no PDF-specific tuning.
    pub engine: String,
    /// Vision model id. Default `"qwen2.5vl:3b"` per PoC (§3.5 family
    /// asymmetry vs image OCR's gemma4:e4b is acknowledged).
    pub model: String,
    /// HTTP endpoint. `None` → fall back to `models.llm.endpoint`.
    #[serde(default)]
    pub endpoint: Option<String>,
    /// BCP-47 language hints rendered into prompt.
    pub languages: Vec<String>,
    /// Long-edge cap (px). Larger images bloat prompt cost.
    pub max_pixels: u32,
    /// HTTP request timeout (sec). Same `0` = "fail immediately"
    /// semantics as `image.ocr.request_timeout_secs` (NOT a disable
    /// sentinel — see image.ocr docs).
    #[serde(default = "default_pdf_ocr_request_timeout_secs")]
    pub request_timeout_secs: u64,
    /// Valid char ratio threshold (0.0..=1.0). Page with ratio below
    /// this is classified as scanned/mojibake → OCR fallback. Default
    /// `0.5`.
    #[serde(default = "default_pdf_ocr_valid_ratio", serialize_with = "ser_f32_clean")]
    pub valid_ratio_threshold: f32,
    /// Minimum char count per page below which page is auto-scanned.
    /// Default `20`.
    #[serde(default = "default_pdf_ocr_min_char_count")]
    pub min_char_count: u32,
    /// Single-page lang hint. Default `Some("kor")`. `None` = no hint.
    #[serde(default = "default_pdf_ocr_lang_hint")]
    pub lang_hint: Option<String>,

    // ── paddle-onnx engine overrides (v3) ───────────────────────────────
    // Symmetric with `[ingest.image.ocr]`. v2 의 "pdf paddle 이 image 의
    // 모델 경로를 빌려쓰던" 비대칭을 제거 — pdf 자체 키로 옮긴다. 마이그레이션
    // (T5)이 image 값을 이 키로 복사해 signature 바이트 동일 유지. 전부
    // `#[serde(default)]` 이라 pre-v3 config 도 로드.
    /// Override path to the detection ONNX model. `None` → bundled.
    #[serde(default)]
    pub det_model: Option<String>,
    /// Override path to the recognition ONNX model. `None` → bundled.
    #[serde(default)]
    pub rec_model: Option<String>,
    /// Override path to the character dictionary. `None` → bundled.
    #[serde(default)]
    pub dict: Option<String>,
    /// DBNet detection box score threshold (0.0..=1.0). Default `0.3`.
    #[serde(default = "default_ocr_score_thresh", serialize_with = "ser_f32_clean")]
    pub score_thresh: f32,
    /// Polygon unclip ratio applied to each detected box. Default `1.5`.
    #[serde(default = "default_ocr_unclip_ratio", serialize_with = "ser_f32_clean")]
    pub unclip_ratio: f32,
    /// Hard cap on detected boxes per page (runaway guard). Default `1000`.
    #[serde(default = "default_ocr_max_boxes")]
    pub max_boxes: usize,
}

impl PdfOcrCfg {
    pub fn defaults() -> Self {
        Self {
            enabled: false,
            always_on: false,
            engine: "ollama-vision".to_string(),
            model: "qwen2.5vl:3b".to_string(),
            endpoint: None,
            languages: vec!["eng".to_string(), "kor".to_string()],
            max_pixels: 2048,
            request_timeout_secs: default_pdf_ocr_request_timeout_secs(),
            valid_ratio_threshold: default_pdf_ocr_valid_ratio(),
            min_char_count: default_pdf_ocr_min_char_count(),
            lang_hint: default_pdf_ocr_lang_hint(),
            det_model: None,
            rec_model: None,
            dict: None,
            score_thresh: default_ocr_score_thresh(),
            unclip_ratio: default_ocr_unclip_ratio(),
            max_boxes: default_ocr_max_boxes(),
        }
    }
}

/// PDF OCR per-page request timeout 의 기본값.
/// 6-32s 가 정상 throughput; 180s 초과는 Ollama 다운 / 매우 dense·고해상도 page 의 신호.
/// `config.toml` 의 `[pdf.ocr] request_timeout_secs = N` 로 override.
///
/// HOTFIXES 2026-05-27 (Bug #11): metro-korea.pdf dogfood 에서 page 8/13 모두
/// 기존 600s default 까지 완전 timeout (`chars: 0, skipped: true` × 20분 cost) →
/// 우선 60s 로 하향. parent spec §1000 / §1628 OQ-1 ("CPU 환경 105s 의 5x 여유") 가
/// 가정한 "page 당 평균 105s" 보다 실측 cloud GPU Ollama 가 6-32s 로 훨씬 빠름.
///
/// HOTFIXES 2026-05-28 (Bug #11 follow-up): 60s 는 dense Korean page (특히
/// metro-korea.pdf page 8/9/13) 의 OCR 을 강제 timeout 시켜 본문 indexed 손실.
/// **conservative starting point 180s 로 재조정** + dogfood evidence 기반 sweet spot
/// 점진적 축소 정책. user 가 `[pdf.ocr] request_timeout_secs = N` 으로 직접 tune.
fn default_pdf_ocr_request_timeout_secs() -> u64 {
    180
}
fn default_pdf_ocr_valid_ratio() -> f32 {
    0.5
}
fn default_pdf_ocr_min_char_count() -> u32 {
    20
}
fn default_pdf_ocr_lang_hint() -> Option<String> {
    Some("kor".to_string())
}

/// p9-fb-14: TUI-only configuration. Currently a single `theme`
/// selector (`"dark"` / `"light"`); future fields (custom role
/// overrides, mode-machine cursor shapes, …) extend the same
/// section so the CLI doesn't grow a per-feature `[ui.*]` table.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UiCfg {
    /// Palette name. Recognized: `"dark"` (default), `"light"`.
    /// Unknown values fall back to `"dark"` at construction time
    /// — config never errors on a typo, the TUI just keeps the
    /// default theme so the user has a working shell.
    pub theme: String,
}

impl UiCfg {
    pub fn defaults() -> Self {
        Self {
            theme: "dark".to_string(),
        }
    }
}

/// v3: 모든 미디어 형식 ingest 설정의 우산. 스칼라(병렬도)는 ← 옛 `[indexing]`,
/// 미디어별 하위 테이블(chunking/code/image/pdf)은 ← 옛 top-level 섹션.
/// 직렬화 순서 = 필드 순서: 스칼라(병렬도) 먼저, 하위 테이블 뒤
/// (TOML 의 "bare key 는 sub-table header 앞" 규칙 준수).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IngestCfg {
    #[serde(default = "default_max_parallel_extractors")]
    pub max_parallel_extractors: u32,
    #[serde(default = "default_max_parallel_embeddings")]
    pub max_parallel_embeddings: u32,
    #[serde(default)]
    pub watch_filesystem: bool,
    #[serde(default = "ChunkingCfg::defaults")]
    pub chunking: ChunkingCfg,
    #[serde(default)]
    pub code: IngestCodeCfg,
    #[serde(default = "ImageCfg::defaults")]
    pub image: ImageCfg,
    #[serde(default = "PdfCfg::defaults")]
    pub pdf: PdfCfg,
}

impl Default for IngestCfg {
    fn default() -> Self {
        Self {
            max_parallel_extractors: default_max_parallel_extractors(),
            max_parallel_embeddings: default_max_parallel_embeddings(),
            watch_filesystem: false,
            chunking: ChunkingCfg::defaults(),
            code: IngestCodeCfg::default(),
            image: ImageCfg::defaults(),
            pdf: PdfCfg::defaults(),
        }
    }
}

fn default_max_parallel_extractors() -> u32 {
    2
}
fn default_max_parallel_embeddings() -> u32 {
    1
}

/// p10-1A-1: settings for the code ingest pipeline. All fields have
/// reasonable defaults so the user need not set anything in `config.toml`
/// to get working code ingest.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct IngestCodeCfg {
    /// Generated header sniff. Reads first ~512 bytes, checks 7 markers.
    pub skip_generated_header: bool,
    /// Max byte size per file. Bigger files skipped.
    pub max_file_bytes: u64,
    /// Max line count per file. Bigger files skipped (byte cap checked first).
    pub max_file_lines: u32,
    /// User extra skip globs (gitignore syntax). Applied on top of built-in
    /// + `.gitignore` + `.kebabignore`.
    pub extra_skip_globs: Vec<String>,
    /// AST chunk size cap. Functions/classes longer than this fall back to
    /// paragraph-based split (1A-2 and later).
    pub ast_chunk_max_lines: u32,
    /// Tier 3 fallback chunker: lines per chunk.
    pub fallback_lines_per_chunk: u32,
    /// Tier 3 fallback chunker: line overlap between adjacent chunks.
    pub fallback_lines_overlap: u32,
}

impl Default for IngestCodeCfg {
    fn default() -> Self {
        Self {
            skip_generated_header: true,
            max_file_bytes: 262_144,
            max_file_lines: 5_000,
            extra_skip_globs: vec![],
            ast_chunk_max_lines: 200,
            fallback_lines_per_chunk: 80,
            fallback_lines_overlap: 20,
        }
    }
}

impl Config {
    /// Defaults per design §6.4.
    pub fn defaults() -> Self {
        Self {
            schema_version: crate::migrate::CURRENT_SCHEMA_VERSION,
            workspace: WorkspaceCfg {
                root: Some("~/KnowledgeBase".to_string()),
                exclude: vec![
                    ".git/**".to_string(),
                    "node_modules/**".to_string(),
                    ".obsidian/**".to_string(),
                ],
                sources: vec![],
            },
            storage: StorageCfg {
                data_dir: "${XDG_DATA_HOME:-~/.local/share}/kebab".to_string(),
                sqlite: "{data_dir}/kebab.sqlite".to_string(),
                vector_dir: "{data_dir}/lancedb".to_string(),
                asset_dir: "{data_dir}/assets".to_string(),
                artifact_dir: "{data_dir}/artifacts".to_string(),
                model_dir: "{data_dir}/models".to_string(),
                runs_dir: "{data_dir}/runs".to_string(),
                copy_threshold_mb: 100,
            },
            models: ModelsCfg {
                embedding: EmbeddingModelCfg {
                    provider: "fastembed".to_string(),
                    model: "multilingual-e5-large".to_string(),
                    version: "v1".to_string(),
                    dimensions: 1024,
                    batch_size: 64,
                    num_threads: 0,
                    endpoint: None,
                },
                llm: LlmCfg {
                    provider: "ollama".to_string(),
                    // gemma4 계열 통일 — OCR (P6-2) + caption (P6-3)
                    // 어댑터가 같은 family 사용. 사용자가 더 큰
                    // variant (gemma4:26b 등) 원하면 자기 config.toml
                    // 에서 override. CPU-only / ≤16 GB RAM 환경이면
                    // gemma3:4b 같은 ≤4B Q4 모델 권장 (README 참조).
                    model: "gemma4:e4b".to_string(),
                    context_tokens: 32768,
                    endpoint: "http://127.0.0.1:11434".to_string(),
                    temperature: 0.0,
                    seed: 0,
                    request_timeout_secs: default_llm_request_timeout_secs(),
                },
                nli: NliCfg::defaults(),
            },
            ingest: IngestCfg {
                max_parallel_extractors: 2,
                max_parallel_embeddings: 1,
                watch_filesystem: false,
                chunking: ChunkingCfg::defaults(),
                code: IngestCodeCfg::default(),
                image: ImageCfg::defaults(),
                pdf: PdfCfg::defaults(),
            },
            search: SearchCfg {
                default_k: 10,
                hybrid_fusion: "rrf".to_string(),
                rrf_k: 60,
                snippet_chars: 220,
                cache_capacity: default_cache_capacity(),
                stale_threshold_days: 30,
            },
            rag: RagCfg {
                prompt_template_version: "rag-v4".to_string(),
                score_gate: 0.30,
                explain_default: false,
                max_context_tokens: 8000,
                multi_hop_max_depth: default_multi_hop_max_depth(),
                multi_hop_max_sub_queries_per_iter: default_multi_hop_max_sub_queries_per_iter(),
                multi_hop_max_pool_chunks: default_multi_hop_max_pool_chunks(),
                nli_threshold: default_nli_threshold(),
            },
            ui: UiCfg::defaults(),
            logging: LoggingCfg::default(),
            // p9-fb-05: defaults are not loaded from disk, so no
            // source_dir. Relative `workspace.root` (rare with
            // defaults) falls back to caller `cwd` via the
            // `unwrap_or_else(...)` in `expand_path_with_base`
            // sites — see kebab-app's resolve_workspace_root.
            source_dir: None,
        }
    }

    /// p9-fb-05: read-only accessor for the source-file directory
    /// (where `from_file` / `load` stamped it). Returns `None` for
    /// `Config::defaults()` and other in-memory constructions.
    pub fn source_dir(&self) -> Option<&Path> {
        self.source_dir.as_deref()
    }

    /// p9-fb-05: builder for tests / programmatic callers that need
    /// to pin `source_dir` without going through `from_file`. Returns
    /// `self` so it chains: `Config::defaults().with_source_dir(p)`.
    pub fn with_source_dir(mut self, dir: PathBuf) -> Self {
        self.source_dir = Some(dir);
        self
    }

    /// p9-fb-05: resolve `workspace.root` to an absolute `PathBuf`.
    /// Order:
    /// 1. tilde / env / `${VAR}` substitutions per [`expand_path`].
    /// 2. if still relative, join onto `source_dir` (config file's
    ///    directory) when known, else `cwd`.
    ///
    /// Tilde / absolute / `${VAR}`-rooted inputs ignore `source_dir`.
    /// `Config::defaults()` (which has no `source_dir`) effectively
    /// uses `cwd` for relative inputs — which is the surprising
    /// case spec p9-fb-05 calls out as a foot-gun, but it can only
    /// arise when the user is using defaults AND has a relative
    /// root, which is rare (defaults ship `~/KnowledgeBase`).
    pub fn resolve_workspace_root(&self) -> PathBuf {
        let base = self.source_dir.clone().unwrap_or_else(|| {
            std::env::current_dir().unwrap_or_else(|e| {
                // chroot / deleted-cwd / permission failure: log so a
                // user with an environment problem doesn't silently
                // wonder why their workspace.root resolved to "./root"
                // (which then fails at `create_dir_all` time with a
                // less obvious error).
                tracing::warn!(
                    target: "kebab-config",
                    error = %e,
                    "current_dir() failed; falling back to '.' for workspace.root resolution"
                );
                PathBuf::from(".")
            })
        });
        paths::expand_path_with_base(&self.primary_root_raw(), "", &base)
    }

    /// The raw (unexpanded) string for the *primary* workspace root, used by
    /// [`resolve_workspace_root`](Self::resolve_workspace_root) and any
    /// single-root code path. Order: first `[[workspace.sources]]` entry's
    /// `root` → bare `workspace.root` → `~/KnowledgeBase` default. This keeps
    /// every pre-existing single-root call site working when only `sources`
    /// is declared.
    fn primary_root_raw(&self) -> String {
        if let Some(s) = self.workspace.sources.first() {
            return s.root.clone();
        }
        self.workspace
            .root
            .clone()
            .unwrap_or_else(|| "~/KnowledgeBase".to_string())
    }

    /// The base directory for resolving relative source roots: the config
    /// file's directory when loaded from disk, else the current dir (mirrors
    /// [`resolve_workspace_root`](Self::resolve_workspace_root)).
    fn root_resolution_base(&self) -> PathBuf {
        self.source_dir.clone().unwrap_or_else(|| {
            std::env::current_dir().unwrap_or_else(|e| {
                tracing::warn!(
                    target: "kebab-config",
                    error = %e,
                    "current_dir() failed; falling back to '.' for source root resolution"
                );
                PathBuf::from(".")
            })
        })
    }

    /// Normalized, resolved list of sources to ingest. Always non-empty:
    ///
    /// - If `[[workspace.sources]]` is declared, each entry is returned with
    ///   its `root` expanded and `exclude` merged with `workspace.exclude`.
    /// - Otherwise a single implicit source `id = "default"` is synthesized
    ///   from `workspace.root` (the legacy single-root path).
    ///
    /// This is the single entry point the ingest pipeline iterates over, so
    /// single-root and multi-source configs share one code path.
    pub fn resolved_sources(&self) -> Vec<ResolvedSource> {
        let base = self.root_resolution_base();
        if self.workspace.sources.is_empty() {
            let root = paths::expand_path_with_base(&self.primary_root_raw(), "", &base);
            return vec![ResolvedSource {
                id: DEFAULT_SOURCE_ID.to_string(),
                root,
                exclude: self.workspace.exclude.clone(),
                trust_level: None,
                source_type: None,
            }];
        }
        self.workspace
            .sources
            .iter()
            .map(|s| {
                let root = paths::expand_path_with_base(&s.root, "", &base);
                let mut exclude = self.workspace.exclude.clone();
                exclude.extend(s.exclude.iter().cloned());
                ResolvedSource {
                    id: s.id.clone(),
                    root,
                    exclude,
                    trust_level: s.trust_level,
                    source_type: s.source_type,
                }
            })
            .collect()
    }

    /// Read config from disk and merge env overrides on top of it. If the
    /// file is missing, defaults are used (so `kb doctor` runs with no
    /// prior `kb init`).
    pub fn load(path: Option<&Path>) -> anyhow::Result<Self> {
        let from_disk = match path {
            Some(p) if p.exists() => Self::from_file(p)?,
            Some(p) => {
                return Err(anyhow::Error::new(ConfigNotFound {
                    path: p.to_path_buf(),
                }));
            }
            None => {
                let p = Self::xdg_config_path();
                if p.exists() {
                    Self::from_file(&p)?
                } else {
                    // macOS migration: if the new XDG path is absent but the
                    // old ~/Library/Application Support/kebab/config.toml exists,
                    // copy it to the new location so the user doesn't lose settings.
                    if let Some(legacy) = Self::macos_legacy_config_path() {
                        if legacy.exists() && !p.exists() {
                            if let Some(parent) = p.parent() {
                                let _ = std::fs::create_dir_all(parent);
                            }
                            if std::fs::copy(&legacy, &p).is_ok() {
                                eprintln!(
                                    "kebab: migrated config {} → {}",
                                    legacy.display(),
                                    p.display()
                                );
                                return Self::from_file(&p)
                                    .map(|c| c.apply_env(&std::env::vars().collect()));
                            }
                        }
                    }
                    Self::defaults()
                }
            }
        };
        let env: HashMap<String, String> = std::env::vars().collect();
        Ok(from_disk.apply_env(&env))
    }

    /// Parse a config from `path`. p9-fb-05: also stamps
    /// `source_dir = path.parent()` so relative `workspace.root`
    /// values resolve against the config file's directory rather
    /// than the user's `cwd`.
    pub fn from_file(path: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path).map_err(|e| {
            anyhow::Error::new(ConfigInvalid {
                path: path.to_path_buf(),
                cause: format!("read_failed: {e}"),
            })
        })?;

        // raw `toml::Value` 를 한 번만 파싱해 (1) legacy `workspace.include`
        // deprecation probe (p9-fb-25) 와 (2) `schema_version` 감지(v3 자동변환)
        // 에 함께 쓴다 — config load 는 매 CLI 호출마다 일어나므로 파싱 1회로.
        let probe = toml::from_str::<toml::Value>(&text).ok();

        // p9-fb-25: legacy `workspace.include` 키가 있으면 일회성 deprecation
        // 경고(process-level OnceLock 로 장기 실행 시 로그 도배 방지).
        if probe
            .as_ref()
            .and_then(|v| v.get("workspace"))
            .and_then(|v| v.get("include"))
            .is_some()
        {
            static DEPRECATION_FIRED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
            DEPRECATION_FIRED.get_or_init(|| {
                tracing::warn!(
                    target: "kebab-config",
                    config = %path.display(),
                    "deprecated config: `workspace.include` 필드는 더 이상 사용되지 않습니다 (p9-fb-25, v0.2.1+). 처리 가능한 형식 (md / png / jpg / pdf) 은 extractor 가 자동 결정. config 에서 이 필드를 제거해도 안전 — 더 이상 enforce 안 됨."
                );
            });
        }

        // v3: 파일의 schema_version 이 CURRENT 보다 낮으면 메모리에서 변환한다
        // (디스크 미변경 — 파일 갱신은 `kebab config migrate`). 미변환 v2 파일도
        // 설정 유실 없이 로드(불변식 #3). non-additive relocation(v2→v3) 은
        // serde default forward-compat 로는 커버 안 되므로 반드시 거쳐야 한다.
        let parse_text = {
            let from = probe
                .as_ref()
                .and_then(|v| v.get("schema_version").and_then(toml::Value::as_integer))
                .unwrap_or(1) as u32;
            if from < crate::migrate::CURRENT_SCHEMA_VERSION {
                static MIGRATE_WARNED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
                MIGRATE_WARNED.get_or_init(|| {
                    tracing::warn!(
                        target: "kebab-config",
                        config = %path.display(),
                        from,
                        to = crate::migrate::CURRENT_SCHEMA_VERSION,
                        "config 가 옛 스키마입니다 — 이번 실행은 메모리에서 변환됨. 파일 갱신: `kebab config migrate`."
                    );
                });
                crate::migrate::migrate_document(&text).new_text
            } else {
                text.clone()
            }
        };

        let mut cfg: Self = toml::from_str(&parse_text).map_err(|e| {
            anyhow::Error::new(ConfigInvalid {
                path: path.to_path_buf(),
                cause: format!("parse_failed: {e}"),
            })
        })?;
        cfg.validate_sources().map_err(|cause| {
            anyhow::Error::new(ConfigInvalid {
                path: path.to_path_buf(),
                cause,
            })
        })?;
        cfg.validate_chunking().map_err(|cause| {
            anyhow::Error::new(ConfigInvalid {
                path: path.to_path_buf(),
                cause,
            })
        })?;
        cfg.source_dir = path.parent().map(Path::to_path_buf);
        Ok(cfg)
    }

    /// Validate `[[workspace.sources]]`: every `id` must be non-empty and
    /// unique across the workspace. Empty `sources` (legacy single-root) is
    /// always valid. Returns the failure cause string for `ConfigInvalid`.
    fn validate_sources(&self) -> Result<(), String> {
        let mut seen = std::collections::HashSet::new();
        for s in &self.workspace.sources {
            if s.id.trim().is_empty() {
                return Err("workspace.sources: an entry has an empty `id`".to_string());
            }
            // rag-provenance-label: source.id renders verbatim into the RAG
            // prompt context header (`[#n] source=<id> trust=… …`), where the
            // newline is the per-chunk line terminator and `[#n]` is the
            // citation grammar. Constrain ids to a safe set so a stray newline
            // / bracket can't split or spoof a chunk header. Also keeps the
            // `--source <id>` CLI filter ergonomics clean. (Single-user local
            // tool — config is self-authored, so this is defense-in-depth.)
            if !s.id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')) {
                return Err(format!(
                    "workspace.sources: source id `{}` has invalid characters \
                     (allowed: ASCII letters, digits, `.` `_` `-`)",
                    s.id
                ));
            }
            if s.root.trim().is_empty() {
                return Err(format!(
                    "workspace.sources: source `{}` has an empty `root`",
                    s.id
                ));
            }
            if !seen.insert(s.id.as_str()) {
                return Err(format!(
                    "workspace.sources: duplicate source id `{}` (ids must be unique)",
                    s.id
                ));
            }
        }
        Ok(())
    }

    /// Validate `[ingest.chunking]` budget fields. A misconfigured `0`
    /// (or any nonsensical combination) would otherwise be silently
    /// absorbed by the chunker's internal `budget.max(1)` clamp into a
    /// flood of 3-byte chunks — catastrophic index bloat, no error. The
    /// floors below reject the clearly-broken configs at load time with a
    /// clear cause, mirroring [`validate_sources`](Self::validate_sources).
    fn validate_chunking(&self) -> Result<(), String> {
        let c = &self.ingest.chunking;
        if c.target_tokens < MIN_CHUNK_TOKENS {
            return Err(format!(
                "ingest.chunking.target_tokens = {} is too small (min {MIN_CHUNK_TOKENS}); \
                 a chunk target this low fragments every document",
                c.target_tokens
            ));
        }
        if c.overlap_tokens >= c.target_tokens {
            return Err(format!(
                "ingest.chunking.overlap_tokens = {} must be < target_tokens = {} \
                 (overlap ≥ target loops / duplicates content)",
                c.overlap_tokens, c.target_tokens
            ));
        }
        if c.max_chunk_tokens < c.target_tokens {
            return Err(format!(
                "ingest.chunking.max_chunk_tokens = {} must be ≥ target_tokens = {} \
                 (a per-chunk cap below the soft target would split every normal chunk)",
                c.max_chunk_tokens, c.target_tokens
            ));
        }
        Ok(())
    }

    /// Apply `KEBAB_<SECTION>_<KEY>` env overrides. Unknown keys are ignored.
    ///
    /// The mapping is an explicit grep-friendly whitelist — one match arm
    /// per leaf key in `Config`. Booleans accept `1` / `true` / `yes`
    /// (case-insensitive) for true and anything else for false. Numeric
    /// keys silently keep their prior value if the env value fails to
    /// parse, so a malformed `KEBAB_*` cannot crash startup.
    pub fn apply_env(mut self, env: &HashMap<String, String>) -> Self {
        for (k, v) in env {
            if !k.starts_with("KEBAB_") {
                continue;
            }
            match k.as_str() {
                // workspace
                "KEBAB_WORKSPACE_ROOT" => self.workspace.root = Some(v.clone()),

                // storage
                "KEBAB_STORAGE_DATA_DIR" => self.storage.data_dir = v.clone(),
                "KEBAB_STORAGE_SQLITE" => self.storage.sqlite = v.clone(),
                "KEBAB_STORAGE_VECTOR_DIR" => self.storage.vector_dir = v.clone(),
                "KEBAB_STORAGE_ASSET_DIR" => self.storage.asset_dir = v.clone(),
                "KEBAB_STORAGE_ARTIFACT_DIR" => self.storage.artifact_dir = v.clone(),
                "KEBAB_STORAGE_MODEL_DIR" => self.storage.model_dir = v.clone(),
                "KEBAB_STORAGE_RUNS_DIR" => self.storage.runs_dir = v.clone(),
                "KEBAB_STORAGE_COPY_THRESHOLD_MB" => {
                    if let Ok(n) = v.parse::<u64>() {
                        self.storage.copy_threshold_mb = n;
                    }
                }

                // indexing
                "KEBAB_INDEXING_MAX_PARALLEL_EXTRACTORS" => {
                    if let Ok(n) = v.parse::<u32>() {
                        self.ingest.max_parallel_extractors = n;
                    }
                }
                "KEBAB_INDEXING_MAX_PARALLEL_EMBEDDINGS" => {
                    if let Ok(n) = v.parse::<u32>() {
                        self.ingest.max_parallel_embeddings = n;
                    }
                }
                "KEBAB_INDEXING_WATCH_FILESYSTEM" => {
                    self.ingest.watch_filesystem = parse_bool(v);
                }

                // chunking
                "KEBAB_CHUNKING_TARGET_TOKENS" => {
                    if let Ok(n) = v.parse::<usize>() {
                        self.ingest.chunking.target_tokens = n;
                    }
                }
                "KEBAB_CHUNKING_OVERLAP_TOKENS" => {
                    if let Ok(n) = v.parse::<usize>() {
                        self.ingest.chunking.overlap_tokens = n;
                    }
                }
                "KEBAB_CHUNKING_RESPECT_MARKDOWN_HEADINGS" => {
                    self.ingest.chunking.respect_markdown_headings = parse_bool(v);
                }
                "KEBAB_CHUNKING_CHUNKER_VERSION" => self.ingest.chunking.chunker_version = v.clone(),
                "KEBAB_CHUNKING_MAX_CHUNK_TOKENS" => {
                    if let Ok(n) = v.parse::<usize>() {
                        self.ingest.chunking.max_chunk_tokens = n;
                    }
                }

                // models.embedding
                "KEBAB_MODELS_EMBEDDING_PROVIDER" => self.models.embedding.provider = v.clone(),
                "KEBAB_MODELS_EMBEDDING_MODEL" => self.models.embedding.model = v.clone(),
                "KEBAB_MODELS_EMBEDDING_VERSION" => self.models.embedding.version = v.clone(),
                "KEBAB_MODELS_EMBEDDING_DIMENSIONS" => {
                    if let Ok(n) = v.parse::<usize>() {
                        self.models.embedding.dimensions = n;
                    }
                }
                "KEBAB_MODELS_EMBEDDING_BATCH_SIZE" => {
                    if let Ok(n) = v.parse::<usize>() {
                        self.models.embedding.batch_size = n;
                    }
                }
                "KEBAB_MODELS_EMBEDDING_NUM_THREADS" => {
                    if let Ok(n) = v.parse::<u32>() {
                        self.models.embedding.num_threads = n;
                    }
                }
                "KEBAB_MODELS_EMBEDDING_ENDPOINT" => {
                    // Empty value → None (= fall back to models.llm.endpoint),
                    // mirroring the OCR endpoint override semantics.
                    self.models.embedding.endpoint =
                        if v.is_empty() { None } else { Some(v.clone()) };
                }

                // models.llm
                "KEBAB_MODELS_LLM_PROVIDER" => self.models.llm.provider = v.clone(),
                "KEBAB_MODELS_LLM_MODEL" => self.models.llm.model = v.clone(),
                "KEBAB_MODELS_LLM_CONTEXT_TOKENS" => {
                    if let Ok(n) = v.parse::<usize>() {
                        self.models.llm.context_tokens = n;
                    }
                }
                "KEBAB_MODELS_LLM_ENDPOINT" => self.models.llm.endpoint = v.clone(),
                "KEBAB_MODELS_LLM_TEMPERATURE" => {
                    if let Ok(f) = v.parse::<f32>() {
                        self.models.llm.temperature = f;
                    }
                }
                "KEBAB_MODELS_LLM_SEED" => {
                    if let Ok(n) = v.parse::<u64>() {
                        self.models.llm.seed = n;
                    }
                }
                "KEBAB_MODELS_LLM_REQUEST_TIMEOUT_SECS" => {
                    if let Ok(n) = v.parse::<u64>() {
                        self.models.llm.request_timeout_secs = n;
                    }
                }

                // models.nli (p9-fb-41 PR-9c-1)
                "KEBAB_MODELS_NLI_MODEL" => self.models.nli.model = v.clone(),
                "KEBAB_MODELS_NLI_PROVIDER" => self.models.nli.provider = v.clone(),

                // search
                "KEBAB_SEARCH_DEFAULT_K" => {
                    if let Ok(n) = v.parse::<usize>() {
                        self.search.default_k = n;
                    }
                }
                "KEBAB_SEARCH_HYBRID_FUSION" => self.search.hybrid_fusion = v.clone(),
                "KEBAB_SEARCH_RRF_K" => {
                    if let Ok(n) = v.parse::<u32>() {
                        self.search.rrf_k = n;
                    }
                }
                "KEBAB_SEARCH_SNIPPET_CHARS" => {
                    if let Ok(n) = v.parse::<usize>() {
                        self.search.snippet_chars = n;
                    }
                }
                "KEBAB_SEARCH_STALE_THRESHOLD_DAYS" => {
                    if let Ok(n) = v.parse::<u32>() {
                        self.search.stale_threshold_days = n;
                    }
                }

                // rag
                "KEBAB_RAG_PROMPT_TEMPLATE_VERSION" => {
                    self.rag.prompt_template_version = v.clone();
                }
                "KEBAB_RAG_SCORE_GATE" => {
                    if let Ok(f) = v.parse::<f32>() {
                        self.rag.score_gate = f;
                    }
                }
                "KEBAB_RAG_EXPLAIN_DEFAULT" => {
                    self.rag.explain_default = parse_bool(v);
                }
                "KEBAB_RAG_MAX_CONTEXT_TOKENS" => {
                    if let Ok(n) = v.parse::<usize>() {
                        self.rag.max_context_tokens = n;
                    }
                }
                "KEBAB_RAG_MULTI_HOP_MAX_DEPTH" => {
                    if let Ok(n) = v.parse::<u32>() {
                        self.rag.multi_hop_max_depth = n;
                    }
                }
                "KEBAB_RAG_MULTI_HOP_MAX_SUB_QUERIES_PER_ITER" => {
                    if let Ok(n) = v.parse::<u32>() {
                        self.rag.multi_hop_max_sub_queries_per_iter = n;
                    }
                }
                "KEBAB_RAG_MULTI_HOP_MAX_POOL_CHUNKS" => {
                    if let Ok(n) = v.parse::<u32>() {
                        self.rag.multi_hop_max_pool_chunks = n;
                    }
                }
                // p9-fb-41 PR-9c-1: NLI gate threshold. Parse failure
                // emits a `tracing::warn!` (not silent like the other
                // numeric env overrides) because this knob gates the
                // NLI verification entirely — a malformed env value
                // would silently disable a security-flavored gate the
                // user thought they enabled, which is the failure mode
                // most worth surfacing. The default (`0.0`) survives
                // on parse failure so behaviour stays well-defined.
                "KEBAB_RAG_NLI_THRESHOLD" => match v.parse::<f32>() {
                    Ok(f) => self.rag.nli_threshold = f,
                    Err(e) => tracing::warn!(
                        target: "kebab-config",
                        env_key = "KEBAB_RAG_NLI_THRESHOLD",
                        env_value = %v,
                        error = %e,
                        "invalid KEBAB_RAG_NLI_THRESHOLD; keeping prior value (0.0 = NLI gate disabled)"
                    ),
                },

                // image.ocr
                "KEBAB_IMAGE_OCR_ENABLED" => {
                    self.ingest.image.ocr.enabled = parse_bool(v);
                }
                "KEBAB_IMAGE_OCR_ENGINE" => self.ingest.image.ocr.engine = v.clone(),
                "KEBAB_IMAGE_OCR_MODEL" => self.ingest.image.ocr.model = v.clone(),
                "KEBAB_IMAGE_OCR_ENDPOINT" => {
                    // Empty env value is treated the same as "fall back
                    // to models.llm.endpoint" — i.e. set None.
                    self.ingest.image.ocr.endpoint = if v.is_empty() { None } else { Some(v.clone()) };
                }
                "KEBAB_IMAGE_OCR_LANGUAGES" => {
                    // Comma-separated list, e.g. "eng,kor".
                    self.ingest.image.ocr.languages = v
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                }
                "KEBAB_IMAGE_OCR_MAX_PIXELS" => {
                    if let Ok(n) = v.parse::<u32>() {
                        self.ingest.image.ocr.max_pixels = n;
                    }
                }
                "KEBAB_IMAGE_OCR_REQUEST_TIMEOUT_SECS" => {
                    if let Ok(n) = v.parse::<u64>() {
                        self.ingest.image.ocr.request_timeout_secs = n;
                    }
                }
                // paddle-onnx engine overrides (v0.27.0). Empty string → None
                // (fall back to bundled / KEBAB_IMAGE_OCR_MODEL_DIR).
                "KEBAB_IMAGE_OCR_DET_MODEL" => {
                    self.ingest.image.ocr.det_model =
                        if v.is_empty() { None } else { Some(v.clone()) };
                }
                "KEBAB_IMAGE_OCR_REC_MODEL" => {
                    self.ingest.image.ocr.rec_model =
                        if v.is_empty() { None } else { Some(v.clone()) };
                }
                "KEBAB_IMAGE_OCR_DICT" => {
                    self.ingest.image.ocr.dict = if v.is_empty() { None } else { Some(v.clone()) };
                }
                "KEBAB_IMAGE_OCR_SCORE_THRESH" => {
                    if let Ok(f) = v.parse::<f32>() {
                        self.ingest.image.ocr.score_thresh = f;
                    }
                }
                "KEBAB_IMAGE_OCR_UNCLIP_RATIO" => {
                    if let Ok(f) = v.parse::<f32>() {
                        self.ingest.image.ocr.unclip_ratio = f;
                    }
                }
                "KEBAB_IMAGE_OCR_MAX_BOXES" => {
                    if let Ok(n) = v.parse::<usize>() {
                        self.ingest.image.ocr.max_boxes = n;
                    }
                }

                // image.caption (P6-3)
                "KEBAB_IMAGE_CAPTION_ENABLED" => {
                    self.ingest.image.caption.enabled = parse_bool(v);
                }
                "KEBAB_IMAGE_CAPTION_MAX_PIXELS" => {
                    if let Ok(n) = v.parse::<u32>() {
                        self.ingest.image.caption.max_pixels = n;
                    }
                }
                "KEBAB_IMAGE_CAPTION_PROMPT_TEMPLATE_VERSION" => {
                    self.ingest.image.caption.prompt_template_version = v.clone();
                }

                // pdf.ocr (v0.20.0 sub-item 1)
                "KEBAB_PDF_OCR_ENABLED" => self.ingest.pdf.ocr.enabled = parse_bool(v),
                "KEBAB_PDF_OCR_ALWAYS_ON" => self.ingest.pdf.ocr.always_on = parse_bool(v),
                "KEBAB_PDF_OCR_ENGINE" => self.ingest.pdf.ocr.engine = v.clone(),
                "KEBAB_PDF_OCR_MODEL" => self.ingest.pdf.ocr.model = v.clone(),
                "KEBAB_PDF_OCR_ENDPOINT" => {
                    self.ingest.pdf.ocr.endpoint = if v.is_empty() { None } else { Some(v.clone()) };
                }
                "KEBAB_PDF_OCR_LANGUAGES" => {
                    self.ingest.pdf.ocr.languages = v
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                }
                "KEBAB_PDF_OCR_MAX_PIXELS" => {
                    if let Ok(n) = v.parse::<u32>() {
                        self.ingest.pdf.ocr.max_pixels = n;
                    }
                }
                "KEBAB_PDF_OCR_REQUEST_TIMEOUT_SECS" => {
                    if let Ok(n) = v.parse::<u64>() {
                        self.ingest.pdf.ocr.request_timeout_secs = n;
                    }
                }
                "KEBAB_PDF_OCR_VALID_RATIO_THRESHOLD" => {
                    if let Ok(n) = v.parse::<f32>() {
                        self.ingest.pdf.ocr.valid_ratio_threshold = n.clamp(0.0, 1.0);
                    }
                }
                "KEBAB_PDF_OCR_MIN_CHAR_COUNT" => {
                    if let Ok(n) = v.parse::<u32>() {
                        self.ingest.pdf.ocr.min_char_count = n;
                    }
                }
                "KEBAB_PDF_OCR_LANG_HINT" => {
                    self.ingest.pdf.ocr.lang_hint = if v.is_empty() { None } else { Some(v.clone()) };
                }
                // pdf paddle-onnx engine overrides (v3). image.ocr paddle 패턴 복제.
                // Empty string → None (fall back to bundled / KEBAB_IMAGE_OCR_MODEL_DIR).
                "KEBAB_PDF_OCR_DET_MODEL" => {
                    self.ingest.pdf.ocr.det_model =
                        if v.is_empty() { None } else { Some(v.clone()) };
                }
                "KEBAB_PDF_OCR_REC_MODEL" => {
                    self.ingest.pdf.ocr.rec_model =
                        if v.is_empty() { None } else { Some(v.clone()) };
                }
                "KEBAB_PDF_OCR_DICT" => {
                    self.ingest.pdf.ocr.dict = if v.is_empty() { None } else { Some(v.clone()) };
                }
                "KEBAB_PDF_OCR_SCORE_THRESH" => {
                    if let Ok(f) = v.parse::<f32>() {
                        self.ingest.pdf.ocr.score_thresh = f;
                    }
                }
                "KEBAB_PDF_OCR_UNCLIP_RATIO" => {
                    if let Ok(f) = v.parse::<f32>() {
                        self.ingest.pdf.ocr.unclip_ratio = f;
                    }
                }
                "KEBAB_PDF_OCR_MAX_BOXES" => {
                    if let Ok(n) = v.parse::<usize>() {
                        self.ingest.pdf.ocr.max_boxes = n;
                    }
                }

                // Unknown KEBAB_* keys are silently ignored — see
                // `env_unknown_key_is_ignored` test.
                _ => {}
            }
        }
        self
    }

    /// `~/.config/kebab/config.toml` (honors `XDG_CONFIG_HOME`).
    pub fn xdg_config_path() -> PathBuf {
        if let Ok(custom) = std::env::var("XDG_CONFIG_HOME") {
            if !custom.is_empty() {
                return PathBuf::from(custom).join("kebab").join("config.toml");
            }
        }
        // Always use XDG-standard ~/.config regardless of platform.
        // macOS dirs::config_dir() returns ~/Library/Application Support which
        // collides with data_dir() — DataOnly reset would delete config too.
        match dirs::home_dir() {
            Some(h) => h.join(".config").join("kebab").join("config.toml"),
            None => PathBuf::from("./kebab/config.toml"),
        }
    }

    /// `~/.local/share/kebab` (honors `XDG_DATA_HOME`).
    pub fn xdg_data_dir() -> PathBuf {
        if let Ok(custom) = std::env::var("XDG_DATA_HOME") {
            if !custom.is_empty() {
                return PathBuf::from(custom).join("kebab");
            }
        }
        // Always use XDG-standard ~/.local/share regardless of platform.
        match dirs::home_dir() {
            Some(h) => h.join(".local").join("share").join("kebab"),
            None => PathBuf::from("./kebab-data"),
        }
    }

    /// `~/.cache/kebab` (honors `XDG_CACHE_HOME`).
    pub fn xdg_cache_dir() -> PathBuf {
        if let Ok(custom) = std::env::var("XDG_CACHE_HOME") {
            if !custom.is_empty() {
                return PathBuf::from(custom).join("kebab");
            }
        }
        // Always use XDG-standard ~/.cache regardless of platform.
        match dirs::home_dir() {
            Some(h) => h.join(".cache").join("kebab"),
            None => PathBuf::from("./kebab-cache"),
        }
    }

    /// `~/.local/state/kebab` (honors `XDG_STATE_HOME`).
    pub fn xdg_state_dir() -> PathBuf {
        if let Ok(custom) = std::env::var("XDG_STATE_HOME") {
            if !custom.is_empty() {
                return PathBuf::from(custom).join("kebab");
            }
        }
        // `dirs` doesn't expose state_dir on all platforms; fall back to
        // `$HOME/.local/state/kebab` if XDG_STATE_HOME is unset.
        if let Some(home) = dirs::home_dir() {
            return home.join(".local").join("state").join("kebab");
        }
        PathBuf::from("./kebab-state")
    }

    /// macOS legacy config path: `~/Library/Application Support/kebab/config.toml`.
    /// Returns `None` on non-macOS or when home dir is unavailable.
    /// Used for one-time migration to the XDG-standard location.
    fn macos_legacy_config_path() -> Option<PathBuf> {
        #[cfg(target_os = "macos")]
        {
            dirs::home_dir().map(|h| {
                h.join("Library")
                    .join("Application Support")
                    .join("kebab")
                    .join("config.toml")
            })
        }
        #[cfg(not(target_os = "macos"))]
        {
            None
        }
    }
}

/// Parse a permissive boolean — `1` / `true` / `yes` (case-insensitive)
/// for true, anything else for false. Used by `apply_env` for boolean
/// leaves of `Config`.
fn parse_bool(s: &str) -> bool {
    matches!(s.to_ascii_lowercase().as_str(), "1" | "true" | "yes")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Legacy TOML fixture written before the `request_timeout_secs`
    /// knobs (LLM in v0.17.1, OCR follow-up) existed. Shared by
    /// `legacy_config_without_request_timeout_secs_uses_default`
    /// (LLM-side) and `legacy_config_without_ocr_request_timeout_secs_uses_default`
    /// (OCR-side) so both invariants pin against the same on-disk
    /// shape — schema drift in the legacy form only needs one edit.
    const LEGACY_PRE_TIMEOUT_TOML: &str = r#"
schema_version = 1

[workspace]
root = "/tmp/x"
exclude = []

[storage]
data_dir = "/tmp/x"
sqlite = "/tmp/x/kebab.sqlite"
vector_dir = "/tmp/x/lancedb"
asset_dir = "/tmp/x/assets"
artifact_dir = "/tmp/x/artifacts"
model_dir = "/tmp/x/models"
runs_dir = "/tmp/x/runs"
copy_threshold_mb = 100

[indexing]
max_parallel_extractors = 2
max_parallel_embeddings = 1
watch_filesystem = false

[chunking]
target_tokens = 500
overlap_tokens = 80
respect_markdown_headings = true
chunker_version = "md-heading-v1"

[models.embedding]
provider = "fastembed"
model = "multilingual-e5-large"
version = "v1"
dimensions = 1024
batch_size = 64

[models.llm]
provider = "ollama"
model = "gemma3:4b"
context_tokens = 4096
endpoint = "http://127.0.0.1:11434"
temperature = 0.0
seed = 0

[search]
default_k = 10
hybrid_fusion = "rrf"
rrf_k = 60
snippet_chars = 220

[rag]
prompt_template_version = "rag-v3"
score_gate = 0.3
explain_default = false
max_context_tokens = 8000

[image.ocr]
enabled = false
engine = "ollama-vision"
model = "gemma3:4b"
languages = ["eng"]
max_pixels = 1600

[image.caption]
enabled = false
max_pixels = 768
prompt_template_version = "caption-v1"

[ui]
theme = "dark"
"#;

    #[test]
    fn defaults_are_serde_roundtrip_stable() {
        let c = Config::defaults();
        let toml_text = toml::to_string(&c).unwrap();
        let back: Config = toml::from_str(&toml_text).unwrap();
        assert_eq!(c, back);
    }

    /// 불변식 #3: `from_file` 이 v2 파일을 디스크 미변경으로 메모리에서 v3
    /// 변환 — 미변환 v2 파일도 설정 유실 0.
    #[test]
    fn from_file_auto_migrates_v2_in_memory() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("config.toml");
        std::fs::write(
            &p,
            "\
schema_version = 2

[workspace]
root = \"/my/notes\"
exclude = []

[chunking]
target_tokens = 777

[image.ocr]
enabled = true
engine = \"ollama-vision\"
model = \"gemma4:e4b\"
languages = [\"kor\"]
max_pixels = 1600
",
        )
        .unwrap();
        let c = Config::from_file(&p).expect("v2 auto-migrate load");
        // 사용자 v2 값이 새 경로로 살아있어야(기본값 유실 X).
        assert_eq!(c.ingest.chunking.target_tokens, 777);
        assert!(c.ingest.image.ocr.enabled);
        assert_eq!(c.ingest.image.ocr.languages, vec!["kor"]);
        // 디스크 파일은 안 바뀜(여전히 schema_version = 2 + [chunking]).
        let on_disk = std::fs::read_to_string(&p).unwrap();
        assert!(
            on_disk.contains("schema_version = 2"),
            "파일이 변경됨:\n{on_disk}"
        );
        assert!(on_disk.contains("[chunking]"), "파일이 변경됨:\n{on_disk}");
    }

    #[test]
    fn v3_layout_nests_media_under_ingest() {
        let c = Config::defaults();
        // 새 경로가 컴파일·접근 가능해야 한다.
        assert_eq!(c.ingest.max_parallel_extractors, 2);
        assert_eq!(c.ingest.chunking.target_tokens, 500);
        assert_eq!(c.ingest.code.max_file_bytes, 262_144);
        assert_eq!(c.ingest.image.ocr.engine, "ollama-vision");
        assert_eq!(c.ingest.image.caption.max_pixels, 768);
        assert_eq!(c.ingest.pdf.ocr.model, "qwen2.5vl:3b");
        // pdf paddle 대칭 키 존재 + 기본값.
        assert_eq!(c.ingest.pdf.ocr.score_thresh, 0.3);
        assert_eq!(c.ingest.pdf.ocr.max_boxes, 1000);
        assert!(c.ingest.pdf.ocr.det_model.is_none());
    }

    #[test]
    fn defaults_match_design_64_score_gate() {
        let c = Config::defaults();
        assert_eq!(c.rag.score_gate, 0.30);
        assert_eq!(c.ingest.chunking.target_tokens, 500);
        assert_eq!(c.models.embedding.model, "multilingual-e5-large");
        assert_eq!(c.models.embedding.dimensions, 1024);
        assert_eq!(c.search.rrf_k, 60);
    }

    #[test]
    fn defaults_rag_prompt_template_version_is_rag_v4() {
        let c = Config::defaults();
        assert_eq!(c.rag.prompt_template_version, "rag-v4");
    }

    #[test]
    fn env_override_score_gate() {
        let mut env = HashMap::new();
        env.insert("KEBAB_RAG_SCORE_GATE".to_string(), "0.5".to_string());
        let c = Config::defaults().apply_env(&env);
        assert!((c.rag.score_gate - 0.5).abs() < 1e-6);
    }

    #[test]
    fn env_override_search_k() {
        let mut env = HashMap::new();
        env.insert("KEBAB_SEARCH_DEFAULT_K".to_string(), "25".to_string());
        let c = Config::defaults().apply_env(&env);
        assert_eq!(c.search.default_k, 25);
    }

    /// 불변식 #2: env override 이름(LHS) 100% 보존 — struct 경로가 바뀌어도
    /// 기존 `KEBAB_*` 스크립트가 새 경로로 대입되어 무파손.
    #[test]
    fn env_names_preserved_target_new_paths() {
        let mut env = HashMap::new();
        env.insert("KEBAB_CHUNKING_TARGET_TOKENS".into(), "640".into());
        env.insert("KEBAB_INDEXING_MAX_PARALLEL_EXTRACTORS".into(), "6".into());
        env.insert("KEBAB_IMAGE_OCR_ENABLED".into(), "true".into());
        env.insert("KEBAB_PDF_OCR_ENGINE".into(), "paddle-onnx".into());
        let c = Config::defaults().apply_env(&env);
        assert_eq!(c.ingest.chunking.target_tokens, 640);
        assert_eq!(c.ingest.max_parallel_extractors, 6);
        assert!(c.ingest.image.ocr.enabled);
        assert_eq!(c.ingest.pdf.ocr.engine, "paddle-onnx");
    }

    #[test]
    fn env_pdf_paddle_symmetric_overrides() {
        let mut env = HashMap::new();
        env.insert("KEBAB_PDF_OCR_DET_MODEL".into(), "/d.onnx".into());
        env.insert("KEBAB_PDF_OCR_SCORE_THRESH".into(), "0.4".into());
        env.insert("KEBAB_PDF_OCR_MAX_BOXES".into(), "500".into());
        let c = Config::defaults().apply_env(&env);
        assert_eq!(c.ingest.pdf.ocr.det_model.as_deref(), Some("/d.onnx"));
        assert!((c.ingest.pdf.ocr.score_thresh - 0.4).abs() < 1e-6);
        assert_eq!(c.ingest.pdf.ocr.max_boxes, 500);
    }

    #[test]
    fn env_unknown_key_is_ignored() {
        let baseline = Config::defaults();
        let mut env = HashMap::new();
        env.insert("KEBAB_NOPE_FOO".to_string(), "garbage".to_string());
        let c = Config::defaults().apply_env(&env);
        assert_eq!(c, baseline);
    }

    #[test]
    fn env_overrides_chunking_target_tokens() {
        let mut env = HashMap::new();
        env.insert(
            "KEBAB_CHUNKING_TARGET_TOKENS".to_string(),
            "777".to_string(),
        );
        let c = Config::defaults().apply_env(&env);
        assert_eq!(c.ingest.chunking.target_tokens, 777);
    }

    #[test]
    fn env_overrides_models_llm_endpoint_and_temperature() {
        let mut env = HashMap::new();
        env.insert(
            "KEBAB_MODELS_LLM_ENDPOINT".to_string(),
            "http://10.0.0.1:11434".to_string(),
        );
        env.insert(
            "KEBAB_MODELS_LLM_TEMPERATURE".to_string(),
            "0.7".to_string(),
        );
        let c = Config::defaults().apply_env(&env);
        assert_eq!(c.models.llm.endpoint, "http://10.0.0.1:11434");
        assert!((c.models.llm.temperature - 0.7).abs() < 1e-6);
    }

    /// v0.17.0 post-dogfood: matches the legacy hard-coded 300s cap so
    /// existing configs that omit the new field are not affected.
    #[test]
    fn default_llm_request_timeout_secs_is_300() {
        assert_eq!(Config::defaults().models.llm.request_timeout_secs, 300);
    }

    #[test]
    fn env_overrides_models_llm_request_timeout_secs() {
        let mut env = HashMap::new();
        env.insert(
            "KEBAB_MODELS_LLM_REQUEST_TIMEOUT_SECS".to_string(),
            "1200".to_string(),
        );
        let c = Config::defaults().apply_env(&env);
        assert_eq!(c.models.llm.request_timeout_secs, 1200);
    }

    /// v0.17.0 post-dogfood: a config file written before the field
    /// existed (no `request_timeout_secs` key) must still parse and fall
    /// back to the 300s default — backwards-compat invariant. Fixture
    /// shared with the OCR-side invariant via [`LEGACY_PRE_TIMEOUT_TOML`].
    #[test]
    fn legacy_config_without_request_timeout_secs_uses_default() {
        let c: Config = toml::from_str(LEGACY_PRE_TIMEOUT_TOML).expect("parse legacy config");
        assert_eq!(c.models.llm.request_timeout_secs, 300);
    }

    #[test]
    fn env_overrides_indexing_watch_filesystem_bool() {
        let mut env = HashMap::new();
        env.insert(
            "KEBAB_INDEXING_WATCH_FILESYSTEM".to_string(),
            "true".to_string(),
        );
        let c = Config::defaults().apply_env(&env);
        assert!(c.ingest.watch_filesystem);
    }

    #[test]
    fn image_ocr_defaults_disabled_with_ollama_vision() {
        let c = Config::defaults();
        assert!(!c.ingest.image.ocr.enabled);
        assert_eq!(c.ingest.image.ocr.engine, "ollama-vision");
        assert_eq!(c.ingest.image.ocr.model, "gemma4:e4b");
        assert_eq!(c.ingest.image.ocr.languages, vec!["eng", "kor"]);
        assert_eq!(c.ingest.image.ocr.max_pixels, 1600);
    }

    /// v0.17.2 post-dogfood: matches the legacy hard-coded 300s cap so
    /// existing configs that omit the new field keep behaving identically.
    #[test]
    fn default_ocr_request_timeout_secs_is_300() {
        assert_eq!(Config::defaults().ingest.image.ocr.request_timeout_secs, 300);
    }

    #[test]
    fn env_overrides_image_ocr_request_timeout_secs() {
        let mut env = HashMap::new();
        env.insert(
            "KEBAB_IMAGE_OCR_REQUEST_TIMEOUT_SECS".to_string(),
            "900".to_string(),
        );
        let c = Config::defaults().apply_env(&env);
        assert_eq!(c.ingest.image.ocr.request_timeout_secs, 900);
    }

    /// post-v0.17.1 dogfood: a config file written before the OCR
    /// timeout field existed must still parse and fall back to the
    /// 300s default — backwards-compat invariant. Fixture shared
    /// with the LLM-side invariant via [`LEGACY_PRE_TIMEOUT_TOML`].
    #[test]
    fn legacy_config_without_ocr_request_timeout_secs_uses_default() {
        let c: Config = toml::from_str(LEGACY_PRE_TIMEOUT_TOML).expect("parse legacy config");
        assert_eq!(c.ingest.image.ocr.request_timeout_secs, 300);
    }

    // ── p9-fb-41: multi-hop RAG knobs ────────────────────────────────────

    #[test]
    fn default_multi_hop_max_depth_is_3() {
        assert_eq!(Config::defaults().rag.multi_hop_max_depth, 3);
    }

    #[test]
    fn default_multi_hop_max_sub_queries_per_iter_is_5() {
        assert_eq!(Config::defaults().rag.multi_hop_max_sub_queries_per_iter, 5);
    }

    #[test]
    fn default_multi_hop_max_pool_chunks_is_15() {
        // v0.18 dogfood (HOTFIXES 2026-05-25 fb-41 post-PR-7) tuned
        // this down from 30 → 15 to keep the synthesize prompt tight
        // enough for gemma3:4b to follow the citation rule.
        assert_eq!(Config::defaults().rag.multi_hop_max_pool_chunks, 15);
    }

    #[test]
    fn env_overrides_multi_hop_knobs() {
        let mut env = HashMap::new();
        env.insert("KEBAB_RAG_MULTI_HOP_MAX_DEPTH".to_string(), "5".to_string());
        env.insert(
            "KEBAB_RAG_MULTI_HOP_MAX_SUB_QUERIES_PER_ITER".to_string(),
            "7".to_string(),
        );
        env.insert(
            "KEBAB_RAG_MULTI_HOP_MAX_POOL_CHUNKS".to_string(),
            "50".to_string(),
        );
        let c = Config::defaults().apply_env(&env);
        assert_eq!(c.rag.multi_hop_max_depth, 5);
        assert_eq!(c.rag.multi_hop_max_sub_queries_per_iter, 7);
        assert_eq!(c.rag.multi_hop_max_pool_chunks, 50);
    }

    /// post-PR-3 fb-41: a config file written before the multi-hop
    /// knobs existed must still parse and fall back to the documented
    /// defaults — backwards-compat invariant. Fixture shared with the
    /// LLM / OCR timeout invariants via [`LEGACY_PRE_TIMEOUT_TOML`]
    /// (that fixture also predates the multi_hop_* fields).
    #[test]
    fn legacy_config_without_multi_hop_knobs_uses_defaults() {
        let c: Config = toml::from_str(LEGACY_PRE_TIMEOUT_TOML).expect("parse legacy config");
        assert_eq!(c.rag.multi_hop_max_depth, 3);
        assert_eq!(c.rag.multi_hop_max_sub_queries_per_iter, 5);
        // v0.18 dogfood (post-PR-7): pool default 30 → 15.
        assert_eq!(c.rag.multi_hop_max_pool_chunks, 15);
    }

    // ── p9-fb-41 PR-9c-1: NLI verification knobs ─────────────────────────

    #[test]
    fn default_nli_threshold_is_zero() {
        // Spec §2.6: NLI gate disabled by default — verification is
        // opt-in. `0.0` keeps multi-hop behavior identical to PR-3b.
        assert_eq!(Config::defaults().rag.nli_threshold, 0.0);
    }

    #[test]
    fn default_nli_model_is_xenova_mdeberta() {
        // Pin the default model id so a refactor that touches NliCfg
        // can't silently flip to a different verifier model.
        assert_eq!(
            Config::defaults().models.nli.model,
            "Xenova/mDeBERTa-v3-base-xnli-multilingual-nli-2mil7"
        );
        assert_eq!(Config::defaults().models.nli.provider, "onnx");
    }

    /// A config file written before the `[models.nli]` / `nli_threshold`
    /// keys existed must still parse and fall back to the documented
    /// defaults. Fixture shared via [`LEGACY_PRE_TIMEOUT_TOML`] (predates
    /// all PR-9c-1 fields).
    #[test]
    fn legacy_config_without_nli_uses_defaults() {
        let c: Config = toml::from_str(LEGACY_PRE_TIMEOUT_TOML).expect("parse legacy config");
        assert_eq!(c.rag.nli_threshold, 0.0);
        assert_eq!(
            c.models.nli.model,
            "Xenova/mDeBERTa-v3-base-xnli-multilingual-nli-2mil7"
        );
        assert_eq!(c.models.nli.provider, "onnx");
    }

    #[test]
    fn env_override_nli_threshold() {
        let mut env = HashMap::new();
        env.insert("KEBAB_RAG_NLI_THRESHOLD".to_string(), "0.5".to_string());
        let c = Config::defaults().apply_env(&env);
        assert!((c.rag.nli_threshold - 0.5).abs() < 1e-6);
    }

    #[test]
    fn env_override_nli_model_and_provider() {
        let mut env = HashMap::new();
        env.insert(
            "KEBAB_MODELS_NLI_MODEL".to_string(),
            "user/custom-nli-model".to_string(),
        );
        env.insert(
            "KEBAB_MODELS_NLI_PROVIDER".to_string(),
            "candle".to_string(),
        );
        let c = Config::defaults().apply_env(&env);
        assert_eq!(c.models.nli.model, "user/custom-nli-model");
        assert_eq!(c.models.nli.provider, "candle");
    }

    /// Malformed `KEBAB_RAG_NLI_THRESHOLD` keeps the prior value (does
    /// NOT silently disable nor crash). The `tracing::warn!` surface
    /// is observable only when the user has tracing wired; the
    /// behavior contract is "default survives".
    #[test]
    fn env_malformed_nli_threshold_keeps_prior_value() {
        let mut env = HashMap::new();
        env.insert(
            "KEBAB_RAG_NLI_THRESHOLD".to_string(),
            "not-a-float".to_string(),
        );
        let c = Config::defaults().apply_env(&env);
        assert_eq!(
            c.rag.nli_threshold, 0.0,
            "malformed env value must keep the default unchanged"
        );
    }

    #[test]
    fn image_ocr_env_overrides() {
        let mut env = HashMap::new();
        env.insert("KEBAB_IMAGE_OCR_ENABLED".to_string(), "true".to_string());
        env.insert(
            "KEBAB_IMAGE_OCR_MODEL".to_string(),
            "gemma4:31b".to_string(),
        );
        env.insert(
            "KEBAB_IMAGE_OCR_ENDPOINT".to_string(),
            "http://192.168.0.47:11434".to_string(),
        );
        // Empty env value should map to None (= fall back to llm.endpoint).
        // We exercise that branch in a separate test.
        env.insert(
            "KEBAB_IMAGE_OCR_LANGUAGES".to_string(),
            "eng, kor, jpn".to_string(),
        );
        env.insert("KEBAB_IMAGE_OCR_MAX_PIXELS".to_string(), "2048".to_string());
        let c = Config::defaults().apply_env(&env);
        assert!(c.ingest.image.ocr.enabled);
        assert_eq!(c.ingest.image.ocr.model, "gemma4:31b");
        assert_eq!(
            c.ingest.image.ocr.endpoint.as_deref(),
            Some("http://192.168.0.47:11434")
        );
        assert_eq!(c.ingest.image.ocr.languages, vec!["eng", "kor", "jpn"]);
        assert_eq!(c.ingest.image.ocr.max_pixels, 2048);
    }

    /// Pre-P6 config files don't have an `[image]` section. The
    /// `#[serde(default)]` attribute on `Config::image` must let those
    /// files load with `ImageCfg::defaults()` instead of erroring.
    #[test]
    fn image_caption_defaults_disabled() {
        let c = Config::defaults();
        assert!(!c.ingest.image.caption.enabled);
        assert_eq!(c.ingest.image.caption.max_pixels, 768);
        assert_eq!(c.ingest.image.caption.prompt_template_version, "caption-v1");
    }

    #[test]
    fn image_caption_env_overrides() {
        let mut env = HashMap::new();
        env.insert(
            "KEBAB_IMAGE_CAPTION_ENABLED".to_string(),
            "true".to_string(),
        );
        env.insert(
            "KEBAB_IMAGE_CAPTION_MAX_PIXELS".to_string(),
            "1024".to_string(),
        );
        env.insert(
            "KEBAB_IMAGE_CAPTION_PROMPT_TEMPLATE_VERSION".to_string(),
            "caption-v2".to_string(),
        );
        let c = Config::defaults().apply_env(&env);
        assert!(c.ingest.image.caption.enabled);
        assert_eq!(c.ingest.image.caption.max_pixels, 1024);
        assert_eq!(c.ingest.image.caption.prompt_template_version, "caption-v2");
    }

    /// `KEBAB_IMAGE_OCR_ENDPOINT=""` (empty value) should map to `None`
    /// rather than to `Some("")` so the fallback to `models.llm.endpoint`
    /// kicks in. Covers the env-equivalent of a missing TOML key.
    #[test]
    fn image_ocr_endpoint_empty_env_value_is_none() {
        let mut env = HashMap::new();
        env.insert("KEBAB_IMAGE_OCR_ENDPOINT".to_string(), String::new());
        let c = Config::defaults().apply_env(&env);
        assert_eq!(c.ingest.image.ocr.endpoint, None);
    }

    #[test]
    fn pre_p6_config_without_image_section_loads_with_defaults() {
        let toml_text = r#"
schema_version = 1

[workspace]
root = "/tmp/x"
include = ["**/*.md"]
exclude = []

[storage]
data_dir = "/tmp/d"
sqlite = "{data_dir}/x.sqlite"
vector_dir = "{data_dir}/v"
asset_dir = "{data_dir}/a"
artifact_dir = "{data_dir}/r"
model_dir = "{data_dir}/m"
runs_dir = "{data_dir}/u"
copy_threshold_mb = 100

[indexing]
max_parallel_extractors = 2
max_parallel_embeddings = 1
watch_filesystem = false

[chunking]
target_tokens = 500
overlap_tokens = 80
respect_markdown_headings = true
chunker_version = "md-heading-v1"

[models.embedding]
provider = "fastembed"
model = "multilingual-e5-large"
version = "v1"
dimensions = 1024
batch_size = 64

[models.llm]
provider = "ollama"
model = "gemma4:e4b"
context_tokens = 32768
endpoint = "http://127.0.0.1:11434"
temperature = 0.0
seed = 0

[search]
default_k = 10
hybrid_fusion = "rrf"
rrf_k = 60
snippet_chars = 220
stale_threshold_days = 30

[rag]
prompt_template_version = "rag-v2"
score_gate = 0.30
explain_default = false
max_context_tokens = 8000
"#;
        let c: Config = toml::from_str(toml_text).expect("pre-P6 TOML must still parse");
        assert_eq!(c.ingest.image, ImageCfg::defaults());
    }

    /// p9-fb-25: legacy config with `workspace.include = [...]` must
    /// still deserialize cleanly (silent unknown-field acceptance).
    #[test]
    fn legacy_include_field_is_ignored_silently() {
        let mut cfg = Config::defaults();
        cfg.workspace.root = Some("/tmp/kebab-legacy".to_string());
        let mut toml_text = toml::to_string(&cfg).expect("default round-trips");
        // Inject a legacy `include = [...]` line into the [workspace] block.
        toml_text = toml_text.replace(
            "[workspace]",
            "[workspace]\ninclude = [\"**/*.md\", \"**/*.txt\"]",
        );
        let parsed: Result<Config, _> = toml::from_str(&toml_text);
        assert!(
            parsed.is_ok(),
            "legacy include must not break load: {:?}",
            parsed.err()
        );
        let cfg = parsed.unwrap();
        assert_eq!(cfg.workspace.root.as_deref(), Some("/tmp/kebab-legacy"));
    }

    /// p9-fb-25: `WorkspaceCfg` must NOT have an `include` field.
    /// Compile-time proof: exhaustive destructure.
    #[test]
    fn workspace_cfg_has_only_root_exclude_sources_fields() {
        let ws = Config::defaults().workspace;
        let WorkspaceCfg {
            root: _,
            exclude: _,
            sources: _,
        } = &ws;
    }

    #[test]
    fn legacy_single_root_normalizes_to_default_source() {
        // A single-root config (no [[workspace.sources]]) must resolve to
        // exactly one source `id = "default"` rooted at workspace.root.
        let mut cfg = Config::defaults();
        cfg.workspace.root = Some("/tmp/kb-notes".to_string());
        let resolved = cfg.resolved_sources();
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].id, DEFAULT_SOURCE_ID);
        assert_eq!(resolved[0].root, std::path::PathBuf::from("/tmp/kb-notes"));
        assert_eq!(resolved[0].trust_level, None);
    }

    #[test]
    fn multi_source_config_resolves_each_with_merged_exclude() {
        let mut cfg = Config::defaults();
        cfg.workspace.root = None;
        cfg.workspace.exclude = vec![".git/**".to_string()];
        cfg.workspace.sources = vec![
            SourceCfg {
                id: "notes".to_string(),
                root: "/tmp/notes".to_string(),
                exclude: vec![],
                trust_level: Some(kebab_core::TrustLevel::Primary),
                source_type: None,
            },
            SourceCfg {
                id: "refs".to_string(),
                root: "/tmp/refs".to_string(),
                exclude: vec!["draft/**".to_string()],
                trust_level: Some(kebab_core::TrustLevel::Secondary),
                source_type: Some(kebab_core::SourceType::Reference),
            },
        ];
        // A multi-source config (no bare root) must round-trip through TOML.
        let toml_text = toml::to_string(&cfg).expect("multi-source serializes");
        let cfg: Config = toml::from_str(&toml_text).expect("multi-source parses");
        cfg.validate_sources().expect("valid sources");
        let resolved = cfg.resolved_sources();
        assert_eq!(resolved.len(), 2);
        assert_eq!(resolved[0].id, "notes");
        assert_eq!(resolved[0].root, std::path::PathBuf::from("/tmp/notes"));
        assert_eq!(resolved[0].exclude, vec![".git/**".to_string()]);
        assert_eq!(resolved[0].trust_level, Some(kebab_core::TrustLevel::Primary));
        assert_eq!(resolved[1].id, "refs");
        // workspace.exclude ∪ per-source exclude.
        assert_eq!(
            resolved[1].exclude,
            vec![".git/**".to_string(), "draft/**".to_string()]
        );
        assert_eq!(
            resolved[1].source_type,
            Some(kebab_core::SourceType::Reference)
        );
        assert_eq!(
            resolved[1].trust_level,
            Some(kebab_core::TrustLevel::Secondary)
        );
    }

    #[test]
    fn defaults_pass_chunking_validation() {
        Config::defaults()
            .validate_chunking()
            .expect("default chunking config must be valid");
    }

    #[test]
    fn rejects_target_tokens_below_floor() {
        let mut cfg = Config::defaults();
        cfg.ingest.chunking.target_tokens = MIN_CHUNK_TOKENS - 1;
        // keep max_chunk ≥ target so target is the failing constraint
        cfg.ingest.chunking.overlap_tokens = 0;
        let err = cfg.validate_chunking().unwrap_err();
        assert!(err.contains("target_tokens"), "got: {err}");
    }

    #[test]
    fn rejects_zero_target_tokens() {
        let mut cfg = Config::defaults();
        cfg.ingest.chunking.target_tokens = 0;
        cfg.ingest.chunking.overlap_tokens = 0;
        assert!(cfg.validate_chunking().is_err());
    }

    #[test]
    fn rejects_overlap_ge_target() {
        let mut cfg = Config::defaults();
        cfg.ingest.chunking.target_tokens = 100;
        cfg.ingest.chunking.overlap_tokens = 100; // == target → invalid
        cfg.ingest.chunking.max_chunk_tokens = 4000;
        let err = cfg.validate_chunking().unwrap_err();
        assert!(err.contains("overlap_tokens"), "got: {err}");
    }

    #[test]
    fn rejects_max_chunk_below_target() {
        let mut cfg = Config::defaults();
        cfg.ingest.chunking.target_tokens = 500;
        cfg.ingest.chunking.overlap_tokens = 80;
        cfg.ingest.chunking.max_chunk_tokens = 400; // < target → invalid
        let err = cfg.validate_chunking().unwrap_err();
        assert!(err.contains("max_chunk_tokens"), "got: {err}");
    }

    #[test]
    fn from_file_rejects_invalid_chunking() {
        // End-to-end: validate_chunking is wired into from_file, so a
        // config whose max_chunk_tokens is below target must fail to load.
        let mut cfg = Config::defaults();
        cfg.workspace.root = Some("/tmp/kb".to_string());
        cfg.ingest.chunking.max_chunk_tokens = 10; // < target 500
        let toml_text = toml::to_string(&cfg).expect("serialize");
        let dir = std::env::temp_dir().join(format!("kebab-cfgtest-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("config.toml");
        std::fs::write(&p, &toml_text).unwrap();
        let res = Config::from_file(&p);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            res.is_err(),
            "from_file must reject max_chunk_tokens < target_tokens"
        );
    }

    fn source_cfg(id: &str, root: &str) -> SourceCfg {
        SourceCfg {
            id: id.to_string(),
            root: root.to_string(),
            exclude: vec![],
            trust_level: None,
            source_type: None,
        }
    }

    #[test]
    fn duplicate_source_ids_rejected() {
        let mut cfg = Config::defaults();
        cfg.workspace.sources = vec![source_cfg("dup", "/a"), source_cfg("dup", "/b")];
        assert!(cfg.validate_sources().is_err(), "duplicate ids must fail");
    }

    #[test]
    fn empty_source_id_rejected() {
        let mut cfg = Config::defaults();
        cfg.workspace.sources = vec![source_cfg("", "/a")];
        assert!(cfg.validate_sources().is_err(), "empty id must fail");
    }

    #[test]
    fn source_id_with_invalid_chars_rejected() {
        // rag-provenance-label: source.id renders into the RAG prompt header;
        // a newline / bracket / space must be rejected at load.
        for bad in ["a b", "a\nb", "a]b", "a/b", "한글"] {
            let mut cfg = Config::defaults();
            cfg.workspace.sources = vec![source_cfg(bad, "/a")];
            assert!(
                cfg.validate_sources().is_err(),
                "source id {bad:?} must be rejected"
            );
        }
        // The allowed set still passes.
        let mut ok = Config::defaults();
        ok.workspace.sources = vec![source_cfg("wiki-v2.notes_1", "/a")];
        ok.validate_sources().expect("clean id must pass");
    }

    #[test]
    fn default_stale_threshold_is_30() {
        let c = Config::defaults();
        assert_eq!(c.search.stale_threshold_days, 30);
    }

    #[test]
    fn env_override_stale_threshold() {
        let c = Config::defaults();
        let env: HashMap<String, String> = [(
            "KEBAB_SEARCH_STALE_THRESHOLD_DAYS".to_string(),
            "7".to_string(),
        )]
        .into_iter()
        .collect();
        let c = c.apply_env(&env);
        assert_eq!(c.search.stale_threshold_days, 7);
    }

    #[test]
    fn env_negative_threshold_silently_ignored() {
        // Env path: malformed numeric values (including negatives that
        // can't fit `u32`) are silently ignored — same pattern as
        // `KEBAB_SEARCH_DEFAULT_K`. The TOML file-load path (covered in
        // `fb27_tests::file_negative_stale_threshold_returns_config_invalid`)
        // is the spec-required hard error surface.
        let c = Config::defaults();
        let env: HashMap<String, String> = [(
            "KEBAB_SEARCH_STALE_THRESHOLD_DAYS".to_string(),
            "-5".to_string(),
        )]
        .into_iter()
        .collect();
        let c = c.apply_env(&env);
        assert_eq!(
            c.search.stale_threshold_days, 30,
            "env path: malformed value must leave the default unchanged"
        );
    }

    #[test]
    fn xdg_paths_honor_env() {
        // Must restore env after the test to avoid polluting other tests.
        let prev = std::env::var("XDG_CONFIG_HOME").ok();
        // SAFETY: tests in this module run sequentially; we restore below.
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", "/tmp/kebabtest-xdg-config");
        }
        let p = Config::xdg_config_path();
        assert_eq!(
            p,
            PathBuf::from("/tmp/kebabtest-xdg-config/kebab/config.toml")
        );
        // SAFETY: scope-local restore.
        unsafe {
            match prev {
                Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
                None => std::env::remove_var("XDG_CONFIG_HOME"),
            }
        }
    }

    #[test]
    fn ingest_code_cfg_defaults() {
        let cfg: IngestCodeCfg = toml::from_str("").unwrap();
        assert_eq!(cfg.max_file_bytes, 262_144);
        assert_eq!(cfg.max_file_lines, 5_000);
        assert!(cfg.skip_generated_header);
        assert!(cfg.extra_skip_globs.is_empty());
        assert_eq!(cfg.ast_chunk_max_lines, 200);
        assert_eq!(cfg.fallback_lines_per_chunk, 80);
        assert_eq!(cfg.fallback_lines_overlap, 20);
    }

    #[test]
    fn ingest_code_cfg_user_override() {
        let toml = r#"
            max_file_bytes = 1048576
            max_file_lines = 20000
            skip_generated_header = false
            extra_skip_globs = ["**/fixtures/**", "**/snapshots/**"]
        "#;
        let cfg: IngestCodeCfg = toml::from_str(toml).unwrap();
        assert_eq!(cfg.max_file_bytes, 1_048_576);
        assert_eq!(cfg.max_file_lines, 20_000);
        assert!(!cfg.skip_generated_header);
        assert_eq!(cfg.extra_skip_globs.len(), 2);
    }

    #[test]
    fn config_with_ingest_code_section() {
        // Build a full valid Config serialization and patch only the
        // [ingest.code] field we care about — avoids having to enumerate
        // every required Config field in the test fixture.
        let base = Config::defaults();
        let mut toml_text = toml::to_string(&base).unwrap();
        // Inject max_file_bytes override into the [ingest.code] table.
        toml_text = toml_text.replace("max_file_bytes = 262144", "max_file_bytes = 524288");
        let cfg: Config = toml::from_str(&toml_text).unwrap();
        assert_eq!(cfg.ingest.code.max_file_bytes, 524_288);
    }

}

#[cfg(test)]
mod fb27_tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn config_invalid_carries_path_and_cause() {
        let nonexistent = PathBuf::from("/this/path/should/not/exist/kebab.toml");
        let err = Config::from_file(&nonexistent).unwrap_err();
        let signal = err
            .downcast_ref::<ConfigInvalid>()
            .expect("from_file error should downcast to ConfigInvalid");
        assert_eq!(signal.path, nonexistent);
        assert!(!signal.cause.is_empty(), "cause should be non-empty");
    }

    #[test]
    fn config_invalid_on_malformed_toml() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("bad.toml");
        std::fs::write(&p, "this is not [valid toml").unwrap();
        let err = Config::from_file(&p).unwrap_err();
        let signal = err
            .downcast_ref::<ConfigInvalid>()
            .expect("malformed TOML should downcast to ConfigInvalid");
        assert_eq!(signal.path, p);
        assert!(!signal.cause.is_empty(), "cause should be non-empty");
    }

    /// Spec §Config: a negative `stale_threshold_days` in TOML must be
    /// rejected at load time (not silently coerced or ignored). serde's
    /// `u32` type-check surfaces the failure as a parse error, which
    /// `from_file` wraps into `ConfigInvalid`. CLI's `error_classify`
    /// downcasts this and emits `error.v1.code = "config_invalid"`.
    #[test]
    fn file_negative_stale_threshold_returns_config_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("neg.toml");
        // Build a minimally valid TOML and override only the field
        // under test — this isolates the failure to the negative
        // value rather than missing required sections.
        let cfg = Config::defaults();
        let mut toml_text = toml::to_string(&cfg).expect("default round-trips");
        assert!(
            toml_text.contains("stale_threshold_days = 30"),
            "default value drifted; update test fixture"
        );
        toml_text = toml_text.replace("stale_threshold_days = 30", "stale_threshold_days = -5");
        std::fs::write(&p, &toml_text).unwrap();
        let err = Config::from_file(&p).unwrap_err();
        let signal = err
            .downcast_ref::<ConfigInvalid>()
            .expect("negative stale_threshold_days should downcast to ConfigInvalid");
        assert_eq!(signal.path, p);
        assert!(
            signal.cause.contains("parse_failed"),
            "expected parse_failed cause, got: {}",
            signal.cause
        );
    }

    #[test]
    fn config_load_explicit_nonexistent_path_returns_config_not_found() {
        // Bug #10: --config /tmp/nonexistent.toml → silent fallback 금지.
        let p = std::path::Path::new("/tmp/__kebab_bugfix3_nonexistent.toml");
        assert!(!p.exists(), "test precondition: path must not exist");

        let err = Config::load(Some(p)).expect_err("expected ConfigNotFound");
        let signal = err
            .downcast_ref::<ConfigNotFound>()
            .expect("from_load error should downcast to ConfigNotFound");
        assert_eq!(signal.path, p.to_path_buf());
    }

    #[test]
    fn pdf_ocr_request_timeout_default_is_180s() {
        // Bug #11 (dogfood 2026-05-27 + follow-up 2026-05-28):
        // default 600s → 60s → 180s (sweet spot 점진적 축소 정책).
        let cfg = PdfOcrCfg::defaults();
        assert_eq!(
            cfg.request_timeout_secs, 180,
            "pdf.ocr.request_timeout_secs default must be 180s (Bug #11, HOTFIXES 2026-05-28)"
        );
    }
}
