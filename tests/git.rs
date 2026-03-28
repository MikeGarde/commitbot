use commitbot::git::{
    find_first_pr_number, format_pr_commit_appendix_with_remote, parse_remote_repo,
    short_commit_hash, split_diff_by_file, PrItem, PrSummaryMode,
};

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
fn pr_summary_mode_as_str() {
    assert_eq!(PrSummaryMode::ByCommits.as_str(), "commits");
    assert_eq!(PrSummaryMode::ByPrs.as_str(), "prs");
}
