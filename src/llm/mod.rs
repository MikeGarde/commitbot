pub mod openai;

use crate::{FileChange, PrItem, PrSummaryMode};
use anyhow::Result;

/// Trait for talking to an LLM (real or dummy).
pub trait LlmClient {
    /// Generate a per-file summary based on diff + metadata.
    fn summarize_file(
        &self,
        branch: &str,
        file: &FileChange,
        ticket_summary: Option<&str>,
        debug: bool,
    ) -> Result<String>;

    /// Generate the final commit message from file summaries + metadata.
    fn generate_commit_message(
        &self,
        branch: &str,
        files: &[FileChange],
        ticket_summary: Option<&str>,
        debug: bool,
    ) -> Result<String>;

    /// Simple mode: commit message from entire diff.
    fn generate_commit_message_simple(
        &self,
        branch: &str,
        diff: &str,
        debug: bool,
    ) -> Result<String>;

    /// PR mode: generate a PR description from commit/PR messages.
    fn generate_pr_message(
        &self,
        base_branch: &str,
        from_branch: &str,
        mode: PrSummaryMode,
        items: &[PrItem],
        ticket_summary: Option<&str>,
        debug: bool,
    ) -> Result<String>;
}

/// No-op / dummy model client for development with --no-model or model=none.
pub struct NoopClient;

impl LlmClient for NoopClient {
    fn summarize_file(
        &self,
        _branch: &str,
        file: &FileChange,
        _ticket_summary: Option<&str>,
        _debug: bool,
    ) -> Result<String> {
        Ok(format!(
            "[DUMMY SUMMARY] {} ({})",
            file.path,
            file.category.as_str()
        ))
    }

    fn generate_commit_message(
        &self,
        branch: &str,
        files: &[FileChange],
        ticket_summary: Option<&str>,
        _debug: bool,
    ) -> Result<String> {
        let mut msg = String::new();
        msg.push_str("Dummy commit message for testing\n\n");
        msg.push_str(&format!("Branch: {branch}\n"));
        if let Some(ts) = ticket_summary {
            msg.push_str(&format!("Ticket: {ts}\n\n"));
        } else {
            msg.push('\n');
        }

        for f in files.iter().filter(|f| !matches!(f.category, crate::FileCategory::Ignored)) {
            msg.push_str(&format!(
                "- {} [{}]: {}\n",
                f.path,
                f.category.as_str(),
                f.summary
                    .as_deref()
                    .unwrap_or("[no summary; dummy client]")
            ));
        }

        Ok(msg)
    }

    fn generate_commit_message_simple(
        &self,
        branch: &str,
        _diff: &str,
        _debug: bool,
    ) -> Result<String> {
        Ok(format!(
            "Dummy simple commit message for branch {branch}\n\n(LLM disabled)"
        ))
    }

    fn generate_pr_message(
        &self,
        base_branch: &str,
        from_branch: &str,
        mode: PrSummaryMode,
        items: &[PrItem],
        ticket_summary: Option<&str>,
        _debug: bool,
    ) -> Result<String> {
        let mut msg = String::new();
        msg.push_str("Dummy PR description for testing\n\n");
        msg.push_str(&format!(
            "Base branch: {base}\nFeature branch: {from}\nMode: {mode}\n\n",
            base = base_branch,
            from = from_branch,
            mode = mode.as_str()
        ));

        if let Some(ts) = ticket_summary {
            msg.push_str(&format!("Ticket summary: {ts}\n\n"));
        }

        for item in items {
            msg.push_str(&format!(
                "- {} {} (PR #{})\n",
                &item.commit_hash.chars().take(7).collect::<String>(),
                item.title.trim(),
                item.pr_number
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "none".into())
            ));
        }

        Ok(msg)
    }
}
