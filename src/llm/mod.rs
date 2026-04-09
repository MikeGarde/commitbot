pub mod ollama;
pub mod openai;
mod prompt_builder;
mod prompts;
mod stream;

use crate::FileChange;
use crate::git::{PrItem, PrSummaryMode};
use anyhow::Result;

/// Trait for talking to an LLM (real backend).
pub trait LlmClient: Send + Sync {
    /// Confirm the configured model is reachable/known by the upstream provider.
    fn validate_model(&self) -> Result<()>;

    /// Generate a per-file summary based on diff + metadata.
    fn summarize_file(
        &self,
        branch: &str,
        file: &FileChange,
        file_index: usize,
        total_files: usize,
        ticket_summary: Option<&str>,
    ) -> Result<String>;

    /// Generate the final commit message from file summaries + metadata.
    fn generate_commit_message(
        &self,
        branch: &str,
        files: &[FileChange],
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

    /// Take aggregated token usage from the client, resetting counters.
    fn take_and_reset_usage(&self) -> Option<(u64, u64, u64)> {
        None
    }
}
