use anyhow::{anyhow, Context, Result};
use std::path::PathBuf;
use std::process::Command as GitCommand;
use std::fs;

/// How we want to summarize a PR.
#[derive(Debug, Clone, Copy)]
pub enum PrSummaryMode {
    ByCommits,
    ByPrs,
}

impl PrSummaryMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            PrSummaryMode::ByCommits => "commits",
            PrSummaryMode::ByPrs => "prs",
        }
    }
}

/// A commit involved in the PR range, plus any detected PR number.
#[derive(Debug, Clone)]
pub struct PrItem {
    pub commit_hash: String,
    pub title: String,
    pub body: String,
    pub pr_number: Option<u32>,
}

/// Run a git command and capture stdout as String.
pub fn git_output(args: &[&str]) -> Result<String> {
    let output = GitCommand::new("git")
        .args(args)
        .output()
        .with_context(|| format!("failed to run git {:?}", args))?;

    if !output.status.success() {
        return Err(anyhow!(
            "git {:?} exited with status {:?}",
            args,
            output.status.code()
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Get the path to the Git directory (e.g. .git)
pub fn git_dir() -> Result<PathBuf> {
    let output = GitCommand::new("git")
        .args(["rev-parse", "--git-dir"])
        .output()
        .context("failed to run git rev-parse --git-dir")?;

    if !output.status.success() {
        return Err(anyhow!(
            "git rev-parse --git-dir exited with status {:?}",
            output.status.code()
        ));
    }

    let dir = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(PathBuf::from(dir))
}

/// Write the commit message into .git/COMMIT_EDITMSG so the next `git commit`
/// will use it as the default message in the editor.
pub fn write_commit_editmsg(message: &str) -> Result<()> {
    let dir = git_dir()?;
    let path = dir.join("COMMIT_EDITMSG");
    fs::write(&path, message)
        .with_context(|| format!("failed to write commit message to {:?}", path))?;
    Ok(())
}

/// Get the current branch name.
pub fn current_branch() -> Result<String> {
    let name = git_output(&["rev-parse", "--abbrev-ref", "HEAD"])?
        .trim()
        .to_string();
    Ok(name)
}

/// Get the full staged diff.
pub fn staged_diff() -> Result<String> {
    let diff = git_output(&["diff", "--cached"])?;
    Ok(diff)
}

/// Get a list of staged files.
pub fn staged_files() -> Result<Vec<String>> {
    let output = git_output(&["diff", "--cached", "--name-only"])?;
    let files = output
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    Ok(files)
}

/// Get per-file staged diff.
pub fn staged_diff_for_file(path: &str) -> Result<String> {
    let diff = git_output(&["diff", "--cached", "--", path])?;
    Ok(diff)
}

/// Find the first PR number in a string, based on '#123' pattern.
fn find_first_pr_number(text: &str) -> Option<u32> {
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        if bytes[i] == b'#' {
            let mut j = i + 1;
            let mut value: u32 = 0;
            let mut found_digit = false;

            while j < len {
                let b = bytes[j];
                if b.is_ascii_digit() {
                    found_digit = true;
                    value = value
                        .saturating_mul(10)
                        .saturating_add((b - b'0') as u32);
                    j += 1;
                } else {
                    break;
                }
            }

            if found_digit {
                return Some(value);
            }
        }
        i += 1;
    }

    None
}

/// Collect commits between base..from as PrItem list.
pub fn collect_pr_items(base: &str, from: &str) -> Result<Vec<PrItem>> {
    let range = format!("{base}..{from}");
    let log_output = git_output(&[
        "log",
        "--reverse",
        "--pretty=format:%H%n%s%n%b%n---END---",
        &range,
    ])?;

    if log_output.trim().is_empty() {
        return Ok(vec![]);
    }

    let mut items = Vec::new();

    for block in log_output.split("\n---END---") {
        let block = block.trim();
        if block.is_empty() {
            continue;
        }

        let mut lines = block.lines();
        let hash = match lines.next() {
            Some(h) => h.trim().to_string(),
            None => continue,
        };
        let title = lines.next().unwrap_or("").trim().to_string();
        let body = lines.collect::<Vec<_>>().join("\n");

        let mut pr_number = find_first_pr_number(&title);
        if pr_number.is_none() {
            pr_number = find_first_pr_number(&body);
        }

        items.push(PrItem {
            commit_hash: hash,
            title,
            body,
            pr_number,
        });
    }

    Ok(items)
}

/// Stage all new, modified, and deleted files
pub fn stage_all() -> Result<()> {
    log::warn!("Staging all changes");
    git_output(&["add", "-A"])?;
    Ok(())
}

/// Try to derive a repo identifier like "owner/repo" from `git remote.origin.url`.
pub fn detect_repo_id() -> Option<String> {
    use std::process::Command;

    let output = Command::new("git")
        .args(["config", "--get", "remote.origin.url"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let url = String::from_utf8(output.stdout).ok()?;
    let trimmed = url.trim().trim_end_matches(".git");

    // For SSH: git@github.com:owner/repo
    // For HTTPS: https://github.com/owner/repo
    let path = if let Some(idx) = trimmed.find("://") {
        // Strip scheme and host, keep "owner/repo"
        let rest = &trimmed[idx + 3..];
        match rest.find('/') {
            Some(slash) => &rest[slash + 1..],
            None => rest,
        }
    } else if let Some(idx) = trimmed.find(':') {
        // SSH-style: after ':' is "owner/repo"
        &trimmed[idx + 1..]
    } else {
        trimmed
    };

    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if segments.len() >= 2 {
        let owner = segments[segments.len() - 2];
        let repo = segments[segments.len() - 1];
        Some(format!("{}/{}", owner, repo))
    } else {
        None
    }
}