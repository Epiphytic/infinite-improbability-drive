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
    #[default]
    Sequential,
    /// Ping-pong mode: Iterative back-and-forth until approved.
    PingPong,
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
    /// Reviewer LLM identifier (e.g., "gemini-cli").
    #[serde(default = "default_reviewer_llm")]
    pub reviewer_llm: String,
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

impl Default for SpawnTeamConfig {
    fn default() -> Self {
        Self {
            mode: CoordinationMode::default(),
            max_iterations: default_max_iterations(),
            primary_llm: default_primary_llm(),
            reviewer_llm: default_reviewer_llm(),
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
    fn coordination_mode_default_is_sequential() {
        assert_eq!(CoordinationMode::default(), CoordinationMode::Sequential);
    }

    #[test]
    fn spawn_team_config_has_sensible_defaults() {
        let config = SpawnTeamConfig::default();

        assert_eq!(config.mode, CoordinationMode::Sequential);
        assert_eq!(config.max_iterations, 3);
        assert_eq!(config.primary_llm, "claude-code");
        assert_eq!(config.reviewer_llm, "gemini-cli");
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
}
