//! Persistent phase sandbox for cruise-control workflows.
//!
//! Unlike transient sandboxes that clean up on drop, PhaseSandbox
//! persists until explicit cleanup or timeout.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::backoff::ExponentialBackoff;
use crate::error::{Error, Result};
use crate::sandbox::{CommentInfo, PhaseState, Sandbox, SandboxManifest, SandboxProvider};

/// A persistent sandbox for a cruise-control phase.
///
/// Survives LLM process exits. Multiple LLM invocations can use
/// the same sandbox. Cleaned up only via explicit `cleanup()` call,
/// PR merge/close, or timeout.
pub struct PhaseSandbox<P: SandboxProvider> {
    provider: P,
    worktree_path: PathBuf,
    branch_name: String,
    repo_path: PathBuf,
    pr_url: Option<String>,
    pr_number: Option<u64>,
    last_activity: Instant,
    timeout: Duration,
    backoff: ExponentialBackoff,
    pending_comments: Vec<CommentInfo>,
    cleaned_up: bool,
}

impl<P: SandboxProvider> PhaseSandbox<P> {
    /// Creates a new persistent phase sandbox.
    ///
    /// The sandbox will NOT be cleaned up when this struct is dropped.
    /// Call `cleanup()` explicitly when the phase is complete.
    pub fn new(provider: P, branch_name: String, timeout: Duration) -> Result<Self> {
        let manifest = SandboxManifest::with_sensible_defaults();
        let sandbox = provider.create_with_branch(manifest, &branch_name)?;
        let worktree_path = sandbox.path().clone();
        let repo_path = provider.repo_path().clone();

        // Prevent auto-cleanup by forgetting the sandbox
        // (we manage cleanup ourselves)
        std::mem::forget(sandbox);

        let backoff = ExponentialBackoff::new(Duration::from_secs(5), Duration::from_secs(300));

        Ok(Self {
            provider,
            worktree_path,
            branch_name,
            repo_path,
            pr_url: None,
            pr_number: None,
            last_activity: Instant::now(),
            timeout,
            backoff,
            pending_comments: Vec::new(),
            cleaned_up: false,
        })
    }

    /// Returns the worktree path.
    pub fn path(&self) -> &PathBuf {
        &self.worktree_path
    }

    /// Returns the branch name.
    pub fn branch_name(&self) -> &str {
        &self.branch_name
    }

    /// Returns a reference to the provider.
    pub fn provider(&self) -> &P {
        &self.provider
    }

    /// Sets the PR URL and number.
    pub fn set_pr(&mut self, url: String, number: u64) {
        self.pr_url = Some(url);
        self.pr_number = Some(number);
    }

    /// Returns the PR URL if set.
    pub fn pr_url(&self) -> Option<&str> {
        self.pr_url.as_deref()
    }

    /// Returns the PR number if set.
    pub fn pr_number(&self) -> Option<u64> {
        self.pr_number
    }

    /// Records activity, resetting the timeout clock.
    pub fn touch(&mut self) {
        self.last_activity = Instant::now();
    }

    /// Checks if the sandbox has timed out.
    pub fn is_timed_out(&self) -> bool {
        self.last_activity.elapsed() > self.timeout
    }

    /// Returns the current backoff interval.
    pub fn backoff_interval(&self) -> Duration {
        self.backoff.current()
    }

    /// Advances the backoff to the next interval.
    pub fn advance_backoff(&mut self) {
        self.backoff.next();
    }

    /// Resets the backoff to initial interval.
    pub fn reset_backoff(&mut self) {
        self.backoff.reset();
    }

    /// Adds a pending comment to be addressed.
    pub fn add_pending_comment(&mut self, comment: CommentInfo) {
        self.pending_comments.push(comment);
    }

    /// Takes all pending comments, clearing the queue.
    pub fn take_pending_comments(&mut self) -> Vec<CommentInfo> {
        std::mem::take(&mut self.pending_comments)
    }

