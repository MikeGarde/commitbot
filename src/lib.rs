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

/// Truncates an oversized diff before it's ever sent to an LLM.
///
/// Some diffs (Jupyter notebooks with embedded base64 image outputs, minified
/// bundles, generated files) can run into the megabytes, which blows past any
/// local model's context window and can wedge the upstream server for every
/// other in-flight request. Cutting on a line boundary keeps the remaining
/// diff readable instead of ending mid-line.
pub fn truncate_diff(diff: &str, max_bytes: usize) -> String {
    if diff.len() <= max_bytes {
        return diff.to_string();
    }

    let mut cut = max_bytes;
    while cut > 0 && !diff.is_char_boundary(cut) {
        cut -= 1;
    }

    let head = match diff[..cut].rfind('\n') {
        Some(idx) => &diff[..idx],
        None => &diff[..cut],
    };

    let omitted = diff.len() - head.len();
    format!(
        "{head}\n... [diff truncated, {omitted} bytes omitted — file is unusually large, \
         likely generated or contains embedded binary/base64 content]"
    )
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_diff_leaves_small_diffs_untouched() {
        let diff = "diff --git a/f b/f\n+hello\n";
        assert_eq!(truncate_diff(diff, 1_000), diff);
    }

    #[test]
    fn truncate_diff_cuts_oversized_diffs_on_a_line_boundary() {
        let diff = "line one\nline two\nline three\n";
        let result = truncate_diff(diff, 15);

        assert!(result.starts_with("line one\n"));
        assert!(!result.contains("line three"));
        assert!(result.contains("truncated"));
    }
}
