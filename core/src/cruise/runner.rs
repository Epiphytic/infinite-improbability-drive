//! CruiseRunner: High-level orchestrator for cruise-control workflows.
//!
//! Provides two workflow modes:
//! - Simple: Single spawn for straightforward tasks
//! - Full: Plan → Approve → Execute with PR integration and beads issue tracking

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use crate::beads::{BeadsClient, CreateOptions, IssueStatus, IssueType, Priority, commit_issue_change};
use crate::error::{Error, Result};
use crate::pr::{get_branch_commits, get_file_changes, PRManager};
use crate::runner::{ClaudeRunner, GeminiRunner, LLMRunner};
use crate::sandbox::{SandboxManifest, SandboxProvider};
use crate::spawn::{SpawnConfig, SpawnResult, SpawnStatus, Spawner};
use crate::team::{CoordinationMode, SpawnTeamConfig};
use crate::team_orchestrator::{format_observability_markdown, SpawnTeamOrchestrator};

use super::config::CruiseConfig;
use super::planner::generate_pr_body as generate_plan_pr_body;
use super::result::{BuildResult, CruiseResult, PlanResult, TaskResult};
use super::task::{CruisePlan, CruiseTask, TaskStatus};

/// Sanitizes a string for use in filenames and branch names.
/// Converts to lowercase, replaces spaces with hyphens, removes special chars.
fn sanitize_for_filename(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| match c {
            ' ' | '_' => '-',
            c if c.is_alphanumeric() || c == '-' => c,
            _ => '-',
        })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .take(5) // Take first 5 words
        .collect::<Vec<_>>()
        .join("-")
        .chars()
        .take(40) // Limit total length
        .collect()
}

/// Workflow phase for branch naming.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowPhase {
    /// Planning phase: plan/feature-name-suffix
    Plan,
    /// Feature implementation: feat/feature-name-suffix
    Feature,
    /// Bug fix implementation: fix/bug-name-suffix
    Fix,
    /// Validation phase: validate/feature-name-suffix
    Validate,
}

impl WorkflowPhase {
    /// Returns the branch prefix for this phase.
    fn prefix(&self) -> &'static str {
        match self {
            WorkflowPhase::Plan => "plan",
            WorkflowPhase::Feature => "feat",
            WorkflowPhase::Fix => "fix",
            WorkflowPhase::Validate => "validate",
        }
    }
}

/// Generates a branch name for a workflow phase.
/// Format: {prefix}/{feature-name}-{short-uuid}
fn generate_branch_name(phase: WorkflowPhase, prompt: &str) -> String {
    let feature_name = sanitize_for_filename(prompt);
    let uuid = uuid::Uuid::new_v4();
    let short_uuid = &uuid.to_string()[..8];

    if feature_name.is_empty() {
        format!("{}/task-{}", phase.prefix(), short_uuid)
    } else {
        format!("{}/{}-{}", phase.prefix(), feature_name, short_uuid)
    }
}

/// Runner type for LLM selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RunnerType {
    #[default]
    Claude,
    Gemini,
}

impl RunnerType {
    /// Creates a boxed LLM runner for this type.
    pub fn create_runner(&self) -> Box<dyn LLMRunner> {
        match self {
            RunnerType::Claude => Box::new(ClaudeRunner::new()),
            RunnerType::Gemini => Box::new(GeminiRunner::new()),
        }
    }
}

/// A beads issue created from the plan.
#[derive(Debug, Clone)]
pub struct PlanIssue {
    /// Beads issue ID (e.g., "bd-1").
    pub beads_id: String,
    /// Original task ID from plan (e.g., "CRUISE-001").
    pub plan_task_id: String,
    /// Task subject/title.
    pub subject: String,
}

/// High-level orchestrator for cruise-control workflows.
///
/// Manages the full lifecycle of a cruise-control run:
/// - Simple workflow: Single spawn for straightforward tasks
/// - Full workflow: Plan → Approve → Execute with PR integration and beads tracking
pub struct CruiseRunner<P: SandboxProvider + Clone> {
    config: CruiseConfig,
    provider: P,
    logs_dir: PathBuf,
    /// Auto-approve PRs (useful for testing)
    auto_approve: bool,
    /// Use spawn-team ping-pong mode with Gemini reviews
    use_spawn_team: bool,
    /// Team coordination mode (PingPong or GitHub)
    team_mode: crate::team::CoordinationMode,
    /// Environment variables to pass to LLM processes
    env_vars: std::collections::HashMap<String, String>,
}

