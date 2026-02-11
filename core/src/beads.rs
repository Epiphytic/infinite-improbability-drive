//! Beads integration for issue tracking.
//!
//! Wraps the `bd` CLI for programmatic beads operations.
//! All commands use `--json` flag for machine-readable output.

use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Issue priority levels (0 = critical, 4 = backlog).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Priority {
    /// Critical - must be fixed immediately.
    Critical = 0,
    /// High priority.
    High = 1,
    /// Medium priority (default).
    #[default]
    Medium = 2,
    /// Low priority.
    Low = 3,
    /// Backlog - nice to have.
    Backlog = 4,
}

impl Priority {
    /// Returns the numeric value for the CLI.
    pub fn as_i32(&self) -> i32 {
        *self as i32
    }
}

/// Issue type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum IssueType {
    /// Bug fix.
    Bug,
    /// New feature.
    Feature,
    /// General task.
    #[default]
    Task,
    /// Epic (container for related issues).
    Epic,
    /// Chore (maintenance).
    Chore,
}

impl IssueType {
    /// Returns the string value for the CLI.
    pub fn as_str(&self) -> &'static str {
        match self {
            IssueType::Bug => "bug",
            IssueType::Feature => "feature",
            IssueType::Task => "task",
            IssueType::Epic => "epic",
            IssueType::Chore => "chore",
        }
    }
}

/// Issue status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum IssueStatus {
    /// Open and ready for work.
    #[default]
    Open,
    /// Currently being worked on.
    InProgress,
    /// Completed.
    Closed,
    /// Deferred for later.
    Deferred,
}

/// Dependency type between issues.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DependencyType {
    /// Hard blocker - blocked issue hides from `bd ready`.
    Blocks,
    /// Soft link - informational only.
    Related,
    /// Epic/subtask hierarchy.
    ParentChild,
    /// Discovered during work on another issue.
    DiscoveredFrom,
}

impl DependencyType {
    /// Returns the string value for the CLI.
    pub fn as_str(&self) -> &'static str {
        match self {
            DependencyType::Blocks => "blocks",
            DependencyType::Related => "related",
            DependencyType::ParentChild => "parent-child",
            DependencyType::DiscoveredFrom => "discovered-from",
        }
    }
}

/// A beads issue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeadsIssue {
    /// Unique issue ID (e.g., "bd-42").
    pub id: String,
    /// One-line summary.
    pub title: String,
    /// Problem statement and context.
    #[serde(default)]
    pub description: Option<String>,
    /// Implementation approach.
    #[serde(default)]
    pub design: Option<String>,
    /// Success criteria.
    #[serde(default)]
    pub acceptance_criteria: Option<String>,
    /// Session handoff notes.
    #[serde(default)]
    pub notes: Option<String>,
    /// Issue status.
    pub status: String,
    /// Priority (0-4).
    pub priority: i32,
    /// Issue type.
    pub issue_type: String,
    /// Labels.
    #[serde(default)]
    pub labels: Vec<String>,
    /// Creation timestamp.
    #[serde(default)]
    pub created_at: Option<String>,
    /// Last update timestamp.
    #[serde(default)]
    pub updated_at: Option<String>,
}

/// Result of creating an issue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateResult {
    /// The created issue ID.
    pub id: String,
    /// The issue title.
    pub title: String,
    /// Status (should be "open").
    pub status: String,
    /// Priority.
    pub priority: i32,
    /// Issue type.
    pub issue_type: String,
}

/// Options for creating an issue.
#[derive(Debug, Clone, Default)]
pub struct CreateOptions {
    /// Issue description.
    pub description: Option<String>,
    /// Implementation design.
    pub design: Option<String>,
    /// Acceptance criteria.
    pub acceptance_criteria: Option<String>,
    /// Priority (default: Medium).
    pub priority: Priority,
    /// Issue type (default: Task).
    pub issue_type: IssueType,
    /// Labels to apply.
    pub labels: Vec<String>,
    /// Dependencies in format "type:id" (e.g., "blocks:bd-41").
    pub dependencies: Vec<String>,
}

/// Beads client for interacting with the bd CLI.
pub struct BeadsClient {
    /// Working directory for commands.
    work_dir: std::path::PathBuf,
}

impl BeadsClient {
    /// Creates a new beads client for the given working directory.
    pub fn new(work_dir: impl Into<std::path::PathBuf>) -> Self {
        Self {
            work_dir: work_dir.into(),
        }
    }

    /// Checks if beads is initialized in the working directory.
    pub fn is_initialized(&self) -> bool {
        self.work_dir.join(".beads").exists()
    }

