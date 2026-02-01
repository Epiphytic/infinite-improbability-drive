//! Permission error detection and recovery.
//!
//! Pattern-matches common permission errors and computes appropriate fixes
//! for the recovery system.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Type of permission error detected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PermissionErrorType {
    /// File read was denied.
    FileReadDenied(PathBuf),
    /// File write was denied.
    FileWriteDenied(PathBuf),
    /// Command was blocked.
    CommandBlocked(String),
    /// Tool is not enabled.
    ToolDisabled(String),
    /// Required environment variable is not set.
    EnvVarMissing(String),
    /// Required secret is not available.
    SecretMissing(String),
    /// Network access was denied.
    NetworkBlocked(String),
}

/// Computed fix for a permission error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PermissionFix {
    /// Add a readable path pattern.
    AddReadPath(String),
    /// Add a writable path pattern.
    AddWritePath(String),
    /// Allow a command.
    AllowCommand(String),
    /// Enable a tool.
    EnableTool(String),
    /// Inject an environment variable.
    InjectEnvVar(String),
    /// Inject a secret.
    InjectSecret(String),
    /// Cannot fix automatically - requires respawn or fail.
    CannotFix(String),
}

/// A detected permission error with its computed fix.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionError {
    /// The type of error.
    pub error_type: PermissionErrorType,
    /// The computed fix, if any.
    pub fix: PermissionFix,
    /// The original error message.
    pub original_message: String,
}

/// Detects and classifies permission errors from output.
pub struct PermissionDetector {
    /// Known patterns for file read denials.
    file_read_patterns: Vec<&'static str>,
    /// Known patterns for file write denials.
    file_write_patterns: Vec<&'static str>,
    /// Known patterns for command blocks.
    command_patterns: Vec<&'static str>,
    /// Known patterns for tool disables.
    tool_patterns: Vec<&'static str>,
    /// Known patterns for missing env vars.
    env_var_patterns: Vec<&'static str>,
    /// Known patterns for missing secrets.
    secret_patterns: Vec<&'static str>,
    /// Known patterns for network blocks.
    network_patterns: Vec<&'static str>,
}

impl Default for PermissionDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl PermissionDetector {
    /// Creates a new permission detector with default patterns.
    pub fn new() -> Self {
        Self {
            file_read_patterns: vec![
                "Permission denied:",
                "cannot read",
                "EACCES",
                "read access denied",
                "cannot open",
                "No such file or directory",
            ],
            file_write_patterns: vec![
                "Cannot write to:",
                "cannot write",
                "write access denied",
                "Read-only file system",
                "EROFS",
            ],
            command_patterns: vec![
                "Command not allowed:",
                "command not found",
                "Permission denied:",
                "not permitted",
            ],
            tool_patterns: vec![
                "Tool '",
                "is not enabled",
                "tool not available",
                "disabled tool",
            ],
            env_var_patterns: vec![
                "Environment variable",
                "not set",
                "undefined variable",
                "missing env",
            ],
            secret_patterns: vec![
                "API key required",
                "secret not provided",
                "authentication required",
                "missing credential",
                "token required",
            ],
            network_patterns: vec![
                "Network access denied",
                "connection refused",
                "ENETUNREACH",
                "network unreachable",
                "blocked by policy",
            ],
        }
    }

