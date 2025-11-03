use anyhow::{anyhow, Context, Result};
use clap::{ArgGroup, Parser};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::env;
use std::io::{self, Write};
use std::process::Command;

/// How the user categorizes each file in interactive mode.
#[derive(Debug, Clone, Copy, Serialize)]
enum FileCategory {
    Main,          // 1
    Supporting,    // 2
    Consequence,   // 3
    Ignored,       // 4
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

/// CLI options
#[derive(Parser, Debug)]
#[command(
    name = "commit",
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
    #[arg(long)]
    ask: bool,

    /// Debug mode: log prompts, responses, token usage
    #[arg(long)]
    debug: bool,

    /// Model name to use (e.g. gpt-4o-mini). If 'none', acts like --no-model.
    #[arg(long)]
    model: Option<String>,

    /// Disable model calls; return dummy responses instead
    #[arg(long)]
    no_model: bool,

    /// API key (otherwise uses OPENAI_API_KEY env var)
    #[arg(long, env = "OPENAI_API_KEY")]
    api_key: Option<String>,

    /// Optional: a brief human description of the ticket (for interactive mode)
    #[arg(long)]
    ticket_summary: Option<String>,
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
    //role: String,
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
                file.path, file.category, truncate(&user_prompt, 2000)
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
        // This is where you can plug in your detailed rules (Introduced vs Refactored etc.)
        // For now, we'll keep it simple but structured.

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
    let output = Command::new("git")
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
    let name = git_output(&["rev-parse", "--abbrev-ref", "HEAD"])?.trim().to_string();
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
            anyhow!("OPENAI_API_KEY (or --api-key) is required unless --no-model or model=none is used")
        })?;

        if cli.debug {
            eprintln!("[DEBUG] Using OpenAiClient with model: {model}");
        }

        Box::new(OpenAiClient::new(key, model))
    };

    if cli.ask {
        run_interactive(&cli, boxed_client.as_ref())
    } else {
        run_simple(&cli, boxed_client.as_ref())
    }
}
