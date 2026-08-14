use super::LlmClient;
use super::prompt_builder;
use super::stream::read_stream_to_string;
use crate::FileChange;
use crate::git::{PrItem, PrSummaryMode};
use anyhow::{Context, Result, anyhow};
use reqwest::StatusCode;
use reqwest::blocking::{Client, RequestBuilder};
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
struct ModelListResponse {
    data: Vec<ModelListEntry>,
}

#[derive(Deserialize)]
struct ModelListEntry {
    id: String,
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

/// How to confirm the configured model exists on the upstream server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelValidation {
    /// `GET {base}/v1/models/{model}` — OpenAI's retrieve-model endpoint.
    Retrieve,
    /// `GET {base}/v1/models`, then look for the id in the listing. LM Studio
    /// only implements the list endpoint, not retrieve.
    List,
}

/// Client for OpenAI and any server speaking the same chat-completions API.
pub struct OpenAiClient {
    client: Client,
    api_key: Option<String>,
    model: String,
    api_base_url: String,
    stream: bool,
    /// Name used in logs and error messages (e.g. "OpenAI", "LM Studio").
    provider_label: String,
    model_validation: ModelValidation,
    usage: Mutex<TokenUsage>,
}

#[derive(Default)]
struct TokenUsage {
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
}

impl OpenAiClient {
    pub fn new(api_key: String, model: String, api_base_url: String, stream: bool, timeout_secs: u64) -> Self {
        Self::openai_compatible(
            Some(api_key),
            model,
            api_base_url,
            stream,
            timeout_secs,
            "OpenAI",
            ModelValidation::Retrieve,
        )
    }

    /// Client for an OpenAI-compatible server (LM Studio, self-hosted gateways).
    ///
    /// `api_key` is optional: LM Studio serves unauthenticated requests by
    /// default, and omitting the key sends no `Authorization` header at all.
    pub fn openai_compatible(
        api_key: Option<String>,
        model: String,
        api_base_url: String,
        stream: bool,
        timeout_secs: u64,
        provider_label: impl Into<String>,
        model_validation: ModelValidation,
    ) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .build()
            .expect("failed to build HTTP client");

