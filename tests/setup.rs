use clap::Parser;
use commitbot::config::Config;
use commitbot::setup::build_llm_client;
use commitbot::Cli;

fn build(args: &[&str]) -> anyhow::Result<()> {
    let missing = std::env::temp_dir().join("commitbot_setup_test_missing.toml");
    let mut argv = vec!["commitbot", "--config", missing.to_str().unwrap()];
    argv.extend_from_slice(args);
    let cfg = Config::from_sources(&Cli::parse_from(argv))?;
    build_llm_client(&cfg).map(|_| ())
}

#[test]
fn builds_every_provider_with_and_without_quiet_oneline() {
    for quiet in [&[][..], &["--oneline", "quiet"][..]] {
        for provider_args in [
            &["--provider", "openai", "--api-key", "test-key", "--model", "m"][..],
            &["--provider", "ollama", "--url", "http://localhost:11434", "--model", "m"][..],
            &["--provider", "lmstudio", "--url", "http://localhost:1234", "--model", "m"][..],
        ] {
            let args: Vec<&str> = provider_args.iter().chain(quiet).copied().collect();
            build(&args).unwrap_or_else(|e| panic!("{args:?} failed: {e}"));
        }
    }
}

#[test]
fn rejects_unknown_provider() {
    assert!(build(&["--provider", "nope", "--model", "m"]).is_err());
}
