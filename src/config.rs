use crate::Cli;
use serde::Deserialize;
use std::env;
use std::fs;
use std::path::PathBuf;

/// Final resolved configuration for commitbot.
#[derive(Debug, Clone)]
pub struct Config {
    pub openai_api_key: String,
    pub model: String,
}

impl Config {
    /// Build the final config from CLI flags, environment, TOML file, and defaults.
    ///
    /// Precedence:
    ///   1. CLI flags (`--model`)
    ///   2. Env var `COMMITBOT_MODEL`
    ///   3. TOML `~/.config/commitbot.toml`
    ///   4. Hardcoded default ("gpt-5-nano")
    pub fn from_sources(cli: &Cli) -> Self {
        let file_cfg = load_file_config().unwrap_or_default();

        let model_cli = cli.model.clone();
        let api_key_cli = cli.api_key.clone();
        let model_env = env::var("COMMITBOT_MODEL").ok();
        let api_key_env = env::var("OPENAI_API_KEY").ok();

        let model = model_cli
            .or(model_env)
            .or(file_cfg.model)
            .unwrap_or_else(|| "gpt-5-nano".to_string());

        let openai_api_key = api_key_cli
            .or(api_key_env)
            .or(file_cfg.openai_api_key)
            .expect("OPENAI_API_KEY must be set via env var or CLI");

        Config { model, openai_api_key }
    }
}

#[derive(Debug, Default, Deserialize)]
struct FileConfig {
    /// Default model to use when not provided via CLI or env.
    pub model: Option<String>,
    pub openai_api_key: Option<String>,
}

/// Return `~/.config/commitbot.toml`
fn config_path() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    Some(home.join(".config").join("commitbot.toml"))
}

fn load_file_config() -> Option<FileConfig> {
    let path = config_path()?;
    if !path.exists() {
        return None;
    }

    let data = fs::read_to_string(&path).ok()?;
    toml::from_str::<FileConfig>(&data).ok()
}
