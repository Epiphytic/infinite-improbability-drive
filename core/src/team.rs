//! Spawn-team coordination for multi-LLM workflows.
//!
//! Supports sequential and ping-pong coordination modes
//! for primary/reviewer LLM interactions.

use serde::{Deserialize, Serialize};

/// Coordination mode for spawn-team.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CoordinationMode {
    /// Sequential mode: Primary runs, then reviewer reviews, then primary fixes.
    Sequential,
    /// Ping-pong mode: Iterative back-and-forth until approved.
    PingPong,
    /// GitHub mode (default): PR-based coordination with GitHub reviews.
    /// - PR is created on first commit
    /// - Reviewer LLMs create GitHub reviews with "request changes" and line comments
    /// - Coder LLM resolves comments with commits
    /// - Full traceability on the PR
    #[default]
    GitHub,
}

/// Configuration for spawn-team coordination.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnTeamConfig {
    /// Coordination mode.
    #[serde(default)]
    pub mode: CoordinationMode,
    /// Maximum iterations for ping-pong mode.
    #[serde(default = "default_max_iterations")]
    pub max_iterations: u32,
    /// Primary LLM identifier (e.g., "claude-code").
    #[serde(default = "default_primary_llm")]
    pub primary_llm: String,
    /// Primary LLM model to use (e.g., "sonnet"). If None, use CLI default.
    #[serde(default)]
    pub primary_model: Option<String>,
    /// Reviewer LLM identifier (e.g., "gemini-cli").
    #[serde(default = "default_reviewer_llm")]
    pub reviewer_llm: String,
    /// Reviewer LLM model to use (e.g., "gemini-3-flash-preview"). If None, use CLI default.
    #[serde(default)]
    pub reviewer_model: Option<String>,
    /// Maximum permission escalations allowed per spawn.
    #[serde(default = "default_max_escalations")]
    pub max_escalations: u32,
    /// Maximum number of reviewer LLMs running concurrently in GitHub mode.
    /// Default: 3. Set to 1 for sequential behavior.
    #[serde(default = "default_max_concurrent_reviewers")]
    pub max_concurrent_reviewers: u32,
}

fn default_max_iterations() -> u32 {
    3
}

fn default_primary_llm() -> String {
    "claude-code".to_string()
}

fn default_reviewer_llm() -> String {
    "gemini-cli".to_string()
}

fn default_max_escalations() -> u32 {
    5 // Allow 5 escalations for complex tasks
}

fn default_max_concurrent_reviewers() -> u32 {
    3
}

impl Default for SpawnTeamConfig {
    fn default() -> Self {
        Self {
            mode: CoordinationMode::default(),
            max_iterations: default_max_iterations(),
            primary_llm: default_primary_llm(),
            primary_model: None, // Use CLI default
            reviewer_llm: default_reviewer_llm(),
            reviewer_model: None, // Use CLI default
            max_escalations: default_max_escalations(),
            max_concurrent_reviewers: default_max_concurrent_reviewers(),
        }
    }
}

/// Verdict from a reviewer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewVerdict {
    /// Changes are approved.
    Approved,
    /// Changes need modifications.
    NeedsChanges,
    /// Review failed or couldn't complete.
    Failed,
}

/// A suggestion from the reviewer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewSuggestion {
    /// File path the suggestion applies to.
    pub file: String,
    /// Line number (if applicable).
    pub line: Option<u32>,
    /// Description of the issue.
    pub issue: String,
    /// Suggested fix.
    pub suggestion: String,
}

/// Result of a review phase.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewResult {
    /// The verdict.
    pub verdict: ReviewVerdict,
    /// Suggestions for improvements.
    pub suggestions: Vec<ReviewSuggestion>,
    /// Summary of the review.
    pub summary: String,
}

/// A GitHub review comment with line-specific feedback.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubReviewComment {
    /// The GitHub comment ID.
    pub id: u64,
    /// File path the comment applies to.
    pub path: String,
    /// Line number in the diff.
    pub line: Option<u32>,
    /// The comment body.
    pub body: String,
    /// Whether the comment has been resolved.
    pub resolved: bool,
    /// Commit SHA that resolved this comment (if resolved).
    pub resolved_by_commit: Option<String>,
}

