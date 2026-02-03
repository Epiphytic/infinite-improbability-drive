//! Spawn-team orchestration for multi-LLM workflows.
//!
//! Implements the actual ping-pong loop between primary and reviewer LLMs,
//! with full observability into permissions, command lines, and review feedback.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use crate::error::{Error, Result};
use crate::runner::{ClaudeRunner, GeminiRunner, LLMRunner};
use crate::sandbox::{SandboxManifest, SandboxProvider};
use crate::spawn::{SpawnConfig, SpawnResult, SpawnStatus, Spawner};
use crate::team::{
    parse_review_response, CoordinationMode, FixPromptBuilder, GitHubReview, GitHubReviewComment,
    GitHubReviewPromptBuilder, ResolveCommentPromptBuilder, ReviewDomain, ReviewPromptBuilder,
    ReviewResult, ReviewVerdict, SpawnTeamConfig, SpawnTeamResult,
};

/// Observability data captured during spawn-team execution.
#[derive(Debug, Clone, Default)]
pub struct SpawnObservability {
    /// Command lines used for each LLM invocation.
    pub command_lines: Vec<CommandLineRecord>,
    /// Permissions requested during execution.
    pub permissions_requested: Vec<PermissionRecord>,
    /// Permissions granted during execution.
    pub permissions_granted: Vec<PermissionRecord>,
    /// All review feedback from Gemini.
    pub review_feedback: Vec<ReviewFeedbackRecord>,
    /// Security review findings.
    pub security_findings: Vec<SecurityFinding>,
    /// Actual sandbox path where the work was done (captured from Spawner).
    pub sandbox_path: Option<PathBuf>,
    /// Commits made during execution.
    pub commits: Vec<CommitRecord>,
    /// GitHub PR URL (for GitHub mode).
    pub pr_url: Option<String>,
    /// GitHub reviews created during execution.
    pub github_reviews: Vec<GitHubReview>,
    /// GitHub review comments resolved.
    pub resolved_comments: Vec<ResolvedCommentRecord>,
}

/// Record of a resolved GitHub review comment.
#[derive(Debug, Clone)]
pub struct ResolvedCommentRecord {
    /// The comment ID.
    pub comment_id: u64,
    /// The commit that resolved the comment.
    pub resolved_by_commit: String,
    /// Timestamp of resolution.
    pub resolved_at: String,
    /// Brief explanation of the fix.
    pub explanation: String,
}

/// Record of a command line invocation.
#[derive(Debug, Clone)]
pub struct CommandLineRecord {
    /// Which LLM was invoked.
    pub llm: String,
    /// The full command line.
    pub command: String,
    /// Working directory.
    pub work_dir: PathBuf,
    /// Iteration number (for ping-pong).
    pub iteration: u32,
    /// Role: "primary" or "reviewer".
    pub role: String,
    /// Timestamp.
    pub timestamp: String,
}

/// Record of a commit made during execution.
#[derive(Debug, Clone)]
pub struct CommitRecord {
    /// Commit hash.
    pub hash: String,
    /// Commit message.
    pub message: String,
    /// Iteration when commit was made.
    pub iteration: u32,
    /// Which LLM made the changes.
    pub llm: String,
    /// Timestamp.
    pub timestamp: String,
    /// Whether it was pushed to remote.
    pub pushed: bool,
}

/// Record of a permission request.
#[derive(Debug, Clone)]
pub struct PermissionRecord {
    /// Type of permission (e.g., "file_read", "file_write", "bash").
    pub permission_type: String,
    /// Resource being accessed.
    pub resource: String,
    /// Whether it was granted.
    pub granted: bool,
    /// Which LLM requested it.
    pub llm: String,
    /// Iteration number.
    pub iteration: u32,
}

/// Record of review feedback from Gemini.
#[derive(Debug, Clone)]
pub struct ReviewFeedbackRecord {
    /// Iteration number.
    pub iteration: u32,
    /// Review phase (if applicable).
    pub phase: Option<String>,
    /// The verdict.
    pub verdict: ReviewVerdict,
    /// Number of suggestions.
    pub suggestion_count: usize,
    /// Full review result.
    pub review: ReviewResult,
    /// Raw response from Gemini.
    pub raw_response: String,
    /// Git diff that was reviewed.
    pub diff_reviewed: String,
}

/// Security finding from review.
#[derive(Debug, Clone)]
pub struct SecurityFinding {
    /// Severity: critical, high, medium, low.
    pub severity: String,
    /// Description of the finding.
    pub description: String,
    /// File affected.
    pub file: Option<String>,
    /// Recommendation.
    pub recommendation: String,
}

/// Orchestrates spawn-team execution with ping-pong mode.
pub struct SpawnTeamOrchestrator<P: SandboxProvider + Clone> {
    config: SpawnTeamConfig,
    provider: P,
    logs_dir: PathBuf,
    observability: SpawnObservability,
}

