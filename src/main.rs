use anyhow::{anyhow, Context, Result};
use clap::{ArgGroup, Parser, Subcommand};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::env;
use std::io::{self, Write};
use std::process::Command as GitCommand;

/// How the user categorizes each file in interactive mode.
#[derive(Debug, Clone, Copy, Serialize)]
enum FileCategory {
    Main,        // 1
    Supporting,  // 2
    Consequence, // 3
    Ignored,     // 4
}

impl FileCategory {
    fn from_choice(choice: &str) -> Option<Self> {
        match choice.trim() {
            "1" => Some(FileCategory::Main),
            "2" => Some(FileCategory::Supporting),
            "3" => Some(FileCategory::Consequence),
            "4" => Some(FileCategory::Ignored),
            _ => None,
        }
    }

    fn as_str(&self) -> &'static str {
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
struct FileChange {
    path: String,
    category: FileCategory,
    diff: String,
    summary: Option<String>, // Filled by per-file model call (or dummy)
}

/// How we want to summarize a PR.
#[derive(Debug, Clone, Copy)]
enum PrSummaryMode {
    ByCommits,
    ByPrs,
}

impl PrSummaryMode {
    fn as_str(&self) -> &'static str {
        match self {
            PrSummaryMode::ByCommits => "commits",
            PrSummaryMode::ByPrs => "prs",
        }
    }
}

/// A commit involved in the PR range, plus any detected PR number.
#[derive(Debug, Clone)]
struct PrItem {
    commit_hash: String,
    title: String,
    body: String,
    pr_number: Option<u32>,
}

/// CLI options
#[derive(Parser, Debug)]
#[command(
    name = "commitbot",
    version,
    about = "LLM-assisted Git commit message generator"
)]
#[command(group(
    ArgGroup::new("model_group")
        .args(["model", "no_model"])
        .multiple(false)
))]
struct Cli {
    /// Interactive mode: classify each file and do per-file summaries
    #[arg(long, global = true)]
    ask: bool,

    /// Debug mode: log prompts, responses, token usage
    #[arg(long, global = true)]
    debug: bool,

    /// Model name to use (e.g. gpt-4o-mini). If 'none', acts like --no-model.
    #[arg(long, global = true)]
    model: Option<String>,

    /// Disable model calls; return dummy responses instead
    #[arg(long, global = true)]
    no_model: bool,

    /// API key (otherwise uses OPENAI_API_KEY env var)
    #[arg(long, env = "OPENAI_API_KEY", global = true)]
    api_key: Option<String>,

    /// Optional: a brief human description of the ticket (for commit/PR summaries)
    #[arg(long, global = true)]
    ticket_summary: Option<String>,

    /// Subcommand (e.g. 'pr')
    #[command(subcommand)]
    command: Option<Command>,
}

/// Subcommands, e.g. `commitbot pr develop`
#[derive(Subcommand, Debug)]
enum Command {
    /// Generate a Pull Request description by summarizing commit or PR messages
    Pr {
        /// Base branch to compare against (e.g. main or develop)
        base: String,

        /// Optional feature/source branch; defaults to current branch if omitted
        from: Option<String>,

        /// Force using PR-oriented grouping (PR numbers) instead of commits
        #[arg(long = "pr")]
        pr_mode: bool,

        /// Force using commit-by-commit mode instead of PR grouping
        #[arg(long = "commit")]
        commit_mode: bool,
    },
}

/// Trait for talking to an LLM (real or dummy).
trait LlmClient {
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
struct NoopClient;

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