    /// Analyzes a line of output for permission errors.
    ///
    /// Returns `Some(PermissionError)` if a permission error is detected.
    pub fn analyze(&self, line: &str) -> Option<PermissionError> {
        // Check for file read denials
        if self.matches_any(line, &self.file_read_patterns) {
            if let Some(path) = self.extract_path(line) {
                let pattern = self.path_to_pattern(&path);
                return Some(PermissionError {
                    error_type: PermissionErrorType::FileReadDenied(path),
                    fix: PermissionFix::AddReadPath(pattern),
                    original_message: line.to_string(),
                });
            }
        }

        // Check for file write denials
        if self.matches_any(line, &self.file_write_patterns) {
            if let Some(path) = self.extract_path(line) {
                let pattern = self.path_to_pattern(&path);
                return Some(PermissionError {
                    error_type: PermissionErrorType::FileWriteDenied(path),
                    fix: PermissionFix::AddWritePath(pattern),
                    original_message: line.to_string(),
                });
            }
        }

        // Check for command blocks
        if self.matches_any(line, &self.command_patterns) {
            if let Some(cmd) = self.extract_command(line) {
                return Some(PermissionError {
                    error_type: PermissionErrorType::CommandBlocked(cmd.clone()),
                    fix: PermissionFix::AllowCommand(cmd),
                    original_message: line.to_string(),
                });
            }
        }

        // Check for tool disables
        if self.matches_any(line, &self.tool_patterns) {
            if let Some(tool) = self.extract_tool(line) {
                return Some(PermissionError {
                    error_type: PermissionErrorType::ToolDisabled(tool.clone()),
                    fix: PermissionFix::EnableTool(tool),
                    original_message: line.to_string(),
                });
            }
        }

        // Check for missing env vars
        if self.matches_any(line, &self.env_var_patterns) {
            if let Some(var) = self.extract_env_var(line) {
                return Some(PermissionError {
                    error_type: PermissionErrorType::EnvVarMissing(var.clone()),
                    fix: PermissionFix::InjectEnvVar(var),
                    original_message: line.to_string(),
                });
            }
        }

        // Check for missing secrets
        if self.matches_any(line, &self.secret_patterns) {
            if let Some(secret) = self.extract_secret(line) {
                return Some(PermissionError {
                    error_type: PermissionErrorType::SecretMissing(secret.clone()),
                    fix: PermissionFix::InjectSecret(secret),
                    original_message: line.to_string(),
                });
            }
        }

        // Check for network blocks
        if self.matches_any(line, &self.network_patterns) {
            if let Some(host) = self.extract_host(line) {
                return Some(PermissionError {
                    error_type: PermissionErrorType::NetworkBlocked(host.clone()),
                    fix: PermissionFix::CannotFix(format!(
                        "Network access to {} requires manual approval",
                        host
                    )),
                    original_message: line.to_string(),
                });
            }
        }

        None
    }

    /// Checks if the line matches any of the patterns.
    fn matches_any(&self, line: &str, patterns: &[&str]) -> bool {
        let lower = line.to_lowercase();
        patterns.iter().any(|p| lower.contains(&p.to_lowercase()))
    }

    /// Extracts a file path from an error message.
    fn extract_path(&self, line: &str) -> Option<PathBuf> {
        // Look for paths in common formats
        // "Permission denied: /path/to/file"
        // "cannot read '/path/to/file'"
        // "EACCES: /path/to/file"

        // Try colon-separated format
        if let Some(idx) = line.find(':') {
            let after_colon = line[idx + 1..].trim();
            if after_colon.starts_with('/') || after_colon.starts_with("./") {
                let path = after_colon.split_whitespace().next()?;
                return Some(PathBuf::from(path.trim_matches(|c| c == '\'' || c == '"')));
            }
        }

        // Try quoted path format
        if let Some(start) = line.find('\'') {
            if let Some(end) = line[start + 1..].find('\'') {
                let path = &line[start + 1..start + 1 + end];
                if path.contains('/') || path.contains('\\') {
                    return Some(PathBuf::from(path));
                }
            }
        }

        // Try double-quoted path format
        if let Some(start) = line.find('"') {
            if let Some(end) = line[start + 1..].find('"') {
                let path = &line[start + 1..start + 1 + end];
                if path.contains('/') || path.contains('\\') {
                    return Some(PathBuf::from(path));
                }
            }
        }

        None
    }

    /// Converts a path to a glob pattern for the parent directory.
    fn path_to_pattern(&self, path: &PathBuf) -> String {
        if let Some(parent) = path.parent() {
            format!("{}/**", parent.display())
        } else {
            "**".to_string()
        }
    }

