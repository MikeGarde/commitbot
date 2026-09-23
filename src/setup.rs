use anyhow::{anyhow, Result};
use crate::config::Config;
use crate::llm::LlmClient;
use crate::llm::ollama::OllamaClient;
use crate::llm::openai::{ModelValidation, OpenAiClient};

/// Build the LLM client based on CLI + config.
pub fn build_llm_client(cfg: &Config) -> Result<Box<dyn LlmClient>> {
    match cfg.provider.as_str() {
        "openai" => {
            let base_url = cfg
                .base_url
                .clone()
                .unwrap_or_else(|| "https://api.openai.com".to_string());

            let is_custom_endpoint = !base_url.starts_with("https://api.openai.com");
            let validation = if is_custom_endpoint {
                ModelValidation::List
            } else {
                ModelValidation::Retrieve
            };

            log::debug!(
                "Using OpenAiClient with model: {} (stream={}, timeout={}s)",
                cfg.model,
                cfg.stream,
                cfg.request_timeout_secs
            );

            Ok(Box::new(OpenAiClient::openai_compatible(
                cfg.openai_api_key.clone(),
                cfg.model.clone(),
                base_url,
                cfg.stream,
                cfg.request_timeout_secs,
                "OpenAI",
                validation,
            )))
        }
        "ollama" => {
            let base_url = cfg
                .base_url
                .clone()
                .unwrap_or_else(|| "http://localhost:11434".to_string());

            log::debug!(
                "Using OllamaClient with model: {} (stream={}, timeout={}s)",
                cfg.model,
                cfg.stream,
                cfg.request_timeout_secs
            );

            Ok(Box::new(OllamaClient::new(
                base_url,
                cfg.model.clone(),
                cfg.stream,
                cfg.request_timeout_secs,
            )))
        }
        "lmstudio" => {
            let base_url = cfg
                .base_url
                .clone()
                .unwrap_or_else(|| "http://localhost:1234".to_string());

            log::debug!(
                "Using LM Studio at {} with model: {} (stream={}, timeout={}s)",
                base_url,
                cfg.model,
                cfg.stream,
                cfg.request_timeout_secs
            );

            Ok(Box::new(OpenAiClient::openai_compatible(
                cfg.openai_api_key.clone(),
                cfg.model.clone(),
                base_url,
                cfg.stream,
                cfg.request_timeout_secs,
                "LM Studio",
                ModelValidation::List,
            )))
        }
        other => Err(anyhow!(
            "Unknown provider: {:?} (expected one of: openai, ollama, lmstudio)",
            other
        )),
    }
}
