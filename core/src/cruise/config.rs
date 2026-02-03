//! Configuration for cruise-control operations.

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// PR strategy for task completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PrStrategy {
    /// One PR per task.
    #[default]
    PerTask,
    /// Group related tasks into batched PRs.
    Batch,
    /// Single accumulating PR with commits per task.
    Single,
}

/// Test repository lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RepoLifecycle {
    /// Delete repository after test completion.
    #[default]
    Ephemeral,
    /// Keep repository but reset between runs.
    Persistent,
    /// Keep all artifacts, create new repos per run.
    Accumulating,
}

/// Test success criteria level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TestLevel {
    /// All phases complete.
    Basic,
    /// All phases complete AND app passes tests.
    #[default]
    Functional,
    /// All phases, app works, AND no critical audit findings.
    Strict,
}

/// Configuration for the planning phase.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanningConfig {
    /// Max ping-pong iterations.
    #[serde(default = "default_ping_pong_iterations")]
    pub ping_pong_iterations: u32,
    /// Reviewer LLM identifier.
    #[serde(default = "default_reviewer_llm")]
    pub reviewer_llm: String,
}

fn default_ping_pong_iterations() -> u32 {
    5
}

fn default_reviewer_llm() -> String {
    "gemini-cli".to_string()
}

impl Default for PlanningConfig {
    fn default() -> Self {
        Self {
            ping_pong_iterations: default_ping_pong_iterations(),
            reviewer_llm: default_reviewer_llm(),
        }
    }
}

/// Configuration for the build phase.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildingConfig {
    /// Maximum parallel spawn-team instances.
    #[serde(default = "default_max_parallel")]
    pub max_parallel: usize,
    /// PR creation strategy.
    #[serde(default)]
    pub pr_strategy: PrStrategy,
    /// Reviewer LLM for sequential mode.
    #[serde(default = "default_reviewer_llm")]
    pub sequential_reviewer: String,
}

fn default_max_parallel() -> usize {
    3
}

impl Default for BuildingConfig {
    fn default() -> Self {
        Self {
            max_parallel: default_max_parallel(),
            pr_strategy: PrStrategy::default(),
            sequential_reviewer: default_reviewer_llm(),
        }
    }
}

/// Configuration for the validation phase.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationConfig {
    /// Test success level.
    #[serde(default)]
    pub test_level: TestLevel,
    /// Curl timeout in seconds.
    #[serde(default = "default_curl_timeout")]
    pub curl_timeout: u64,
}

fn default_curl_timeout() -> u64 {
    30
}

impl Default for ValidationConfig {
    fn default() -> Self {
        Self {
            test_level: TestLevel::default(),
            curl_timeout: default_curl_timeout(),
        }
    }
}

/// Configuration for PR approval polling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalConfig {
    /// Initial poll interval.
    #[serde(default = "default_poll_initial")]
    pub poll_initial: Duration,
    /// Maximum poll interval.
    #[serde(default = "default_poll_max")]
    pub poll_max: Duration,
    /// Exponential backoff multiplier.
    #[serde(default = "default_poll_backoff")]
    pub poll_backoff: f64,
}

fn default_poll_initial() -> Duration {
    Duration::from_secs(60)
}

fn default_poll_max() -> Duration {
    Duration::from_secs(1800)
}

fn default_poll_backoff() -> f64 {
    2.0
}

impl Default for ApprovalConfig {
    fn default() -> Self {
        Self {
            poll_initial: default_poll_initial(),
            poll_max: default_poll_max(),
            poll_backoff: default_poll_backoff(),
        }
    }
}

/// Configuration for E2E testing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestConfig {
    /// Default GitHub organization.
    #[serde(default = "default_org")]
    pub default_org: String,
    /// Repository lifecycle.
    #[serde(default)]
    pub repo_lifecycle: RepoLifecycle,
}

/// Configuration for LLM timeouts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeoutsConfig {
    /// Idle timeout in seconds (no activity before termination).
    /// Default: 300s (5 minutes) - allows thinking time for complex tasks.
    #[serde(default = "default_idle_timeout")]
    pub idle_timeout_secs: u64,
    /// Total timeout in seconds (wall-clock time limit).
    /// Default: 3600s (1 hour) for full workflow tasks.
    #[serde(default = "default_total_timeout")]
    pub total_timeout_secs: u64,
    /// Planning-specific idle timeout (planning often requires more thinking).
    /// Default: 600s (10 minutes).
    #[serde(default = "default_planning_idle_timeout")]
    pub planning_idle_timeout_secs: u64,
}

fn default_idle_timeout() -> u64 {
    300 // 5 minutes - allows thinking time
}