impl<P: SandboxProvider + Clone + 'static> CruiseRunner<P> {
    /// Creates a new CruiseRunner with the given sandbox provider.
    pub fn new(provider: P, logs_dir: PathBuf) -> Self {
        // Default env vars include FORK_JOIN_DISABLED to avoid conflicts
        let mut env_vars = std::collections::HashMap::new();
        env_vars.insert("FORK_JOIN_DISABLED".to_string(), "1".to_string());

        Self {
            config: CruiseConfig::default(),
            provider,
            logs_dir,
            auto_approve: false,
            use_spawn_team: false,
            team_mode: crate::team::CoordinationMode::default(),
            env_vars,
        }
    }

    /// Enables spawn-team mode with Gemini reviews.
    pub fn with_spawn_team(mut self, enabled: bool) -> Self {
        self.use_spawn_team = enabled;
        self
    }

    /// Sets the team coordination mode.
    pub fn with_team_mode(mut self, mode: crate::team::CoordinationMode) -> Self {
        self.team_mode = mode;
        self
    }

    /// Adds environment variables to pass to LLM processes.
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env_vars.insert(key.into(), value.into());
        self
    }

    /// Sets the cruise configuration.
    pub fn with_config(mut self, config: CruiseConfig) -> Self {
        self.config = config;
        self
    }

    /// Enables auto-approval for PRs (useful for testing).
    pub fn with_auto_approve(mut self, auto_approve: bool) -> Self {
        self.auto_approve = auto_approve;
        self
    }

    /// Returns the configuration.
    pub fn config(&self) -> &CruiseConfig {
        &self.config
    }

    /// Runs a simple workflow: single spawn with PR creation.
    ///
    /// This is the basic workflow for straightforward tasks that don't
    /// need planning or approval steps.
    pub async fn run_simple(
        &self,
        prompt: &str,
        runner_type: RunnerType,
        timeout: Duration,
    ) -> Result<CruiseResult> {
        let start = Instant::now();

        let spawner = Spawner::new(self.provider.clone(), self.logs_dir.clone());
        let config = SpawnConfig::new(prompt).with_total_timeout(timeout);
        let manifest = SandboxManifest::default();
        let runner = runner_type.create_runner();

        let spawn_result = spawner.spawn(config, manifest, runner).await?;
        let success = spawn_result.status == SpawnStatus::Success;

        Ok(CruiseResult {
            success,
            prompt: prompt.to_string(),
            plan_result: None,
            build_result: Some(BuildResult {
                success,
                summary: spawn_result.summary.clone(),
                task_results: vec![],
                max_parallelism: 1,
                duration: spawn_result.duration,
                completed_count: if success { 1 } else { 0 },
                blocked_count: 0,
                observability: None, // Simple workflow doesn't use spawn-team
            }),
            validation_result: None,
            total_duration: start.elapsed(),
            summary: spawn_result.summary,
        })
    }

    /// Runs a full workflow: Plan → Approve → Execute with beads integration.
    ///
    /// This workflow:
    /// 1. Initializes beads in the repository
    /// 2. Runs a planning spawn to generate an implementation plan
    /// 3. Creates beads issues from the plan
    /// 4. Creates a PR for the plan with enhanced formatting
    /// 5. Waits for approval (or auto-approves in test mode)
    /// 6. Runs an execution spawn to implement the plan
    /// 7. Closes beads issues with individual commits
    /// 8. Creates a PR for the implementation with commits and file stats
    pub async fn run_full(
        &self,
        prompt: &str,
        runner_type: RunnerType,
        timeout: Duration,
        repo_path: &PathBuf,
    ) -> Result<CruiseResult> {
        let start = Instant::now();

        // Phase 0: Initialize beads
        self.init_beads(repo_path)?;

        // Phase 1: Planning
        let planning_prompt = self.create_planning_prompt(prompt);
        let (plan_result, plan_issues) = self
            .run_planning_phase(&planning_prompt, runner_type, timeout, repo_path, prompt)
            .await?;

        if !plan_result.success {
            return Ok(CruiseResult {
                success: false,
                prompt: prompt.to_string(),
                plan_result: Some(plan_result),
                build_result: None,
                validation_result: None,
                total_duration: start.elapsed(),
                summary: "Planning phase failed".to_string(),
            });
        }

        tracing::info!(
            issues_created = plan_issues.len(),
            "created beads issues from plan"
        );

        // Phase 2: Wait for approval (or auto-approve)
        if let Some(ref pr_url) = plan_result.pr_url {
            if self.auto_approve {
                self.auto_approve_pr(pr_url)?;

                // Pull the merged plan PR changes to local repo
                // This ensures the execution sandbox includes the plan files
                tracing::info!(repo_path = ?repo_path, "pulling merged plan changes to local repo");

                // First fetch to ensure we have latest refs
                let fetch_output = Command::new("git")
                    .current_dir(repo_path)
                    .args(["fetch", "origin", "main"])
                    .output()
                    .map_err(|e| Error::Git(format!("failed to fetch: {}", e)))?;

                if !fetch_output.status.success() {
                    tracing::warn!(
                        error = %String::from_utf8_lossy(&fetch_output.stderr),
                        "git fetch failed"
                    );
                }

                // Reset to origin/main to ensure we have the merged changes
                let reset_output = Command::new("git")
                    .current_dir(repo_path)
                    .args(["reset", "--hard", "origin/main"])
                    .output()
                    .map_err(|e| Error::Git(format!("failed to reset: {}", e)))?;

                if !reset_output.status.success() {
                    tracing::warn!(
                        error = %String::from_utf8_lossy(&reset_output.stderr),
                        "git reset failed"
                    );
                } else {
                    tracing::info!("successfully synced local repo with merged plan");
                }

                // Verify the plan files exist
                let plan_dir = repo_path.join("docs").join("plans");
                if plan_dir.exists() {
                    let plan_files: Vec<_> = std::fs::read_dir(&plan_dir)
                        .map(|entries| entries.filter_map(|e| e.ok()).collect())
                        .unwrap_or_default();
                    tracing::info!(
                        plan_dir = ?plan_dir,
                        plan_files = plan_files.len(),
                        "verified plan files exist after sync"
                    );
                } else {
                    tracing::warn!(plan_dir = ?plan_dir, "plan directory does not exist after sync");
                }
            } else {
                // In production, would poll for approval
                tracing::info!(pr_url = %pr_url, "plan PR created, waiting for approval");
            }
        }

        // Phase 3: Execution
        let build_result = self
            .run_execution_phase(prompt, runner_type, timeout, repo_path, &plan_issues)
            .await?;

        let success = build_result.success;

        // If auto-approve is enabled, pull merged changes to local repo
        // This ensures validation can see the files created by merged PRs
        if self.auto_approve && success {
            tracing::info!(repo_path = ?repo_path, "pulling merged changes to local repo");
            let pull_output = Command::new("git")
                .current_dir(repo_path)
                .args(["pull", "--rebase=false", "origin", "main"])
                .output();

            match pull_output {
                Ok(output) if output.status.success() => {
                    tracing::info!("successfully pulled merged changes");
                }
                Ok(output) => {
                    tracing::warn!(
                        error = %String::from_utf8_lossy(&output.stderr),
                        "git pull completed with warnings"
                    );
                }
                Err(e) => {
                    tracing::warn!(error = %e, "failed to pull merged changes");
                }
            }
        }

        // Use build_result summary for detailed error message
        let summary = if success {
            "Full workflow completed successfully".to_string()
        } else {
            format!("Execution phase failed: {}", build_result.summary)
        };

        Ok(CruiseResult {
            success,
            prompt: prompt.to_string(),
            plan_result: Some(plan_result),
            build_result: Some(build_result),
            validation_result: None,
            total_duration: start.elapsed(),
            summary,
        })
    }

    /// Initializes beads in the repository if not already initialized.
    fn init_beads(&self, repo_path: &PathBuf) -> Result<()> {
        let beads = BeadsClient::new(repo_path);

        if beads.is_initialized() {
            tracing::info!(path = ?repo_path, "beads already initialized");
            return Ok(());
        }

        beads.init()?;

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
        // Generate plan filename with date and sanitized feature name
        let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let feature_name = sanitize_for_filename(prompt);
        let plan_path = format!("docs/plans/{}-cruise-control-{}.md", date, feature_name);

        format!(
            r#"**THIS IS A PLANNING-ONLY PHASE. DO NOT IMPLEMENT ANYTHING.**

Create a detailed implementation plan for the following task and SAVE IT TO A FILE.

**CRITICAL INSTRUCTIONS:**
1. First, create the `docs/plans/` directory if it doesn't exist
2. You MUST create a file at: `{plan_path}`
3. You MUST NOT create any implementation files (no Cargo.toml, no src/, no tests/)
4. You MUST NOT implement the task - only plan it

The PLAN.md file must contain:

1. A header with the plan title
2. An Overview section explaining the approach
3. A Risk Areas section listing potential issues
4. A JSON block with the tasks and spawn instances in this exact format:

```json
{{
  "title": "Short plan title",
  "overview": "Brief overview",
  "spawn_instances": [
    {{
      "id": "SPAWN-001",
      "name": "Core Infrastructure Setup",
      "use_spawn_team": true,
      "cli_params": "claude --model sonnet --allowedTools Read,Write,Edit,Bash,Glob,Grep --timeout 300",
      "permissions": ["Read", "Write", "Edit", "Bash", "Glob", "Grep"],
      "task_ids": ["CRUISE-001", "CRUISE-002"]
    }},
    {{
      "id": "SPAWN-002",
      "name": "Frontend Implementation",
      "use_spawn_team": false,
      "cli_params": "claude --model haiku --allowedTools Read,Write,Edit --timeout 180",
      "permissions": ["Read", "Write", "Edit"],
      "task_ids": ["CRUISE-003"]
    }}
  ],
  "tasks": [
    {{
      "id": "CRUISE-001",
      "subject": "Task title",
      "description": "What needs to be done",
      "blocked_by": [],
      "complexity": "low|medium|high",
      "acceptance_criteria": ["Criterion 1", "Criterion 2"],
      "permissions": ["Read", "Write", "Edit", "Bash"],
      "cli_params": "claude --model sonnet --allowedTools Read,Write,Edit,Bash",
      "spawn_instance": "SPAWN-001"
    }}
  ],
  "risks": ["Risk 1", "Risk 2"]
}}
```

**SECURITY-CRITICAL REQUIREMENTS:**

For each task, you MUST specify:
- `permissions`: Array of tool permissions required (e.g., ["Read", "Write", "Edit", "Bash", "Glob", "Grep"])
- `cli_params`: Exact CLI command parameters to launch the LLM with appropriate restrictions
- `spawn_instance`: Which spawn/spawn-team instance this task belongs to

For each spawn_instance, you MUST specify:
- `id`: Unique identifier (SPAWN-XXX format)
- `name`: Human-readable description
- `use_spawn_team`: Whether to use ping-pong review mode (true for complex/security-sensitive tasks)
- `cli_params`: Full CLI command to launch this instance
- `permissions`: Union of all permissions needed by tasks in this instance
- `task_ids`: List of task IDs that will be executed in this instance

Group related tasks into spawn instances based on:
1. Dependencies (tasks that must execute together)
2. Security boundaries (isolate tasks with different permission levels)
3. Efficiency (minimize spawn overhead by grouping small tasks)

Use CRUISE-XXX IDs for tasks and SPAWN-XXX IDs for instances.

**REMEMBER: ONLY create the plan file at `{plan_path}`. Do NOT implement the code.**

Task to plan (do NOT implement): {prompt}"#,
            plan_path = plan_path,
            prompt = prompt
        )
    }

    /// Creates an execution-specific prompt that references the plan.
    ///
    /// This tells the LLM to implement the plan, not create another plan.
    fn create_execution_prompt(&self, original_prompt: &str, repo_path: &PathBuf) -> String {
        // Find the plan file
        let plans_dir = repo_path.join("docs").join("plans");
        let plan_file = if plans_dir.exists() {
            std::fs::read_dir(&plans_dir)
                .ok()
                .and_then(|entries| {
                    entries
                        .filter_map(|e| e.ok())
                        .filter(|e| e.path().extension().map(|ext| ext == "md").unwrap_or(false))
                        .max_by_key(|e| e.metadata().ok().and_then(|m| m.modified().ok()))
                })
                .map(|e| e.path())
        } else {
            None
        };

        let plan_reference = match plan_file {
            Some(path) => format!(
                "\n\n**IMPLEMENTATION PLAN:** Read and follow the plan at `{}`",
                path.strip_prefix(repo_path).unwrap_or(&path).display()
            ),
            None => String::new(),
        };

        format!(
            r#"**THIS IS THE IMPLEMENTATION PHASE. IMPLEMENT THE CODE NOW.**

You must implement the following task. There is a detailed plan available in the repository.
{}

**CRITICAL INSTRUCTIONS:**
1. Read the implementation plan in `docs/plans/` if it exists
2. Create all necessary files: source code, configuration, tests
3. Do NOT create another planning document - IMPLEMENT the actual code
4. Start with the foundation (package/project setup) and work through the plan
5. Create working code, not just stubs or placeholders
6. Commit your changes frequently

**Original Task:**
{}"#,
            plan_reference, original_prompt
        )
    }

    /// Runs the planning phase, creates beads issues, and creates a plan PR.
    ///
    /// When spawn-team is enabled, uses ping-pong mode with Gemini reviews.
    async fn run_planning_phase(
        &self,
        planning_prompt: &str,
        _runner_type: RunnerType, // Always uses Claude for planning, Gemini for review
        timeout: Duration,
        repo_path: &PathBuf,
        original_prompt: &str,
    ) -> Result<(PlanResult, Vec<PlanIssue>)> {
        let start = Instant::now();

        if self.use_spawn_team {
            // Use SpawnTeamOrchestrator for planning with Gemini reviews
            self.run_planning_phase_with_team(planning_prompt, timeout, repo_path, original_prompt)
                .await
        } else {
            // Simple single-spawn planning
            self.run_planning_phase_simple(planning_prompt, timeout, repo_path, original_prompt)
                .await
                .map(|(result, issues)| {
                    let elapsed = start.elapsed();
                    (
                        PlanResult {
                            duration: elapsed,
                            ..result
                        },
                        issues,
                    )
                })
        }
    }

    /// Runs planning phase with spawn-team ping-pong mode.
    ///
    /// Uses Claude to create the plan and Gemini to review it iteratively.
    async fn run_planning_phase_with_team(
        &self,
        planning_prompt: &str,
        timeout: Duration,
        repo_path: &PathBuf,
        original_prompt: &str,
    ) -> Result<(PlanResult, Vec<PlanIssue>)> {
        let start = Instant::now();

        // Generate a consistent branch name for the planning phase
        let branch_name = generate_branch_name(WorkflowPhase::Plan, original_prompt);

        tracing::info!(
            branch = %branch_name,
            "starting planning phase"
        );

        // Configure spawn-team for planning
        let team_config = SpawnTeamConfig {
            mode: self.team_mode,
            max_iterations: self.config.planning.ping_pong_iterations,
            primary_llm: "claude-code".to_string(),
            reviewer_llm: "gemini-cli".to_string(),
        };

        let mut orchestrator = SpawnTeamOrchestrator::new(
            self.provider.clone(),
            self.logs_dir.join("planning-team"),
        )
        .with_config(team_config);

        // Run the orchestrator with the explicit branch name
        // All iterations use the same branch for consistency
        let team_result = orchestrator
            .run_with_branch(planning_prompt, timeout, repo_path, Some(&branch_name))
            .await?;

        // Get observability data from orchestrator
        let observability = orchestrator.take_observability();

        tracing::info!(
            iterations = team_result.iterations,
            reviews = team_result.reviews.len(),
            success = team_result.success,
            sandbox_path = ?observability.sandbox_path,
            "spawn-team planning completed"
        );

        // Get the actual sandbox path where the work was done
        let sandbox_path = observability.sandbox_path.clone();

        let mut plan_issues = Vec::new();
        let mut task_count = 0;
        let mut parsed_plan: Option<CruisePlan> = None;

        // Create beads issues from plan if successful
        if team_result.success {
            if let Some(ref sandbox) = sandbox_path {
                match self.extract_and_create_beads_issues(sandbox, repo_path) {
                    Ok((issues, plan)) => {
                        task_count = issues.len();
                        plan_issues = issues;
                        parsed_plan = Some(plan);
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "failed to create beads issues from plan");
                    }
                }
            }
        }

        // Get PR URL from observability (created on first commit by SpawnTeamOrchestrator)
        // If not present (e.g., in Sequential mode), create one now
        let pr_url = observability.pr_url.clone().or_else(|| {
            if team_result.success {
                if let Some(ref sandbox) = sandbox_path {
                    // Include observability in the PR body
                    self.create_plan_pr_with_observability(
                        sandbox,
                        repo_path,
                        original_prompt,
                        parsed_plan.as_ref(),
                        &observability,
                        team_result.iterations,
                    )
                } else {
                    None
                }
            } else {
                None
            }
        });

        let plan_result = PlanResult {
            success: team_result.success,
            iterations: team_result.iterations,
            task_count,
            pr_url,
            duration: start.elapsed(),
            plan_file: None,
            error: if team_result.success {
                None
            } else {
                Some(team_result.summary)
            },
            observability: Some(observability),
        };

        Ok((plan_result, plan_issues))
    }

    /// Simple planning phase without spawn-team (single spawn).
    async fn run_planning_phase_simple(
        &self,
        planning_prompt: &str,
        timeout: Duration,
        repo_path: &PathBuf,
        original_prompt: &str,
    ) -> Result<(PlanResult, Vec<PlanIssue>)> {
        let start = Instant::now();

        // Generate a consistent branch name for the planning phase
        let branch_name = generate_branch_name(WorkflowPhase::Plan, original_prompt);

        tracing::info!(
            branch = %branch_name,
            "starting simple planning phase"
        );

        let spawner = Spawner::new(self.provider.clone(), self.logs_dir.join("planning"));
        let config = SpawnConfig::new(planning_prompt).with_total_timeout(timeout);
        let manifest = SandboxManifest::default();
        let runner = ClaudeRunner::new();

        let spawn_result = spawner.spawn_with_branch(config, manifest, Box::new(runner), Some(&branch_name)).await?;
        let success = spawn_result.status == SpawnStatus::Success;

        let mut plan_issues = Vec::new();
        let mut task_count = 0;
        let mut parsed_plan: Option<CruisePlan> = None;

        // Create beads issues from plan if successful
        if success {
            if let Some(ref sandbox_path) = spawn_result.sandbox_path {
                match self.extract_and_create_beads_issues(sandbox_path, repo_path) {
                    Ok((issues, plan)) => {
                        task_count = issues.len();
                        plan_issues = issues;
                        parsed_plan = Some(plan);
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "failed to create beads issues from plan");
                    }
                }
            }
        }

        // Create plan PR if successful
        let pr_url = if success {
            if let Some(ref sandbox_path) = spawn_result.sandbox_path {
                self.create_plan_pr(sandbox_path, repo_path, original_prompt, &spawn_result, parsed_plan.as_ref())
            } else {
                None
            }
        } else {
            None
        };

        let plan_result = PlanResult {
            success,
            iterations: 1,
            task_count,
            pr_url,
            duration: start.elapsed(),
            plan_file: None,
            error: if success {
                None
            } else {
                Some(spawn_result.summary)
            },
            observability: None,
        };

        Ok((plan_result, plan_issues))
    }

    /// Creates a plan PR with observability data included.
    fn create_plan_pr_with_observability(
        &self,
        sandbox_path: &PathBuf,
        repo_path: &PathBuf,
        prompt: &str,
        parsed_plan: Option<&CruisePlan>,
        observability: &crate::team_orchestrator::SpawnObservability,
        iterations: u32,
    ) -> Option<String> {
        let pr_manager = PRManager::new(repo_path.clone());

        let task_count = parsed_plan.map(|p| p.tasks.len()).unwrap_or(0);

        // Commit changes
        let commit_message = format!(
            "Plan: {}\n\nCreated {} tasks after {} iteration(s)",
            truncate_string(prompt, 50),
            task_count,
            iterations
        );

        if pr_manager
            .commit_changes(sandbox_path, &commit_message)
            .ok()?
            .is_none()
        {
            tracing::info!("no plan changes to commit");
            return None;
        }

        // Get branch name
        let branch_name = sandbox_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("plan");

        // Push branch
        if pr_manager.push_branch(sandbox_path, branch_name).is_err() {
            return None;
        }

        // Use parsed plan if available, otherwise create empty plan
        let plan = match parsed_plan {
            Some(p) => p.clone(),
            None => {
                let mut empty = CruisePlan::new(prompt);
                empty.title = "Implementation Plan".to_string();
                empty
            }
        };
        let mut pr_body = generate_plan_pr_body(&plan, prompt, iterations);

        // Append observability section
        pr_body.push_str("\n\n---\n\n");
        pr_body.push_str(&format_observability_markdown(observability));

        // Create PR
        pr_manager
            .create_pr(
                &format!("Plan: {}", truncate_string(prompt, 50)),
                &pr_body,
                branch_name,
                "main",
            )
            .ok()
            .map(|pr| pr.url)
    }

    /// Extracts plan from spawn output and creates beads issues.
    /// Returns both the created issues and the parsed plan.
    fn extract_and_create_beads_issues(
        &self,
        sandbox_path: &PathBuf,
        repo_path: &PathBuf,
    ) -> Result<(Vec<PlanIssue>, CruisePlan)> {
        // Look for plan files in the sandbox
        let plan_content = self.find_and_read_plan(sandbox_path)?;

        // Try to parse as CruisePlan
        let plan = match super::planner::parse_plan_json(&plan_content) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(error = %e, "could not parse plan JSON, creating single task");
                // Create a single task for the whole prompt
                let mut fallback_plan = CruisePlan::new("");
                fallback_plan.title = "Implementation".to_string();
                fallback_plan.tasks.push(CruiseTask::new("CRUISE-001", "Implement feature"));
                fallback_plan
            }
        };

        // Create beads issues
        let beads = BeadsClient::new(repo_path);
        let mut created_issues = Vec::new();
        let mut id_mapping: std::collections::HashMap<String, String> = std::collections::HashMap::new();

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
                    super::task::TaskComplexity::Low => Priority::Low,
                    super::task::TaskComplexity::Medium => Priority::Medium,
                    super::task::TaskComplexity::High => Priority::High,
                },
                issue_type: IssueType::Task,
                labels: vec!["cruise-control".to_string()],
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

        Ok((created_issues, plan))
    }

    /// Finds and reads plan content from the sandbox.
    fn find_and_read_plan(&self, sandbox_path: &PathBuf) -> Result<String> {
        // Look for common plan file locations
        let plan_paths = [
            sandbox_path.join("docs/plans"),
            sandbox_path.join("docs"),
            sandbox_path.join("plan"),
            sandbox_path.clone(),
        ];

        for dir in &plan_paths {
            if dir.exists() {
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

        // If no plan file found, return empty which will trigger fallback
        Err(Error::Cruise("No plan file found in sandbox".to_string()))
    }

    /// Creates a PR for the plan with enhanced formatting.
    fn create_plan_pr(
        &self,
        sandbox_path: &PathBuf,
        repo_path: &PathBuf,
        prompt: &str,
        spawn_result: &SpawnResult,
        parsed_plan: Option<&CruisePlan>,
    ) -> Option<String> {
        let pr_manager = PRManager::new(repo_path.clone());

        let task_count = parsed_plan.map(|p| p.tasks.len()).unwrap_or(0);

        // Commit changes
        let commit_message = format!(
            "Plan: {}\n\nCreated {} tasks\nSpawn ID: {}",
            truncate_string(prompt, 50),
            task_count,
            spawn_result.spawn_id
        );

        if pr_manager
            .commit_changes(sandbox_path, &commit_message)
            .ok()?
            .is_none()
        {
            tracing::info!("no plan changes to commit");
            return None;
        }

        // Get branch name
        let branch_name = sandbox_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("plan");

        // Push branch
        if pr_manager.push_branch(sandbox_path, branch_name).is_err() {
            return None;
        }

        // Use parsed plan if available, otherwise create empty plan
        let plan = match parsed_plan {
            Some(p) => p.clone(),
            None => {
                let mut empty = CruisePlan::new(prompt);
                empty.title = "Implementation Plan".to_string();
                empty
            }
        };
        let pr_body = generate_plan_pr_body(&plan, prompt, 1);

        // Create PR
        pr_manager
            .create_pr(
                &format!("Plan: {}", truncate_string(prompt, 50)),
                &pr_body,
                branch_name,
                "main",
            )
            .ok()
            .map(|pr| pr.url)
    }

    /// Runs the execution phase, closes beads issues, and creates an implementation PR.
    ///
    /// When spawn-team is enabled, uses ping-pong mode with Gemini reviews.
    async fn run_execution_phase(
        &self,
        prompt: &str,
        _runner_type: RunnerType, // Always uses Claude for execution, Gemini for review
        timeout: Duration,
        repo_path: &PathBuf,
        plan_issues: &[PlanIssue],
    ) -> Result<BuildResult> {
        if self.use_spawn_team {
            self.run_execution_phase_with_team(prompt, timeout, repo_path, plan_issues)
                .await
        } else {
            self.run_execution_phase_simple(prompt, timeout, repo_path, plan_issues)
                .await
        }
    }

    /// Runs execution phase with spawn-team ping-pong mode.
    ///
    /// Uses Claude to implement and Gemini to review iteratively.
    async fn run_execution_phase_with_team(
        &self,
        prompt: &str,
        timeout: Duration,
        repo_path: &PathBuf,
        plan_issues: &[PlanIssue],
    ) -> Result<BuildResult> {
        let start = Instant::now();

        // Generate a consistent branch name for the feature implementation phase
        let branch_name = generate_branch_name(WorkflowPhase::Feature, prompt);

        tracing::info!(
            branch = %branch_name,
            "starting execution phase"
        );

        // Configure spawn-team for execution
        let team_config = SpawnTeamConfig {
            mode: self.team_mode,
            max_iterations: self.config.planning.ping_pong_iterations, // Reuse planning iterations
            primary_llm: "claude-code".to_string(),
            reviewer_llm: "gemini-cli".to_string(),
        };

        let mut orchestrator = SpawnTeamOrchestrator::new(
            self.provider.clone(),
            self.logs_dir.join("execution-team"),
        )
        .with_config(team_config);

        // Create an execution-specific prompt that references the plan
        let execution_prompt = self.create_execution_prompt(prompt, repo_path);

        // Run the orchestrator with the explicit branch name
        let team_result = orchestrator.run_with_branch(&execution_prompt, timeout, repo_path, Some(&branch_name)).await?;

        // Get observability data from orchestrator
        let observability = orchestrator.take_observability();

        tracing::info!(
            iterations = team_result.iterations,
            reviews = team_result.reviews.len(),
            success = team_result.success,
            sandbox_path = ?observability.sandbox_path,
            "spawn-team execution completed"
        );

        // Get the actual sandbox path where the work was done
        let sandbox_path = observability.sandbox_path.clone();

        // Close beads issues and create individual commits
        let task_results = if team_result.success {
            self.close_beads_issues(repo_path, plan_issues)
        } else {
            vec![]
        };

        let completed_count = task_results
            .iter()
            .filter(|r| r.status == TaskStatus::Completed)
            .count();

        // Create implementation PR for partial work (even on timeout)
        // This allows partial work to be reviewed and merged instead of being lost
        let mut impl_pr_url = None;
        if let Some(ref sandbox) = sandbox_path {
            impl_pr_url = self.create_implementation_pr_with_observability(
                sandbox,
                repo_path,
                prompt,
                &observability,
                team_result.iterations,
            );

            // Auto-merge the implementation PR if auto_approve is enabled
            // This includes partial work from timeouts so E2E tests can validate content
            if self.auto_approve {
                if let Some(ref pr_url) = impl_pr_url {
                    if let Err(e) = self.auto_approve_pr(pr_url) {
                        tracing::warn!(error = %e, "failed to auto-merge implementation PR");
                    } else {
                        // Sync local repo with merged changes for validation
                        self.sync_with_remote(repo_path);
                    }
                }
            }
        }

        Ok(BuildResult {
            success: team_result.success,
            summary: team_result.summary.clone(),
            task_results,
            max_parallelism: 1,
            duration: start.elapsed(),
            completed_count,
            blocked_count: 0,
            observability: Some(observability),
        })
    }

    /// Simple execution phase without spawn-team (single spawn).
    async fn run_execution_phase_simple(
        &self,
        prompt: &str,
        timeout: Duration,
        repo_path: &PathBuf,
        plan_issues: &[PlanIssue],
    ) -> Result<BuildResult> {
        let start = Instant::now();

        // Generate a consistent branch name for the feature implementation phase
        let branch_name = generate_branch_name(WorkflowPhase::Feature, prompt);

        tracing::info!(
            branch = %branch_name,
            "starting simple execution phase"
        );

        let spawner = Spawner::new(self.provider.clone(), self.logs_dir.join("execution"));
        let config = SpawnConfig::new(prompt).with_total_timeout(timeout);
        let manifest = SandboxManifest::default();
        let runner = ClaudeRunner::new();

        let spawn_result = spawner.spawn_with_branch(config, manifest, Box::new(runner), Some(&branch_name)).await?;
        let success = spawn_result.status == SpawnStatus::Success;

        // Close beads issues and create individual commits
        let task_results = if success {
            self.close_beads_issues(repo_path, plan_issues)
        } else {
            vec![]
        };

        let completed_count = task_results
            .iter()
            .filter(|r| r.status == TaskStatus::Completed)
            .count();

        // Create implementation PR if successful
        let mut impl_pr_url = None;
        if success {
            if let Some(ref sandbox_path) = spawn_result.sandbox_path {
                impl_pr_url = self.create_implementation_pr(sandbox_path, repo_path, prompt, &spawn_result);

                // Auto-merge the implementation PR if auto_approve is enabled
                if self.auto_approve {
                    if let Some(ref pr_url) = impl_pr_url {
                        if let Err(e) = self.auto_approve_pr(pr_url) {
                            tracing::warn!(error = %e, "failed to auto-merge implementation PR");
                        } else {
                            // Sync local repo with merged changes for validation
                            self.sync_with_remote(repo_path);
                        }
                    }
                }
            }
        }

        Ok(BuildResult {
            success,
            summary: spawn_result.summary.clone(),
            task_results,
            max_parallelism: 1,
            duration: start.elapsed(),
            completed_count,
            blocked_count: 0,
            observability: None,
        })
    }

    /// Creates an implementation PR with observability data included.
    fn create_implementation_pr_with_observability(
        &self,
        sandbox_path: &PathBuf,
        repo_path: &PathBuf,
        prompt: &str,
        observability: &crate::team_orchestrator::SpawnObservability,
        iterations: u32,
    ) -> Option<String> {
        let pr_manager = PRManager::new(repo_path.clone());

        // Get the actual git branch name (not sandbox directory name which is sanitized)
        let branch_output = Command::new("git")
            .current_dir(sandbox_path)
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .output()
            .ok()?;

        let branch_name = String::from_utf8_lossy(&branch_output.stdout)
            .trim()
            .to_string();

        if branch_name.is_empty() || branch_name == "HEAD" {
            tracing::warn!("could not determine branch name from sandbox");
            return None;
        }

        tracing::info!(branch = %branch_name, "creating implementation PR");

        // Try to commit any remaining changes (orchestrator may have already committed)
        let commit_message = format!(
            "Implement: {}\n\nCompleted after {} iteration(s)",
            truncate_string(prompt, 50),
            iterations
        );

        // Commit is optional - orchestrator may have already committed
        let _ = pr_manager.commit_changes(sandbox_path, &commit_message);

        // Push branch (may already be pushed, but ensure it's up to date)
        if pr_manager.push_branch(sandbox_path, &branch_name).is_err() {
            tracing::warn!("push_branch failed, checking if branch exists remotely");
        }

        // Check if we have any commits on this branch relative to main
        // If not, there's nothing to PR
        let has_commits = Command::new("git")
            .current_dir(sandbox_path)
            .args(["rev-list", "--count", &format!("main..{}", &branch_name)])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| s.trim().parse::<u64>().ok())
            .unwrap_or(0);

        if has_commits == 0 {
            tracing::info!("no commits on branch relative to main, skipping PR creation");
            return None;
        }

        // Get commits and file changes for enhanced PR body
        let commits = get_branch_commits(sandbox_path, &branch_name, "main").unwrap_or_default();
        let files_changed =
            get_file_changes(sandbox_path, &branch_name, "main").unwrap_or_default();

        // Generate enhanced PR body
        let mut pr_body = pr_manager.generate_enhanced_pr_body(
            prompt,
            &format!("Implementation completed after {} iteration(s)", iterations),
            &commits,
            &files_changed,
            &format!("spawn-team-{}-iterations", iterations),
        );

        // Append observability section
        pr_body.push_str("\n\n---\n\n");
        pr_body.push_str(&format_observability_markdown(observability));

        // Create PR
        pr_manager
            .create_pr(
                &format!("Implement: {}", truncate_string(prompt, 50)),
                &pr_body,
                &branch_name,
                "main",
            )
            .ok()
            .map(|pr| pr.url)
    }

    /// Closes beads issues with individual commits for each.
    fn close_beads_issues(&self, repo_path: &PathBuf, issues: &[PlanIssue]) -> Vec<TaskResult> {
        let beads = BeadsClient::new(repo_path);
        let mut results = Vec::new();

        for issue in issues {
            let start = Instant::now();

            // Update to in_progress first
            if let Err(e) = beads.update_status(&issue.beads_id, IssueStatus::InProgress) {
                tracing::warn!(
                    error = %e,
                    issue = %issue.beads_id,
                    "failed to update issue to in_progress"
                );
            }

            // Close the issue
            let close_result = beads.close(
                &issue.beads_id,
                Some(&format!("Completed as part of cruise-control execution")),
            );

            let (status, error) = match close_result {
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

                    (TaskStatus::Completed, None)
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        issue = %issue.beads_id,
                        "failed to close issue"
                    );
                    (TaskStatus::Blocked, Some(e.to_string()))
                }
            };

            results.push(TaskResult {
                task_id: issue.plan_task_id.clone(),
                status,
                pr_url: None,
                duration: start.elapsed(),
                error,
            });
        }

        // Sync beads at the end
        if let Err(e) = beads.sync() {
            tracing::warn!(error = %e, "failed to sync beads");
        }

        results
    }

    /// Creates an implementation PR with enhanced formatting (commits, file stats).
    fn create_implementation_pr(
        &self,
        sandbox_path: &PathBuf,
        repo_path: &PathBuf,
        prompt: &str,
        spawn_result: &SpawnResult,
    ) -> Option<String> {
        let pr_manager = PRManager::new(repo_path.clone());

        // Commit changes
        let commit_message = format!(
            "Implement: {}\n\nSpawn ID: {}",
            truncate_string(prompt, 50),
            spawn_result.spawn_id
        );

        if pr_manager
            .commit_changes(sandbox_path, &commit_message)
            .ok()?
            .is_none()
        {
            tracing::info!("no implementation changes to commit");
            return None;
        }

        // Get branch name
        let branch_name = sandbox_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("implementation");

        // Push branch
        if pr_manager.push_branch(sandbox_path, branch_name).is_err() {
            return None;
        }

        // Get commits and file changes for enhanced PR body
        let commits = get_branch_commits(sandbox_path, branch_name, "main").unwrap_or_default();
        let files_changed =
            get_file_changes(sandbox_path, branch_name, "main").unwrap_or_default();

        // Generate enhanced PR body
        let pr_body = pr_manager.generate_enhanced_pr_body(
            prompt,
            "Implementation completed successfully",
            &commits,
            &files_changed,
            &spawn_result.spawn_id,
        );

        // Create PR
        pr_manager
            .create_pr(
                &format!("Implement: {}", truncate_string(prompt, 50)),
                &pr_body,
                branch_name,
                "main",
            )
            .ok()
            .map(|pr| pr.url)
    }

    /// Auto-approves and merges a PR (for testing).
    fn auto_approve_pr(&self, pr_url: &str) -> Result<()> {
        // Extract PR number from URL
        let pr_number = pr_url
            .split('/')
            .last()
            .ok_or_else(|| Error::Git("Invalid PR URL".to_string()))?;

        // Extract repo from URL
        let parts: Vec<&str> = pr_url.split('/').collect();
        if parts.len() < 5 {
            return Err(Error::Git("Invalid PR URL format".to_string()));
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
                "Auto-approved by CruiseRunner",
            ])
            .output()
            .map_err(|e| Error::Git(format!("Failed to run gh pr review: {}", e)))?;

        if !approve_output.status.success() {
            tracing::warn!(
                error = %String::from_utf8_lossy(&approve_output.stderr),
                "PR approval failed (may already be approved)"
            );
        }

        // Merge the PR (use --admin to bypass branch protection and failing checks)
        // This is needed for E2E tests where CI checks might fail initially
        let merge_output = Command::new("gh")
            .args([
                "pr",
                "merge",
                pr_number,
                "--repo",
                &repo,
                "--merge",
                "--delete-branch",
                "--admin", // Bypass branch protection and required checks
            ])
            .output()
            .map_err(|e| Error::Git(format!("Failed to run gh pr merge: {}", e)))?;

        if !merge_output.status.success() {
            return Err(Error::Git(format!(
                "PR merge failed: {}",
                String::from_utf8_lossy(&merge_output.stderr)
            )));
        }

        tracing::info!(pr_number = %pr_number, "auto-merged PR");
        Ok(())
    }

    /// Syncs local repo with remote after PR merge.
    ///
    /// This ensures validation runs against the merged content.
    fn sync_with_remote(&self, repo_path: &PathBuf) {
        tracing::info!(repo_path = ?repo_path, "syncing local repo with merged changes");

        // Fetch latest from origin
        let fetch_output = Command::new("git")
            .current_dir(repo_path)
            .args(["fetch", "origin", "main"])
            .output();

        if let Ok(output) = fetch_output {
            if !output.status.success() {
                tracing::warn!(
                    error = %String::from_utf8_lossy(&output.stderr),
                    "git fetch failed"
                );
                return;
            }
        }

        // Reset to origin/main to get merged content
        let reset_output = Command::new("git")
            .current_dir(repo_path)
            .args(["reset", "--hard", "origin/main"])
            .output();

        if let Ok(output) = reset_output {
            if output.status.success() {
                tracing::info!("successfully synced local repo with merged implementation");
            } else {
                tracing::warn!(
                    error = %String::from_utf8_lossy(&output.stderr),
                    "git reset failed"
                );
            }
        }
    }
}