/// A GitHub review with its state and comments.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubReview {
    /// The GitHub review ID.
    pub id: u64,
    /// Review state: CHANGES_REQUESTED, APPROVED, COMMENTED.
    pub state: String,
    /// Review domain (Security, TechnicalFeasibility, etc.).
    pub domain: String,
    /// Review body/summary.
    pub body: String,
    /// Line-specific comments in this review.
    pub comments: Vec<GitHubReviewComment>,
    /// Timestamp of the review.
    pub submitted_at: String,
}

/// Domains for specialized review passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewDomain {
    /// Security review: auth, injection, OWASP top 10.
    Security,
    /// Technical feasibility: architecture, performance.
    TechnicalFeasibility,
    /// Task granularity: appropriate sizing for parallel execution.
    TaskGranularity,
    /// Dependency completeness: are all dependencies identified.
    DependencyCompleteness,
    /// General polish: code quality, documentation.
    GeneralPolish,
}

impl ReviewDomain {
    /// Returns all review domains in order.
    pub fn all() -> &'static [ReviewDomain] {
        &[
            ReviewDomain::Security,
            ReviewDomain::TechnicalFeasibility,
            ReviewDomain::TaskGranularity,
            ReviewDomain::DependencyCompleteness,
            ReviewDomain::GeneralPolish,
        ]
    }

    /// Returns the domain name as a string.
    pub fn as_str(&self) -> &'static str {
        match self {
            ReviewDomain::Security => "Security",
            ReviewDomain::TechnicalFeasibility => "TechnicalFeasibility",
            ReviewDomain::TaskGranularity => "TaskGranularity",
            ReviewDomain::DependencyCompleteness => "DependencyCompleteness",
            ReviewDomain::GeneralPolish => "GeneralPolish",
        }
    }

    /// Returns instructions for the reviewer for this domain.
    pub fn instructions(&self) -> &'static str {
        match self {
            ReviewDomain::Security => {
                "You are reviewing for SECURITY issues ONLY. Focus on:\n\
                 - Authentication and authorization flaws\n\
                 - Input validation and sanitization\n\
                 - Injection vulnerabilities (SQL, command, XSS)\n\
                 - Secrets/credentials exposure\n\
                 - OWASP Top 10 vulnerabilities\n\n\
                 Create a GitHub review with 'request changes' if you find issues.\n\
                 Use line-specific comments for each issue found."
            }
            ReviewDomain::TechnicalFeasibility => {
                "You are reviewing for TECHNICAL FEASIBILITY ONLY. Focus on:\n\
                 - Is the architecture sound and appropriate?\n\
                 - Are the right technologies/libraries being used?\n\
                 - Are there performance concerns or scalability issues?\n\
                 - Is error handling appropriate?\n\n\
                 Create a GitHub review with 'request changes' if you find issues.\n\
                 Use line-specific comments for each concern."
            }
            ReviewDomain::TaskGranularity => {
                "You are reviewing for TASK GRANULARITY ONLY. Focus on:\n\
                 - Are tasks appropriately sized for parallel execution?\n\
                 - Should any large tasks be split?\n\
                 - Should any small tasks be combined?\n\
                 - Is the task breakdown clear and actionable?\n\n\
                 Create a GitHub review with 'request changes' if you find issues.\n\
                 Use line-specific comments for suggestions."
            }
            ReviewDomain::DependencyCompleteness => {
                "You are reviewing for DEPENDENCY COMPLETENESS ONLY. Focus on:\n\
                 - Are all task dependencies correctly identified?\n\
                 - Are there missing dependencies that could cause issues?\n\
                 - Are there opportunities for parallelization being missed?\n\
                 - Is the dependency graph cycle-free and logical?\n\n\
                 Create a GitHub review with 'request changes' if you find issues.\n\
                 Use line-specific comments for missing dependencies."
            }
            ReviewDomain::GeneralPolish => {
                "You are doing a FINAL POLISH review. Focus on:\n\
                 - Code quality and readability\n\
                 - Documentation completeness\n\
                 - Test coverage adequacy\n\
                 - Consistency with project conventions\n\
                 - Did previous review fixes actually address the issues?\n\n\
                 Create a GitHub review with 'request changes' if you find issues.\n\
                 Use line-specific comments for polish suggestions."
            }
        }
    }
}

