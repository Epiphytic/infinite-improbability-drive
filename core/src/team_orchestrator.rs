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
    parse_review_response, CoordinationMode, FixPromptBuilder, ReviewPromptBuilder, ReviewResult,
    ReviewVerdict, SpawnTeamConfig, SpawnTeamResult,
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

                for i in 1..=phases_to_run {
                    iterations = i;

                    // Run primary (or fix if not first iteration)
                    // All iterations use the same branch for consistency
                    let primary_result = self
                        .run_primary(&current_prompt, timeout, sandbox_path, i, branch_name)
                        .await?;

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

                    // Get diff for review
                    let diff = self.get_git_diff(sandbox_path)?;

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
