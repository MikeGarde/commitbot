//! Live integration tests for prompt validation
//!
//! These tests actually invoke the commitbot binary with test diffs and
//! validate the LLM output against various constraints.

use assert_cmd::cargo;
use std::fs;
use std::path::PathBuf;

/// Test case definition
struct TestCase {
    name: &'static str,
    language: &'static str,
    diff_file: &'static str,
    expected_keywords: Vec<&'static str>,
    // Constraints (adjust these based on your prompt tuning goals)
    min_words: usize,
    max_words: usize,
    max_lines: usize,
    max_summary_length: usize,
}

/// Output metrics
#[derive(Debug)]
struct OutputMetrics {
    lines: usize,
    words: usize,
    characters: usize,
    summary_length: usize,
    //has_body: bool,
}

impl OutputMetrics {
    fn from_output(text: &str) -> Self {
        let trimmed = text.trim();
        let lines: Vec<&str> = trimmed.lines().collect();

        Self {
            lines: lines.len(),
            words: trimmed.split_whitespace().count(),
            characters: trimmed.chars().count(),
            summary_length: lines.first().map(|l| l.len()).unwrap_or(0),
            //has_body: lines.len() > 2 && lines.get(1).map(|l| l.is_empty()).unwrap_or(false),
        }
    }
}

/// Test result
#[derive(Debug)]
struct TestResult {
    name: String,
    language: String,
    output: String,
    metrics: OutputMetrics,
    keywords_found: Vec<String>,
    keywords_missing: Vec<String>,
    constraint_failures: Vec<String>,
}

impl TestResult {
    fn passed(&self) -> bool {
        self.constraint_failures.is_empty()
    }

    fn print_summary(&self) {
        let status = if self.passed() { "✅ PASS" } else { "❌ FAIL" };
        println!("\n{} {} ({})", status, self.name, self.language);
        println!("  Output: \"{}\"", self.output.lines().next().unwrap_or(""));
        println!("  Metrics: {} lines, {} words, {} chars, summary={} chars",
            self.metrics.lines, self.metrics.words,
            self.metrics.characters, self.metrics.summary_length);
        println!("  Keywords: {}/{} found",
            self.keywords_found.len(),
            self.keywords_found.len() + self.keywords_missing.len());

        if !self.keywords_missing.is_empty() {
            println!("  Missing keywords: {:?}", self.keywords_missing);
        }

        if !self.constraint_failures.is_empty() {
            println!("  Constraint failures:");
            for failure in &self.constraint_failures {
                println!("    - {}", failure);
            }
        }
    }
}

fn get_test_diffs_dir() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests");
    path.push("test_diffs");
    path
}

fn run_commitbot_with_diff(diff_file: &str, branch: &str) -> Result<String, String> {
    let diff_path = get_test_diffs_dir().join(diff_file);

    if !diff_path.exists() {
        return Err(format!("Diff file not found: {:?}", diff_path));
    }

    let mut cmd = cargo::cargo_bin_cmd!("commitbot");

    cmd.arg("--diff")
       .arg(&diff_path)
       .arg("--branch")
       .arg(branch)
       .arg("--stream=false"); // Disable streaming for easier output capture

    let output = cmd.output().expect("Failed to execute commitbot");

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).to_string())
    }
}

fn validate_output(test: &TestCase, output: &str) -> TestResult {
    let metrics = OutputMetrics::from_output(output);
    let mut failures = Vec::new();
    let mut found = Vec::new();
    let mut missing = Vec::new();

    let output_lower = output.to_lowercase();

    // Check keywords
    for kw in &test.expected_keywords {
        if output_lower.contains(&kw.to_lowercase()) {
            found.push(kw.to_string());
        } else {
            missing.push(kw.to_string());
        }
    }

    // Check constraints
    if metrics.words < test.min_words {
        failures.push(format!("words {} < min {}", metrics.words, test.min_words));
    }
    if metrics.words > test.max_words {
        failures.push(format!("words {} > max {}", metrics.words, test.max_words));
    }
    if metrics.lines > test.max_lines {
        failures.push(format!("lines {} > max {}", metrics.lines, test.max_lines));
    }
    if metrics.summary_length > test.max_summary_length {
        failures.push(format!("summary {} > max {}", metrics.summary_length, test.max_summary_length));
    }

    TestResult {
        name: test.name.to_string(),
        language: test.language.to_string(),
        output: output.trim().to_string(),
        metrics,
        keywords_found: found,
        keywords_missing: missing,
        constraint_failures: failures,
    }
}