/// Extracted metadata for PR creation.
///
/// This is extracted from the task description using an LLM before
/// starting the workflow to ensure meaningful PR titles and branch names.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedMetadata {
    /// Concise PR title (max 72 chars).
    pub pr_title: String,
    /// Branch name in kebab-case (max 50 chars).
    pub branch_name: String,
}

/// Status of a spawn-team iteration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IterationStatus {
    /// Primary phase completed.
    PrimaryComplete,
    /// Review phase completed.
    ReviewComplete(ReviewVerdict),
    /// Fix phase completed.
    FixComplete,
    /// Iteration limit reached.
    MaxIterationsReached,
}

/// Result of a spawn-team operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnTeamResult {
    /// Whether the team operation succeeded.
    pub success: bool,
    /// Number of iterations performed.
    pub iterations: u32,
    /// Final review verdict (if reviewed).
    pub final_verdict: Option<ReviewVerdict>,
    /// All review results.
    pub reviews: Vec<ReviewResult>,
    /// Summary of the team operation.
    pub summary: String,
}

/// Builder for creating review prompts.
pub struct ReviewPromptBuilder {
    original_prompt: String,
    git_diff: String,
}

impl ReviewPromptBuilder {
    /// Creates a new review prompt builder.
    pub fn new(original_prompt: impl Into<String>) -> Self {
        Self {
            original_prompt: original_prompt.into(),
            git_diff: String::new(),
        }
    }

    /// Sets the git diff of changes to review.
    pub fn with_diff(mut self, diff: impl Into<String>) -> Self {
        self.git_diff = diff.into();
        self
    }

    /// Builds the review prompt.
    pub fn build(&self) -> String {
        let mut prompt = String::new();

        prompt.push_str("## Code Review Request\n\n");
        prompt.push_str("Please review the following changes and provide feedback.\n\n");

        prompt.push_str("### Original Task\n\n");
        prompt.push_str(&self.original_prompt);
        prompt.push_str("\n\n");

        prompt.push_str("### Changes Made\n\n");
        prompt.push_str("```diff\n");
        prompt.push_str(&self.git_diff);
        prompt.push_str("\n```\n\n");

        prompt.push_str("### Response Format\n\n");
        prompt.push_str("Respond with a JSON object:\n");
        prompt.push_str("```json\n");
        prompt.push_str("{\n");
        prompt.push_str("  \"verdict\": \"approved\" | \"needs_changes\",\n");
        prompt.push_str("  \"suggestions\": [\n");
        prompt.push_str("    {\n");
        prompt.push_str("      \"file\": \"path/to/file\",\n");
        prompt.push_str("      \"line\": 42,\n");
        prompt.push_str("      \"issue\": \"description of issue\",\n");
        prompt.push_str("      \"suggestion\": \"how to fix it\"\n");
        prompt.push_str("    }\n");
        prompt.push_str("  ]\n");
        prompt.push_str("}\n");
        prompt.push_str("```\n");

        prompt
    }
}

/// Builder for creating fix prompts based on review suggestions.
pub struct FixPromptBuilder {
    original_prompt: String,
    suggestions: Vec<ReviewSuggestion>,
}

impl FixPromptBuilder {
    /// Creates a new fix prompt builder.
    pub fn new(original_prompt: impl Into<String>) -> Self {
        Self {
            original_prompt: original_prompt.into(),
            suggestions: Vec::new(),
        }
    }

    /// Adds suggestions to address.
    pub fn with_suggestions(mut self, suggestions: Vec<ReviewSuggestion>) -> Self {
        self.suggestions = suggestions;
        self
    }

    /// Builds the fix prompt.
    pub fn build(&self) -> String {
        let mut prompt = String::new();

        prompt.push_str("## Fix Request\n\n");
        prompt.push_str("Address the following review feedback:\n\n");

        prompt.push_str("### Original Task\n\n");
        prompt.push_str(&self.original_prompt);
        prompt.push_str("\n\n");

        prompt.push_str("### Issues to Fix\n\n");
        for (i, suggestion) in self.suggestions.iter().enumerate() {
            prompt.push_str(&format!("{}. **{}**", i + 1, suggestion.file));
            if let Some(line) = suggestion.line {
                prompt.push_str(&format!(" (line {})", line));
            }
            prompt.push_str("\n");
            prompt.push_str(&format!("   - Issue: {}\n", suggestion.issue));
            prompt.push_str(&format!("   - Suggestion: {}\n\n", suggestion.suggestion));
        }

        prompt
    }
}

