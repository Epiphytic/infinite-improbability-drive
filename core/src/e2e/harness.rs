//! Main E2E test orchestrator.

use std::time::Duration;

use crate::pr::PRManager;
use crate::runner::{ClaudeRunner, GeminiRunner, LLMRunner};
use crate::sandbox::WorktreeSandbox;
use crate::spawn::{SpawnConfig, SpawnResult, Spawner};

use super::fixture::{Fixture, RunnerType};
use super::repo::EphemeralRepo;
use super::validator::{ValidationResult, Validator};

/// Result of an E2E test run.
#[derive(Debug)]
pub struct E2EResult {
    /// The fixture that was run.
    pub fixture_name: String,
    /// Whether the spawn succeeded.
    pub spawn_success: bool,
    /// Spawn result (if successful).
    pub spawn_result: Option<SpawnResult>,
    /// Validation result.
    pub validation: Option<ValidationResult>,
    /// Overall pass/fail.
    pub passed: bool,
    /// Error message if failed.
    pub error: Option<String>,
    /// URL of the created PR (if any).
    pub pr_url: Option<String>,
}

/// E2E test harness.
pub struct E2EHarness {
    /// GitHub organization for ephemeral repos.
    org: String,
}

impl E2EHarness {
    /// Creates a new E2E harness for the given organization.
    pub fn new(org: impl Into<String>) -> Self {
        Self { org: org.into() }
    }

    /// Runs a fixture and returns the result.
    pub async fn run_fixture(&self, fixture: &Fixture) -> E2EResult {
        let fixture_name = fixture.name.clone();

        // Create ephemeral repo
        let repo = match EphemeralRepo::create(&self.org, "e2e") {
            Ok(r) => r,
            Err(e) => {
                return E2EResult {
                    fixture_name,
                    spawn_success: false,
                    spawn_result: None,
                    validation: None,
                    passed: false,
                    error: Some(format!("Failed to create repo: {}", e)),
                    pr_url: None,
                };
            }
        };

        tracing::info!(
            fixture = %fixture.name,
            repo = %repo.full_name(),
            "running E2E fixture"
        );

        // Create runner
        let runner: Box<dyn LLMRunner> = match fixture.runner {
            RunnerType::Claude => Box::new(ClaudeRunner::new()),
            RunnerType::Gemini => Box::new(GeminiRunner::new()),
        };

        // Create spawner
        let logs_dir = repo.path().join(".e2e-logs");
        let provider = WorktreeSandbox::new(repo.path().clone(), None);
        let spawner = Spawner::new(provider, logs_dir);

        // Configure spawn
        let config = SpawnConfig::new(&fixture.prompt)
            .with_total_timeout(Duration::from_secs(fixture.timeout));

        let manifest = crate::SandboxManifest::default();

        // Run spawn
        let spawn_result = match spawner.spawn(config, manifest, runner).await {
            Ok(result) => result,
            Err(e) => {
                return E2EResult {
                    fixture_name,
                    spawn_success: false,
                    spawn_result: None,
                    validation: None,
                    passed: false,
                    error: Some(format!("Spawn failed: {}", e)),
                    pr_url: None,
                };
            }
        };

        let spawn_success = spawn_result.status == crate::SpawnStatus::Success;

        // Determine validation path - use sandbox if available, otherwise repo root
        let validation_path = spawn_result
            .sandbox_path
            .as_ref()
            .unwrap_or_else(|| repo.path());

        tracing::info!(
            validation_path = ?validation_path,
            sandbox_available = spawn_result.sandbox_path.is_some(),
            "validating results"
        );

        // Validate results
        let validation = match Validator::validate(validation_path, &fixture.validation) {
            Ok(v) => Some(v),
            Err(e) => {
                // Cleanup sandbox if it exists
                if let Some(sandbox_path) = &spawn_result.sandbox_path {
                    let _ = std::fs::remove_dir_all(sandbox_path);
                }

                return E2EResult {
                    fixture_name,
                    spawn_success,
                    spawn_result: Some(spawn_result),
                    validation: None,
                    passed: false,
                    error: Some(format!("Validation error: {}", e)),
                    pr_url: None,
                };
            }
        };

        let passed = spawn_success && validation.as_ref().map(|v| v.passed).unwrap_or(false);

        // If validation passed, commit changes and create PR
        let mut pr_url = None;
        if passed {
            if let Some(sandbox_path) = &spawn_result.sandbox_path {
                pr_url = self.commit_and_create_pr(
                    sandbox_path,
                    repo.path(),
                    &fixture.name,
                    &fixture.prompt,
                    &spawn_result.spawn_id,
                );
            }
        }

        // Cleanup sandbox after validation and PR creation
        if let Some(sandbox_path) = &spawn_result.sandbox_path {
            tracing::info!(path = ?sandbox_path, "cleaning up sandbox after validation");
            let _ = std::fs::remove_dir_all(sandbox_path);
        }

        E2EResult {
            fixture_name,
            spawn_success,
            spawn_result: Some(spawn_result),
            validation,
            passed,
            error: None,
            pr_url,
        }
    }

    /// Commits changes in the sandbox and creates a PR.
    fn commit_and_create_pr(
        &self,
        sandbox_path: &std::path::PathBuf,
        repo_path: &std::path::PathBuf,
        fixture_name: &str,
        prompt: &str,
        spawn_id: &str,
    ) -> Option<String> {
        let pr_manager = PRManager::new(repo_path.clone());

        // Commit changes in sandbox
        let commit_message = format!("E2E test: {}\n\nSpawn ID: {}", fixture_name, spawn_id);
        match pr_manager.commit_changes(sandbox_path, &commit_message) {
            Ok(Some(hash)) => {
                tracing::info!(hash = %hash, "committed changes");
            }
            Ok(None) => {
                tracing::info!("no changes to commit");
                return None;
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to commit changes");
                return None;
            }
        }

        // Get branch name from sandbox path
        let branch_name = sandbox_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("e2e-test");

        // Push branch to remote
        if let Err(e) = pr_manager.push_branch(sandbox_path, branch_name) {
            tracing::warn!(error = %e, "failed to push branch");
            return None;
        }

        tracing::info!(branch = %branch_name, "pushed branch to remote");

        // Generate PR body
        let pr_body = pr_manager.generate_pr_body(prompt, "E2E test completed successfully", &[], spawn_id);

        // Create PR
        match pr_manager.create_pr(
            &format!("E2E: {}", fixture_name),
            &pr_body,
            branch_name,
            "main",
        ) {
            Ok(pr) => {
                tracing::info!(pr_url = %pr.url, pr_number = %pr.number, "created PR");
                Some(pr.url)
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to create PR");
                None
            }
        }
    }
}
