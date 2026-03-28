use anyhow::{anyhow, Result};
use clap::Parser;
use commitbot::config::Config;
use commitbot::git::{
    collect_pr_items, current_branch, format_pr_commit_appendix, split_diff_by_file,
    staged_diff_for_file, staged_files, PrSummaryMode,
};
use commitbot::llm::LlmClient;
use commitbot::{Cli, Command, FileCategory, FileChange};
use crossterm::{
    cursor,
    event::{self, Event, KeyCode},
    execute,
    style::{self, Color},
    terminal::{self, Clear, ClearType},
};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

use std::collections::HashSet;
use std::io::{self, Read, Write};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

fn prompt_input(prompt: &str) -> Result<String> {
    print!("{prompt}");
    io::stdout().flush()?;

    let mut buf = String::new();
    io::stdin().read_line(&mut buf)?;
    Ok(buf.trim().to_string())
}

fn resolved_ticket_summary(cli: &Cli) -> Option<String> {
    match &cli.command {
        Some(Command::Summary(words)) if !words.is_empty() => Some(words.join(" ")),
        _ => None,
    }
}

fn tprintln<W: Write>(out: &mut W, s: &str) -> io::Result<()> {
    write!(out, "{}\r\n", s)
}

fn preview_snippet(text: &str) -> String {
    let trimmed = text.trim();
    let first_line = trimmed.lines().next().unwrap_or("");
    const MAX: usize = 80;
    if first_line.len() > MAX {
        format!("{}…", &first_line[..MAX])
    } else {
        first_line.to_string()
    }
}

fn dimmed(text: &str) -> String {
    format!("\x1b[2m{text}\x1b[0m")
}

fn categorize_file_interactive(idx: usize, total: usize, path: &str) -> Result<FileCategory> {
    use FileCategory::*;

    let mut stdout = io::stdout();
    std::io::stderr()
        .flush()
        .map_err(|e| anyhow!("failed to flush stderr: {e}"))?;
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
                    KeyCode::Char('1') => return Ok(Main),
                    KeyCode::Char('2') => return Ok(Supporting),
                    KeyCode::Char('3') => return Ok(Consequence),
                    KeyCode::Char('4') => return Ok(Ignored),
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

type SummarizeResultInner = Vec<(usize, Result<String>)>;
type SummarizeResults = Arc<Mutex<SummarizeResultInner>>;

struct SummarizeContext<'a> {
    branch: &'a str,
    ticket_summary: Option<&'a str>,
    llm: &'a dyn LlmClient,
    max_concurrent_requests: usize,
}