/// Builder for creating GitHub review prompts.
///
/// This generates a prompt that instructs the reviewer to use `gh pr review`
/// to create a GitHub review with line-specific comments.
pub struct GitHubReviewPromptBuilder {
    pr_number: u64,
    repo: String,
    domain: ReviewDomain,
    original_prompt: String,
    git_diff: String,
}

impl GitHubReviewPromptBuilder {
    /// Creates a new GitHub review prompt builder.
    pub fn new(pr_number: u64, repo: impl Into<String>, domain: ReviewDomain) -> Self {
        Self {
            pr_number,
            repo: repo.into(),
            domain,
            original_prompt: String::new(),
            git_diff: String::new(),
        }
    }

    /// Sets the original task prompt.
    pub fn with_original_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.original_prompt = prompt.into();
        self
    }

    /// Sets the git diff to review.
    pub fn with_diff(mut self, diff: impl Into<String>) -> Self {
        self.git_diff = diff.into();
        self
    }

    /// Builds the GitHub review prompt.
    pub fn build(&self) -> String {
        let mut prompt = String::new();

        prompt.push_str(&format!(
            "## GitHub Code Review: {} Domain\n\n",
            self.domain.as_str()
        ));

        prompt.push_str(self.domain.instructions());
        prompt.push_str("\n\n");

        prompt.push_str("### Original Task\n\n");
        prompt.push_str(&self.original_prompt);
        prompt.push_str("\n\n");

        prompt.push_str("### Changes to Review\n\n");
        prompt.push_str("```diff\n");
        prompt.push_str(&self.git_diff);
        prompt.push_str("\n```\n\n");

        prompt.push_str("### Instructions\n\n");
        prompt.push_str("You MUST use the `gh` CLI to submit your feedback as PR comments.\n\n");

        prompt.push_str(
            "**NOTE:** You cannot use `gh pr review --request-changes` on your own PRs. ",
        );
        prompt.push_str("Use PR comments instead to provide feedback.\n\n");

        prompt.push_str("If you find issues that need addressing, add a summary comment:\n\n");
        prompt.push_str("```bash\n");
        prompt.push_str(&format!(
            "gh pr comment {} --repo {} --body \"[{} REVIEW - NEEDS CHANGES]\n\n<your detailed findings>\"\n",
            self.pr_number, self.repo, self.domain.as_str().to_uppercase()
        ));
        prompt.push_str("```\n\n");

        prompt.push_str("For line-specific comments on specific files, use:\n\n");
        prompt.push_str("```bash\n");
        prompt.push_str(&format!(
            "gh api repos/{}/pulls/{}/comments --method POST \\\n",
            self.repo, self.pr_number
        ));
        prompt.push_str("  -f body=\"Your comment\" -f path=\"path/to/file\" -F line=42 -f commit_id=\"$(gh pr view --repo ");
        prompt.push_str(&format!(
            "{} {} --json headRefOid --jq .headRefOid)\"\n",
            self.repo, self.pr_number
        ));
        prompt.push_str("```\n\n");

        prompt.push_str("If the code passes this review domain with no issues, add:\n\n");
        prompt.push_str("```bash\n");
        prompt.push_str(&format!(
            "gh pr comment {} --repo {} --body \"[{} REVIEW - APPROVED]\n\n{} review passed with no issues.\"\n",
            self.pr_number,
            self.repo,
            self.domain.as_str().to_uppercase(),
            self.domain.as_str()
        ));
        prompt.push_str("```\n\n");

        prompt.push_str("IMPORTANT: You must ONLY create comments using the `gh` command. ");
        prompt.push_str("Do not make any other changes to the repository. ");
        prompt.push_str(
            "Your entire output should be the `gh` commands you run and their results.\n",
        );

        prompt
    }
}

