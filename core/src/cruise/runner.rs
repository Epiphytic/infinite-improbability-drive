//! CruiseRunner: High-level orchestrator for cruise-control workflows.
//!
//! Provides two workflow modes:
//! - Simple: Single spawn for straightforward tasks
//! - Full: Plan → Approve → Execute with PR integration

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use crate::error::{Error, Result};
use crate::pr::{get_branch_commits, get_file_changes, PRManager};
use crate::runner::{ClaudeRunner, GeminiRunner, LLMRunner};
use crate::sandbox::{SandboxManifest, SandboxProvider};
use crate::spawn::{SpawnConfig, SpawnResult, SpawnStatus, Spawner};

use super::config::CruiseConfig;
use super::planner::generate_pr_body as generate_plan_pr_body;
use super::result::{BuildResult, CruiseResult, PlanResult};
use super::task::CruisePlan;

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

/// High-level orchestrator for cruise-control workflows.
///
/// Manages the full lifecycle of a cruise-control run:
/// - Simple workflow: Single spawn for straightforward tasks
/// - Full workflow: Plan → Approve → Execute with PR integration
pub struct CruiseRunner<P: SandboxProvider + Clone> {
    config: CruiseConfig,
    provider: P,
    logs_dir: PathBuf,
    /// Auto-approve PRs (useful for testing)
    auto_approve: bool,
}

