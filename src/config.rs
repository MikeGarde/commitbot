use crate::{git, Cli};
use serde::Deserialize;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::PathBuf;
use log::info;
use git::detect_repo_id;

/// Final resolved configuration for commitbot.
#[derive(Debug, Clone)]
pub struct Config {
    pub openai_api_key: String,
    pub model: String,
    pub max_concurrent_requests: usize,
}

impl Config {
    /// Build the final config from CLI flags, environment, TOML file, and defaults.
    ///
    /// Precedence (highest to lowest):
    ///   1. CLI flags (`--model`, `--api-key`, `--max`)
    ///   2. Env vars (`COMMITBOT_MODEL`, `OPENAI_API_KEY`, `COMMITBOT_MAX_CONCURRENT_REQUESTS`)
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
        let model_cli = cli.model.clone();
        let api_key_cli = cli.api_key.clone();
        let max_cli = cli.max;

        // Env values
        let model_env = env::var("COMMITBOT_MODEL").ok();
        let api_key_env = env::var("OPENAI_API_KEY").ok();
        let max_env = env::var("COMMITBOT_MAX_CONCURRENT_REQUESTS")
            .ok()
            .and_then(|s| s.parse::<usize>().ok());

        // Resolve model
        let model = model_cli
            .or(model_env)
            .or(repo_file_cfg.model)
            .or(default_file_cfg.model)
            .unwrap_or_else(|| "gpt-5-nano".to_string());

        // Resolve API key (must exist somewhere)
        let openai_api_key = api_key_cli
            .or(api_key_env)
            .or(repo_file_cfg.openai_api_key)
            .or(default_file_cfg.openai_api_key)
            .expect("OPENAI_API_KEY must be set via CLI, env var, or config file");

        // Resolve max concurrency; default to 4 if not specified anywhere
        let max_concurrent_requests = max_cli
            .or(max_env)
            .or(repo_file_cfg.max_concurrent_requests)
            .or(default_file_cfg.max_concurrent_requests)
            .unwrap_or(4);

        Config {
            model,
            openai_api_key,
            max_concurrent_requests,
        }
    }
}

#[derive(Debug, Default, Deserialize, Clone)]
struct FileConfig {
    /// Default model to use when not provided via CLI or env.
    pub model: Option<String>,
    pub openai_api_key: Option<String>,
    pub max_concurrent_requests: Option<usize>,
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
        info!("No config file found");
        return None;
    }

    let data = fs::read_to_string(&path).ok()?;
    toml::from_str::<FileConfigRoot>(&data).ok()
}

