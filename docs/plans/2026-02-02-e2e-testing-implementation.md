# E2E Testing Infrastructure Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Enable real end-to-end testing of the spawn framework using actual Claude and Gemini CLIs against ephemeral GitHub repositories.

**Architecture:** Convert spawn to async, wire WatcherAgent into Spawner, build E2E test harness with fixture-driven tests and GitHub repo lifecycle management.

**Tech Stack:** Rust, tokio async, serde for YAML fixtures, gh CLI for GitHub operations.

**Design Document:** `docs/plans/2026-02-02-e2e-testing-design.md`

---

## Task 1: Make CLI Async

**Files:**
- Modify: `core/src/main.rs`

**Step 1: Add tokio main attribute**

Replace the current `fn main()` with async main:

```rust
//! Infinite Improbability Drive CLI
//!
//! CLI tool for spawning sandboxed LLM instances.

use std::path::PathBuf;

use improbability_drive::runner::{ClaudeRunner, GeminiRunner, LLMRunner};
use improbability_drive::sandbox::WorktreeSandbox;
use improbability_drive::spawn::Spawner;
use improbability_drive::{SandboxManifest, SpawnConfig, SpawnStatus};

#[tokio::main]
async fn main() {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    // Parse args (basic for now - will add clap in later phase)
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        eprintln!("Usage: {} <prompt>", args[0]);
        eprintln!("\nSpawns a sandboxed LLM instance with the given prompt.");
        eprintln!("\nEnvironment variables:");
        eprintln!("  SPAWN_RUNNER=claude|gemini  Select LLM runner (default: claude)");
        std::process::exit(1);
    }

    let prompt = args[1..].join(" ");

    // Get current repo path
    let repo_path = std::env::current_dir().expect("failed to get current directory");

    // Setup directories
    let logs_dir = PathBuf::from(".improbability-drive/spawns");
    let sandbox_dir = std::env::temp_dir().join("improbability-drive-sandboxes");

    // Select runner based on environment variable
    let runner_name = std::env::var("SPAWN_RUNNER").unwrap_or_else(|_| "claude".to_string());
    let runner: Box<dyn LLMRunner> = match runner_name.as_str() {
        "gemini" => {
            tracing::info!("using Gemini runner");
            Box::new(GeminiRunner::new())
        }
        _ => {
            tracing::info!("using Claude runner");
            Box::new(ClaudeRunner::new())
        }
    };

    // Create spawner
    let provider = WorktreeSandbox::new(repo_path, Some(sandbox_dir));
    let spawner = Spawner::new(provider, logs_dir);

    // Create config
    let config = SpawnConfig::new(&prompt);
    let manifest = SandboxManifest::default();

    // Run spawn
    tracing::info!(prompt = %prompt, "starting spawn");

    match spawner.spawn(config, manifest, runner).await {
        Ok(result) => {
            println!("\n{}", "=".repeat(60));
            println!("Spawn Complete: {}", result.spawn_id);
            println!("{}", "=".repeat(60));
            println!();
            println!("Status: {:?}", result.status);
            println!("Duration: {:?}", result.duration);
            println!();
            println!("Summary:");
            println!("  {}", result.summary);
            println!();
            println!("Logs: {}", result.logs.stdout.parent().unwrap().display());

            if result.status != SpawnStatus::Success {
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("Spawn failed: {}", e);
            std::process::exit(1);
        }
    }
}
```

**Step 2: Verify it compiles**

