//! Pull request creation and management.
//!
//! Handles creating PRs from worktree branches and resolving merge conflicts.

use std::path::PathBuf;
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Information about a created pull request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullRequest {
    /// PR number.
    pub number: u64,
    /// PR URL.
    pub url: String,
    /// PR title.
    pub title: String,
    /// Target branch.
    pub base_branch: String,
    /// Source branch.
    pub head_branch: String,
}

/// Strategy for handling merge conflicts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConflictStrategy {
    /// Attempt to auto-resolve small conflicts.
    #[default]
    AutoResolve,
    /// Fail on any conflict.
    Fail,
    /// Mark conflicts and continue.
    Mark,
}

/// Result of a merge conflict check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeStatus {
    /// No conflicts, clean merge possible.
    Clean,
    /// Conflicts detected.
    Conflicts(Vec<ConflictFile>),
    /// Branch is already up to date.
    UpToDate,
}

/// Information about a conflicting file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConflictFile {
    /// Path to the conflicting file.
    pub path: PathBuf,
    /// Number of conflict markers in the file.
    pub conflict_count: usize,
    /// Whether this is a simple conflict (few lines).
    pub is_simple: bool,
}

/// Manager for creating and updating pull requests.
pub struct PRManager {
    /// Repository path.
    repo_path: PathBuf,
    /// Conflict handling strategy.
    conflict_strategy: ConflictStrategy,
}

impl PRManager {
    /// Creates a new PR manager for the given repository.
    pub fn new(repo_path: PathBuf) -> Self {
        Self {
            repo_path,
            conflict_strategy: ConflictStrategy::default(),
        }
    }

    /// Sets the conflict handling strategy.
    pub fn with_conflict_strategy(mut self, strategy: ConflictStrategy) -> Self {
        self.conflict_strategy = strategy;
        self
    }

    /// Commits any uncommitted changes in the worktree.
    pub fn commit_changes(&self, worktree_path: &PathBuf, message: &str) -> Result<Option<String>> {
        // Check for changes
        let status = Command::new("git")
            .current_dir(worktree_path)
            .args(["status", "--porcelain"])
            .output()?;

        let status_output = String::from_utf8_lossy(&status.stdout);
        if status_output.trim().is_empty() {
            return Ok(None); // No changes to commit
        }

        // Stage all changes
        let add = Command::new("git")
            .current_dir(worktree_path)
            .args(["add", "-A"])
            .output()?;

        if !add.status.success() {
            return Err(Error::Git(format!(
                "failed to stage changes: {}",
                String::from_utf8_lossy(&add.stderr)
            )));
        }

        // Commit
        let commit = Command::new("git")
            .current_dir(worktree_path)
            .args(["commit", "-m", message])
            .output()?;

        if !commit.status.success() {
            let stderr = String::from_utf8_lossy(&commit.stderr);
            // Check if it's just "nothing to commit"
            if stderr.contains("nothing to commit") {
                return Ok(None);
            }
            return Err(Error::Git(format!("failed to commit: {}", stderr)));
        }

        // Get commit hash
        let rev = Command::new("git")
            .current_dir(worktree_path)
            .args(["rev-parse", "HEAD"])
            .output()?;

        let hash = String::from_utf8_lossy(&rev.stdout).trim().to_string();
        Ok(Some(hash))
    }

