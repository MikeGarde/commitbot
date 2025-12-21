use anyhow::{anyhow, Result};
use log;
use musli::{Encode, Decode};
use musli::json;
use reqwest::blocking::Client;
use std::io::BufReader;

use crate::FileChange;
use crate::git::{PrItem, PrSummaryMode};

use super::{prompt_builder, LlmClient};
use super::stream::read_stream_to_string;

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

/// Synchronous Ollama client using /api/chat.
pub struct OllamaClient {
    http: Client,
    base_url: String,
    model: String,
    stream: bool,
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

        let parsed: OllamaChatResponse =
            json::from_str(&resp_text).map_err(|e| anyhow!("Failed to decode Ollama JSON: {e}"))?;

        Ok(parsed.message.content.trim().to_string())
    }

}

fn parse_stream_line(line: &str) -> Result<Option<String>> {
    let parsed: OllamaStreamResponse =
        json::from_str(line).map_err(|e| anyhow!("Failed to decode Ollama stream JSON: {e}"))?;

    if parsed.done.unwrap_or(false) {
        return Ok(None);
    }

    let content = parsed
        .message
        .and_then(|m| if m.content.is_empty() { None } else { Some(m.content) });

    Ok(content)
}

impl LlmClient for OllamaClient {
    fn summarize_file(
        &self,
        branch: &str,
        file: &FileChange,
        ticket_summary: Option<&str>,
    ) -> Result<String> {
        let prompts = prompt_builder::file_summary_prompt(branch, file, ticket_summary);
        self.chat(prompts.system, prompts.user, false)
    }

    fn generate_commit_message(
        &self,
        branch: &str,
        files: &[FileChange],
        ticket_summary: Option<&str>,
    ) -> Result<String> {
        let prompts = prompt_builder::commit_message_prompt(branch, files, ticket_summary);
        self.chat(prompts.system, prompts.user, self.stream)
    }

    fn generate_commit_message_simple(
        &self,
        branch: &str,
        diff: &str,
        ticket_summary: Option<&str>,
    ) -> Result<String> {
        let prompts = prompt_builder::commit_message_simple_prompt(branch, diff, ticket_summary);
        self.chat(prompts.system, prompts.user, self.stream)
    }

    fn generate_pr_message(
        &self,
        base_branch: &str,
        from_branch: &str,
        mode: PrSummaryMode,
        items: &[PrItem],
        ticket_summary: Option<&str>,
    ) -> Result<String> {
        let prompts =
            prompt_builder::pr_message_prompt(base_branch, from_branch, mode, items, ticket_summary);
        self.chat(prompts.system, prompts.user, self.stream)
    }
}