    /// Initializes beads in the working directory.
    ///
    /// Creates the `.beads/` directory with database and config.
    pub fn init(&self) -> Result<()> {
        if self.is_initialized() {
            return Ok(());
        }

        let output = Command::new("bd")
            .current_dir(&self.work_dir)
            .args(["init"])
            .output()
            .map_err(|e| Error::Beads(format!("failed to run bd init: {}", e)))?;

        if !output.status.success() {
            return Err(Error::Beads(format!(
                "bd init failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        tracing::info!(path = ?self.work_dir, "initialized beads");
        Ok(())
    }

    /// Creates a new issue.
    pub fn create(&self, title: &str, options: CreateOptions) -> Result<CreateResult> {
        let mut args = vec![
            "create".to_string(),
            title.to_string(),
            "-p".to_string(),
            options.priority.as_i32().to_string(),
            "-t".to_string(),
            options.issue_type.as_str().to_string(),
            "--json".to_string(),
        ];

        if let Some(desc) = &options.description {
            args.push("-d".to_string());
            args.push(desc.clone());
        }

        if let Some(design) = &options.design {
            args.push("--design".to_string());
            args.push(design.clone());
        }

        if let Some(acceptance) = &options.acceptance_criteria {
            args.push("--acceptance".to_string());
            args.push(acceptance.clone());
        }

        if !options.labels.is_empty() {
            args.push("-l".to_string());
            args.push(options.labels.join(","));
        }

        if !options.dependencies.is_empty() {
            args.push("--deps".to_string());
            args.push(options.dependencies.join(","));
        }

        let output = Command::new("bd")
            .current_dir(&self.work_dir)
            .args(&args)
            .output()
            .map_err(|e| Error::Beads(format!("failed to run bd create: {}", e)))?;

        if !output.status.success() {
            return Err(Error::Beads(format!(
                "bd create failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        let result: CreateResult = serde_json::from_slice(&output.stdout)
            .map_err(|e| Error::Beads(format!("failed to parse bd create output: {}", e)))?;

        tracing::info!(id = %result.id, title = %result.title, "created beads issue");
        Ok(result)
    }

    /// Updates an issue's status.
    pub fn update_status(&self, id: &str, status: IssueStatus) -> Result<()> {
        let status_str = match status {
            IssueStatus::Open => "open",
            IssueStatus::InProgress => "in_progress",
            IssueStatus::Closed => "closed",
            IssueStatus::Deferred => "deferred",
        };

        let output = Command::new("bd")
            .current_dir(&self.work_dir)
            .args(["update", id, "--status", status_str, "--json"])
            .output()
            .map_err(|e| Error::Beads(format!("failed to run bd update: {}", e)))?;

        if !output.status.success() {
            return Err(Error::Beads(format!(
                "bd update failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        tracing::info!(id = %id, status = %status_str, "updated beads issue status");
        Ok(())
    }

    /// Updates an issue's notes.
    pub fn update_notes(&self, id: &str, notes: &str) -> Result<()> {
        let output = Command::new("bd")
            .current_dir(&self.work_dir)
            .args(["update", id, "--notes", notes, "--json"])
            .output()
            .map_err(|e| Error::Beads(format!("failed to run bd update: {}", e)))?;

        if !output.status.success() {
            return Err(Error::Beads(format!(
                "bd update failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        tracing::info!(id = %id, "updated beads issue notes");
        Ok(())
    }

    /// Closes an issue with an optional reason.
    pub fn close(&self, id: &str, reason: Option<&str>) -> Result<()> {
        let mut args = vec!["close", id, "--json"];

        if let Some(r) = reason {
            args.push("--reason");
            args.push(r);
        }

        let output = Command::new("bd")
            .current_dir(&self.work_dir)
            .args(&args)
            .output()
            .map_err(|e| Error::Beads(format!("failed to run bd close: {}", e)))?;

        if !output.status.success() {
            return Err(Error::Beads(format!(
                "bd close failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        tracing::info!(id = %id, "closed beads issue");
        Ok(())
    }

    /// Lists all issues.
    pub fn list(&self) -> Result<Vec<BeadsIssue>> {
        let output = Command::new("bd")
            .current_dir(&self.work_dir)
            .args(["list", "--json"])
            .output()
            .map_err(|e| Error::Beads(format!("failed to run bd list: {}", e)))?;

        if !output.status.success() {
            return Err(Error::Beads(format!(
                "bd list failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        let issues: Vec<BeadsIssue> = serde_json::from_slice(&output.stdout)
            .map_err(|e| Error::Beads(format!("failed to parse bd list output: {}", e)))?;

        Ok(issues)
    }

    /// Lists issues that are ready to work on (no blockers).
    pub fn ready(&self) -> Result<Vec<BeadsIssue>> {
        let output = Command::new("bd")
            .current_dir(&self.work_dir)
            .args(["ready", "--json"])
            .output()
            .map_err(|e| Error::Beads(format!("failed to run bd ready: {}", e)))?;

        if !output.status.success() {
            return Err(Error::Beads(format!(
                "bd ready failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        let issues: Vec<BeadsIssue> = serde_json::from_slice(&output.stdout)
            .map_err(|e| Error::Beads(format!("failed to parse bd ready output: {}", e)))?;

        Ok(issues)
    }

    /// Adds a dependency between issues.
    pub fn add_dependency(
        &self,
        from_id: &str,
        to_id: &str,
        dep_type: DependencyType,
    ) -> Result<()> {
        let output = Command::new("bd")
            .current_dir(&self.work_dir)
            .args(["dep", "add", from_id, to_id, "--type", dep_type.as_str()])
            .output()
            .map_err(|e| Error::Beads(format!("failed to run bd dep add: {}", e)))?;

        if !output.status.success() {
            return Err(Error::Beads(format!(
                "bd dep add failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        tracing::info!(from = %from_id, to = %to_id, dep_type = ?dep_type, "added beads dependency");
        Ok(())
    }

    /// Syncs beads with git (exports, commits, pulls, imports, pushes).
    pub fn sync(&self) -> Result<()> {
        let output = Command::new("bd")
            .current_dir(&self.work_dir)
            .args(["sync"])
            .output()
            .map_err(|e| Error::Beads(format!("failed to run bd sync: {}", e)))?;

        if !output.status.success() {
            // Sync may fail if there's no remote, which is OK for ephemeral repos
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !stderr.contains("no remote") && !stderr.contains("not a git repository") {
                return Err(Error::Beads(format!("bd sync failed: {}", stderr)));
            }
            tracing::warn!(error = %stderr, "bd sync warning (continuing)");
        }

        tracing::info!("synced beads");
        Ok(())
    }

    /// Shows details of a specific issue.
    pub fn show(&self, id: &str) -> Result<BeadsIssue> {
        let output = Command::new("bd")
            .current_dir(&self.work_dir)
            .args(["show", id, "--json"])
            .output()
            .map_err(|e| Error::Beads(format!("failed to run bd show: {}", e)))?;

        if !output.status.success() {
            return Err(Error::Beads(format!(
                "bd show failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        let issue: BeadsIssue = serde_json::from_slice(&output.stdout)
            .map_err(|e| Error::Beads(format!("failed to parse bd show output: {}", e)))?;

        Ok(issue)
    }
}

/// Commits beads changes with a message referencing an issue.
pub fn commit_issue_change(work_dir: &Path, issue_id: &str, message: &str) -> Result<String> {
    // Stage beads files
    let add_output = Command::new("git")
        .current_dir(work_dir)
        .args(["add", ".beads/"])
        .output()
        .map_err(|e| Error::Git(format!("failed to stage beads files: {}", e)))?;

    if !add_output.status.success() {
        return Err(Error::Git(format!(
            "git add failed: {}",
            String::from_utf8_lossy(&add_output.stderr)
        )));
    }

    // Check if there are staged changes
    let status_output = Command::new("git")
        .current_dir(work_dir)
        .args(["diff", "--cached", "--quiet"])
        .output()
        .map_err(|e| Error::Git(format!("failed to check staged changes: {}", e)))?;

    if status_output.status.success() {
        // No changes to commit
        return Ok(String::new());
    }

    // Commit with issue reference
    let commit_message = format!("{} ({})", message, issue_id);
    let commit_output = Command::new("git")
        .current_dir(work_dir)
        .args(["commit", "-m", &commit_message])
        .output()
        .map_err(|e| Error::Git(format!("failed to commit: {}", e)))?;

    if !commit_output.status.success() {
        return Err(Error::Git(format!(
            "git commit failed: {}",
            String::from_utf8_lossy(&commit_output.stderr)
        )));
    }

    // Get commit hash
    let rev_output = Command::new("git")
        .current_dir(work_dir)
        .args(["rev-parse", "HEAD"])
        .output()
        .map_err(|e| Error::Git(format!("failed to get commit hash: {}", e)))?;

    let hash = String::from_utf8_lossy(&rev_output.stdout)
        .trim()
        .to_string();
    tracing::info!(hash = %hash, issue = %issue_id, "committed beads change");
    Ok(hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_as_i32() {
        assert_eq!(Priority::Critical.as_i32(), 0);
        assert_eq!(Priority::High.as_i32(), 1);
        assert_eq!(Priority::Medium.as_i32(), 2);
        assert_eq!(Priority::Low.as_i32(), 3);
        assert_eq!(Priority::Backlog.as_i32(), 4);
    }

    #[test]
    fn issue_type_as_str() {
        assert_eq!(IssueType::Bug.as_str(), "bug");
        assert_eq!(IssueType::Feature.as_str(), "feature");
        assert_eq!(IssueType::Task.as_str(), "task");
        assert_eq!(IssueType::Epic.as_str(), "epic");
        assert_eq!(IssueType::Chore.as_str(), "chore");
    }

    #[test]
    fn dependency_type_as_str() {
        assert_eq!(DependencyType::Blocks.as_str(), "blocks");
        assert_eq!(DependencyType::Related.as_str(), "related");
        assert_eq!(DependencyType::ParentChild.as_str(), "parent-child");
        assert_eq!(DependencyType::DiscoveredFrom.as_str(), "discovered-from");
    }

    #[test]
    fn create_options_default() {
        let opts = CreateOptions::default();
        assert_eq!(opts.priority, Priority::Medium);
        assert_eq!(opts.issue_type, IssueType::Task);
        assert!(opts.labels.is_empty());
        assert!(opts.dependencies.is_empty());
    }

    #[test]
    fn beads_client_checks_initialized() {
        let client = BeadsClient::new("/nonexistent/path");
        assert!(!client.is_initialized());
    }
}
