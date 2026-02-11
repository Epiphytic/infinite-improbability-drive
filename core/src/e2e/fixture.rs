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

/// Workflow type for the test.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowType {
    /// Simple: run prompt directly.
    #[default]
    Simple,
    /// Full: plan -> approve -> execute.
    Full,
}

/// Team coordination mode for spawn-team operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TeamMode {
    /// PingPong mode: iterative back-and-forth between primary and reviewer.
    #[default]
    PingPong,
    /// GitHub mode: PR-based coordination with GitHub reviews.
    GitHub,
}

/// Which phase(s) of the workflow to execute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionPhase {
    /// Run all phases (plan -> build -> validate).
    #[default]
    All,
    /// Run only the planning phase.
    PlanOnly,
    /// Run only the build phase (requires existing plan PR).
    BuildOnly,
    /// Run only the validate phase (requires existing implementation).
    ValidateOnly,
    /// Run plan + build (skip validation).
    PlanAndBuild,
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

    /// Workflow type (simple or full).
    #[serde(default)]
    pub workflow: WorkflowType,

    /// The prompt to send to the LLM (for simple workflow, this is the only prompt;
    /// for full workflow, this is the execution prompt after plan approval).
    pub prompt: String,

    /// Planning prompt for full workflow (optional).
    /// If not provided in full workflow, a planning prompt is auto-generated from `prompt`.
    #[serde(default)]
    pub planning_prompt: Option<String>,

    /// Validation configuration.
    #[serde(default)]
    pub validation: ValidationConfig,

    /// Timeout in seconds.
    #[serde(default = "default_timeout")]
    pub timeout: u64,

    /// Team coordination mode (for spawn-team operations).
    #[serde(default)]
    pub team_mode: TeamMode,

    /// Which phase(s) to execute.
    #[serde(default)]
    pub phase: ExecutionPhase,

    /// Existing plan PR URL (for build_only or validate_only phases).
    /// If not provided, a fresh plan will be generated.
    #[serde(default)]
    pub existing_plan_pr: Option<String>,

    /// Existing implementation PR URL (for validate_only phase).
    #[serde(default)]
    pub existing_impl_pr: Option<String>,

    /// Environment variables to set when running LLMs.
    #[serde(default)]
    pub env: HashMap<String, String>,
}

fn default_timeout() -> u64 {
    300 // 5 minutes
}

impl Fixture {
    /// Loads a fixture from a YAML file.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let content = std::fs::read_to_string(path.as_ref()).map_err(Error::Io)?;

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
