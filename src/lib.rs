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

/// Lock files whose names don't end in `.lock`, so the extension check misses them.
const LOCK_FILE_NAMES: &[&str] = &[
    "package-lock.json",   // npm
    "npm-shrinkwrap.json", // npm
    "pnpm-lock.yaml",      // pnpm
    "bun.lockb",           // bun (pre-1.2 binary format)
    "packages.lock.json",  // NuGet
    "gradle.lockfile",     // Gradle
];

/// True when the path names a dependency lock file: any `*.lock` file, plus the
/// well-known lock files that use a different extension ([`LOCK_FILE_NAMES`]).
///
/// Lock files are regenerated wholesale by a package manager, so their diffs
/// carry no intent worth asking the LLM about. We skip summarizing them and
/// only tell the final summary that they were touched.
pub fn is_lock_file(path: &str) -> bool {
    let path = std::path::Path::new(path);

    if path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("lock"))
    {
        return true;
    }

    path.file_name().is_some_and(|name| {
        LOCK_FILE_NAMES
            .iter()
            .any(|known| name.eq_ignore_ascii_case(known))
    })
}

/// How each file is categorized. The first four come from the user in
/// interactive mode; `Lock` is assigned automatically by [`is_lock_file`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum FileCategory {
    Main,        // 1
    Supporting,  // 2
    Consequence, // 3
    Ignored,     // 4
    Lock,        // auto-assigned to *.lock files
}

impl FileCategory {
    /// Convert the category to a string representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            FileCategory::Main => "main",
            FileCategory::Supporting => "supporting",
            FileCategory::Consequence => "consequence",
            FileCategory::Ignored => "ignored",
            FileCategory::Lock => "lock",
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
    /// LLM-generated summary for this file. Always `None` for
    /// [`FileCategory::Lock`] and [`FileCategory::Ignored`] files.
    pub summary: Option<String>,
}
