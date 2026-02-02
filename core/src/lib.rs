//! Infinite Improbability Drive - Claude Code plugin for spawning sandboxed LLMs
//!
//! This library provides the core functionality for launching isolated LLM instances
//! in git worktree sandboxes with intelligent resource provisioning and lifecycle management.

pub mod beads;
pub mod config;
pub mod cruise;
pub mod e2e;
pub mod error;
pub mod monitor;
pub mod permissions;
pub mod pr;
pub mod prompt;
pub mod runner;
pub mod sandbox;
pub mod secrets;
pub mod spawn;
pub mod team;
pub mod team_orchestrator;
pub mod watcher;

pub use error::Error;
pub use monitor::{ProgressMonitor, ProgressSummary, TimeoutConfig, TimeoutReason};
pub use permissions::{PermissionDetector, PermissionError, PermissionErrorType, PermissionFix};
pub use pr::{ConflictFile, ConflictStrategy, MergeStatus, PRManager, PullRequest};
pub use runner::{ClaudeRunner, GeminiRunner, LLMOutput, LLMResult, LLMRunner, LLMSpawnConfig};
pub use sandbox::{Sandbox, SandboxManifest, SandboxProvider};
pub use secrets::{SecretError, SecretRef, SecretSource, SecretsManager};
pub use prompt::{augment_prompt_with_gitignore, has_gitignore, prompt_mentions_gitignore};
pub use spawn::{SpawnConfig, SpawnResult, SpawnStatus};
pub use team::{
    CoordinationMode, FixPromptBuilder, ReviewPromptBuilder, ReviewResult, ReviewSuggestion,
    ReviewVerdict, SpawnTeamConfig, SpawnTeamResult,
};
pub use watcher::{RecoveryStrategy, TerminationReason, WatcherAgent, WatcherConfig, WatcherResult};

pub use e2e::{E2EHarness, E2EResult, Fixture, RunnerType, ValidationLevel};

pub use config::{
    validate_spawn_operation, validate_spawn_team_operation, Validate, ValidationResult,
    KNOWN_LLMS, KNOWN_TOOLS,
};
pub use cruise::{
    generate_plan_markdown, generate_pr_body, parse_plan_json,
    validate_plan as validate_cruise_plan, AdherenceCheck, AdherenceStatus, ApprovalConfig,
    AuditFinding, BuildResult, BuildingConfig, CruiseConfig, CruisePlan, CruiseResult, CruiseTask,
    FindingSeverity, FunctionalTestResult, PlanPromptBuilder, PlanResult, Planner, PlanningConfig,
    PlanReviewPromptBuilder, PrStrategy, RepoLifecycle, ReviewPhase, TaskComplexity, TaskResult,
    TaskStatus, TestConfig, TestLevel, ValidationConfig as CruiseValidationConfig,
    ValidationResult as CruiseValidationResult,
};
pub use beads::{
    BeadsClient, BeadsIssue, CreateOptions as BeadsCreateOptions, CreateResult as BeadsCreateResult,
    DependencyType as BeadsDependencyType, IssueStatus as BeadsIssueStatus,
    IssueType as BeadsIssueType, Priority as BeadsPriority,
};
pub use team_orchestrator::{
    format_observability_markdown, CommandLineRecord, PermissionRecord, ReviewFeedbackRecord,
    SecurityFinding, SpawnObservability, SpawnTeamOrchestrator,
};
