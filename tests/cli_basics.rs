use assert_cmd::cargo;
use clap::Parser;
use commitbot::{Cli, Command};

#[test]
fn prints_help() {
    let mut cmd = cargo::cargo_bin_cmd!();

    cmd.arg("--help")
        .assert()
        .success()
        .stdout(predicates::str::contains("Usage"));
}

#[test]
fn prints_version() {
    let mut cmd = cargo::cargo_bin_cmd!();

    cmd.arg("--version")
        .assert()
        .success()
        .stdout(predicates::str::contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn cli_parsing() {
    let cli = Cli::parse_from(["commitbot", "summary"]);

    match cli.command {
        Some(Command::Summary(words)) => assert_eq!(words, vec!["summary"]),
        other => panic!("expected summary command, got {:?}", other),
    }
}
