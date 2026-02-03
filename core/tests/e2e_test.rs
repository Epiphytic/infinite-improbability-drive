//! E2E integration tests.
//!
//! These tests create real GitHub repos and run real LLM commands.
//! They require:
//! - `gh` CLI authenticated
//! - `claude` or `gemini` CLI available
//!
//! Run with: `cargo test --test e2e_test`
//! Run specific: `cargo test --test e2e_test smoke_hello`
//!
//! Environment variables:
//! - `E2E_DELETE_ON_SUCCESS=1` - Delete repos even on success
//! - `E2E_KEEP_REPOS=1` - Keep all repos for inspection (overrides other settings)

use std::fs;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use improbability_drive::e2e::{E2EConfig, E2EHarness, E2EResult, Fixture};

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("e2e")
        .join("fixtures")
}

fn results_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("results")
}

fn create_harness() -> E2EHarness {
    // E2E_KEEP_REPOS=1 overrides all deletion settings
    let keep_all = std::env::var("E2E_KEEP_REPOS")
        .map(|v| v == "1" || v.to_lowercase() == "true")
        .unwrap_or(false);

    let delete_on_success = if keep_all {
        false
    } else {
        std::env::var("E2E_DELETE_ON_SUCCESS")
            .map(|v| v == "1" || v.to_lowercase() == "true")
            .unwrap_or(false)
    };

    let delete_on_failure = !keep_all;

    let config = E2EConfig::new("epiphytic")
        .with_delete_on_success(delete_on_success)
        .with_delete_on_failure(delete_on_failure);

    E2EHarness::with_config(config)
}

fn print_result(result: &E2EResult) {
    println!("\n=== E2E Result ===");
    println!("Fixture: {}", result.fixture_name);
    println!("Spawn success: {}", result.spawn_success);
    println!("Passed: {}", result.passed);

    if let Some(repo_name) = &result.repo_name {
        println!("Repository: {}", repo_name);
        if result.repo_deleted {
            println!("  (deleted)");
        } else {
            println!("  (kept - https://github.com/{})", repo_name);
        }
    }

    if let Some(spawn_result) = &result.spawn_result {
        println!("Duration: {:?}", spawn_result.duration);
        println!("Summary: {}", spawn_result.summary);
        if !spawn_result.commits.is_empty() {
            println!("Commits: {}", spawn_result.commits.len());
        }
    }

    if let Some(plan_pr_url) = &result.plan_pr_url {
        println!("Plan PR URL: {}", plan_pr_url);
    }

    if let Some(pr_url) = &result.pr_url {
        println!("PR URL: {}", pr_url);
    }

    if let Some(validation) = &result.validation {
        println!("Validation:");
        for msg in &validation.messages {
            println!("  - {}", msg);
        }
    }

    if let Some(error) = &result.error {
        println!("Error: {}", error);
    }
}

