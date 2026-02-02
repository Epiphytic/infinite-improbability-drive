//! Main E2E test orchestrator.

use std::time::Duration;

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
                };
            }
        };

        let spawn_success = spawn_result.status == crate::SpawnStatus::Success;

        // Validate results
        let validation = match Validator::validate(repo.path(), &fixture.validation) {
            Ok(v) => Some(v),
            Err(e) => {
                return E2EResult {
                    fixture_name,
                    spawn_success,
                    spawn_result: Some(spawn_result),
                    validation: None,
                    passed: false,
                    error: Some(format!("Validation error: {}", e)),
                };
            }
        };

        let passed = spawn_success && validation.as_ref().map(|v| v.passed).unwrap_or(false);

        E2EResult {
            fixture_name,
            spawn_success,
            spawn_result: Some(spawn_result),
            validation,
            passed,
            error: None,
        }
    }
}
