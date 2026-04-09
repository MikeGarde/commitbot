//! Core types and shared functionality for commitbot.
//!
//! This module contains shared types and functions used across the application.

pub mod cli_args;
pub mod config;
pub mod git;
pub mod llm;
pub mod logging;
pub mod setup;

pub use cli_args::{Cli, Command};
pub use git::{
    collect_pr_items, current_branch, format_pr_commit_appendix, split_diff_by_file, stage_all,
    staged_diff_for_file, staged_files, PrSummaryMode,
};
pub use llm::LlmClient;

/// How the user categorizes each file in interactive mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum FileCategory {
    Main,        // 1
    Supporting,  // 2
    Consequence, // 3
    Ignored,     // 4
}

impl FileCategory {
    /// Convert the category to a string representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            FileCategory::Main => "main",
            FileCategory::Supporting => "supporting",
            FileCategory::Consequence => "consequence",
            FileCategory::Ignored => "ignored",
        }
    }
}

/// Represents a single staged file's change and metadata.
#[derive(Debug, Clone)]
pub struct FileChange {
    /// Path to the file
    pub path: String,
    /// User-defined category for this file
    pub category: FileCategory,
    /// Git diff for this file
    pub diff: String,
    /// LLM-generated summary for this file
    pub summary: Option<String>,
}
