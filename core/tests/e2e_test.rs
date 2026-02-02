//! E2E integration tests.
//!
//! These tests create real GitHub repos and run real LLM commands.
//! They require:
//! - `gh` CLI authenticated
//! - `claude` or `gemini` CLI available
//!
//! Run with: `cargo test --test e2e_test`
//! Run specific: `cargo test --test e2e_test smoke_hello`

use std::path::PathBuf;

use improbability_drive::e2e::{E2EHarness, Fixture};

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("e2e")
        .join("fixtures")
}

#[tokio::test]
#[ignore] // Run manually with --ignored
async fn smoke_hello() {
    let harness = E2EHarness::new("epiphytic");
    let fixture = Fixture::load(fixtures_dir().join("smoke-hello.yaml"))
        .expect("failed to load fixture");

    let result = harness.run_fixture(&fixture).await;

    println!("\n=== E2E Result ===");
    println!("Fixture: {}", result.fixture_name);
    println!("Spawn success: {}", result.spawn_success);
    println!("Passed: {}", result.passed);
    if let Some(pr_url) = &result.pr_url {
        println!("PR URL: {}", pr_url);
    }
    if let Some(validation) = &result.validation {
        println!("Validation messages:");
        for msg in &validation.messages {
            println!("  - {}", msg);
        }
    }
    if let Some(error) = &result.error {
        println!("Error: {}", error);
    }

    assert!(result.passed, "E2E test failed: {:?}", result.error);
}

#[tokio::test]
#[ignore]
async fn code_generation() {
    let harness = E2EHarness::new("epiphytic");
    let fixture = Fixture::load(fixtures_dir().join("code-generation.yaml"))
        .expect("failed to load fixture");

    let result = harness.run_fixture(&fixture).await;

    println!("\n=== E2E Result ===");
    println!("Fixture: {}", result.fixture_name);
    println!("Spawn success: {}", result.spawn_success);
    println!("Passed: {}", result.passed);
    if let Some(spawn_result) = &result.spawn_result {
        println!("Duration: {:?}", spawn_result.duration);
        println!("Summary: {}", spawn_result.summary);
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

    assert!(result.passed, "E2E test failed: {:?}", result.error);
}

#[tokio::test]
#[ignore]
async fn full_web_app() {
    let harness = E2EHarness::new("epiphytic");
    let fixture = Fixture::load(fixtures_dir().join("full-web-app.yaml"))
        .expect("failed to load fixture");

    let result = harness.run_fixture(&fixture).await;

    println!("\n=== E2E Result ===");
    println!("Fixture: {}", result.fixture_name);
    println!("Spawn success: {}", result.spawn_success);
    println!("Passed: {}", result.passed);
    if let Some(spawn_result) = &result.spawn_result {
        println!("Duration: {:?}", spawn_result.duration);
        println!("Summary: {}", spawn_result.summary);
        println!("Commits: {}", spawn_result.commits.len());
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

    assert!(result.passed, "E2E test failed: {:?}", result.error);
}

/// Test that runs with Gemini instead of Claude.
#[tokio::test]
#[ignore]
async fn smoke_hello_gemini() {
    let harness = E2EHarness::new("epiphytic");

    // Load and modify fixture for Gemini
    let yaml = std::fs::read_to_string(fixtures_dir().join("smoke-hello.yaml"))
        .expect("failed to read fixture");
    let yaml = yaml.replace("runner: claude", "runner: gemini");
    let fixture: Fixture = serde_yaml::from_str(&yaml).expect("failed to parse");

    let result = harness.run_fixture(&fixture).await;

    println!("\n=== E2E Result (Gemini) ===");
    println!("Passed: {}", result.passed);
    if let Some(pr_url) = &result.pr_url {
        println!("PR URL: {}", pr_url);
    }

    assert!(result.passed, "E2E test failed: {:?}", result.error);
}
