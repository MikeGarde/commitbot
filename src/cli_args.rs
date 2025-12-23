use clap::{Parser, Subcommand, ArgAction};

/// CLI options
#[derive(Parser, Debug)]
#[command(
    name = "commitbot",
    version,
    about = "LLM-assisted Git commit message generator"
)]
pub struct Cli {
    /// Interactive mode: classify each file and do per-file summaries
    #[arg(long, global = true)]
    pub ask: bool,

    /// Stage all changes before generating the commit message
    #[arg(short, long, global = true)]
    pub stage: bool,

    /// Max concurrent requests to the LLM API
    #[arg(long, global = true)]
    pub max: Option<usize>,

    /// Model name to use (e.g. gpt-4o-mini)
    #[arg(short, long, global = true)]
    pub model: Option<String>,

    /// API key (otherwise uses OPENAI_API_KEY env var)
    #[arg(short = 'k', long, global = true)]
    pub api_key: Option<String>,

    /// LLM provider / API style (openai or ollama)
    #[arg(long, global = true)]
    pub provider: Option<String>,

    /// Base URL for the selected provider (e.g. http://localhost:11434) llama3.1:8b-instruct-q5_K_M
    #[arg(long, global = true)]
    pub url: Option<String>,

    /// Stream responses as they are generated (use `--stream=false` to disable)
    #[arg(
        long,
        global = true,
        default_missing_value = "true",
        num_args = 0..=1,
        value_parser = clap::value_parser!(bool)
    )]
    pub stream: Option<bool>,

    /// Increase verbosity (-v, -vv, -vvv)
    #[arg(short = 'v', long = "verbose", action = ArgAction::Count)]
    pub verbose: u8,

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

    /// Freeform summary provided at the end of the command.
    #[command(external_subcommand)]
    Summary(Vec<String>),
}
