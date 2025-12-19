pub mod openai;
pub mod ollama;
mod prompts;
mod prompt_builder;
mod stream;

use crate::git::{PrItem, PrSummaryMode};
use crate::FileChange;
use anyhow::Result;

/// Trait for talking to an LLM (real backend).
pub trait LlmClient: Send + Sync {
    /// Generate a per-file summary based on diff + metadata.
    fn summarize_file(
        &self,
        branch: &str,
        file: &FileChange,
        ticket_summary: Option<&str>,
    ) -> Result<String>;

    /// Generate the final commit message from file summaries + metadata.
    fn generate_commit_message(
        &self,
        branch: &str,
        files: &[FileChange],
        ticket_summary: Option<&str>,
    ) -> Result<String>;

    /// Simple mode: commit message from entire diff.
    fn generate_commit_message_simple(
        &self,
        branch: &str,
        diff: &str,
        ticket_summary: Option<&str>,
    ) -> Result<String>;

    /// PR mode: generate a PR description from commit/PR messages.
    fn generate_pr_message(
        &self,
        base_branch: &str,
        from_branch: &str,
        mode: PrSummaryMode,
        items: &[PrItem],
        ticket_summary: Option<&str>,
    ) -> Result<String>;
}
