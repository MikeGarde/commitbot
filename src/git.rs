use anyhow::{Context, Result, anyhow};
use std::process::Command as GitCommand;

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

#[derive(Debug, Clone, PartialEq, Eq)]
struct RemoteRepo {
    path: String,
    web_base_url: String,
    provider: GitProvider,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GitProvider {
    GitHub,
    GitLab,
    Bitbucket,
    AzureDevOps,
    Unknown,
}

impl RemoteRepo {
    fn repo_id(&self) -> Option<String> {
        if self.provider == GitProvider::AzureDevOps {
            let segments: Vec<&str> = self.path.split('/').filter(|s| !s.is_empty()).collect();
            if segments.len() >= 4 && segments[2] == "_git" {
                return Some(format!("{}/{}", segments[1], segments[3]));
            }
        }

        let segments: Vec<&str> = self.path.split('/').filter(|s| !s.is_empty()).collect();
        if segments.len() >= 2 {
            let owner = segments[segments.len() - 2];
            let repo = segments[segments.len() - 1];
            Some(format!("{owner}/{repo}"))
        } else {
            None
        }
    }

    fn commit_url(&self, commit_hash: &str) -> Option<String> {
        match self.provider {
            GitProvider::GitHub => Some(format!("{}/commit/{}", self.web_base_url, commit_hash)),
            GitProvider::GitLab => Some(format!("{}/-/commit/{}", self.web_base_url, commit_hash)),
            GitProvider::Bitbucket => {
                Some(format!("{}/commits/{}", self.web_base_url, commit_hash))
            }
            GitProvider::AzureDevOps => {
                Some(format!("{}/commit/{}", self.web_base_url, commit_hash))
            }
            GitProvider::Unknown => None,
        }
    }
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

fn remote_origin_url() -> Option<String> {
    let output = GitCommand::new("git")
        .args(["config", "--get", "remote.origin.url"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    String::from_utf8(output.stdout)
        .ok()
        .map(|url| url.trim().to_string())
        .filter(|url| !url.is_empty())
}

/// Get the current branch name.
pub fn current_branch() -> Result<String> {
    let name = git_output(&["rev-parse", "--abbrev-ref", "HEAD"])?
        .trim()
        .to_string();
    Ok(name)
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
                    value = value.saturating_mul(10).saturating_add((b - b'0') as u32);
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

/// Split a combined diff string into (path, diff) pairs, one per file.
/// Handles both `diff --git` headers and legacy `--- a/` headers.
pub fn split_diff_by_file(diff: &str) -> Vec<(String, String)> {
    let mut results = Vec::new();
    let mut current_path: Option<String> = None;
    let mut current_lines: Vec<&str> = Vec::new();

    for line in diff.lines() {
        if line.starts_with("diff --git ") {
            // Save previous file
            if let Some(path) = current_path.take() {
                results.push((path, current_lines.join("\n")));
            }
            current_lines = vec![line];

            // Extract path from "diff --git a/foo b/foo"
            let path = line
                .split_whitespace()
                .last()
                .and_then(|s| s.strip_prefix("b/"))
                .unwrap_or("")
                .to_string();
            current_path = Some(path);
        } else {
            current_lines.push(line);
        }
    }

    // Flush the last file
    if let Some(path) = current_path.take() {
        results.push((path, current_lines.join("\n")));
    }

    results
}

/// Stage all new, modified, and deleted files
pub fn stage_all() -> Result<()> {
    log::info!("Staging all changes");
    git_output(&["add", "-A"])?;
    Ok(())
}

/// Try to derive a repo identifier like "owner/repo" from `git remote.origin.url`.
pub fn detect_repo_id() -> Option<String> {
    let remote = remote_origin_url()?;
    parse_remote_repo(&remote)?.repo_id()
}

pub fn format_pr_commit_appendix(items: &[PrItem]) -> String {
    if items.is_empty() {
        return String::new();
    }

    let remote = remote_origin_url().and_then(|url| parse_remote_repo(&url));
    format_pr_commit_appendix_with_remote(items, remote.as_ref())
}

fn format_pr_commit_appendix_with_remote(items: &[PrItem], remote: Option<&RemoteRepo>) -> String {
    let mut out = String::from("Commits in this PR:\n");
    for item in items {
        let short = short_commit_hash(&item.commit_hash);
        let title = item.title.trim();

        match remote.and_then(|repo| repo.commit_url(&item.commit_hash)) {
            Some(url) => {
                out.push_str(&format!("- [`{short}`]({url}) {title}\n"));
            }
            None => {
                out.push_str(&format!("- `{short}` {title}\n"));
            }
        }
    }

    out.trim_end().to_string()
}

fn short_commit_hash(hash: &str) -> String {
    hash.chars().take(7).collect()
}

fn parse_remote_repo(url: &str) -> Option<RemoteRepo> {
    let trimmed = url.trim().trim_end_matches(".git");
    if trimmed.is_empty() {
        return None;
    }

    if let Some(rest) = trimmed.strip_prefix("git@ssh.dev.azure.com:v3/") {
        let parts: Vec<&str> = rest.split('/').filter(|s| !s.is_empty()).collect();
        if parts.len() >= 3 {
            let org = parts[0];
            let project = parts[1];
            let repo = parts[2];
            return Some(RemoteRepo {
                path: format!("{org}/{project}/_git/{repo}"),
                web_base_url: format!("https://dev.azure.com/{org}/{project}/_git/{repo}"),
                provider: GitProvider::AzureDevOps,
            });
        }
    }

    if let Some(rest) = trimmed.strip_prefix("ssh://") {
        return parse_remote_repo_from_authority_path(rest);
    }

    if let Some((host_part, path_part)) = trimmed.split_once("://") {
        let _scheme = host_part;
        return parse_remote_repo_from_authority_path(path_part);
    }

    if let Some((user_host, path)) = trimmed.split_once(':') {
        let host = user_host.rsplit('@').next()?.to_string();
        return build_remote_repo(host, path.trim_start_matches('/'));
    }

    None
}

fn parse_remote_repo_from_authority_path(input: &str) -> Option<RemoteRepo> {
    let without_user = input.rsplit('@').next().unwrap_or(input);
    let (host, path) = without_user.split_once('/')?;
    build_remote_repo(host.to_string(), path)
}

fn build_remote_repo(host: String, path: &str) -> Option<RemoteRepo> {
    let clean_path = path.trim_matches('/').to_string();
    if clean_path.is_empty() {
        return None;
    }

    let provider = if host.contains("github") {
        GitProvider::GitHub
    } else if host.contains("gitlab") {
        GitProvider::GitLab
    } else if host.contains("bitbucket") {
        GitProvider::Bitbucket
    } else if host.contains("azure")
        || host == "dev.azure.com"
        || host.ends_with(".visualstudio.com")
    {
        GitProvider::AzureDevOps
    } else {
        GitProvider::Unknown
    };

    let web_base_url = match provider {
        GitProvider::AzureDevOps => normalize_azure_web_base(&host, &clean_path)?,
        _ => format!("https://{host}/{clean_path}"),
    };

    Some(RemoteRepo {
        path: clean_path,
        web_base_url,
        provider,
    })
}

fn normalize_azure_web_base(host: &str, path: &str) -> Option<String> {
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    if host == "dev.azure.com" && segments.len() >= 4 && segments[2] == "_git" {
        return Some(format!(
            "https://dev.azure.com/{}/{}/_git/{}",
            segments[0], segments[1], segments[3]
        ));
    }

    if host.ends_with(".visualstudio.com") {
        let org = host.trim_end_matches(".visualstudio.com");
        if segments.len() >= 3 && segments[1] == "_git" {
            return Some(format!(
                "https://{}.visualstudio.com/{}/_git/{}",
                org, segments[0], segments[2]
            ));
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::{GitProvider, PrItem, parse_remote_repo};

    #[test]
    fn parses_github_ssh_remote() {
        let remote = parse_remote_repo("git@github.com:owner/repo.git").unwrap();
        assert_eq!(remote.provider, GitProvider::GitHub);
        assert_eq!(remote.repo_id().as_deref(), Some("owner/repo"));
        assert_eq!(
            remote.commit_url("abcdef123456").as_deref(),
            Some("https://github.com/owner/repo/commit/abcdef123456")
        );
    }

    #[test]
    fn parses_gitlab_https_remote() {
        let remote =
            parse_remote_repo("https://gitlab.example.com/group/subgroup/repo.git").unwrap();
        assert_eq!(remote.provider, GitProvider::GitLab);
        assert_eq!(remote.repo_id().as_deref(), Some("subgroup/repo"));
        assert_eq!(
            remote.commit_url("abcdef123456").as_deref(),
            Some("https://gitlab.example.com/group/subgroup/repo/-/commit/abcdef123456")
        );
    }

    #[test]
    fn parses_azure_ssh_remote() {
        let remote = parse_remote_repo("git@ssh.dev.azure.com:v3/org/project/repo").unwrap();
        assert_eq!(remote.provider, GitProvider::AzureDevOps);
        assert_eq!(remote.repo_id().as_deref(), Some("project/repo"));
        assert_eq!(
            remote.commit_url("abcdef123456").as_deref(),
            Some("https://dev.azure.com/org/project/_git/repo/commit/abcdef123456")
        );
    }

    #[test]
    fn appendix_falls_back_to_hash_only_when_no_remote() {
        let items = vec![PrItem {
            commit_hash: "abcdef123456".to_string(),
            title: "Refine PR footer rendering".to_string(),
            body: String::new(),
            pr_number: None,
        }];

        let appendix = super::format_pr_commit_appendix_with_remote(&items, None);
        assert!(appendix.contains("Commits in this PR:"));
        assert!(appendix.contains("- `abcdef1` Refine PR footer rendering"));
    }
}