fn summarize_files_concurrently(
    file_changes: &mut [FileChange],
    indices: &[usize],
    ctx: &SummarizeContext<'_>,
    pb: &ProgressBar,
    file_lines: Option<&[ProgressBar]>,
) -> Result<()> {
    if indices.is_empty() {
        return Ok(());
    }

    let max_concurrent = ctx.max_concurrent_requests.max(1);
    let total_files = file_changes.len();

    let results: SummarizeResults = Arc::new(Mutex::new(Vec::new()));

    for chunk in indices.chunks(max_concurrent) {
        thread::scope(|scope| {
            for &file_idx in chunk {
                let results = Arc::clone(&results);
                let pb = pb.clone();
                let file_line = file_lines.and_then(|lines| lines.get(file_idx)).cloned();

                if let Some(line) = &file_line {
                    line.set_message("summarizing...");
                }

                let path = file_changes[file_idx].path.clone();
                let diff = file_changes[file_idx].diff.clone();
                let category = file_changes[file_idx].category;

                scope.spawn(move || {
                    log::debug!("Summarizing file: {}", path);

                    let res = (|| -> Result<String> {
                        let fc = FileChange {
                            path,
                            category,
                            diff,
                            summary: None,
                        };

                        let summary = ctx.llm.summarize_file(
                            ctx.branch,
                            &fc,
                            file_idx,
                            total_files,
                            ctx.ticket_summary,
                        )?;
                        Ok(summary)
                    })();

                    pb.inc(1);

                    if let Some(line) = &file_line {
                        match &res {
                            Ok(summary) => {
                                let snippet = preview_snippet(summary);
                                line.finish_with_message(dimmed(&snippet));
                            }
                            Err(err) => {
                                line.finish_with_message(dimmed(&format!("error: {err}")));
                            }
                        }
                    }

                    let mut lock = results.lock().expect("results mutex poisoned");
                    lock.push((file_idx, res));
                });
            }
        });
    }

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

fn run_interactive(cli: &Cli, cfg: &Config, llm: &dyn LlmClient) -> Result<()> {
    let (branch, file_pairs) = if let Some(ref diff_arg) = cli.diff {
        let combined = if diff_arg == "-" {
            let mut buf = String::new();
            io::stdin().read_to_string(&mut buf)?;
            buf
        } else {
            std::fs::read_to_string(diff_arg)
                .map_err(|e| anyhow!("Failed to read diff file '{}': {}", diff_arg, e))?
        };
        if combined.trim().is_empty() {
            println!("No diff content found.");
            return Ok(());
        }
        let mut per_file = split_diff_by_file(&combined);
        if per_file.is_empty() {
            per_file = vec![("(diff)".to_string(), combined)];
        }
        let branch = cli
            .branch
            .clone()
            .unwrap_or_else(|| current_branch().unwrap_or_else(|_| "unknown-branch".to_string()));
        (branch, per_file)
    } else {
        let branch = current_branch()?;
        let files = staged_files()?;
        if files.is_empty() {
            println!("No staged changes found.");
            return Ok(());
        }
        let mut pairs = Vec::new();
        for path in files {
            let diff = staged_diff_for_file(&path)?;
            pairs.push((path, diff));
        }
        (branch, pairs)
    };

    let mut ticket_summary = resolved_ticket_summary(cli);
    if ticket_summary.is_none() {
        let ans = prompt_input("Optional: brief ticket summary (enter to skip): ")?;
        if !ans.is_empty() {
            ticket_summary = Some(ans);
        }
    }

    let mut file_changes: Vec<FileChange> = Vec::new();

    let total_files = file_pairs.len();
    for (idx, (path, diff)) in file_pairs.into_iter().enumerate() {
        let category = categorize_file_interactive(idx, total_files, &path)?;
        file_changes.push(FileChange {
            path,
            category,
            diff,
            summary: None,
        });
    }

    println!();
    println!("Asking {}...", cfg.model);

    let total = file_changes.len();
    let mp = MultiProgress::new();
    let mut file_lines = Vec::new();

    for fc in &file_changes {
        let line = mp.add(ProgressBar::new_spinner());
        line.set_style(
            ProgressStyle::with_template("{spinner:.cyan} {prefix:.bold}: {msg}")
                .expect("progress style template"),
        );
        line.set_prefix(fc.path.clone());
        if matches!(fc.category, FileCategory::Ignored) {
            line.finish_with_message(dimmed("ignored"));
        } else {
            line.enable_steady_tick(Duration::from_millis(120));
            line.set_message("waiting");
        }
        file_lines.push(line);
    }

    let pb = mp.add(ProgressBar::new((total + 1) as u64));
    pb.set_style(
        ProgressStyle::with_template("{wide_bar:.green} {pos}/{len} files")
            .unwrap_or_else(|_| ProgressStyle::default_bar()),
    );

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

    log::info!(
        "Summarizing {} files ({} ignored). max_concurrent_requests = {}",
        indices_to_summarize.len(),
        ignored_count,
        cfg.max_concurrent_requests,
    );

    let ctx = SummarizeContext {
        branch: &branch,
        ticket_summary: ticket_summary.as_deref(),
        llm,
        max_concurrent_requests: cfg.max_concurrent_requests,
    };

    summarize_files_concurrently(
        &mut file_changes,
        &indices_to_summarize,
        &ctx,
        &pb,
        Some(&file_lines),
    )?;

    pb.inc(1);
    pb.finish_with_message("Done");

    println!();

    if cfg.stream {
        let _msg =
            llm.generate_commit_message(&branch, &file_changes, ticket_summary.as_deref())?;
        println!();
    } else {
        let msg = llm.generate_commit_message(&branch, &file_changes, ticket_summary.as_deref())?;
        println!("{msg}");
    }

    Ok(())
}

fn run_auto(cli: &Cli, cfg: &Config, llm: &dyn LlmClient) -> Result<()> {
    let using_external_diff = cli.diff.is_some();
    let (branch, file_pairs): (String, Vec<(String, String)>) =
        if let Some(ref diff_arg) = cli.diff {
            let combined = if diff_arg == "-" {
                let mut buf = String::new();
                io::stdin().read_to_string(&mut buf)?;
                buf
            } else {
                std::fs::read_to_string(diff_arg)
                    .map_err(|e| anyhow!("Failed to read diff file '{}': {}", diff_arg, e))?
            };

            if combined.trim().is_empty() {
                println!("No diff content found.");
                return Ok(());
            }

            let mut per_file = split_diff_by_file(&combined);
            if per_file.is_empty() {
                per_file = vec![("(diff)".to_string(), combined)];
            }
            let branch = cli.branch.clone().unwrap_or_else(|| {
                current_branch().unwrap_or_else(|_| "unknown-branch".to_string())
            });
            (branch, per_file)
        } else {
            let branch = current_branch()?;
            let files = staged_files()?;
            if files.is_empty() {
                println!("No staged changes found.");
                return Ok(());
            }
            let mut pairs = Vec::new();
            for path in files {
                let diff = staged_diff_for_file(&path)?;
                pairs.push((path, diff));
            }
            (branch, pairs)
        };

    let ticket_summary = resolved_ticket_summary(cli);

    let mut file_changes: Vec<FileChange> = file_pairs
        .into_iter()
        .map(|(path, diff)| FileChange {
            path,
            category: FileCategory::Main,
            diff,
            summary: None,
        })
        .collect();

    println!();
    println!("Asking {}...", cfg.model);
    if let Some(ref diff_arg) = cli.diff {
        let diff_source = if diff_arg == "-" { "stdin" } else { diff_arg };
        println!("Using external diff: {diff_source}");
    }

    let total = file_changes.len();
    let mp = MultiProgress::new();
    let mut file_lines = Vec::new();

    for fc in &file_changes {
        let line = mp.add(ProgressBar::new_spinner());
        line.set_style(
            ProgressStyle::with_template("{spinner:.cyan} {prefix:.bold}: {msg}")
                .expect("progress style template"),
        );
        let prefix = if using_external_diff {
            format!("[diff] {}", fc.path)
        } else {
            fc.path.clone()
        };
        line.set_prefix(prefix);
        line.enable_steady_tick(Duration::from_millis(120));
        line.set_message("waiting");
        file_lines.push(line);
    }

    let pb = mp.add(ProgressBar::new((total + 1) as u64));
    pb.set_style(
        ProgressStyle::with_template("{wide_bar:.green} {pos}/{len} files")
            .unwrap_or_else(|_| ProgressStyle::default_bar()),
    );

    let indices_to_summarize: Vec<usize> = (0..total).collect();

    log::info!(
        "Auto-summarizing {} files. max_concurrent_requests = {}",
        total,
        cfg.max_concurrent_requests,
    );

    let ctx = SummarizeContext {
        branch: &branch,
        ticket_summary: ticket_summary.as_deref(),
        llm,
        max_concurrent_requests: cfg.max_concurrent_requests,
    };

    summarize_files_concurrently(
        &mut file_changes,
        &indices_to_summarize,
        &ctx,
        &pb,
        Some(&file_lines),
    )?;

    pb.inc(1);
    pb.finish_with_message("Done");

    println!();

    if cfg.stream {
        let _msg =
            llm.generate_commit_message(&branch, &file_changes, ticket_summary.as_deref())?;
        println!();
    } else {
        let msg = llm.generate_commit_message(&branch, &file_changes, ticket_summary.as_deref())?;
        println!("{msg}");
    }

    Ok(())
}

fn run_pr(
    cli: &Cli,
    cfg: &Config,
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

    let mode = if pr_flag {
        PrSummaryMode::ByPrs
    } else if commit_flag {
        PrSummaryMode::ByCommits
    } else {
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

    log::info!(
        "PR mode: base={base}, from={from}, mode={mode}",
        base = base,
        from = from_branch,
        mode = mode.as_str()
    );
    log::info!("Found {} commits in range.", items.len());

    let ticket_summary = resolved_ticket_summary(cli);
    let _pr_message = if cfg.stream {
        println!();
        let msg =
            llm.generate_pr_message(base, &from_branch, mode, &items, ticket_summary.as_deref())?;
        println!();
        msg
    } else {
        println!();
        let msg =
            llm.generate_pr_message(base, &from_branch, mode, &items, ticket_summary.as_deref())?;

        println!("{msg}");
        msg
    };

    let appendix = format_pr_commit_appendix(&items);
    if !appendix.is_empty() {
        println!();
        println!("{appendix}");
    }

    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Handle custom --version early so we print just the version number.
    if cli.version {
        println!("{}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    commitbot::logging::init_logger(cli.verbose);

    if cli.diff.is_some() && matches!(&cli.command, Some(Command::Pr { .. })) {
        return Err(anyhow!(
            "The --diff flag cannot be used with the 'pr' command.\n\
             PR mode analyzes commit history, not staged diffs."
        ));
    }

    let cfg = Config::from_sources(&cli)?;

    if cli.stage {
        commitbot::git::stage_all()?;
    }

    let boxed_client = commitbot::setup::build_llm_client(&cfg)?;
    boxed_client.validate_model()?;

    match &cli.command {
        Some(Command::Pr {
            base,
            from,
            pr_mode,
            commit_mode,
        }) => run_pr(
            &cli,
            &cfg,
            boxed_client.as_ref(),
            base.as_str(),
            from.as_deref(),
            *pr_mode,
            *commit_mode,
        ),
        Some(Command::Summary(_)) | None => {
            if cli.ask {
                run_interactive(&cli, &cfg, boxed_client.as_ref())
            } else {
                run_auto(&cli, &cfg, boxed_client.as_ref())
            }
        }
    }
}
