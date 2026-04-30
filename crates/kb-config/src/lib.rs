//! `kb-config` — `Config` schema and XDG path resolution (§6).
//!
//! Layer order (`Config::load`): defaults → file → env (`KB_<SECTION>_<KEY>`).
//! CLI overrides land later, applied by `kb-cli` after `Config::load`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

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
                data_dir: "${XDG_DATA_HOME:-~/.local/share}/kb".to_string(),
                sqlite: "{data_dir}/kb.sqlite".to_string(),
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
                    model: "qwen2.5:14b-instruct".to_string(),
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
        }
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

    pub fn from_file(path: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        let cfg: Self = toml::from_str(&text)?;
        Ok(cfg)
    }

    /// Apply `KB_<SECTION>_<KEY>` env overrides. Unknown keys are ignored.
    ///
    /// The mapping is an explicit grep-friendly whitelist — one match arm
    /// per leaf key in `Config`. Booleans accept `1` / `true` / `yes`
    /// (case-insensitive) for true and anything else for false. Numeric
    /// keys silently keep their prior value if the env value fails to
    /// parse, so a malformed `KB_*` cannot crash startup.
    pub fn apply_env(mut self, env: &HashMap<String, String>) -> Self {
        for (k, v) in env {
            if !k.starts_with("KB_") {
                continue;
            }
            match k.as_str() {
                // workspace
                "KB_WORKSPACE_ROOT" => self.workspace.root = v.clone(),

                // storage
                "KB_STORAGE_DATA_DIR" => self.storage.data_dir = v.clone(),
                "KB_STORAGE_SQLITE" => self.storage.sqlite = v.clone(),
                "KB_STORAGE_VECTOR_DIR" => self.storage.vector_dir = v.clone(),
                "KB_STORAGE_ASSET_DIR" => self.storage.asset_dir = v.clone(),
                "KB_STORAGE_ARTIFACT_DIR" => self.storage.artifact_dir = v.clone(),
                "KB_STORAGE_MODEL_DIR" => self.storage.model_dir = v.clone(),
                "KB_STORAGE_RUNS_DIR" => self.storage.runs_dir = v.clone(),
                "KB_STORAGE_COPY_THRESHOLD_MB" => {
                    if let Ok(n) = v.parse::<u64>() {
                        self.storage.copy_threshold_mb = n;
                    }
                }

                // indexing
                "KB_INDEXING_MAX_PARALLEL_EXTRACTORS" => {
                    if let Ok(n) = v.parse::<u32>() {
                        self.indexing.max_parallel_extractors = n;
                    }
                }
                "KB_INDEXING_MAX_PARALLEL_EMBEDDINGS" => {
                    if let Ok(n) = v.parse::<u32>() {
                        self.indexing.max_parallel_embeddings = n;
                    }
                }
                "KB_INDEXING_WATCH_FILESYSTEM" => {
                    self.indexing.watch_filesystem = parse_bool(v);
                }

                // chunking
                "KB_CHUNKING_TARGET_TOKENS" => {
                    if let Ok(n) = v.parse::<usize>() {
                        self.chunking.target_tokens = n;
                    }
                }
                "KB_CHUNKING_OVERLAP_TOKENS" => {
                    if let Ok(n) = v.parse::<usize>() {
                        self.chunking.overlap_tokens = n;
                    }
                }
                "KB_CHUNKING_RESPECT_MARKDOWN_HEADINGS" => {
                    self.chunking.respect_markdown_headings = parse_bool(v);
                }
                "KB_CHUNKING_CHUNKER_VERSION" => self.chunking.chunker_version = v.clone(),

                // models.embedding
                "KB_MODELS_EMBEDDING_PROVIDER" => self.models.embedding.provider = v.clone(),
                "KB_MODELS_EMBEDDING_MODEL" => self.models.embedding.model = v.clone(),
                "KB_MODELS_EMBEDDING_VERSION" => self.models.embedding.version = v.clone(),
                "KB_MODELS_EMBEDDING_DIMENSIONS" => {
                    if let Ok(n) = v.parse::<usize>() {
                        self.models.embedding.dimensions = n;
                    }
                }
                "KB_MODELS_EMBEDDING_BATCH_SIZE" => {
                    if let Ok(n) = v.parse::<usize>() {
                        self.models.embedding.batch_size = n;
                    }
                }

                // models.llm
                "KB_MODELS_LLM_PROVIDER" => self.models.llm.provider = v.clone(),
                "KB_MODELS_LLM_MODEL" => self.models.llm.model = v.clone(),
                "KB_MODELS_LLM_CONTEXT_TOKENS" => {
                    if let Ok(n) = v.parse::<usize>() {
                        self.models.llm.context_tokens = n;
                    }
                }
                "KB_MODELS_LLM_ENDPOINT" => self.models.llm.endpoint = v.clone(),
                "KB_MODELS_LLM_TEMPERATURE" => {
                    if let Ok(f) = v.parse::<f32>() {
                        self.models.llm.temperature = f;
                    }
                }
                "KB_MODELS_LLM_SEED" => {
                    if let Ok(n) = v.parse::<u64>() {
                        self.models.llm.seed = n;
                    }
                }

                // search
                "KB_SEARCH_DEFAULT_K" => {
                    if let Ok(n) = v.parse::<usize>() {
                        self.search.default_k = n;
                    }
                }
                "KB_SEARCH_HYBRID_FUSION" => self.search.hybrid_fusion = v.clone(),
                "KB_SEARCH_RRF_K" => {
                    if let Ok(n) = v.parse::<u32>() {
                        self.search.rrf_k = n;
                    }
                }
                "KB_SEARCH_SNIPPET_CHARS" => {
                    if let Ok(n) = v.parse::<usize>() {
                        self.search.snippet_chars = n;
                    }
                }

                // rag
                "KB_RAG_PROMPT_TEMPLATE_VERSION" => {
                    self.rag.prompt_template_version = v.clone();
                }
                "KB_RAG_SCORE_GATE" => {
                    if let Ok(f) = v.parse::<f32>() {
                        self.rag.score_gate = f;
                    }
                }
                "KB_RAG_EXPLAIN_DEFAULT" => {
                    self.rag.explain_default = parse_bool(v);
                }
                "KB_RAG_MAX_CONTEXT_TOKENS" => {
                    if let Ok(n) = v.parse::<usize>() {
                        self.rag.max_context_tokens = n;
                    }
                }

                // Unknown KB_* keys are silently ignored — see
                // `env_unknown_key_is_ignored` test.
                _ => {}
            }
        }
        self
    }

    /// `~/.config/kb/config.toml` (honors `XDG_CONFIG_HOME`).
    pub fn xdg_config_path() -> PathBuf {
        if let Ok(custom) = std::env::var("XDG_CONFIG_HOME") {
            if !custom.is_empty() {
                return PathBuf::from(custom).join("kb").join("config.toml");
            }
        }
        match dirs::config_dir() {
            Some(d) => d.join("kb").join("config.toml"),
            None => PathBuf::from("./kb/config.toml"),
        }
    }

    /// `~/.local/share/kb` (honors `XDG_DATA_HOME`).
    pub fn xdg_data_dir() -> PathBuf {
        if let Ok(custom) = std::env::var("XDG_DATA_HOME") {
            if !custom.is_empty() {
                return PathBuf::from(custom).join("kb");
            }
        }
        match dirs::data_dir() {
            Some(d) => d.join("kb"),
            None => PathBuf::from("./kb-data"),
        }
    }

    /// `~/.cache/kb` (honors `XDG_CACHE_HOME`).
    pub fn xdg_cache_dir() -> PathBuf {
        if let Ok(custom) = std::env::var("XDG_CACHE_HOME") {
            if !custom.is_empty() {
                return PathBuf::from(custom).join("kb");
            }
        }
        match dirs::cache_dir() {
            Some(d) => d.join("kb"),
            None => PathBuf::from("./kb-cache"),
        }
    }

    /// `~/.local/state/kb` (honors `XDG_STATE_HOME`).
    pub fn xdg_state_dir() -> PathBuf {
        if let Ok(custom) = std::env::var("XDG_STATE_HOME") {
            if !custom.is_empty() {
                return PathBuf::from(custom).join("kb");
            }
        }
        // `dirs` doesn't expose state_dir on all platforms; fall back to
        // `$HOME/.local/state/kb` if XDG_STATE_HOME is unset.
        if let Some(home) = dirs::home_dir() {
            return home.join(".local").join("state").join("kb");
        }
        PathBuf::from("./kb-state")
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
        env.insert("KB_RAG_SCORE_GATE".to_string(), "0.5".to_string());
        let c = Config::defaults().apply_env(&env);
        assert!((c.rag.score_gate - 0.5).abs() < 1e-6);
    }

    #[test]
    fn env_override_search_k() {
        let mut env = HashMap::new();
        env.insert("KB_SEARCH_DEFAULT_K".to_string(), "25".to_string());
        let c = Config::defaults().apply_env(&env);
        assert_eq!(c.search.default_k, 25);
    }

    #[test]
    fn env_unknown_key_is_ignored() {
        let baseline = Config::defaults();
        let mut env = HashMap::new();
        env.insert("KB_NOPE_FOO".to_string(), "garbage".to_string());
        let c = Config::defaults().apply_env(&env);
        assert_eq!(c, baseline);
    }

    #[test]
    fn env_overrides_chunking_target_tokens() {
        let mut env = HashMap::new();
        env.insert("KB_CHUNKING_TARGET_TOKENS".to_string(), "777".to_string());
        let c = Config::defaults().apply_env(&env);
        assert_eq!(c.chunking.target_tokens, 777);
    }

    #[test]
    fn env_overrides_models_llm_endpoint_and_temperature() {
        let mut env = HashMap::new();
        env.insert(
            "KB_MODELS_LLM_ENDPOINT".to_string(),
            "http://10.0.0.1:11434".to_string(),
        );
        env.insert("KB_MODELS_LLM_TEMPERATURE".to_string(), "0.7".to_string());
        let c = Config::defaults().apply_env(&env);
        assert_eq!(c.models.llm.endpoint, "http://10.0.0.1:11434");
        assert!((c.models.llm.temperature - 0.7).abs() < 1e-6);
    }

    #[test]
    fn env_overrides_indexing_watch_filesystem_bool() {
        let mut env = HashMap::new();
        env.insert(
            "KB_INDEXING_WATCH_FILESYSTEM".to_string(),
            "true".to_string(),
        );
        let c = Config::defaults().apply_env(&env);
        assert!(c.indexing.watch_filesystem);
    }

    #[test]
    fn xdg_paths_honor_env() {
        // Must restore env after the test to avoid polluting other tests.
        let prev = std::env::var("XDG_CONFIG_HOME").ok();
        // SAFETY: tests in this module run sequentially; we restore below.
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", "/tmp/kbtest-xdg-config");
        }
        let p = Config::xdg_config_path();
        assert_eq!(p, PathBuf::from("/tmp/kbtest-xdg-config/kb/config.toml"));
        // SAFETY: scope-local restore.
        unsafe {
            match prev {
                Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
                None => std::env::remove_var("XDG_CONFIG_HOME"),
            }
        }
    }
}
