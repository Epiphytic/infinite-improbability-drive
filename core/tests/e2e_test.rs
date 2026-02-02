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

use std::path::PathBuf;

use improbability_drive::e2e::{E2EConfig, E2EHarness, Fixture};

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("e2e")
        .join("fixtures")
}

fn create_harness() -> E2EHarness {
    let delete_on_success = std::env::var("E2E_DELETE_ON_SUCCESS")
        .map(|v| v == "1" || v.to_lowercase() == "true")
        .unwrap_or(false);

    let config = E2EConfig::new("epiphytic")
        .with_delete_on_success(delete_on_success)
        .with_delete_on_failure(true);

    E2EHarness::with_config(config)
}

fn print_result(result: &improbability_drive::e2e::E2EResult) {
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

#[tokio::test]
#[ignore] // Run manually with --ignored
async fn smoke_hello() {
    let harness = create_harness();
    let fixture = Fixture::load(fixtures_dir().join("smoke-hello.yaml"))
        .expect("failed to load fixture");

    let result = harness.run_fixture(&fixture).await;
    print_result(&result);

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
