//! Configuration validation for spawn operations.
//!
//! Validates configuration before spawning to catch errors early.

use std::time::Duration;

use crate::error::{Error, Result};
use crate::sandbox::SandboxManifest;
use crate::spawn::SpawnConfig;
use crate::team::SpawnTeamConfig;
use crate::watcher::WatcherConfig;

/// Known LLM runner identifiers.
pub const KNOWN_LLMS: &[&str] = &["claude-code", "gemini-cli"];

/// Known tool names that can be allowed/disabled.
pub const KNOWN_TOOLS: &[&str] = &[
    "Read",
    "Write",
    "Edit",
    "Bash",
    "Glob",
    "Grep",
    "LS",
    "Task",
    "WebFetch",
    "WebSearch",
    "NotebookEdit",
    "NotebookRead",
];

/// Validation result containing all found issues.
#[derive(Debug, Clone, Default)]
pub struct ValidationResult {
    /// List of validation errors (fatal).
    pub errors: Vec<String>,
    /// List of validation warnings (non-fatal).
    pub warnings: Vec<String>,
}

impl ValidationResult {
    /// Returns true if validation passed (no errors).
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }

    /// Adds an error to the result.
    pub fn add_error(&mut self, msg: impl Into<String>) {
        self.errors.push(msg.into());
    }

    /// Adds a warning to the result.
    pub fn add_warning(&mut self, msg: impl Into<String>) {
        self.warnings.push(msg.into());
    }

    /// Merges another validation result into this one.
    pub fn merge(&mut self, other: ValidationResult) {
        self.errors.extend(other.errors);
        self.warnings.extend(other.warnings);
    }

    /// Converts to a Result, failing if there are errors.
    pub fn into_result(self) -> Result<Vec<String>> {
        if self.is_valid() {
            Ok(self.warnings)
        } else {
            Err(Error::Config(self.errors.join("; ")))
        }
    }
}

/// Trait for validatable configuration types.
pub trait Validate {
    /// Validates the configuration and returns any issues found.
    fn validate(&self) -> ValidationResult;
}

impl Validate for SpawnConfig {
    fn validate(&self) -> ValidationResult {
        let mut result = ValidationResult::default();

        // Prompt must not be empty
        if self.prompt.trim().is_empty() {
            result.add_error("prompt cannot be empty");
        }

        // Idle timeout should be less than total timeout
        if self.idle_timeout >= self.total_timeout {
            result.add_error("idle_timeout must be less than total_timeout");
        }

        // Warn if timeouts are very short
        if self.idle_timeout < Duration::from_secs(10) {
            result.add_warning("idle_timeout less than 10 seconds may cause premature termination");
        }

        // Warn if total timeout is very long
        if self.total_timeout > Duration::from_secs(7200) {
            result.add_warning("total_timeout over 2 hours may indicate a misconfiguration");
        }

        result
    }
}

impl Validate for SandboxManifest {
    fn validate(&self) -> ValidationResult {
        let mut result = ValidationResult::default();

        // Check for unknown tools
        for tool in &self.allowed_tools {
            if !KNOWN_TOOLS.contains(&tool.as_str()) {
                result.add_warning(format!("unknown tool '{}' in allowed_tools", tool));
            }
        }

        // Warn about wildcard paths (security consideration)
        for path in &self.readable_paths {
            if path.contains("**") {
                result.add_warning(format!(
                    "readable_path '{}' uses recursive glob - consider being more specific",
                    path
                ));
            }
        }

        for path in &self.writable_paths {
            if path.contains("**") {
                result.add_warning(format!(
                    "write_path '{}' uses recursive glob - consider being more specific",
                    path
                ));
            }
            // Warn about writing to sensitive locations
            if path.starts_with("/etc") || path.starts_with("/usr") || path.contains(".ssh") {
                result.add_warning(format!(
                    "write_path '{}' includes sensitive system location",
                    path
                ));
            }
        }

        result
    }
}

