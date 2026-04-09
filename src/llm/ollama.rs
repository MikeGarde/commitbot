use anyhow::{Result, anyhow};
use log;
use musli::json;
use musli::{Decode, Encode};
use reqwest::StatusCode;
use reqwest::blocking::Client;
use std::io::BufReader;
use std::sync::Mutex;

use crate::FileChange;
use crate::git::{PrItem, PrSummaryMode};

use super::stream::read_stream_to_string;
use super::{LlmClient, prompt_builder};

#[derive(Debug, Encode, Decode)]
struct OllamaMessage {
    role: String,
    content: String,
}

#[derive(Debug, Encode, Decode)]
struct OllamaChatResponse {
    message: OllamaMessage,
}

#[derive(Debug, Decode)]
struct OllamaStreamResponse {
    message: Option<OllamaMessage>,
    done: Option<bool>,
}

#[derive(Debug, Decode)]
struct OllamaUsage {
    prompt_tokens: Option<u32>,
    completion_tokens: Option<u32>,
    total_tokens: Option<u32>,
}

#[derive(Debug, Decode)]
struct OllamaTagModel {
    name: String,
}

#[derive(Debug, Decode)]
struct OllamaTagsResponse {
    models: Vec<OllamaTagModel>,
}

/// Synchronous Ollama client using /api/chat.
pub struct OllamaClient {
    http: Client,
    base_url: String,
    model: String,
    stream: bool,
    usage: Mutex<TokenUsage>,
}

#[derive(Default)]
struct TokenUsage {
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
}

impl OllamaClient {
    pub fn new(base_url: impl Into<String>, model: impl Into<String>, stream: bool) -> Self {
        let http = Client::builder()
            .build()
            .expect("failed to build HTTP client");
        Self {
            http,
            base_url: base_url.into().trim_end_matches('/').to_string(),
            model: model.into(),
            stream,
            usage: Mutex::new(TokenUsage::default()),
        }
    }

    /// Internal helper to talk to /api/chat.
    fn chat(&self, system_prompt: String, user_prompt: String, stream: bool) -> Result<String> {
        // Request structs we encode with musli::json.
        #[derive(Debug, Encode)]
        struct ChatMessage {
            role: String,
            content: String,
        }

        #[derive(Debug, Encode)]
        struct ChatRequest {
            model: String,
            stream: bool,
            messages: Vec<ChatMessage>,
        }

        let req_body = ChatRequest {
            model: self.model.clone(),
            stream,
            messages: vec![
                ChatMessage {
                    role: "system".to_string(),
                    content: system_prompt,
                },
                ChatMessage {
                    role: "user".to_string(),
                    content: user_prompt,
                },
            ],
        };

        let body_str = json::to_string(&req_body)
            .map_err(|e| anyhow!("Failed to encode Ollama JSON request: {e}"))?;

        log::trace!("Ollama request body: {body_str}");

        let url = format!("{}/api/chat", self.base_url);

        let resp = self
            .http
            .post(&url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body_str)
            .send()
            .map_err(|e| anyhow!("Error calling Ollama at {url}: {e}"))?
            .error_for_status()
            .map_err(|e| anyhow!("Ollama HTTP error from {url}: {e}"))?;

        if stream {
            let reader = BufReader::new(resp);
            return read_stream_to_string(reader, parse_stream_line);
        }

        let resp_text = resp
            .text()
            .map_err(|e| anyhow!("Failed to read Ollama response body: {e}"))?;

        log::trace!("Ollama raw JSON response: {resp_text}");

        #[derive(Debug, Decode)]
        struct OllamaChatWrapper {
            message: OllamaMessage,
            usage: Option<OllamaUsage>,
        }

        // First try decoding into the wrapper. If that fails fall back to the
        // older shape without usage.
        match json::from_str::<OllamaChatWrapper>(&resp_text) {
            Ok(parsed) => {
                if let Some(usage) = parsed.usage {
                    let mut u = self.usage.lock().expect("usage mutex poisoned");
                    u.prompt_tokens += usage.prompt_tokens.unwrap_or(0) as u64;
                    u.completion_tokens += usage.completion_tokens.unwrap_or(0) as u64;
                    u.total_tokens += usage.total_tokens.unwrap_or(0) as u64;
                }
                Ok(parsed.message.content.trim().to_string())
            }
            Err(_) => {
                // Fallback to the simple response shape.
                let parsed: OllamaChatResponse = json::from_str(&resp_text)
                    .map_err(|e| anyhow!("Failed to decode Ollama JSON: {e}"))?;
                Ok(parsed.message.content.trim().to_string())
            }
        }
    }

