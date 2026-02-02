//! Spawn command implementation.
//!
//! This module provides the entry point for spawning sandboxed LLM instances.

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::monitor::TimeoutConfig;
use crate::prompt::augment_prompt_with_gitignore;
use crate::runner::LLMRunner;
use crate::sandbox::{SandboxManifest, SandboxProvider};
use crate::watcher::{RecoveryStrategy, TerminationReason, WatcherAgent, WatcherConfig};

/// Mode for prompt handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SpawnMode {
    /// Convert prompt to AISP format for structured communication.
    #[default]
    Aisp,
    /// Pass prompt directly without conversion.
    Passthrough,
}

/// Configuration for a spawn operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnConfig {
    /// The prompt to send to the spawned LLM.
    pub prompt: String,

    /// Mode for prompt handling.
    #[serde(default)]
    pub mode: SpawnMode,

    /// Idle timeout before termination.
    #[serde(default = "default_idle_timeout")]
    pub idle_timeout: Duration,

    /// Total timeout for the entire operation.
    #[serde(default = "default_total_timeout")]
    pub total_timeout: Duration,

    /// Maximum permission escalations allowed.
    #[serde(default = "default_max_escalations")]
    pub max_permission_escalations: u32,
}

fn default_idle_timeout() -> Duration {
    Duration::from_secs(120)
}

fn default_total_timeout() -> Duration {
    Duration::from_secs(1800)
}

fn default_max_escalations() -> u32 {
    1
}

impl SpawnConfig {
    /// Creates a new spawn configuration with the given prompt.
    pub fn new(prompt: impl Into<String>) -> Self {
        Self {
            prompt: prompt.into(),
            mode: SpawnMode::default(),
            idle_timeout: default_idle_timeout(),
            total_timeout: default_total_timeout(),
            max_permission_escalations: default_max_escalations(),
        }
    }

    /// Sets the spawn mode.
    pub fn with_mode(mut self, mode: SpawnMode) -> Self {
        self.mode = mode;
        self
    }

    /// Sets the idle timeout.
    pub fn with_idle_timeout(mut self, timeout: Duration) -> Self {
        self.idle_timeout = timeout;
        self
    }

    /// Sets the total timeout.
    pub fn with_total_timeout(mut self, timeout: Duration) -> Self {
        self.total_timeout = timeout;
        self
    }
}

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

/// Status of a completed spawn operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SpawnStatus {
    /// Spawn completed successfully.
    Success,
    /// Spawn failed due to an error.
    Failed,
    /// Spawn was terminated due to timeout.
    TimedOut,
}

/// Information about a file change made during spawn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChange {
    /// Path to the changed file (relative to worktree).
    pub path: PathBuf,
    /// Lines added.
    pub additions: u32,
    /// Lines removed.
    pub deletions: u32,
}

/// Information about a commit made during spawn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitInfo {
    /// The commit hash.
    pub hash: String,
    /// The commit message.
    pub message: String,
}

/// Paths to spawn log files.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnLogs {
    /// Path to stdout log.
    pub stdout: PathBuf,
    /// Path to stderr log.
    pub stderr: PathBuf,
    /// Path to events log.
    pub events: PathBuf,
}

/// Result of a spawn operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnResult {
    /// Status of the spawn.
    pub status: SpawnStatus,
    /// Unique identifier for this spawn.
    pub spawn_id: String,
    /// Duration of the spawn operation.
    pub duration: Duration,
    /// Files changed during the spawn.
    pub files_changed: Vec<FileChange>,
    /// Commits made during the spawn.
    pub commits: Vec<CommitInfo>,
    /// Human-readable summary.
    pub summary: String,
    /// URL of the created PR, if any.
    pub pr_url: Option<String>,
    /// Paths to log files.
    pub logs: SpawnLogs,
    /// Path to sandbox (if success, for validation before cleanup).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sandbox_path: Option<PathBuf>,
}

/// Spawner that creates and manages sandboxed LLM instances.
pub struct Spawner<P: SandboxProvider + Clone> {
    provider: P,
    logs_dir: PathBuf,
}

impl<P: SandboxProvider + Clone + 'static> Spawner<P> {
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

        let start_time = std::time::Instant::now();

        // Augment prompt with gitignore instruction if needed
        let prompt = augment_prompt_with_gitignore(&config.prompt, self.provider.repo_path());

        tracing::info!(
            spawn_id = %spawn_id,
            mode = ?config.mode,
            runner = %runner.name(),
            prompt_augmented = (prompt != config.prompt),
            "starting spawn with watcher"
        );

        // Create watcher config from spawn config
        let watcher_config = WatcherConfig::from(&config);

        // Create watcher agent
        let watcher = WatcherAgent::new(self.provider.clone(), runner, watcher_config);

        // Run the watcher with potentially augmented prompt
        let watcher_result = watcher.run(prompt, manifest).await?;

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
                    watcher_result.progress.files_read.len(),
                    watcher_result.progress.files_written.len()
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
            sandbox_path: watcher_result.sandbox_path,
        })
    }
}

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

    #[test]
    fn spawn_config_has_sensible_defaults() {
        let config = SpawnConfig::new("test prompt");

        assert_eq!(config.prompt, "test prompt");
        assert_eq!(config.mode, SpawnMode::Aisp);
        assert_eq!(config.idle_timeout, Duration::from_secs(120));
        assert_eq!(config.total_timeout, Duration::from_secs(1800));
        assert_eq!(config.max_permission_escalations, 1);
    }

    #[test]
    fn spawn_config_builder_works() {
        let config = SpawnConfig::new("my prompt")
            .with_mode(SpawnMode::Passthrough)
            .with_idle_timeout(Duration::from_secs(60))
            .with_total_timeout(Duration::from_secs(300));

        assert_eq!(config.prompt, "my prompt");
        assert_eq!(config.mode, SpawnMode::Passthrough);
        assert_eq!(config.idle_timeout, Duration::from_secs(60));
        assert_eq!(config.total_timeout, Duration::from_secs(300));
    }

    #[test]
    fn spawn_mode_serializes_correctly() {
        assert_eq!(serde_json::to_string(&SpawnMode::Aisp).unwrap(), "\"aisp\"");
        assert_eq!(
            serde_json::to_string(&SpawnMode::Passthrough).unwrap(),
            "\"passthrough\""
        );
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