/// Truncates a string to the given length, adding "..." if truncated.
fn truncate_string(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::WorktreeSandbox;

    #[test]
    fn runner_type_creates_correct_runner() {
        let claude_runner = RunnerType::Claude.create_runner();
        assert_eq!(claude_runner.name(), "claude-code");

        let gemini_runner = RunnerType::Gemini.create_runner();
        assert_eq!(gemini_runner.name(), "gemini-cli");
    }

    #[test]
    fn truncate_string_short() {
        assert_eq!(truncate_string("hello", 10), "hello");
    }

    #[test]
    fn truncate_string_long() {
        assert_eq!(truncate_string("hello world", 8), "hello...");
    }

    #[test]
    fn truncate_string_exact() {
        assert_eq!(truncate_string("hello", 5), "hello");
    }

    #[test]
    fn cruise_runner_can_be_created() {
        let provider = WorktreeSandbox::new(PathBuf::from("/tmp"), None);
        let runner = CruiseRunner::new(provider, PathBuf::from("/tmp/logs"));
        assert!(!runner.auto_approve);
    }

    #[test]
    fn cruise_runner_with_auto_approve() {
        let provider = WorktreeSandbox::new(PathBuf::from("/tmp"), None);
        let runner =
            CruiseRunner::new(provider, PathBuf::from("/tmp/logs")).with_auto_approve(true);
        assert!(runner.auto_approve);
    }

    #[test]
    fn cruise_runner_with_spawn_team() {
        let provider = WorktreeSandbox::new(PathBuf::from("/tmp"), None);
        let runner =
            CruiseRunner::new(provider, PathBuf::from("/tmp/logs")).with_spawn_team(true);
        assert!(runner.use_spawn_team);
    }

    #[test]
    fn cruise_runner_spawn_team_default_false() {
        let provider = WorktreeSandbox::new(PathBuf::from("/tmp"), None);
        let runner = CruiseRunner::new(provider, PathBuf::from("/tmp/logs"));
        assert!(!runner.use_spawn_team);
    }

    #[test]
    fn cruise_runner_with_config() {
        let provider = WorktreeSandbox::new(PathBuf::from("/tmp"), None);
        let config = CruiseConfig::default();
        let runner =
            CruiseRunner::new(provider, PathBuf::from("/tmp/logs")).with_config(config.clone());
        assert_eq!(runner.config().planning.ping_pong_iterations, 5);
    }

    #[test]
    fn create_planning_prompt_includes_json_format() {
        let provider = WorktreeSandbox::new(PathBuf::from("/tmp"), None);
        let runner = CruiseRunner::new(provider, PathBuf::from("/tmp/logs"));

        let prompt = runner.create_planning_prompt("Build a REST API");
        assert!(prompt.contains("Build a REST API"));
        assert!(prompt.contains("```json"));
        assert!(prompt.contains("CRUISE-"));
        assert!(prompt.contains("blocked_by"));
    }

    #[test]
    fn plan_issue_struct() {
        let issue = PlanIssue {
            beads_id: "bd-1".to_string(),
            plan_task_id: "CRUISE-001".to_string(),
            subject: "Setup project".to_string(),
        };
        assert_eq!(issue.beads_id, "bd-1");
        assert_eq!(issue.plan_task_id, "CRUISE-001");
    }
}
