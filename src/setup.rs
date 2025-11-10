use log::debug;
use crate::config::Config;
use crate::llm::LlmClient;
use crate::llm::openai::OpenAiClient;

/// Build the LLM client based on CLI + config.
pub fn build_llm_client(cfg: &Config) -> Box<dyn LlmClient> {
    let key = cfg.openai_api_key.clone();

    debug!("Using OpenAiClient with model: {}", cfg.model);

    Box::new(OpenAiClient::new(key, cfg.model.clone()))
}