impl Validate for WatcherConfig {
    fn validate(&self) -> ValidationResult {
        let mut result = ValidationResult::default();

        // Max escalations should be reasonable
        if self.max_escalations > 10 {
            result
                .add_warning("max_escalations > 10 may indicate insufficient initial permissions");
        }

        if self.max_escalations == 0 {
            result.add_warning(
                "max_escalations = 0 means no automatic permission fixes will be attempted",
            );
        }

        result
    }
}

impl Validate for SpawnTeamConfig {
    fn validate(&self) -> ValidationResult {
        let mut result = ValidationResult::default();

        // Check LLM identifiers
        if !KNOWN_LLMS.contains(&self.primary_llm.as_str()) {
            result.add_warning(format!("unknown primary_llm '{}'", self.primary_llm));
        }

        if !KNOWN_LLMS.contains(&self.reviewer_llm.as_str()) {
            result.add_warning(format!("unknown reviewer_llm '{}'", self.reviewer_llm));
        }

        // Max iterations must be at least 1
        if self.max_iterations == 0 {
            result.add_error("max_iterations must be at least 1");
        }

        // Warn if max iterations is very high
        if self.max_iterations > 10 {
            result.add_warning("max_iterations > 10 may lead to excessive LLM calls");
        }

        // Primary and reviewer should be different (usually)
        if self.primary_llm == self.reviewer_llm {
            result.add_warning(
                "primary_llm and reviewer_llm are the same - this may limit review value",
            );
        }

        result
    }
}

/// Validates all configuration for a spawn operation.
pub fn validate_spawn_operation(
    spawn_config: &SpawnConfig,
    manifest: &SandboxManifest,
) -> ValidationResult {
    let mut result = ValidationResult::default();
    result.merge(spawn_config.validate());
    result.merge(manifest.validate());
    result
}

