use commitbot::git::{
    find_first_pr_number, format_git_error, format_pr_commit_appendix_with_remote,
    looks_like_commit_hash, parse_remote_repo, resolve_commit_diff, short_commit_hash,
    split_diff_by_file, staged_diff_for_file, staged_files, PrItem, PrSummaryMode,
};
use std::process::Command;
use std::sync::Mutex;

/// `std::env::set_current_dir` changes process-wide state, so tests that use
/// it must not run concurrently with each other.
static CWD_LOCK: Mutex<()> = Mutex::new(());

/// Set up a throwaway git repo with a staged change in a nested file, and
/// return its tempdir handle plus the nested directory's path.
fn repo_with_staged_nested_file() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();

    let run = |args: &[&str]| {
        let output = Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .expect("run git");
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let err = format_git_error(args, output.status.code(), &stderr);
            panic!("{err}");
        }
    };

    run(&["init", "-q"]);
    run(&["config", "user.email", "test@example.com"]);
    run(&["config", "user.name", "Test"]);

    let nested_dir = root.join("app").join("Models");
    std::fs::create_dir_all(&nested_dir).expect("mkdir");
    let file = nested_dir.join("OrderItem.php");
    std::fs::write(&file, "original\n").expect("write");
    run(&["add", "."]);
    run(&["commit", "-q", "-m", "initial"]);

    std::fs::write(&file, "original\nchanged\n").expect("rewrite");
    run(&["add", "."]);

    (dir, nested_dir)
}

#[test]
fn parses_github_ssh_remote() {
    let remote = parse_remote_repo("git@github.com:owner/repo.git").unwrap();
    assert_eq!(remote.provider, commitbot::git::GitProvider::GitHub);
    assert_eq!(remote.repo_id().as_deref(), Some("owner/repo"));
    assert_eq!(
        remote.commit_url("abcdef123456").as_deref(),
        Some("https://github.com/owner/repo/commit/abcdef123456")
    );
}

#[test]
fn parses_gitlab_https_remote() {
    let remote = parse_remote_repo("https://gitlab.example.com/group/subgroup/repo.git").unwrap();
    assert_eq!(remote.provider, commitbot::git::GitProvider::GitLab);
    assert_eq!(remote.repo_id().as_deref(), Some("subgroup/repo"));
    assert_eq!(
        remote.commit_url("abcdef123456").as_deref(),
        Some("https://gitlab.example.com/group/subgroup/repo/-/commit/abcdef123456")
    );
}

#[test]
fn parses_azure_ssh_remote() {
    let remote = parse_remote_repo("git@ssh.dev.azure.com:v3/org/project/repo").unwrap();
    assert_eq!(remote.provider, commitbot::git::GitProvider::AzureDevOps);
    assert_eq!(remote.repo_id().as_deref(), Some("project/repo"));
    assert_eq!(
        remote.commit_url("abcdef123456").as_deref(),
        Some("https://dev.azure.com/org/project/_git/repo/commit/abcdef123456")
    );
}

#[test]
fn appendix_falls_back_to_hash_only_when_no_remote() {
    let items = vec![PrItem {
        commit_hash: "abcdef123456".to_string(),
        title: "Refine PR footer rendering".to_string(),
        body: String::new(),
        pr_number: None,
    }];

    let appendix = format_pr_commit_appendix_with_remote(&items, None);
    assert!(appendix.contains("Commits in this PR:"));
    assert!(appendix.contains("- `abcdef1` Refine PR footer rendering"));
}

#[test]
fn find_first_pr_number_in_title() {
    let result = find_first_pr_number("Fix bug in #123");
    assert_eq!(result, Some(123));
}

#[test]
fn find_first_pr_number_in_body() {
    let result = find_first_pr_number("Closes #456");
    assert_eq!(result, Some(456));
}

#[test]
fn find_first_pr_number_multiple_hashes() {
    let result = find_first_pr_number("Related to #100, fixes #200");
    assert_eq!(result, Some(100));
}

#[test]
fn find_first_pr_number_no_hash() {
    let result = find_first_pr_number("No PR here");
    assert_eq!(result, None);
}

#[test]
fn find_first_pr_number_empty() {
    let result = find_first_pr_number("");
    assert_eq!(result, None);
}

#[test]
fn split_diff_by_file_single_file() {
    let diff = r#"diff --git a/src/main.rs b/src/main.rs
index 1234567..89abcdef 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,3 +1,4 @@
+use std::io;
 fn main() {
     println!("Hello");
 }"#;
    let result = split_diff_by_file(diff);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].0, "src/main.rs");
    assert!(result[0].1.contains("diff --git"));
}

#[test]
fn split_diff_by_file_multiple_files() {
    let diff = r#"diff --git a/src/main.rs b/src/main.rs
index 1234567..89abcdef 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,3 +1,4 @@
+use std::io;
diff --git a/src/lib.rs b/src/lib.rs
index abcdefg..1234567 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,2 +1,3 @@
+pub fn helper() {}
 pub fn other() {}"#;
    let result = split_diff_by_file(diff);
    assert_eq!(result.len(), 2);
    assert_eq!(result[0].0, "src/main.rs");
    assert_eq!(result[1].0, "src/lib.rs");
}

