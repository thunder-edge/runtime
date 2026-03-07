//! Rust-level contract tests for the edge testing library.
//!
//! These tests validate the testing library contract by executing the workspace
//! CLI test runner against dedicated JS suites and asserting exit/status output.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("missing crates dir")
        .parent()
        .expect("missing workspace root")
        .to_path_buf()
}

fn run_cli_test(path: &str) -> (i32, String, String) {
    let mut child = Command::new("cargo")
        .args([
            "run",
            "--",
            "test",
            "--path",
            path,
            "--ignore",
            "./tests/js/lib/**",
        ])
        .current_dir(workspace_root())
        .env("NO_COLOR", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to execute cargo run -- test");

    // Prevent indefinite hangs if a nested cargo process gets stuck.
    let timeout = Duration::from_secs(120);
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => break,
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    let output = child
                        .wait_with_output()
                        .expect("failed to collect timed-out process output");

                    return (
                        -1,
                        String::from_utf8_lossy(&output.stdout).to_string(),
                        format!(
                            "timed out after {}s while running cargo run -- test for path '{}'.\n{}",
                            timeout.as_secs(),
                            path,
                            String::from_utf8_lossy(&output.stderr)
                        ),
                    );
                }
                thread::sleep(Duration::from_millis(200));
            }
            Err(e) => {
                return (
                    -1,
                    String::new(),
                    format!("failed while waiting for child process: {e}"),
                );
            }
        }
    }

    let output = child
        .wait_with_output()
        .expect("failed to collect process output");

    let code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    (code, stdout, stderr)
}

fn assert_ok_run(path: &str) -> String {
    let (code, stdout, stderr) = run_cli_test(path);
    if code != 0 {
        panic!(
            "CLI test run failed for path '{}'.\nexit code: {}\nstdout:\n{}\nstderr:\n{}",
            path, code, stdout, stderr
        );
    }
    format!("{}\n{}", stdout, stderr)
}

#[test]
fn contract_mocks_suite_passes() {
    let out = assert_ok_run("./tests/js/mocking_system.test.ts");
    assert!(
        out.contains("Test Suites: 1 total, 1 passed, 0 failed"),
        "expected suite summary in output, got:\n{}",
        out
    );
    assert!(
        out.contains("Tests: 5 total, 5 executed, 5 passed, 0 failed, 0 ignored"),
        "expected stable tests summary in output, got:\n{}",
        out
    );
}

#[test]
fn contract_runner_features_suite_passes() {
    let out = assert_ok_run("./tests/js/runner_advanced_features.test.ts");
    assert!(
        out.contains("Test Suites: 1 total, 1 passed, 0 failed"),
        "expected suite summary in output, got:\n{}",
        out
    );
    assert!(
        out.contains("Tests: 16 total, 14 executed, 14 passed, 0 failed, 2 ignored"),
        "expected advanced-runner summary in output, got:\n{}",
        out
    );
}

#[test]
fn contract_full_js_suite_stays_green() {
    let out = assert_ok_run("./tests/js/**/*.ts");
    assert!(
        out.contains("Test Suites: 10 total, 10 passed, 0 failed"),
        "expected full suite summary in output, got:\n{}",
        out
    );
    assert!(
        out.contains("Tests:"),
        "expected test totals summary line in output, got:\n{}",
        out
    );
}
