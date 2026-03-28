use anyhow::{anyhow, Result};
use crate::config::Config;
use crate::llm::LlmClient;
use crate::llm::ollama::OllamaClient;
use crate::llm::openai::OpenAiClient;

/// Build the LLM client based on CLI + config.
pub fn build_llm_client(cfg: &Config) -> Result<Box<dyn LlmClient>> {
    match cfg.provider.as_str() {
        "openai" => {
            let key = cfg
                .openai_api_key
                .clone()
                .ok_or_else(|| anyhow!("OPENAI_API_KEY must be set for provider=openai"))?;
            let base_url = cfg
                .base_url
                .clone()
                .unwrap_or_else(|| "https://api.openai.com".to_string());

            log::debug!(
                "Using OpenAiClient with model: {} (stream={})",
                cfg.model,
                cfg.stream
            );

            Ok(Box::new(OpenAiClient::new(
                key,
                cfg.model.clone(),
                base_url,
                cfg.stream,
            )))
        }
        "ollama" => {
            let base_url = cfg
                .base_url
                .clone()
                .unwrap_or_else(|| "http://localhost:11434".to_string());

            log::debug!(
                "Using OllamaClient with model: {} (stream={})",
                cfg.model,
                cfg.stream
            );

            Ok(Box::new(OllamaClient::new(
                base_url,
                cfg.model.clone(),
                cfg.stream,
            )))
        }
        other => Err(anyhow!("Unknown provider: {}", other)),
    }
}
