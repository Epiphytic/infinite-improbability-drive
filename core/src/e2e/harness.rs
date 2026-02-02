//! Main E2E test orchestrator.

use std::process::Command;
use std::time::Duration;

use crate::pr::PRManager;
use crate::runner::{ClaudeRunner, GeminiRunner, LLMRunner};
use crate::sandbox::WorktreeSandbox;
use crate::spawn::{SpawnConfig, SpawnResult, Spawner};

use super::fixture::{Fixture, RunnerType, WorkflowType};
use super::repo::EphemeralRepo;
use super::validator::{ValidationResult, Validator};

/// Configuration for E2E test harness.
#[derive(Debug, Clone)]
pub struct E2EConfig {
    /// GitHub organization for ephemeral repos.
    pub org: String,
    /// Whether to delete repos on test success (default: false - keep successful repos).
    pub delete_on_success: bool,
    /// Whether to delete repos on test failure (default: true).
    pub delete_on_failure: bool,
}

impl E2EConfig {
    /// Creates a new E2E config for the given organization.
    pub fn new(org: impl Into<String>) -> Self {
        Self {
            org: org.into(),
            delete_on_success: false,
            delete_on_failure: true,
        }
    }

    /// Sets whether to delete repos on success.
    pub fn with_delete_on_success(mut self, delete: bool) -> Self {
        self.delete_on_success = delete;
        self
    }

    /// Sets whether to delete repos on failure.
    pub fn with_delete_on_failure(mut self, delete: bool) -> Self {
        self.delete_on_failure = delete;
        self
    }
}

impl Default for E2EConfig {
    fn default() -> Self {
        Self::new("epiphytic")
    }
}

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
    /// URL of the plan PR (for full workflow).
    pub plan_pr_url: Option<String>,
    /// Repository full name (org/repo).
    pub repo_name: Option<String>,
    /// Whether the repository was deleted.
    pub repo_deleted: bool,
}

/// E2E test harness.
pub struct E2EHarness {
    config: E2EConfig,
}

impl E2EHarness {
    /// Creates a new E2E harness for the given organization.
    pub fn new(org: impl Into<String>) -> Self {
        Self {
            config: E2EConfig::new(org),
        }
    }

    /// Creates a new E2E harness with the given configuration.
    pub fn with_config(config: E2EConfig) -> Self {
        Self { config }
    }

    /// Runs a fixture and returns the result.
    pub async fn run_fixture(&self, fixture: &Fixture) -> E2EResult {
        match fixture.workflow {
            WorkflowType::Simple => self.run_simple_workflow(fixture).await,
            WorkflowType::Full => self.run_full_workflow(fixture).await,
        }
    }

