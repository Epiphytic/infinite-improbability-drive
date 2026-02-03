//! Sandbox provider trait and types.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::error::Result;

/// Pattern for matching paths (glob-style).
pub type PathPattern = String;

/// Pattern for matching commands.
pub type CommandPattern = String;

/// Reference to a secret that should be injected.
pub type SecretRef = String;

/// Estimated task complexity for timeout tuning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TaskComplexity {
    Low,
    #[default]
    Medium,
    High,
}

/// Manifest specifying sandbox permissions and resources.
///
/// This is produced by the watcher agent via LLM-assisted evaluation
/// and defines what the sandboxed LLM can access.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SandboxManifest {
    /// Paths the sandboxed LLM can read (relative to worktree root).
    pub readable_paths: Vec<PathPattern>,

    /// Paths the sandboxed LLM can write (relative to worktree root).
    pub writable_paths: Vec<PathPattern>,

    /// Tools the sandboxed LLM can use.
    pub allowed_tools: Vec<String>,

    /// Commands the LLM might need to run.
    pub allowed_commands: Vec<CommandPattern>,

    /// Environment variables to inject.
    pub environment: HashMap<String, String>,

    /// Secrets to inject (fetched from secure storage, never logged).
    pub secrets: Vec<SecretRef>,

    /// Estimated complexity for timeout tuning.
    pub complexity: TaskComplexity,
}

/// Represents an active sandbox environment.
pub trait Sandbox: Send + Sync {
    /// Returns the working directory path of the sandbox.
    fn path(&self) -> &PathBuf;

    /// Returns the manifest used to create this sandbox.
    fn manifest(&self) -> &SandboxManifest;

    /// Cleans up the sandbox, removing all resources.
    fn cleanup(&mut self) -> Result<()>;
}

/// Provider for creating sandboxed environments.
pub trait SandboxProvider: Send + Sync {
    /// The type of sandbox this provider creates.
    type Sandbox: Sandbox;

    /// Creates a new sandbox with the given manifest.
    /// Uses auto-generated branch name.
    fn create(&self, manifest: SandboxManifest) -> Result<Self::Sandbox>;

    /// Creates a new sandbox with an explicit branch name.
    /// This allows CruiseRunner to control branch naming per workflow phase.
    fn create_with_branch(
        &self,
        manifest: SandboxManifest,
        branch_name: &str,
    ) -> Result<Self::Sandbox>;

    /// Returns the path to the repository root.
    ///
    /// This is used for checking repo state before launching LLMs
    /// (e.g., checking for .gitignore).
    fn repo_path(&self) -> &PathBuf;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_manifest_has_sensible_defaults() {
        let manifest = SandboxManifest::default();

        assert!(manifest.readable_paths.is_empty());
        assert!(manifest.writable_paths.is_empty());
        assert!(manifest.allowed_tools.is_empty());
        assert!(manifest.allowed_commands.is_empty());
        assert!(manifest.environment.is_empty());
        assert!(manifest.secrets.is_empty());
        assert_eq!(manifest.complexity, TaskComplexity::Medium);
    }

    #[test]
    fn sandbox_manifest_can_be_built_with_paths() {
        let manifest = SandboxManifest {
            readable_paths: vec!["src/**".to_string(), "tests/**".to_string()],
            writable_paths: vec!["src/auth/**".to_string()],
            allowed_tools: vec!["Read".to_string(), "Write".to_string()],
            allowed_commands: vec!["cargo test".to_string()],
            environment: HashMap::from([("RUST_BACKTRACE".to_string(), "1".to_string())]),
            secrets: vec!["API_KEY".to_string()],
            complexity: TaskComplexity::High,
        };

        assert_eq!(manifest.readable_paths.len(), 2);
        assert_eq!(manifest.writable_paths.len(), 1);
        assert_eq!(manifest.allowed_tools.len(), 2);
        assert_eq!(manifest.complexity, TaskComplexity::High);
    }

    #[test]
    fn task_complexity_serializes_to_lowercase() {
        let low = serde_json::to_string(&TaskComplexity::Low).unwrap();
        let medium = serde_json::to_string(&TaskComplexity::Medium).unwrap();
        let high = serde_json::to_string(&TaskComplexity::High).unwrap();

        assert_eq!(low, "\"low\"");
        assert_eq!(medium, "\"medium\"");
        assert_eq!(high, "\"high\"");
    }
}