/// Writes E2E test result to a markdown file.
fn write_result_markdown(result: &E2EResult, test_name: &str) {
    let results_path = results_dir();
    let _ = fs::create_dir_all(&results_path);

    let timestamp: DateTime<Utc> = Utc::now();
    let date_str = timestamp.format("%Y-%m-%d").to_string();
    let time_str = timestamp.format("%H:%M:%S UTC").to_string();

    let status_emoji = if result.passed { "✅" } else { "❌" };
    let status_text = if result.passed { "PASSED" } else { "FAILED" };

    let mut md = String::new();

    // Header
    md.push_str(&format!("# E2E Test Result: {}\n\n", result.fixture_name));
    md.push_str(&format!("**Status:** {} {}\n\n", status_emoji, status_text));
    md.push_str(&format!("**Date:** {}\n\n", date_str));
    md.push_str(&format!("**Time:** {}\n\n", time_str));

    // Summary table
    md.push_str("## Summary\n\n");
    md.push_str("| Metric | Value |\n");
    md.push_str("|--------|-------|\n");
    md.push_str(&format!("| Fixture | `{}` |\n", result.fixture_name));
    md.push_str(&format!("| Spawn Success | {} |\n", result.spawn_success));
    md.push_str(&format!("| Overall Passed | {} |\n", result.passed));

    if let Some(spawn_result) = &result.spawn_result {
        md.push_str(&format!("| Duration | {:.2}s |\n", spawn_result.duration.as_secs_f64()));
    }

    if let Some(repo_name) = &result.repo_name {
        if result.repo_deleted {
            md.push_str(&format!("| Repository | `{}` (deleted) |\n", repo_name));
        } else {
            md.push_str(&format!(
                "| Repository | [`{}`](https://github.com/{}) |\n",
                repo_name, repo_name
            ));
        }
    }

    md.push_str("\n");

    // PRs
    if result.plan_pr_url.is_some() || result.pr_url.is_some() {
        md.push_str("## Pull Requests\n\n");

        if let Some(plan_pr_url) = &result.plan_pr_url {
            md.push_str(&format!("- **Plan PR:** [View PR]({})\n", plan_pr_url));
        }

        if let Some(pr_url) = &result.pr_url {
            md.push_str(&format!("- **Implementation PR:** [View PR]({})\n", pr_url));
        }

        md.push_str("\n");
    }

    // Spawn details
    if let Some(spawn_result) = &result.spawn_result {
        md.push_str("## Spawn Details\n\n");
        md.push_str(&format!("**Spawn ID:** `{}`\n\n", spawn_result.spawn_id));
        md.push_str(&format!("**Summary:** {}\n\n", spawn_result.summary));

        if !spawn_result.commits.is_empty() {
            md.push_str(&format!("**Commits:** {}\n\n", spawn_result.commits.len()));
        }
    }

    // Validation results
    if let Some(validation) = &result.validation {
        md.push_str("## Validation\n\n");
        md.push_str(&format!("**Passed:** {}\n\n", validation.passed));

        if !validation.messages.is_empty() {
            md.push_str("### Checks\n\n");
            for msg in &validation.messages {
                let check_emoji = if msg.contains("passed") || msg.contains("Found") {
                    "✅"
                } else {
                    "❌"
                };
                md.push_str(&format!("- {} {}\n", check_emoji, msg));
            }
            md.push_str("\n");
        }
    }

    // Error (if any)
    if let Some(error) = &result.error {
        md.push_str("## Error\n\n");
        md.push_str(&format!("```\n{}\n```\n\n", error));
    }

    // Write individual result file
    let filename = format!("{}-{}.md", test_name, date_str);
    let filepath = results_path.join(&filename);
    let _ = fs::write(&filepath, &md);

    // Also write/update the latest result for this test
    let latest_filename = format!("{}-latest.md", test_name);
    let latest_filepath = results_path.join(&latest_filename);
    let _ = fs::write(&latest_filepath, &md);

    // Update the index
    update_results_index();

    println!("\n📄 Test result written to: {}", filepath.display());
}

/// Updates the results index file with all latest results.
fn update_results_index() {
    let results_path = results_dir();

    let mut md = String::new();
    md.push_str("# E2E Test Results\n\n");
    md.push_str("This directory contains automated E2E test results for infinite-improbability-drive.\n\n");

    md.push_str("## Latest Results\n\n");
    md.push_str("| Test | Status | Date | Link |\n");
    md.push_str("|------|--------|------|------|\n");

    // Find all *-latest.md files
    if let Ok(entries) = fs::read_dir(&results_path) {
        let mut latest_files: Vec<_> = entries
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .ends_with("-latest.md")
            })
            .collect();

        latest_files.sort_by(|a, b| a.file_name().cmp(&b.file_name()));

        for entry in latest_files {
            let filename = entry.file_name().to_string_lossy().to_string();
            let test_name = filename.trim_end_matches("-latest.md");

            // Read the file to extract status and date
            if let Ok(content) = fs::read_to_string(entry.path()) {
                let status = if content.contains("✅ PASSED") {
                    "✅ PASSED"
                } else {
                    "❌ FAILED"
                };

                let date = content
                    .lines()
                    .find(|l| l.starts_with("**Date:**"))
                    .map(|l| l.trim_start_matches("**Date:** ").to_string())
                    .unwrap_or_else(|| "Unknown".to_string());

                md.push_str(&format!(
                    "| {} | {} | {} | [View]({}) |\n",
                    test_name, status, date, filename
                ));
            }
        }
    }

    md.push_str("\n## Test Descriptions\n\n");
    md.push_str("| Test | Description |\n");
    md.push_str("|------|-------------|\n");
    md.push_str("| `smoke_hello` | Basic smoke test - create simple file |\n");
    md.push_str("| `code_generation` | Generate Rust code with tests |\n");
    md.push_str("| `full_web_app` | Full workflow with PingPong mode |\n");
    md.push_str("| `full_web_app_github` | Full workflow with GitHub PR-based reviews |\n");
    md.push_str("| `full_workflow_simple` | Full plan→approve→execute workflow |\n");
    md.push_str("| `smoke_hello_gemini` | Smoke test using Gemini instead of Claude |\n");

    md.push_str("\n## Running Tests\n\n");
    md.push_str("```bash\n");
    md.push_str("# Run all E2E tests (repos deleted on success)\n");
    md.push_str("cargo test --test e2e_test -- --ignored\n");
    md.push_str("\n");
    md.push_str("# Run specific test, keep repo for inspection\n");
    md.push_str("E2E_KEEP_REPOS=1 cargo test --test e2e_test full_web_app -- --ignored\n");
    md.push_str("```\n");

    let index_path = results_path.join("README.md");
    let _ = fs::write(index_path, md);
}