    /// Runs a simple workflow: prompt -> validate -> PR.
    async fn run_simple_workflow(&self, fixture: &Fixture) -> E2EResult {
        let fixture_name = fixture.name.clone();

        // Create ephemeral repo
        let mut repo = match EphemeralRepo::create(&self.config.org, "e2e") {
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
                    plan_pr_url: None,
                    repo_name: None,
                    repo_deleted: false,
                };
            }
        };

        let repo_name = repo.full_name();
        tracing::info!(
            fixture = %fixture.name,
            repo = %repo_name,
            "running E2E fixture (simple workflow)"
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
                // Delete repo on failure if configured
                if self.config.delete_on_failure {
                    let _ = repo.delete();
                } else {
                    repo.keep();
                }
                return E2EResult {
                    fixture_name,
                    spawn_success: false,
                    spawn_result: None,
                    validation: None,
                    passed: false,
                    error: Some(format!("Spawn failed: {}", e)),
                    pr_url: None,
                    plan_pr_url: None,
                    repo_name: Some(repo_name),
                    repo_deleted: self.config.delete_on_failure,
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

                // Delete repo on failure if configured
                if self.config.delete_on_failure {
                    let _ = repo.delete();
                } else {
                    repo.keep();
                }

                return E2EResult {
                    fixture_name,
                    spawn_success,
                    spawn_result: Some(spawn_result),
                    validation: None,
                    passed: false,
                    error: Some(format!("Validation error: {}", e)),
                    pr_url: None,
                    plan_pr_url: None,
                    repo_name: Some(repo_name),
                    repo_deleted: self.config.delete_on_failure,
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

        // Handle repo deletion based on outcome
        let repo_deleted = if passed {
            if self.config.delete_on_success {
                let _ = repo.delete();
                true
            } else {
                repo.keep();
                false
            }
        } else {
            if self.config.delete_on_failure {
                let _ = repo.delete();
                true
            } else {
                repo.keep();
                false
            }
        };

        E2EResult {
            fixture_name,
            spawn_success,
            spawn_result: Some(spawn_result),
            validation,
            passed,
            error: None,
            pr_url,
            plan_pr_url: None,
            repo_name: Some(repo_name),
            repo_deleted,
        }
    }

    /// Runs a full workflow: plan -> approve -> execute -> validate -> PR.
    async fn run_full_workflow(&self, fixture: &Fixture) -> E2EResult {
        let fixture_name = fixture.name.clone();

        // Create ephemeral repo
        let mut repo = match EphemeralRepo::create(&self.config.org, "e2e") {
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
                    plan_pr_url: None,
                    repo_name: None,
                    repo_deleted: false,
                };
            }
        };

        let repo_name = repo.full_name();
        tracing::info!(
            fixture = %fixture.name,
            repo = %repo_name,
            "running E2E fixture (full workflow)"
        );

        // Phase 1: Planning
        let planning_prompt = fixture.planning_prompt.clone().unwrap_or_else(|| {
            format!(
                "Create a detailed implementation plan for the following task. \
                 Output your plan as a structured markdown document with sections for: \
                 Overview, Tasks (with dependencies), and Risk Areas.\n\n\
                 Task: {}",
                fixture.prompt
            )
        });

        let plan_result = self
            .run_phase(
                &mut repo,
                &planning_prompt,
                fixture.runner,
                fixture.timeout,
                "planning",
            )
            .await;

        let (plan_spawn_result, plan_sandbox_path) = match plan_result {
            Ok((result, path)) => (result, path),
            Err(e) => {
                if self.config.delete_on_failure {
                    let _ = repo.delete();
                } else {
                    repo.keep();
                }
                return E2EResult {
                    fixture_name,
                    spawn_success: false,
                    spawn_result: None,
                    validation: None,
                    passed: false,
                    error: Some(format!("Planning phase failed: {}", e)),
                    pr_url: None,
                    plan_pr_url: None,
                    repo_name: Some(repo_name),
                    repo_deleted: self.config.delete_on_failure,
                };
            }
        };

        // Create plan PR
        let plan_pr_url = if let Some(ref sandbox_path) = plan_sandbox_path {
            self.commit_and_create_pr(
                sandbox_path,
                repo.path(),
                &format!("{}-plan", fixture.name),
                &planning_prompt,
                &plan_spawn_result.spawn_id,
            )
        } else {
            None
        };

        // Phase 2: Approve plan PR (auto-merge)
        if let Some(ref pr_url) = plan_pr_url {
            tracing::info!(pr_url = %pr_url, "approving plan PR");
            if let Err(e) = self.approve_and_merge_pr(pr_url, &repo_name) {
                tracing::warn!(error = %e, "failed to auto-merge plan PR, continuing anyway");
            }
        }

        // Cleanup planning sandbox
        if let Some(sandbox_path) = plan_sandbox_path {
            let _ = std::fs::remove_dir_all(&sandbox_path);
        }

        // Pull merged changes into repo
        let _ = Command::new("git")
            .args(["pull", "origin", "main"])
            .current_dir(repo.path())
            .output();

        // Phase 3: Execution
        let exec_result = self
            .run_phase(
                &mut repo,
                &fixture.prompt,
                fixture.runner,
                fixture.timeout,
                "execution",
            )
            .await;

        let (spawn_result, exec_sandbox_path) = match exec_result {
            Ok((result, path)) => (result, path),
            Err(e) => {
                if self.config.delete_on_failure {
                    let _ = repo.delete();
                } else {
                    repo.keep();
                }
                return E2EResult {
                    fixture_name,
                    spawn_success: false,
                    spawn_result: None,
                    validation: None,
                    passed: false,
                    error: Some(format!("Execution phase failed: {}", e)),
                    pr_url: None,
                    plan_pr_url,
                    repo_name: Some(repo_name),
                    repo_deleted: self.config.delete_on_failure,
                };
            }
        };

        let spawn_success = spawn_result.status == crate::SpawnStatus::Success;

        // Validate results
        let validation_path = exec_sandbox_path
            .as_ref()
            .unwrap_or_else(|| repo.path());

        let validation = match Validator::validate(validation_path, &fixture.validation) {
            Ok(v) => Some(v),
            Err(e) => {
                if let Some(sandbox_path) = &exec_sandbox_path {
                    let _ = std::fs::remove_dir_all(sandbox_path);
                }
                if self.config.delete_on_failure {
                    let _ = repo.delete();
                } else {
                    repo.keep();
                }
                return E2EResult {
                    fixture_name,
                    spawn_success,
                    spawn_result: Some(spawn_result),
                    validation: None,
                    passed: false,
                    error: Some(format!("Validation error: {}", e)),
                    pr_url: None,
                    plan_pr_url,
                    repo_name: Some(repo_name),
                    repo_deleted: self.config.delete_on_failure,
                };
            }
        };

        let passed = spawn_success && validation.as_ref().map(|v| v.passed).unwrap_or(false);

        // Create execution PR
        let mut pr_url = None;
        if passed {
            if let Some(ref sandbox_path) = exec_sandbox_path {
                pr_url = self.commit_and_create_pr(
                    sandbox_path,
                    repo.path(),
                    &fixture.name,
                    &fixture.prompt,
                    &spawn_result.spawn_id,
                );
            }
        }

        // Cleanup execution sandbox
        if let Some(sandbox_path) = exec_sandbox_path {
            let _ = std::fs::remove_dir_all(&sandbox_path);
        }

        // Handle repo deletion based on outcome
        let repo_deleted = if passed {
            if self.config.delete_on_success {
                let _ = repo.delete();
                true
            } else {
                repo.keep();
                false
            }
        } else {
            if self.config.delete_on_failure {
                let _ = repo.delete();
                true
            } else {
                repo.keep();
                false
            }
        };

        E2EResult {
            fixture_name,
            spawn_success,
            spawn_result: Some(spawn_result),
            validation,
            passed,
            error: None,
            pr_url,
            plan_pr_url,
            repo_name: Some(repo_name),
            repo_deleted,
        }
    }

    /// Runs a single phase (planning or execution).
    async fn run_phase(
        &self,
        repo: &mut EphemeralRepo,
        prompt: &str,
        runner_type: RunnerType,
        timeout: u64,
        phase_name: &str,
    ) -> Result<(SpawnResult, Option<std::path::PathBuf>), String> {
        let runner: Box<dyn LLMRunner> = match runner_type {
            RunnerType::Claude => Box::new(ClaudeRunner::new()),
            RunnerType::Gemini => Box::new(GeminiRunner::new()),
        };

        let logs_dir = repo.path().join(format!(".e2e-logs-{}", phase_name));
        let provider = WorktreeSandbox::new(repo.path().clone(), None);
        let spawner = Spawner::new(provider, logs_dir);

        let config = SpawnConfig::new(prompt).with_total_timeout(Duration::from_secs(timeout));

        let manifest = crate::SandboxManifest::default();

        tracing::info!(phase = %phase_name, "running phase");

        let result = spawner
            .spawn(config, manifest, runner)
            .await
            .map_err(|e| format!("{} spawn failed: {}", phase_name, e))?;

        let sandbox_path = result.sandbox_path.clone();
        Ok((result, sandbox_path))
    }

    /// Approves and merges a PR.
    fn approve_and_merge_pr(&self, pr_url: &str, _repo_name: &str) -> Result<(), String> {
        // Extract PR number from URL
        let pr_number = pr_url
            .split('/')
            .last()
            .ok_or_else(|| "Invalid PR URL".to_string())?;

        // Extract repo from URL (format: https://github.com/org/repo/pull/N)
        let parts: Vec<&str> = pr_url.split('/').collect();
        if parts.len() < 5 {
            return Err("Invalid PR URL format".to_string());
        }
        let repo = format!("{}/{}", parts[parts.len() - 4], parts[parts.len() - 3]);

        // Approve the PR
        let approve_output = Command::new("gh")
            .args([
                "pr",
                "review",
                pr_number,
                "--repo",
                &repo,
                "--approve",
                "--body",
                "Auto-approved by E2E test harness",
            ])
            .output()
            .map_err(|e| format!("Failed to run gh pr review: {}", e))?;

        if !approve_output.status.success() {
            let stderr = String::from_utf8_lossy(&approve_output.stderr);
            tracing::warn!(error = %stderr, "PR approval failed (may already be approved)");
        }

        // Merge the PR
        let merge_output = Command::new("gh")
            .args([
                "pr",
                "merge",
                pr_number,
                "--repo",
                &repo,
                "--merge",
                "--delete-branch",
            ])
            .output()
            .map_err(|e| format!("Failed to run gh pr merge: {}", e))?;

        if !merge_output.status.success() {
            let stderr = String::from_utf8_lossy(&merge_output.stderr);
            return Err(format!("PR merge failed: {}", stderr));
        }

        tracing::info!(pr_number = %pr_number, "merged PR");
        Ok(())
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
        let pr_body =
            pr_manager.generate_pr_body(prompt, "E2E test completed successfully", &[], spawn_id);

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