/// Validates all configuration for a spawn-team operation.
pub fn validate_spawn_team_operation(
    spawn_config: &SpawnConfig,
    manifest: &SandboxManifest,
    team_config: &SpawnTeamConfig,
) -> ValidationResult {
    let mut result = validate_spawn_operation(spawn_config, manifest);
    result.merge(team_config.validate());
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::team::CoordinationMode;

    // ========================================
    // SpawnConfig validation tests
    // ========================================

    #[test]
    fn spawn_config_valid_passes() {
        let config = SpawnConfig::new("Fix the bug");
        let result = config.validate();
        assert!(result.is_valid());
    }

    #[test]
    fn spawn_config_empty_prompt_fails() {
        let config = SpawnConfig {
            prompt: "".to_string(),
            mode: Default::default(),
            idle_timeout: Duration::from_secs(120),
            total_timeout: Duration::from_secs(1800),
            max_permission_escalations: 1,
        };
        let result = config.validate();
        assert!(!result.is_valid());
        assert!(result.errors.iter().any(|e| e.contains("prompt")));
    }

    #[test]
    fn spawn_config_whitespace_prompt_fails() {
        let config = SpawnConfig {
            prompt: "   \n\t  ".to_string(),
            mode: Default::default(),
            idle_timeout: Duration::from_secs(120),
            total_timeout: Duration::from_secs(1800),
            max_permission_escalations: 1,
        };
        let result = config.validate();
        assert!(!result.is_valid());
        assert!(result.errors.iter().any(|e| e.contains("prompt")));
    }

    #[test]
    fn spawn_config_idle_ge_total_fails() {
        let config = SpawnConfig::new("test")
            .with_idle_timeout(Duration::from_secs(300))
            .with_total_timeout(Duration::from_secs(300));
        let result = config.validate();
        assert!(!result.is_valid());
        assert!(result.errors.iter().any(|e| e.contains("idle_timeout")));
    }

    #[test]
    fn spawn_config_idle_gt_total_fails() {
        let config = SpawnConfig::new("test")
            .with_idle_timeout(Duration::from_secs(600))
            .with_total_timeout(Duration::from_secs(300));
        let result = config.validate();
        assert!(!result.is_valid());
    }

    #[test]
    fn spawn_config_short_idle_warns() {
        let config = SpawnConfig::new("test").with_idle_timeout(Duration::from_secs(5));
        let result = config.validate();
        assert!(result.is_valid()); // Warning, not error
        assert!(result.warnings.iter().any(|w| w.contains("10 seconds")));
    }

    #[test]
    fn spawn_config_long_total_warns() {
        let config = SpawnConfig::new("test").with_total_timeout(Duration::from_secs(10000));
        let result = config.validate();
        assert!(result.is_valid()); // Warning, not error
        assert!(result.warnings.iter().any(|w| w.contains("2 hours")));
    }

    // ========================================
    // SandboxManifest validation tests
    // ========================================

    #[test]
    fn sandbox_manifest_default_valid() {
        let manifest = SandboxManifest::default();
        let result = manifest.validate();
        assert!(result.is_valid());
    }

    #[test]
    fn sandbox_manifest_known_tools_valid() {
        let manifest = SandboxManifest {
            allowed_tools: vec!["Read".to_string(), "Write".to_string(), "Bash".to_string()],
            ..Default::default()
        };
        let result = manifest.validate();
        assert!(result.is_valid());
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn sandbox_manifest_unknown_tool_warns() {
        let manifest = SandboxManifest {
            allowed_tools: vec!["UnknownTool".to_string()],
            ..Default::default()
        };
        let result = manifest.validate();
        assert!(result.is_valid()); // Warning, not error
        assert!(result.warnings.iter().any(|w| w.contains("UnknownTool")));
    }

    #[test]
    fn sandbox_manifest_recursive_glob_warns() {
        let manifest = SandboxManifest {
            readable_paths: vec!["src/**/*.rs".to_string()],
            writable_paths: vec!["**/*".to_string()],
            ..Default::default()
        };
        let result = manifest.validate();
        assert!(result.is_valid());
        assert!(result.warnings.len() >= 2);
    }

    #[test]
    fn sandbox_manifest_sensitive_write_path_warns() {
        let manifest = SandboxManifest {
            writable_paths: vec!["/etc/config".to_string()],
            ..Default::default()
        };
        let result = manifest.validate();
        assert!(result.warnings.iter().any(|w| w.contains("sensitive")));
    }

    #[test]
    fn sandbox_manifest_ssh_write_path_warns() {
        let manifest = SandboxManifest {
            writable_paths: vec!["~/.ssh/config".to_string()],
            ..Default::default()
        };
        let result = manifest.validate();
        assert!(result.warnings.iter().any(|w| w.contains(".ssh")));
    }

    // ========================================
    // WatcherConfig validation tests
    // ========================================

    #[test]
    fn watcher_config_default_valid() {
        let config = WatcherConfig::default();
        let result = config.validate();
        assert!(result.is_valid());
    }

    #[test]
    fn watcher_config_high_escalations_warns() {
        let config = WatcherConfig {
            max_escalations: 15,
            ..Default::default()
        };
        let result = config.validate();
        assert!(result.is_valid());
        assert!(result.warnings.iter().any(|w| w.contains("10")));
    }

    #[test]
    fn watcher_config_zero_escalations_warns() {
        let config = WatcherConfig {
            max_escalations: 0,
            ..Default::default()
        };
        let result = config.validate();
        assert!(result.is_valid());
        assert!(result.warnings.iter().any(|w| w.contains("0")));
    }

    // ========================================
    // SpawnTeamConfig validation tests
    // ========================================

    #[test]
    fn spawn_team_config_default_valid() {
        let config = SpawnTeamConfig::default();
        let result = config.validate();
        assert!(result.is_valid());
    }

    #[test]
    fn spawn_team_config_zero_iterations_fails() {
        let config = SpawnTeamConfig {
            mode: CoordinationMode::Sequential,
            max_iterations: 0,
            primary_llm: "claude-code".to_string(),
            primary_model: None,
            reviewer_llm: "gemini-cli".to_string(),
            reviewer_model: None,
            max_escalations: 5,
            max_concurrent_reviewers: 3,
        };
        let result = config.validate();
        assert!(!result.is_valid());
        assert!(result.errors.iter().any(|e| e.contains("max_iterations")));
    }

    #[test]
    fn spawn_team_config_high_iterations_warns() {
        let config = SpawnTeamConfig {
            mode: CoordinationMode::PingPong,
            max_iterations: 20,
            primary_llm: "claude-code".to_string(),
            primary_model: None,
            reviewer_llm: "gemini-cli".to_string(),
            reviewer_model: None,
            max_escalations: 5,
            max_concurrent_reviewers: 3,
        };
        let result = config.validate();
        assert!(result.is_valid());
        assert!(result.warnings.iter().any(|w| w.contains("10")));
    }

    #[test]
    fn spawn_team_config_unknown_primary_warns() {
        let config = SpawnTeamConfig {
            mode: CoordinationMode::Sequential,
            max_iterations: 3,
            primary_llm: "unknown-llm".to_string(),
            primary_model: None,
            reviewer_llm: "gemini-cli".to_string(),
            reviewer_model: None,
            max_escalations: 5,
            max_concurrent_reviewers: 3,
        };
        let result = config.validate();
        assert!(result.is_valid());
        assert!(result.warnings.iter().any(|w| w.contains("unknown-llm")));
    }

    #[test]
    fn spawn_team_config_unknown_reviewer_warns() {
        let config = SpawnTeamConfig {
            mode: CoordinationMode::Sequential,
            max_iterations: 3,
            primary_llm: "claude-code".to_string(),
            primary_model: None,
            reviewer_llm: "gpt-4".to_string(),
            reviewer_model: None,
            max_escalations: 5,
            max_concurrent_reviewers: 3,
        };
        let result = config.validate();
        assert!(result.is_valid());
        assert!(result.warnings.iter().any(|w| w.contains("gpt-4")));
    }

    #[test]
    fn spawn_team_config_same_llms_warns() {
        let config = SpawnTeamConfig {
            mode: CoordinationMode::Sequential,
            max_iterations: 3,
            primary_llm: "claude-code".to_string(),
            primary_model: None,
            reviewer_llm: "claude-code".to_string(),
            reviewer_model: None,
            max_escalations: 5,
            max_concurrent_reviewers: 3,
        };
        let result = config.validate();
        assert!(result.is_valid());
        assert!(result.warnings.iter().any(|w| w.contains("same")));
    }

    // ========================================
    // Combined validation tests
    // ========================================

    #[test]
    fn validate_spawn_operation_combines_results() {
        let config = SpawnConfig::new("test").with_idle_timeout(Duration::from_secs(5));
        let manifest = SandboxManifest {
            allowed_tools: vec!["FakeTool".to_string()],
            ..Default::default()
        };

        let result = validate_spawn_operation(&config, &manifest);
        assert!(result.is_valid());
        assert!(result.warnings.len() >= 2); // Both should have warnings
    }

    #[test]
    fn validate_spawn_team_operation_combines_all() {
        let config = SpawnConfig::new("test");
        let manifest = SandboxManifest::default();
        let team_config = SpawnTeamConfig {
            mode: CoordinationMode::Sequential,
            max_iterations: 3,
            primary_llm: "claude-code".to_string(),
            primary_model: None,
            reviewer_llm: "claude-code".to_string(), // Same - should warn
            reviewer_model: None,
            max_escalations: 5,
            max_concurrent_reviewers: 3,
        };

        let result = validate_spawn_team_operation(&config, &manifest, &team_config);
        assert!(result.is_valid());
        assert!(!result.warnings.is_empty());
    }

    #[test]
    fn validation_result_into_result_ok_on_valid() {
        let mut result = ValidationResult::default();
        result.add_warning("just a warning");
        let res = result.into_result();
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), vec!["just a warning"]);
    }

    #[test]
    fn validation_result_into_result_err_on_invalid() {
        let mut result = ValidationResult::default();
        result.add_error("fatal error");
        result.add_warning("warning");
        let res = result.into_result();
        assert!(res.is_err());
    }
}