    /// Extracts a command from an error message.
    fn extract_command(&self, line: &str) -> Option<String> {
        // "Command not allowed: npm install"
        // "npm: command not found"
        if line.contains("Command not allowed:") {
            let parts: Vec<&str> = line.split("Command not allowed:").collect();
            if parts.len() > 1 {
                return Some(parts[1].trim().to_string());
            }
        }

        // Check for "command not found" pattern
        if line.contains("command not found") {
            let parts: Vec<&str> = line.split(':').collect();
            if !parts.is_empty() {
                return Some(parts[0].trim().to_string());
            }
        }

        None
    }

    /// Extracts a tool name from an error message.
    fn extract_tool(&self, line: &str) -> Option<String> {
        // "Tool 'Bash' is not enabled"
        if line.contains("Tool '") {
            if let Some(start) = line.find("Tool '") {
                let after = &line[start + 6..];
                if let Some(end) = after.find('\'') {
                    return Some(after[..end].to_string());
                }
            }
        }

        None
    }

    /// Extracts an environment variable name from an error message.
    fn extract_env_var(&self, line: &str) -> Option<String> {
        // "Environment variable NODE_ENV not set"
        // "undefined variable: DATABASE_URL"
        let words: Vec<&str> = line.split_whitespace().collect();
        for (i, word) in words.iter().enumerate() {
            // Look for uppercase words that look like env vars
            if word.chars().all(|c| c.is_ascii_uppercase() || c == '_')
                && word.len() > 2
                && word.contains('_')
            {
                return Some(word.to_string());
            }
            // Check if followed by "not set"
            if *word == "variable" && i + 1 < words.len() {
                let var = words[i + 1].trim_matches(|c: char| !c.is_alphanumeric() && c != '_');
                if !var.is_empty() {
                    return Some(var.to_string());
                }
            }
        }

        None
    }

    /// Extracts a secret reference from an error message.
    fn extract_secret(&self, line: &str) -> Option<String> {
        // "API key required" -> "API_KEY"
        // "missing credential: github_token"
        if line.to_lowercase().contains("api key") {
            return Some("API_KEY".to_string());
        }
        if line.to_lowercase().contains("token required") {
            return Some("AUTH_TOKEN".to_string());
        }

        // Look for explicit secret names
        let words: Vec<&str> = line.split_whitespace().collect();
        for word in &words {
            let clean = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '_');
            if clean.to_lowercase().contains("token")
                || clean.to_lowercase().contains("key")
                || clean.to_lowercase().contains("secret")
            {
                return Some(clean.to_uppercase().replace("-", "_"));
            }
        }

