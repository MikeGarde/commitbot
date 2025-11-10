use super::LlmClient;
use crate::{FileChange};
use crate::git::{PrItem, PrSummaryMode};
use anyhow::{anyhow, Context, Result};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::Duration;
use log::{debug, info, trace, warn};

/// Minimal request/response structs for OpenAI Chat Completions API.
#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
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

/// OpenAI-based implementation of LlmClient.
pub struct OpenAiClient {
    client: Client,
    api_key: String,
    model: String,
}

impl OpenAiClient {
    pub fn new(api_key: String, model: String) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(90))
            .build()
            .expect("failed to build HTTP client");

        OpenAiClient {
            client,
            api_key,
            model,
        }
    }

    fn call_chat(&self, req: &ChatRequest) -> Result<(String, Option<ChatUsage>)> {
        let url = "https://api.openai.com/v1/chat/completions";

        info!("Calling OpenAI model {:?}", &req.model);

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
            warn!("Token usage: prompt={}, completion={}, total={}",
                usage.prompt_tokens, usage.completion_tokens, usage.total_tokens
            );
        }

        Ok((content, chat_resp.usage))
    }
}

impl LlmClient for OpenAiClient {
    fn summarize_file(
        &self,
        branch: &str,
        file: &FileChange,
        ticket_summary: Option<&str>,
    ) -> Result<String> {
        let mut system_instructions = String::from(
            "You are a helpful assistant that explains code changes file-by-file \
             to later help generate a Git commit message.\n\
             Focus on intent, not line-by-line diffs.\n\
             Keep the summary to an appropriate number of bullet points that is consistent with the size of the commit.\n\
             You are unaware of any other files being changed; only consider this one, you will later be informed of the other files,\n\
             at that point you can determine if a change is preparatory or is supporting another change.",
        );

        if let Some(ts) = ticket_summary {
            system_instructions.push_str("\nOverall ticket goal: ");
            system_instructions.push_str(ts);
        }

        let user_prompt = format!(
            "Branch: {branch}\n\
             File: {path}\n\
             Category: {category}\n\n\
             Diff:\n\
             ```diff\n{diff}\n```",
            branch = branch,
            path = file.path,
            category = file.category.as_str(),
            diff = file.diff
        );

        debug!(
            "Per-file summarize prompt for {} ({:?}):\n{}",
            file.path,
            file.category,
            truncate(&user_prompt, 2000)
        );

        let req = ChatRequest {
            model: self.model.clone(),
            messages: vec![
                ChatMessage {
                    role: "system".into(),
                    content: system_instructions,
                },
                ChatMessage {
                    role: "user".into(),
                    content: user_prompt,
                },
            ],
        };

        let (content, _usage) = self.call_chat(&req)?;
        Ok(content)
    }

    fn generate_commit_message(
        &self,
        branch: &str,
        files: &[FileChange],
        ticket_summary: Option<&str>,
    ) -> Result<String> {
        let mut per_file_block = String::new();
        for file in files.iter().filter(|f| !matches!(f.category, crate::FileCategory::Ignored)) {
            per_file_block.push_str(&format!(
                "File: {path}\nCategory: {category}\nSummary:\n{summary}\n\n",
                path = file.path,
                category = file.category.as_str(),
                summary = file
                    .summary
                    .as_deref()
                    .unwrap_or("[missing per-file summary]")
            ));
        }

        let mut system_instructions = String::from(
            "You are a Git commit message assistant.\n\
             Write a descriptive Git commit message based on the file summaries.\n\
             Rules:\n\
             1. Start with a summary line under 50 characters, no formatting.\n\
             2. Follow with a detailed breakdown grouped by type of change.\n\
             3. Use headlines (## Migrations, ## Factories, ## Models, etc.).\n\
             4. Use bullet points under each group.\n\
             5. If something is new, call it 'Introduced', not 'Refactored'.\n\
             6. If it fixes broken or incomplete behavior, prefer 'Fixed' or 'Refined'.\n\
             7. Do not call something a refactor if it is being introduced.\n\
             8. Avoid generic terms like 'update' or 'improve' unless strictly accurate.\n\
             9. Group repetitive changes (like renames) instead of repeating them per file.\n\
             10. Focus on the main purpose and supporting work; only briefly mention consequences.",
        );

        if let Some(ts) = ticket_summary {
            system_instructions.push_str("\nOverall ticket goal: ");
            system_instructions.push_str(ts);
        }

        let user_prompt = format!(
            "Branch: {branch}\n\nPer-file summaries:\n\n{per_file}",
            branch = branch,
            per_file = per_file_block
        );

        debug!(
            "Final commit-message prompt:\n{}",
            truncate(&user_prompt, 3000)
        );

        let req = ChatRequest {
            model: self.model.clone(),
            messages: vec![
                ChatMessage {
                    role: "system".into(),
                    content: system_instructions,
                },
                ChatMessage {
                    role: "user".into(),
                    content: user_prompt,
                },
            ],
        };

        let (content, _usage) = self.call_chat(&req)?;
        Ok(content)
    }

