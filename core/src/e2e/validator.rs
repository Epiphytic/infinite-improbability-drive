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
            .map_err(Error::Io)?;

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
