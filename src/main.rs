mod config;
mod llm;

use anyhow::{anyhow, Context, Result};
use clap::{ArgGroup, Parser, Subcommand};
use config::Config;
use llm::{LlmClient, NoopClient};
use llm::openai::OpenAiClient;
use std::collections::HashSet;
use std::io::{self, Write};
use std::process::Command as GitCommand;

/// How the user categorizes each file in interactive mode.
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub enum FileCategory {
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
    pub path: String,
    pub category: FileCategory,
    pub diff: String,
    pub summary: Option<String>, // Filled by per-file model call (or dummy)
}

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
pub struct Cli {
    /// Interactive mode: classify each file and do per-file summaries
    #[arg(long, global = true)]
    pub ask: bool,

    /// Debug mode: log prompts, responses, token usage
    #[arg(long, global = true)]
    pub debug: bool,

    /// Model name to use (e.g. gpt-4o-mini). If 'none', acts like --no-model.
    #[arg(long, global = true)]
    pub model: Option<String>,
    /// Disable model calls; return dummy responses instead
    #[arg(long, global = true)]
    pub no_model: bool,

    /// API key (otherwise uses OPENAI_API_KEY env var)
    #[arg(long, env = "OPENAI_API_KEY", global = true)]
    pub api_key: Option<String>,

    /// Optional: a brief human description of the ticket (for commit/PR summaries)
    #[arg(long, global = true)]
    pub ticket_summary: Option<String>,

    /// Subcommand (e.g. 'pr')
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Subcommands, e.g. `commitbot pr develop`
#[derive(Subcommand, Debug)]
pub enum Command {
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

    // Resolve config (model from cli/env/toml)
    let cfg = Config::from_sources(&cli);

    let use_no_model = cli.no_model || cfg.model.to_lowercase() == "none";
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
            eprintln!("[DEBUG] Using OpenAiClient with model: {}", cfg.model);
        }

        Box::new(OpenAiClient::new(key, cfg.model.clone()))
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