        OpenAiClient {
            client,
            api_key,
            model,
            api_base_url: api_base_url.trim_end_matches('/').to_string(),
            stream,
            provider_label: provider_label.into(),
            model_validation,
            usage: Mutex::new(TokenUsage::default()),
        }
    }

    /// Attach bearer auth when a key is configured, otherwise send the request
    /// unauthenticated.
    fn authed(&self, req: RequestBuilder) -> RequestBuilder {
        match &self.api_key {
            Some(key) => req.bearer_auth(key),
            None => req,
        }
    }

    /// Join `path` under `/v1`, tolerating a base URL that already ends in `/v1`.
    fn v1_url(&self, path: &str) -> String {
        if self.api_base_url.ends_with("/v1") {
            format!("{}/{}", self.api_base_url, path)
        } else {
            format!("{}/v1/{}", self.api_base_url, path)
        }
    }

    fn chat_url(&self) -> String {
        self.v1_url("chat/completions")
    }

    fn model_url(&self) -> String {
        self.v1_url(&format!("models/{}", self.model))
    }

    fn models_url(&self) -> String {
        self.v1_url("models")
    }

    fn call_chat(&self, req: &ChatRequest) -> Result<String> {
        if req.stream {
            return self.call_chat_streaming(req);
        }

        let url = self.chat_url();

        log::info!("Calling {} model {:?}", self.provider_label, req.model);

        let t0 = std::time::Instant::now();
        let resp = self
            .authed(self.client.post(&url))
            .json(req)
            .send()
            .with_context(|| format!("failed to send request to {} at {}", self.provider_label, url))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().unwrap_or_default();
            return Err(anyhow!(
                "{} API error: HTTP {} - {}",
                self.provider_label,
                status.as_u16(),
                text
            ));
        }

        let chat_resp: ChatResponse = resp
            .json()
            .with_context(|| format!("failed to parse {} response", self.provider_label))?;
        log::debug!("{} response time: {:.2?}", self.provider_label, t0.elapsed());
        let content = chat_resp
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .ok_or_else(|| anyhow!("no choices returned from {}", self.provider_label))?;

        if let Some(usage) = &chat_resp.usage {
            // Recover from a poisoned mutex instead of panicking so the CLI
            // can continue in the face of concurrent thread panics.
            let mut u = self.usage.lock().unwrap_or_else(|e| e.into_inner());
            u.prompt_tokens += usage.prompt_tokens as u64;
            u.completion_tokens += usage.completion_tokens as u64;
            u.total_tokens += usage.total_tokens as u64;
        }

        Ok(content)
    }

    fn call_chat_streaming(&self, req: &ChatRequest) -> Result<String> {
        let url = self.chat_url();

        log::info!("Streaming {} model {:?}", self.provider_label, req.model);

        let t0 = std::time::Instant::now();
        let resp = self
            .authed(self.client.post(&url))
            .json(req)
            .send()
            .with_context(|| {
                format!(
                    "failed to send streaming request to {} at {}",
                    self.provider_label, url
                )
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().unwrap_or_default();
            return Err(anyhow!(
                "{} API error: HTTP {} - {}",
                self.provider_label,
                status.as_u16(),
                text
            ));
        }

        let reader = BufReader::new(resp);
        let result = read_stream_to_string(reader, parse_stream_line);
        log::debug!(
            "{} streaming response time: {:.2?}",
            self.provider_label,
            t0.elapsed()
        );
        result
    }

    /// `GET /v1/models/{model}` — OpenAI's retrieve-model endpoint.
    fn validate_model_by_retrieve(&self) -> Result<()> {
        let url = self.model_url();
        let resp = self
            .authed(self.client.get(&url))
            .send()
            .with_context(|| {
                format!(
                    "failed to send model validation request to {} at {}",
                    self.provider_label, url
                )
            })?;

        if resp.status() == StatusCode::OK {
            return Ok(());
        }

        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        Err(anyhow!(
            "{} model validation failed for {:?} at {}: HTTP {} - {}",
            self.provider_label,
            self.model,
            url,
            status.as_u16(),
            text
        ))
    }

    /// `GET /v1/models` plus a membership check, for servers that list models
    /// but do not implement retrieve-by-id.
    fn validate_model_by_list(&self) -> Result<()> {
        let url = self.models_url();
        let resp = self
            .authed(self.client.get(&url))
            .send()
            .with_context(|| {
                format!(
                    "failed to reach {} at {} — is the server running and reachable?",
                    self.provider_label, url
                )
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().unwrap_or_default();
            return Err(anyhow!(
                "{} model listing failed at {}: HTTP {} - {}",
                self.provider_label,
                url,
                status.as_u16(),
                text
            ));
        }

        let listed: ModelListResponse = resp
            .json()
            .with_context(|| format!("failed to parse model list from {url}"))?;

        if listed.data.iter().any(|m| m.id == self.model) {
            return Ok(());
        }

        let available = listed
            .data
            .iter()
            .map(|m| m.id.as_str())
            .collect::<Vec<_>>()
            .join(", ");

        Err(anyhow!(
            "{} at {} has no model {:?}. Available: {}",
            self.provider_label,
            url,
            self.model,
            if available.is_empty() {
                "(none)"
            } else {
                &available
            }
        ))
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
        match self.model_validation {
            ModelValidation::Retrieve => self.validate_model_by_retrieve(),
            ModelValidation::List => self.validate_model_by_list(),
        }
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
            90,
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
            90,
        );

        assert_eq!(
            client.model_url(),
            "https://api.openai.com/v1/models/gpt-5-nano"
        );
    }

    fn lm_studio(base_url: &str) -> OpenAiClient {
        OpenAiClient::openai_compatible(
            None,
            "qwen/qwen3-coder-30b".into(),
            base_url.into(),
            false,
            90,
            "LM Studio",
            ModelValidation::List,
        )
    }

    #[test]
    fn builds_lm_studio_urls_from_v1_base() {
        let client = lm_studio("http://localhost:1234/v1");

        assert_eq!(client.models_url(), "http://localhost:1234/v1/models");
        assert_eq!(
            client.chat_url(),
            "http://localhost:1234/v1/chat/completions"
        );
    }

    #[test]
    fn builds_lm_studio_urls_from_root_base() {
        // Given without the /v1 suffix, and with a trailing slash, still lands
        // on the same paths.
        let client = lm_studio("http://localhost:1234/");

        assert_eq!(client.models_url(), "http://localhost:1234/v1/models");
        assert_eq!(
            client.chat_url(),
            "http://localhost:1234/v1/chat/completions"
        );
    }

    #[test]
    fn lm_studio_validates_against_the_model_list() {
        // LM Studio implements GET /v1/models but not GET /v1/models/{id},
        // so validation has to go through the listing.
        assert_eq!(
            lm_studio("http://localhost:1234/v1").model_validation,
            ModelValidation::List
        );
        assert_eq!(
            OpenAiClient::new("k".into(), "m".into(), "https://api.openai.com".into(), false, 90)
                .model_validation,
            ModelValidation::Retrieve
        );
    }

    #[test]
    fn decodes_model_list_payload() {
        let body = r#"{"object":"list","data":[
            {"id":"qwen/qwen3-coder-30b","object":"model","owned_by":"organization_owner"},
            {"id":"text-embedding-nomic-embed-text-v1.5","object":"model","owned_by":"organization_owner"}
        ]}"#;

        let parsed: ModelListResponse = serde_json::from_str(body).expect("valid model list");
        let ids: Vec<&str> = parsed.data.iter().map(|m| m.id.as_str()).collect();

        assert_eq!(
            ids,
            ["qwen/qwen3-coder-30b", "text-embedding-nomic-embed-text-v1.5"]
        );
    }
}