        for f in files.iter().filter(|f| !matches!(f.category, FileCategory::Ignored)) {
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
struct OpenAiClient {
    client: Client,
    api_key: String,
    model: String,
}

impl OpenAiClient {
    fn new(api_key: String, model: String) -> Self {
        OpenAiClient {
            client: Client::new(),
            api_key,
            model,
        }
    }

    fn call_chat(&self, req: &ChatRequest, debug: bool) -> Result<(String, Option<ChatUsage>)> {
        let url = "https://api.openai.com/v1/chat/completions";

        if debug {
            eprintln!("[DEBUG] Calling OpenAI model: {}", req.model);
        }

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
            .get(0)
            .map(|c| c.message.content.clone())
            .ok_or_else(|| anyhow!("no choices returned from OpenAI"))?;

        if debug {
            if let Some(usage) = &chat_resp.usage {
                eprintln!(
                    "[DEBUG] Token usage: prompt={}, completion={}, total={}",
                    usage.prompt_tokens, usage.completion_tokens, usage.total_tokens
                );
            }
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
        debug: bool,
    ) -> Result<String> {
        let mut system_instructions = String::from(
            "You are a helpful assistant that explains code changes file-by-file \
             to later help generate a Git commit message.\n\
             Focus on intent, not line-by-line diffs.\n\
             Keep the summary to 2-4 bullet points.",
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

        if debug {
            eprintln!(
                "[DEBUG] Per-file summarize prompt for {} ({:?}):\n{}",
                file.path,
                file.category,
                truncate(&user_prompt, 2000)
            );
        }

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

        let (content, _usage) = self.call_chat(&req, debug)?;
        Ok(content)
    }

    fn generate_commit_message(
        &self,
        branch: &str,
        files: &[FileChange],
        ticket_summary: Option<&str>,
        debug: bool,
    ) -> Result<String> {
        let mut per_file_block = String::new();
        for file in files.iter().filter(|f| !matches!(f.category, FileCategory::Ignored)) {
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

        if debug {
            eprintln!(
                "[DEBUG] Final commit-message prompt:\n{}",
                truncate(&user_prompt, 3000)
            );
        }

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

        let (content, _usage) = self.call_chat(&req, debug)?;
        Ok(content)
    }

    fn generate_commit_message_simple(
        &self,
        branch: &str,
        diff: &str,
        debug: bool,
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

        if debug {
            eprintln!(
                "[DEBUG] Simple commit-message prompt:\n{}",
                truncate(&user_prompt, 3000)
            );
        }

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

        let (content, _usage) = self.call_chat(&req, debug)?;
        Ok(content)
    }

    fn generate_pr_message(
        &self,
        base_branch: &str,
        from_branch: &str,
        mode: PrSummaryMode,
        items: &[PrItem],
        ticket_summary: Option<&str>,
        debug: bool,
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
             7. Avoid generic phrases like 'misc changes' or 'small fixes'; be specific.",
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

        if debug {
            eprintln!(
                "[DEBUG] PR description prompt:\n{}",
                truncate(&user_prompt, 3500)
            );
        }

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

        let (content, _usage) = self.call_chat(&req, debug)?;
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

/// Run a git command and capture stdout as String.
fn git_output(args: &[&str]) -> Result<String> {
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

/// Get the current branch name.
fn current_branch() -> Result<String> {
    let name = git_output(&["rev-parse", "--abbrev-ref", "HEAD"])?
        .trim()
        .to_string();
    Ok(name)
}

/// Get the full staged diff.
fn staged_diff() -> Result<String> {
    let diff = git_output(&["diff", "--cached"])?;
    Ok(diff)
}

/// Get a list of staged files.
fn staged_files() -> Result<Vec<String>> {
    let output = git_output(&["diff", "--cached", "--name-only"])?;
    let files = output
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    Ok(files)
}

/// Get per-file staged diff.
fn staged_diff_for_file(path: &str) -> Result<String> {
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
fn collect_pr_items(base: &str, from: &str) -> Result<Vec<PrItem>> {
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

/// Ask the user a question and return a trimmed input line.
fn prompt_input(prompt: &str) -> Result<String> {
    print!("{prompt}");
    io::stdout().flush()?;

    let mut buf = String::new();
    io::stdin().read_line(&mut buf)?;
    Ok(buf.trim().to_string())
}

/// Interactive mode: classify files, get per-file summaries, then final commit message.
fn run_interactive(cli: &Cli, llm: &dyn LlmClient) -> Result<()> {
    let branch = current_branch()?;
    let files = staged_files()?;
    if files.is_empty() {
        println!("No staged changes found.");
        return Ok(());
    }

    let mut ticket_summary = cli.ticket_summary.clone();
    if ticket_summary.is_none() {
        let ans = prompt_input("Optional: brief ticket summary (enter to skip): ")?;
        if !ans.is_empty() {
            ticket_summary = Some(ans);
        }
    }

    println!("Current branch: {branch}");
    println!("Found {} staged file(s).", files.len());

    let mut file_changes: Vec<FileChange> = Vec::new();

    for (idx, path) in files.iter().enumerate() {
        println!("\n[{} / {}] {}", idx + 1, files.len(), path);
        println!("How does this file relate to the ticket?");
        println!("  1) Main purpose");
        println!("  2) Supporting change");
        println!("  3) Consequence / ripple");
        println!("  4) Ignore / unrelated cleanup");

        let category = loop {
            let input = prompt_input("Enter choice [1-4]: ")?;
            if let Some(cat) = FileCategory::from_choice(&input) {
                break cat;
            } else {
                println!("Invalid choice. Please enter 1, 2, 3, or 4.");
            }
        };

        let diff = staged_diff_for_file(path)?;

        let mut file_change = FileChange {
            path: path.clone(),
            category,
            diff,
            summary: None,
        };

        if !matches!(category, FileCategory::Ignored) {
            let summary = llm.summarize_file(
                &branch,
                &file_change,
                ticket_summary.as_deref(),
                cli.debug,
            )?;
            file_change.summary = Some(summary);
        }

        file_changes.push(file_change);
    }

    // Final commit message
    let commit_message = llm.generate_commit_message(
        &branch,
        &file_changes,
        ticket_summary.as_deref(),
        cli.debug,
    )?;

    println!();
    println!("----- Commit Message Preview -----");
    println!("{commit_message}");
    println!("----------------------------------");
    Ok(())
}

/// Simple mode: one-shot commit message from entire staged diff.
fn run_simple(cli: &Cli, llm: &dyn LlmClient) -> Result<()> {
    let branch = current_branch()?;
    let diff = staged_diff()?;

    if diff.trim().is_empty() {
        println!("No staged changes found.");
        return Ok(());
    }

    let commit_message = llm.generate_commit_message_simple(&branch, &diff, cli.debug)?;

    println!();
    println!("----- Commit Message Preview -----");
    println!("{commit_message}");
    println!("----------------------------------");

    Ok(())
}

/// PR mode: summarize commits/PRs between base..from into a PR description.
fn run_pr(
    cli: &Cli,
    llm: &dyn LlmClient,
    base: &str,
    from_opt: Option<&str>,
    pr_flag: bool,
    commit_flag: bool,
) -> Result<()> {
    let from_branch = match from_opt {
        Some(name) => name.to_string(),
        None => current_branch()?,
    };

    let items = collect_pr_items(base, &from_branch)?;
    if items.is_empty() {
        println!("No commits found between {base} and {from_branch}.");
        return Ok(());
    }

    // Determine mode
    let mode = if pr_flag {
        PrSummaryMode::ByPrs
    } else if commit_flag {
        PrSummaryMode::ByCommits
    } else {
        // Auto-detect: if multiple distinct PR numbers are present, use PR mode.
        let mut distinct_prs: HashSet<u32> = HashSet::new();
        for item in &items {
            if let Some(n) = item.pr_number {
                distinct_prs.insert(n);
            }
        }
        if distinct_prs.len() >= 2 {
            PrSummaryMode::ByPrs
        } else {
            PrSummaryMode::ByCommits
        }
    };

    if cli.debug {
        eprintln!(
            "[DEBUG] PR mode: base={base}, from={from}, mode={mode}",
            base = base,
            from = from_branch,
            mode = mode.as_str()
        );
        eprintln!("[DEBUG] Found {} commits in range.", items.len());
    }

    let pr_message = llm.generate_pr_message(
        base,
        &from_branch,
        mode,
        &items,
        cli.ticket_summary.as_deref(),
        cli.debug,
    )?;

    println!();
    println!("----- PR Message Preview -----");
    println!("{pr_message}");
    println!("------------------------------");

    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Decide model + client
    let model_from_cli = cli.model.clone();
    let model_env = env::var("COMMITBOT_MODEL").ok();
    let model = model_from_cli
        .or(model_env)
        .unwrap_or_else(|| "gpt-4o-mini".to_string());

    let use_no_model = cli.no_model || model.to_lowercase() == "none";

    let api_key = cli.api_key.clone();

    let boxed_client: Box<dyn LlmClient> = if use_no_model {
        if cli.debug {
            eprintln!("[DEBUG] Using NoopClient (no model calls).");
        }
        Box::new(NoopClient)
    } else {
        let key = api_key.ok_or_else(|| {
            anyhow!(
                "OPENAI_API_KEY (or --api-key) is required unless --no-model or model=none is used"
            )
        })?;

        if cli.debug {
            eprintln!("[DEBUG] Using OpenAiClient with model: {model}");
        }

        Box::new(OpenAiClient::new(key, model))
    };

    match &cli.command {
        Some(Command::Pr {
                 base,
                 from,
                 pr_mode,
                 commit_mode,
             }) => run_pr(
            &cli,
            boxed_client.as_ref(),
            base,
            from.as_deref(),
            *pr_mode,
            *commit_mode,
        ),
        None => {
            if cli.ask {
                run_interactive(&cli, boxed_client.as_ref())
            } else {
                run_simple(&cli, boxed_client.as_ref())
            }
        }
    }
}
