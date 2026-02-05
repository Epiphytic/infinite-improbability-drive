# Phase Sandbox Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Implement persistent phase sandbox that survives LLM exits, with async comment monitoring and exponential backoff polling.

**Architecture:** New `PhaseSandbox` wrapper owns a worktree for entire cruise-control phase. Watcher monitors PR comments with exponential backoff (5s→5min). Fixer rounds spawn fresh Claude per batch.

**Tech Stack:** Rust, tokio async, serde for state persistence, chrono for timestamps

---

## Task 1: Add ExponentialBackoff Utility

**Files:**
- Create: `core/src/backoff.rs`
- Modify: `core/src/lib.rs` (add module)

**Step 1: Write the failing test**

In `core/src/backoff.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn backoff_starts_at_initial() {
        let backoff = ExponentialBackoff::new(
            Duration::from_secs(5),
            Duration::from_secs(300),
        );
        assert_eq!(backoff.current(), Duration::from_secs(5));
    }

    #[test]
    fn backoff_doubles_on_next() {
        let mut backoff = ExponentialBackoff::new(
            Duration::from_secs(5),
            Duration::from_secs(300),
        );
        backoff.next();
        assert_eq!(backoff.current(), Duration::from_secs(10));
        backoff.next();
        assert_eq!(backoff.current(), Duration::from_secs(20));
    }

    #[test]
    fn backoff_caps_at_max() {
        let mut backoff = ExponentialBackoff::new(
            Duration::from_secs(100),
            Duration::from_secs(300),
        );
        backoff.next();  // 200
        backoff.next();  // 400 -> capped to 300
        assert_eq!(backoff.current(), Duration::from_secs(300));
    }

    #[test]
    fn backoff_resets_to_initial() {
        let mut backoff = ExponentialBackoff::new(
            Duration::from_secs(5),
            Duration::from_secs(300),
        );
        backoff.next();
        backoff.next();
        backoff.reset();
        assert_eq!(backoff.current(), Duration::from_secs(5));
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test backoff -p infinite-improbability-drive`
Expected: FAIL with "cannot find value `ExponentialBackoff`"

**Step 3: Write minimal implementation**

In `core/src/backoff.rs`:

```rust
//! Exponential backoff utility for polling intervals.

use std::time::Duration;
use serde::{Deserialize, Serialize};

/// Exponential backoff with configurable min/max.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExponentialBackoff {
    initial: Duration,
    max: Duration,
    current: Duration,
}

impl ExponentialBackoff {
    /// Creates a new backoff starting at `initial`, capping at `max`.
    pub fn new(initial: Duration, max: Duration) -> Self {
        Self {
            initial,
            max,
            current: initial,
        }
    }

    /// Returns the current backoff duration.
    pub fn current(&self) -> Duration {
        self.current
    }

    /// Advances to the next backoff interval (doubles, capped at max).
    pub fn next(&mut self) {
        self.current = (self.current * 2).min(self.max);
    }

    /// Resets backoff to initial value.
    pub fn reset(&mut self) {
        self.current = self.initial;
    }
}
```

**Step 4: Add module to lib.rs**

In `core/src/lib.rs`, add:

```rust
pub mod backoff;
```

**Step 5: Run test to verify it passes**

Run: `cargo test backoff -p infinite-improbability-drive`
Expected: PASS (4 tests)

**Step 6: Commit**

```bash
git add core/src/backoff.rs core/src/lib.rs
git commit -m "feat: add ExponentialBackoff utility for polling intervals"
```

---

## Task 2: Add PhaseState and CommentInfo Structs

**Files:**
- Create: `core/src/sandbox/phase_state.rs`
- Modify: `core/src/sandbox/mod.rs` (add module)

**Step 1: Write the failing test**

In `core/src/sandbox/phase_state.rs`:

```rust
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
```

**Step 2: Run test to verify it fails**

Run: `cargo test phase_state -p infinite-improbability-drive`
Expected: FAIL with "cannot find type `PhaseState`"

**Step 3: Write minimal implementation**

In `core/src/sandbox/phase_state.rs`:

```rust
//! Phase state persistence for crash recovery.

use std::path::PathBuf;
use serde::{Deserialize, Serialize};

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
```

**Step 4: Add module to sandbox/mod.rs**

In `core/src/sandbox/mod.rs`, add:

