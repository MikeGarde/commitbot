use crate::{git, Cli};
use serde::Deserialize;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::PathBuf;
use git::detect_repo_id;

/// Final resolved configuration for commitbot.
#[derive(Debug, Clone)]
pub struct Config {
    pub provider: String,
    pub openai_api_key: Option<String>,
    pub base_url: Option<String>,
    pub model: String,
    pub max_concurrent_requests: usize,
    pub stream: bool,
}

impl Config {
    /// Build the final config from CLI flags, environment, TOML file, and defaults.
    ///
    /// Precedence (highest to lowest):
    ///   1. CLI flags (`--provider`, `--model`, `--api-key`, `--base-url`, `--max`, `--stream`)
    ///   2. Env vars (`COMMITBOT_PROVIDER`, `COMMITBOT_MODEL`, `OPENAI_API_KEY`, `COMMITBOT_BASE_URL`, `COMMITBOT_MAX_CONCURRENT_REQUESTS`, `COMMITBOT_STREAM`)
    ///   3. Per-repo table in `~/.config/commitbot.toml` (e.g. ["mikegarde/commitbot"])
    ///   4. [default] table in `~/.config/commitbot.toml`
    ///   5. Hardcoded defaults (model = "gpt-5-nano", max_concurrent_requests = 4)
    pub fn from_sources(cli: &Cli) -> Self {
        let file_cfg_root = load_file_config().unwrap_or_default();
        let repo_id = detect_repo_id();

        // Split file config into [default] and repo-specific override (if any).
        let default_file_cfg = file_cfg_root.default.unwrap_or_default();
        let repo_file_cfg = repo_id
            .as_deref()
            .and_then(|id| file_cfg_root.repos.get(id))
            .cloned()
            .unwrap_or_default();

        // CLI values
        let provider_cli = cli.provider.clone();
        let model_cli = cli.model.clone();
        let api_key_cli = cli.api_key.clone();
        let base_url_cli = cli.url.clone();
        let max_cli = cli.max;
        let stream_cli = cli.stream;

        // Env values
        let provider_env = env::var("COMMITBOT_PROVIDER").ok();
        let model_env = env::var("COMMITBOT_MODEL").ok();
        let api_key_env = env::var("OPENAI_API_KEY").ok();
        let base_url_env = env::var("COMMITBOT_BASE_URL").ok();
        let max_env = env::var("COMMITBOT_MAX_CONCURRENT_REQUESTS")
            .ok()
            .and_then(|s| s.parse::<usize>().ok());
        let stream_env = env::var("COMMITBOT_STREAM")
            .ok()
            .and_then(|s| s.parse::<bool>().ok());

        // Resolve provider
        let provider = provider_cli
            .or(provider_env)
            .or(repo_file_cfg.provider)
            .or(default_file_cfg.provider)
            .unwrap_or_else(|| "openai".to_string())
            .to_lowercase();

        // Resolve model
        let model = model_cli
            .or(model_env)
            .or(repo_file_cfg.model)
            .or(default_file_cfg.model)
            .unwrap_or_else(|| "gpt-5-nano".to_string());

        // Resolve API key (only required for OpenAI-style providers)
        let openai_api_key = api_key_cli
            .or(api_key_env)
            .or(repo_file_cfg.openai_api_key)
            .or(default_file_cfg.openai_api_key);

        // Resolve base URL (used for ollama or openai-compatible endpoints)
        let base_url = base_url_cli
            .or(base_url_env)
            .or(repo_file_cfg.base_url)
            .or(default_file_cfg.base_url);

        // Resolve max concurrency; default to 4 if not specified anywhere
        let max_concurrent_requests = max_cli
            .or(max_env)
            .or(repo_file_cfg.max_concurrent_requests)
            .or(default_file_cfg.max_concurrent_requests)
            .unwrap_or(4);

        let stream = stream_cli
            .or(stream_env)
            .or(repo_file_cfg.stream)
            .or(default_file_cfg.stream)
            .unwrap_or(true);

        let provider = provider.trim_matches('"').to_string();
        let model = model.trim_matches('"').to_string();
        let openai_api_key = openai_api_key.map(|s| s.trim_matches('"').to_string());
        let base_url = base_url.map(|s| s.trim_matches('"').to_string());

        let url = base_url.clone().unwrap_or_default();

        log::debug!("Provider: {}", provider);
        log::debug!("Model:    {}", model);
        log::debug!("API key:  {}", openai_api_key.is_some());
        log::debug!("URL:      {}", url);
        log::debug!("Max Req:  {}", max_concurrent_requests);
        log::debug!("Stream:   {}", stream);

        if provider == "openai" && openai_api_key.is_none() {
            panic!("OPENAI_API_KEY must be set via CLI, env var, or config file for provider=openai");
        }

        Config {
            provider,
            model,
            openai_api_key,
            base_url,
            max_concurrent_requests,
            stream,
        }
    }
}

#[derive(Debug, Default, Deserialize, Clone)]
struct FileConfig {
    /// LLM provider / API style (openai or ollama).
    pub provider: Option<String>,
    /// Default model to use when not provided via CLI or env.
    pub model: Option<String>,
    pub openai_api_key: Option<String>,
    pub base_url: Option<String>,
    pub max_concurrent_requests: Option<usize>,
    pub stream: Option<bool>,
}

/// Root of the TOML file:
/// - [default]
/// - ["owner/repo"] tables flattened into `repos`
#[derive(Debug, Default, Deserialize)]
struct FileConfigRoot {
    pub default: Option<FileConfig>,

    #[serde(flatten)]
    pub repos: HashMap<String, FileConfig>,
}

/// Return `~/.config/commitbot.toml`
fn config_path() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    Some(home.join(".config").join("commitbot.toml"))
}

fn load_file_config() -> Option<FileConfigRoot> {
    let path = config_path()?;
    if !path.exists() {
        return None;
    }

    let data = fs::read_to_string(&path).ok()?;
    toml::from_str::<FileConfigRoot>(&data).ok()
}
