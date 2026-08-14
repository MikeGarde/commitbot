//! Tests for the prompts we hand to the LLM.

use commitbot::llm::prompt_builder::commit_message_prompt;
use commitbot::{FileCategory, FileChange};

fn file(path: &str, category: FileCategory, summary: Option<&str>) -> FileChange {
    FileChange {
        path: path.to_string(),
        category,
        diff: "diff --git a/x b/x".to_string(),
        summary: summary.map(str::to_string),
    }
}

#[test]
fn lock_files_are_listed_instead_of_summarized() {
    let files = vec![
        file("src/main.rs", FileCategory::Main, Some("- Wire up X")),
        file("Cargo.lock", FileCategory::Lock, None),
    ];

    let prompt = commit_message_prompt("feature/x", &files, None);

    assert!(prompt.user.contains("src/main.rs"));
    assert!(prompt.user.contains("- Wire up X"));
    assert!(prompt.user.contains("Lock files touched/updated"));
    assert!(prompt.user.contains("- Cargo.lock"));
    // The lock file never gets a per-file block, so it can't be reported missing.
    assert!(!prompt.user.contains("Cargo.lock\nCategory:"));
    assert!(!prompt.user.contains("[missing per-file summary]"));
}

#[test]
fn no_lock_section_without_lock_files() {
    let files = vec![file("src/main.rs", FileCategory::Main, Some("- Wire up X"))];

    let prompt = commit_message_prompt("feature/x", &files, None);

    assert!(!prompt.user.contains("Lock files touched/updated"));
}

#[test]
fn ignored_files_stay_out_of_the_lock_section() {
    let files = vec![
        file("notes.txt", FileCategory::Ignored, None),
        file("yarn.lock", FileCategory::Lock, None),
    ];

    let prompt = commit_message_prompt("feature/x", &files, None);

    assert!(!prompt.user.contains("notes.txt"));
    assert!(prompt.user.contains("- yarn.lock"));
}

#[test]
fn file_count_matches_the_changeset() {
    let files = vec![
        file("src/main.rs", FileCategory::Main, Some("- Wire up X")),
        file("src/lib.rs", FileCategory::Supporting, Some("- Export X")),
        file("Cargo.lock", FileCategory::Lock, None),
    ];

    let prompt = commit_message_prompt("feature/x", &files, None);

    // Every changed file is counted, including the ones we did not summarize.
    assert!(prompt.user.contains("Files Changed: 3"));
}

#[test]
fn per_file_blocks_are_numbered_without_gaps() {
    let files = vec![
        file("src/main.rs", FileCategory::Main, Some("- Wire up X")),
        file("Cargo.lock", FileCategory::Lock, None),
        file("notes.txt", FileCategory::Ignored, None),
        file("src/lib.rs", FileCategory::Supporting, Some("- Export X")),
    ];

    let prompt = commit_message_prompt("feature/x", &files, None);

    // Two files were summarized, so they are "1 of 2" and "2 of 2" — the
    // skipped files in between must not leave a hole in the numbering.
    assert!(prompt.user.contains("File 1 of 2: src/main.rs"));
    assert!(prompt.user.contains("File 2 of 2: src/lib.rs"));
    assert!(!prompt.user.contains("of 4"));
}