#[test]
fn split_diff_by_file_empty() {
    let result = split_diff_by_file("");
    assert!(result.is_empty());
}

#[test]
fn short_commit_hash_works() {
    let result = short_commit_hash("abcdef123456");
    assert_eq!(result, "abcdef1");
}

#[test]
fn short_commit_hash_short_input() {
    let result = short_commit_hash("abc");
    assert_eq!(result, "abc");
}

#[test]
fn staged_diff_for_file_works_from_subdirectory() {
    let _guard = CWD_LOCK.lock().expect("cwd lock");
    let (_dir, nested_dir) = repo_with_staged_nested_file();

    let original_cwd = std::env::current_dir().expect("current dir");
    std::env::set_current_dir(&nested_dir).expect("chdir into nested dir");

    let result = (|| {
        let files = staged_files()?;
        assert_eq!(files, vec!["app/Models/OrderItem.php".to_string()]);

        let diff = staged_diff_for_file(&files[0])?;
        assert!(
            diff.contains("+changed"),
            "expected diff to contain the staged change, got: {diff:?}"
        );
        anyhow::Ok(())
    })();

    std::env::set_current_dir(original_cwd).expect("restore cwd");
    result.expect("staged diff lookup from subdirectory");
}

#[test]
fn pr_summary_mode_as_str() {
    assert_eq!(PrSummaryMode::ByCommits.as_str(), "commits");
    assert_eq!(PrSummaryMode::ByPrs.as_str(), "prs");
}

#[test]
fn format_git_error_detects_xcode_license_error() {
    let stderr = "You have not agreed to the Xcode license agreements. Please run 'sudo xcodebuild -license' from within a Terminal window to review and agree to the Xcode and Apple SDKs license.";
    let err = format_git_error(&["rev-parse", "--abbrev-ref", "HEAD"], Some(69), stderr);
    let msg = err.to_string();
    assert!(msg.contains("Xcode license agreement required"));
    assert!(msg.contains("sudo xcodebuild -license"));
}

#[test]
fn format_git_error_legacy_xcode_license_error() {
    let stderr = "Agreeing to the Xcode/iOS license requires admin privileges, please run 'sudo xcodebuild -license' and then retry this command.";
    let err = format_git_error(&["diff", "--cached"], Some(69), stderr);
    let msg = err.to_string();
    assert!(msg.contains("Xcode license agreement required"));
    assert!(msg.contains("sudo xcodebuild -license"));
}

#[test]
fn format_git_error_standard_git_error() {
    let stderr = "fatal: not a git repository (or any of the parent directories): .git";
    let err = format_git_error(&["status"], Some(128), stderr);
    let msg = err.to_string();
    assert!(msg.contains("git [\"status\"] exited with status Some(128)"));
    assert!(msg.contains("fatal: not a git repository"));
}

#[test]
fn looks_like_commit_hash_accepts_hex_strings_in_range() {
    assert!(looks_like_commit_hash("a5484b6"));
    assert!(looks_like_commit_hash("a5484b6ce03e4f1503c7a0fdda7c120bf73c8bc"));
    assert!(looks_like_commit_hash("dead"));
}

#[test]
fn looks_like_commit_hash_rejects_non_hex_or_bad_length() {
    assert!(!looks_like_commit_hash("abc")); // too short
    assert!(!looks_like_commit_hash("my-changes.diff")); // not hex
    assert!(!looks_like_commit_hash("-")); // stdin marker
    assert!(!looks_like_commit_hash(
        "a5484b6ce03e4f1503c7a0fdda7c120bf73c8bcaa" // too long
    ));
}

#[test]
fn resolve_commit_diff_finds_existing_commit() {
    let _guard = CWD_LOCK.lock().expect("cwd lock");
    let (_dir, nested_dir) = repo_with_staged_nested_file();

    let original_cwd = std::env::current_dir().expect("current dir");
    std::env::set_current_dir(&nested_dir).expect("chdir into nested dir");

    let result = (|| {
        let hash = commitbot::git::git_output(&["rev-parse", "HEAD"])?
            .trim()
            .to_string();
        let diff = resolve_commit_diff(&hash)?.expect("commit should resolve");
        assert!(diff.contains("OrderItem.php"));
        anyhow::Ok(())
    })();

    std::env::set_current_dir(original_cwd).expect("restore cwd");
    result.expect("resolve_commit_diff for existing commit");
}

#[test]
fn resolve_commit_diff_returns_none_for_unknown_hash() {
    let _guard = CWD_LOCK.lock().expect("cwd lock");
    let (_dir, nested_dir) = repo_with_staged_nested_file();

    let original_cwd = std::env::current_dir().expect("current dir");
    std::env::set_current_dir(&nested_dir).expect("chdir into nested dir");

    let result = resolve_commit_diff("deadbeef");

    std::env::set_current_dir(original_cwd).expect("restore cwd");
    assert_eq!(result.expect("should not error"), None);
}

#[test]
fn format_git_error_empty_stderr() {
    let err = format_git_error(&["status"], Some(1), "");
    let msg = err.to_string();
    assert_eq!(msg, "git [\"status\"] exited with status Some(1)");
}
