use crate::cli_args::Cli;
use crate::config::Config;
use crate::llm::{LlmClient, NoopClient};
use crate::llm::openai::OpenAiClient;

/// Build the LLM client (OpenAI or Noop) based on CLI + config.
pub fn build_llm_client(cli: &Cli, cfg: &Config) -> Box<dyn LlmClient> {
    let use_no_model = cli.no_model || cfg.model.to_lowercase() == "none";

    if use_no_model {
        if cli.debug {
            eprintln!("[DEBUG] Using NoopClient (no model calls).");
        }
        Box::new(NoopClient)
    } else {
        let key = cfg.openai_api_key.clone();

        if cli.debug {
            eprintln!("[DEBUG] Using OpenAiClient with model: {}", cfg.model);
        }

        Box::new(OpenAiClient::new(key, cfg.model.clone()))
    }
}