/// Builder for creating prompts to resolve GitHub review comments.
///
/// This generates a prompt that instructs the coder to resolve
/// specific review comments with code changes.
pub struct ResolveCommentPromptBuilder {
    pr_number: u64,
    repo: String,
    comment: GitHubReviewComment,
    original_prompt: String,
}

impl ResolveCommentPromptBuilder {
    /// Creates a new resolve comment prompt builder.
    pub fn new(pr_number: u64, repo: impl Into<String>, comment: GitHubReviewComment) -> Self {
        Self {
            pr_number,
            repo: repo.into(),
            comment,
            original_prompt: String::new(),
        }
    }

    /// Sets the original task prompt for context.
    pub fn with_original_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.original_prompt = prompt.into();
        self
    }

    /// Builds the resolve comment prompt.
    pub fn build(&self) -> String {
        let mut prompt = String::new();

        prompt.push_str("## Resolve GitHub Review Comment\n\n");
        prompt.push_str("A reviewer has left feedback on a plan file. ");
        prompt.push_str("Your task is to EDIT the plan file to address their feedback.\n\n");

        prompt.push_str("### Review Comment to Address\n\n");
        prompt.push_str(&format!("**File:** `{}`\n", self.comment.path));
        if let Some(line) = self.comment.line {
            prompt.push_str(&format!("**Line:** {}\n", line));
        }
        prompt.push_str(&format!("**Comment ID:** {}\n\n", self.comment.id));
        prompt.push_str(&format!("**Feedback:**\n{}\n\n", self.comment.body));

        prompt.push_str("### Instructions\n\n");
        prompt.push_str("**You MUST perform these actions:**\n\n");
        prompt.push_str(&format!(
            "1. Use the Read tool to read the file: `{}`\n",
            self.comment.path
        ));
        prompt.push_str(&format!(
            "2. Use the Edit tool to modify `{}` to address the reviewer's feedback\n\n",
            self.comment.path
        ));
        prompt.push_str(
            "The commit and reply will be handled automatically after you make the edits.\n",
        );

        prompt
    }
}

