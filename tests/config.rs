use clap::Parser;
use commitbot::config::Config;
use commitbot::Cli;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn unique_config_path(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("commitbot_{name}_{nanos}.toml"))
}

fn write_temp_config(name: &str, contents: &str) -> PathBuf {
    let path = unique_config_path(name);
    fs::write(&path, contents).expect("write temp config");
    path
}

#[test]
fn missing_config_uses_cli_values() {
    let missing_path = unique_config_path("missing");
    let cli = Cli::parse_from([
        "commitbot",
        "--config",
        missing_path.to_str().expect("utf-8 path"),
        "--provider",
        "ollama",
        "--model",
        "qwen3-coder:30b",
        "--url",
        "http://localhost:11434",
    ]);

    let cfg = Config::from_sources(&cli).expect("config should resolve from cli");
    assert_eq!(cfg.provider, "ollama");
    assert_eq!(cfg.model, "qwen3-coder:30b");
    assert_eq!(cfg.base_url.as_deref(), Some("http://localhost:11434"));
    assert!(cfg.stream);
}

#[test]
fn reads_values_from_default_config_table() {
    let config_path = write_temp_config(
        "default_table",
        r#"
[default]
provider = "ollama"
model = "llama3.1:8b-instruct-q5_K_M"
url = "http://gpu.trigapi.com:11434"
max_concurrent_requests = 2
stream = false
"#,
    );

    let cli = Cli::parse_from([
        "commitbot",
        "--config",
        config_path.to_str().expect("utf-8 path"),
    ]);

    let cfg = Config::from_sources(&cli).expect("config should load default table");
    assert_eq!(cfg.provider, "ollama");
    assert_eq!(cfg.model, "llama3.1:8b-instruct-q5_K_M");
    assert_eq!(cfg.base_url.as_deref(), Some("http://gpu.trigapi.com:11434"));
    assert_eq!(cfg.max_concurrent_requests, 2);
    assert!(!cfg.stream);

    fs::remove_file(config_path).ok();
}

#[test]
fn cli_overrides_file_values() {
    let config_path = write_temp_config(
        "cli_override",
        r#"
[default]
provider = "ollama"
model = "qwen3-coder:30b"
url = "http://gpu.trigapi.com:11434"
max_concurrent_requests = 1
stream = true
"#,
    );

    let cli = Cli::parse_from([
        "commitbot",
        "--config",
        config_path.to_str().expect("utf-8 path"),
        "--model",
        "gpt-oss:20b",
        "--url",
        "http://localhost:11434",
        "--max",
        "4",
        "--no-stream",
    ]);

    let cfg = Config::from_sources(&cli).expect("cli values should override file");
    assert_eq!(cfg.provider, "ollama");
    assert_eq!(cfg.model, "gpt-oss:20b");
    assert_eq!(cfg.base_url.as_deref(), Some("http://localhost:11434"));
    assert_eq!(cfg.max_concurrent_requests, 4);
    assert!(!cfg.stream);

    fs::remove_file(config_path).ok();
}

#[test]
fn lmstudio_needs_no_url_or_api_key() {
    // Like ollama: the url is optional and falls back to a local default,
    // and no API key is required.
    let config_path = write_temp_config(
        "lmstudio_defaults",
        r#"
[default]
provider = "lmstudio"
model = "qwen/qwen3-coder-30b"
"#,
    );

    let cli = Cli::parse_from([
        "commitbot",
        "--config",
        config_path.to_str().expect("utf-8 path"),
    ]);

    let cfg = Config::from_sources(&cli).expect("lmstudio config should resolve");
    assert_eq!(cfg.provider, "lmstudio");
    assert_eq!(cfg.model, "qwen/qwen3-coder-30b");
    assert_eq!(cfg.base_url, None, "the default is applied when building the client");

    fs::remove_file(config_path).ok();
}

#[test]
fn lmstudio_url_can_point_at_a_remote_host() {
    let config_path = write_temp_config(
        "lmstudio_remote",
        r#"
[default]
provider = "lmstudio"
model = "qwen/qwen3-coder-30b"
url = "http://localhost:1234/v1"
"#,
    );

    let cli = Cli::parse_from([
        "commitbot",
        "--config",
        config_path.to_str().expect("utf-8 path"),
        "--url",
        "http://gpu.example.com:1234/v1",
    ]);

    let cfg = Config::from_sources(&cli).expect("cli url should override file");
    assert_eq!(cfg.provider, "lmstudio");
    assert_eq!(
        cfg.base_url.as_deref(),
        Some("http://gpu.example.com:1234/v1")
    );

    fs::remove_file(config_path).ok();
}

#[test]
fn invalid_openai_config_returns_error() {
    let config_path = write_temp_config(
        "invalid_openai",
        r#"
[default]
provider = "openai"
model = "gpt-5-nano"
"#,
    );

    let cli = Cli::parse_from([
        "commitbot",
        "--config",
        config_path.to_str().expect("utf-8 path"),
    ]);

    let err = Config::from_sources(&cli).expect_err("openai config should require an api key");
    assert!(
        err.to_string()
            .contains("OPENAI_API_KEY must be set via CLI, env var, or config file")
    );

    fs::remove_file(config_path).ok();
}