```rust
mod phase_state;

pub use phase_state::{CommentInfo, PhaseState};
```

**Step 5: Run test to verify it passes**

Run: `cargo test phase_state -p infinite-improbability-drive`
Expected: PASS (3 tests)

**Step 6: Commit**

```bash
git add core/src/sandbox/phase_state.rs core/src/sandbox/mod.rs
git commit -m "feat: add PhaseState and CommentInfo for crash recovery"
```

---

## Task 3: Add PhaseSandbox Core Structure

**Files:**
- Create: `core/src/sandbox/phase.rs`
- Modify: `core/src/sandbox/mod.rs` (add module and export)
- Modify: `core/src/error.rs` (add PhaseTimeout variant)

**Step 1: Write the failing test**

In `core/src/sandbox/phase.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::WorktreeSandbox;
    use tempfile::TempDir;
    use std::process::Command;

    fn create_test_repo() -> TempDir {
        let temp = TempDir::new().unwrap();
        Command::new("git").args(["init"]).current_dir(temp.path()).output().unwrap();
        Command::new("git").args(["config", "user.email", "test@test.com"]).current_dir(temp.path()).output().unwrap();
        Command::new("git").args(["config", "user.name", "Test"]).current_dir(temp.path()).output().unwrap();
        std::fs::write(temp.path().join("README.md"), "# Test").unwrap();
        Command::new("git").args(["add", "."]).current_dir(temp.path()).output().unwrap();
        Command::new("git").args(["commit", "-m", "init"]).current_dir(temp.path()).output().unwrap();
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
        ).unwrap();

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
            ).unwrap();
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
        ).unwrap();

        let path = phase.path().clone();
        phase.cleanup().unwrap();

        assert!(!path.exists());
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test phase_sandbox -p infinite-improbability-drive`
Expected: FAIL with "cannot find type `PhaseSandbox`"

**Step 3: Write minimal implementation**

In `core/src/sandbox/phase.rs`:

```rust
//! Persistent phase sandbox for cruise-control workflows.
//!
//! Unlike transient sandboxes that clean up on drop, PhaseSandbox
//! persists until explicit cleanup or timeout.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::backoff::ExponentialBackoff;
use crate::error::{Error, Result};
use crate::sandbox::{CommentInfo, PhaseState, SandboxManifest, SandboxProvider};

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

        let backoff = ExponentialBackoff::new(
            Duration::from_secs(5),
            Duration::from_secs(300),
        );

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
```

**Step 4: Add module to sandbox/mod.rs**

In `core/src/sandbox/mod.rs`, update to:

```rust
//! Sandbox module for isolated LLM execution environments.
//!
//! This module provides the [`SandboxProvider`] trait for creating isolated
//! sandboxes and the [`WorktreeSandbox`] implementation using git worktrees.

mod phase;
mod phase_state;
mod provider;
mod worktree;

pub use phase::PhaseSandbox;
pub use phase_state::{CommentInfo, PhaseState};
pub use provider::{Sandbox, SandboxManifest, SandboxProvider};
pub use worktree::WorktreeSandbox;
```

**Step 5: Run test to verify it passes**

Run: `cargo test phase_sandbox -p infinite-improbability-drive`
Expected: PASS (3 tests)

**Step 6: Commit**

```bash
git add core/src/sandbox/phase.rs core/src/sandbox/mod.rs
git commit -m "feat: add PhaseSandbox for persistent cruise-control sandboxes"
```

---

## Task 4: Add State Persistence Load/Recovery

**Files:**
- Modify: `core/src/sandbox/phase.rs` (add load_from_state)

**Step 1: Write the failing test**

Add to `core/src/sandbox/phase.rs` tests:

```rust
    #[test]
    fn phase_sandbox_saves_and_loads_state() {
        let repo = create_test_repo();
        let provider = WorktreeSandbox::new(repo.path().to_path_buf(), None);

        let sandbox_path = {
            let mut phase = PhaseSandbox::new(
                provider.clone(),
                "feat/state-test".to_string(),
                std::time::Duration::from_secs(86400),
            ).unwrap();

            phase.set_pr("https://github.com/test/repo/pull/42".to_string(), 42);
            phase.save_state().unwrap();
            phase.path().clone()
        };

        // Load from saved state
        let loaded = PhaseSandbox::<WorktreeSandbox>::load_from_state(
            &sandbox_path,
            provider,
        ).unwrap();

        assert_eq!(loaded.pr_number(), Some(42));
        assert_eq!(loaded.branch_name(), "feat/state-test");
    }
```