    /// Returns true if there are pending comments.
    pub fn has_pending_comments(&self) -> bool {
        !self.pending_comments.is_empty()
    }

    /// Explicitly cleans up the sandbox.
    ///
    /// Removes the worktree and deletes the branch.
    pub fn cleanup(&mut self) -> Result<()> {
        if self.cleaned_up {
            return Ok(());
        }

        // Remove worktree
        let output = std::process::Command::new("git")
            .current_dir(&self.repo_path)
            .args(["worktree", "remove", "--force"])
            .arg(&self.worktree_path)
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::SandboxCleanup {
                path: self.worktree_path.clone(),
                reason: stderr.to_string(),
            });
        }

        // Delete branch
        let _ = std::process::Command::new("git")
            .current_dir(&self.repo_path)
            .args(["branch", "-D", &self.branch_name])
            .output();

        self.cleaned_up = true;
        Ok(())
    }

    /// Saves state to disk for crash recovery.
    pub fn save_state(&self) -> Result<()> {
        let state_dir = self.worktree_path.join(".cruise");
        std::fs::create_dir_all(&state_dir)?;

        let state = PhaseState {
            sandbox_path: self.worktree_path.clone(),
            branch_name: self.branch_name.clone(),
            pr_url: self.pr_url.clone(),
            pr_number: self.pr_number,
            phase: "planning".to_string(), // TODO: track actual phase
            current_review_domain: None,
            last_activity: chrono::Utc::now().to_rfc3339(),
            backoff_interval_secs: self.backoff.current().as_secs(),
            pending_comment_ids: self.pending_comments.iter().map(|c| c.id).collect(),
            completed_rounds: 0, // TODO: track actual rounds
        };

        let state_file = state_dir.join("phase-state.json");
        let json = serde_json::to_string_pretty(&state)
            .map_err(|e| Error::Cruise(format!("failed to serialize state: {}", e)))?;
        std::fs::write(&state_file, json)?;

        Ok(())
    }
}

// Note: No Drop implementation - cleanup is explicit only

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::WorktreeSandbox;
    use std::process::Command;
    use tempfile::TempDir;

    fn create_test_repo() -> TempDir {
        let temp = TempDir::new().unwrap();
        Command::new("git")
            .args(["init"])
            .current_dir(temp.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(temp.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(temp.path())
            .output()
            .unwrap();
        std::fs::write(temp.path().join("README.md"), "# Test").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(temp.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(temp.path())
            .output()
            .unwrap();
        temp
    }

    #[test]
    fn phase_sandbox_creates_persistent_worktree() {
        let repo = create_test_repo();
        let provider = WorktreeSandbox::new(repo.path().to_path_buf(), None);

        let phase = PhaseSandbox::new(
            provider,
            "feat/test-branch".to_string(),
            std::time::Duration::from_secs(86400),
        )
        .unwrap();

        assert!(phase.path().exists());
        assert_eq!(phase.branch_name(), "feat/test-branch");
    }

    #[test]
    fn phase_sandbox_does_not_cleanup_on_drop() {
        let repo = create_test_repo();
        let provider = WorktreeSandbox::new(repo.path().to_path_buf(), None);

        let path = {
            let phase = PhaseSandbox::new(
                provider,
                "feat/persist-test".to_string(),
                std::time::Duration::from_secs(86400),
            )
            .unwrap();
            phase.path().clone()
        };

        // Path should still exist after PhaseSandbox dropped
        assert!(path.exists());
    }

    #[test]
    fn phase_sandbox_explicit_cleanup_removes_worktree() {
        let repo = create_test_repo();
        let provider = WorktreeSandbox::new(repo.path().to_path_buf(), None);

        let mut phase = PhaseSandbox::new(
            provider,
            "feat/cleanup-test".to_string(),
            std::time::Duration::from_secs(86400),
        )
        .unwrap();

        let path = phase.path().clone();
        phase.cleanup().unwrap();

        assert!(!path.exists());
    }
}
