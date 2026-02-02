//! Prompt utilities for spawn operations.
//!
//! This module provides utilities for checking and augmenting prompts
//! before they are sent to LLMs.

use std::path::Path;

/// The gitignore instruction to append when needed.
pub const GITIGNORE_INSTRUCTION: &str = "\n\nBefore committing any code, set up a full .gitignore file that will exclude any keys, credentials, temporary files, build artifacts and dependencies that are pulled in during builds, as well as .fork-join directories, editor/ide files, OS files for mac/windows/linux, .env files, and log files for this project.";

/// Checks if a .gitignore file exists at the given path.
pub fn has_gitignore(repo_path: &Path) -> bool {
    repo_path.join(".gitignore").exists()
}

/// Checks if a prompt mentions gitignore (case-insensitive).
pub fn prompt_mentions_gitignore(prompt: &str) -> bool {
    prompt.to_lowercase().contains("gitignore")
}

/// Augments a prompt with gitignore instructions if needed.
///
/// This checks:
/// 1. If a .gitignore file exists at the repo root
/// 2. If the prompt already mentions gitignore
///
/// If neither condition is met, the gitignore instruction is appended.
pub fn augment_prompt_with_gitignore(prompt: &str, repo_path: &Path) -> String {
    if has_gitignore(repo_path) {
        tracing::debug!(repo_path = ?repo_path, "repo has .gitignore, not augmenting prompt");
        return prompt.to_string();
    }

    if prompt_mentions_gitignore(prompt) {
        tracing::debug!("prompt mentions gitignore, not augmenting");
        return prompt.to_string();
    }

    tracing::info!(
        repo_path = ?repo_path,
        "no .gitignore found and prompt doesn't mention it, augmenting prompt"
    );

    format!("{}{}", prompt, GITIGNORE_INSTRUCTION)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn has_gitignore_returns_true_when_exists() {
        let temp_dir = TempDir::new().unwrap();
        std::fs::write(temp_dir.path().join(".gitignore"), "node_modules/\n").unwrap();

        assert!(has_gitignore(temp_dir.path()));
    }

    #[test]
    fn has_gitignore_returns_false_when_missing() {
        let temp_dir = TempDir::new().unwrap();

        assert!(!has_gitignore(temp_dir.path()));
    }

    #[test]
    fn prompt_mentions_gitignore_detects_lowercase() {
        assert!(prompt_mentions_gitignore("Create a gitignore file"));
        assert!(prompt_mentions_gitignore("Set up .gitignore"));
    }

    #[test]
    fn prompt_mentions_gitignore_detects_uppercase() {
        assert!(prompt_mentions_gitignore("Create a GITIGNORE file"));
        assert!(prompt_mentions_gitignore("Set up .GITIGNORE"));
    }

    #[test]
    fn prompt_mentions_gitignore_detects_mixed_case() {
        assert!(prompt_mentions_gitignore("Create a GitIgnore file"));
        assert!(prompt_mentions_gitignore("Set up .Gitignore"));
    }

    #[test]
    fn prompt_mentions_gitignore_returns_false_when_not_mentioned() {
        assert!(!prompt_mentions_gitignore("Build a web app"));
        assert!(!prompt_mentions_gitignore("Create a Rust project"));
    }

    #[test]
    fn augment_prompt_does_not_change_when_gitignore_exists() {
        let temp_dir = TempDir::new().unwrap();
        std::fs::write(temp_dir.path().join(".gitignore"), "*.log\n").unwrap();

        let prompt = "Build a web app";
        let result = augment_prompt_with_gitignore(prompt, temp_dir.path());

        assert_eq!(result, prompt);
    }

    #[test]
    fn augment_prompt_does_not_change_when_mentioned() {
        let temp_dir = TempDir::new().unwrap();
        // No .gitignore file

        let prompt = "Build a web app and set up a proper gitignore";
        let result = augment_prompt_with_gitignore(prompt, temp_dir.path());

        assert_eq!(result, prompt);
    }

    #[test]
    fn augment_prompt_appends_instruction_when_needed() {
        let temp_dir = TempDir::new().unwrap();
        // No .gitignore file

        let prompt = "Build a web app";
        let result = augment_prompt_with_gitignore(prompt, temp_dir.path());

        assert!(result.starts_with(prompt));
        assert!(result.contains("Before committing any code"));
        assert!(result.contains(".gitignore"));
        assert!(result.contains(".env files"));
        assert!(result.contains(".fork-join directories"));
    }

    #[test]
    fn gitignore_instruction_has_expected_content() {
        assert!(GITIGNORE_INSTRUCTION.contains("keys"));
        assert!(GITIGNORE_INSTRUCTION.contains("credentials"));
        assert!(GITIGNORE_INSTRUCTION.contains("build artifacts"));
        assert!(GITIGNORE_INSTRUCTION.contains(".fork-join"));
        assert!(GITIGNORE_INSTRUCTION.contains("editor/ide"));
        assert!(GITIGNORE_INSTRUCTION.contains("mac/windows/linux"));
        assert!(GITIGNORE_INSTRUCTION.contains(".env"));
        assert!(GITIGNORE_INSTRUCTION.contains("log files"));
    }
}
