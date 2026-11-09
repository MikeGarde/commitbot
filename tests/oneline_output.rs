//! End-to-end tests for the output modes (`--oneline`, `--oneline quiet`) and
//! the trailing Provider/Tokens/Time footer.
//!
//! They run the real binary against a tiny in-process fake of an
//! OpenAI-compatible server (what LM Studio speaks), so no network or real
//! model is needed.

use assert_cmd::cargo;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::thread;

const MESSAGE_PARTS: [&str; 2] = ["Add widget\n\n", "- first bullet\n- second bullet"];
const MESSAGE: &str = "Add widget\n\n- first bullet\n- second bullet";
const ESCAPED_MESSAGE: &str = "Add widget\\n\\n- first bullet\\n- second bullet";

const DIFF: &str = "diff --git a/src/widget.rs b/src/widget.rs\n\
index 111..222 100644\n\
--- a/src/widget.rs\n\
+++ b/src/widget.rs\n\
@@ -1,2 +1,3 @@\n\
 fn widget() {}\n\
+fn gadget() {}\n";

/// Read one HTTP request, returning (request line, body).
fn read_request(stream: &mut TcpStream) -> Option<(String, String)> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos + 4;
        }
    };

    let head = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let content_length = head
        .lines()
        .find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.eq_ignore_ascii_case("content-length").then(|| v.trim().parse::<usize>().ok())?
        })
        .unwrap_or(0);

    while buf.len() < header_end + content_length {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }

    let body = String::from_utf8_lossy(&buf[header_end..]).to_string();
    let request_line = head.lines().next().unwrap_or("").to_string();
    Some((request_line, body))
}

fn respond(stream: &mut TcpStream, content_type: &str, body: &str) {
    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

fn handle(mut stream: TcpStream) {
    let Some((request_line, body)) = read_request(&mut stream) else {
        return;
    };

    if request_line.contains("/api/chat") {
        // Ollama: NDJSON when streaming, a single JSON document otherwise.
        if body.contains("\"stream\":true") {
            let mut nd = String::new();
            for part in MESSAGE_PARTS {
                nd.push_str(&format!(
                    "{}\n",
                    serde_json::json!({ "message": { "role": "assistant", "content": part }, "done": false })
                ));
            }
            nd.push_str(&format!("{}\n", serde_json::json!({ "message": { "role": "assistant", "content": "" }, "done": true, "prompt_eval_count": 11, "eval_count": 7 })));
            respond(&mut stream, "application/x-ndjson", &nd);
        } else {
            let body = serde_json::json!({
                "message": { "role": "assistant", "content": MESSAGE },
                "usage": { "prompt_tokens": 11, "completion_tokens": 7, "total_tokens": 18 }
            })
            .to_string();
            respond(&mut stream, "application/json", &body);
        }
    } else if request_line.contains("/models") {
        let body = serde_json::json!({ "data": [{ "id": "test-model" }] }).to_string();
        respond(&mut stream, "application/json", &body);
    } else if body.contains("\"stream\":true") {
        let mut sse = String::new();
        for part in MESSAGE_PARTS {
            let chunk = serde_json::json!({ "choices": [{ "delta": { "content": part } }] });
            sse.push_str(&format!("data: {chunk}\n\n"));
        }
        // Like a real server, report usage in a trailing chunk, but only when
        // the client asked for it.
        if body.contains("include_usage") {
            let usage = serde_json::json!({
                "choices": [],
                "usage": { "prompt_tokens": 11, "completion_tokens": 7, "total_tokens": 18 }
            });
            sse.push_str(&format!("data: {usage}\n\n"));
        }
        sse.push_str("data: [DONE]\n\n");
        respond(&mut stream, "text/event-stream", &sse);
    } else {
        let body = serde_json::json!({
            "choices": [{ "message": { "content": MESSAGE } }],
            "usage": { "prompt_tokens": 11, "completion_tokens": 7, "total_tokens": 18 }
        })
        .to_string();
        respond(&mut stream, "application/json", &body);
    }
}

/// Start the fake server on a free port; it lives until the test process exits.
fn spawn_fake_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake server");
    let url = format!("http://{}", listener.local_addr().unwrap());
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            thread::spawn(move || handle(stream));
        }
    });
    url
}

struct Run {
    stdout: String,
    stderr: String,
}

fn run_commitbot(extra_args: &[&str]) -> Run {
    run_commitbot_with("lmstudio", extra_args)
}

fn run_commitbot_with(provider: &str, extra_args: &[&str]) -> Run {
    let url = spawn_fake_server();
    let dir = tempfile::tempdir().expect("tempdir");
    let diff_path: PathBuf = dir.path().join("change.diff");
    std::fs::write(&diff_path, DIFF).unwrap();

    let output = cargo::cargo_bin_cmd!()
        .current_dir(dir.path())
        // Never pick up the developer's real config or credentials.
        .env("HOME", dir.path())
        .env_remove("OPENAI_API_KEY")
        .args(["--config", dir.path().join("none.toml").to_str().unwrap()])
        .args(["--provider", provider, "--model", "test-model", "--url", &url])
        .args(["--branch", "main", "--diff", diff_path.to_str().unwrap()])
        .args(extra_args)
        .output()
        .expect("run commitbot");

    let run = Run {
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    };
    assert!(output.status.success(), "commitbot failed: {}", run.stderr);
    run
}

fn assert_footer_order(stdout: &str) {
    let provider = stdout.find("Provider: ").expect("Provider line");
    let tokens = stdout.find("Tokens:").expect("Tokens line");
    let time = stdout.find("Time:").expect("Time line");
    assert!(provider < tokens && tokens < time, "footer order wrong:\n{stdout}");
    assert!(!stdout.contains("generated in"), "old timing line is back:\n{stdout}");
}

