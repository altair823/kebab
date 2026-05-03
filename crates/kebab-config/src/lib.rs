//! `kb-config` — `Config` schema and XDG path resolution (§6).
//!
//! Layer order (`Config::load`): defaults → file → env (`KEBAB_<SECTION>_<KEY>`).
//! CLI overrides land later, applied by `kb-cli` after `Config::load`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

mod paths;
pub use paths::{expand_path, expand_path_with_base};

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
    pub include: Vec<String>,
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
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SearchCfg {
    pub default_k: usize,
    pub hybrid_fusion: String,
    pub rrf_k: u32,
    pub snippet_chars: usize,
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
        }
    }
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

impl Config {
    /// Defaults per design §6.4.
    pub fn defaults() -> Self {
        Self {
            schema_version: 1,
            workspace: WorkspaceCfg {
                root: "~/KnowledgeBase".to_string(),
                include: vec!["**/*.md".to_string()],
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
                    model: "multilingual-e5-small".to_string(),
                    version: "v1".to_string(),
                    dimensions: 384,
                    batch_size: 64,
                },
                llm: LlmCfg {
                    provider: "ollama".to_string(),
                    // gemma4 계열 통일 — OCR (P6-2) + caption (P6-3)
                    // 어댑터가 같은 family 사용. 사용자가 더 큰
                    // variant (gemma4:26b 등) 원하면 자기 config.toml
                    // 에서 override.
                    model: "gemma4:e4b".to_string(),
                    context_tokens: 32768,
                    endpoint: "http://127.0.0.1:11434".to_string(),
                    temperature: 0.0,
                    seed: 0,
                },
            },
            search: SearchCfg {
                default_k: 10,
                hybrid_fusion: "rrf".to_string(),
                rrf_k: 60,
                snippet_chars: 220,
            },
            rag: RagCfg {
                prompt_template_version: "rag-v1".to_string(),
                score_gate: 0.30,
                explain_default: false,
                max_context_tokens: 8000,
            },
            image: ImageCfg::defaults(),
            ui: UiCfg::defaults(),
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
        let text = std::fs::read_to_string(path)?;
        let mut cfg: Self = toml::from_str(&text)?;
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
        match dirs::config_dir() {
            Some(d) => d.join("kebab").join("config.toml"),
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
        match dirs::data_dir() {
            Some(d) => d.join("kebab"),
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
        match dirs::cache_dir() {
            Some(d) => d.join("kebab"),
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
        assert_eq!(c.models.embedding.dimensions, 384);
        assert_eq!(c.search.rrf_k, 60);
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
model = "multilingual-e5-small"
version = "v1"
dimensions = 384
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

[rag]
prompt_template_version = "rag-v1"
score_gate = 0.30
explain_default = false
max_context_tokens = 8000
"#;
        let c: Config = toml::from_str(toml_text).expect("pre-P6 TOML must still parse");
        assert_eq!(c.image, ImageCfg::defaults());
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
}
