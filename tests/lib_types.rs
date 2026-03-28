//! Tests for core types and functionality in lib.rs

use commitbot::{FileCategory, FileChange};

#[test]
fn file_category_str_representation() {
    assert_eq!(FileCategory::Main.as_str(), "main");
    assert_eq!(FileCategory::Supporting.as_str(), "supporting");
    assert_eq!(FileCategory::Consequence.as_str(), "consequence");
    assert_eq!(FileCategory::Ignored.as_str(), "ignored");
}

#[test]
fn file_change_creation() {
    let file_change = FileChange {
        path: "src/main.rs".to_string(),
        category: FileCategory::Main,
        diff: "diff --git a/src/main.rs b/src/main.rs\n--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1,3 +1,3 @@\n-println!(\"Hello world\");\n+println!(\"Hello commitbot\");\n".to_string(),
        summary: Some("Update hello message".to_string()),
    };

    assert_eq!(file_change.path, "src/main.rs");
    assert_eq!(file_change.category, FileCategory::Main);
    assert_eq!(file_change.summary.as_deref(), Some("Update hello message"));
}
