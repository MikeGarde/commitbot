use clap::{ArgGroup, Parser, Subcommand};

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

    /// If set, write the generated message into .git/COMMIT_EDITMSG (no commit is created)
    #[arg(long, global = true)]
    pub apply: bool,

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
