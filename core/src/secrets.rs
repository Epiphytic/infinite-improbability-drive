//! Secret management for spawned LLM instances.
//!
//! Handles secure injection of secrets as environment variables
//! and redaction of secret values from logs.

use std::collections::HashMap;
use std::env;

use serde::{Deserialize, Serialize};

/// A reference to a secret that should be injected.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SecretRef {
    /// Name of the secret (used as env var name).
    pub name: String,
    /// Source of the secret value.
    pub source: SecretSource,
}

/// Source from which to retrieve a secret value.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SecretSource {
    /// Read from an environment variable.
    EnvVar(String),
    /// Read from a file.
    File(String),
    /// Provided directly (for testing only).
    Direct(String),
}

/// Manages secrets for a spawn operation.
pub struct SecretsManager {
    /// Resolved secrets (name -> value).
    secrets: HashMap<String, String>,
    /// Redaction patterns (sorted by length descending for proper replacement).
    redaction_patterns: Vec<String>,
}

impl SecretsManager {
    /// Creates a new secrets manager.
    pub fn new() -> Self {
        Self {
            secrets: HashMap::new(),
            redaction_patterns: Vec::new(),
        }
    }

    /// Resolves and loads a secret from its source.
    ///
    /// Returns an error if the secret cannot be resolved.
    pub fn load_secret(&mut self, secret_ref: &SecretRef) -> Result<(), SecretError> {
        let value = match &secret_ref.source {
            SecretSource::EnvVar(var_name) => env::var(var_name).map_err(|_| {
                SecretError::NotFound(format!("environment variable '{}' not set", var_name))
            })?,
            SecretSource::File(path) => std::fs::read_to_string(path)
                .map_err(|e| SecretError::NotFound(format!("cannot read file '{}': {}", path, e)))?
                .trim()
                .to_string(),
            SecretSource::Direct(value) => value.clone(),
        };

        // Store the secret
        self.secrets.insert(secret_ref.name.clone(), value.clone());

        // Add to redaction patterns if not already present
        if !value.is_empty() && !self.redaction_patterns.contains(&value) {
            self.redaction_patterns.push(value);
            // Sort by length descending so longer patterns are replaced first
            self.redaction_patterns
                .sort_by(|a, b| b.len().cmp(&a.len()));
        }

        Ok(())
    }

    /// Returns the environment variables to inject into the spawned process.
    pub fn environment(&self) -> &HashMap<String, String> {
        &self.secrets
    }

    /// Redacts all known secret values from a string.
    ///
    /// Secret values are replaced with `[REDACTED:<name>]`.
    pub fn redact(&self, text: &str) -> String {
        let mut result = text.to_string();

        for (name, value) in &self.secrets {
            if !value.is_empty() {
                result = result.replace(value, &format!("[REDACTED:{}]", name));
            }
        }

        result
    }

    /// Redacts secrets from multiple lines.
    pub fn redact_lines(&self, lines: &[String]) -> Vec<String> {
        lines.iter().map(|line| self.redact(line)).collect()
    }

    /// Returns true if any secrets are loaded.
    pub fn has_secrets(&self) -> bool {
        !self.secrets.is_empty()
    }

    /// Returns the number of loaded secrets.
    pub fn secret_count(&self) -> usize {
        self.secrets.len()
    }

    /// Clears all loaded secrets.
    pub fn clear(&mut self) {
        self.secrets.clear();
        self.redaction_patterns.clear();
    }
}

impl Default for SecretsManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Error type for secret operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretError {
    /// Secret source could not be found or read.
    NotFound(String),
}

impl std::fmt::Display for SecretError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SecretError::NotFound(msg) => write!(f, "secret not found: {}", msg),
        }
    }
}

impl std::error::Error for SecretError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_manager_starts_empty() {
        let manager = SecretsManager::new();

