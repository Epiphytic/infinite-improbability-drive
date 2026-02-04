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
    parse_review_response, CoordinationMode, ExtractedMetadata, FixPromptBuilder, GitHubReview,
    GitHubReviewComment, GitHubReviewPromptBuilder, ResolveCommentPromptBuilder, ReviewDomain,
    ReviewPromptBuilder, ReviewResult, ReviewVerdict, SpawnTeamConfig, SpawnTeamResult,
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
    /// Environment variables to pass to LLM processes.
    env_vars: std::collections::HashMap<String, String>,
}

impl<P: SandboxProvider + Clone + 'static> SpawnTeamOrchestrator<P> {
    /// Creates a new orchestrator.
    pub fn new(provider: P, logs_dir: PathBuf) -> Self {
        // Default env vars include FORK_JOIN_DISABLED to avoid conflicts
        let mut env_vars = std::collections::HashMap::new();
        env_vars.insert("FORK_JOIN_DISABLED".to_string(), "1".to_string());

        Self {
            config: SpawnTeamConfig::default(),
            provider,
            logs_dir,
            observability: SpawnObservability::default(),
            env_vars,
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

    /// Extracts PR metadata (title and branch name) from the task description.
    ///
    /// Uses a lightweight LLM call to extract meaningful PR title and branch name
    /// from the task description, avoiding ugly auto-generated names.
    pub fn extract_metadata(&self, task: &str, work_dir: &Path) -> Result<ExtractedMetadata> {
        let prompt = format!(
            r#"Extract a concise PR title and branch name from this task description.

Task: {}

Requirements:
- pr_title: A clear, concise title (max 72 chars) describing what the PR does
- branch_name: A kebab-case branch name (max 50 chars) like "feat/add-user-auth" or "fix/login-bug"

Respond with ONLY a JSON object (no markdown, no explanation):
{{"pr_title": "...", "branch_name": "..."}}"#,
            task.chars().take(500).collect::<String>()
        );

        tracing::debug!("extracting PR metadata from task description");

        // Use Claude for extraction (quick, lightweight call)
        let output = Command::new("claude")
            .current_dir(work_dir)
            .args(["--print", "-p", &prompt])
            .output()
            .map_err(|e| Error::Cruise(format!("failed to run claude for metadata extraction: {}", e)))?;

        let response = String::from_utf8_lossy(&output.stdout).to_string();

        // Parse JSON response
        let json_start = response.find('{');
        let json_end = response.rfind('}');

        if let (Some(start), Some(end)) = (json_start, json_end) {
            let json_str = &response[start..=end];
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(json_str) {
                let pr_title = parsed
                    .get("pr_title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Automated changes")
                    .chars()
                    .take(72)
                    .collect::<String>();

                let branch_name = parsed
                    .get("branch_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("feat/automated-changes")
                    .chars()
                    .take(50)
                    .collect::<String>()
                    // Sanitize branch name
                    .to_lowercase()
                    .replace(' ', "-")
                    .replace('_', "-");

                tracing::info!(
                    pr_title = %pr_title,
                    branch_name = %branch_name,
                    "extracted PR metadata"
                );

                return Ok(ExtractedMetadata {
                    pr_title,
                    branch_name,
                });
            }
        }

        // Fallback: generate from task
        tracing::warn!("failed to extract PR metadata, using fallback");
        let fallback_title: String = task.chars().take(70).collect();
        let fallback_branch = format!(
            "feat/{}",
            task.chars()
                .take(40)
                .collect::<String>()
                .to_lowercase()
                .replace(' ', "-")
                .replace(|c: char| !c.is_alphanumeric() && c != '-', "")
        );

        Ok(ExtractedMetadata {
            pr_title: fallback_title,
            branch_name: fallback_branch,
        })
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
    /// Both PingPong and GitHub modes share the same core flow:
    /// 1. Primary creates initial work (plan or implementation)
    /// 2. Watcher pulls changes, commits, pushes
    /// 3. **PR is created on first commit** (shared)
    /// 4. Review phases run: Security → Technical Feasibility → Task Granularity →
    ///    Dependency Completeness → General Polish
    /// 5. Each phase: reviewer reviews, coder fixes
    ///
    /// The only difference is how reviews are delivered:
    /// - PingPong: review as stdout, appended to PR body
    /// - GitHub: review as `gh pr review --request-changes` with line comments
    pub async fn run_with_branch(
        &mut self,
        prompt: &str,
        timeout: Duration,
        sandbox_path: &Path,
        branch_name: Option<&str>,
    ) -> Result<SpawnTeamResult> {
        let _start = Instant::now();
        let mut iterations = 0;
        let mut reviews = Vec::new();
        let mut final_verdict = None;

        // Get repo name for PR operations (needed for both modes)
        let repo = self.get_repo_name(sandbox_path)?;

        // Track the sandbox path across iterations
        let mut active_sandbox_path: Option<PathBuf> = None;
        let mut pr_url: Option<String> = None;

        // Determine if this is GitHub mode (affects how reviews are posted)
        let use_github_reviews = matches!(self.config.mode, CoordinationMode::GitHub);

        // Extract PR metadata (title and branch name) before starting workflow
        // This ensures meaningful PR titles and branch names
        let extracted_metadata = if branch_name.is_none() {
            match self.extract_metadata(prompt, sandbox_path) {
                Ok(meta) => Some(meta),
                Err(e) => {
                    tracing::warn!(error = %e, "failed to extract PR metadata, using fallback");
                    None
                }
            }
        } else {
            None // Branch name provided explicitly, don't extract
        };

        // Use extracted branch name if available
        let effective_branch_name = branch_name
            .map(|s| s.to_string())
            .or_else(|| extracted_metadata.as_ref().map(|m| m.branch_name.clone()));

        // Legacy sequential mode - kept for backwards compatibility
        if matches!(self.config.mode, CoordinationMode::Sequential) {
            return self
                .run_sequential_mode(prompt, timeout, sandbox_path, branch_name)
                .await;
        }

        // ========================================
        // SHARED FLOW: PingPong and GitHub modes
        // ========================================

        // Step 1: Run primary to create initial work
        tracing::info!(
            mode = ?self.config.mode,
            "starting spawn-team: running primary LLM"
        );

        let primary_result = self
            .run_primary(prompt, timeout, sandbox_path, 1, effective_branch_name.as_deref())
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

        // Capture sandbox path for observability
        if let Some(ref path) = primary_result.sandbox_path {
            active_sandbox_path = Some(path.clone());
        }

        // IMPORTANT: We have two paths with different purposes:
        // - work_path (original repo): Used for PR operations via `gh` CLI and branch checking
        //   since these operate on remote refs and don't need local checkout
        // - worktree_path: Used for operations that modify files and commit,
        //   since it has the feature branch checked out
        let work_path = sandbox_path;

        // The worktree path is needed for operations that commit to the feature branch
        // Create a longer-lived binding if we need to use the sandbox_path as fallback
        let fallback_path = sandbox_path.to_path_buf();
        let worktree_path = active_sandbox_path.as_ref().unwrap_or(&fallback_path);

        // Step 2: CREATE PR ON FIRST COMMIT (shared by both modes)
        // First, verify there are commits to create a PR for
        if !self.has_commits_on_branch(work_path)? {
            tracing::warn!("primary LLM didn't create any commits, cannot create PR");
            return Ok(SpawnTeamResult {
                success: false,
                iterations: 1,
                final_verdict: None,
                reviews,
                summary: "Primary LLM completed but made no commits. No code changes were produced.".to_string(),
            });
        }

        tracing::info!("creating PR on first commit");
        let created_pr_url = self.create_pr_on_first_commit(work_path, prompt, &repo, extracted_metadata.as_ref())?;
        self.observability.pr_url = Some(created_pr_url.clone());
        pr_url = Some(created_pr_url.clone());

        // Extract PR number for API operations
        let extracted_pr_number = self.extract_pr_number(&created_pr_url)?;

        tracing::info!(
            pr_url = %created_pr_url,
            pr_number = extracted_pr_number,
            "PR created on first commit"
        );

        // Step 3: Run review phases
        // All 5 phases: Security, TechnicalFeasibility, TaskGranularity,
        // DependencyCompleteness, GeneralPolish
        let review_phases = ReviewDomain::all();
        let phases_to_run = review_phases.len().min(self.config.max_iterations as usize);
        let mut current_prompt = prompt.to_string();

        for (phase_idx, domain) in review_phases.iter().take(phases_to_run).enumerate() {
            iterations = (phase_idx + 1) as u32;
            let is_last_phase = phase_idx == phases_to_run - 1;

            tracing::info!(
                domain = %domain.as_str(),
                iteration = iterations,
                is_last_phase = is_last_phase,
                use_github_reviews = use_github_reviews,
                "starting review phase"
            );

            // Get current diff for this phase - use worktree where feature branch is checked out
            let diff = self.get_git_diff(worktree_path)?;

            // Run review - method differs based on mode
            let review_result = if use_github_reviews {
                // GitHub mode: reviewer creates GitHub review with line comments
                self.run_github_reviewer(
                    &current_prompt,
                    &diff,
                    timeout,
                    worktree_path,
                    *domain,
                    extracted_pr_number,
                    &repo,
                )
                .await?
            } else {
                // PingPong mode: reviewer outputs to stdout
                // Use worktree_path where feature branch files are available
                let review = self
                    .run_reviewer(
                        &current_prompt,
                        &diff,
                        timeout,
                        worktree_path,
                        iterations,
                        Some(domain.as_str()),
                    )
                    .await?;

                // Append review to PR body for traceability
                if let Some(ref url) = pr_url {
                    self.append_review_to_pr(url, &review, domain)?;
                }

                review
            };

            reviews.push(review_result.clone());
            final_verdict = Some(review_result.verdict.clone());

            // If review requested changes, run coder to fix
            if review_result.verdict == ReviewVerdict::NeedsChanges {
                tracing::info!(
                    domain = %domain.as_str(),
                    suggestions = review_result.suggestions.len(),
                    "reviewer requested changes, running coder to fix"
                );

                if use_github_reviews {
                    // GitHub mode: resolve each comment with commits
                    let pending_comments = self.get_pending_review_comments(
                        extracted_pr_number,
                        &repo,
                        work_path,
                    )?;

                    if pending_comments.is_empty() {
                        // Reviewer said NEEDS CHANGES but didn't post line-specific comments.
                        // This means the feedback is general (in the PR comment body).
                        // We'll continue to the next review phase since we can't auto-fix
                        // without specific file/line guidance.
                        tracing::warn!(
                            domain = %domain.as_str(),
                            "reviewer requested changes but no line-specific comments found - continuing to next phase"
                        );
                    } else {
                        for comment in pending_comments {
                            tracing::info!(
                                comment_id = comment.id,
                                path = %comment.path,
                                "resolving GitHub review comment"
                            );

                            // Use worktree_path for commits to go to the feature branch
                            self.resolve_github_comment(
                                &current_prompt,
                                timeout,
                                worktree_path,
                                extracted_pr_number,
                                &repo,
                                comment,
                            )
                            .await?;
                        }
                    }
                } else {
                    // PingPong mode: run fix phase
                    // Use worktree_path for commits to go to the feature branch
                    let _fix_result = self
                        .run_fix(
                            &current_prompt,
                            &review_result,
                            timeout,
                            worktree_path,
                            iterations,
                            None, // Use existing branch
                        )
                        .await?;
                }

                // Update prompt with suggestions for next phase
                current_prompt = FixPromptBuilder::new(prompt)
                    .with_suggestions(review_result.suggestions.clone())
                    .build();
            } else {
                tracing::info!(
                    domain = %domain.as_str(),
                    "reviewer approved this phase"
                );
            }

            // Special handling for General Polish phase (last phase)
            if is_last_phase && use_github_reviews {
                // GitHub mode: check for user comments and assess if work is complete
                // IMPORTANT: Use worktree_path (not work_path) so commits go to the PR branch
                self.handle_general_polish_github(
                    &current_prompt,
                    timeout,
                    worktree_path,
                    extracted_pr_number,
                    &repo,
                )
                .await?;
            }
        }

        // Determine final success
        let success = final_verdict
            .as_ref()
            .map(|v| *v == ReviewVerdict::Approved)
            .unwrap_or(false);

        let summary = format!(
            "Spawn-team completed. PR: {}. {} phases reviewed. Mode: {:?}",
            pr_url.as_deref().unwrap_or("none"),
            iterations,
            self.config.mode
        );

        Ok(SpawnTeamResult {
            success,
            iterations,
            final_verdict,
            reviews,
            summary,
        })
    }

    /// Legacy sequential mode for backwards compatibility.
    async fn run_sequential_mode(
        &mut self,
        prompt: &str,
        timeout: Duration,
        sandbox_path: &Path,
        branch_name: Option<&str>,
    ) -> Result<SpawnTeamResult> {
        let mut reviews = Vec::new();

        // Run primary
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

        // Get diff for review
        let diff = self.get_git_diff(sandbox_path)?;

        // Run reviewer
        let review_result = self
            .run_reviewer(prompt, &diff, timeout, sandbox_path, 1, None)
            .await?;

        let final_verdict = Some(review_result.verdict.clone());
        reviews.push(review_result.clone());

        // If needs changes, run fix phase
        if review_result.verdict == ReviewVerdict::NeedsChanges {
            let _fix_result = self
                .run_fix(prompt, &review_result, timeout, sandbox_path, 1, branch_name)
                .await?;
        }

        let success = final_verdict
            .as_ref()
            .map(|v| *v == ReviewVerdict::Approved)
            .unwrap_or(false);

        Ok(SpawnTeamResult {
            success,
            iterations: 1,
            final_verdict,
            reviews,
            summary: "Sequential mode completed".to_string(),
        })
    }

    /// Appends a review to the PR body (for PingPong mode traceability).
    fn append_review_to_pr(
        &self,
        pr_url: &str,
        review: &ReviewResult,
        domain: &ReviewDomain,
    ) -> Result<()> {
        // Extract PR number from URL
        let pr_number = self.extract_pr_number(pr_url)?;

        // Get current PR body
        let output = Command::new("gh")
            .args(["pr", "view", &pr_number.to_string(), "--json", "body", "-q", ".body"])
            .output()
            .map_err(|e| Error::Cruise(format!("failed to get PR body: {}", e)))?;

        let current_body = String::from_utf8_lossy(&output.stdout).trim().to_string();

        // Build review section
        let verdict_emoji = match review.verdict {
            ReviewVerdict::Approved => "✅",
            ReviewVerdict::NeedsChanges => "🔄",
            ReviewVerdict::Failed => "❌",
        };

        let mut review_section = format!(
            "\n\n---\n## {} Review: {} {}\n\n**Summary:** {}\n",
            verdict_emoji,
            domain.as_str(),
            verdict_emoji,
            review.summary
        );

        if !review.suggestions.is_empty() {
            review_section.push_str("\n### Suggestions\n\n");
            for (i, suggestion) in review.suggestions.iter().enumerate() {
                review_section.push_str(&format!(
                    "{}. **{}**",
                    i + 1,
                    suggestion.file
                ));
                if let Some(line) = suggestion.line {
                    review_section.push_str(&format!(" (line {})", line));
                }
                review_section.push_str(&format!(
                    "\n   - Issue: {}\n   - Suggestion: {}\n\n",
                    suggestion.issue, suggestion.suggestion
                ));
            }
        }

        // Update PR body
        let new_body = format!("{}{}", current_body, review_section);
        let edit_output = Command::new("gh")
            .args(["pr", "edit", &pr_number.to_string(), "--body", &new_body])
            .output()
            .map_err(|e| Error::Cruise(format!("failed to update PR body: {}", e)))?;

        if !edit_output.status.success() {
            tracing::warn!(
                error = %String::from_utf8_lossy(&edit_output.stderr),
                "failed to append review to PR body"
            );
        }

        Ok(())
    }

    /// Handles the General Polish phase for GitHub mode.
    ///
    /// In this phase:
    /// - Check if the planner did what was asked
    /// - Look for any user comments and resolve them
    /// - Re-open or create new comments as needed
    async fn handle_general_polish_github(
        &mut self,
        prompt: &str,
        timeout: Duration,
        worktree_path: &Path,
        pr_number: u64,
        repo: &str,
    ) -> Result<()> {
        tracing::info!(
            pr_number = pr_number,
            "GitHub mode: handling general polish - checking user comments"
        );

        // Get all comments on the PR (including user comments)
        // Note: gh CLI works from any directory, but we use worktree for consistency
        let comments_output = Command::new("gh")
            .current_dir(worktree_path)
            .args([
                "api",
                &format!("repos/{}/pulls/{}/comments", repo, pr_number),
            ])
            .output()
            .map_err(|e| Error::Cruise(format!("failed to get PR comments: {}", e)))?;

        if !comments_output.status.success() {
            tracing::warn!(
                error = %String::from_utf8_lossy(&comments_output.stderr),
                "failed to get PR comments for general polish"
            );
            return Ok(());
        }

        // Parse comments to find user comments (not from bot/automation)
        let comments_json = String::from_utf8_lossy(&comments_output.stdout);
        if let Ok(comments) = serde_json::from_str::<Vec<serde_json::Value>>(&comments_json) {
            let user_comments: Vec<_> = comments
                .iter()
                .filter(|c| {
                    // Filter for unresolved user comments
                    // Check if the comment is not from a bot and has no resolution
                    let user_type = c.get("user")
                        .and_then(|u| u.get("type"))
                        .and_then(|t| t.as_str())
                        .unwrap_or("User");
                    user_type == "User"
                })
                .collect();

            if !user_comments.is_empty() {
                tracing::info!(
                    user_comments = user_comments.len(),
                    "found user comments to address in general polish"
                );

                // Build prompt for coder to address user comments
                for comment in user_comments {
                    let comment_body = comment.get("body")
                        .and_then(|b| b.as_str())
                        .unwrap_or("");
                    let comment_path = comment.get("path")
                        .and_then(|p| p.as_str())
                        .unwrap_or("unknown");
                    let comment_id = comment.get("id")
                        .and_then(|id| id.as_u64())
                        .unwrap_or(0);

                    if comment_body.is_empty() {
                        continue;
                    }

                    // Create a synthetic GitHubReviewComment for the resolver
                    let user_comment = GitHubReviewComment {
                        id: comment_id,
                        path: comment_path.to_string(),
                        line: comment.get("line").and_then(|l| l.as_u64()).map(|l| l as u32),
                        body: comment_body.to_string(),
                        resolved: false,
                        resolved_by_commit: None,
                    };

                    tracing::info!(
                        comment_id = comment_id,
                        path = %comment_path,
                        "resolving user comment in general polish"
                    );

                    // Resolve the user comment using worktree_path so commits go to PR branch
                    self.resolve_github_comment(
                        prompt,
                        timeout,
                        worktree_path,
                        pr_number,
                        repo,
                        user_comment,
                    )
                    .await?;
                }
            }
        }

        Ok(())
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
            command: command.clone(),
            work_dir: sandbox_path.to_path_buf(),
            iteration,
            role: "primary".to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
        });

        crate::debug::debug_llm_invocation(
            &self.config.primary_llm,
            prompt,
            sandbox_path,
            iteration,
            "primary",
        );

        let spawner = Spawner::new(
            self.provider.clone(),
            self.logs_dir.join(format!("primary-{}", iteration)),
        );

        let config = SpawnConfig::new(prompt)
            .with_total_timeout(timeout)
            .with_max_escalations(self.config.max_escalations);
        let manifest = SandboxManifest::with_sensible_defaults();

        tracing::info!(
            iteration = iteration,
            llm = %self.config.primary_llm,
            branch = ?branch_name,
            "running primary LLM"
        );

        let result = spawner.spawn_with_branch(config, manifest, Box::new(runner), branch_name).await;

        // Handle spawn result
        let result = match result {
            Ok(r) => r,
            Err(e) => {
                if crate::debug::is_debug() {
                    eprintln!("[CRUISE_DEBUG] Primary LLM spawn FAILED: {}", e);
                }
                if crate::debug::is_fail_fast() {
                    return Err(e);
                }
                return Err(e);
            }
        };

        if crate::debug::is_debug() {
            eprintln!("[CRUISE_DEBUG] Primary LLM spawn result:");
            eprintln!("  status: {:?}", result.status);
            eprintln!("  duration: {:?}", result.duration);
            eprintln!("  sandbox_path: {:?}", result.sandbox_path);
        }

        // Capture the actual sandbox path where work was done
        if let Some(ref sandbox) = result.sandbox_path {
            self.observability.sandbox_path = Some(sandbox.clone());
            // Update the command line record with the actual work directory
            if let Some(last_cmd) = self.observability.command_lines.last_mut() {
                last_cmd.work_dir = sandbox.clone();
            }

            // Commit and push changes even on partial completion (timeout)
            // This preserves partial work that would otherwise be lost
            self.commit_and_push_changes(
                sandbox,
                iteration,
                &self.config.primary_llm.clone(),
                None,
            );
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
        // IMPORTANT: -p flag is required for --print mode
        // IMPORTANT: --permission-mode acceptEdits allows Claude to make file edits
        let output = std::process::Command::new("claude")
            .current_dir(sandbox_path)
            .envs(&self.env_vars)
            .args(["--print", "--verbose", "--output-format", "stream-json", "--permission-mode", "acceptEdits", "-p", prompt])
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
        // Use --yolo for auto-approval with --allowed-tools for Read access
        let mut gemini_args = vec![
            "--yolo".to_string(),  // Auto-approve all actions
            "--output-format".to_string(), "stream-json".to_string(),  // Structured output for debugging
            "--allowed-tools".to_string(), "Read,Glob,Grep".to_string(),  // Read-only tools for review
        ];

        // Add model if specified (for testing, use gemini-3-flash-preview)
        if let Some(ref model) = self.config.reviewer_model {
            gemini_args.push("--model".to_string());
            gemini_args.push(model.clone());
        }

        gemini_args.push("-p".to_string());
        gemini_args.push(review_prompt.clone());

        let args_refs: Vec<&str> = gemini_args.iter().map(|s| s.as_str()).collect();
        crate::debug::debug_command("gemini", &args_refs, sandbox_path);

        let output = Command::new("gemini")
            .current_dir(sandbox_path)
            .envs(&self.env_vars)
            .args(&gemini_args)
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
    /// IMPORTANT: Always pushes if there are local commits ahead of remote,
    /// even if Claude already committed the changes itself.
    fn commit_and_push_changes(
        &mut self,
        sandbox_path: &Path,
        iteration: u32,
        llm: &str,
        phase: Option<&str>,
    ) -> Option<String> {
        // Get branch name first - needed for both commit and push
        let branch_output = Command::new("git")
            .current_dir(sandbox_path)
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .output()
            .ok()?;

        let branch = String::from_utf8_lossy(&branch_output.stdout).trim().to_string();

        // Check if there are uncommitted changes to commit
        let status_output = Command::new("git")
            .current_dir(sandbox_path)
            .args(["status", "--porcelain"])
            .output()
            .ok()?;

        let status = String::from_utf8_lossy(&status_output.stdout);
        let has_uncommitted = !status.trim().is_empty();

        let hash = if has_uncommitted {
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
                // Continue to push check even if staging fails
            } else {
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
                }
            }

            // Get commit hash
            let hash_output = Command::new("git")
                .current_dir(sandbox_path)
                .args(["rev-parse", "HEAD"])
                .output()
                .ok()?;

            String::from_utf8_lossy(&hash_output.stdout).trim().to_string()
        } else {
            tracing::debug!(iteration = iteration, "no uncommitted changes");

            // Get current HEAD hash even if we didn't make a new commit
            // (Claude might have committed directly)
            let hash_output = Command::new("git")
                .current_dir(sandbox_path)
                .args(["rev-parse", "HEAD"])
                .output()
                .ok()?;

            String::from_utf8_lossy(&hash_output.stdout).trim().to_string()
        };

        // ALWAYS check if we need to push - Claude may have committed but not pushed
        // Check for commits ahead of remote
        let ahead_output = Command::new("git")
            .current_dir(sandbox_path)
            .args(["rev-list", "--count", &format!("origin/{}..HEAD", branch)])
            .output();

        let commits_ahead = ahead_output
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| s.trim().parse::<u64>().ok())
            .unwrap_or(0);

        // Also check if the remote branch exists at all (for new branches)
        let remote_exists = Command::new("git")
            .current_dir(sandbox_path)
            .args(["ls-remote", "--heads", "origin", &branch])
            .output()
            .ok()
            .map(|o| !o.stdout.is_empty())
            .unwrap_or(false);

        let needs_push = commits_ahead > 0 || !remote_exists;

        if !needs_push {
            tracing::debug!(
                iteration = iteration,
                branch = %branch,
                "branch is up to date with remote"
            );
            return Some(hash);
        }

        tracing::info!(
            commits_ahead = commits_ahead,
            remote_exists = remote_exists,
            branch = %branch,
            "pushing commits to remote"
        );

        let push_output = Command::new("git")
            .current_dir(sandbox_path)
            .args(["push", "-u", "origin", &branch])
            .output()
            .ok();

        let pushed = match &push_output {
            Some(output) => {
                if !output.status.success() {
                    tracing::error!(
                        branch = %branch,
                        stderr = %String::from_utf8_lossy(&output.stderr),
                        "git push failed"
                    );
                }
                output.status.success()
            }
            None => {
                tracing::error!("git push command failed to execute");
                false
            }
        };

        // Create commit message for observability record
        let phase_str = phase.map(|p| format!(" - {}", p)).unwrap_or_default();
        let record_message = format!(
            "[cruise-control] {} iteration {}{}",
            llm, iteration, phase_str
        );

        // Record commit
        self.observability.commits.push(CommitRecord {
            hash: hash.clone(),
            message: record_message,
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

    /// Checks if the pushed branch has commits different from the default branch.
    ///
    /// Returns true if there are commits that can be used to create a PR.
    /// This checks remote refs since the sandbox worktree may be cleaned up.
    /// Includes retry logic to handle timing issues with remote refs.
    fn has_commits_on_branch(&self, repo_path: &Path) -> Result<bool> {
        // Get default branch name
        let default_branch = self.get_default_branch(repo_path)?;

        // Retry up to 3 times with delays to handle timing issues
        // Remote refs might not be immediately visible after push from worktree
        let max_retries = 3;
        let mut work_branch: Option<String> = None;

        for attempt in 1..=max_retries {
            // Fetch to ensure we have latest refs
            let _ = Command::new("git")
                .current_dir(repo_path)
                .args(["fetch", "origin"])
                .output();

            // List remote branches that aren't the default branch
            let branch_output = Command::new("git")
                .current_dir(repo_path)
                .args(["branch", "-r", "--list", "origin/*"])
                .output()
                .map_err(|e| Error::Git(format!("failed to list remote branches: {}", e)))?;

            let branches_str = String::from_utf8_lossy(&branch_output.stdout);

            // Find work branches (feat/, feature/, plan/, etc.) - anything that's not the default branch
            work_branch = branches_str
                .lines()
                .map(|s| s.trim())
                .find(|b| {
                    let stripped = b.trim_start_matches("origin/");
                    stripped.starts_with("feat") ||
                    stripped.starts_with("feature") ||
                    stripped.starts_with("plan") ||
                    stripped.starts_with("impl")
                })
                .map(|s| s.to_string());

            if work_branch.is_some() {
                break;
            }

            if attempt < max_retries {
                tracing::debug!(
                    attempt = attempt,
                    "no work branch found, retrying after delay"
                );
                std::thread::sleep(std::time::Duration::from_secs(2));
            }
        }

        let Some(work_branch) = work_branch else {
            tracing::warn!("no work branch found on remote after {} attempts", max_retries);
            return Ok(false);
        };

        // Count commits between default and work branch
        let count_output = Command::new("git")
            .current_dir(repo_path)
            .args([
                "rev-list",
                "--count",
                &format!("origin/{}..{}", default_branch, work_branch),
            ])
            .output()
            .map_err(|e| Error::Git(format!("failed to count commits: {}", e)))?;

        let count: u64 = String::from_utf8_lossy(&count_output.stdout)
            .trim()
            .parse()
            .unwrap_or(0);

        tracing::info!(
            default_branch = %default_branch,
            work_branch = %work_branch,
            commit_count = count,
            "checked for commits on branch"
        );

        Ok(count > 0)
    }

    /// Creates a PR on the first commit to the branch.
    ///
    /// Since the sandbox worktree may be cleaned up, this finds the feature branch
    /// from remote refs instead of using the local HEAD.
    fn create_pr_on_first_commit(
        &self,
        repo_path: &Path,
        prompt: &str,
        repo: &str,
        metadata: Option<&ExtractedMetadata>,
    ) -> Result<String> {
        // Get default branch
        let default_branch = self.get_default_branch(repo_path)?;

        // Fetch to ensure we have latest refs
        let _ = Command::new("git")
            .current_dir(repo_path)
            .args(["fetch", "origin"])
            .output();

        // Find the work branch from remote refs (feat/, feature/, plan/, impl/)
        let branch_output = Command::new("git")
            .current_dir(repo_path)
            .args(["branch", "-r", "--list", "origin/*"])
            .output()
            .map_err(|e| Error::Git(format!("failed to list remote branches: {}", e)))?;

        let branches_str = String::from_utf8_lossy(&branch_output.stdout);
        let work_branch = branches_str
            .lines()
            .map(|s| s.trim())
            .find(|b| {
                let stripped = b.trim_start_matches("origin/");
                stripped.starts_with("feat") ||
                stripped.starts_with("feature") ||
                stripped.starts_with("plan") ||
                stripped.starts_with("impl")
            })
            .map(|s| s.trim_start_matches("origin/").to_string());

        let Some(branch) = work_branch else {
            return Err(Error::Git("no work branch found on remote".to_string()));
        };

        tracing::info!(
            branch = %branch,
            default_branch = %default_branch,
            "creating PR from remote work branch"
        );

        // Create PR title - prefer extracted metadata, fallback to prompt
        let title = if let Some(meta) = metadata {
            meta.pr_title.clone()
        } else {
            let title: String = prompt.chars().take(70).collect();
            if prompt.len() > 70 {
                format!("{}...", title)
            } else {
                title
            }
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
            .current_dir(repo_path)
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

        // Log review phase in debug mode
        crate::debug::debug_review_phase(domain.as_str(), pr_number, diff.len());

        // Run Gemini with proper CLI arguments to execute the review
        // Using --yolo for auto-approval (--approval-mode plan requires experimental flag)
        // Include Bash in --allowed-tools for `gh pr comment` access
        let mut gemini_args = vec![
            "--yolo".to_string(),  // Auto-approve all actions
            "--output-format".to_string(), "stream-json".to_string(),  // Structured output for debugging
            "--allowed-tools".to_string(), "Read,Glob,Grep,Bash".to_string(),  // Need Bash for gh pr comment
        ];

        // Add model if specified (for testing, use gemini-3-flash-preview)
        if let Some(ref model) = self.config.reviewer_model {
            gemini_args.push("--model".to_string());
            gemini_args.push(model.clone());
        }

        gemini_args.push("-p".to_string());
        gemini_args.push(review_prompt.clone());

        let args_refs: Vec<&str> = gemini_args.iter().map(|s| s.as_str()).collect();
        crate::debug::debug_command("gemini", &args_refs, sandbox_path);

        let output = Command::new("gemini")
            .current_dir(sandbox_path)
            .envs(&self.env_vars)
            .args(&gemini_args)
            .output()
            .map_err(|e| Error::Cruise(format!("failed to run gemini: {}", e)))?;

        let raw_response = String::from_utf8_lossy(&output.stdout).to_string();
        let raw_stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let gemini_success = output.status.success();

        tracing::debug!(
            domain = %domain.as_str(),
            exit_code = ?output.status.code(),
            stdout_len = raw_response.len(),
            stderr_len = raw_stderr.len(),
            "gemini reviewer finished"
        );

        // In debug mode, print stderr if there was any
        if crate::debug::is_debug() && !raw_stderr.is_empty() {
            eprintln!("[CRUISE_DEBUG] Gemini stderr:\n{}", raw_stderr);
        }

        // In fail-fast mode, abort if gemini failed
        if !gemini_success && crate::debug::is_fail_fast() {
            return Err(Error::Cruise(format!(
                "Gemini reviewer failed in {} phase. Exit code: {:?}. Stderr: {}",
                domain.as_str(),
                output.status.code(),
                raw_stderr
            )));
        }

        // Check if any review comments were created on the PR
        let reviews_created = self.check_for_new_reviews(pr_number, repo, sandbox_path)?;

        // Determine verdict based on review comments
        let verdict = if reviews_created {
            // Check if the latest review requested changes
            if self.latest_review_requests_changes(pr_number, repo, sandbox_path)? {
                ReviewVerdict::NeedsChanges
            } else {
                ReviewVerdict::Approved
            }
        } else {
            // No review comment was created by Gemini
            // This could mean: approved with no issues, OR Gemini failed to post
            // Always post a status comment for traceability
            let status_comment = if gemini_success {
                format!(
                    "[{} REVIEW - APPROVED]\n\n{} review completed. No issues found.",
                    domain.as_str().to_uppercase(),
                    domain.as_str()
                )
            } else {
                format!(
                    "[{} REVIEW - SKIPPED]\n\nReviewer did not complete successfully. Manual review recommended.",
                    domain.as_str().to_uppercase()
                )
            };

            // Post the status comment to the PR
            let _ = Command::new("gh")
                .current_dir(sandbox_path)
                .args([
                    "pr", "comment",
                    &pr_number.to_string(),
                    "--repo", repo,
                    "--body", &status_comment,
                ])
                .output();

            if gemini_success {
                ReviewVerdict::Approved
            } else {
                // Reviewer failed, but don't block - just note it
                ReviewVerdict::Approved
            }
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

    /// Checks if new review comments were created on the PR.
    ///
    /// Looks for comments with review markers like `[SECURITY REVIEW - NEEDS CHANGES]`
    /// or `[SECURITY REVIEW - APPROVED]` since we use comments instead of formal reviews
    /// (GitHub doesn't allow the same user to submit request-changes reviews on own PRs).
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
                "comments",
                "-q",
                ".comments | length",
            ])
            .output()
            .map_err(|e| Error::Cruise(format!("failed to check comments: {}", e)))?;

        let count: u64 = String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse()
            .unwrap_or(0);

        Ok(count > 0)
    }

    /// Checks if the latest review comment indicates changes are needed.
    ///
    /// Looks for comments containing `[* REVIEW - NEEDS CHANGES]` marker.
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
                "comments",
                "-q",
                ".comments[-1].body",
            ])
            .output()
            .map_err(|e| Error::Cruise(format!("failed to check comment body: {}", e)))?;

        let body = String::from_utf8_lossy(&output.stdout).trim().to_string();

        // Check if the comment contains a "NEEDS CHANGES" marker
        Ok(body.contains("REVIEW - NEEDS CHANGES"))
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
        // IMPORTANT: -p flag is required for --print mode
        // IMPORTANT: --permission-mode acceptEdits allows Claude to make file edits
        let output = Command::new("claude")
            .current_dir(sandbox_path)
            .envs(&self.env_vars)
            .args([
                "--print",
                "--verbose",
                "--output-format",
                "stream-json",
                "--permission-mode",
                "acceptEdits",
                "-p",
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