    fn generate_commit_message_simple(
        &self,
        branch: &str,
        diff: &str,
    ) -> Result<String> {
        let system_prompt = String::from(
            "You are a Git commit message assistant.\n\
             Write a descriptive Git commit message for the given diff.\n\
             Follow these rules:\n\
             1. Start with a summary line under 50 characters, no formatting.\n\
             2. Follow with a detailed breakdown grouped by type of change.\n\
             3. Use headlines (## Migrations, ## Factories, ## Models, etc.).\n\
             4. Use bullet points under each group.\n\
             5. If something is new, call it 'Introduced', not 'Refactored'.\n\
             6. If it fixes broken or incomplete behavior, prefer 'Fixed' or 'Refined'.\n\
             7. Do not call something a refactor if it is being introduced.\n\
             8. Avoid generic terms like 'update' or 'improve' unless strictly accurate.\n\
             9. Group repetitive changes (like renames) instead of repeating them per file.\n\
             10. Infer intent where possible from names and context.",
        );

        let user_prompt = format!(
            "Branch: {branch}\n\nDiff:\n```diff\n{diff}\n```",
            branch = branch,
            diff = diff
        );

        trace!(
            "Simple commit-message prompt:\n{}",
            truncate(&user_prompt, 3000)
        );

        let req = ChatRequest {
            model: self.model.clone(),
            messages: vec![
                ChatMessage {
                    role: "system".into(),
                    content: system_prompt,
                },
                ChatMessage {
                    role: "user".into(),
                    content: user_prompt,
                },
            ],
        };

        let (content, _usage) = self.call_chat(&req)?;
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
        let mut system_instructions = String::from(
            "You are a GitHub Pull Request description assistant.\n\
             Your job is to summarize the *overall goal* of the branch and the important changes.\n\
             Rules:\n\
             1. Start with a concise PR title (<= 72 characters, no formatting).\n\
             2. Then include sections, for example:\n\
                - ## Overview\n\
                - ## Changes\n\
                - ## Testing / Validation\n\
                - ## Notes / Risks\n\
             3. Focus on user-visible behavior and domain-level intent, not line-by-line diffs.\n\
             4. De-emphasize purely mechanical changes (formatting-only, CI-only, or style-only).\n\
             5. If PR numbers are provided, reference them in the summary (e.g. 'PR #123').\n\
             6. When multiple PRs contributed, explain how they fit together into a single story.\n\
             7. Avoid generic phrases like 'misc changes' or 'small fixes'; be specific.\n\
             8. In contradiction to point 7, if there are many small changes that don't merit \n\
                individual mention it's okay to summarize them briefly and together.",
        );

        if let Some(ts) = ticket_summary {
            system_instructions.push_str("\nOverall ticket goal: ");
            system_instructions.push_str(ts);
        }

        let mut user_prompt = String::new();
        user_prompt.push_str(&format!(
            "Base branch: {base}\nFeature branch: {from}\nSummary mode: {mode}\n\n",
            base = base_branch,
            from = from_branch,
            mode = mode.as_str()
        ));

        match mode {
            PrSummaryMode::ByCommits => {
                user_prompt.push_str("Commit history (oldest first):\n");
                for item in items {
                    let short = item.commit_hash.chars().take(7).collect::<String>();
                    let pr_tag = item
                        .pr_number
                        .map(|n| format!(" (PR #{n})"))
                        .unwrap_or_default();
                    user_prompt.push_str(&format!(
                        "- {short}{pr_tag}: {title}\n",
                        title = item.title.trim()
                    ));
                    if !item.body.trim().is_empty() {
                        user_prompt.push_str("  Body:\n");
                        user_prompt.push_str("  ");
                        user_prompt.push_str(&item.body.replace('\n', "\n  "));
                        user_prompt.push('\n');
                    }
                }
            }
            PrSummaryMode::ByPrs => {
                let mut grouped: BTreeMap<u32, Vec<&PrItem>> = BTreeMap::new();
                let mut no_pr: Vec<&PrItem> = Vec::new();

                for item in items {
                    if let Some(num) = item.pr_number {
                        grouped.entry(num).or_default().push(item);
                    } else {
                        no_pr.push(item);
                    }
                }

                user_prompt.push_str(
                    "Pull requests contributing to this branch (oldest commits first):\n",
                );

                for (num, group) in grouped {
                    let short = group[0]
                        .commit_hash
                        .chars()
                        .take(7)
                        .collect::<String>();
                    let title = group[0].title.trim();
                    user_prompt.push_str(&format!("\nPR #{num}: {title} [{short}]\n"));

                    if group.len() > 1 {
                        user_prompt.push_str("Additional commits in this PR:\n");
                        for item in group.iter().skip(1) {
                            let sh = item
                                .commit_hash
                                .chars()
                                .take(7)
                                .collect::<String>();
                            user_prompt.push_str(&format!(
                                "- {sh}: {title}\n",
                                title = item.title.trim()
                            ));
                        }
                    }
                }

                if !no_pr.is_empty() {
                    user_prompt.push_str(
                        "\nCommits without associated PR numbers (may be small fixes or direct pushes):\n",
                    );
                    for item in no_pr {
                        let short = item
                            .commit_hash
                            .chars()
                            .take(7)
                            .collect::<String>();
                        user_prompt.push_str(&format!(
                            "- {short}: {title}\n",
                            title = item.title.trim()
                        ));
                    }
                }
            }
        }

        trace!(
            "PR description prompt:\n{}",
            truncate(&user_prompt, 3500)
        );

        let req = ChatRequest {
            model: self.model.clone(),
            messages: vec![
                ChatMessage {
                    role: "system".into(),
                    content: system_instructions,
                },
                ChatMessage {
                    role: "user".into(),
                    content: user_prompt,
                },
            ],
        };

        let (content, _usage) = self.call_chat(&req)?;
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
