//! Main E2E test orchestrator.
//!
//! The E2E harness is intentionally thin - it creates ephemeral repos,
//! kicks off CruiseRunner, and validates results. All the actual work
//! (planning, beads issues, PRs, execution) is done by CruiseRunner.
//! If something doesn't work, it's a bug in CruiseRunner to fix.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use crate::beads::{
    commit_issue_change, BeadsClient, CreateOptions, IssueStatus, IssueType, Priority,
};
use crate::cruise::planner::parse_plan_json;
use crate::cruise::runner::{CruiseRunner, PlanIssue, RunnerType as CruiseRunnerType};
use crate::cruise::task::TaskComplexity;
use crate::pr::{get_branch_commits, get_file_changes, PRManager};
use crate::runner::{ClaudeRunner, GeminiRunner, LLMRunner};
use crate::sandbox::WorktreeSandbox;
use crate::spawn::{SpawnConfig, SpawnResult, Spawner};
use crate::team::{CoordinationMode, SpawnTeamResult};
use crate::team_orchestrator::SpawnObservability;

use super::fixture::{ExecutionPhase, Fixture, RunnerType, TeamMode, WorkflowType};
use super::repo::EphemeralRepo;
use super::validator::{ValidationResult, Validator};

use crate::debug::{is_debug, is_fail_fast};

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
    /// Observability data from spawn-team (if used).
    pub observability: Option<SpawnObservability>,
    /// Spawn-team result (if used).
    pub spawn_team_result: Option<SpawnTeamResult>,
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
        if is_debug() {
            eprintln!("[E2E_DEBUG] === Starting fixture: {} ===", fixture.name);
            eprintln!("  workflow: {:?}", fixture.workflow);
            eprintln!("  phase: {:?}", fixture.phase);
            eprintln!("  team_mode: {:?}", fixture.team_mode);
            eprintln!("  timeout: {} seconds", fixture.timeout);
            eprintln!("  fail_fast: {}", is_fail_fast());
        }

        match fixture.workflow {
            WorkflowType::Simple => self.run_simple_workflow(fixture).await,
            WorkflowType::Full => {
                // Handle phase isolation
                match fixture.phase {
                    ExecutionPhase::All => self.run_full_workflow(fixture).await,
                    ExecutionPhase::PlanOnly => self.run_plan_only(fixture).await,
                    ExecutionPhase::BuildOnly => self.run_build_only(fixture).await,
                    ExecutionPhase::ValidateOnly => self.run_validate_only(fixture).await,
                    ExecutionPhase::PlanAndBuild => self.run_plan_and_build(fixture).await,
                }
            }
        }
    }

    /// Runs a simple workflow: prompt -> validate -> PR.
    async fn run_simple_workflow(&self, fixture: &Fixture) -> E2EResult {
        let fixture_name = fixture.name.clone();

        // Create ephemeral repo with fixture name for clarity
        let mut repo = match EphemeralRepo::create_with_name(&self.config.org, "e2e", Some(&fixture.name)) {
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
                    observability: None,
                    spawn_team_result: None,
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

        // Configure spawn with higher escalation limit for complex E2E tasks
        let config = SpawnConfig::new(&fixture.prompt)
            .with_total_timeout(Duration::from_secs(fixture.timeout))
            .with_max_escalations(5);

        let manifest = crate::SandboxManifest::with_sensible_defaults();

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
                    observability: None,
                    spawn_team_result: None,
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
                    observability: None,
                    spawn_team_result: None,
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
            observability: None,
            spawn_team_result: None,
        }
    }

    /// Runs a full workflow using CruiseRunner.
    ///
    /// The harness is a thin wrapper that:
    /// 1. Creates an ephemeral repo
    /// 2. Calls CruiseRunner.run_full() with spawn-team enabled
    /// 3. Auto-approves the plan PR (simulating human approval)
    /// 4. Validates the results
    /// 5. Reports what happened
    ///
    /// All the actual work (planning, beads issues, PRs, execution) is done by CruiseRunner.
    /// If something doesn't work, it's a bug in CruiseRunner to fix.
    async fn run_full_workflow(&self, fixture: &Fixture) -> E2EResult {
        let fixture_name = fixture.name.clone();

        // Step 1: Create ephemeral repo
        let mut repo = match EphemeralRepo::create_with_name(&self.config.org, "e2e", Some(&fixture.name)) {
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
                    observability: None,
                    spawn_team_result: None,
                };
            }
        };

        let repo_name = repo.full_name();
        let repo_path = repo.path().clone();
        let logs_dir = repo_path.join(".cruise-logs");

        tracing::info!(
            fixture = %fixture.name,
            repo = %repo_name,
            "running E2E fixture (full workflow via CruiseRunner)"
        );

        // Step 2: Create and configure CruiseRunner
        let provider = WorktreeSandbox::new(repo_path.clone(), None);

        // Convert fixture team_mode to CoordinationMode
        let team_mode = match fixture.team_mode {
            TeamMode::PingPong => CoordinationMode::PingPong,
            TeamMode::GitHub => CoordinationMode::GitHub,
        };

        let runner = CruiseRunner::new(provider, logs_dir)
            .with_spawn_team(true)  // Enable spawn-team with Gemini reviews
            .with_team_mode(team_mode)  // Use the fixture's team mode
            .with_auto_approve(true);  // Auto-approve plan PR

        // Step 3: Run CruiseRunner.run_full() - this does ALL the work
        let cruise_result = match runner
            .run_full(
                &fixture.prompt,
                CruiseRunnerType::Claude,
                Duration::from_secs(fixture.timeout),
                &repo_path,
            )
            .await
        {
            Ok(result) => result,
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
                    error: Some(format!("CruiseRunner failed: {}", e)),
                    pr_url: None,
                    plan_pr_url: None,
                    repo_name: Some(repo_name),
                    repo_deleted: self.config.delete_on_failure,
                    observability: None,
                    spawn_team_result: None,
                };
            }
        };

        // Extract results from CruiseRunner
        let plan_pr_url = cruise_result
            .plan_result
            .as_ref()
            .and_then(|p| p.pr_url.clone());

        // Get implementation PR URL from observability (where it's stored by SpawnTeamOrchestrator)
        let pr_url = cruise_result
            .build_result
            .as_ref()
            .and_then(|b| b.observability.as_ref())
            .and_then(|o| o.pr_url.clone());

        let observability = Some(cruise_result.combined_observability());

        // Step 3.5: Validate plan PR requirements
        // Fail early if plan PR doesn't meet requirements
        let plan_validation_errors = self.validate_plan_pr(&cruise_result);
        if !plan_validation_errors.is_empty() {
            if self.config.delete_on_failure {
                let _ = repo.delete();
            } else {
                repo.keep();
            }
            return E2EResult {
                fixture_name,
                spawn_success: cruise_result.success,
                spawn_result: None,
                validation: None,
                passed: false,
                error: Some(format!("Plan PR validation failed:\n{}", plan_validation_errors.join("\n"))),
                pr_url,
                plan_pr_url,
                repo_name: Some(repo_name),
                repo_deleted: self.config.delete_on_failure,
                observability,
                spawn_team_result: None,
            };
        }

        // Step 4: Validate results (harness responsibility - checks fixture expectations)
        let validation = match Validator::validate(&repo_path, &fixture.validation) {
            Ok(v) => Some(v),
            Err(e) => {
                if self.config.delete_on_failure {
                    let _ = repo.delete();
                } else {
                    repo.keep();
                }
                return E2EResult {
                    fixture_name,
                    spawn_success: cruise_result.success,
                    spawn_result: None,
                    validation: None,
                    passed: false,
                    error: Some(format!("Validation error: {}", e)),
                    pr_url,
                    plan_pr_url,
                    repo_name: Some(repo_name),
                    repo_deleted: self.config.delete_on_failure,
                    observability,
                    spawn_team_result: None,
                };
            }
        };

        let passed = cruise_result.success && validation.as_ref().map(|v| v.passed).unwrap_or(false);

        // Step 5: Handle repo cleanup
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

        // Step 6: Return results
        E2EResult {
            fixture_name,
            spawn_success: cruise_result.success,
            spawn_result: None, // CruiseRunner doesn't expose individual SpawnResult
            validation,
            passed,
            error: if cruise_result.success {
                None
            } else {
                Some(cruise_result.summary.clone())
            },
            pr_url,
            plan_pr_url,
            repo_name: Some(repo_name),
            repo_deleted,
            observability,
            spawn_team_result: None,
        }
    }

    /// Runs only the planning phase (no build, no validate).
    ///
    /// This is useful for testing the planning phase in isolation.
    /// It creates an ephemeral repo, runs the planning phase, creates a plan PR,
    /// and returns without executing the plan.
    async fn run_plan_only(&self, fixture: &Fixture) -> E2EResult {
        let fixture_name = fixture.name.clone();

        if is_debug() {
            eprintln!("[E2E_DEBUG] === Running PLAN_ONLY phase ===");
        }

        // Step 1: Create ephemeral repo
        let mut repo = match EphemeralRepo::create_with_name(&self.config.org, "e2e", Some(&fixture.name)) {
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
                    observability: None,
                    spawn_team_result: None,
                };
            }
        };

        let repo_name = repo.full_name();
        let repo_path = repo.path().clone();
        let logs_dir = repo_path.join(".cruise-logs");

        if is_debug() {
            eprintln!("[E2E_DEBUG] Created ephemeral repo: {}", repo_name);
            eprintln!("[E2E_DEBUG] Repo path: {}", repo_path.display());
        }

        tracing::info!(
            fixture = %fixture.name,
            repo = %repo_name,
            "running E2E fixture (PLAN_ONLY phase)"
        );

        // Step 2: Create and configure CruiseRunner for planning only
        let provider = WorktreeSandbox::new(repo_path.clone(), None);

        let team_mode = match fixture.team_mode {
            TeamMode::PingPong => CoordinationMode::PingPong,
            TeamMode::GitHub => CoordinationMode::GitHub,
        };

        let runner = CruiseRunner::new(provider, logs_dir)
            .with_spawn_team(true)
            .with_team_mode(team_mode)
            .with_auto_approve(false); // Don't auto-approve for plan-only

        // Step 3: Run planning phase only
        // CruiseRunner.run_full() does planning, but we just want to run planning phase
        // and stop before execution. We need to use run_planning directly.
        // However, CruiseRunner doesn't expose run_planning_phase directly.
        // So we'll run run_full with a very short execution timeout that will fail,
        // OR we can modify CruiseRunner to expose run_plan_only.
        // For now, let's call run_full and just not validate the build output.
        let cruise_result = match runner
            .run_full(
                &fixture.prompt,
                CruiseRunnerType::Claude,
                Duration::from_secs(fixture.timeout),
                &repo_path,
            )
            .await
        {
            Ok(result) => result,
            Err(e) => {
                if is_debug() {
                    eprintln!("[E2E_DEBUG] CruiseRunner failed: {}", e);
                }
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
                    error: Some(format!("CruiseRunner failed: {}", e)),
                    pr_url: None,
                    plan_pr_url: None,
                    repo_name: Some(repo_name),
                    repo_deleted: self.config.delete_on_failure,
                    observability: None,
                    spawn_team_result: None,
                };
            }
        };

        // For PLAN_ONLY, we consider it a success if planning succeeded
        let plan_pr_url = cruise_result
            .plan_result
            .as_ref()
            .and_then(|p| p.pr_url.clone());

        let observability = Some(cruise_result.combined_observability());

        let plan_success = cruise_result
            .plan_result
            .as_ref()
            .map(|p| p.success)
            .unwrap_or(false);

        let passed = plan_success && plan_pr_url.is_some();

        if is_debug() {
            eprintln!("[E2E_DEBUG] Plan result:");
            eprintln!("  success: {}", plan_success);
            eprintln!("  plan_pr_url: {:?}", plan_pr_url);
            eprintln!("  passed: {}", passed);
        }

        // Cleanup
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
            spawn_success: plan_success,
            spawn_result: None,
            validation: None, // No validation for plan-only
            passed,
            error: if plan_success {
                None
            } else {
                cruise_result.plan_result.as_ref().and_then(|p| p.error.clone())
            },
            pr_url: None, // No implementation PR for plan-only
            plan_pr_url,
            repo_name: Some(repo_name),
            repo_deleted,
            observability,
            spawn_team_result: None,
        }
    }

    /// Runs only the build phase (requires existing plan PR).
    ///
    /// This is useful for testing the build phase in isolation.
    async fn run_build_only(&self, fixture: &Fixture) -> E2EResult {
        let fixture_name = fixture.name.clone();

        if is_debug() {
            eprintln!("[E2E_DEBUG] === Running BUILD_ONLY phase ===");
        }

        // BUILD_ONLY requires an existing plan PR
        if fixture.existing_plan_pr.is_none() {
            return E2EResult {
                fixture_name,
                spawn_success: false,
                spawn_result: None,
                validation: None,
                passed: false,
                error: Some("BUILD_ONLY phase requires existing_plan_pr in fixture".to_string()),
                pr_url: None,
                plan_pr_url: None,
                repo_name: None,
                repo_deleted: false,
                observability: None,
                spawn_team_result: None,
            };
        }

        // TODO: Implement build_only by cloning the repo from the existing plan PR
        // For now, return an error indicating this is not yet implemented
        E2EResult {
            fixture_name,
            spawn_success: false,
            spawn_result: None,
            validation: None,
            passed: false,
            error: Some("BUILD_ONLY phase not yet implemented. Use PLAN_ONLY for now.".to_string()),
            pr_url: None,
            plan_pr_url: fixture.existing_plan_pr.clone(),
            repo_name: None,
            repo_deleted: false,
            observability: None,
            spawn_team_result: None,
        }
    }

    /// Runs only the validate phase (requires existing implementation).
    ///
    /// This is useful for testing validation in isolation.
    async fn run_validate_only(&self, fixture: &Fixture) -> E2EResult {
        let fixture_name = fixture.name.clone();

        if is_debug() {
            eprintln!("[E2E_DEBUG] === Running VALIDATE_ONLY phase ===");
        }

        // VALIDATE_ONLY requires an existing implementation PR
        if fixture.existing_impl_pr.is_none() {
            return E2EResult {
                fixture_name,
                spawn_success: false,
                spawn_result: None,
                validation: None,
                passed: false,
                error: Some("VALIDATE_ONLY phase requires existing_impl_pr in fixture".to_string()),
                pr_url: None,
                plan_pr_url: None,
                repo_name: None,
                repo_deleted: false,
                observability: None,
                spawn_team_result: None,
            };
        }

        // TODO: Implement validate_only by cloning the repo from the existing impl PR
        E2EResult {
            fixture_name,
            spawn_success: false,
            spawn_result: None,
            validation: None,
            passed: false,
            error: Some("VALIDATE_ONLY phase not yet implemented. Use PLAN_ONLY for now.".to_string()),
            pr_url: fixture.existing_impl_pr.clone(),
            plan_pr_url: fixture.existing_plan_pr.clone(),
            repo_name: None,
            repo_deleted: false,
            observability: None,
            spawn_team_result: None,
        }
    }

    /// Runs plan and build phases (skip validation).
    ///
    /// This runs the full workflow but skips validation.
    async fn run_plan_and_build(&self, fixture: &Fixture) -> E2EResult {
        let fixture_name = fixture.name.clone();

        if is_debug() {
            eprintln!("[E2E_DEBUG] === Running PLAN_AND_BUILD phase ===");
        }

        // For now, this just runs the full workflow and ignores validation failures
        let mut result = self.run_full_workflow(fixture).await;

        // For PLAN_AND_BUILD, we consider it passed if both plan and build succeeded
        // even if validation failed
        result.passed = result.spawn_success
            && result.plan_pr_url.is_some()
            && result.pr_url.is_some();

        if is_debug() {
            eprintln!("[E2E_DEBUG] Plan and build result:");
            eprintln!("  spawn_success: {}", result.spawn_success);
            eprintln!("  plan_pr_url: {:?}", result.plan_pr_url);
            eprintln!("  pr_url: {:?}", result.pr_url);
            eprintln!("  passed: {}", result.passed);
        }

        result
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

        let config = SpawnConfig::new(prompt)
            .with_total_timeout(Duration::from_secs(timeout))
            .with_max_escalations(5);

        let manifest = crate::SandboxManifest::with_sensible_defaults();

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

        // Commit changes in sandbox (if any uncommitted)
        let commit_message = format!("E2E test: {}\n\nSpawn ID: {}", fixture_name, spawn_id);
        match pr_manager.commit_changes(sandbox_path, &commit_message) {
            Ok(Some(hash)) => {
                tracing::info!(hash = %hash, "committed changes");
            }
            Ok(None) => {
                // No uncommitted changes, but LLM may have already committed
                tracing::debug!("no uncommitted changes, checking for existing commits");
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to commit changes");
                return None;
            }
        }

        // Check if we have any commits ahead of main (including LLM commits)
        let commits_ahead = Command::new("git")
            .current_dir(sandbox_path)
            .args(["rev-list", "--count", "main..HEAD"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<u32>().ok())
            .unwrap_or(0);

        if commits_ahead == 0 {
            tracing::info!("no commits to create PR for");
            return None;
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

        // Get commits and file changes for enhanced PR body
        let commits = match get_branch_commits(sandbox_path, branch_name, "main") {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "failed to get branch commits, using empty list");
                Vec::new()
            }
        };

        let files_changed = match get_file_changes(sandbox_path, branch_name, "main") {
            Ok(f) => f,
            Err(e) => {
                tracing::warn!(error = %e, "failed to get file changes, using empty list");
                Vec::new()
            }
        };

        // Generate enhanced PR body with accordion, commits, and file stats
        let pr_body = pr_manager.generate_enhanced_pr_body(
            prompt,
            "E2E test completed successfully",
            &commits,
            &files_changed,
            spawn_id,
        );

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

    /// Initializes beads in the repository if not already initialized.
    fn init_beads(&self, repo_path: &PathBuf) -> Result<(), String> {
        let beads = BeadsClient::new(repo_path);

        if beads.is_initialized() {
            tracing::info!(path = ?repo_path, "beads already initialized");
            return Ok(());
        }

        beads.init().map_err(|e| format!("beads init failed: {}", e))?;

        // Stage and commit beads initialization
        let _ = Command::new("git")
            .current_dir(repo_path)
            .args(["add", ".beads/"])
            .output();

        let _ = Command::new("git")
            .current_dir(repo_path)
            .args(["commit", "-m", "Initialize beads issue tracking"])
            .output();

        tracing::info!(path = ?repo_path, "initialized beads");
        Ok(())
    }

    /// Creates a planning prompt that requests structured JSON output.
    fn create_planning_prompt(&self, prompt: &str) -> String {
        format!(
            r#"<!-- E2E_HARNESS_PLANNING_PHASE: This plan is generated by the E2E test harness using the cruise-control planning module -->
<!-- BEADS_INTEGRATION: Issues will be created from this plan and tracked in .beads/ -->

**THIS IS A PLANNING-ONLY PHASE. DO NOT IMPLEMENT ANYTHING.**

Create a detailed implementation plan for the following task and SAVE IT TO A FILE.

**CRITICAL INSTRUCTIONS:**
1. You MUST create a file called `PLAN.md` at the root of the repository.
2. You MUST NOT create any implementation files (no Cargo.toml, no src/, no tests/).
3. You MUST NOT implement the task - only plan it.
4. You MUST NOT make any commits - the harness will commit your changes.

The PLAN.md file must contain:

1. A header that includes: `[E2E Test Plan - Cruise-Control Module Active]`
2. An Overview section explaining the approach
3. A Risk Areas section listing potential issues
4. A JSON block with the tasks in this exact format:

```json
{{
  "title": "Short plan title",
  "overview": "Brief overview",
  "tasks": [
    {{
      "id": "CRUISE-001",
      "subject": "Task title",
      "description": "What needs to be done",
      "blocked_by": [],
      "complexity": "low|medium|high",
      "acceptance_criteria": ["Criterion 1", "Criterion 2"]
    }}
  ],
  "risks": ["Risk 1", "Risk 2"]
}}
```

Use CRUISE-XXX IDs (e.g., CRUISE-001, CRUISE-002). List dependencies in blocked_by using task IDs.

**REMEMBER: ONLY create PLAN.md. Do NOT implement the code. Do NOT commit.**

Task to plan (do NOT implement): {}"#,
            prompt
        )
    }

    /// Extracts plan from spawn output and creates beads issues.
    fn extract_and_create_beads_issues(
        &self,
        sandbox_path: &PathBuf,
        repo_path: &PathBuf,
    ) -> Result<Vec<PlanIssue>, String> {
        tracing::debug!(sandbox_path = ?sandbox_path, repo_path = ?repo_path, "extracting plan and creating beads issues");

        // Look for plan files in the sandbox
        let plan_content = self
            .find_and_read_plan(sandbox_path)
            .map_err(|e| format!("failed to find plan: {}", e))?;

        tracing::debug!(content_len = plan_content.len(), "found plan content");

        // Try to parse as CruisePlan
        let plan = match parse_plan_json(&plan_content) {
            Ok(p) => {
                tracing::info!(title = %p.title, task_count = p.tasks.len(), "parsed plan JSON");
                p
            }
            Err(e) => {
                tracing::warn!(error = %e, "could not parse plan JSON");
                return Err(format!("could not parse plan: {}", e));
            }
        };

        // Create beads issues
        let beads = BeadsClient::new(repo_path);

        // Check if beads is initialized
        if !beads.is_initialized() {
            if let Err(e) = beads.init() {
                return Err(format!("failed to init beads: {}", e));
            }
        }

        let mut created_issues = Vec::new();
        let mut id_mapping: HashMap<String, String> = HashMap::new();

        for task in &plan.tasks {
            let options = CreateOptions {
                description: Some(task.description.clone()),
                design: None,
                acceptance_criteria: if task.acceptance_criteria.is_empty() {
                    None
                } else {
                    Some(task.acceptance_criteria.join("\n- "))
                },
                priority: match task.complexity {
                    TaskComplexity::Low => Priority::Low,
                    TaskComplexity::Medium => Priority::Medium,
                    TaskComplexity::High => Priority::High,
                },
                issue_type: IssueType::Task,
                labels: vec!["cruise-control".to_string(), "e2e-test".to_string()],
                dependencies: vec![],
            };

            match beads.create(&task.subject, options) {
                Ok(result) => {
                    tracing::debug!(beads_id = %result.id, plan_id = %task.id, "created beads issue");
                    id_mapping.insert(task.id.clone(), result.id.clone());
                    created_issues.push(PlanIssue {
                        beads_id: result.id,
                        plan_task_id: task.id.clone(),
                        subject: task.subject.clone(),
                    });
                }
                Err(e) => {
                    tracing::warn!(error = %e, task_id = %task.id, "failed to create beads issue");
                }
            }
        }

        // Add dependencies after all issues are created
        for task in &plan.tasks {
            if let Some(beads_id) = id_mapping.get(&task.id) {
                for dep_id in &task.blocked_by {
                    if let Some(dep_beads_id) = id_mapping.get(dep_id) {
                        if let Err(e) = beads.add_dependency(
                            beads_id,
                            dep_beads_id,
                            crate::beads::DependencyType::Blocks,
                        ) {
                            tracing::warn!(
                                error = %e,
                                from = %beads_id,
                                to = %dep_beads_id,
                                "failed to add dependency"
                            );
                        }
                    }
                }
            }
        }

        // Export beads database to JSONL before committing
        if !created_issues.is_empty() {
            // Export the database to issues.jsonl file
            let jsonl_path = repo_path.join(".beads/issues.jsonl");
            let export_output = Command::new("bd")
                .current_dir(repo_path)
                .args(["export", "-o", jsonl_path.to_str().unwrap_or(".beads/issues.jsonl")])
                .output();

            if let Ok(output) = &export_output {
                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    tracing::warn!(error = %stderr, "beads export failed");
                }
            }

            // Commit the beads issue creation
            let _ = commit_issue_change(
                repo_path,
                "plan",
                &format!("Create {} beads issues from plan", created_issues.len()),
            );
        }

        Ok(created_issues)
    }

    /// Finds and reads plan content from the sandbox.
    fn find_and_read_plan(&self, sandbox_path: &PathBuf) -> Result<String, String> {
        // Look for common plan file locations
        let plan_paths = [
            sandbox_path.join("docs/plans"),
            sandbox_path.join("docs"),
            sandbox_path.join("plan"),
            sandbox_path.clone(),
        ];

        // First, check for specifically named PLAN.md or plan.md in any location
        for dir in &plan_paths {
            for plan_name in ["PLAN.md", "plan.md", "Plan.md"] {
                let plan_file = dir.join(plan_name);
                if plan_file.exists() {
                    if let Ok(content) = std::fs::read_to_string(&plan_file) {
                        tracing::debug!(plan_file = ?plan_file, content_len = content.len(), "found plan file");
                        return Ok(content);
                    }
                }
            }
        }

        // Fall back to searching for any .md file with plan markers
        for dir in &plan_paths {
            if dir.exists() && dir.is_dir() {
                if let Ok(entries) = std::fs::read_dir(dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.extension().map(|e| e == "md").unwrap_or(false) {
                            if let Ok(content) = std::fs::read_to_string(&path) {
                                if content.contains("CRUISE-") || content.contains("```json") {
                                    tracing::debug!(path = ?path, "found plan file with markers");
                                    return Ok(content);
                                }
                            }
                        }
                    }
                }
            }
        }

        Err("No plan file found in sandbox".to_string())
    }

    /// Closes beads issues with individual commits for each.
    fn close_beads_issues(&self, repo_path: &PathBuf, issues: &[PlanIssue]) {
        let beads = BeadsClient::new(repo_path);

        for issue in issues {
            // Update to in_progress first
            if let Err(e) = beads.update_status(&issue.beads_id, IssueStatus::InProgress) {
                tracing::warn!(
                    error = %e,
                    issue = %issue.beads_id,
                    "failed to update issue to in_progress"
                );
            }

            // Close the issue
            match beads.close(
                &issue.beads_id,
                Some("Completed as part of E2E test execution"),
            ) {
                Ok(()) => {
                    // Create a commit for this issue closure
                    let _ = commit_issue_change(
                        repo_path,
                        &issue.beads_id,
                        &format!("Close {}: {}", issue.beads_id, issue.subject),
                    );

                    tracing::info!(
                        beads_id = %issue.beads_id,
                        subject = %issue.subject,
                        "closed beads issue"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        issue = %issue.beads_id,
                        "failed to close issue"
                    );
                }
            }
        }

        // Sync beads at the end (may fail if no remote, which is OK)
        if let Err(e) = beads.sync() {
            tracing::debug!(error = %e, "beads sync warning (expected for ephemeral repos)");
        }
    }

    /// Validates the plan PR meets all requirements.
    /// Returns a list of validation errors (empty if all passed).
    fn validate_plan_pr(&self, cruise_result: &crate::cruise::result::CruiseResult) -> Vec<String> {
        let mut errors = Vec::new();

        // 1. Check that plan PR was created
        let plan_result = match &cruise_result.plan_result {
            Some(pr) => pr,
            None => {
                errors.push("No plan result - planning phase did not complete".to_string());
                return errors;
            }
        };

        if plan_result.pr_url.is_none() {
            errors.push("No plan PR was created".to_string());
        }

        // 2. Check that beads issues exist
        if plan_result.task_count == 0 {
            errors.push("No beads issues were created from the plan".to_string());
        }

        // 3. Check that plan was successful (implies plan file exists)
        if !plan_result.success {
            errors.push(format!(
                "Planning phase failed: {}",
                plan_result.error.as_deref().unwrap_or("unknown error")
            ));
        }

        // 4. Check that all 5 review phases are present
        if let Some(ref obs) = plan_result.observability {
            let review_count = obs.review_feedback.len();
            if review_count < 5 {
                errors.push(format!(
                    "Only {} of 5 required review phases completed",
                    review_count
                ));
            }

            // Check specific phases are present
            let expected_phases = ["Security", "TechnicalFeasibility", "TaskGranularity", "DependencyCompleteness", "GeneralPolish"];
            for (i, expected) in expected_phases.iter().enumerate() {
                let phase_present = obs.review_feedback.iter().any(|rf| {
                    rf.phase.as_deref() == Some(expected)
                });
                if !phase_present && i < review_count {
                    // Only report missing if we should have reached this phase
                    // (i.e., don't report phase 3 missing if we only got 2 reviews)
                    errors.push(format!("Missing review phase: {}", expected));
                }
            }
        } else {
            errors.push("No observability data - cannot verify review phases".to_string());
        }

        errors
    }
}