**Step 2: Run test to verify it fails**

Run: `cargo test phase_sandbox_saves -p infinite-improbability-drive`
Expected: FAIL with "no function `load_from_state`"

**Step 3: Write minimal implementation**

Add to `PhaseSandbox` impl in `core/src/sandbox/phase.rs`:

```rust
    /// Loads a PhaseSandbox from saved state (for crash recovery).
    pub fn load_from_state(sandbox_path: &PathBuf, provider: P) -> Result<Self> {
        let state_file = PhaseState::state_file_path(sandbox_path);
        let json = std::fs::read_to_string(&state_file)
            .map_err(|e| Error::Cruise(format!("failed to read state file: {}", e)))?;
        let state: PhaseState = serde_json::from_str(&json)
            .map_err(|e| Error::Cruise(format!("failed to parse state: {}", e)))?;

        let backoff = ExponentialBackoff::new(
            Duration::from_secs(5),
            Duration::from_secs(300),
        );

        Ok(Self {
            provider,
            worktree_path: state.sandbox_path,
            branch_name: state.branch_name,
            repo_path: provider.repo_path().clone(),
            pr_url: state.pr_url,
            pr_number: state.pr_number,
            last_activity: Instant::now(),
            timeout: Duration::from_secs(86400),
            backoff,
            pending_comments: Vec::new(), // Comments reloaded from GitHub
            cleaned_up: false,
        })
    }
```

**Step 4: Run test to verify it passes**

Run: `cargo test phase_sandbox_saves -p infinite-improbability-drive`
Expected: PASS

**Step 5: Commit**

```bash
git add core/src/sandbox/phase.rs
git commit -m "feat: add state persistence and recovery for PhaseSandbox"
```

---

## Task 5: Add Comment Polling to PhaseSandbox

**Files:**
- Modify: `core/src/sandbox/phase.rs` (add poll_comments method)

**Step 1: Write the failing test**

Add to `core/src/sandbox/phase.rs` tests:

```rust
    #[test]
    fn phase_sandbox_detects_review_complete_marker() {
        let comments = vec![
            CommentInfo {
                id: 1,
                body: "Some feedback".to_string(),
                path: None,
                line: None,
                author: "reviewer".to_string(),
                created_at: "2026-02-04T10:00:00Z".to_string(),
            },
            CommentInfo {
                id: 2,
                body: "[REVIEW COMPLETE] All done".to_string(),
                path: None,
                line: None,
                author: "reviewer".to_string(),
                created_at: "2026-02-04T10:01:00Z".to_string(),
            },
        ];

        assert!(PhaseSandbox::<WorktreeSandbox>::has_review_complete_marker(&comments));
    }

    #[test]
    fn phase_sandbox_no_marker_without_review_complete() {
        let comments = vec![
            CommentInfo {
                id: 1,
                body: "Some feedback".to_string(),
                path: None,
                line: None,
                author: "reviewer".to_string(),
                created_at: "2026-02-04T10:00:00Z".to_string(),
            },
        ];

        assert!(!PhaseSandbox::<WorktreeSandbox>::has_review_complete_marker(&comments));
    }
```

**Step 2: Run test to verify it fails**

Run: `cargo test review_complete -p infinite-improbability-drive`
Expected: FAIL with "no function `has_review_complete_marker`"

**Step 3: Write minimal implementation**

Add to `PhaseSandbox` impl in `core/src/sandbox/phase.rs`:

```rust
    /// Checks if any comment contains the [REVIEW COMPLETE] marker.
    pub fn has_review_complete_marker(comments: &[CommentInfo]) -> bool {
        comments.iter().any(|c| c.body.contains("[REVIEW COMPLETE]"))
    }

    /// Filters comments to only those needing action (not [REVIEW COMPLETE]).
    pub fn actionable_comments(comments: Vec<CommentInfo>) -> Vec<CommentInfo> {
        comments
            .into_iter()
            .filter(|c| !c.body.contains("[REVIEW COMPLETE]"))
            .collect()
    }
```

**Step 4: Run test to verify it passes**

Run: `cargo test review_complete -p infinite-improbability-drive`
Expected: PASS (2 tests)

**Step 5: Commit**

