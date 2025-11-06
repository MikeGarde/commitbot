use assert_cmd::{cargo}; // handy crate for testing CLIs

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