/// Parses a review response from JSON.
pub fn parse_review_response(response: &str) -> Option<ReviewResult> {
    // Try to find JSON in the response
    let json_start = response.find('{')?;
    let json_end = response.rfind('}')?;
    let json_str = &response[json_start..=json_end];

    // Parse the JSON
    let parsed: serde_json::Value = serde_json::from_str(json_str).ok()?;

    let verdict = match parsed.get("verdict")?.as_str()? {
        "approved" => ReviewVerdict::Approved,
        "needs_changes" => ReviewVerdict::NeedsChanges,
        _ => ReviewVerdict::Failed,
    };

    let mut suggestions = Vec::new();
    if let Some(arr) = parsed.get("suggestions").and_then(|v| v.as_array()) {
        for item in arr {
            if let (Some(file), Some(issue), Some(suggestion)) = (
                item.get("file").and_then(|v| v.as_str()),
                item.get("issue").and_then(|v| v.as_str()),
                item.get("suggestion").and_then(|v| v.as_str()),
            ) {
                suggestions.push(ReviewSuggestion {
                    file: file.to_string(),
                    line: item.get("line").and_then(|v| v.as_u64()).map(|l| l as u32),
                    issue: issue.to_string(),
                    suggestion: suggestion.to_string(),
                });
            }
        }
    }

    Some(ReviewResult {
        verdict,
        suggestions,
        summary: String::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordination_mode_default_is_github() {
        assert_eq!(CoordinationMode::default(), CoordinationMode::GitHub);
    }

    #[test]
    fn spawn_team_config_has_sensible_defaults() {
        let config = SpawnTeamConfig::default();

        assert_eq!(config.mode, CoordinationMode::GitHub);
        assert_eq!(config.max_iterations, 3);
        assert_eq!(config.primary_llm, "claude-code");
        assert_eq!(config.reviewer_llm, "gemini-cli");
        assert_eq!(config.max_escalations, 5);
    }

    #[test]
    fn spawn_team_config_default_max_concurrent_reviewers() {
        let config = SpawnTeamConfig::default();
        assert_eq!(config.max_concurrent_reviewers, 3);
    }

    #[test]
    fn coordination_mode_serializes() {
        assert_eq!(
            serde_json::to_string(&CoordinationMode::Sequential).unwrap(),
            "\"sequential\""
        );
        assert_eq!(
            serde_json::to_string(&CoordinationMode::PingPong).unwrap(),
            "\"pingpong\""
        );
        assert_eq!(
            serde_json::to_string(&CoordinationMode::GitHub).unwrap(),
            "\"github\""
        );
    }

    #[test]
    fn review_verdict_serializes() {
        assert_eq!(
            serde_json::to_string(&ReviewVerdict::Approved).unwrap(),
            "\"approved\""
        );
        assert_eq!(
            serde_json::to_string(&ReviewVerdict::NeedsChanges).unwrap(),
            "\"needs_changes\""
        );
    }

    #[test]
    fn review_prompt_builder_creates_prompt() {
        let prompt = ReviewPromptBuilder::new("Fix the auth bug")
            .with_diff("+ new code\n- old code")
            .build();

        assert!(prompt.contains("Fix the auth bug"));
        assert!(prompt.contains("+ new code"));
        assert!(prompt.contains("- old code"));
        assert!(prompt.contains("verdict"));
    }

    #[test]
    fn fix_prompt_builder_creates_prompt() {
        let suggestions = vec![ReviewSuggestion {
            file: "src/auth.rs".to_string(),
            line: Some(42),
            issue: "Missing error handling".to_string(),
            suggestion: "Add Result return type".to_string(),
        }];

        let prompt = FixPromptBuilder::new("Implement auth")
            .with_suggestions(suggestions)
            .build();

        assert!(prompt.contains("src/auth.rs"));
        assert!(prompt.contains("line 42"));
        assert!(prompt.contains("Missing error handling"));
        assert!(prompt.contains("Add Result return type"));
    }

    #[test]
    fn parse_review_response_extracts_approved() {
        let response = r#"
            Here's my review:
            ```json
            {
                "verdict": "approved",
                "suggestions": []
            }
            ```
        "#;

        let result = parse_review_response(response);
        assert!(result.is_some());
        assert_eq!(result.unwrap().verdict, ReviewVerdict::Approved);
    }

    #[test]
    fn parse_review_response_extracts_suggestions() {
        let response = r#"
            {
                "verdict": "needs_changes",
                "suggestions": [
                    {
                        "file": "src/main.rs",
                        "line": 10,
                        "issue": "Unused variable",
                        "suggestion": "Remove or use it"
                    }
                ]
            }
        "#;

        let result = parse_review_response(response).unwrap();
        assert_eq!(result.verdict, ReviewVerdict::NeedsChanges);
        assert_eq!(result.suggestions.len(), 1);
        assert_eq!(result.suggestions[0].file, "src/main.rs");
        assert_eq!(result.suggestions[0].line, Some(10));
    }

    #[test]
    fn parse_review_response_handles_missing_line() {
        let response = r#"
            {
                "verdict": "needs_changes",
                "suggestions": [
                    {
                        "file": "src/lib.rs",
                        "issue": "General issue",
                        "suggestion": "Fix it"
                    }
                ]
            }
        "#;

        let result = parse_review_response(response).unwrap();
        assert!(result.suggestions[0].line.is_none());
    }

    #[test]
    fn parse_review_response_returns_none_for_invalid() {
        let result = parse_review_response("not json at all");
        assert!(result.is_none());
    }

    #[test]
    fn iteration_status_equality() {
        assert_eq!(
            IterationStatus::PrimaryComplete,
            IterationStatus::PrimaryComplete
        );
        assert_eq!(
            IterationStatus::ReviewComplete(ReviewVerdict::Approved),
            IterationStatus::ReviewComplete(ReviewVerdict::Approved)
        );
        assert_ne!(
            IterationStatus::ReviewComplete(ReviewVerdict::Approved),
            IterationStatus::ReviewComplete(ReviewVerdict::NeedsChanges)
        );
    }

    #[test]
    fn spawn_team_result_serializes() {
        let result = SpawnTeamResult {
            success: true,
            iterations: 2,
            final_verdict: Some(ReviewVerdict::Approved),
            reviews: vec![],
            summary: "All good".to_string(),
        };

        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"success\":true"));
        assert!(json.contains("\"iterations\":2"));
    }

    #[test]
    fn review_domain_all_returns_five_domains() {
        let domains = ReviewDomain::all();
        assert_eq!(domains.len(), 5);
        assert_eq!(domains[0], ReviewDomain::Security);
        assert_eq!(domains[4], ReviewDomain::GeneralPolish);
    }

    #[test]
    fn review_domain_as_str_works() {
        assert_eq!(ReviewDomain::Security.as_str(), "Security");
        assert_eq!(
            ReviewDomain::TechnicalFeasibility.as_str(),
            "TechnicalFeasibility"
        );
        assert_eq!(ReviewDomain::GeneralPolish.as_str(), "GeneralPolish");
    }

    #[test]
    fn review_domain_instructions_not_empty() {
        for domain in ReviewDomain::all() {
            let instructions = domain.instructions();
            assert!(!instructions.is_empty());
            assert!(instructions.contains("GitHub review"));
        }
    }

    #[test]
    fn github_review_prompt_builder_creates_prompt() {
        let prompt = GitHubReviewPromptBuilder::new(123, "owner/repo", ReviewDomain::Security)
            .with_original_prompt("Fix the auth bug")
            .with_diff("+ new code\n- old code")
            .build();

        assert!(prompt.contains("Security Domain"));
        assert!(prompt.contains("Fix the auth bug"));
        assert!(prompt.contains("+ new code"));
        assert!(prompt.contains("gh pr comment 123"));
        assert!(prompt.contains("--repo owner/repo"));
        assert!(prompt.contains("[SECURITY REVIEW - NEEDS CHANGES]"));
        assert!(prompt.contains("[SECURITY REVIEW - APPROVED]"));
        // Verify we mention the limitation about request-changes (explains why we use comments)
        assert!(prompt.contains("cannot use `gh pr review --request-changes`"));
        // Verify we DON'T have a command line suggesting to use --request-changes
        // (we only mention it in the explanation)
        assert!(!prompt.contains("gh pr review 123 --repo owner/repo --request-changes"));
    }

    #[test]
    fn resolve_comment_prompt_builder_creates_prompt() {
        let comment = GitHubReviewComment {
            id: 456,
            path: "src/auth.rs".to_string(),
            line: Some(42),
            body: "Missing error handling".to_string(),
            resolved: false,
            resolved_by_commit: None,
        };

        let prompt = ResolveCommentPromptBuilder::new(123, "owner/repo", comment)
            .with_original_prompt("Implement authentication")
            .build();

        assert!(prompt.contains("src/auth.rs"));
        assert!(prompt.contains("Line:** 42"));
        assert!(prompt.contains("456"));
        assert!(prompt.contains("Missing error handling"));
        // Verify it instructs to use Read/Edit tools (commit handled automatically)
        assert!(prompt.contains("Read tool"));
        assert!(prompt.contains("Edit tool"));
    }

    #[test]
    fn github_review_comment_serializes() {
        let comment = GitHubReviewComment {
            id: 123,
            path: "src/main.rs".to_string(),
            line: Some(10),
            body: "Fix this".to_string(),
            resolved: false,
            resolved_by_commit: None,
        };

        let json = serde_json::to_string(&comment).unwrap();
        assert!(json.contains("\"id\":123"));
        assert!(json.contains("\"path\":\"src/main.rs\""));
    }

    #[test]
    fn extracted_metadata_serializes() {
        let metadata = ExtractedMetadata {
            pr_title: "Add user authentication feature".to_string(),
            branch_name: "feat/add-user-auth".to_string(),
        };

        let json = serde_json::to_string(&metadata).unwrap();
        assert!(json.contains("Add user authentication feature"));
        assert!(json.contains("feat/add-user-auth"));
    }

    #[test]
    fn github_review_serializes() {
        let review = GitHubReview {
            id: 789,
            state: "CHANGES_REQUESTED".to_string(),
            domain: "Security".to_string(),
            body: "Found issues".to_string(),
            comments: vec![],
            submitted_at: "2024-01-01T00:00:00Z".to_string(),
        };

        let json = serde_json::to_string(&review).unwrap();
        assert!(json.contains("\"id\":789"));
        assert!(json.contains("CHANGES_REQUESTED"));
    }
}