#[tokio::test]
#[ignore] // Run manually with --ignored
async fn smoke_hello() {
    let harness = create_harness();
    let fixture = Fixture::load(fixtures_dir().join("smoke-hello.yaml"))
        .expect("failed to load fixture");

    let result = harness.run_fixture(&fixture).await;
    print_result(&result);
    write_result_markdown(&result, "smoke_hello");

    assert!(result.passed, "E2E test failed: {:?}", result.error);
}

#[tokio::test]
#[ignore]
async fn code_generation() {
    let harness = create_harness();
    let fixture = Fixture::load(fixtures_dir().join("code-generation.yaml"))
        .expect("failed to load fixture");

    let result = harness.run_fixture(&fixture).await;
    print_result(&result);
    write_result_markdown(&result, "code_generation");

    assert!(result.passed, "E2E test failed: {:?}", result.error);
}

#[tokio::test]
#[ignore]
async fn full_web_app() {
    let harness = create_harness();
    let fixture = Fixture::load(fixtures_dir().join("full-web-app.yaml"))
        .expect("failed to load fixture");

    let result = harness.run_fixture(&fixture).await;
    print_result(&result);
    write_result_markdown(&result, "full_web_app");

    assert!(result.passed, "E2E test failed: {:?}", result.error);
}

/// Test that runs with Gemini instead of Claude.
#[tokio::test]
#[ignore]
async fn smoke_hello_gemini() {
    let harness = create_harness();

    // Load and modify fixture for Gemini
    let yaml = std::fs::read_to_string(fixtures_dir().join("smoke-hello.yaml"))
        .expect("failed to read fixture");
    let yaml = yaml.replace("runner: claude", "runner: gemini");
    let fixture: Fixture = serde_yaml::from_str(&yaml).expect("failed to parse");

    let result = harness.run_fixture(&fixture).await;
    print_result(&result);
    write_result_markdown(&result, "smoke_hello_gemini");

    assert!(result.passed, "E2E test failed: {:?}", result.error);
}

/// Test the full workflow: plan -> approve -> execute.
#[tokio::test]
#[ignore]
async fn full_workflow_simple() {
    let harness = create_harness();
    let fixture = Fixture::load(fixtures_dir().join("full-workflow-simple.yaml"))
        .expect("failed to load fixture");

    let result = harness.run_fixture(&fixture).await;
    print_result(&result);
    write_result_markdown(&result, "full_workflow_simple");

    // For full workflow, we expect both plan PR and implementation PR
    if result.passed {
        assert!(
            result.plan_pr_url.is_some(),
            "Full workflow should create a plan PR"
        );
        assert!(
            result.pr_url.is_some(),
            "Full workflow should create an implementation PR"
        );
    }

    assert!(result.passed, "E2E test failed: {:?}", result.error);
}

/// Test the full workflow with GitHub PR-based coordination mode.
/// This uses GitHub reviews with line comments instead of ping-pong.
#[tokio::test]
#[ignore]
async fn full_web_app_github() {
    let harness = create_harness();
    let fixture = Fixture::load(fixtures_dir().join("full-web-app-github.yaml"))
        .expect("failed to load fixture");

    let result = harness.run_fixture(&fixture).await;
    print_result(&result);
    write_result_markdown(&result, "full_web_app_github");

    // For GitHub mode, we expect PR to be created early with reviews
    if result.passed {
        assert!(
            result.plan_pr_url.is_some(),
            "GitHub mode should create a plan PR"
        );
    }

    assert!(result.passed, "E2E test failed: {:?}", result.error);
}
