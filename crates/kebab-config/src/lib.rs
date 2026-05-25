//! `kb-config` — `Config` schema and XDG path resolution (§6).
//!
//! Layer order (`Config::load`): defaults → file → env (`KEBAB_<SECTION>_<KEY>`).
//! CLI overrides land later, applied by `kb-cli` after `Config::load`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

mod paths;
pub use paths::{expand_path, expand_path_with_base};

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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Config {
    pub schema_version: u32,
    pub workspace: WorkspaceCfg,
    pub storage: StorageCfg,
    pub indexing: IndexingCfg,
    pub chunking: ChunkingCfg,
    pub models: ModelsCfg,
    pub search: SearchCfg,
    pub rag: RagCfg,
    /// Image-pipeline settings (P6: OCR, captioning). Tagged
    /// `#[serde(default)]` so pre-P6 config files that predate the
    /// `[image]` section still load — defaults disable OCR / caption
    /// (they cost a model call per asset).
    #[serde(default = "ImageCfg::defaults")]
    pub image: ImageCfg,
    /// p9-fb-14: TUI palette + role-style mapping. `#[serde(default)]`
    /// so configs that predate this section still load (defaults to
    /// `dark`).
    #[serde(default = "UiCfg::defaults")]
    pub ui: UiCfg,
    /// p10-1A-1: code ingest settings. `#[serde(default)]` so existing
    /// config files without an `[ingest]` / `[ingest.code]` section
    /// load cleanly with built-in defaults.
    #[serde(default)]
    pub ingest: IngestCfg,
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
    pub root: String,
    pub exclude: Vec<String>,
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
pub struct IndexingCfg {
    pub max_parallel_extractors: u32,
    pub max_parallel_embeddings: u32,
    pub watch_filesystem: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChunkingCfg {
    pub target_tokens: usize,
    pub overlap_tokens: usize,
    pub respect_markdown_headings: bool,
    pub chunker_version: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelsCfg {
    pub embedding: EmbeddingModelCfg,
    pub llm: LlmCfg,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EmbeddingModelCfg {
    pub provider: String,
    pub model: String,
    pub version: String,
    pub dimensions: usize,
    pub batch_size: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LlmCfg {
    pub provider: String,
    pub model: String,
    pub context_tokens: usize,
    pub endpoint: String,
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
    pub score_gate: f32,
    pub explain_default: bool,
    pub max_context_tokens: usize,
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
    /// `LlmCfg::request_timeout_secs`: `Duration::from_secs(0)` means
    /// "every request fails immediately", not "no timeout". Use a
    /// large finite value for an effectively-uncapped budget.
    #[serde(default = "default_ocr_request_timeout_secs")]
    pub request_timeout_secs: u64,
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
        }
    }
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

/// p10-1A-1: top-level ingest configuration wrapper. Contains per-media-type
/// sub-sections; currently only `code` is defined.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct IngestCfg {
    pub code: IngestCodeCfg,
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
            schema_version: 1,
            workspace: WorkspaceCfg {
                root: "~/KnowledgeBase".to_string(),
                exclude: vec![
                    ".git/**".to_string(),
                    "node_modules/**".to_string(),
                    ".obsidian/**".to_string(),
                ],
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
            indexing: IndexingCfg {
                max_parallel_extractors: 2,
                max_parallel_embeddings: 1,
                watch_filesystem: false,
            },
            chunking: ChunkingCfg {
                target_tokens: 500,
                overlap_tokens: 80,
                respect_markdown_headings: true,
                chunker_version: "md-heading-v1".to_string(),
            },
            models: ModelsCfg {
                embedding: EmbeddingModelCfg {
                    provider: "fastembed".to_string(),
                    model: "multilingual-e5-large".to_string(),
                    version: "v1".to_string(),
                    dimensions: 1024,
                    batch_size: 64,
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
                prompt_template_version: "rag-v2".to_string(),
                score_gate: 0.30,
                explain_default: false,
                max_context_tokens: 8000,
            },
            image: ImageCfg::defaults(),
            ui: UiCfg::defaults(),
            ingest: IngestCfg::default(),
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
        paths::expand_path_with_base(&self.workspace.root, "", &base)
    }

    /// Read config from disk and merge env overrides on top of it. If the
    /// file is missing, defaults are used (so `kb doctor` runs with no
    /// prior `kb init`).
    pub fn load(path: Option<&Path>) -> anyhow::Result<Self> {
        let from_disk = match path {
            Some(p) if p.exists() => Self::from_file(p)?,
            Some(_) => Self::defaults(),
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

        // p9-fb-25: probe for the legacy `workspace.include` key — if
        // present, emit a one-shot deprecation warning. Detection uses
        // raw `toml::Value` lookup; the warning fires via a process-
        // level OnceLock so a long-running TUI / CLI run doesn't spam
        // the log on every Config::load.
        if let Ok(value) = toml::from_str::<toml::Value>(&text) {
            if value
                .get("workspace")
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
        }

        let mut cfg: Self = toml::from_str(&text).map_err(|e| {
            anyhow::Error::new(ConfigInvalid {
                path: path.to_path_buf(),
                cause: format!("parse_failed: {e}"),
            })
        })?;
        cfg.source_dir = path.parent().map(Path::to_path_buf);
        Ok(cfg)
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
                "KEBAB_WORKSPACE_ROOT" => self.workspace.root = v.clone(),

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
                        self.indexing.max_parallel_extractors = n;
                    }
                }
                "KEBAB_INDEXING_MAX_PARALLEL_EMBEDDINGS" => {
                    if let Ok(n) = v.parse::<u32>() {
                        self.indexing.max_parallel_embeddings = n;
                    }
                }
                "KEBAB_INDEXING_WATCH_FILESYSTEM" => {
                    self.indexing.watch_filesystem = parse_bool(v);
                }

                // chunking
                "KEBAB_CHUNKING_TARGET_TOKENS" => {
                    if let Ok(n) = v.parse::<usize>() {
                        self.chunking.target_tokens = n;
                    }
                }
                "KEBAB_CHUNKING_OVERLAP_TOKENS" => {
                    if let Ok(n) = v.parse::<usize>() {
                        self.chunking.overlap_tokens = n;
                    }
                }
                "KEBAB_CHUNKING_RESPECT_MARKDOWN_HEADINGS" => {
                    self.chunking.respect_markdown_headings = parse_bool(v);
                }
                "KEBAB_CHUNKING_CHUNKER_VERSION" => self.chunking.chunker_version = v.clone(),

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

                // image.ocr
                "KEBAB_IMAGE_OCR_ENABLED" => {
                    self.image.ocr.enabled = parse_bool(v);
                }
                "KEBAB_IMAGE_OCR_ENGINE" => self.image.ocr.engine = v.clone(),
                "KEBAB_IMAGE_OCR_MODEL" => self.image.ocr.model = v.clone(),
                "KEBAB_IMAGE_OCR_ENDPOINT" => {
                    // Empty env value is treated the same as "fall back
                    // to models.llm.endpoint" — i.e. set None.
                    self.image.ocr.endpoint = if v.is_empty() {
                        None
                    } else {
                        Some(v.clone())
                    };
                }
                "KEBAB_IMAGE_OCR_LANGUAGES" => {
                    // Comma-separated list, e.g. "eng,kor".
                    self.image.ocr.languages = v
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                }
                "KEBAB_IMAGE_OCR_MAX_PIXELS" => {
                    if let Ok(n) = v.parse::<u32>() {
                        self.image.ocr.max_pixels = n;
                    }
                }
                "KEBAB_IMAGE_OCR_REQUEST_TIMEOUT_SECS" => {
                    if let Ok(n) = v.parse::<u64>() {
                        self.image.ocr.request_timeout_secs = n;
                    }
                }

                // image.caption (P6-3)
                "KEBAB_IMAGE_CAPTION_ENABLED" => {
                    self.image.caption.enabled = parse_bool(v);
                }
                "KEBAB_IMAGE_CAPTION_MAX_PIXELS" => {
                    if let Ok(n) = v.parse::<u32>() {
                        self.image.caption.max_pixels = n;
                    }
                }
                "KEBAB_IMAGE_CAPTION_PROMPT_TEMPLATE_VERSION" => {
                    self.image.caption.prompt_template_version = v.clone();
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

    #[test]
    fn defaults_are_serde_roundtrip_stable() {
        let c = Config::defaults();
        let toml_text = toml::to_string(&c).unwrap();
        let back: Config = toml::from_str(&toml_text).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn defaults_match_design_64_score_gate() {
        let c = Config::defaults();
        assert_eq!(c.rag.score_gate, 0.30);
        assert_eq!(c.chunking.target_tokens, 500);
        assert_eq!(c.models.embedding.model, "multilingual-e5-large");
        assert_eq!(c.models.embedding.dimensions, 1024);
        assert_eq!(c.search.rrf_k, 60);
    }

    #[test]
    fn defaults_rag_prompt_template_version_is_rag_v2() {
        let c = Config::defaults();
        assert_eq!(c.rag.prompt_template_version, "rag-v2");
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
        env.insert("KEBAB_CHUNKING_TARGET_TOKENS".to_string(), "777".to_string());
        let c = Config::defaults().apply_env(&env);
        assert_eq!(c.chunking.target_tokens, 777);
    }

    #[test]
    fn env_overrides_models_llm_endpoint_and_temperature() {
        let mut env = HashMap::new();
        env.insert(
            "KEBAB_MODELS_LLM_ENDPOINT".to_string(),
            "http://10.0.0.1:11434".to_string(),
        );
        env.insert("KEBAB_MODELS_LLM_TEMPERATURE".to_string(), "0.7".to_string());
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
    /// back to the 300s default — backwards-compat invariant.
    #[test]
    fn legacy_config_without_request_timeout_secs_uses_default() {
        let toml_src = r#"
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
prompt_template_version = "rag-v2"
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
        let c: Config = toml::from_str(toml_src).expect("parse legacy config");
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
        assert!(c.indexing.watch_filesystem);
    }

    #[test]
    fn image_ocr_defaults_disabled_with_ollama_vision() {
        let c = Config::defaults();
        assert!(!c.image.ocr.enabled);
        assert_eq!(c.image.ocr.engine, "ollama-vision");
        assert_eq!(c.image.ocr.model, "gemma4:e4b");
        assert_eq!(c.image.ocr.languages, vec!["eng", "kor"]);
        assert_eq!(c.image.ocr.max_pixels, 1600);
    }

    /// v0.17.2 post-dogfood: matches the legacy hard-coded 300s cap so
    /// existing configs that omit the new field keep behaving identically.
    #[test]
    fn default_ocr_request_timeout_secs_is_300() {
        assert_eq!(
            Config::defaults().image.ocr.request_timeout_secs,
            300
        );
    }

    #[test]
    fn env_overrides_image_ocr_request_timeout_secs() {
        let mut env = HashMap::new();
        env.insert(
            "KEBAB_IMAGE_OCR_REQUEST_TIMEOUT_SECS".to_string(),
            "900".to_string(),
        );
        let c = Config::defaults().apply_env(&env);
        assert_eq!(c.image.ocr.request_timeout_secs, 900);
    }

    /// v0.17.2 post-dogfood: a config file written before the OCR
    /// timeout field existed must still parse and fall back to the
    /// 300s default — backwards-compat invariant. Reuses the same
    /// minimal legacy TOML fixture as the LLM-side test.
    #[test]
    fn legacy_config_without_ocr_request_timeout_secs_uses_default() {
        let toml_src = r#"
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
prompt_template_version = "rag-v2"
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
        let c: Config = toml::from_str(toml_src).expect("parse legacy config");
        assert_eq!(c.image.ocr.request_timeout_secs, 300);
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
        assert!(c.image.ocr.enabled);
        assert_eq!(c.image.ocr.model, "gemma4:31b");
        assert_eq!(
            c.image.ocr.endpoint.as_deref(),
            Some("http://192.168.0.47:11434")
        );
        assert_eq!(c.image.ocr.languages, vec!["eng", "kor", "jpn"]);
        assert_eq!(c.image.ocr.max_pixels, 2048);
    }

    /// Pre-P6 config files don't have an `[image]` section. The
    /// `#[serde(default)]` attribute on `Config::image` must let those
    /// files load with `ImageCfg::defaults()` instead of erroring.
    #[test]
    fn image_caption_defaults_disabled() {
        let c = Config::defaults();
        assert!(!c.image.caption.enabled);
        assert_eq!(c.image.caption.max_pixels, 768);
        assert_eq!(c.image.caption.prompt_template_version, "caption-v1");
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
        assert!(c.image.caption.enabled);
        assert_eq!(c.image.caption.max_pixels, 1024);
        assert_eq!(c.image.caption.prompt_template_version, "caption-v2");
    }

    /// `KEBAB_IMAGE_OCR_ENDPOINT=""` (empty value) should map to `None`
    /// rather than to `Some("")` so the fallback to `models.llm.endpoint`
    /// kicks in. Covers the env-equivalent of a missing TOML key.
    #[test]
    fn image_ocr_endpoint_empty_env_value_is_none() {
        let mut env = HashMap::new();
        env.insert("KEBAB_IMAGE_OCR_ENDPOINT".to_string(), String::new());
        let c = Config::defaults().apply_env(&env);
        assert_eq!(c.image.ocr.endpoint, None);
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
        assert_eq!(c.image, ImageCfg::defaults());
    }

    /// p9-fb-25: legacy config with `workspace.include = [...]` must
    /// still deserialize cleanly (silent unknown-field acceptance).
    #[test]
    fn legacy_include_field_is_ignored_silently() {
        let mut cfg = Config::defaults();
        cfg.workspace.root = "/tmp/kebab-legacy".to_string();
        let mut toml_text = toml::to_string(&cfg).expect("default round-trips");
        // Inject a legacy `include = [...]` line into the [workspace] block.
        toml_text = toml_text.replace(
            "[workspace]",
            "[workspace]\ninclude = [\"**/*.md\", \"**/*.txt\"]",
        );
        let parsed: Result<Config, _> = toml::from_str(&toml_text);
        assert!(parsed.is_ok(), "legacy include must not break load: {:?}", parsed.err());
        let cfg = parsed.unwrap();
        assert_eq!(cfg.workspace.root, "/tmp/kebab-legacy");
    }

    /// p9-fb-25: `WorkspaceCfg` must NOT have an `include` field.
    /// Compile-time proof: exhaustive destructure.
    #[test]
    fn workspace_cfg_has_only_root_and_exclude_fields() {
        let ws = Config::defaults().workspace;
        let WorkspaceCfg { root: _, exclude: _ } = &ws;
    }

    #[test]
    fn default_stale_threshold_is_30() {
        let c = Config::defaults();
        assert_eq!(c.search.stale_threshold_days, 30);
    }

    #[test]
    fn env_override_stale_threshold() {
        let c = Config::defaults();
        let env: HashMap<String, String> = [
            ("KEBAB_SEARCH_STALE_THRESHOLD_DAYS".to_string(), "7".to_string()),
        ]
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
        let env: HashMap<String, String> = [
            ("KEBAB_SEARCH_STALE_THRESHOLD_DAYS".to_string(), "-5".to_string()),
        ]
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
        assert_eq!(p, PathBuf::from("/tmp/kebabtest-xdg-config/kebab/config.toml"));
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
        toml_text = toml_text.replace(
            "max_file_bytes = 262144",
            "max_file_bytes = 524288",
        );
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
        let signal = err.downcast_ref::<ConfigInvalid>()
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
        let signal = err.downcast_ref::<ConfigInvalid>()
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
        toml_text = toml_text.replace(
            "stale_threshold_days = 30",
            "stale_threshold_days = -5",
        );
        std::fs::write(&p, &toml_text).unwrap();
        let err = Config::from_file(&p).unwrap_err();
        let signal = err.downcast_ref::<ConfigInvalid>()
            .expect("negative stale_threshold_days should downcast to ConfigInvalid");
        assert_eq!(signal.path, p);
        assert!(
            signal.cause.contains("parse_failed"),
            "expected parse_failed cause, got: {}",
            signal.cause
        );
    }
}
