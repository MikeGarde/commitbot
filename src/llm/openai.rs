use super::LlmClient;
use super::stream::read_stream_to_string;
use crate::{FileChange};
use crate::git::{PrItem, PrSummaryMode};
use anyhow::{anyhow, Context, Result};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::io::BufReader;
use std::time::Duration;
use super::prompt_builder;

/// Minimal request/response structs for OpenAI Chat Completions API.
#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    stream: bool,
}

#[derive(Serialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
    usage: Option<ChatUsage>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessageResponse,
}

#[derive(Deserialize)]
struct ChatMessageResponse {
    content: String,
}

#[derive(Deserialize)]
struct ChatUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
}

#[derive(Deserialize)]
struct StreamResponse {
    choices: Vec<StreamChoice>,
}

#[derive(Deserialize)]
struct StreamChoice {
    delta: StreamDelta,
}

#[derive(Deserialize)]
struct StreamDelta {
    content: Option<String>,
}

/// OpenAI-based implementation of LlmClient.
pub struct OpenAiClient {
    client: Client,
    api_key: String,
    model: String,
    api_base_url: String,
    stream: bool,
}

impl OpenAiClient {
    pub fn new(api_key: String, model: String, api_base_url: String, stream: bool) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(90))
            .build()
            .expect("failed to build HTTP client");

        OpenAiClient {
            client,
            api_key,
            model,
            api_base_url: api_base_url.trim_end_matches('/').to_string(),
            stream,
        }
    }

    fn chat_url(&self) -> String {
        if self.api_base_url.ends_with("/v1") {
            format!("{}/chat/completions", self.api_base_url)
        } else {
            format!("{}/v1/chat/completions", self.api_base_url)
        }
    }

    fn call_chat(&self, req: &ChatRequest) -> Result<String> {
        if req.stream {
            return self.call_chat_streaming(req);
        }

        let url = self.chat_url();

        log::info!("Calling OpenAI model {:?}", &req.model);

        let resp = self
            .client
            .post(url)
            .bearer_auth(&self.api_key)
            .json(req)
            .send()
            .context("failed to send request to OpenAI")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().unwrap_or_default();
            return Err(anyhow!(
                "OpenAI API error: HTTP {} - {}",
                status.as_u16(),
                text
            ));
        }

        let chat_resp: ChatResponse = resp.json().context("failed to parse OpenAI response")?;
        let content = chat_resp
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .ok_or_else(|| anyhow!("no choices returned from OpenAI"))?;

        if let Some(usage) = &chat_resp.usage {
            log::warn!("Token usage: prompt={}, completion={}, total={}",
                usage.prompt_tokens, usage.completion_tokens, usage.total_tokens
            );
        }

        Ok(content)
    }

    fn call_chat_streaming(&self, req: &ChatRequest) -> Result<String> {
        let url = self.chat_url();

        log::info!("Streaming OpenAI model {:?}", &req.model);

        let resp = self
            .client
            .post(url)
            .bearer_auth(&self.api_key)
            .json(req)
            .send()
            .context("failed to send streaming request to OpenAI")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().unwrap_or_default();
            return Err(anyhow!(
                "OpenAI API error: HTTP {} - {}",
                status.as_u16(),
                text
            ));
        }

        let reader = BufReader::new(resp);
        read_stream_to_string(reader, |line| parse_stream_line(line))
    }
}

fn parse_stream_line(line: &str) -> Result<Option<String>> {
    let line = line.trim_start();
    if !line.starts_with("data:") {
        return Ok(None);
    }

    let data = line.trim_start_matches("data:").trim();
    if data == "[DONE]" {
        return Ok(None);
    }

    let chunk: StreamResponse =
        serde_json::from_str(data).context("failed to parse OpenAI streaming chunk")?;
    let content = chunk
        .choices
        .first()
        .and_then(|c| c.delta.content.clone());

    Ok(content)
}

impl LlmClient for OpenAiClient {
    fn summarize_file(
        &self,
        branch: &str,
        file: &FileChange,
        ticket_summary: Option<&str>,
    ) -> Result<String> {
        let prompts = prompt_builder::file_summary_prompt(branch, file, ticket_summary);

        log::debug!(
            "Per-file summarize prompt for {} ({:?}):\n{}",
            file.path,
            file.category,
            truncate(&prompts.user, 2000)
        );

        let req = ChatRequest {
            model: self.model.clone(),
            messages: vec![
                ChatMessage {
                    role: "system".into(),
                    content: prompts.system,
                },
                ChatMessage {
                    role: "user".into(),
                    content: prompts.user,
                },
            ],
            stream: false,
        };

        let content = self.call_chat(&req)?;
        Ok(content)
    }

    fn generate_commit_message(
        &self,
        branch: &str,
        files: &[FileChange],
        ticket_summary: Option<&str>,
    ) -> Result<String> {
        let prompts = prompt_builder::commit_message_prompt(branch, files, ticket_summary);

        log::debug!(
            "Final commit-message prompt:\n{}",
            truncate(&prompts.user, 3000)
        );

        let req = ChatRequest {
            model: self.model.clone(),
            messages: vec![
                ChatMessage {
                    role: "system".into(),
                    content: prompts.system,
                },
                ChatMessage {
                    role: "user".into(),
                    content: prompts.user,
                },
            ],
            stream: self.stream,
        };

        let content = self.call_chat(&req)?;
        Ok(content)
    }

    fn generate_commit_message_simple(
        &self,
        branch: &str,
        diff: &str,
        ticket_summary: Option<&str>,
    ) -> Result<String> {
        let prompts = prompt_builder::commit_message_simple_prompt(branch, diff, ticket_summary);

        log::trace!(
            "Simple commit-message prompt:\n{}",
            truncate(&prompts.user, 3000)
        );

        let req = ChatRequest {
            model: self.model.clone(),
            messages: vec![
                ChatMessage {
                    role: "system".into(),
                    content: prompts.system,
                },
                ChatMessage {
                    role: "user".into(),
                    content: prompts.user,
                },
            ],
            stream: self.stream,
        };

        let content = self.call_chat(&req)?;
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
        let prompts =
            prompt_builder::pr_message_prompt(base_branch, from_branch, mode, items, ticket_summary);

        log::trace!(
            "PR description prompt:\n{}",
            truncate(&prompts.user, 3500)
        );

        let req = ChatRequest {
            model: self.model.clone(),
            messages: vec![
                ChatMessage {
                    role: "system".into(),
                    content: prompts.system,
                },
                ChatMessage {
                    role: "user".into(),
                    content: prompts.user,
                },
            ],
            stream: self.stream,
        };

        let content = self.call_chat(&req)?;
        Ok(content)
    }
}

/// Truncate long strings for debug logging.
fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...\n[truncated {} chars]", &s[..max_len], s.len() - max_len)
    }
}
