//! Git worktree-based sandbox implementation.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use crate::error::{Error, Result};

use super::provider::{Sandbox, SandboxManifest, SandboxProvider};

/// A sandbox implemented using git worktrees.
///
/// This provides isolation by creating a separate worktree for each
/// spawned LLM, ensuring changes don't pollute the main working directory.
pub struct WorktreeSandboxInstance {
    /// Path to the worktree directory.
    path: PathBuf,
    /// Path to the parent git repository.
    repo_path: PathBuf,
    /// Branch name for this worktree.
    branch_name: String,
    /// The manifest used to create this sandbox.
    manifest: SandboxManifest,
    /// Whether the sandbox has been cleaned up.
    cleaned_up: bool,
}

impl Sandbox for WorktreeSandboxInstance {
    fn path(&self) -> &PathBuf {
        &self.path
    }

    fn manifest(&self) -> &SandboxManifest {
        &self.manifest
    }

    fn cleanup(&mut self) -> Result<()> {
        if self.cleaned_up {
            return Ok(());
        }

        // Remove the worktree (must run from parent repo)
        let output = Command::new("git")
            .current_dir(&self.repo_path)
            .args(["worktree", "remove", "--force"])
            .arg(&self.path)
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::SandboxCleanup {
                path: self.path.clone(),
                reason: stderr.to_string(),
            });
        }

        // Delete the branch (must run from parent repo)
        let output = Command::new("git")
            .current_dir(&self.repo_path)
            .args(["branch", "-D", &self.branch_name])
            .output()?;

        if !output.status.success() {
            // Branch deletion failure is non-fatal - worktree is already gone
            tracing::warn!(
                branch = %self.branch_name,
                "failed to delete worktree branch, may need manual cleanup"
            );
        }

        self.cleaned_up = true;
        Ok(())
    }
}

impl Drop for WorktreeSandboxInstance {
    fn drop(&mut self) {
        if !self.cleaned_up {
            if let Err(e) = self.cleanup() {
                tracing::error!(error = %e, path = ?self.path, "failed to cleanup sandbox on drop");
            }
        }
    }
}

/// Provider that creates sandboxes using git worktrees.
#[derive(Clone)]
pub struct WorktreeSandbox {
    /// Path to the git repository.
    repo_path: PathBuf,
    /// Base directory for worktrees. If None, uses a temp directory.
    base_dir: Option<PathBuf>,
    /// Counter for generating unique branch names (shared across clones).
    counter: Arc<std::sync::atomic::AtomicU64>,
}

impl WorktreeSandbox {
    /// Creates a new worktree sandbox provider.
    ///
    /// `repo_path` is the path to the git repository where worktrees will be created.
    /// If `base_dir` is provided, worktrees are created there.
    /// Otherwise, a system temp directory is used.
    pub fn new(repo_path: PathBuf, base_dir: Option<PathBuf>) -> Self {
        Self {
            repo_path,
            base_dir,
            counter: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }

    fn generate_branch_name(&self) -> String {
        let id = self
            .counter
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        format!("spawn-sandbox-{}-{}", timestamp, id)
    }

    fn get_worktree_path(&self, branch_name: &str) -> Result<PathBuf> {
        let base = match &self.base_dir {
            Some(dir) => dir.clone(),
            None => std::env::temp_dir().join("improbability-drive-sandboxes"),
        };

        // Ensure base directory exists
        std::fs::create_dir_all(&base)?;

        Ok(base.join(branch_name))
    }
}

impl SandboxProvider for WorktreeSandbox {
    type Sandbox = WorktreeSandboxInstance;

    fn repo_path(&self) -> &PathBuf {
        &self.repo_path
    }

    fn create(&self, manifest: SandboxManifest) -> Result<Self::Sandbox> {
        let branch_name = self.generate_branch_name();
        let worktree_path = self.get_worktree_path(&branch_name)?;

        // Create the worktree with a new branch (run from repo dir)
        let output = Command::new("git")
            .current_dir(&self.repo_path)
            .args(["worktree", "add", "-b", &branch_name])
            .arg(&worktree_path)
            .arg("HEAD")
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::SandboxCreation(format!(
                "git worktree add failed: {}",
                stderr
            )));
        }

        tracing::info!(
            path = ?worktree_path,
            branch = %branch_name,
            "created sandbox worktree"
        );

