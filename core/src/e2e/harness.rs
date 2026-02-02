//! Main E2E test orchestrator.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use crate::beads::{
    commit_issue_change, BeadsClient, CreateOptions, IssueStatus, IssueType, Priority,
};
use crate::cruise::planner::parse_plan_json;
use crate::cruise::task::TaskComplexity;
use crate::pr::{get_branch_commits, get_file_changes, PRManager};
use crate::runner::{ClaudeRunner, GeminiRunner, LLMRunner};
use crate::sandbox::WorktreeSandbox;
use crate::spawn::{SpawnConfig, SpawnResult, Spawner};

use super::fixture::{Fixture, RunnerType, WorkflowType};
use super::repo::EphemeralRepo;
use super::validator::{ValidationResult, Validator};

/// A beads issue created from the plan.
#[derive(Debug, Clone)]
struct PlanIssue {
    /// Beads issue ID (e.g., "bd-1").
    beads_id: String,
    /// Original task ID from plan (e.g., "CRUISE-001").
    #[allow(dead_code)]
    plan_task_id: String,
    /// Task subject/title.
    subject: String,
}

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
                };
            }
        };

        let repo_name = repo.full_name();
        tracing::info!(
            fixture = %fixture.name,
            repo = %repo_name,
            "running E2E fixture (full workflow)"
        );

        // Phase 0: Initialize beads for issue tracking
        if let Err(e) = self.init_beads(repo.path()) {
            tracing::warn!(error = %e, "failed to initialize beads, continuing without issue tracking");
        }

        // Phase 1: Planning
        let planning_prompt = fixture.planning_prompt.clone().unwrap_or_else(|| {
            self.create_planning_prompt(&fixture.prompt)
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

        // Create beads issues from plan output
        let plan_issues = if let Some(ref sandbox_path) = plan_sandbox_path {
            match self.extract_and_create_beads_issues(sandbox_path, repo.path()) {
                Ok(issues) => {
                    tracing::info!(issue_count = issues.len(), "created beads issues from plan");
                    issues
                }
                Err(e) => {
                    tracing::warn!(error = %e, "failed to create beads issues from plan");
                    Vec::new()
                }
            }
        } else {
            Vec::new()
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

        // Close beads issues with individual commits if successful
        if passed && !plan_issues.is_empty() {
            self.close_beads_issues(repo.path(), &plan_issues);
        }

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

Create a detailed implementation plan for the following task.

Output your plan in two parts:

1. First, create a markdown document with:
   - A header that includes: `[E2E Test Plan - Cruise-Control Module Active]`
   - Overview section explaining the approach
   - Risk Areas section listing potential issues

2. Then, output a JSON block with the tasks in this exact format:

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

Use CRUISE-XXX IDs. List dependencies in blocked_by using task IDs.

Task: {}"#,
            prompt
        )
    }

    /// Extracts plan from spawn output and creates beads issues.
    fn extract_and_create_beads_issues(
        &self,
        sandbox_path: &PathBuf,
        repo_path: &PathBuf,
    ) -> Result<Vec<PlanIssue>, String> {
        // Look for plan files in the sandbox
        let plan_content = self
            .find_and_read_plan(sandbox_path)
            .map_err(|e| format!("failed to find plan: {}", e))?;

        // Try to parse as CruisePlan
        let plan = match parse_plan_json(&plan_content) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(error = %e, "could not parse plan JSON, skipping beads issue creation");
                return Err(format!("could not parse plan: {}", e));
            }
        };

        // Create beads issues
        let beads = BeadsClient::new(repo_path);
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
                    tracing::info!(
                        beads_id = %result.id,
                        plan_id = %task.id,
                        subject = %task.subject,
                        "created beads issue"
                    );

                    id_mapping.insert(task.id.clone(), result.id.clone());

                    created_issues.push(PlanIssue {
                        beads_id: result.id,
                        plan_task_id: task.id.clone(),
                        subject: task.subject.clone(),
                    });
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        task_id = %task.id,
                        "failed to create beads issue"
                    );
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

        // Commit the beads issue creation
        if !created_issues.is_empty() {
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

        for dir in &plan_paths {
            if dir.exists() && dir.is_dir() {
                if let Ok(entries) = std::fs::read_dir(dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.extension().map(|e| e == "md").unwrap_or(false) {
                            if let Ok(content) = std::fs::read_to_string(&path) {
                                if content.contains("CRUISE-") || content.contains("```json") {
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
}
