//! Error types for the infinite-improbability-drive plugin.

use std::path::PathBuf;
use thiserror::Error;

/// Top-level error type for spawn operations.
#[derive(Error, Debug)]
pub enum Error {
    /// Failed to create a sandbox.
    #[error("failed to create sandbox: {0}")]
    SandboxCreation(String),

    /// Failed to clean up a sandbox.
    #[error("failed to clean up sandbox at {path}: {reason}")]
    SandboxCleanup { path: PathBuf, reason: String },

    /// Git operation failed.
    #[error("git operation failed: {0}")]
    Git(String),

    /// IO error during sandbox operations.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// The sandbox path is not valid.
    #[error("invalid sandbox path: {0}")]
    InvalidPath(PathBuf),

    /// Spawn configuration error.
    #[error("configuration error: {0}")]
    Config(String),

    /// Cruise-control operation failed.
    #[error("cruise-control error: {0}")]
    Cruise(String),

    /// GitHub API operation failed.
    #[error("GitHub operation failed: {0}")]
    GitHub(String),

    /// Plan approval timeout.
    #[error("plan approval timed out after {0} seconds")]
    ApprovalTimeout(u64),

    /// Dependency cycle detected in plan.
    #[error("dependency cycle detected: {0}")]
    DependencyCycle(String),
}

/// Result type alias for spawn operations.
pub type Result<T> = std::result::Result<T, Error>;