#[test]
fn default_output_streams_message_then_footer() {
    let run = run_commitbot(&[]);

    assert!(run.stdout.contains(MESSAGE), "message missing:\n{}", run.stdout);
    assert_footer_order(&run.stdout);
    // The streamed message has no trailing newline; the footer must not be
    // glued onto its last line.
    assert!(
        run.stdout.contains("- second bullet\n"),
        "footer ran into message:\n{}",
        run.stdout
    );
    assert!(!run.stdout.contains(ESCAPED_MESSAGE));
}

#[test]
fn no_stream_output_prints_message_then_footer() {
    let run = run_commitbot(&["--no-stream"]);

    assert!(run.stdout.contains(MESSAGE), "message missing:\n{}", run.stdout);
    assert_footer_order(&run.stdout);
}

#[test]
fn oneline_adds_a_summary_after_the_readable_output() {
    for extra in [&["--oneline"][..], &["--oneline", "--no-stream"][..]] {
        let run = run_commitbot(extra);

        assert!(run.stdout.contains("Asking test-model"), "readable output missing");
        assert_footer_order(&run.stdout);

        let last = run.stdout.lines().rev().find(|l| !l.trim().is_empty()).unwrap();
        assert!(last.starts_with(ESCAPED_MESSAGE), "one-liner wrong: {last}");
        assert!(last.contains("[lmstudio test-model | tokens prompt="), "stats missing: {last}");
    }
}

#[test]
fn oneline_quiet_prints_only_the_summary_line() {
    for extra in [&["--oneline", "quiet"][..], &["--oneline=quiet", "--no-stream"][..]] {
        let run = run_commitbot(extra);

        let lines: Vec<&str> = run.stdout.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(lines.len(), 1, "expected exactly one line, got:\n{}", run.stdout);
        assert!(lines[0].starts_with(ESCAPED_MESSAGE), "one-liner wrong: {}", lines[0]);
        assert!(lines[0].contains("[lmstudio test-model | tokens prompt="));

        for noise in ["Asking", "Provider:", "Tokens:", "Time:", "Using external diff"] {
            assert!(!run.stdout.contains(noise), "{noise:?} leaked:\n{}", run.stdout);
        }
    }
}

#[test]
fn ollama_quiet_streaming_prints_only_the_summary_line() {
    for extra in [&["--oneline", "quiet"][..], &["--oneline", "quiet", "--no-stream"][..]] {
        let run = run_commitbot_with("ollama", extra);

        let lines: Vec<&str> = run.stdout.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(lines.len(), 1, "expected exactly one line, got:\n{}", run.stdout);
        assert!(lines[0].starts_with(ESCAPED_MESSAGE), "one-liner wrong: {}", lines[0]);
        assert!(lines[0].contains("[ollama test-model | tokens prompt="), "stats: {}", lines[0]);
    }
}

#[test]
fn ollama_default_output_streams_message_then_footer() {
    let run = run_commitbot_with("ollama", &[]);

    assert!(run.stdout.contains(MESSAGE), "message missing:\n{}", run.stdout);
    assert_footer_order(&run.stdout);
}

fn git(dir: &std::path::Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .current_dir(dir)
        .args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
        .args(["-c", "commit.gpgsign=false", "-c", "init.defaultBranch=main"])
        .args(args)
        .output()
        .expect("run git");
    assert!(out.status.success(), "git {args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
}

/// Run `commitbot pr main feature` in a throwaway repo with one commit ahead.
fn run_pr(extra_args: &[&str]) -> Run {
    let url = spawn_fake_server();
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = dir.path();

    git(repo, &["init", "-q"]);
    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    git(repo, &["add", "."]);
    git(repo, &["commit", "-q", "-m", "Initial commit"]);
    git(repo, &["checkout", "-q", "-b", "feature"]);
    std::fs::write(repo.join("a.txt"), "two\n").unwrap();
    git(repo, &["commit", "-q", "-am", "Add the second line"]);

    let output = cargo::cargo_bin_cmd!()
        .current_dir(repo)
        .env("HOME", repo)
        .env_remove("OPENAI_API_KEY")
        .args(["--config", repo.join("none.toml").to_str().unwrap()])
        .args(["--provider", "lmstudio", "--model", "test-model", "--url", &url])
        .args(extra_args)
        .args(["pr", "main", "feature"])
        .output()
        .expect("run commitbot pr");

    let run = Run {
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    };
    assert!(output.status.success(), "commitbot pr failed: {}", run.stderr);
    run
}

#[test]
fn pr_output_puts_the_footer_below_the_commit_appendix() {
    for extra in [&["--no-stream"][..], &[][..]] {
        let run = run_pr(extra);

        assert!(run.stdout.contains(MESSAGE), "message missing:\n{}", run.stdout);
        assert_footer_order(&run.stdout);
        assert!(
            run.stdout.contains("Tokens:   prompt=11, completion=7, total=18"),
            "token counts missing:\n{}",
            run.stdout
        );

        let commits = run.stdout.find("Add the second line").expect("commit appendix missing");
        let provider = run.stdout.find("Provider: ").unwrap();
        assert!(commits < provider, "footer must come after the commit list:\n{}", run.stdout);
        assert!(run.stdout.trim_end().lines().last().unwrap().starts_with("Time:"));
    }
}

#[test]
fn pr_oneline_quiet_prints_only_the_summary_line() {
    let run = run_pr(&["--oneline", "quiet", "--no-stream"]);

    let lines: Vec<&str> = run.stdout.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), 1, "expected exactly one line, got:\n{}", run.stdout);
    assert!(lines[0].starts_with(ESCAPED_MESSAGE), "one-liner wrong: {}", lines[0]);
}
