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
}

impl EphemeralRepo {
    /// Creates a new ephemeral repository.
    pub fn create(org: &str, prefix: &str) -> Result<Self> {
        let name = format!(
            "{}-{}",
            prefix,
            uuid::Uuid::new_v4().to_string()[..8].to_string()
        );
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

        // Get the cloned path
        let path = PathBuf::from(&name);

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

        Ok(Self {
            org: org.to_string(),
            name,
            path,
            deleted: false,
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
        if !self.deleted {
            let _ = self.delete();
        }
    }
}
