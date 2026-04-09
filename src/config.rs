use crate::{Cli, git};
use anyhow::{anyhow, Result};
use git::detect_repo_id;
use serde::Deserialize;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

/// Final resolved configuration for commitbot.
#[derive(Debug, Clone)]
pub struct Config {
    /// LLM provider (openai, ollama)
    pub provider: String,
    /// OpenAI API key for authentication (sensitive – redacted in logs)
    pub openai_api_key: Option<String>,
    /// Base URL for the LLM provider
    pub base_url: Option<String>,
    /// Model name to use for LLM calls
    pub model: String,
    /// Maximum concurrent requests to the LLM API
    pub max_concurrent_requests: usize,
    /// Whether to stream responses from the LLM
    pub stream: bool,
}

impl Config {
    /// Returns the names of fields that contain sensitive data (e.g. API keys).
    /// These are redacted in debug/verbose log output.
    pub fn sensitive_field_names() -> &'static [&'static str] {
        &["openai_api_key"]
    }

    /// Build the final config from CLI flags, environment, TOML file, and defaults.
    ///
    /// Precedence (highest to lowest):
    ///   1. CLI flags
    ///   2. Env vars
    ///   3. Per-repo table in config file (e.g. ["mikegarde/commitbot"])
    ///   4. [default] table in config file
    ///   5. Hardcoded defaults
    pub fn from_sources(cli: &Cli) -> Result<Self> {
        let r = ConfigResolver::new(cli);

        let provider = r.get_string("provider", "openai").to_lowercase();
        let model = r.get_string("model", "gpt-5-nano");

        // secrets: logged as <set>/<unset>
        let openai_api_key = r.get_secret_opt_string("openai_api_key");

        // optional
        let base_url = r.get_opt_string("base_url");

        let max_concurrent_requests = r.get_usize("max_concurrent_requests", 4);
        let stream = r.get_bool("stream", true);

        // Cleanup: trim stray quotes if any upstream included them
        let provider = provider.trim_matches('"').to_string();
        let model = model.trim_matches('"').to_string();
        let openai_api_key = openai_api_key.map(|s| s.trim_matches('"').to_string());
        let base_url = base_url.map(|s| s.trim_matches('"').to_string());

        if provider == "openai" && openai_api_key.is_none() {
            return Err(anyhow!(
                "OPENAI_API_KEY must be set via CLI, env var, or config file for provider=openai"
            ));
        }

        Ok(Config {
            provider,
            model,
            openai_api_key,
            base_url,
            max_concurrent_requests,
            stream,
        })
    }
}

#[derive(Debug, Default, Deserialize, Clone)]
struct FileConfig {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub openai_api_key: Option<String>,
    #[serde(alias = "url")]
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

#[derive(Debug, Clone, Copy)]
enum ValueSource {
    Hardcoded,
    FileDefault,
    FileRepo,
    Env,
    Cli,
}

fn source_label(s: ValueSource) -> &'static str {
    match s {
        ValueSource::Cli => "cli",
        ValueSource::Env => "env",
        ValueSource::FileRepo => "file:[repo]",
        ValueSource::FileDefault => "file:[default]",
        ValueSource::Hardcoded => "hardcoded",
    }
}

struct ConfigResolver<'a> {
    cli: &'a Cli,
    repo_id: Option<String>,
    file_default: FileConfig,
    file_repo: FileConfig,
}

impl<'a> ConfigResolver<'a> {
    pub fn new(cli: &'a Cli) -> Self {
        // config file string (cli > env > default string)
        let config_file_to_use: String = cli
            .config
            .clone()
            .or_else(|| env::var("COMMITBOT_CONFIG").ok())
            .unwrap_or_else(|| "~/.config/commitbot.toml".to_string());

        let config_path = expand_tilde_to_path(&config_file_to_use);
        log::debug!("Config File: {}", config_path.display());

        let root = load_file_config_from_path(&config_path);

        let repo_id = detect_repo_id();
        log::debug!("Repo ID: {:?}", repo_id);

        let file_default = root.default.clone().unwrap_or_default();
        let file_repo = repo_id
            .as_deref()
            .and_then(|id| root.repos.get(id))
            .cloned()
            .unwrap_or_default();

        if let Some(id) = repo_id.as_deref() {
            log::debug!("Repo table present: {}", root.repos.contains_key(id));
        }

        Self {
            cli,
            repo_id,
            file_default,
            file_repo,
        }
    }