impl<P: SandboxProvider + Clone + 'static> CruiseRunner<P> {
    /// Creates a new CruiseRunner with the given sandbox provider.
    pub fn new(provider: P, logs_dir: PathBuf) -> Self {
        Self {
            config: CruiseConfig::default(),
            provider,
            logs_dir,
            auto_approve: false,
        }
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
                task_results: vec![],
                max_parallelism: 1,
                duration: spawn_result.duration,
                completed_count: if success { 1 } else { 0 },
                blocked_count: 0,
            }),
            validation_result: None,
            total_duration: start.elapsed(),
            summary: spawn_result.summary,
        })
    }

    /// Runs a full workflow: Plan → Approve → Execute.
    ///
    /// This workflow:
    /// 1. Runs a planning spawn to generate an implementation plan
    /// 2. Creates a PR for the plan with enhanced formatting
    /// 3. Waits for approval (or auto-approves in test mode)
    /// 4. Runs an execution spawn to implement the plan
    /// 5. Creates a PR for the implementation with commits and file stats
    pub async fn run_full(
        &self,
        prompt: &str,
        runner_type: RunnerType,
        timeout: Duration,
        repo_path: &PathBuf,
    ) -> Result<CruiseResult> {
        let start = Instant::now();

        // Phase 1: Planning
        let planning_prompt = self.create_planning_prompt(prompt);
        let plan_result = self
            .run_planning_phase(&planning_prompt, runner_type, timeout, repo_path)
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

        // Phase 2: Wait for approval (or auto-approve)
        if let Some(ref pr_url) = plan_result.pr_url {
            if self.auto_approve {
                self.auto_approve_pr(pr_url)?;
            } else {
                // In production, would poll for approval
                // For now, just log and continue
                tracing::info!(pr_url = %pr_url, "plan PR created, waiting for approval");
            }
        }

        // Phase 3: Execution
        let build_result = self
            .run_execution_phase(prompt, runner_type, timeout, repo_path)
            .await?;

        let success = build_result.success;

        Ok(CruiseResult {
            success,
            prompt: prompt.to_string(),
            plan_result: Some(plan_result),
            build_result: Some(build_result),
            validation_result: None,
            total_duration: start.elapsed(),
            summary: if success {
                "Full workflow completed successfully".to_string()
            } else {
                "Execution phase failed".to_string()
            },
        })
    }

    /// Creates a planning prompt from the user's original prompt.
    fn create_planning_prompt(&self, prompt: &str) -> String {
        format!(
            "Create a detailed implementation plan for the following task. \
             Output your plan as a structured markdown document with sections for: \
             Overview, Tasks (with dependencies), and Risk Areas.\n\n\
             Task: {}",
            prompt
        )
    }

    /// Runs the planning phase and creates a plan PR.
    async fn run_planning_phase(
        &self,
        planning_prompt: &str,
        runner_type: RunnerType,
        timeout: Duration,
        repo_path: &PathBuf,
    ) -> Result<PlanResult> {
        let start = Instant::now();

        let spawner = Spawner::new(self.provider.clone(), self.logs_dir.join("planning"));
        let config = SpawnConfig::new(planning_prompt).with_total_timeout(timeout);
        let manifest = SandboxManifest::default();
        let runner = runner_type.create_runner();

        let spawn_result = spawner.spawn(config, manifest, runner).await?;
        let success = spawn_result.status == SpawnStatus::Success;

        // Create plan PR if successful
        let pr_url = if success {
            if let Some(ref sandbox_path) = spawn_result.sandbox_path {
                self.create_plan_pr(sandbox_path, repo_path, planning_prompt, &spawn_result)
            } else {
                None
            }
        } else {
            None
        };

        Ok(PlanResult {
            success,
            iterations: 1, // Single-shot for now
            task_count: 0, // Would extract from plan
            pr_url,
            duration: start.elapsed(),
            plan_file: None,
            error: if success {
                None
            } else {
                Some(spawn_result.summary)
            },
        })
    }

    /// Creates a PR for the plan with enhanced formatting.
    fn create_plan_pr(
        &self,
        sandbox_path: &PathBuf,
        repo_path: &PathBuf,
        prompt: &str,
        spawn_result: &SpawnResult,
    ) -> Option<String> {
        let pr_manager = PRManager::new(repo_path.clone());

        // Commit changes
        let commit_message = format!("Plan: {}\n\nSpawn ID: {}",
            truncate_string(prompt, 50),
            spawn_result.spawn_id
        );

        if pr_manager.commit_changes(sandbox_path, &commit_message).ok()?.is_none() {
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

        // Create a simple plan for the PR body
        let plan = CruisePlan::new(prompt);
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

    /// Runs the execution phase and creates an implementation PR.
    async fn run_execution_phase(
        &self,
        prompt: &str,
        runner_type: RunnerType,
        timeout: Duration,
        repo_path: &PathBuf,
    ) -> Result<BuildResult> {
        let start = Instant::now();

        let spawner = Spawner::new(self.provider.clone(), self.logs_dir.join("execution"));
        let config = SpawnConfig::new(prompt).with_total_timeout(timeout);
        let manifest = SandboxManifest::default();
        let runner = runner_type.create_runner();

        let spawn_result = spawner.spawn(config, manifest, runner).await?;
        let success = spawn_result.status == SpawnStatus::Success;

        // Create implementation PR if successful
        if success {
            if let Some(ref sandbox_path) = spawn_result.sandbox_path {
                self.create_implementation_pr(sandbox_path, repo_path, prompt, &spawn_result);
            }
        }

        Ok(BuildResult {
            success,
            task_results: vec![],
            max_parallelism: 1,
            duration: start.elapsed(),
            completed_count: if success { 1 } else { 0 },
            blocked_count: 0,
        })
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

        if pr_manager.commit_changes(sandbox_path, &commit_message).ok()?.is_none() {
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
        let files_changed = get_file_changes(sandbox_path, branch_name, "main").unwrap_or_default();

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
        let runner = CruiseRunner::new(provider, PathBuf::from("/tmp/logs"))
            .with_auto_approve(true);
        assert!(runner.auto_approve);
    }

    #[test]
    fn cruise_runner_with_config() {
        let provider = WorktreeSandbox::new(PathBuf::from("/tmp"), None);
        let config = CruiseConfig::default();
        let runner = CruiseRunner::new(provider, PathBuf::from("/tmp/logs"))
            .with_config(config.clone());
        assert_eq!(runner.config().planning.ping_pong_iterations, 5);
    }

    #[test]
    fn create_planning_prompt() {
        let provider = WorktreeSandbox::new(PathBuf::from("/tmp"), None);
        let runner = CruiseRunner::new(provider, PathBuf::from("/tmp/logs"));

        let prompt = runner.create_planning_prompt("Build a REST API");
        assert!(prompt.contains("Build a REST API"));
        assert!(prompt.contains("implementation plan"));
    }
}