    fn tags_url(&self) -> String {
        format!("{}/api/tags", self.base_url)
    }
}

fn parse_stream_line(line: &str) -> Result<Option<String>> {
    let parsed: OllamaStreamResponse =
        json::from_str(line).map_err(|e| anyhow!("Failed to decode Ollama stream JSON: {e}"))?;

    if parsed.done.unwrap_or(false) {
        return Ok(None);
    }

    let content = parsed.message.and_then(|m| {
        if m.content.is_empty() {
            None
        } else {
            Some(m.content)
        }
    });

    Ok(content)
}

impl LlmClient for OllamaClient {
    fn validate_model(&self) -> Result<()> {
        let url = self.tags_url();
        let resp = self
            .http
            .get(&url)
            .send()
            .map_err(|e| anyhow!("Error calling Ollama at {url}: {e}"))?;

        if resp.status() != StatusCode::OK {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            return Err(anyhow!(
                "Ollama model validation failed at {url}: HTTP {} - {}",
                status.as_u16(),
                body
            ));
        }

        let body = resp
            .text()
            .map_err(|e| anyhow!("Failed to read Ollama tags response from {url}: {e}"))?;
        let parsed: OllamaTagsResponse = json::from_str(&body)
            .map_err(|e| anyhow!("Failed to decode Ollama tags response from {url}: {e}"))?;

        if parsed.models.iter().any(|model| model.name == self.model) {
            return Ok(());
        }

        let available = parsed
            .models
            .iter()
            .map(|model| model.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");

        Err(anyhow!(
            "Model {:?} was not found at {}. Available models: {}",
            self.model,
            url,
            if available.is_empty() {
                "<none>"
            } else {
                &available
            }
        ))
    }

    fn summarize_file(
        &self,
        branch: &str,
        file: &FileChange,
        file_index: usize,
        total_files: usize,
        ticket_summary: Option<&str>,
    ) -> Result<String> {
        let prompts = prompt_builder::file_summary_prompt(
            branch,
            file,
            file_index,
            total_files,
            ticket_summary,
        );
        self.chat(prompts.system, prompts.user, false)
    }

    fn generate_commit_message(
        &self,
        branch: &str,
        files: &[FileChange],
        ticket_summary: Option<&str>,
    ) -> Result<String> {
        let prompts = prompt_builder::commit_message_prompt(branch, files, ticket_summary);
        let content = self.chat(prompts.system, prompts.user, self.stream)?;
        self.log_and_reset_usage();
        Ok(content)
    }

    fn generate_pr_message(
        &self,
        base_branch: &str,
        from_branch: &str,
        mode: PrSummaryMode,
        items: &[PrItem],
        ticket_summary: Option<&str>,
    ) -> Result<String> {
        let prompts = prompt_builder::pr_message_prompt(
            base_branch,
            from_branch,
            mode,
            items,
            ticket_summary,
        );
        let content = self.chat(prompts.system, prompts.user, self.stream)?;
        self.log_and_reset_usage();
        Ok(content)
    }
}

impl OllamaClient {
    fn log_and_reset_usage(&self) {
        let mut u = self.usage.lock().expect("usage mutex poisoned");
        if u.total_tokens > 0 {
            log::warn!(
                "Token usage (aggregate): prompt={}, completion={}, total={}",
                u.prompt_tokens,
                u.completion_tokens,
                u.total_tokens
            );
            *u = TokenUsage::default();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_trailing_slash_in_tags_url() {
        let client = OllamaClient::new("http://localhost:11434/", "qwen3-coder:30b", false);
        assert_eq!(client.tags_url(), "http://localhost:11434/api/tags");
    }

    #[test]
    fn decodes_ollama_tags_payload() {
        let body = r#"{"models":[{"name":"qwen3-coder:30b"},{"name":"gpt-oss:20b"}]}"#;
        let parsed: OllamaTagsResponse = json::from_str(body).expect("valid tags payload");

        assert!(parsed
            .models
            .iter()
            .any(|model| model.name == "qwen3-coder:30b"));
        assert!(!parsed
            .models
            .iter()
            .any(|model| model.name == "missing:model"));
    }
}
