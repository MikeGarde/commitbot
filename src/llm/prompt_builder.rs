use std::collections::BTreeMap;

use crate::llm::prompts;
use crate::{FileCategory, FileChange};
use crate::git::{PrItem, PrSummaryMode};

pub struct PromptPair {
    pub system: String,
    pub user: String,
}

pub fn file_summary_prompt(
    branch: &str,
    file: &FileChange,
    ticket_summary: Option<&str>,
) -> PromptPair {
    let mut system = prompts::FILE_SUMMARY.to_owned();
    if let Some(ts) = ticket_summary {
        system.push_str("\nOverall ticket goal: ");
        system.push_str(ts);
    }

    let user = format!(
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

    PromptPair { system, user }
}

pub fn commit_message_prompt(
    branch: &str,
    files: &[FileChange],
    ticket_summary: Option<&str>,
) -> PromptPair {
    let mut system = prompts::SYSTEM_INSTRUCTIONS.to_owned();
    if let Some(ts) = ticket_summary {
        system.push_str("\nOverall ticket goal: ");
        system.push_str(ts);
    }

    let per_file = render_per_file_summaries(files);
    let user = format!(
        "Branch: {branch}\n\nPer-file summaries:\n\n{per_file}",
        branch = branch,
        per_file = per_file
    );

    PromptPair { system, user }
}

pub fn commit_message_simple_prompt(
    branch: &str,
    diff: &str,
    ticket_summary: Option<&str>,
) -> PromptPair {
    let mut system = prompts::SYSTEM_INSTRUCTIONS.to_owned();
    if let Some(ts) = ticket_summary {
        system.push_str("\nOverall ticket goal: ");
        system.push_str(ts);
    }

    let user = format!(
        "Branch: {branch}\n\nDiff:\n```diff\n{diff}\n```",
        branch = branch,
        diff = diff
    );

    PromptPair { system, user }
}

pub fn pr_message_prompt(
    base_branch: &str,
    from_branch: &str,
    mode: PrSummaryMode,
    items: &[PrItem],
    ticket_summary: Option<&str>,
) -> PromptPair {
    let mut system = prompts::PR_INSTRUCTIONS.to_owned();
    if let Some(ts) = ticket_summary {
        system.push_str("\nOverall ticket goal: ");
        system.push_str(ts);
    }

    let mut user = String::new();
    user.push_str(&format!(
        "Base branch: {base}\nFeature branch: {from}\nSummary mode: {mode}\n\n",
        base = base_branch,
        from = from_branch,
        mode = mode.as_str()
    ));

    match mode {
        PrSummaryMode::ByCommits => {
            user.push_str("Commit history (oldest first):\n");
            for item in items {
                let short = item.commit_hash.chars().take(7).collect::<String>();
                let pr_tag = item
                    .pr_number
                    .map(|n| format!(" (PR #{n})"))
                    .unwrap_or_default();
                user.push_str(&format!(
                    "- {short}{pr_tag}: {title}\n",
                    title = item.title.trim()
                ));
                if !item.body.trim().is_empty() {
                    user.push_str("  Body:\n");
                    user.push_str("  ");
                    user.push_str(&item.body.replace('\n', "\n  "));
                    user.push('\n');
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

            user.push_str(
                "Pull requests contributing to this branch (oldest commits first):\n",
            );

            for (num, group) in grouped {
                let short = group[0]
                    .commit_hash
                    .chars()
                    .take(7)
                    .collect::<String>();
                let title = group[0].title.trim();
                user.push_str(&format!("\nPR #{num}: {title} [{short}]\n"));

                if group.len() > 1 {
                    user.push_str("Additional commits in this PR:\n");
                    for item in group.iter().skip(1) {
                        let sh = item
                            .commit_hash
                            .chars()
                            .take(7)
                            .collect::<String>();
                        user.push_str(&format!(
                            "- {sh}: {title}\n",
                            title = item.title.trim()
                        ));
                    }
                }
            }

            if !no_pr.is_empty() {
                user.push_str(
                    "\nCommits without associated PR numbers (may be small fixes or direct pushes):\n",
                );
                for item in no_pr {
                    let short = item
                        .commit_hash
                        .chars()
                        .take(7)
                        .collect::<String>();
                    user.push_str(&format!(
                        "- {short}: {title}\n",
                        title = item.title.trim()
                    ));
                }
            }
        }
    }

    PromptPair { system, user }
}

fn render_per_file_summaries(files: &[FileChange]) -> String {
    let mut out = String::new();
    for file in files.iter().filter(|f| !matches!(f.category, FileCategory::Ignored)) {
        out.push_str(&format!(
            "File: {path}\nCategory: {category}\nSummary:\n{summary}\n\n",
            path = file.path,
            category = file.category.as_str(),
            summary = file
                .summary
                .as_deref()
                .unwrap_or("[missing per-file summary]")
        ));
    }
    out
}