fn all_test_cases() -> Vec<TestCase> {
    vec![
        TestCase {
            name: "Rust Auth Feature",
            language: "Rust",
            diff_file: "rust_auth_feature.diff",
            expected_keywords: vec!["auth", "token", "validate"],
            min_words: 15,
            max_words: 45,
            max_lines: 10,
            max_summary_length: 72,
        },
        TestCase {
            name: "Rust Fix Panic",
            language: "Rust",
            diff_file: "rust_fix_panic.diff",
            expected_keywords: vec!["fix", "error", "panic"],
            min_words: 3,
            max_words: 15,
            max_lines: 1,
            max_summary_length: 72,
        },
        TestCase {
            name: "Python Rate Limiter",
            language: "Python",
            diff_file: "python_rate_limit.diff",
            expected_keywords: vec!["rate", "limit", "decorator"],
            min_words: 3,
            max_words: 15,
            max_lines: 1,
            max_summary_length: 72,
        },
        TestCase {
            name: "JavaScript Debounce Hook",
            language: "JavaScript",
            diff_file: "js_debounce_hook.diff",
            expected_keywords: vec!["debounce", "hook"],
            min_words: 3,
            max_words: 15,
            max_lines: 1,
            max_summary_length: 72,
        },
        TestCase {
            name: "TypeScript API Types",
            language: "TypeScript",
            diff_file: "ts_api_types.diff",
            expected_keywords: vec!["type", "interface", "api"],
            min_words: 3,
            max_words: 15,
            max_lines: 1,
            max_summary_length: 72,
        },
        TestCase {
            name: "Go Logging Middleware",
            language: "Go",
            diff_file: "go_logging_middleware.diff",
            expected_keywords: vec!["logging", "middleware", "http"],
            min_words: 3,
            max_words: 15,
            max_lines: 1,
            max_summary_length: 72,
        },
        TestCase {
            name: "Java Email Service",
            language: "Java",
            diff_file: "java_email_service.diff",
            expected_keywords: vec!["email", "service"],
            min_words: 3,
            max_words: 15,
            max_lines: 1,
            max_summary_length: 72,
        },
        TestCase {
            name: "PHP Phone Validation",
            language: "PHP",
            diff_file: "php_phone_validation.diff",
            expected_keywords: vec!["validation", "phone"],
            min_words: 3,
            max_words: 15,
            max_lines: 1,
            max_summary_length: 72,
        },
        TestCase {
            name: "C# Repository Pattern",
            language: "C#",
            diff_file: "csharp_repository.diff",
            expected_keywords: vec!["repository"],
            min_words: 3,
            max_words: 15,
            max_lines: 1,
            max_summary_length: 72,
        },
        TestCase {
            name: "Ruby Soft Delete",
            language: "Ruby",
            diff_file: "ruby_soft_delete.diff",
            expected_keywords: vec!["soft", "delete"],
            min_words: 3,
            max_words: 15,
            max_lines: 1,
            max_summary_length: 72,
        },
    ]
}

// =============================================================================
// TESTS
// =============================================================================

#[test]
fn test_all_diff_files_exist() {
    let diffs_dir = get_test_diffs_dir();

    for test in all_test_cases() {
        let path = diffs_dir.join(test.diff_file);
        assert!(path.exists(), "Missing diff file: {} ({:?})", test.name, path);

        let content = fs::read_to_string(&path).expect("Failed to read diff file");
        assert!(!content.is_empty(), "Empty diff file: {}", test.name);
        assert!(
            content.contains("diff --git") || content.contains("---"),
            "Invalid diff format in {}", test.name
        );
    }

    println!("✅ All {} diff files exist and are valid", all_test_cases().len());
}

#[test]
fn test_list_available_fixtures() {
    println!("\n=== Available Test Fixtures ===\n");

    for test in all_test_cases() {
        println!("📁 {} ({})", test.name, test.language);
        println!("   File: {}", test.diff_file);
        println!("   Expected keywords: {:?}", test.expected_keywords);
        println!("   Constraints: {}-{} words, max {} lines, summary <= {} chars",
            test.min_words, test.max_words, test.max_lines, test.max_summary_length);
        println!();
    }
}

