//! Ephemeral GitHub repository management.

use std::path::PathBuf;
use std::process::Command;

use crate::error::{Error, Result};

/// An ephemeral GitHub repository for E2E testing.
pub struct EphemeralRepo {
    /// Organization name.
    org: String,
    /// Repository name.
    name: String,
    /// Local path to cloned repo.
    path: PathBuf,
    /// Whether the repo has been deleted.
    deleted: bool,
    /// Whether to auto-delete on drop.
    auto_delete: bool,
}

impl EphemeralRepo {
    /// Creates a new ephemeral repository.
    ///
    /// The `prefix` is typically "e2e" and `test_name` is the fixture/test name.
    /// Resulting repo name format: `{prefix}-{test_name}-{short_uuid}`
    pub fn create(org: &str, prefix: &str) -> Result<Self> {
        Self::create_with_name(org, prefix, None)
    }

    /// Creates a new ephemeral repository with a specific test name.
    ///
    /// The `test_name` is included in the repo name for clarity.
    /// Resulting repo name format: `{prefix}-{test_name}-{short_uuid}`
    pub fn create_with_name(org: &str, prefix: &str, test_name: Option<&str>) -> Result<Self> {
        let short_uuid = &uuid::Uuid::new_v4().to_string()[..8];
        let name = match test_name {
            Some(test) => {
                // Sanitize test name for use in repo name (replace non-alphanumeric with dash)
                let sanitized: String = test
                    .chars()
                    .map(|c| if c.is_alphanumeric() { c } else { '-' })
                    .collect::<String>()
                    .to_lowercase();
                format!("{}-{}-{}", prefix, sanitized, short_uuid)
            }
            None => format!("{}-{}", prefix, short_uuid),
        };
        let full_name = format!("{}/{}", org, name);

        tracing::info!(repo = %full_name, "creating ephemeral repository");

        // Create repo using gh CLI
        let output = Command::new("gh")
            .args(["repo", "create", &full_name, "--public", "--clone"])
            .output()
            .map_err(|e| Error::GitHub(format!("failed to run gh: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::GitHub(format!("failed to create repo: {}", stderr)));
        }

        // Get the cloned path (make it absolute to avoid issues with cwd changes)
        let path = std::env::current_dir()
            .map_err(|e| Error::Git(format!("failed to get current dir: {}", e)))?
            .join(&name);

        // Initialize with a minimal commit
        let readme_content = format!(
            "# E2E Test Repository\n\nCreated for automated testing.\n\nRepo: {}\n",
            full_name
        );
        std::fs::write(path.join("README.md"), readme_content)?;

        Command::new("git")
            .args(["add", "."])
            .current_dir(&path)
            .output()
            .map_err(|e| Error::Git(format!("failed to stage: {}", e)))?;

        Command::new("git")
            .args(["commit", "-m", "Initial commit for E2E test"])
            .current_dir(&path)
            .output()
            .map_err(|e| Error::Git(format!("failed to commit: {}", e)))?;

        // Ensure we're on a branch called 'main' (git init might use different defaults)
        Command::new("git")
            .args(["branch", "-M", "main"])
            .current_dir(&path)
            .output()
            .map_err(|e| Error::Git(format!("failed to rename branch to main: {}", e)))?;

        // Push initial commit to remote to establish 'main' as the default branch
        let push_output = Command::new("git")
            .args(["push", "-u", "origin", "main"])
            .current_dir(&path)
            .output()
            .map_err(|e| Error::Git(format!("failed to push: {}", e)))?;

        if !push_output.status.success() {
            let stderr = String::from_utf8_lossy(&push_output.stderr);
            return Err(Error::Git(format!(
                "failed to push initial commit: {}",
                stderr
            )));
        }

        Ok(Self {
            org: org.to_string(),
            name,
            path,
            deleted: false,
            auto_delete: true, // Default to auto-delete
        })
    }

    /// Returns the full repository name (org/repo).
    pub fn full_name(&self) -> String {
        format!("{}/{}", self.org, self.name)
    }

    /// Returns the local path to the repository.
    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    /// Sets whether the repository should be auto-deleted on drop.
    pub fn set_auto_delete(&mut self, auto_delete: bool) {
        self.auto_delete = auto_delete;
    }

    /// Keeps the repository (disables auto-delete on drop).
    pub fn keep(&mut self) {
        self.auto_delete = false;
    }

    /// Deletes the repository.
    pub fn delete(&mut self) -> Result<()> {
        if self.deleted {
            return Ok(());
        }

        let full_name = self.full_name();
        tracing::info!(repo = %full_name, "deleting ephemeral repository");

        let output = Command::new("gh")
            .args(["repo", "delete", &full_name, "--yes"])
            .output()
            .map_err(|e| Error::GitHub(format!("failed to run gh: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::warn!(repo = %full_name, error = %stderr, "failed to delete repo");
            // Don't fail - we tried our best
        }

        // Clean up local directory
        if self.path.exists() {
            let _ = std::fs::remove_dir_all(&self.path);
        }

        self.deleted = true;
        Ok(())
    }
}

impl Drop for EphemeralRepo {
    fn drop(&mut self) {
        if !self.deleted && self.auto_delete {
            let _ = self.delete();
        }
    }
}