        None
    }

    /// Extracts a host/URL from an error message.
    fn extract_host(&self, line: &str) -> Option<String> {
        // Look for URLs or hostnames
        let words: Vec<&str> = line.split_whitespace().collect();
        for word in words {
            if word.contains("://") {
                return Some(word.to_string());
            }
            if word.contains('.') && !word.ends_with('.') && !word.starts_with('.') {
                // Might be a hostname
                let clean =
                    word.trim_matches(|c: char| !c.is_alphanumeric() && c != '.' && c != '-');
                if clean.contains('.') {
                    return Some(clean.to_string());
                }
            }
        }

        Some("unknown host".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detector_detects_file_read_denial() {
        let detector = PermissionDetector::new();

        let result = detector.analyze("Permission denied: /etc/passwd");
        assert!(result.is_some());

        let error = result.unwrap();
        assert!(matches!(
            error.error_type,
            PermissionErrorType::FileReadDenied(_)
        ));
        assert!(matches!(error.fix, PermissionFix::AddReadPath(_)));
    }

    #[test]
    fn detector_detects_file_write_denial() {
        let detector = PermissionDetector::new();

        let result = detector.analyze("Cannot write to: /var/log/app.log");
        assert!(result.is_some());

        let error = result.unwrap();
        assert!(matches!(
            error.error_type,
            PermissionErrorType::FileWriteDenied(_)
        ));
        assert!(matches!(error.fix, PermissionFix::AddWritePath(_)));
    }

    #[test]
    fn detector_detects_command_blocked() {
        let detector = PermissionDetector::new();

        let result = detector.analyze("Command not allowed: npm install");
        assert!(result.is_some());

        let error = result.unwrap();
        assert!(matches!(
            error.error_type,
            PermissionErrorType::CommandBlocked(_)
        ));
        if let PermissionFix::AllowCommand(cmd) = &error.fix {
            assert_eq!(cmd, "npm install");
        } else {
            panic!("Expected AllowCommand fix");
        }
    }

    #[test]
    fn detector_detects_tool_disabled() {
        let detector = PermissionDetector::new();

        let result = detector.analyze("Tool 'Bash' is not enabled");
        assert!(result.is_some());

        let error = result.unwrap();
        assert!(matches!(
            error.error_type,
            PermissionErrorType::ToolDisabled(_)
        ));
        if let PermissionFix::EnableTool(tool) = &error.fix {
            assert_eq!(tool, "Bash");
        } else {
            panic!("Expected EnableTool fix");
        }
    }

    #[test]
    fn detector_detects_env_var_missing() {
        let detector = PermissionDetector::new();

        let result = detector.analyze("Environment variable DATABASE_URL not set");
        assert!(result.is_some());

        let error = result.unwrap();
        assert!(matches!(
            error.error_type,
            PermissionErrorType::EnvVarMissing(_)
        ));
        if let PermissionFix::InjectEnvVar(var) = &error.fix {
            assert_eq!(var, "DATABASE_URL");
        } else {
            panic!("Expected InjectEnvVar fix");
        }
    }

    #[test]
    fn detector_detects_secret_missing() {
        let detector = PermissionDetector::new();

        let result = detector.analyze("API key required but not provided");
        assert!(result.is_some());

        let error = result.unwrap();
        assert!(matches!(
            error.error_type,
            PermissionErrorType::SecretMissing(_)
        ));
        assert!(matches!(error.fix, PermissionFix::InjectSecret(_)));
    }

    #[test]
    fn detector_detects_network_blocked() {
        let detector = PermissionDetector::new();

        let result = detector.analyze("Network access denied to api.example.com");
        assert!(result.is_some());

        let error = result.unwrap();
        assert!(matches!(
            error.error_type,
            PermissionErrorType::NetworkBlocked(_)
        ));
        // Network blocks cannot be auto-fixed
        assert!(matches!(error.fix, PermissionFix::CannotFix(_)));
    }

    #[test]
    fn detector_returns_none_for_normal_output() {
        let detector = PermissionDetector::new();

        assert!(detector.analyze("Compiling my_crate v0.1.0").is_none());
        assert!(detector.analyze("Running tests...").is_none());
        assert!(detector.analyze("All tests passed!").is_none());
    }

    #[test]
    fn detector_extracts_path_from_various_formats() {
        let detector = PermissionDetector::new();

        // Colon format
        let r1 = detector.analyze("Permission denied: /home/user/file.txt");
        assert!(r1.is_some());
        if let PermissionErrorType::FileReadDenied(p) = &r1.unwrap().error_type {
            assert_eq!(p, &PathBuf::from("/home/user/file.txt"));
        }

        // Quoted format
        let r2 = detector.analyze("cannot read '/var/data/config.json'");
        assert!(r2.is_some());
        if let PermissionErrorType::FileReadDenied(p) = &r2.unwrap().error_type {
            assert_eq!(p, &PathBuf::from("/var/data/config.json"));
        }
    }

    #[test]
    fn path_to_pattern_creates_glob() {
        let detector = PermissionDetector::new();

        let pattern = detector.path_to_pattern(&PathBuf::from("/home/user/project/src/main.rs"));
        assert_eq!(pattern, "/home/user/project/src/**");
    }
}