    fn env_key_for(&self, key: &str) -> Option<&'static str> {
        match key {
            "provider" => Some("COMMITBOT_PROVIDER"),
            "model" => Some("COMMITBOT_MODEL"),
            "openai_api_key" => Some("OPENAI_API_KEY"),
            "base_url" => Some("COMMITBOT_BASE_URL"),
            "max_concurrent_requests" => Some("COMMITBOT_MAX_CONCURRENT_REQUESTS"),
            "stream" => Some("COMMITBOT_STREAM"),
            _ => None,
        }
    }

    // ---- FILE SOURCES ----

    fn file_string(&self, key: &str, repo: bool) -> Option<String> {
        let cfg = if repo {
            &self.file_repo
        } else {
            &self.file_default
        };
        match key {
            "provider" => cfg.provider.clone(),
            "model" => cfg.model.clone(),
            "openai_api_key" => cfg.openai_api_key.clone(),
            "base_url" => cfg.base_url.clone(),
            _ => None,
        }
    }

    fn file_usize(&self, key: &str, repo: bool) -> Option<usize> {
        let cfg = if repo {
            &self.file_repo
        } else {
            &self.file_default
        };
        match key {
            "max_concurrent_requests" => cfg.max_concurrent_requests,
            _ => None,
        }
    }

    fn file_bool(&self, key: &str, repo: bool) -> Option<bool> {
        let cfg = if repo {
            &self.file_repo
        } else {
            &self.file_default
        };
        match key {
            "stream" => cfg.stream,
            _ => None,
        }
    }

    // ---- ENV SOURCES ----

    fn env_string(&self, key: &str) -> Option<String> {
        let env_key = self.env_key_for(key)?;
        env::var(env_key).ok()
    }

    fn env_usize(&self, key: &str) -> Option<usize> {
        let env_key = self.env_key_for(key)?;
        env::var(env_key).ok().and_then(|s| s.parse::<usize>().ok())
    }

    fn env_bool(&self, key: &str) -> Option<bool> {
        let env_key = self.env_key_for(key)?;
        env::var(env_key).ok().and_then(|s| s.parse::<bool>().ok())
    }

    // Expectation for a stream flag:
    //   - `--no-stream` present => stream = Some(false)
    //   - absent                => stream = None (file/env/default wins)
    fn cli_string(&self, key: &str) -> Option<String> {
        match key {
            "provider" => self.cli.provider.clone(),
            "model" => self.cli.model.clone(),
            "openai_api_key" => self.cli.api_key.clone(),
            "base_url" => self.cli.url.clone(),
            _ => None,
        }
    }

    fn cli_usize(&self, key: &str) -> Option<usize> {
        match key {
            "max_concurrent_requests" => self.cli.max,
            _ => None,
        }
    }

    fn cli_bool(&self, key: &str) -> Option<bool> {
        match key {
            "stream" => self.cli.no_stream.then_some(false),
            _ => None,
        }
    }

    fn log_decision<T: std::fmt::Debug>(&self, key: &str, value: &T, src: ValueSource) {
        // Redact sensitive keys (API keys, tokens, secrets) when printing
        // so verbose/debug logs don't leak credentials. For option-like
        // values we print <set>/<unset>, otherwise we print <redacted>.
        let printable = if self.is_sensitive_key(key) {
            // Try to detect Option<T> formatted output ("Some(...)" / "None").
            let debug_str = format!("{:?}", value);
            if debug_str == "None" {
                "<unset>".to_string()
            } else if debug_str.starts_with("Some(") {
                "<set>".to_string()
            } else {
                "<redacted>".to_string()
            }
        } else {
            format!("{:?}", value)
        };

        if let Some(env_key) = self.env_key_for(key) {
            log::debug!(
                "Config: {} = {} (source={}, env={})",
                key,
                printable,
                source_label(src),
                env_key
            );
        } else {
            log::debug!(
                "Config: {} = {} (source={})",
                key,
                printable,
                source_label(src)
            );
        }
    }

    // Treat a small set of keys as sensitive so logs redact them.
    // Add other names here if you later introduce more secrets.
    // Currently only the OpenAI API key is treated as sensitive. We keep the
    // check centralized here so it can later be expanded. The field in the
    // structs is annotated with a doc comment to indicate sensitivity.
    fn is_sensitive_key(&self, key: &str) -> bool {
        // Use the generated list from the Config struct (proc-macro)
        // to determine which fields are sensitive.
        Config::sensitive_field_names().contains(&key)
    }

    fn log_decision_secret_opt_string(&self, key: &str, present: bool, src: ValueSource) {
        let printable = if present { "<set>" } else { "<unset>" };
        if let Some(env_key) = self.env_key_for(key) {
            log::debug!(
                "Config: {} = {} (source={}, env={})",
                key,
                printable,
                source_label(src),
                env_key
            );
        } else {
            log::debug!(
                "Config: {} = {} (source={})",
                key,
                printable,
                source_label(src)
            );
        }
    }

    /// Resolve a required string.
    pub fn get_string(&self, key: &str, default: &str) -> String {
        let mut value = default.to_string();
        let mut src = ValueSource::Hardcoded;

        if let Some(v) = self.file_string(key, false) {
            value = v;
            src = ValueSource::FileDefault;
        }
        if let Some(v) = self.file_string(key, true) {
            value = v;
            src = ValueSource::FileRepo;
        }
        if let Some(v) = self.env_string(key) {
            value = v;
            src = ValueSource::Env;
        }
        if let Some(v) = self.cli_string(key) {
            value = v;
            src = ValueSource::Cli;
        }

        self.log_decision(key, &value, src);
        value
    }

    /// Resolve an optional string (None if not set anywhere).
    pub fn get_opt_string(&self, key: &str) -> Option<String> {
        let mut value: Option<String> = None;
        let mut src = ValueSource::Hardcoded;

        if let Some(v) = self.file_string(key, false) {
            value = Some(v);
            src = ValueSource::FileDefault;
        }
        if let Some(v) = self.file_string(key, true) {
            value = Some(v);
            src = ValueSource::FileRepo;
        }
        if let Some(v) = self.env_string(key) {
            value = Some(v);
            src = ValueSource::Env;
        }
        if let Some(v) = self.cli_string(key) {
            value = Some(v);
            src = ValueSource::Cli;
        }

        self.log_decision(key, &value, src);
        value
    }

    /// Resolve an optional secret string; logs <set>/<unset> only.
    pub fn get_secret_opt_string(&self, key: &str) -> Option<String> {
        let mut value: Option<String> = None;
        let mut src = ValueSource::Hardcoded;

        if let Some(v) = self.file_string(key, false) {
            value = Some(v);
            src = ValueSource::FileDefault;
        }
        if let Some(v) = self.file_string(key, true) {
            value = Some(v);
            src = ValueSource::FileRepo;
        }
        if let Some(v) = self.env_string(key) {
            value = Some(v);
            src = ValueSource::Env;
        }
        if let Some(v) = self.cli_string(key) {
            value = Some(v);
            src = ValueSource::Cli;
        }

        self.log_decision_secret_opt_string(key, value.is_some(), src);
        value
    }

    /// Resolve a usize.
    pub fn get_usize(&self, key: &str, default: usize) -> usize {
        let mut value = default;
        let mut src = ValueSource::Hardcoded;

        if let Some(v) = self.file_usize(key, false) {
            value = v;
            src = ValueSource::FileDefault;
        }
        if let Some(v) = self.file_usize(key, true) {
            value = v;
            src = ValueSource::FileRepo;
        }
        if let Some(v) = self.env_usize(key) {
            value = v;
            src = ValueSource::Env;
        }
        if let Some(v) = self.cli_usize(key) {
            value = v;
            src = ValueSource::Cli;
        }

        self.log_decision(key, &value, src);
        value
    }

    /// Resolve a bool.
    pub fn get_bool(&self, key: &str, default: bool) -> bool {
        let mut value = default;
        let mut src = ValueSource::Hardcoded;

        if let Some(v) = self.file_bool(key, false) {
            value = v;
            src = ValueSource::FileDefault;
        }
        if let Some(v) = self.file_bool(key, true) {
            value = v;
            src = ValueSource::FileRepo;
        }
        if let Some(v) = self.env_bool(key) {
            value = v;
            src = ValueSource::Env;
        }
        if let Some(v) = self.cli_bool(key) {
            value = v;
            src = ValueSource::Cli;
        }

        self.log_decision(key, &value, src);
        value
    }

    #[allow(dead_code)]
    pub fn repo_id(&self) -> Option<&str> {
        self.repo_id.as_deref()
    }
}

fn expand_tilde_to_path(s: &str) -> PathBuf {
    if let (Some(rest), Some(home)) = (s.strip_prefix("~/"), env::var_os("HOME")) {
        return PathBuf::from(home).join(rest);
    }
    PathBuf::from(s)
}

fn load_file_config_from_path(path: &Path) -> FileConfigRoot {
    if !path.exists() {
        log::warn!("Config file not found: {}", path.display());
        return FileConfigRoot::default();
    }

    let data = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(err) => {
            log::warn!("Failed to read config file {}: {}", path.display(), err);
            return FileConfigRoot::default();
        }
    };

    match toml::from_str::<FileConfigRoot>(&data) {
        Ok(cfg) => cfg,
        Err(err) => {
            log::warn!("Invalid TOML in {}: {}", path.display(), err);
            FileConfigRoot::default()
        }
    }
}
