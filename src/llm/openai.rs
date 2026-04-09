use super::LlmClient;
use super::prompt_builder;
use super::stream::read_stream_to_string;
use crate::FileChange;
use crate::git::{PrItem, PrSummaryMode};
use anyhow::{Context, Result, anyhow};
use reqwest::StatusCode;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::io::BufReader;
use std::time::Duration;
use std::sync::Mutex;

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
    usage: Mutex<TokenUsage>,
}

#[derive(Default)]
struct TokenUsage {
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
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
            usage: Mutex::new(TokenUsage::default()),
        }
    }

    fn chat_url(&self) -> String {
        if self.api_base_url.ends_with("/v1") {
            format!("{}/chat/completions", self.api_base_url)
        } else {
            format!("{}/v1/chat/completions", self.api_base_url)
        }
    }

    fn model_url(&self) -> String {
        if self.api_base_url.ends_with("/v1") {
            format!("{}/models/{}", self.api_base_url, self.model)
        } else {
            format!("{}/v1/models/{}", self.api_base_url, self.model)
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
            let mut u = self.usage.lock().expect("usage mutex poisoned");
            u.prompt_tokens += usage.prompt_tokens as u64;
            u.completion_tokens += usage.completion_tokens as u64;
            u.total_tokens += usage.total_tokens as u64;
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
        read_stream_to_string(reader, parse_stream_line)
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
    let content = chunk.choices.first().and_then(|c| c.delta.content.clone());

    Ok(content)
}

impl LlmClient for OpenAiClient {
    fn validate_model(&self) -> Result<()> {
        let url = self.model_url();
        let resp = self
            .client
            .get(&url)
            .bearer_auth(&self.api_key)
            .send()
            .context("failed to send model validation request to OpenAI")?;

        if resp.status() == StatusCode::OK {
            return Ok(());
        }

        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        Err(anyhow!(
            "OpenAI model validation failed for {:?} at {}: HTTP {} - {}",
            self.model,
            url,
            status.as_u16(),
            text
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

        log::debug!(
            "Per-file summarize prompt for {} ({:?}) [truncated]:\n{}",
            file.path,
            file.category,
            truncate(&prompts.user, 1000)
        );
        log::trace!(
            "Per-file summarize prompt for {} ({:?}) [full]:\n--- SYSTEM ---\n{}\n--- USER ---\n{}",
            file.path,
            file.category,
            prompts.system,
            prompts.user
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

        log::info!(
            "Final commit-message prompt [truncated]:\n{}",
            truncate(&prompts.user, 1000)
        );
        log::debug!(
            "Final commit-message prompt [full]:\n--- SYSTEM ---\n{}\n--- USER ---\n{}",
            prompts.system,
            prompts.user
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

        log::info!(
            "PR description prompt [truncated]:\n{}",
            truncate(&prompts.user, 1000)
        );
        log::debug!(
            "PR description prompt [full]:\n--- SYSTEM ---\n{}\n--- USER ---\n{}",
            prompts.system,
            prompts.user
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
        // Final PR message generation — report aggregated usage once.
        self.log_and_reset_usage();
        Ok(content)
    }
}

impl OpenAiClient {
    fn log_and_reset_usage(&self) {
        let mut u = self.usage.lock().expect("usage mutex poisoned");
        if u.total_tokens > 0 {
            println!();
            log::warn!(
                "Token usage: prompt={}, completion={}, total={}",
                u.prompt_tokens,
                u.completion_tokens,
                u.total_tokens
            );
            *u = TokenUsage::default();
        }
    }
}

/// Truncate long strings for debug logging.
fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!(
            "{}...\n[truncated {} chars]",
            &s[..max_len],
            s.len() - max_len
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_model_url_from_root_base() {
        let client = OpenAiClient::new(
            "test-key".into(),
            "gpt-5-nano".into(),
            "https://api.openai.com".into(),
            false,
        );

        assert_eq!(
            client.model_url(),
            "https://api.openai.com/v1/models/gpt-5-nano"
        );
    }

    #[test]
    fn builds_model_url_from_v1_base() {
        let client = OpenAiClient::new(
            "test-key".into(),
            "gpt-5-nano".into(),
            "https://api.openai.com/v1".into(),
            false,
        );

        assert_eq!(
            client.model_url(),
            "https://api.openai.com/v1/models/gpt-5-nano"
        );
    }
}