fn default_total_timeout() -> u64 {
    3600 // 1 hour
}

fn default_planning_idle_timeout() -> u64 {
    600 // 10 minutes - planning requires more thinking
}

impl Default for TimeoutsConfig {
    fn default() -> Self {
        Self {
            idle_timeout_secs: default_idle_timeout(),
            total_timeout_secs: default_total_timeout(),
            planning_idle_timeout_secs: default_planning_idle_timeout(),
        }
    }
}

impl TimeoutsConfig {
    /// Returns the idle timeout as a Duration.
    pub fn idle_timeout(&self) -> Duration {
        Duration::from_secs(self.idle_timeout_secs)
    }

    /// Returns the total timeout as a Duration.
    pub fn total_timeout(&self) -> Duration {
        Duration::from_secs(self.total_timeout_secs)
    }

    /// Returns the planning idle timeout as a Duration.
    pub fn planning_idle_timeout(&self) -> Duration {
        Duration::from_secs(self.planning_idle_timeout_secs)
    }
}

fn default_org() -> String {
    "epiphytic".to_string()
}

impl Default for TestConfig {
    fn default() -> Self {
        Self {
            default_org: default_org(),
            repo_lifecycle: RepoLifecycle::default(),
        }
    }
}

/// Top-level cruise-control configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CruiseConfig {
    /// Planning phase configuration.
    #[serde(default)]
    pub planning: PlanningConfig,
    /// Building phase configuration.
    #[serde(default)]
    pub building: BuildingConfig,
    /// Validation phase configuration.
    #[serde(default)]
    pub validation: ValidationConfig,
    /// Approval polling configuration.
    #[serde(default)]
    pub approval: ApprovalConfig,
    /// E2E test configuration.
    #[serde(default)]
    pub test: TestConfig,
    /// LLM timeout configuration.
    #[serde(default)]
    pub timeouts: TimeoutsConfig,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cruise_config_has_sensible_defaults() {
        let config = CruiseConfig::default();

        assert_eq!(config.planning.ping_pong_iterations, 5);
        assert_eq!(config.planning.reviewer_llm, "gemini-cli");
        assert_eq!(config.building.max_parallel, 3);
        assert_eq!(config.building.pr_strategy, PrStrategy::PerTask);
        assert_eq!(config.validation.test_level, TestLevel::Functional);
        assert_eq!(config.approval.poll_initial, Duration::from_secs(60));
        assert_eq!(config.test.default_org, "epiphytic");
        // Timeout defaults - generous for LLM planning
        assert_eq!(config.timeouts.idle_timeout_secs, 300);
        assert_eq!(config.timeouts.total_timeout_secs, 3600);
        assert_eq!(config.timeouts.planning_idle_timeout_secs, 600);
    }

    #[test]
    fn pr_strategy_serializes_correctly() {
        assert_eq!(
            serde_json::to_string(&PrStrategy::PerTask).unwrap(),
            "\"per-task\""
        );
        assert_eq!(
            serde_json::to_string(&PrStrategy::Batch).unwrap(),
            "\"batch\""
        );
        assert_eq!(
            serde_json::to_string(&PrStrategy::Single).unwrap(),
            "\"single\""
        );
    }

    #[test]
    fn repo_lifecycle_serializes_correctly() {
        assert_eq!(
            serde_json::to_string(&RepoLifecycle::Ephemeral).unwrap(),
            "\"ephemeral\""
        );
        assert_eq!(
            serde_json::to_string(&RepoLifecycle::Persistent).unwrap(),
            "\"persistent\""
        );
    }

    #[test]
    fn test_level_serializes_correctly() {
        assert_eq!(
            serde_json::to_string(&TestLevel::Basic).unwrap(),
            "\"basic\""
        );
        assert_eq!(
            serde_json::to_string(&TestLevel::Functional).unwrap(),
            "\"functional\""
        );
        assert_eq!(
            serde_json::to_string(&TestLevel::Strict).unwrap(),
            "\"strict\""
        );
    }

    #[test]
    fn cruise_config_deserializes_from_toml() {
        let toml = r#"
            [planning]
            ping_pong_iterations = 3
            reviewer_llm = "claude-code"

            [building]
            max_parallel = 5
            pr_strategy = "batch"

            [validation]
            test_level = "strict"
        "#;

        let config: CruiseConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.planning.ping_pong_iterations, 3);
        assert_eq!(config.planning.reviewer_llm, "claude-code");
        assert_eq!(config.building.max_parallel, 5);
        assert_eq!(config.building.pr_strategy, PrStrategy::Batch);
        assert_eq!(config.validation.test_level, TestLevel::Strict);
    }
}