Run: `cd core && cargo check 2>&1 | head -20`
Expected: Error about `spawner.spawn()` signature (expected since we haven't updated spawn.rs yet)

**Step 3: Commit CLI changes**

```bash
git add core/src/main.rs
git commit -m "feat: make CLI async with configurable runner"
```

---

## Task 2: Make Spawner Async with Runner Parameter

**Files:**
- Modify: `core/src/spawn.rs`

**Step 1: Add runner imports and update spawn signature**

Add to imports at top of `core/src/spawn.rs`:

```rust
use crate::runner::LLMRunner;
```

**Step 2: Update Spawner struct to be generic over runner**

The Spawner needs to accept a runner at spawn time. Update the `spawn` method signature:

```rust
impl<P: SandboxProvider> Spawner<P> {
    /// Creates a new spawner with the given sandbox provider.
    pub fn new(provider: P, logs_dir: PathBuf) -> Self {
        Self { provider, logs_dir }
    }

    /// Spawns a sandboxed LLM with the given configuration.
    ///
    /// The runner parameter allows selecting between Claude and Gemini.
    pub async fn spawn(
        &self,
        config: SpawnConfig,
        manifest: SandboxManifest,
        runner: Box<dyn LLMRunner>,
    ) -> Result<SpawnResult> {
        // Generate spawn ID
        let spawn_id = uuid::Uuid::new_v4().to_string();

        // Create logs directory for this spawn
        let spawn_logs_dir = self.logs_dir.join(&spawn_id);
        std::fs::create_dir_all(&spawn_logs_dir)?;

        // Create log files
        let logs = SpawnLogs {
            stdout: spawn_logs_dir.join("stdout.log"),
            stderr: spawn_logs_dir.join("stderr.log"),
            events: spawn_logs_dir.join("events.jsonl"),
        };

        // Write config to logs
        let config_path = spawn_logs_dir.join("config.json");
        let config_json = serde_json::to_string_pretty(&config)
            .map_err(|e| Error::Config(format!("failed to serialize config: {}", e)))?;
        std::fs::write(&config_path, config_json)?;

        // Write manifest to logs
        let manifest_path = spawn_logs_dir.join("manifest.json");
        let manifest_json = serde_json::to_string_pretty(&manifest)
            .map_err(|e| Error::Config(format!("failed to serialize manifest: {}", e)))?;
        std::fs::write(&manifest_path, manifest_json)?;

        // Create sandbox
        let start_time = std::time::Instant::now();
        let mut sandbox = self.provider.create(manifest.clone())?;

        tracing::info!(
            spawn_id = %spawn_id,
            sandbox_path = ?sandbox.path(),
            mode = ?config.mode,
            runner = %runner.name(),
            "created spawn sandbox"
        );

        // TODO: Phase 2 integration - WatcherAgent will be wired here
        // For now, just clean up and return a basic result
        let _ = runner; // Silence unused warning until watcher integration
        let duration = start_time.elapsed();
        sandbox.cleanup()?;

        Ok(SpawnResult {
            status: SpawnStatus::Success,
            spawn_id,
            duration,
            files_changed: vec![],
            commits: vec![],
            summary: format!(
                "Sandbox created and cleaned up successfully. Prompt: {}",
                config.prompt
            ),
            pr_url: None,
            logs,
        })
    }
}
```

**Step 3: Update tests to use async**

Update the test module in `core/src/spawn.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::ClaudeRunner;
    use crate::sandbox::WorktreeSandbox;
    use std::process::Command;
    use tempfile::TempDir;

    /// Helper to create a temp git repo for testing.
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

        // Create an initial commit (required for worktree creation)
        std::fs::write(temp_dir.path().join("README.md"), "# Test Repo\n")
            .expect("failed to create readme");

        Command::new("git")
            .args(["add", "."])
            .current_dir(temp_dir.path())
            .output()
            .expect("failed to stage files");

        Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(temp_dir.path())
            .output()
            .expect("failed to commit");

        temp_dir
    }

    #[tokio::test]
    async fn spawner_creates_logs_directory() {
        let repo_dir = create_temp_git_repo();
        let logs_dir = repo_dir.path().join("logs");

        let provider = WorktreeSandbox::new(repo_dir.path().to_path_buf(), None);
        let spawner = Spawner::new(provider, logs_dir.clone());

        let config = SpawnConfig::new("test prompt");
        let manifest = SandboxManifest::default();
        let runner: Box<dyn LLMRunner> = Box::new(ClaudeRunner::new());

        let result = spawner.spawn(config, manifest, runner).await.unwrap();

        // Verify logs directory was created
        assert!(logs_dir.join(&result.spawn_id).exists());
        assert!(result.logs.stdout.exists() || result.logs.stdout.parent().unwrap().exists());
    }

    #[tokio::test]
    async fn spawner_writes_config_and_manifest_to_logs() {
        let repo_dir = create_temp_git_repo();
        let logs_dir = repo_dir.path().join("logs");

        let provider = WorktreeSandbox::new(repo_dir.path().to_path_buf(), None);
        let spawner = Spawner::new(provider, logs_dir.clone());

        let config = SpawnConfig::new("test prompt");
        let manifest = SandboxManifest::default();
        let runner: Box<dyn LLMRunner> = Box::new(ClaudeRunner::new());

        let result = spawner.spawn(config, manifest, runner).await.unwrap();

        // Verify config and manifest files
        let spawn_dir = logs_dir.join(&result.spawn_id);
        assert!(spawn_dir.join("config.json").exists());
        assert!(spawn_dir.join("manifest.json").exists());
    }
}
```

**Step 4: Run tests**

Run: `cd core && cargo test spawn::tests`
Expected: 2 tests pass

**Step 5: Commit**

```bash
git add core/src/spawn.rs
git commit -m "feat: make Spawner::spawn async with runner parameter"
```

---

## Task 3: Wire WatcherAgent into Spawner

**Files:**
- Modify: `core/src/spawn.rs`

**Step 1: Add watcher imports**

Add to imports at top of `core/src/spawn.rs`:

```rust
use crate::monitor::TimeoutConfig;
use crate::runner::{LLMRunner, LLMSpawnConfig};
use crate::watcher::{RecoveryStrategy, TerminationReason, WatcherAgent, WatcherConfig};
```

**Step 2: Create WatcherConfig from SpawnConfig helper**

Add this impl block after SpawnConfig:

```rust
impl From<&SpawnConfig> for WatcherConfig {
    fn from(config: &SpawnConfig) -> Self {
        Self {
            timeout: TimeoutConfig {
                idle_timeout: config.idle_timeout,
                total_timeout: config.total_timeout,
            },
            recovery_strategy: RecoveryStrategy::Moderate,
            max_escalations: config.max_permission_escalations,
        }
    }
}
```

**Step 3: Update spawn method to use WatcherAgent**

Replace the TODO section in the `spawn` method with actual watcher integration:

```rust
    pub async fn spawn(
        &self,
        config: SpawnConfig,
        manifest: SandboxManifest,
        runner: Box<dyn LLMRunner>,
    ) -> Result<SpawnResult> {
        // Generate spawn ID
        let spawn_id = uuid::Uuid::new_v4().to_string();

        // Create logs directory for this spawn
        let spawn_logs_dir = self.logs_dir.join(&spawn_id);
        std::fs::create_dir_all(&spawn_logs_dir)?;

        // Create log files
        let logs = SpawnLogs {
            stdout: spawn_logs_dir.join("stdout.log"),
            stderr: spawn_logs_dir.join("stderr.log"),
            events: spawn_logs_dir.join("events.jsonl"),
        };

        // Write config to logs
        let config_path = spawn_logs_dir.join("config.json");
        let config_json = serde_json::to_string_pretty(&config)
            .map_err(|e| Error::Config(format!("failed to serialize config: {}", e)))?;
        std::fs::write(&config_path, config_json)?;

        // Write manifest to logs
        let manifest_path = spawn_logs_dir.join("manifest.json");
        let manifest_json = serde_json::to_string_pretty(&manifest)
            .map_err(|e| Error::Config(format!("failed to serialize manifest: {}", e)))?;
        std::fs::write(&manifest_path, manifest_json)?;

        let start_time = std::time::Instant::now();

        tracing::info!(
            spawn_id = %spawn_id,
            mode = ?config.mode,
            runner = %runner.name(),
            "starting spawn with watcher"
        );

        // Create watcher config from spawn config
        let watcher_config = WatcherConfig::from(&config);

        // Create watcher agent with a dummy provider (watcher creates its own sandbox)
        // We need to clone the provider for the watcher
        let watcher = WatcherAgent::new(
            self.provider.clone(),
            runner,
            watcher_config,
        );

        // Run the watcher
        let watcher_result = watcher.run(config.prompt.clone(), manifest).await?;

        let duration = start_time.elapsed();

        // Convert WatcherResult to SpawnResult
        let status = if watcher_result.success {
            SpawnStatus::Success
        } else {
            match &watcher_result.termination_reason {
                Some(TerminationReason::Timeout(_)) => SpawnStatus::TimedOut,
                _ => SpawnStatus::Failed,
            }
        };

        let summary = match &watcher_result.termination_reason {
            Some(TerminationReason::Success) => {
                format!(
                    "Completed successfully. Files read: {}, written: {}",
                    watcher_result.progress.files_read,
                    watcher_result.progress.files_written
                )
            }
            Some(TerminationReason::Timeout(reason)) => {
                format!("Timed out: {:?}", reason)
            }
            Some(TerminationReason::LLMError(msg)) => {
                format!("LLM error: {}", msg)
            }
            Some(TerminationReason::PermissionError(msg)) => {
                format!("Permission error: {}", msg)
            }
            Some(TerminationReason::EscalationLimitReached) => {
                "Escalation limit reached".to_string()
            }
            None => "Unknown termination".to_string(),
        };

        // Extract commits from progress
        let commits = watcher_result
            .progress
            .commits
            .iter()
            .map(|c| CommitInfo {
                hash: c.hash.clone(),
                message: c.message.clone(),
            })
            .collect();

        Ok(SpawnResult {
            status,
            spawn_id,
            duration,
            files_changed: vec![], // TODO: Extract from watcher result
            commits,
            summary,
            pr_url: None, // TODO: Extract from PR creation
            logs,
        })
    }
```

**Step 4: Make SandboxProvider Clone**

The watcher needs to own a clone of the provider. Add Clone bound to Spawner:

In `core/src/spawn.rs`, update Spawner:

```rust
/// Spawner that creates and manages sandboxed LLM instances.
pub struct Spawner<P: SandboxProvider + Clone> {
    provider: P,
    logs_dir: PathBuf,
}

impl<P: SandboxProvider + Clone> Spawner<P> {
```

**Step 5: Verify it compiles**

Run: `cd core && cargo check`
Expected: Compiles (may have warnings about unused fields)

**Step 6: Run tests**

Run: `cd core && cargo test spawn::tests`
Expected: Tests pass

**Step 7: Commit**

```bash
git add core/src/spawn.rs
git commit -m "feat: wire WatcherAgent into Spawner for full lifecycle"
```

---

## Task 4: Add E2E Module Structure

**Files:**
- Create: `core/src/e2e/mod.rs`
- Create: `core/src/e2e/fixture.rs`
- Create: `core/src/e2e/repo.rs`
- Create: `core/src/e2e/validator.rs`
- Create: `core/src/e2e/harness.rs`
- Modify: `core/src/lib.rs`
- Modify: `core/Cargo.toml`

**Step 1: Add serde_yaml dependency**

Add to `core/Cargo.toml` under `[dependencies]`:

```toml
serde_yaml = "0.9"
```

**Step 2: Create e2e module file**

Create `core/src/e2e/mod.rs`:

```rust
//! End-to-end testing infrastructure.
//!
//! Provides fixture-driven E2E tests using real LLM runners
//! against ephemeral GitHub repositories.

pub mod fixture;
pub mod harness;
pub mod repo;
pub mod validator;

pub use fixture::{Fixture, RunnerType, ValidationConfig, ValidationLevel};
pub use harness::{E2EHarness, E2EResult};
pub use repo::EphemeralRepo;
pub use validator::Validator;
```

**Step 3: Create fixture.rs**

Create `core/src/e2e/fixture.rs`:

```rust
//! Test fixture loading and parsing.

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Type of LLM runner to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RunnerType {
    #[default]
    Claude,
    Gemini,
}

/// Validation level for test results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ValidationLevel {
    /// Just check expected files exist.
    #[default]
    FileExists,
    /// Files exist + build succeeds.
    Build,
    /// Build + unit tests pass.
    Test,
    /// Build + tests + e2e tests pass.
    Full,
}

/// Validation configuration for a fixture.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationConfig {
    /// Validation level.
    #[serde(default)]
    pub level: ValidationLevel,

    /// Expected files to exist.
    #[serde(default)]
    pub expected_files: Vec<String>,

    /// Expected file contents (path -> content).
    #[serde(default)]
    pub expected_content: HashMap<String, String>,

    /// Build command to run.
    #[serde(default)]
    pub build_command: Option<String>,

    /// Test command to run.
    #[serde(default)]
    pub test_command: Option<String>,

    /// E2E test command to run.
    #[serde(default)]
    pub e2e_command: Option<String>,
}

impl Default for ValidationConfig {
    fn default() -> Self {
        Self {
            level: ValidationLevel::FileExists,
            expected_files: Vec::new(),
            expected_content: HashMap::new(),
            build_command: None,
            test_command: None,
            e2e_command: None,
        }
    }
}

/// A test fixture defining a prompt and validation criteria.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fixture {
    /// Fixture name.
    pub name: String,

    /// Description of what this fixture tests.
    #[serde(default)]
    pub description: String,

    /// Which runner to use.
    #[serde(default)]
    pub runner: RunnerType,

    /// The prompt to send to the LLM.
    pub prompt: String,

    /// Validation configuration.
    #[serde(default)]
    pub validation: ValidationConfig,

    /// Timeout in seconds.
    #[serde(default = "default_timeout")]
    pub timeout: u64,
}

fn default_timeout() -> u64 {
    300 // 5 minutes
}

impl Fixture {
    /// Loads a fixture from a YAML file.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let content = std::fs::read_to_string(path.as_ref())
            .map_err(|e| Error::IO(e))?;

        serde_yaml::from_str(&content)
            .map_err(|e| Error::Config(format!("failed to parse fixture: {}", e)))
    }

    /// Returns the timeout as a Duration.
    pub fn timeout_duration(&self) -> Duration {
        Duration::from_secs(self.timeout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_parses_minimal_yaml() {
        let yaml = r#"
name: test
prompt: "Create hello.txt"
"#;
        let fixture: Fixture = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(fixture.name, "test");
        assert_eq!(fixture.prompt, "Create hello.txt");
        assert_eq!(fixture.runner, RunnerType::Claude);
        assert_eq!(fixture.validation.level, ValidationLevel::FileExists);
    }

    #[test]
    fn fixture_parses_full_yaml() {
        let yaml = r#"
name: full-test
description: "A complete test"
runner: gemini
prompt: "Build an app"
validation:
  level: full
  expected_files:
    - "Cargo.toml"
    - "src/main.rs"
  build_command: "cargo build"
  test_command: "cargo test"
timeout: 1800
"#;
        let fixture: Fixture = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(fixture.name, "full-test");
        assert_eq!(fixture.runner, RunnerType::Gemini);
        assert_eq!(fixture.validation.level, ValidationLevel::Full);
        assert_eq!(fixture.timeout, 1800);
    }
}
```

**Step 4: Create repo.rs**

Create `core/src/e2e/repo.rs`:

```rust
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
        let name = format!("{}-{}", prefix, uuid::Uuid::new_v4().to_string()[..8].to_string());
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
```

**Step 5: Create validator.rs**

Create `core/src/e2e/validator.rs`:

```rust
//! Result validation engine.

use std::path::Path;
use std::process::Command;

use crate::error::{Error, Result};

use super::fixture::{ValidationConfig, ValidationLevel};

/// Result of validation.
#[derive(Debug)]
pub struct ValidationResult {
    /// Whether validation passed.
    pub passed: bool,
    /// Validation messages.
    pub messages: Vec<String>,
}

/// Validates spawn results against fixture expectations.
pub struct Validator;

impl Validator {
    /// Validates the repository state against the fixture config.
    pub fn validate(repo_path: &Path, config: &ValidationConfig) -> Result<ValidationResult> {
        let mut messages = Vec::new();
        let mut passed = true;

        // Check expected files exist
        for file in &config.expected_files {
            let file_path = repo_path.join(file);
            if !file_path.exists() {
                messages.push(format!("Missing expected file: {}", file));
                passed = false;
            } else {
                messages.push(format!("Found expected file: {}", file));
            }
        }

        // Check expected content
        for (file, expected) in &config.expected_content {
            let file_path = repo_path.join(file);
            match std::fs::read_to_string(&file_path) {
                Ok(content) => {
                    if content.contains(expected) {
                        messages.push(format!("Content check passed: {}", file));
                    } else {
                        messages.push(format!(
                            "Content check failed: {} (expected to contain '{}')",
                            file, expected
                        ));
                        passed = false;
                    }
                }
                Err(e) => {
                    messages.push(format!("Failed to read {}: {}", file, e));
                    passed = false;
                }
            }
        }

        // If we're only checking files, we're done
        if config.level == ValidationLevel::FileExists {
            return Ok(ValidationResult { passed, messages });
        }

        // Run build command
        if let Some(cmd) = &config.build_command {
            let result = Self::run_command(repo_path, cmd)?;
            if result {
                messages.push(format!("Build passed: {}", cmd));
            } else {
                messages.push(format!("Build failed: {}", cmd));
                passed = false;
            }
        }

        if config.level == ValidationLevel::Build {
            return Ok(ValidationResult { passed, messages });
        }

        // Run test command
        if let Some(cmd) = &config.test_command {
            let result = Self::run_command(repo_path, cmd)?;
            if result {
                messages.push(format!("Tests passed: {}", cmd));
            } else {
                messages.push(format!("Tests failed: {}", cmd));
                passed = false;
            }
        }

        if config.level == ValidationLevel::Test {
            return Ok(ValidationResult { passed, messages });
        }

        // Run e2e command
        if let Some(cmd) = &config.e2e_command {
            let result = Self::run_command(repo_path, cmd)?;
            if result {
                messages.push(format!("E2E tests passed: {}", cmd));
            } else {
                messages.push(format!("E2E tests failed: {}", cmd));
                passed = false;
            }
        }

        Ok(ValidationResult { passed, messages })
    }

    /// Runs a shell command and returns whether it succeeded.
    fn run_command(cwd: &Path, cmd: &str) -> Result<bool> {
        let output = Command::new("sh")
            .args(["-c", cmd])
            .current_dir(cwd)
            .output()
            .map_err(|e| Error::IO(e))?;

        Ok(output.status.success())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn validator_checks_file_exists() {
        let temp = TempDir::new().unwrap();
        std::fs::write(temp.path().join("hello.txt"), "Hello").unwrap();

        let config = ValidationConfig {
            level: ValidationLevel::FileExists,
            expected_files: vec!["hello.txt".to_string()],
            ..Default::default()
        };

        let result = Validator::validate(temp.path(), &config).unwrap();
        assert!(result.passed);
    }

    #[test]
    fn validator_fails_missing_file() {
        let temp = TempDir::new().unwrap();

        let config = ValidationConfig {
            level: ValidationLevel::FileExists,
            expected_files: vec!["missing.txt".to_string()],
            ..Default::default()
        };

        let result = Validator::validate(temp.path(), &config).unwrap();
        assert!(!result.passed);
    }
}
```

**Step 6: Create harness.rs**

Create `core/src/e2e/harness.rs`:

```rust
//! Main E2E test orchestrator.

use std::time::Duration;

use crate::error::Result;
use crate::runner::{ClaudeRunner, GeminiRunner, LLMRunner};
use crate::sandbox::WorktreeSandbox;
use crate::spawn::{SpawnConfig, SpawnResult, Spawner};

use super::fixture::{Fixture, RunnerType};
use super::repo::EphemeralRepo;
use super::validator::{ValidationResult, Validator};

/// Result of an E2E test run.
#[derive(Debug)]
pub struct E2EResult {
    /// The fixture that was run.
    pub fixture_name: String,
    /// Whether the spawn succeeded.
    pub spawn_success: bool,
    /// Spawn result (if successful).
    pub spawn_result: Option<SpawnResult>,
    /// Validation result.
    pub validation: Option<ValidationResult>,
    /// Overall pass/fail.
    pub passed: bool,
    /// Error message if failed.
    pub error: Option<String>,
}

/// E2E test harness.
pub struct E2EHarness {
    /// GitHub organization for ephemeral repos.
    org: String,
}

impl E2EHarness {
    /// Creates a new E2E harness for the given organization.
    pub fn new(org: impl Into<String>) -> Self {
        Self { org: org.into() }
    }

    /// Runs a fixture and returns the result.
    pub async fn run_fixture(&self, fixture: &Fixture) -> E2EResult {
        let fixture_name = fixture.name.clone();

        // Create ephemeral repo
        let repo = match EphemeralRepo::create(&self.org, "e2e") {
            Ok(r) => r,
            Err(e) => {
                return E2EResult {
                    fixture_name,
                    spawn_success: false,
                    spawn_result: None,
                    validation: None,
                    passed: false,
                    error: Some(format!("Failed to create repo: {}", e)),
                };
            }
        };

        tracing::info!(
            fixture = %fixture.name,
            repo = %repo.full_name(),
            "running E2E fixture"
        );

        // Create runner
        let runner: Box<dyn LLMRunner> = match fixture.runner {
            RunnerType::Claude => Box::new(ClaudeRunner::new()),
            RunnerType::Gemini => Box::new(GeminiRunner::new()),
        };

        // Create spawner
        let logs_dir = repo.path().join(".e2e-logs");
        let provider = WorktreeSandbox::new(repo.path().clone(), None);
        let spawner = Spawner::new(provider, logs_dir);

        // Configure spawn
        let config = SpawnConfig::new(&fixture.prompt)
            .with_total_timeout(Duration::from_secs(fixture.timeout));

        let manifest = crate::SandboxManifest::default();

        // Run spawn
        let spawn_result = match spawner.spawn(config, manifest, runner).await {
            Ok(result) => result,
            Err(e) => {
                return E2EResult {
                    fixture_name,
                    spawn_success: false,
                    spawn_result: None,
                    validation: None,
                    passed: false,
                    error: Some(format!("Spawn failed: {}", e)),
                };
            }
        };

        let spawn_success = spawn_result.status == crate::SpawnStatus::Success;

        // Validate results
        let validation = match Validator::validate(repo.path(), &fixture.validation) {
            Ok(v) => Some(v),
            Err(e) => {
                return E2EResult {
                    fixture_name,
                    spawn_success,
                    spawn_result: Some(spawn_result),
                    validation: None,
                    passed: false,
                    error: Some(format!("Validation error: {}", e)),
                };
            }
        };

        let passed = spawn_success && validation.as_ref().map(|v| v.passed).unwrap_or(false);

        E2EResult {
            fixture_name,
            spawn_success,
            spawn_result: Some(spawn_result),
            validation,
            passed,
            error: None,
        }
    }
}
```

**Step 7: Add e2e module to lib.rs**

Add to `core/src/lib.rs`:

```rust
pub mod e2e;
```

And add exports:

```rust
pub use e2e::{E2EHarness, E2EResult, Fixture, RunnerType, ValidationLevel};
```

**Step 8: Run tests**

Run: `cd core && cargo test e2e`
Expected: Tests pass (fixture and validator tests)

**Step 9: Commit**

```bash
git add core/src/e2e/ core/src/lib.rs core/Cargo.toml
git commit -m "feat: add E2E test infrastructure module"
```

---

## Task 5: Create Test Fixtures

**Files:**
- Create: `core/tests/e2e/fixtures/smoke-hello.yaml`
- Create: `core/tests/e2e/fixtures/code-generation.yaml`
- Create: `core/tests/e2e/fixtures/full-web-app.yaml`

**Step 1: Create fixtures directory**

```bash
mkdir -p core/tests/e2e/fixtures
```

**Step 2: Create smoke-hello.yaml**

Create `core/tests/e2e/fixtures/smoke-hello.yaml`:

```yaml
name: smoke-hello
description: Minimal smoke test - create a single file
runner: claude

prompt: |
  Create a file called hello.txt containing exactly:
  Hello, World!

validation:
  level: file_exists
  expected_files:
    - hello.txt
  expected_content:
    hello.txt: "Hello, World!"

timeout: 60
```

**Step 3: Create code-generation.yaml**

Create `core/tests/e2e/fixtures/code-generation.yaml`:

```yaml
name: code-generation
description: Generate a simple Rust function with tests
runner: claude

prompt: |
  Create a Rust library that provides a function to add two numbers.

  Requirements:
  - Create Cargo.toml for a library crate named "adder"
  - Create src/lib.rs with a function `add(a: i32, b: i32) -> i32`
  - Include unit tests that verify add(2, 2) == 4 and add(-1, 1) == 0

  Make sure the tests pass when running `cargo test`.

validation:
  level: test
  expected_files:
    - Cargo.toml
    - src/lib.rs
  build_command: cargo build
  test_command: cargo test

timeout: 300
```

**Step 4: Create full-web-app.yaml**

Create `core/tests/e2e/fixtures/full-web-app.yaml`:

```yaml
name: full-web-app
description: Complete web application with authentication and tests
runner: claude

prompt: |
  Build a web application that is a simple web UI to an SQLite database.

  Requirements:
  - Use Rust with Axum for the web framework
  - SQLite database for storage
  - JWT authentication using a locally generated private key
  - REST API for CRUD operations on a "notes" table (id, title, content, created_at)
  - Simple HTML UI for login and viewing/creating notes
  - Unit tests for the authentication logic
  - Integration tests that show the ability to add and delete notes through the API
  - E2E test of login flow using the test client

  The application should:
  1. Compile with `cargo build`
  2. Pass unit tests with `cargo test --lib`
  3. Pass integration tests with `cargo test --test integration`

validation:
  level: full
  expected_files:
    - Cargo.toml
    - src/main.rs
    - src/lib.rs
    - tests/integration.rs
  build_command: cargo build --release
  test_command: cargo test --lib
  e2e_command: cargo test --test integration

timeout: 1800
```

**Step 5: Commit fixtures**

```bash
git add core/tests/e2e/fixtures/
git commit -m "feat: add E2E test fixtures"
```

---

## Task 6: Create E2E Test Entry Point

**Files:**
- Create: `core/tests/e2e/e2e_test.rs`

**Step 1: Create test file**

Create `core/tests/e2e/e2e_test.rs`:

```rust
//! E2E integration tests.
//!
//! These tests create real GitHub repos and run real LLM commands.
//! They require:
//! - `gh` CLI authenticated
//! - `claude` or `gemini` CLI available
//!
//! Run with: `cargo test --test e2e_test`
//! Run specific: `cargo test --test e2e_test smoke_hello`

use std::path::PathBuf;

use improbability_drive::e2e::{E2EHarness, Fixture};

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("e2e")
        .join("fixtures")
}

#[tokio::test]
#[ignore] // Run manually with --ignored
async fn smoke_hello() {
    let harness = E2EHarness::new("epiphytic");
    let fixture = Fixture::load(fixtures_dir().join("smoke-hello.yaml"))
        .expect("failed to load fixture");

    let result = harness.run_fixture(&fixture).await;

    println!("\n=== E2E Result ===");
    println!("Fixture: {}", result.fixture_name);
    println!("Spawn success: {}", result.spawn_success);
    println!("Passed: {}", result.passed);
    if let Some(validation) = &result.validation {
        println!("Validation messages:");
        for msg in &validation.messages {
            println!("  - {}", msg);
        }
    }
    if let Some(error) = &result.error {
        println!("Error: {}", error);
    }

    assert!(result.passed, "E2E test failed: {:?}", result.error);
}

#[tokio::test]
#[ignore]
async fn code_generation() {
    let harness = E2EHarness::new("epiphytic");
    let fixture = Fixture::load(fixtures_dir().join("code-generation.yaml"))
        .expect("failed to load fixture");

    let result = harness.run_fixture(&fixture).await;

    println!("\n=== E2E Result ===");
    println!("Fixture: {}", result.fixture_name);
    println!("Spawn success: {}", result.spawn_success);
    println!("Passed: {}", result.passed);
    if let Some(spawn_result) = &result.spawn_result {
        println!("Duration: {:?}", spawn_result.duration);
        println!("Summary: {}", spawn_result.summary);
    }
    if let Some(validation) = &result.validation {
        println!("Validation:");
        for msg in &validation.messages {
            println!("  - {}", msg);
        }
    }
    if let Some(error) = &result.error {
        println!("Error: {}", error);
    }

    assert!(result.passed, "E2E test failed: {:?}", result.error);
}

#[tokio::test]
#[ignore]
async fn full_web_app() {
    let harness = E2EHarness::new("epiphytic");
    let fixture = Fixture::load(fixtures_dir().join("full-web-app.yaml"))
        .expect("failed to load fixture");

    let result = harness.run_fixture(&fixture).await;

    println!("\n=== E2E Result ===");
    println!("Fixture: {}", result.fixture_name);
    println!("Spawn success: {}", result.spawn_success);
    println!("Passed: {}", result.passed);
    if let Some(spawn_result) = &result.spawn_result {
        println!("Duration: {:?}", spawn_result.duration);
        println!("Summary: {}", spawn_result.summary);
        println!("Commits: {}", spawn_result.commits.len());
    }
    if let Some(validation) = &result.validation {
        println!("Validation:");
        for msg in &validation.messages {
            println!("  - {}", msg);
        }
    }
    if let Some(error) = &result.error {
        println!("Error: {}", error);
    }

    assert!(result.passed, "E2E test failed: {:?}", result.error);
}

/// Test that runs with Gemini instead of Claude.
#[tokio::test]
#[ignore]
async fn smoke_hello_gemini() {
    let harness = E2EHarness::new("epiphytic");

    // Load and modify fixture for Gemini
    let yaml = std::fs::read_to_string(fixtures_dir().join("smoke-hello.yaml"))
        .expect("failed to read fixture");
    let yaml = yaml.replace("runner: claude", "runner: gemini");
    let fixture: Fixture = serde_yaml::from_str(&yaml).expect("failed to parse");

    let result = harness.run_fixture(&fixture).await;

    println!("\n=== E2E Result (Gemini) ===");
    println!("Passed: {}", result.passed);

    assert!(result.passed, "E2E test failed: {:?}", result.error);
}
```

**Step 2: Verify test compiles**

Run: `cd core && cargo test --test e2e_test --no-run`
Expected: Compiles successfully

**Step 3: Commit**

```bash
git add core/tests/e2e/
git commit -m "feat: add E2E test entry point"
```

---

## Task 7: Integration Test Without GitHub

**Files:**
- Create: `core/tests/integration/spawn_integration.rs`

**Step 1: Create integration test for local testing**

Create directory and test file:

```bash
mkdir -p core/tests/integration
```

Create `core/tests/integration/spawn_integration.rs`:

```rust
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
```

**Step 2: Verify test compiles**

Run: `cd core && cargo test --test spawn_integration --no-run`
Expected: Compiles

**Step 3: Commit**

```bash
git add core/tests/integration/
git commit -m "feat: add local integration test for spawn"
```

---

## Summary

This implementation plan covers:

| Task | Description | Commits |
|------|-------------|---------|
| Task 1 | Make CLI async with runner selection | 1 |
| Task 2 | Make Spawner::spawn async with runner param | 1 |
| Task 3 | Wire WatcherAgent into Spawner | 1 |
| Task 4 | Add E2E module structure | 1 |
| Task 5 | Create test fixtures | 1 |
| Task 6 | Create E2E test entry point | 1 |
| Task 7 | Create local integration test | 1 |

**Total: 7 tasks, 7 commits**

**Running the E2E tests:**

```bash
# Smoke test with Claude (creates GitHub repo)
cargo test --test e2e_test smoke_hello -- --ignored --nocapture

# Smoke test with Gemini
cargo test --test e2e_test smoke_hello_gemini -- --ignored --nocapture

# Full web app test (30 minute timeout)
cargo test --test e2e_test full_web_app -- --ignored --nocapture

# Local integration test (no GitHub)
cargo test --test spawn_integration -- --ignored --nocapture
```
