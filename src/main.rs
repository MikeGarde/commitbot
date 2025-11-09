mod config;
mod llm;
mod git;
mod cli_args;
mod setup;

use anyhow::{anyhow, Result};
use clap::Parser;
use config::Config;
use indicatif::ProgressBar;

use std::collections::HashSet;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use std::thread;

use crate::cli_args::{Cli, Command};
use crate::git::{
    collect_pr_items, current_branch, staged_diff, staged_diff_for_file, staged_files, stage_all,
    write_commit_editmsg, PrSummaryMode,
};
use crate::llm::LlmClient;

use crossterm::{
    cursor,
    event::{self, Event, KeyCode},
    execute,
    style::{self, Color},
    terminal::{self, Clear, ClearType},
};

/// Re-export for modules like `config` that might refer to `crate::Cli` / `crate::Command`.
pub use cli_args::{Cli as RootCli, Command as RootCommand};

/// How the user categorizes each file in interactive mode.
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub enum FileCategory {
    Main,        // 1
    Supporting,  // 2
    Consequence, // 3
    Ignored,     // 4
}

impl FileCategory {
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

/// Ask the user a question and return a trimmed input line.
fn prompt_input(prompt: &str) -> Result<String> {
    print!("{prompt}");
    io::stdout().flush()?;

    let mut buf = String::new();
    io::stdin().read_line(&mut buf)?;
    Ok(buf.trim().to_string())
}

/// Helper: write a line in raw mode so we also reset the cursor to column 0.
fn tprintln<W: Write>(out: &mut W, s: &str) -> io::Result<()> {
    write!(out, "{}\r\n", s)
}

/// Arrow-key UI for choosing a FileCategory (no diff preview).
fn categorize_file_interactive(idx: usize, total: usize, path: &str) -> Result<FileCategory> {
    use FileCategory::*;

    let mut stdout = io::stdout();
    terminal::enable_raw_mode().map_err(|e| anyhow!("failed to enable raw mode: {e}"))?;

    let res = (|| -> Result<FileCategory> {
        let labels = [
            "1) Main purpose",
            "2) Supporting change",
            "3) Consequence / ripple",
            "4) Ignore / unrelated cleanup",
        ];

        let mut selected_index: usize = 0;

        loop {
            execute!(stdout, Clear(ClearType::All), cursor::MoveTo(0, 0))?;

            tprintln(&mut stdout, &format!("[{} / {}] {}", idx + 1, total, path))?;
            tprintln(&mut stdout, "")?;
            tprintln(&mut stdout, "How does this file relate to the ticket?")?;
            tprintln(&mut stdout, "")?;

            for (i, label) in labels.iter().enumerate() {
                let color = if i == selected_index {
                    Color::White
                } else {
                    Color::DarkGrey
                };

                execute!(stdout, style::SetForegroundColor(color))?;
                tprintln(&mut stdout, &format!("  {}", label))?;
                execute!(stdout, style::ResetColor)?;
            }

            tprintln(&mut stdout, "")?;
            tprintln(
                &mut stdout,
                "Use ↑/↓ to move, Enter to select, or 1–4 as a shortcut.",
            )?;

            stdout.flush()?;

            let ev = event::read()?;
            if let Event::Key(key) = ev {
                match key.code {
                    // Arrow navigation
                    KeyCode::Up => {
                        if selected_index == 0 {
                            selected_index = labels.len() - 1;
                        } else {
                            selected_index -= 1;
                        }
                    }
                    KeyCode::Down => {
                        selected_index = (selected_index + 1) % labels.len();
                    }

                    // Numeric shortcuts
                    KeyCode::Char('1') => return Ok(Main),
                    KeyCode::Char('2') => return Ok(Supporting),
                    KeyCode::Char('3') => return Ok(Consequence),
                    KeyCode::Char('4') => return Ok(Ignored),

                    // Enter = accept current selection
                    KeyCode::Enter => {
                        let cat = match selected_index {
                            0 => Main,
                            1 => Supporting,
                            2 => Consequence,
                            3 => Ignored,
                            _ => Main,
                        };
                        return Ok(cat);
                    }

                    // Esc = abort
                    KeyCode::Esc => {
                        return Err(anyhow!("aborted by user"));
                    }

                    _ => {}
                }
            }
        }
    })();

    let _ = terminal::disable_raw_mode();
    res
}

/// Run per-file summaries concurrently, honoring `max_concurrent_requests`.
fn summarize_files_concurrently(
    branch: &str,
    file_changes: &mut [FileChange],
    indices: &[usize],
    ticket_summary: Option<&str>,
    llm: &dyn LlmClient,
    max_concurrent_requests: usize,
    debug: bool,
    pb: &ProgressBar,
) -> Result<()> {
    if indices.is_empty() {
        return Ok(());
    }

    let max_concurrent = max_concurrent_requests.max(1);

    // Store (file_index, result) for all summarizations.
    let results: Arc<Mutex<Vec<(usize, Result<String>)>>> =
        Arc::new(Mutex::new(Vec::new()));

    // Process in chunks of `max_concurrent` so we never have more than that many
    // in-flight LLM calls at once.
    for chunk in indices.chunks(max_concurrent) {
        thread::scope(|scope| {
            for &file_idx in chunk {
                let llm_ref = llm;
                let branch = branch.to_string();
                let ticket_summary = ticket_summary.map(str::to_owned);
                let results = Arc::clone(&results);
                let pb = pb.clone();

                // Clone just the data we need from this file so we don't share &mut across threads.
                let path = file_changes[file_idx].path.clone();
                let diff = file_changes[file_idx].diff.clone();
                let category = file_changes[file_idx].category;

                scope.spawn(move || {
                    if debug {
                        eprintln!("[DEBUG] Summarizing file: {}", path);
                    }

                    let res = (|| -> Result<String> {
                        let fc = FileChange {
                            path,
                            category,
                            diff,
                            summary: None,
                        };

                        let summary = llm_ref.summarize_file(
                            &branch,
                            &fc,
                            ticket_summary.as_deref(),
                            debug,
                        )?;
                        Ok(summary)
                    })();

                    // Always advance the progress bar for this file, even if it errors.
                    pb.inc(1);

                    let mut lock = results.lock().expect("results mutex poisoned");
                    lock.push((file_idx, res));
                });
            }
        });
    }

    // Unwrap Arc and Mutex and apply results back onto file_changes.
    let results = Arc::try_unwrap(results)
        .expect("results Arc still has multiple owners")
        .into_inner()
        .expect("results mutex poisoned");

    let mut first_err: Option<anyhow::Error> = None;

    for (idx, res) in results {
        match res {
            Ok(summary) => {
                file_changes[idx].summary = Some(summary);
            }
            Err(e) => {
                if first_err.is_none() {
                    first_err = Some(e);
                }
            }
        }
    }

    if let Some(err) = first_err {
        return Err(err);
    }

    Ok(())
}

/// Interactive mode: classify files, then do all LLM calls afterward (batched with concurrency).
fn run_interactive(cli: &Cli, cfg: &Config, llm: &dyn LlmClient) -> Result<()> {
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

    let mut file_changes: Vec<FileChange> = Vec::new();

    // Phase 1: Cointeractive classification
    for (idx, path) in files.iter().enumerate() {
        let diff = staged_diff_for_file(path)?;

        let category = categorize_file_interactive(idx, files.len(), path)?;

        file_changes.push(FileChange {
            path: path.clone(),
            category,
            diff,
            summary: None,
        });
    }

    // Phase 2: LLM calls
    println!();
    println!("Asking {}...", cfg.model);

    let total = file_changes.len();
    let pb = ProgressBar::new((total + 1) as u64);

    // Pre-increment for ignored files and collect indices that actually need summarization.
    let mut indices_to_summarize = Vec::new();
    let mut ignored_count = 0usize;

    for (idx, fc) in file_changes.iter().enumerate() {
        if matches!(fc.category, FileCategory::Ignored) {
            pb.inc(1);
            ignored_count += 1;
        } else {
            indices_to_summarize.push(idx);
        }
    }

    if cli.debug {
        eprintln!(
            "[DEBUG] Summarizing {} files ({} ignored). max_concurrent_requests = {}",
            indices_to_summarize.len(),
            ignored_count,
            cfg.max_concurrent_requests,
        );
    }

    summarize_files_concurrently(
        &branch,
        &mut file_changes,
        &indices_to_summarize,
        ticket_summary.as_deref(),
        llm,
        cfg.max_concurrent_requests,
        cli.debug,
        &pb,
    )?;

    // Final commit message
    let commit_message = llm.generate_commit_message(
        &branch,
        &file_changes,
        ticket_summary.as_deref(),
        cli.debug,
    )?;

    pb.inc(1);
    pb.finish_with_message("Done.");

    println!();
    println!("----- Commit Message Preview -----");
    println!("{commit_message}");
    println!("----------------------------------");

    if cli.apply {
        write_commit_editmsg(&commit_message)?;
        println!("(Message written to .git/COMMIT_EDITMSG; run `git commit` to edit/confirm.)");
    }

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

    if cli.apply {
        write_commit_editmsg(&commit_message)?;
        println!("(Message written to .git/COMMIT_EDITMSG; run `git commit` to edit/confirm.)");
    }

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
    // CLI + config
    let cli = Cli::parse();
    let cfg = Config::from_sources(&cli);

    // Pre-Work Items
    if cli.stage {
        stage_all()?;
    }

    // LLM client setup
    let boxed_client = setup::build_llm_client(&cli, &cfg);

    match &cli.command {
        Some(Command::Pr {
                 base,
                 from,
                 pr_mode,
                 commit_mode,
             }) => run_pr(
            &cli,
            boxed_client.as_ref(),
            base.as_str(),
            from.as_deref(),
            *pr_mode,
            *commit_mode,
        ),
        None => {
            if cli.ask {
                run_interactive(&cli, &cfg, boxed_client.as_ref())
            } else {
                run_simple(&cli, boxed_client.as_ref())
            }
        }
    }
}
