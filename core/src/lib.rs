//! Infinite Improbability Drive - Claude Code plugin for spawning sandboxed LLMs
//!
//! This library provides the core functionality for launching isolated LLM instances
//! in git worktree sandboxes with intelligent resource provisioning and lifecycle management.

pub mod config;
pub mod error;
pub mod monitor;
pub mod permissions;
pub mod pr;
pub mod runner;
pub mod sandbox;
pub mod secrets;
pub mod spawn;
pub mod team;
pub mod watcher;

pub use error::Error;
pub use monitor::{ProgressMonitor, ProgressSummary, TimeoutConfig, TimeoutReason};
pub use permissions::{PermissionDetector, PermissionError, PermissionErrorType, PermissionFix};
pub use pr::{ConflictFile, ConflictStrategy, MergeStatus, PRManager, PullRequest};
pub use runner::{ClaudeRunner, GeminiRunner, LLMOutput, LLMResult, LLMRunner, LLMSpawnConfig};
pub use sandbox::{Sandbox, SandboxManifest, SandboxProvider};
pub use secrets::{SecretError, SecretRef, SecretSource, SecretsManager};
pub use spawn::{SpawnConfig, SpawnResult, SpawnStatus};
pub use team::{
    CoordinationMode, FixPromptBuilder, ReviewPromptBuilder, ReviewResult, ReviewSuggestion,
    ReviewVerdict, SpawnTeamConfig, SpawnTeamResult,
};
pub use watcher::{RecoveryStrategy, TerminationReason, WatcherAgent, WatcherConfig, WatcherResult};

pub use config::{
    validate_spawn_operation, validate_spawn_team_operation, Validate, ValidationResult,
    KNOWN_LLMS, KNOWN_TOOLS,
};