        Ok(WorktreeSandboxInstance {
            path: worktree_path,
            repo_path: self.repo_path.clone(),
            branch_name,
            manifest,
            cleaned_up: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Helper to create a temp git repo for testing.
    fn create_temp_git_repo() -> TempDir {
        let temp_dir = TempDir::new().expect("failed to create temp dir");

        // Initialize git repo
        Command::new("git")
            .args(["init"])
            .current_dir(temp_dir.path())
            .output()
            .expect("failed to init git repo");

        // Configure git user for commits
        Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(temp_dir.path())
            .output()
            .expect("failed to config git email");

        Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(temp_dir.path())
            .output()
            .expect("failed to config git name");

        // Create initial commit (required for worktrees)
        std::fs::write(temp_dir.path().join("README.md"), "# Test Repo\n")
            .expect("failed to write README");

        Command::new("git")
            .args(["add", "."])
            .current_dir(temp_dir.path())
            .output()
            .expect("failed to add files");

        Command::new("git")
            .args(["commit", "-m", "Initial commit"])
            .current_dir(temp_dir.path())
            .output()
            .expect("failed to create initial commit");

        temp_dir
    }

    #[test]
    fn worktree_sandbox_provider_can_be_created() {
        let git_repo = create_temp_git_repo();
        let provider = WorktreeSandbox::new(git_repo.path().to_path_buf(), None);
        assert!(provider.base_dir.is_none());

        let temp_dir = TempDir::new().unwrap();
        let provider_with_base = WorktreeSandbox::new(
            git_repo.path().to_path_buf(),
            Some(temp_dir.path().to_path_buf()),
        );
        assert!(provider_with_base.base_dir.is_some());
    }

    #[test]
    fn worktree_sandbox_generates_unique_branch_names() {
        let git_repo = create_temp_git_repo();
        let provider = WorktreeSandbox::new(git_repo.path().to_path_buf(), None);

        let name1 = provider.generate_branch_name();
        let name2 = provider.generate_branch_name();

        assert_ne!(name1, name2);
        assert!(name1.starts_with("spawn-sandbox-"));
        assert!(name2.starts_with("spawn-sandbox-"));
    }

    #[test]
    fn worktree_sandbox_creates_and_cleans_up() {
        let git_repo = create_temp_git_repo();
        let sandbox_dir = TempDir::new().expect("failed to create sandbox dir");

        let provider = WorktreeSandbox::new(
            git_repo.path().to_path_buf(),
            Some(sandbox_dir.path().to_path_buf()),
        );

        let manifest = SandboxManifest {
            readable_paths: vec!["src/**".to_string()],
            ..Default::default()
        };

        // Create sandbox
        let mut sandbox = provider.create(manifest).expect("failed to create sandbox");

        // Verify sandbox was created
        assert!(sandbox.path().exists());
        assert!(sandbox.path().is_dir());

        // The worktree should contain the README from the parent repo
        assert!(sandbox.path().join("README.md").exists());

        // Get the path before cleanup
        let sandbox_path = sandbox.path().clone();

        // Cleanup
        sandbox.cleanup().expect("failed to cleanup sandbox");

        // Verify sandbox was removed
        assert!(!sandbox_path.exists());
    }

    #[test]
    fn worktree_sandbox_cleanup_is_idempotent() {
        let git_repo = create_temp_git_repo();
        let sandbox_dir = TempDir::new().expect("failed to create sandbox dir");

        let provider = WorktreeSandbox::new(
            git_repo.path().to_path_buf(),
            Some(sandbox_dir.path().to_path_buf()),
        );
        let mut sandbox = provider
            .create(SandboxManifest::default())
            .expect("failed to create sandbox");

        // Cleanup twice should be fine
        sandbox.cleanup().expect("first cleanup failed");
        sandbox
            .cleanup()
            .expect("second cleanup should be idempotent");
    }

    #[test]
    fn worktree_sandbox_exposes_manifest() {
        let git_repo = create_temp_git_repo();
        let sandbox_dir = TempDir::new().expect("failed to create sandbox dir");

        let provider = WorktreeSandbox::new(
            git_repo.path().to_path_buf(),
            Some(sandbox_dir.path().to_path_buf()),
        );

        let manifest = SandboxManifest {
            readable_paths: vec!["test/**".to_string()],
            allowed_tools: vec!["Read".to_string()],
            ..Default::default()
        };

        let sandbox = provider
            .create(manifest.clone())
            .expect("failed to create sandbox");

        assert_eq!(sandbox.manifest().readable_paths, manifest.readable_paths);
        assert_eq!(sandbox.manifest().allowed_tools, manifest.allowed_tools);
    }
}