    /// Pushes a branch to the remote.
    pub fn push_branch(&self, worktree_path: &PathBuf, branch_name: &str) -> Result<()> {
        let output = Command::new("git")
            .current_dir(worktree_path)
            .args(["push", "-u", "origin", branch_name])
            .output()?;

        if !output.status.success() {
            return Err(Error::Git(format!(
                "failed to push branch: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        Ok(())
    }

    /// Creates a pull request using the gh CLI.
    pub fn create_pr(
        &self,
        title: &str,
        body: &str,
        head_branch: &str,
        base_branch: &str,
    ) -> Result<PullRequest> {
        let output = Command::new("gh")
            .current_dir(&self.repo_path)
            .args([
                "pr",
                "create",
                "--title",
                title,
                "--body",
                body,
                "--head",
                head_branch,
                "--base",
                base_branch,
            ])
            .output()?;

        if !output.status.success() {
            return Err(Error::Git(format!(
                "failed to create PR: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        // Parse PR URL from output
        let url = String::from_utf8_lossy(&output.stdout).trim().to_string();

        // Extract PR number from URL
        let number = url
            .split('/')
            .last()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);

        Ok(PullRequest {
            number,
            url,
            title: title.to_string(),
            base_branch: base_branch.to_string(),
            head_branch: head_branch.to_string(),
        })
    }

    /// Checks for merge conflicts between the head and base branches.
    pub fn check_conflicts(&self, head_branch: &str, base_branch: &str) -> Result<MergeStatus> {
        // Fetch latest
        let _ = Command::new("git")
            .current_dir(&self.repo_path)
            .args(["fetch", "origin", base_branch])
            .output()?;

        // Try a dry-run merge
        let output = Command::new("git")
            .current_dir(&self.repo_path)
            .args([
                "merge-tree",
                &format!("origin/{}", base_branch),
                head_branch,
            ])
            .output()?;

        let merge_output = String::from_utf8_lossy(&output.stdout);

        // Check for conflict markers
        if merge_output.contains("<<<<<<<") || merge_output.contains(">>>>>>>") {
            let conflicts = self.parse_conflicts(&merge_output);
            return Ok(MergeStatus::Conflicts(conflicts));
        }

        // Check if already up to date
        let merge_base = Command::new("git")
            .current_dir(&self.repo_path)
            .args([
                "merge-base",
                head_branch,
                &format!("origin/{}", base_branch),
            ])
            .output()?;

        let head_rev = Command::new("git")
            .current_dir(&self.repo_path)
            .args(["rev-parse", head_branch])
            .output()?;

        let base_output = String::from_utf8_lossy(&merge_base.stdout)
            .trim()
            .to_string();
        let head_output = String::from_utf8_lossy(&head_rev.stdout).trim().to_string();

        if base_output == head_output {
            return Ok(MergeStatus::UpToDate);
        }

        Ok(MergeStatus::Clean)
    }

    /// Parses conflict information from merge-tree output.
    fn parse_conflicts(&self, output: &str) -> Vec<ConflictFile> {
        let mut conflicts = Vec::new();
        let mut current_file: Option<String> = None;
        let mut current_count = 0;

        for line in output.lines() {
            if line.starts_with("diff --git") {
                // Save previous file if any
                if let Some(file) = current_file.take() {
                    conflicts.push(ConflictFile {
                        path: PathBuf::from(&file),
                        conflict_count: current_count,
                        is_simple: current_count <= 2,
                    });
                }

                // Extract file path
                if let Some(path) = line.split(" b/").last() {
                    current_file = Some(path.to_string());
                    current_count = 0;
                }
            } else if line.contains("<<<<<<<") {
                current_count += 1;
            }
        }

        // Save last file
        if let Some(file) = current_file {
            conflicts.push(ConflictFile {
                path: PathBuf::from(&file),
                conflict_count: current_count,
                is_simple: current_count <= 2,
            });
        }

        conflicts
    }

    /// Attempts to auto-resolve simple conflicts.
    pub fn auto_resolve_conflicts(&self, worktree_path: &PathBuf) -> Result<bool> {
        // This is a simplified implementation
        // In practice, this would use more sophisticated conflict resolution

        let output = Command::new("git")
            .current_dir(worktree_path)
            .args(["diff", "--name-only", "--diff-filter=U"])
            .output()?;

        let output_str = String::from_utf8_lossy(&output.stdout);
        let conflicted_files: Vec<&str> = output_str.lines().filter(|s| !s.is_empty()).collect();

        if conflicted_files.is_empty() {
            return Ok(true); // No conflicts
        }

        // For now, we only handle simple cases where we can use "theirs"
        // In a full implementation, this would be more sophisticated
        for file in conflicted_files {
            let checkout = Command::new("git")
                .current_dir(worktree_path)
                .args(["checkout", "--theirs", file])
                .output()?;

            if !checkout.status.success() {
                return Ok(false); // Cannot auto-resolve
            }

            let add = Command::new("git")
                .current_dir(worktree_path)
                .args(["add", file])
                .output()?;

            if !add.status.success() {
                return Ok(false);
            }
        }

        Ok(true)
    }

    /// Generates a PR description for a spawn result.
    pub fn generate_pr_body(
        &self,
        prompt: &str,
        summary: &str,
        files_changed: &[(PathBuf, i32, i32)],
        spawn_id: &str,
    ) -> String {
        let mut body = String::new();

        body.push_str("## Spawn Result\n\n");
        body.push_str(&format!("**Spawn ID:** `{}`\n\n", spawn_id));

        body.push_str("### Original Prompt\n\n");
        body.push_str(&format!("> {}\n\n", prompt));

        body.push_str("### Summary\n\n");
        body.push_str(summary);
        body.push_str("\n\n");

        if !files_changed.is_empty() {
            body.push_str("### Files Changed\n\n");
            for (path, additions, deletions) in files_changed {
                body.push_str(&format!(
                    "- `{}` (+{}, -{})\n",
                    path.display(),
                    additions,
                    deletions
                ));
            }
            body.push('\n');
        }

        body.push_str("---\n");
        body.push_str("*Created by infinite-improbability-drive*\n");

        body
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_repo() -> TempDir {
        let temp_dir = TempDir::new().expect("failed to create temp dir");

        Command::new("git")
            .current_dir(temp_dir.path())
            .args(["init"])
            .output()
            .expect("failed to init git");

        Command::new("git")
            .current_dir(temp_dir.path())
            .args(["config", "user.email", "test@test.com"])
            .output()
            .expect("failed to config email");

        Command::new("git")
            .current_dir(temp_dir.path())
            .args(["config", "user.name", "Test"])
            .output()
            .expect("failed to config name");

        std::fs::write(temp_dir.path().join("README.md"), "# Test\n").unwrap();

        Command::new("git")
            .current_dir(temp_dir.path())
            .args(["add", "-A"])
            .output()
            .unwrap();

        Command::new("git")
            .current_dir(temp_dir.path())
            .args(["commit", "-m", "Initial"])
            .output()
            .unwrap();

        temp_dir
    }

    #[test]
    fn pr_manager_can_be_created() {
        let manager = PRManager::new(PathBuf::from("/tmp/test"));
        assert_eq!(manager.conflict_strategy, ConflictStrategy::AutoResolve);
    }

    #[test]
    fn pr_manager_conflict_strategy_can_be_set() {
        let manager = PRManager::new(PathBuf::from("/tmp/test"))
            .with_conflict_strategy(ConflictStrategy::Fail);

        assert_eq!(manager.conflict_strategy, ConflictStrategy::Fail);
    }

    #[test]
    fn pr_manager_commits_changes() {
        let repo = create_test_repo();
        let manager = PRManager::new(repo.path().to_path_buf());

        // Add a new file
        std::fs::write(repo.path().join("new_file.txt"), "content").unwrap();

        let result = manager.commit_changes(&repo.path().to_path_buf(), "Add new file");

        assert!(result.is_ok());
        let hash = result.unwrap();
        assert!(hash.is_some());
        assert!(!hash.unwrap().is_empty());
    }

    #[test]
    fn pr_manager_returns_none_for_no_changes() {
        let repo = create_test_repo();
        let manager = PRManager::new(repo.path().to_path_buf());

        let result = manager.commit_changes(&repo.path().to_path_buf(), "No changes");

        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn conflict_file_simple_detection() {
        let simple = ConflictFile {
            path: PathBuf::from("test.rs"),
            conflict_count: 1,
            is_simple: true,
        };
        assert!(simple.is_simple);

        let complex = ConflictFile {
            path: PathBuf::from("test.rs"),
            conflict_count: 5,
            is_simple: false,
        };
        assert!(!complex.is_simple);
    }

    #[test]
    fn merge_status_equality() {
        assert_eq!(MergeStatus::Clean, MergeStatus::Clean);
        assert_eq!(MergeStatus::UpToDate, MergeStatus::UpToDate);
        assert_ne!(MergeStatus::Clean, MergeStatus::UpToDate);
    }

    #[test]
    fn pr_body_generation() {
        let manager = PRManager::new(PathBuf::from("/tmp"));

        let files = vec![
            (PathBuf::from("src/main.rs"), 10, 5),
            (PathBuf::from("tests/test.rs"), 20, 0),
        ];

        let body = manager.generate_pr_body(
            "Fix the auth bug",
            "Fixed authentication issue by updating token validation.",
            &files,
            "abc123",
        );

        assert!(body.contains("Fix the auth bug"));
        assert!(body.contains("abc123"));
        assert!(body.contains("src/main.rs"));
        assert!(body.contains("+10"));
        assert!(body.contains("-5"));
        assert!(body.contains("infinite-improbability-drive"));
    }

    #[test]
    fn pr_body_handles_empty_files() {
        let manager = PRManager::new(PathBuf::from("/tmp"));

        let body = manager.generate_pr_body("Do something", "Did it", &[], "xyz789");

        assert!(body.contains("Do something"));
        assert!(body.contains("Did it"));
        assert!(!body.contains("Files Changed"));
    }

    #[test]
    fn conflict_strategy_default() {
        assert_eq!(ConflictStrategy::default(), ConflictStrategy::AutoResolve);
    }

    #[test]
    fn parse_conflicts_extracts_files() {
        let manager = PRManager::new(PathBuf::from("/tmp"));

        let output = r#"diff --git a/file1.rs b/file1.rs
<<<<<<< HEAD
some content
=======
other content
>>>>>>> branch
diff --git a/file2.rs b/file2.rs
<<<<<<< HEAD
more
=======
stuff
>>>>>>> branch
<<<<<<< HEAD
even more
=======
conflicts
>>>>>>> branch"#;

        let conflicts = manager.parse_conflicts(output);

        assert_eq!(conflicts.len(), 2);
        assert_eq!(conflicts[0].path, PathBuf::from("file1.rs"));
        assert_eq!(conflicts[0].conflict_count, 1);
        assert!(conflicts[0].is_simple);
        assert_eq!(conflicts[1].path, PathBuf::from("file2.rs"));
        assert_eq!(conflicts[1].conflict_count, 2);
        assert!(conflicts[1].is_simple);
    }
}
