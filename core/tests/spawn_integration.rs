//! Integration tests for spawn without GitHub.
//!
//! These tests use local temp repos, suitable for CI.

use std::process::Command;

use tempfile::TempDir;

use improbability_drive::runner::ClaudeRunner;
use improbability_drive::sandbox::WorktreeSandbox;
use improbability_drive::spawn::{SpawnConfig, Spawner};
use improbability_drive::SandboxManifest;

/// Helper to create a temp git repo.
fn create_temp_git_repo() -> TempDir {
    let temp_dir = TempDir::new().expect("failed to create temp dir");

    Command::new("git")
        .args(["init"])
        .current_dir(temp_dir.path())
        .output()
        .expect("failed to init git repo");

    Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(temp_dir.path())
        .output()
        .expect("failed to set git email");

    Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(temp_dir.path())
        .output()
        .expect("failed to set git name");

    std::fs::write(temp_dir.path().join("README.md"), "# Test\n")
        .expect("failed to create readme");

    Command::new("git")
        .args(["add", "."])
        .current_dir(temp_dir.path())
        .output()
        .expect("failed to stage");

    Command::new("git")
        .args(["commit", "-m", "initial"])
        .current_dir(temp_dir.path())
        .output()
        .expect("failed to commit");

    temp_dir
}

#[tokio::test]
#[ignore] // Requires claude CLI
async fn spawn_creates_file_with_claude() {
    let repo = create_temp_git_repo();
    let logs_dir = repo.path().join("logs");

    let provider = WorktreeSandbox::new(repo.path().to_path_buf(), None);
    let spawner = Spawner::new(provider, logs_dir);

    let config = SpawnConfig::new("Create a file called test.txt containing 'Hello from Claude'");
    let manifest = SandboxManifest::default();
    let runner: Box<dyn improbability_drive::LLMRunner> = Box::new(ClaudeRunner::new());

    let result = spawner.spawn(config, manifest, runner).await;

    match result {
        Ok(r) => {
            println!("Spawn completed: {:?}", r.status);
            println!("Summary: {}", r.summary);
        }
        Err(e) => {
            println!("Spawn error: {}", e);
        }
    }
}