impl<P: SandboxProvider + Clone + 'static> SpawnTeamOrchestrator<P> {
    /// Creates a new orchestrator.
    pub fn new(provider: P, logs_dir: PathBuf) -> Self {
        Self {
            config: SpawnTeamConfig::default(),
            provider,
            logs_dir,
            observability: SpawnObservability::default(),
        }
    }

    /// Sets the team configuration.
    pub fn with_config(mut self, config: SpawnTeamConfig) -> Self {
        self.config = config;
        self
    }

    /// Returns the observability data collected during execution.
    pub fn observability(&self) -> &SpawnObservability {
        &self.observability
    }

    /// Takes ownership of the observability data.
    pub fn take_observability(self) -> SpawnObservability {
        self.observability
    }

    /// Runs the spawn-team workflow.
    ///
    /// For Sequential mode: Primary -> Review -> Fix (once)
    /// For PingPong mode: Primary -> Review -> Fix -> Review -> ... (until approved or max iterations)
    pub async fn run(
        &mut self,
        prompt: &str,
        timeout: Duration,
        sandbox_path: &Path,
    ) -> Result<SpawnTeamResult> {
        self.run_with_branch(prompt, timeout, sandbox_path, None).await
    }

    /// Runs the spawn-team workflow with an explicit branch name.
    ///
    /// This allows CruiseRunner to control branch naming per workflow phase.
    pub async fn run_with_branch(
        &mut self,
        prompt: &str,
        timeout: Duration,
        sandbox_path: &Path,
        branch_name: Option<&str>,
    ) -> Result<SpawnTeamResult> {
        let start = Instant::now();
        let mut iterations = 0;
        let mut reviews = Vec::new();
        let mut final_verdict = None;

        match self.config.mode {
            CoordinationMode::Sequential => {
                // Single iteration: primary -> review -> fix
                iterations = 1;

                // Run primary
                let primary_result = self
                    .run_primary(prompt, timeout, sandbox_path, iterations, branch_name)
                    .await?;

                if primary_result.status != SpawnStatus::Success {
                    return Ok(SpawnTeamResult {
                        success: false,
                        iterations,
                        final_verdict: None,
                        reviews,
                        summary: format!("Primary LLM failed: {}", primary_result.summary),
                    });
                }

                // Get diff for review
                let diff = self.get_git_diff(sandbox_path)?;

                // Run reviewer
                let review_result = self
                    .run_reviewer(prompt, &diff, timeout, sandbox_path, iterations, None)
                    .await?;

                final_verdict = Some(review_result.verdict.clone());
                reviews.push(review_result.clone());

                // If needs changes, run fix phase
                if review_result.verdict == ReviewVerdict::NeedsChanges {
                    let _fix_result = self
                        .run_fix(prompt, &review_result, timeout, sandbox_path, iterations, branch_name)
                        .await?;
                }
            }

            CoordinationMode::PingPong => {
                // Iterative: primary -> review -> fix -> review -> ...
                // IMPORTANT: Always run all 5 review phases regardless of approval
                let mut current_prompt = prompt.to_string();
                let total_phases = 5; // Security, TechnicalFeasibility, TaskGranularity, DependencyCompleteness, GeneralPolish
                let phases_to_run = total_phases.min(self.config.max_iterations);

                // Track the sandbox path from the first iteration to reuse for subsequent iterations
                let mut active_sandbox_path: Option<PathBuf> = None;

                for i in 1..=phases_to_run {
                    iterations = i;

                    // Run primary (or fix if not first iteration)
                    // First iteration: create sandbox with branch name
                    // Subsequent iterations: reuse the existing sandbox
                    let primary_result = if i == 1 {
                        // First iteration - create the sandbox with the branch
                        let result = self
                            .run_primary(&current_prompt, timeout, sandbox_path, i, branch_name)
                            .await?;
                        // Capture the sandbox path for reuse
                        if let Some(ref path) = result.sandbox_path {
                            active_sandbox_path = Some(path.clone());
                        }
                        result
                    } else {
                        // Subsequent iterations - run on the existing sandbox
                        // Pass None for branch_name to use the existing worktree
                        if let Some(ref existing_sandbox) = active_sandbox_path {
                            self.run_primary_on_existing_sandbox(&current_prompt, timeout, existing_sandbox, i)
                                .await?
                        } else {
                            // Fallback: create new sandbox if we lost track (shouldn't happen)
                            self.run_primary(&current_prompt, timeout, sandbox_path, i, None)
                                .await?
                        }
                    };

                    if primary_result.status != SpawnStatus::Success {
                        return Ok(SpawnTeamResult {
                            success: false,
                            iterations,
                            final_verdict: None,
                            reviews,
                            summary: format!(
                                "Primary LLM failed on iteration {}: {}",
                                i, primary_result.summary
                            ),
                        });
                    }

                    // Update active sandbox path if it changed
                    if let Some(ref path) = primary_result.sandbox_path {
                        active_sandbox_path = Some(path.clone());
                    }

                    // Get diff for review - use the active sandbox path
                    let default_path = sandbox_path.to_path_buf();
                    let diff_sandbox = active_sandbox_path.as_ref().unwrap_or(&default_path);
                    let diff = self.get_git_diff(diff_sandbox)?;

                    // Determine review phase based on iteration
                    let phase = self.get_review_phase(i);

                    // Run reviewer
                    let review_result = self
                        .run_reviewer(
                            prompt,
                            &diff,
                            timeout,
                            sandbox_path,
                            i,
                            Some(phase.as_str()),
                        )
                        .await?;

                    final_verdict = Some(review_result.verdict.clone());
                    reviews.push(review_result.clone());

                    // Log the phase result but continue to next phase
                    // All 5 review phases run regardless of approval in earlier phases
                    if review_result.verdict == ReviewVerdict::Approved {
                        tracing::info!(
                            iteration = i,
                            phase = %phase,
                            "reviewer approved for this phase, continuing to next phase"
                        );
                        // Only build fix prompt if there are actually suggestions
                        if !review_result.suggestions.is_empty() {
                            current_prompt = FixPromptBuilder::new(prompt)
                                .with_suggestions(review_result.suggestions.clone())
                                .build();
                        }
                    } else {
                        // Build fix prompt for next iteration
                        current_prompt = FixPromptBuilder::new(prompt)
                            .with_suggestions(review_result.suggestions.clone())
                            .build();

                        tracing::info!(
                            iteration = i,
                            phase = %phase,
                            suggestions = review_result.suggestions.len(),
                            "reviewer requested changes, preparing fix"
                        );
                    }
                }
            }

            CoordinationMode::GitHub => {
                // GitHub-based workflow:
                // 1. Primary creates initial implementation
                // 2. Create PR on first commit
                // 3. Reviewers create GitHub reviews with line comments
                // 4. Coder resolves each comment with commits
                // 5. All domains reviewed in sequence

                let repo = self.get_repo_name(sandbox_path)?;

                // Track the sandbox path from the first iteration
                let mut active_sandbox_path: Option<PathBuf> = None;

                // Step 1: Run primary to create initial implementation
                tracing::info!("GitHub mode: running primary LLM for initial implementation");
                let primary_result = self
                    .run_primary(prompt, timeout, sandbox_path, 1, branch_name)
                    .await?;

                if primary_result.status != SpawnStatus::Success {
                    return Ok(SpawnTeamResult {
                        success: false,
                        iterations: 1,
                        final_verdict: None,
                        reviews,
                        summary: format!("Primary LLM failed: {}", primary_result.summary),
                    });
                }

                // Capture sandbox path
                if let Some(ref path) = primary_result.sandbox_path {
                    active_sandbox_path = Some(path.clone());
                }

                let default_path = sandbox_path.to_path_buf();
                let work_sandbox = active_sandbox_path.as_ref().unwrap_or(&default_path);

                // Step 2: Create PR on first commit
                let pr_url = self.create_pr_on_first_commit(work_sandbox, prompt, &repo)?;
                self.observability.pr_url = Some(pr_url.clone());

                // Extract PR number from URL
                let pr_number = self.extract_pr_number(&pr_url)?;

                tracing::info!(
                    pr_url = %pr_url,
                    pr_number = pr_number,
                    "GitHub mode: created PR on first commit"
                );

                // Step 3: Run each review domain
                for domain in ReviewDomain::all() {
                    iterations += 1;

                    tracing::info!(
                        domain = %domain.as_str(),
                        iteration = iterations,
                        "GitHub mode: starting review domain"
                    );

                    // Get current diff for this domain
                    let diff = self.get_git_diff(work_sandbox)?;

                    // Run reviewer for this domain - creates GitHub review
                    let review = self
                        .run_github_reviewer(
                            prompt,
                            &diff,
                            timeout,
                            work_sandbox,
                            *domain,
                            pr_number,
                            &repo,
                        )
                        .await?;

                    reviews.push(review.clone());
                    final_verdict = Some(review.verdict.clone());

                    // If review requested changes, resolve comments
                    if review.verdict == ReviewVerdict::NeedsChanges {
                        // Get pending comments from the PR
                        let pending_comments =
                            self.get_pending_review_comments(pr_number, &repo, work_sandbox)?;

                        // Resolve each comment
                        for comment in pending_comments {
                            tracing::info!(
                                comment_id = comment.id,
                                path = %comment.path,
                                "GitHub mode: resolving review comment"
                            );

                            self.resolve_github_comment(
                                prompt,
                                timeout,
                                work_sandbox,
                                pr_number,
                                &repo,
                                comment,
                            )
                            .await?;
                        }
                    }
                }

                // Final success is determined by the last review phase
                let success = final_verdict
                    .as_ref()
                    .map(|v| *v == ReviewVerdict::Approved)
                    .unwrap_or(false);

                return Ok(SpawnTeamResult {
                    success,
                    iterations,
                    final_verdict,
                    reviews,
                    summary: format!(
                        "GitHub workflow completed. PR: {}. {} domains reviewed.",
                        pr_url,
                        ReviewDomain::all().len()
                    ),
                });
            }
        }

        let success = final_verdict
            .as_ref()
            .map(|v| *v == ReviewVerdict::Approved)
            .unwrap_or(false);

        let summary = if success {
            format!(
                "Spawn-team completed successfully after {} iteration(s)",
                iterations
            )
        } else if iterations >= self.config.max_iterations {
            format!(
                "Spawn-team reached max iterations ({}) without approval",
                self.config.max_iterations
            )
        } else {
            "Spawn-team completed with issues".to_string()
        };

        tracing::info!(
            success = success,
            iterations = iterations,
            duration = ?start.elapsed(),
            "spawn-team completed"
        );

        Ok(SpawnTeamResult {
            success,
            iterations,
            final_verdict,
            reviews,
            summary,
        })
    }

    /// Runs the primary LLM (Claude).
    async fn run_primary(
        &mut self,
        prompt: &str,
        timeout: Duration,
        sandbox_path: &Path,
        iteration: u32,
        branch_name: Option<&str>,
    ) -> Result<SpawnResult> {
        let runner = ClaudeRunner::new();

        // Record command line
        let command = format!(
            "{} --print --output-format stream-json \"{}\"",
            runner.name(),
            prompt.chars().take(50).collect::<String>()
        );
        self.observability.command_lines.push(CommandLineRecord {
            llm: self.config.primary_llm.clone(),
            command,
            work_dir: sandbox_path.to_path_buf(),
            iteration,
            role: "primary".to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
        });

        let spawner = Spawner::new(
            self.provider.clone(),
            self.logs_dir.join(format!("primary-{}", iteration)),
        );

        let config = SpawnConfig::new(prompt).with_total_timeout(timeout);
        let manifest = SandboxManifest::default();

        tracing::info!(
            iteration = iteration,
            llm = %self.config.primary_llm,
            branch = ?branch_name,
            "running primary LLM"
        );

        let result = spawner.spawn_with_branch(config, manifest, Box::new(runner), branch_name).await?;

        // Capture the actual sandbox path where work was done
        if let Some(ref sandbox) = result.sandbox_path {
            self.observability.sandbox_path = Some(sandbox.clone());
            // Update the command line record with the actual work directory
            if let Some(last_cmd) = self.observability.command_lines.last_mut() {
                last_cmd.work_dir = sandbox.clone();
            }

            // Commit and push changes after each LLM run for observability
            if result.status == SpawnStatus::Success {
                self.commit_and_push_changes(
                    sandbox,
                    iteration,
                    &self.config.primary_llm.clone(),
                    None,
                );
            }
        }

        Ok(result)
    }

    /// Runs the primary LLM on an existing sandbox (for subsequent iterations).
    ///
    /// This avoids creating a new worktree and reuses the existing sandbox
    /// from the first iteration.
    async fn run_primary_on_existing_sandbox(
        &mut self,
        prompt: &str,
        timeout: Duration,
        sandbox_path: &Path,
        iteration: u32,
    ) -> Result<SpawnResult> {
        let runner = ClaudeRunner::new();

        // Record command line
        let command = format!(
            "{} --print --output-format stream-json \"{}\"",
            runner.name(),
            prompt.chars().take(50).collect::<String>()
        );
        self.observability.command_lines.push(CommandLineRecord {
            llm: self.config.primary_llm.clone(),
            command,
            work_dir: sandbox_path.to_path_buf(),
            iteration,
            role: "primary".to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
        });

        tracing::info!(
            iteration = iteration,
            llm = %self.config.primary_llm,
            sandbox_path = ?sandbox_path,
            "running primary LLM on existing sandbox"
        );

        // Run the LLM directly using Command instead of spawning a new sandbox
        // Use --print with --verbose for stream-json output format
        let output = std::process::Command::new("claude")
            .current_dir(sandbox_path)
            .args(["--print", "--verbose", "--output-format", "stream-json", prompt])
            .output()
            .map_err(|e| Error::Cruise(format!("failed to run claude: {}", e)))?;

        let success = output.status.success();
        let summary = if success {
            "LLM completed successfully".to_string()
        } else {
            format!(
                "LLM failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )
        };

        // Commit and push changes after LLM run for observability
        if success {
            self.commit_and_push_changes(
                sandbox_path,
                iteration,
                &self.config.primary_llm.clone(),
                None,
            );
        }

        Ok(SpawnResult {
            status: if success {
                SpawnStatus::Success
            } else {
                SpawnStatus::Failed
            },
            spawn_id: format!("existing-sandbox-iter-{}", iteration),
            duration: timeout, // We don't track actual duration here
            files_changed: vec![],
            commits: vec![],
            summary,
            pr_url: None,
            logs: crate::spawn::SpawnLogs {
                stdout: sandbox_path.join("stdout.log"),
                stderr: sandbox_path.join("stderr.log"),
                events: sandbox_path.join("events.jsonl"),
            },
            sandbox_path: Some(sandbox_path.to_path_buf()),
        })
    }

    /// Runs the reviewer LLM (Gemini).
    async fn run_reviewer(
        &mut self,
        original_prompt: &str,
        diff: &str,
        _timeout: Duration,
        sandbox_path: &Path,
        iteration: u32,
        phase: Option<&str>,
    ) -> Result<ReviewResult> {
        let runner = GeminiRunner::new();

        // Build review prompt with phase-specific focus
        let mut review_prompt = ReviewPromptBuilder::new(original_prompt)
            .with_diff(diff)
            .build();

        // Add phase-specific instructions
        if let Some(phase_name) = phase {
            review_prompt = format!(
                "{}\n\n### Review Focus: {}\n\n{}",
                review_prompt,
                phase_name,
                self.get_phase_instructions(phase_name)
            );
        }

        // Record command line
        let command = format!(
            "{} --print \"{}\"",
            runner.name(),
            review_prompt.chars().take(50).collect::<String>()
        );
        self.observability.command_lines.push(CommandLineRecord {
            llm: self.config.reviewer_llm.clone(),
            command,
            work_dir: sandbox_path.to_path_buf(),
            iteration,
            role: "reviewer".to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
        });

        tracing::info!(
            iteration = iteration,
            llm = %self.config.reviewer_llm,
            phase = ?phase,
            "running reviewer LLM"
        );

        // Run Gemini directly (not through spawner - it's a reviewer, not making changes)
        let output = Command::new("gemini")
            .current_dir(sandbox_path)
            .args(["--print", &review_prompt])
            .output()
            .map_err(|e| Error::Cruise(format!("failed to run gemini: {}", e)))?;

        let raw_response = String::from_utf8_lossy(&output.stdout).to_string();

        // Parse review response
        let review = parse_review_response(&raw_response).unwrap_or_else(|| {
            tracing::warn!("failed to parse review response, treating as approved");
            ReviewResult {
                verdict: ReviewVerdict::Approved,
                suggestions: vec![],
                summary: "Could not parse review response".to_string(),
            }
        });

        // Record feedback
        self.observability
            .review_feedback
            .push(ReviewFeedbackRecord {
                iteration,
                phase: phase.map(String::from),
                verdict: review.verdict.clone(),
                suggestion_count: review.suggestions.len(),
                review: review.clone(),
                raw_response: raw_response.clone(),
                diff_reviewed: diff.to_string(),
            });

        // Extract security findings if this is a security review phase
        if phase == Some("Security") {
            self.extract_security_findings(&raw_response);
        }

        tracing::info!(
            iteration = iteration,
            verdict = ?review.verdict,
            suggestions = review.suggestions.len(),
            "reviewer completed"
        );

        Ok(review)
    }

    /// Runs a fix phase with the primary LLM.
    async fn run_fix(
        &mut self,
        original_prompt: &str,
        review: &ReviewResult,
        timeout: Duration,
        sandbox_path: &Path,
        iteration: u32,
        branch_name: Option<&str>,
    ) -> Result<SpawnResult> {
        let fix_prompt = FixPromptBuilder::new(original_prompt)
            .with_suggestions(review.suggestions.clone())
            .build();

        self.run_primary(&fix_prompt, timeout, sandbox_path, iteration, branch_name)
            .await
    }

    /// Gets the git diff of changes in the sandbox.
    fn get_git_diff(&self, sandbox_path: &Path) -> Result<String> {
        let output = Command::new("git")
            .current_dir(sandbox_path)
            .args(["diff", "HEAD~1..HEAD"])
            .output()
            .map_err(|e| Error::Git(format!("failed to get diff: {}", e)))?;

        if !output.status.success() {
            // Try diff against nothing (for first commit)
            let output = Command::new("git")
                .current_dir(sandbox_path)
                .args(["diff", "--cached"])
                .output()
                .map_err(|e| Error::Git(format!("failed to get diff: {}", e)))?;

            return Ok(String::from_utf8_lossy(&output.stdout).to_string());
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// Gets the review phase name based on iteration.
    fn get_review_phase(&self, iteration: u32) -> String {
        match iteration {
            1 => "Security".to_string(),
            2 => "TechnicalFeasibility".to_string(),
            3 => "TaskGranularity".to_string(),
            4 => "DependencyCompleteness".to_string(),
            _ => "GeneralPolish".to_string(),
        }
    }

    /// Gets phase-specific review instructions.
    fn get_phase_instructions(&self, phase: &str) -> String {
        match phase {
            "Security" => {
                "Focus on security issues: authentication, authorization, input validation, \
                 secrets handling, injection vulnerabilities, and OWASP top 10."
                    .to_string()
            }
            "TechnicalFeasibility" => {
                "Focus on technical approach: Is the architecture sound? Are the right \
                 technologies being used? Are there performance concerns?"
                    .to_string()
            }
            "TaskGranularity" => {
                "Focus on task sizing: Are tasks appropriately sized for parallel execution? \
                 Should any be split or combined?"
                    .to_string()
            }
            "DependencyCompleteness" => {
                "Focus on dependencies: Are all task dependencies correctly identified? \
                 Are there missing dependencies or opportunities for parallelization?"
                    .to_string()
            }
            _ => {
                "General review: Look for any remaining issues, code quality, \
                 documentation, and overall polish."
                    .to_string()
            }
        }
    }

    /// Commits and pushes changes made by the LLM.
    /// This improves observability by creating a commit for each LLM iteration.
    fn commit_and_push_changes(
        &mut self,
        sandbox_path: &Path,
        iteration: u32,
        llm: &str,
        phase: Option<&str>,
    ) -> Option<String> {
        // Check if there are changes to commit
        let status_output = Command::new("git")
            .current_dir(sandbox_path)
            .args(["status", "--porcelain"])
            .output()
            .ok()?;

        let status = String::from_utf8_lossy(&status_output.stdout);
        if status.trim().is_empty() {
            tracing::debug!(iteration = iteration, "no changes to commit");
            return None;
        }

        // Stage all changes
        let add_output = Command::new("git")
            .current_dir(sandbox_path)
            .args(["add", "-A"])
            .output()
            .ok()?;

        if !add_output.status.success() {
            tracing::warn!(
                error = %String::from_utf8_lossy(&add_output.stderr),
                "failed to stage changes"
            );
            return None;
        }

        // Create commit message
        let phase_str = phase.map(|p| format!(" - {}", p)).unwrap_or_default();
        let commit_message = format!(
            "[cruise-control] {} iteration {}{}",
            llm, iteration, phase_str
        );

        // Commit changes
        let commit_output = Command::new("git")
            .current_dir(sandbox_path)
            .args(["commit", "-m", &commit_message])
            .output()
            .ok()?;

        if !commit_output.status.success() {
            tracing::warn!(
                error = %String::from_utf8_lossy(&commit_output.stderr),
                "failed to commit changes"
            );
            return None;
        }

        // Get commit hash
        let hash_output = Command::new("git")
            .current_dir(sandbox_path)
            .args(["rev-parse", "HEAD"])
            .output()
            .ok()?;

        let hash = String::from_utf8_lossy(&hash_output.stdout).trim().to_string();

        // Push to remote
        let branch_output = Command::new("git")
            .current_dir(sandbox_path)
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .output()
            .ok()?;

        let branch = String::from_utf8_lossy(&branch_output.stdout).trim().to_string();

        let push_output = Command::new("git")
            .current_dir(sandbox_path)
            .args(["push", "-u", "origin", &branch])
            .output()
            .ok();

        let pushed = push_output
            .map(|o| o.status.success())
            .unwrap_or(false);

        // Record commit
        self.observability.commits.push(CommitRecord {
            hash: hash.clone(),
            message: commit_message,
            iteration,
            llm: llm.to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            pushed,
        });

        tracing::info!(
            iteration = iteration,
            hash = %hash,
            pushed = pushed,
            "committed and pushed changes"
        );

        Some(hash)
    }

    /// Extracts security findings from review response.
    fn extract_security_findings(&mut self, response: &str) {
        // Look for security-related keywords and extract findings
        let keywords = [
            "vulnerability",
            "security",
            "injection",
            "authentication",
            "authorization",
            "secret",
            "credential",
            "sensitive",
        ];

        for line in response.lines() {
            let lower = line.to_lowercase();
            for keyword in &keywords {
                if lower.contains(keyword) {
                    self.observability.security_findings.push(SecurityFinding {
                        severity: if lower.contains("critical") {
                            "critical"
                        } else if lower.contains("high") {
                            "high"
                        } else if lower.contains("medium") {
                            "medium"
                        } else {
                            "low"
                        }
                        .to_string(),
                        description: line.trim().to_string(),
                        file: None,
                        recommendation: "Review and address security concern".to_string(),
                    });
                    break;
                }
            }
        }
    }

    // =========================================================================
    // GitHub Mode Helper Methods
    // =========================================================================

    /// Gets the repository name from the sandbox path (e.g., "owner/repo").
    fn get_repo_name(&self, sandbox_path: &Path) -> Result<String> {
        let output = Command::new("gh")
            .current_dir(sandbox_path)
            .args(["repo", "view", "--json", "nameWithOwner", "-q", ".nameWithOwner"])
            .output()
            .map_err(|e| Error::Cruise(format!("failed to get repo name: {}", e)))?;

        if !output.status.success() {
            return Err(Error::Cruise(format!(
                "gh repo view failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Creates a PR on the first commit to the branch.
    fn create_pr_on_first_commit(
        &self,
        sandbox_path: &Path,
        prompt: &str,
        repo: &str,
    ) -> Result<String> {
        // Get current branch
        let branch_output = Command::new("git")
            .current_dir(sandbox_path)
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .output()
            .map_err(|e| Error::Git(format!("failed to get branch: {}", e)))?;

        let branch = String::from_utf8_lossy(&branch_output.stdout).trim().to_string();

        // Get default branch
        let default_branch = self.get_default_branch(sandbox_path)?;

        // Push the branch first
        let push_output = Command::new("git")
            .current_dir(sandbox_path)
            .args(["push", "-u", "origin", &branch])
            .output()
            .map_err(|e| Error::Git(format!("failed to push branch: {}", e)))?;

        if !push_output.status.success() {
            tracing::warn!(
                error = %String::from_utf8_lossy(&push_output.stderr),
                "failed to push branch, may already be pushed"
            );
        }

        // Create PR title from prompt (truncate if needed)
        let title: String = prompt.chars().take(70).collect();
        let title = if prompt.len() > 70 {
            format!("{}...", title)
        } else {
            title
        };

        // Create PR body with accordion for prompt
        let body = format!(
            "## Summary\n\n\
             Initial implementation created by cruise-control.\n\n\
             <details>\n\
             <summary>Original Prompt</summary>\n\n\
             {}\n\n\
             </details>\n\n\
             ---\n\
             *This PR will be updated as reviews are addressed.*\n",
            prompt
        );

        // Create the PR using gh CLI
        let pr_output = Command::new("gh")
            .current_dir(sandbox_path)
            .args([
                "pr",
                "create",
                "--repo",
                repo,
                "--title",
                &title,
                "--body",
                &body,
                "--base",
                &default_branch,
                "--head",
                &branch,
            ])
            .output()
            .map_err(|e| Error::Cruise(format!("failed to create PR: {}", e)))?;

        if !pr_output.status.success() {
            return Err(Error::Cruise(format!(
                "gh pr create failed: {}",
                String::from_utf8_lossy(&pr_output.stderr)
            )));
        }

        Ok(String::from_utf8_lossy(&pr_output.stdout).trim().to_string())
    }

    /// Gets the default branch of the repository.
    fn get_default_branch(&self, sandbox_path: &Path) -> Result<String> {
        let output = Command::new("gh")
            .current_dir(sandbox_path)
            .args([
                "repo",
                "view",
                "--json",
                "defaultBranchRef",
                "-q",
                ".defaultBranchRef.name",
            ])
            .output()
            .map_err(|e| Error::Cruise(format!("failed to get default branch: {}", e)))?;

        if !output.status.success() {
            // Fallback to main
            return Ok("main".to_string());
        }

        let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if branch.is_empty() {
            Ok("main".to_string())
        } else {
            Ok(branch)
        }
    }

    /// Extracts the PR number from a PR URL.
    fn extract_pr_number(&self, pr_url: &str) -> Result<u64> {
        // URL format: https://github.com/owner/repo/pull/123
        pr_url
            .split('/')
            .last()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| Error::Cruise(format!("failed to extract PR number from: {}", pr_url)))
    }

    /// Runs the GitHub reviewer for a specific domain.
    ///
    /// The reviewer uses `gh pr review` to create a GitHub review with line comments.
    async fn run_github_reviewer(
        &mut self,
        original_prompt: &str,
        diff: &str,
        _timeout: Duration,
        sandbox_path: &Path,
        domain: ReviewDomain,
        pr_number: u64,
        repo: &str,
    ) -> Result<ReviewResult> {
        let runner = GeminiRunner::new();

        // Build GitHub review prompt
        let review_prompt = GitHubReviewPromptBuilder::new(pr_number, repo, domain)
            .with_original_prompt(original_prompt)
            .with_diff(diff)
            .build();

        // Record command line
        let command = format!(
            "{} --print \"GitHub review for {} domain\"",
            runner.name(),
            domain.as_str()
        );
        self.observability.command_lines.push(CommandLineRecord {
            llm: self.config.reviewer_llm.clone(),
            command,
            work_dir: sandbox_path.to_path_buf(),
            iteration: 0, // GitHub mode uses domain instead of iteration
            role: format!("reviewer-{}", domain.as_str().to_lowercase()),
            timestamp: chrono::Utc::now().to_rfc3339(),
        });

        tracing::info!(
            domain = %domain.as_str(),
            pr_number = pr_number,
            llm = %self.config.reviewer_llm,
            "running GitHub reviewer"
        );

        // Run Gemini directly - it will use gh commands to create the review
        let output = Command::new("gemini")
            .current_dir(sandbox_path)
            .args(["--print", &review_prompt])
            .output()
            .map_err(|e| Error::Cruise(format!("failed to run gemini: {}", e)))?;

        let raw_response = String::from_utf8_lossy(&output.stdout).to_string();

        // Check if any reviews were created by looking at PR reviews
        let reviews_created = self.check_for_new_reviews(pr_number, repo, sandbox_path)?;

        // Determine verdict based on whether reviews requested changes
        let verdict = if reviews_created {
            // Check if the latest review requested changes
            if self.latest_review_requests_changes(pr_number, repo, sandbox_path)? {
                ReviewVerdict::NeedsChanges
            } else {
                ReviewVerdict::Approved
            }
        } else {
            // No review created, assume approved (reviewer found nothing)
            ReviewVerdict::Approved
        };

        let review = ReviewResult {
            verdict: verdict.clone(),
            suggestions: vec![], // Suggestions are in GitHub comments
            summary: format!("{} review completed", domain.as_str()),
        };

        // Record feedback
        self.observability
            .review_feedback
            .push(ReviewFeedbackRecord {
                iteration: 0,
                phase: Some(domain.as_str().to_string()),
                verdict: review.verdict.clone(),
                suggestion_count: 0, // Comments are on GitHub
                review: review.clone(),
                raw_response: raw_response.clone(),
                diff_reviewed: diff.to_string(),
            });

        // Extract security findings if this is a security review
        if domain == ReviewDomain::Security {
            self.extract_security_findings(&raw_response);
        }

        tracing::info!(
            domain = %domain.as_str(),
            verdict = ?review.verdict,
            "GitHub reviewer completed"
        );

        Ok(review)
    }

    /// Checks if new reviews were created on the PR.
    fn check_for_new_reviews(
        &self,
        pr_number: u64,
        _repo: &str,
        sandbox_path: &Path,
    ) -> Result<bool> {
        let output = Command::new("gh")
            .current_dir(sandbox_path)
            .args([
                "pr",
                "view",
                &pr_number.to_string(),
                "--json",
                "reviews",
                "-q",
                ".reviews | length",
            ])
            .output()
            .map_err(|e| Error::Cruise(format!("failed to check reviews: {}", e)))?;

        let count: u64 = String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse()
            .unwrap_or(0);

        Ok(count > 0)
    }

    /// Checks if the latest review requested changes.
    fn latest_review_requests_changes(
        &self,
        pr_number: u64,
        _repo: &str,
        sandbox_path: &Path,
    ) -> Result<bool> {
        let output = Command::new("gh")
            .current_dir(sandbox_path)
            .args([
                "pr",
                "view",
                &pr_number.to_string(),
                "--json",
                "reviews",
                "-q",
                ".reviews[-1].state",
            ])
            .output()
            .map_err(|e| Error::Cruise(format!("failed to check review state: {}", e)))?;

        let state = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(state == "CHANGES_REQUESTED")
    }

    /// Gets pending (unresolved) review comments from a PR.
    fn get_pending_review_comments(
        &self,
        pr_number: u64,
        _repo: &str,
        sandbox_path: &Path,
    ) -> Result<Vec<GitHubReviewComment>> {
        let output = Command::new("gh")
            .current_dir(sandbox_path)
            .args([
                "api",
                &format!("repos/{{owner}}/{{repo}}/pulls/{}/comments", pr_number),
                "--jq",
                r#".[] | select(.position != null) | {id: .id, path: .path, line: .line, body: .body}"#,
            ])
            .output()
            .map_err(|e| Error::Cruise(format!("failed to get PR comments: {}", e)))?;

        let raw = String::from_utf8_lossy(&output.stdout);
        let mut comments = Vec::new();

        // Parse JSONL output (one JSON object per line)
        for line in raw.lines() {
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(line) {
                let id = parsed.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
                let path = parsed
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let line_num = parsed.get("line").and_then(|v| v.as_u64()).map(|l| l as u32);
                let body = parsed
                    .get("body")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                if id > 0 && !path.is_empty() {
                    comments.push(GitHubReviewComment {
                        id,
                        path,
                        line: line_num,
                        body,
                        resolved: false,
                        resolved_by_commit: None,
                    });
                }
            }
        }

        Ok(comments)
    }

    /// Resolves a GitHub review comment by having the coder address it.
    async fn resolve_github_comment(
        &mut self,
        original_prompt: &str,
        _timeout: Duration,
        sandbox_path: &Path,
        pr_number: u64,
        repo: &str,
        comment: GitHubReviewComment,
    ) -> Result<()> {
        let comment_id = comment.id;

        // Build prompt for coder to resolve this comment
        let resolve_prompt =
            ResolveCommentPromptBuilder::new(pr_number, repo, comment.clone())
                .with_original_prompt(original_prompt)
                .build();

        // Record command line
        self.observability.command_lines.push(CommandLineRecord {
            llm: self.config.primary_llm.clone(),
            command: format!("claude --print \"resolve comment {}\"", comment_id),
            work_dir: sandbox_path.to_path_buf(),
            iteration: 0,
            role: "resolver".to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
        });

        tracing::info!(
            comment_id = comment_id,
            path = %comment.path,
            "resolving GitHub review comment"
        );

        // Run Claude to resolve the comment
        let output = Command::new("claude")
            .current_dir(sandbox_path)
            .args([
                "--print",
                "--verbose",
                "--output-format",
                "stream-json",
                &resolve_prompt,
            ])
            .output()
            .map_err(|e| Error::Cruise(format!("failed to run claude: {}", e)))?;

        if !output.status.success() {
            tracing::warn!(
                comment_id = comment_id,
                error = %String::from_utf8_lossy(&output.stderr),
                "failed to resolve comment"
            );
            return Ok(()); // Don't fail the whole workflow for one comment
        }

        // Commit and push changes
        if let Some(commit_hash) = self.commit_and_push_changes(
            sandbox_path,
            0,
            &self.config.primary_llm.clone(),
            Some(&format!("resolving-comment-{}", comment_id)),
        ) {
            // Record resolution
            self.observability
                .resolved_comments
                .push(ResolvedCommentRecord {
                    comment_id,
                    resolved_by_commit: commit_hash.clone(),
                    resolved_at: chrono::Utc::now().to_rfc3339(),
                    explanation: format!("Resolved in commit {}", commit_hash),
                });

            // Reply to the comment on GitHub
            let reply_body = format!("Fixed in commit {}", commit_hash);
            let _ = Command::new("gh")
                .current_dir(sandbox_path)
                .args([
                    "pr",
                    "comment",
                    &pr_number.to_string(),
                    "--body",
                    &reply_body,
                ])
                .output();
        }

        Ok(())
    }
}

/// Formats observability data as markdown for inclusion in PRs and beads issues.
pub fn format_observability_markdown(obs: &SpawnObservability) -> String {
    let mut md = String::new();

    md.push_str("## Spawn-Team Observability\n\n");

    // Command lines
    if !obs.command_lines.is_empty() {
        md.push_str("### LLM Invocations\n\n");
        md.push_str("| Iteration | Role | LLM | Timestamp |\n");
        md.push_str("|-----------|------|-----|----------|\n");
        for cmd in &obs.command_lines {
            md.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                cmd.iteration, cmd.role, cmd.llm, cmd.timestamp
            ));
        }
        md.push_str("\n");
    }

    // Commits
    if !obs.commits.is_empty() {
        md.push_str("### Commits\n\n");
        md.push_str("| Iteration | Commit | LLM | Pushed | Timestamp |\n");
        md.push_str("|-----------|--------|-----|--------|----------|\n");
        for commit in &obs.commits {
            let short_hash = commit.hash.chars().take(7).collect::<String>();
            let pushed_icon = if commit.pushed { "✓" } else { "✗" };
            md.push_str(&format!(
                "| {} | `{}` | {} | {} | {} |\n",
                commit.iteration, short_hash, commit.llm, pushed_icon, commit.timestamp
            ));
        }
        md.push_str("\n");
    }

    // Review feedback
    if !obs.review_feedback.is_empty() {
        md.push_str("### Review Feedback\n\n");
        for feedback in &obs.review_feedback {
            md.push_str(&format!(
                "#### Iteration {} - {} - {:?}\n\n",
                feedback.iteration,
                feedback.phase.as_deref().unwrap_or("General"),
                feedback.verdict
            ));
            md.push_str(&format!(
                "**Suggestions:** {}\n\n",
                feedback.suggestion_count
            ));

            if !feedback.review.suggestions.is_empty() {
                md.push_str("<details>\n<summary>View Suggestions</summary>\n\n");
                for suggestion in &feedback.review.suggestions {
                    md.push_str(&format!("- **{}**", suggestion.file));
                    if let Some(line) = suggestion.line {
                        md.push_str(&format!(" (line {})", line));
                    }
                    md.push_str(&format!(
                        "\n  - Issue: {}\n  - Fix: {}\n\n",
                        suggestion.issue, suggestion.suggestion
                    ));
                }
                md.push_str("</details>\n\n");
            }

            // Include diff reviewed (truncated)
            if !feedback.diff_reviewed.is_empty() {
                md.push_str("<details>\n<summary>Diff Reviewed</summary>\n\n```diff\n");
                let truncated: String = feedback.diff_reviewed.chars().take(2000).collect();
                md.push_str(&truncated);
                if feedback.diff_reviewed.len() > 2000 {
                    md.push_str("\n... (truncated)");
                }
                md.push_str("\n```\n</details>\n\n");
            }
        }
    }

    // Security findings
    if !obs.security_findings.is_empty() {
        md.push_str("### Security Findings\n\n");
        md.push_str("| Severity | Finding |\n");
        md.push_str("|----------|--------|\n");
        for finding in &obs.security_findings {
            md.push_str(&format!(
                "| {} | {} |\n",
                finding.severity, finding.description
            ));
        }
        md.push_str("\n");
    }

    // Permissions
    if !obs.permissions_requested.is_empty() {
        md.push_str("### Permissions\n\n");
        md.push_str("| Type | Resource | Granted | LLM |\n");
        md.push_str("|------|----------|---------|-----|\n");
        for perm in &obs.permissions_requested {
            md.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                perm.permission_type, perm.resource, perm.granted, perm.llm
            ));
        }
        md.push_str("\n");
    }

    md
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observability_default_is_empty() {
        let obs = SpawnObservability::default();
        assert!(obs.command_lines.is_empty());
        assert!(obs.review_feedback.is_empty());
        assert!(obs.security_findings.is_empty());
    }

    #[test]
    fn format_observability_markdown_handles_empty() {
        let obs = SpawnObservability::default();
        let md = format_observability_markdown(&obs);
        assert!(md.contains("Spawn-Team Observability"));
    }

    #[test]
    fn command_line_record_captures_data() {
        let record = CommandLineRecord {
            llm: "claude-code".to_string(),
            command: "claude --print".to_string(),
            work_dir: PathBuf::from("/tmp"),
            iteration: 1,
            role: "primary".to_string(),
            timestamp: "2024-01-01T00:00:00Z".to_string(),
        };
        assert_eq!(record.llm, "claude-code");
        assert_eq!(record.iteration, 1);
    }

    #[test]
    fn security_finding_captures_severity() {
        let finding = SecurityFinding {
            severity: "critical".to_string(),
            description: "SQL injection vulnerability".to_string(),
            file: Some("src/db.rs".to_string()),
            recommendation: "Use parameterized queries".to_string(),
        };
        assert_eq!(finding.severity, "critical");
    }
}
