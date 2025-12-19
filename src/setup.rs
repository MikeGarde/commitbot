use crate::config::Config;
use crate::llm::LlmClient;
use crate::llm::openai::OpenAiClient;
use crate::llm::ollama::OllamaClient;

/// Build the LLM client based on CLI + config.
pub fn build_llm_client(cfg: &Config) -> Box<dyn LlmClient> {
    match cfg.provider.as_str() {
        "openai" => {
            let key = cfg
                .openai_api_key
                .clone()
                .expect("OPENAI_API_KEY must be set for provider=openai");
            let base_url = cfg
                .base_url
                .clone()
                .unwrap_or_else(|| "https://api.openai.com".to_string());

            log::debug!(
                "Using OpenAiClient with model: {} (stream={})",
                cfg.model, cfg.stream
            );

            Box::new(OpenAiClient::new(
                key,
                cfg.model.clone(),
                base_url,
                cfg.stream,
            ))
        }
        "ollama" => {
            let base_url = cfg
                .base_url
                .clone()
                .unwrap_or_else(|| "http://localhost:11434".to_string());

            log::debug!(
                "Using OllamaClient with model: {} (stream={})",
                cfg.model, cfg.stream
            );

            Box::new(OllamaClient::new(
                base_url,
                cfg.model.clone(),
                cfg.stream,
            ))
        }
        other => panic!("Unknown provider: {}", other),
    }
}
