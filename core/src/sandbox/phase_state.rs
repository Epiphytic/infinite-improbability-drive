//! Phase state persistence for crash recovery.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Persistent state for a cruise-control phase.
///
/// Saved to `.cruise/phase-state.json` in the sandbox for crash recovery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseState {
    /// Path to the worktree directory.
    pub sandbox_path: PathBuf,
    /// Git branch name.
    pub branch_name: String,
    /// GitHub PR URL (if created).
    pub pr_url: Option<String>,
    /// GitHub PR number (if created).
    pub pr_number: Option<u64>,
    /// Current phase: "planning", "building", "validating".
    pub phase: String,
    /// Current review domain (Security, TechnicalFeasibility, etc.).
    pub current_review_domain: Option<String>,
    /// ISO 8601 timestamp of last activity.
    pub last_activity: String,
    /// Current backoff interval in seconds.
    pub backoff_interval_secs: u64,
    /// IDs of comments pending fixer action.
    pub pending_comment_ids: Vec<u64>,
    /// Number of completed review rounds.
    pub completed_rounds: u32,
}

/// Information about a PR comment to be addressed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommentInfo {
    /// GitHub comment ID.
    pub id: u64,
    /// Comment body text.
    pub body: String,
    /// File path (for line comments).
    pub path: Option<String>,
    /// Line number (for line comments).
    pub line: Option<u32>,
    /// Comment author.
    pub author: String,
    /// ISO 8601 timestamp.
    pub created_at: String,
}

impl PhaseState {
    /// Creates state file path within sandbox.
    pub fn state_file_path(sandbox_path: &PathBuf) -> PathBuf {
        sandbox_path.join(".cruise").join("phase-state.json")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_state_serializes_to_json() {
        let state = PhaseState {
            sandbox_path: PathBuf::from("/tmp/sandbox"),
            branch_name: "feat/test".to_string(),
            pr_url: Some("https://github.com/owner/repo/pull/1".to_string()),
            pr_number: Some(1),
            phase: "planning".to_string(),
            current_review_domain: Some("Security".to_string()),
            last_activity: "2026-02-04T10:00:00Z".to_string(),
            backoff_interval_secs: 5,
            pending_comment_ids: vec![123, 456],
            completed_rounds: 2,
        };

        let json = serde_json::to_string(&state).unwrap();
        assert!(json.contains("feat/test"));
        assert!(json.contains("123"));
    }

    #[test]
    fn phase_state_deserializes_from_json() {
        let json = r#"{
            "sandbox_path": "/tmp/sandbox",
            "branch_name": "feat/test",
            "pr_url": null,
            "pr_number": null,
            "phase": "planning",
            "current_review_domain": null,
            "last_activity": "2026-02-04T10:00:00Z",
            "backoff_interval_secs": 5,
            "pending_comment_ids": [],
            "completed_rounds": 0
        }"#;

        let state: PhaseState = serde_json::from_str(json).unwrap();
        assert_eq!(state.branch_name, "feat/test");
        assert_eq!(state.backoff_interval_secs, 5);
    }

    #[test]
    fn comment_info_captures_details() {
        let comment = CommentInfo {
            id: 123,
            body: "Fix this".to_string(),
            path: Some("src/main.rs".to_string()),
            line: Some(42),
            author: "reviewer".to_string(),
            created_at: "2026-02-04T10:00:00Z".to_string(),
        };

        assert_eq!(comment.id, 123);
        assert_eq!(comment.line, Some(42));
    }
}