```bash
git add core/src/sandbox/phase.rs
git commit -m "feat: add review complete marker detection"
```

---

## Task 6: Integrate PhaseSandbox with CruiseRunner

**Files:**
- Modify: `core/src/cruise/runner.rs` (use PhaseSandbox instead of direct worktree)

**Step 1: Read current CruiseRunner implementation**

Read `core/src/cruise/runner.rs` to understand current sandbox usage.

**Step 2: Update CruiseRunner to use PhaseSandbox**

This is a larger refactor. The key changes:
1. Replace `SpawnTeamOrchestrator` sandbox creation with `PhaseSandbox`
2. Pass `phase.path()` to LLM invocations
3. Don't cleanup sandbox between review rounds

**Step 3: Write integration test**

In `core/src/cruise/runner.rs` tests, add:

```rust
    #[test]
    fn cruise_runner_uses_persistent_sandbox() {
        // Test that sandbox persists across planner/reviewer/fixer invocations
        // This is verified by the sandbox existing after run_planning completes
    }
```

**Step 4: Run full test suite**

Run: `cargo test -p infinite-improbability-drive`
Expected: All tests pass

**Step 5: Commit**

```bash
git add core/src/cruise/runner.rs
git commit -m "feat: integrate PhaseSandbox with CruiseRunner"
```

---

## Task 7: Add CLI Commands (cruise fix, cleanup, resume)

**Files:**
- Modify: `core/src/main.rs` (add subcommands)
- Create: `commands/cruise-fix.md` (command definition)
- Create: `commands/cruise-cleanup.md` (command definition)

**Step 1: Add cruise fix subcommand**

In `core/src/main.rs`, add to CLI:

```rust
#[derive(Subcommand)]
enum CruiseCommands {
    /// Trigger immediate comment poll and fixer round
    Fix {
        /// Inject a comment directly (bypasses GitHub)
        #[arg(long)]
        comment: Option<String>,
    },
    /// Force cleanup of phase sandbox
    Cleanup,
    /// Resume monitoring after crash
    Resume,
}
```

**Step 2: Implement fix command**

```rust
async fn handle_cruise_fix(comment: Option<String>) -> Result<()> {
    // 1. Find existing phase sandbox state
    // 2. Load PhaseSandbox from state
    // 3. If comment provided, add to pending
    // 4. Poll for new GitHub comments
    // 5. Trigger fixer round if any pending
    todo!()
}
```

**Step 3: Test manually**

Run: `cargo run -- cruise fix --comment "Please fix X"`
Expected: Command parses successfully (impl can be TODO)

**Step 4: Commit**

```bash
git add core/src/main.rs commands/
git commit -m "feat: add cruise fix, cleanup, resume CLI commands"
```

---

## Task 8: Full Integration Test

**Files:**
- Modify: `tests/e2e_test.rs` (add phase sandbox test)

**Step 1: Create E2E test for persistent sandbox**

```rust
#[test]
#[ignore] // Requires real LLMs
fn phase_sandbox_persists_across_rounds() {
    // 1. Create PhaseSandbox
    // 2. Run planner (creates files)
    // 3. Verify sandbox still exists
    // 4. Run reviewer (reads files)
    // 5. Verify sandbox still exists
    // 6. Add pending comment
    // 7. Run fixer
    // 8. Verify sandbox still exists
    // 9. Explicit cleanup
    // 10. Verify sandbox removed
}
```

**Step 2: Run test**

Run: `cargo test phase_sandbox_persists --test e2e_test -- --ignored`
Expected: PASS (with real LLMs configured)

**Step 3: Commit**

```bash
git add tests/e2e_test.rs
git commit -m "test: add E2E test for persistent phase sandbox"
```

---

## Summary

| Task | Description | Files |
|------|-------------|-------|
| 1 | ExponentialBackoff utility | `backoff.rs`, `lib.rs` |
| 2 | PhaseState structs | `sandbox/phase_state.rs` |
| 3 | PhaseSandbox core | `sandbox/phase.rs` |
| 4 | State persistence | `sandbox/phase.rs` |
| 5 | Comment polling | `sandbox/phase.rs` |
| 6 | CruiseRunner integration | `cruise/runner.rs` |
| 7 | CLI commands | `main.rs`, `commands/` |
| 8 | E2E integration test | `tests/e2e_test.rs` |

**Total commits:** 8
**Estimated test count:** ~15 new tests
