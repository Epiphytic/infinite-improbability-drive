//! Infinite Improbability Drive - Claude Code plugin for spawning sandboxed LLMs
//!
//! This library provides the core functionality for launching isolated LLM instances
//! in git worktree sandboxes with intelligent resource provisioning and lifecycle management.

pub mod error;
pub mod monitor;
pub mod permissions;
pub mod pr;
pub mod runner;
pub mod sandbox;
pub mod secrets;
pub mod spawn;
pub mod watcher;

pub use error::Error;
pub use monitor::{ProgressMonitor, ProgressSummary, TimeoutConfig, TimeoutReason};
pub use permissions::{PermissionDetector, PermissionError, PermissionErrorType, PermissionFix};
pub use pr::{ConflictFile, ConflictStrategy, MergeStatus, PRManager, PullRequest};
pub use runner::{ClaudeRunner, LLMOutput, LLMResult, LLMRunner, LLMSpawnConfig};
pub use sandbox::{Sandbox, SandboxManifest, SandboxProvider};
pub use secrets::{SecretError, SecretRef, SecretSource, SecretsManager};
pub use spawn::{SpawnConfig, SpawnResult, SpawnStatus};
pub use watcher::{RecoveryStrategy, TerminationReason, WatcherAgent, WatcherConfig, WatcherResult};