#[test]
#[cfg_attr(test, doc = "example")]
fn live_test_all_languages() {
    let mut total_passed = 0;
    let mut total_failed = 0;
    let mut results = Vec::new();

    for test in all_test_cases() {
        print!("Testing {} ({})... ", test.name, test.language);

        match run_commitbot_with_diff(test.diff_file, &format!("feature/{}", test.language.to_lowercase())) {
            Ok(output) => {
                let result = validate_output(&test, &output);
                if result.passed() {
                    println!("✅");
                    total_passed += 1;
                } else {
                    println!("❌");
                    total_failed += 1;
                }
                results.push(result);
            }
            Err(e) => {
                println!("❌ Error: {}", e);
                total_failed += 1;
            }
        }
    }

    println!("\n=== Detailed Results ===");
    for result in &results {
        result.print_summary();
    }

    println!("\n=== Summary ===");
    println!("Passed: {}/{}", total_passed, total_passed + total_failed);
    println!("Failed: {}/{}", total_failed, total_passed + total_failed);

    // Generate CSV report
    println!("\n=== CSV Report ===");
    println!("name,language,lines,words,chars,summary_len,keywords_found,keywords_missing,passed");
    for result in &results {
        println!("{},{},{},{},{},{},{},{},{}",
            result.name,
            result.language,
            result.metrics.lines,
            result.metrics.words,
            result.metrics.characters,
            result.metrics.summary_length,
            result.keywords_found.len(),
            result.keywords_missing.len(),
            result.passed()
        );
    }
}

#[test]
#[cfg_attr(test, doc = "example")]
fn live_test_single_rust() {
    let test = &all_test_cases()[0]; // Rust Auth Feature

    println!("\nRunning single test: {}", test.name);

    match run_commitbot_with_diff(test.diff_file, "feature/auth") {
        Ok(output) => {
            let result = validate_output(test, &output);
            result.print_summary();

            // For manual inspection during prompt tuning
            println!("\n--- Full Output ---");
            println!("{}", output);
            println!("--- End Output ---");
        }
        Err(e) => {
            println!("Error: {}", e);
        }
    }
}

// =============================================================================
// METRIC-FOCUSED TESTS FOR PROMPT TUNING
// =============================================================================

#[test]
#[cfg_attr(test, doc = "example")]
fn live_test_word_count_distribution() {
    let mut word_counts = Vec::new();

    for test in all_test_cases() {
        if let Ok(output) = run_commitbot_with_diff(test.diff_file, "feature/test") {
            let metrics = OutputMetrics::from_output(&output);
            word_counts.push((test.name, test.language, metrics.words));
        }
    }

    word_counts.sort_by_key(|x| x.2);

    println!("Word count distribution (sorted):");
    for (name, lang, words) in &word_counts {
        let bar = "█".repeat(*words);
        println!("{:>3} words | {:20} | {} | {}", words, name, lang, bar);
    }

    if !word_counts.is_empty() {
        let avg: f64 = word_counts.iter().map(|x| x.2 as f64).sum::<f64>() / word_counts.len() as f64;
        let min = word_counts.first().map(|x| x.2).unwrap_or(0);
        let max = word_counts.last().map(|x| x.2).unwrap_or(0);

        println!("\nStatistics:");
        println!("  Min: {} words", min);
        println!("  Max: {} words", max);
        println!("  Avg: {:.1} words", avg);
    }
}

#[test]
#[cfg_attr(test, doc = "example")]
fn live_test_keyword_hit_rate() {
    let mut total_expected = 0;
    let mut total_found = 0;

    for test in all_test_cases() {
        if let Ok(output) = run_commitbot_with_diff(test.diff_file, "feature/test") {
            let result = validate_output(&test, &output);
            let expected = result.keywords_found.len() + result.keywords_missing.len();
            let found = result.keywords_found.len();

            total_expected += expected;
            total_found += found;

            let rate = if expected > 0 { (found as f64 / expected as f64) * 100.0 } else { 0.0 };
            println!("{:20} | {}/{} ({:.0}%) | Found: {:?}",
                test.name, found, expected, rate, result.keywords_found);
        }
    }

    if total_expected > 0 {
        let overall_rate = (total_found as f64 / total_expected as f64) * 100.0;
        println!("\nOverall keyword hit rate: {}/{} ({:.1}%)",
            total_found, total_expected, overall_rate);
    }
}

