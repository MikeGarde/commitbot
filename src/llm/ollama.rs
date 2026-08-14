use anyhow::{Result, anyhow};
use log;
use musli::json;
use musli::{Decode, Encode};
use reqwest::blocking::Client;
use std::io::BufReader;
use std::sync::Mutex;
use std::time::Instant;

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

// Scaffolding for a future "list models" command; only exercised by tests today.
#[allow(dead_code)]
#[derive(Debug, Decode)]
struct OllamaTagModel {
    name: String,
}

#[allow(dead_code)]
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
    pub fn new(base_url: impl Into<String>, model: impl Into<String>, stream: bool, timeout_secs: u64) -> Self {
        let http = Client::builder()
            .timeout(std::time::Duration::from_secs(timeout_secs))
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

        let t0 = Instant::now();
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
            let result = read_stream_to_string(reader, parse_stream_line);
            log::debug!("Ollama response time: {:.2?}", t0.elapsed());
            return result;
        }

        let resp_text = resp
            .text()
            .map_err(|e| anyhow!("Failed to read Ollama response body: {e}"))?;

        log::debug!("Ollama response time: {:.2?}", t0.elapsed());
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
                    // Recover from a poisoned mutex instead of panicking so the CLI
                    // can continue in the face of concurrent thread panics.
                    let mut u = self.usage.lock().unwrap_or_else(|e| {
                        log::warn!("usage mutex was poisoned, recovering token counters");
                        e.into_inner()
                    });
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

    /// Scaffolding for a future "list models" command; only exercised by tests today.
    #[allow(dead_code)]
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
        // Skipping model validation for Ollama; use a future "list models" command
        // to query /api/tags if the user requests it.
        Ok(())
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
        Ok(content)
    }
    fn take_and_reset_usage(&self) -> Option<(u64, u64, u64)> {
        let mut u = self.usage.lock().unwrap_or_else(|e| {
            log::warn!("usage mutex was poisoned, recovering token counters");
            e.into_inner()
        });

        if u.total_tokens > 0 {
            let res = (u.prompt_tokens, u.completion_tokens, u.total_tokens);
            *u = TokenUsage::default();
            Some(res)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_trailing_slash_in_tags_url() {
        let client = OllamaClient::new("http://localhost:11434/", "gemma4:31b", false, 90);
        assert_eq!(client.tags_url(), "http://localhost:11434/api/tags");
    }

    #[test]
    fn decodes_ollama_tags_payload() {
        let body = r#"{"models":[{"name":"gemma4:31b"},{"name":"qwen3-coder:30b"},{"name":"gpt-oss:20b"}]}"#;
        let parsed: OllamaTagsResponse = json::from_str(body).expect("valid tags payload");

        assert!(parsed
            .models
            .iter()
            .any(|model| model.name == "gemma4:31b"));
        assert!(!parsed
            .models
            .iter()
            .any(|model| model.name == "missing:model"));
    }
}