        assert!(!manager.has_secrets());
        assert_eq!(manager.secret_count(), 0);
        assert!(manager.environment().is_empty());
    }

    #[test]
    fn secrets_manager_loads_direct_secret() {
        let mut manager = SecretsManager::new();

        let secret = SecretRef {
            name: "API_KEY".to_string(),
            source: SecretSource::Direct("super-secret-value".to_string()),
        };

        manager.load_secret(&secret).expect("should load secret");

        assert!(manager.has_secrets());
        assert_eq!(manager.secret_count(), 1);
        assert_eq!(
            manager.environment().get("API_KEY"),
            Some(&"super-secret-value".to_string())
        );
    }

    #[test]
    fn secrets_manager_loads_env_var_secret() {
        let mut manager = SecretsManager::new();

        // Set up test env var
        env::set_var("TEST_SECRET_VAR", "env-secret-value");

        let secret = SecretRef {
            name: "MY_SECRET".to_string(),
            source: SecretSource::EnvVar("TEST_SECRET_VAR".to_string()),
        };

        manager.load_secret(&secret).expect("should load secret");

        assert_eq!(
            manager.environment().get("MY_SECRET"),
            Some(&"env-secret-value".to_string())
        );

        // Clean up
        env::remove_var("TEST_SECRET_VAR");
    }

    #[test]
    fn secrets_manager_fails_on_missing_env_var() {
        let mut manager = SecretsManager::new();

        let secret = SecretRef {
            name: "MISSING".to_string(),
            source: SecretSource::EnvVar("DEFINITELY_NOT_SET_12345".to_string()),
        };

        let result = manager.load_secret(&secret);
        assert!(result.is_err());
    }

    #[test]
    fn secrets_manager_redacts_single_secret() {
        let mut manager = SecretsManager::new();

        let secret = SecretRef {
            name: "TOKEN".to_string(),
            source: SecretSource::Direct("abc123xyz".to_string()),
        };
        manager.load_secret(&secret).unwrap();

        let text = "Using token abc123xyz for authentication";
        let redacted = manager.redact(text);

        assert_eq!(redacted, "Using token [REDACTED:TOKEN] for authentication");
        assert!(!redacted.contains("abc123xyz"));
    }

    #[test]
    fn secrets_manager_redacts_multiple_secrets() {
        let mut manager = SecretsManager::new();

        manager
            .load_secret(&SecretRef {
                name: "API_KEY".to_string(),
                source: SecretSource::Direct("key123".to_string()),
            })
            .unwrap();

        manager
            .load_secret(&SecretRef {
                name: "DB_PASSWORD".to_string(),
                source: SecretSource::Direct("pass456".to_string()),
            })
            .unwrap();

        let text = "Connecting with key123 and password pass456";
        let redacted = manager.redact(text);

        assert!(!redacted.contains("key123"));
        assert!(!redacted.contains("pass456"));
        assert!(redacted.contains("[REDACTED:API_KEY]"));
        assert!(redacted.contains("[REDACTED:DB_PASSWORD]"));
    }

    #[test]
    fn secrets_manager_redacts_multiple_occurrences() {
        let mut manager = SecretsManager::new();

        manager
            .load_secret(&SecretRef {
                name: "SECRET".to_string(),
                source: SecretSource::Direct("mysecret".to_string()),
            })
            .unwrap();

        let text = "First mysecret then mysecret again";
        let redacted = manager.redact(text);

        assert_eq!(
            redacted,
            "First [REDACTED:SECRET] then [REDACTED:SECRET] again"
        );
    }

    #[test]
    fn secrets_manager_redacts_lines() {
        let mut manager = SecretsManager::new();

        manager
            .load_secret(&SecretRef {
                name: "KEY".to_string(),
                source: SecretSource::Direct("secret".to_string()),
            })
            .unwrap();

        let lines = vec![
            "Line 1 with secret".to_string(),
            "Line 2 without".to_string(),
            "Line 3 with secret again".to_string(),
        ];

        let redacted = manager.redact_lines(&lines);

        assert_eq!(redacted[0], "Line 1 with [REDACTED:KEY]");
        assert_eq!(redacted[1], "Line 2 without");
        assert_eq!(redacted[2], "Line 3 with [REDACTED:KEY] again");
    }

    #[test]
    fn secrets_manager_handles_empty_text() {
        let mut manager = SecretsManager::new();

        manager
            .load_secret(&SecretRef {
                name: "KEY".to_string(),
                source: SecretSource::Direct("secret".to_string()),
            })
            .unwrap();

        assert_eq!(manager.redact(""), "");
    }

    #[test]
    fn secrets_manager_handles_no_secrets() {
        let manager = SecretsManager::new();

        let text = "Some text with no secrets to redact";
        assert_eq!(manager.redact(text), text);
    }

    #[test]
    fn secrets_manager_clear_removes_all() {
        let mut manager = SecretsManager::new();

        manager
            .load_secret(&SecretRef {
                name: "KEY".to_string(),
                source: SecretSource::Direct("secret".to_string()),
            })
            .unwrap();

        assert!(manager.has_secrets());

        manager.clear();

        assert!(!manager.has_secrets());
        assert_eq!(manager.secret_count(), 0);
    }

    #[test]
    fn secrets_manager_loads_file_secret() {
        use std::io::Write;

        let mut manager = SecretsManager::new();

        // Create temp file with secret
        let temp_dir = tempfile::tempdir().unwrap();
        let secret_file = temp_dir.path().join("secret.txt");
        let mut file = std::fs::File::create(&secret_file).unwrap();
        writeln!(file, "file-secret-value").unwrap();

        let secret = SecretRef {
            name: "FILE_SECRET".to_string(),
            source: SecretSource::File(secret_file.to_string_lossy().to_string()),
        };

        manager.load_secret(&secret).expect("should load secret");

        assert_eq!(
            manager.environment().get("FILE_SECRET"),
            Some(&"file-secret-value".to_string())
        );
    }

    #[test]
    fn secrets_manager_fails_on_missing_file() {
        let mut manager = SecretsManager::new();

        let secret = SecretRef {
            name: "MISSING".to_string(),
            source: SecretSource::File("/nonexistent/path/secret.txt".to_string()),
        };

        let result = manager.load_secret(&secret);
        assert!(result.is_err());
    }
}
