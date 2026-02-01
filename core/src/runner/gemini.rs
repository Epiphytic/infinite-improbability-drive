//! Gemini CLI runner.

use std::process::Stdio;

use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

use crate::error::{Error, Result};

use super::{LLMOutput, LLMResult, LLMRunner, LLMSpawnConfig};

/// Runner for Gemini CLI.
pub struct GeminiRunner {
    /// Path to the gemini CLI binary.
    cli_path: String,
}

impl Default for GeminiRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl GeminiRunner {
    /// Creates a new Gemini runner using the default `gemini` command.
    pub fn new() -> Self {
        Self {
            cli_path: "gemini".to_string(),
        }
    }

    /// Creates a new Gemini runner with a custom CLI path.
    pub fn with_cli_path(cli_path: impl Into<String>) -> Self {
        Self {
            cli_path: cli_path.into(),
        }
    }

    /// Builds the command arguments for spawning Gemini.
    fn build_args(&self, config: &LLMSpawnConfig) -> Vec<String> {
        let mut args = vec![
            "--non-interactive".to_string(),
        ];

        // Add model if specified
        if let Some(model) = &config.model {
            args.push("--model".to_string());
            args.push(model.clone());
        }

        // Add sandbox mode based on manifest
        if !config.manifest.allowed_commands.is_empty() {
            args.push("--sandbox".to_string());
            args.push("permissive".to_string());
        } else {
            args.push("--sandbox".to_string());
            args.push("strict".to_string());
        }

        // Add the prompt
        args.push("--prompt".to_string());
        args.push(config.prompt.clone());

        args
    }
}

#[async_trait]
impl LLMRunner for GeminiRunner {
    async fn spawn(
        &self,
        config: LLMSpawnConfig,
        output_tx: mpsc::Sender<LLMOutput>,
    ) -> Result<LLMResult> {
        let args = self.build_args(&config);

        tracing::info!(
            cli = %self.cli_path,
            working_dir = ?config.working_dir,
            "spawning Gemini CLI"
        );

        let mut child = Command::new(&self.cli_path)
            .args(&args)
            .current_dir(&config.working_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null())
            .spawn()
            .map_err(|e| Error::SandboxCreation(format!("failed to spawn gemini: {}", e)))?;

        let stdout = child.stdout.take().expect("stdout was piped");
        let stderr = child.stderr.take().expect("stderr was piped");

        let mut stdout_reader = BufReader::new(stdout).lines();
        let mut stderr_reader = BufReader::new(stderr).lines();

        let mut output_lines = 0;

        // Process stdout and stderr concurrently
        loop {
            tokio::select! {
                line = stdout_reader.next_line() => {
                    match line {
                        Ok(Some(line)) => {
                            output_lines += 1;

                            // Check for tool calls and file operations
                            let output = self.parse_output_line(&line);
                            if output_tx.send(output).await.is_err() {
                                tracing::warn!("output receiver dropped");
                                break;
                            }
                        }
                        Ok(None) => break,
                        Err(e) => {
                            tracing::error!(error = %e, "error reading stdout");
                            break;
                        }
                    }
                }
                line = stderr_reader.next_line() => {
                    match line {
                        Ok(Some(line)) => {
                            output_lines += 1;
                            if output_tx.send(LLMOutput::Stderr(line)).await.is_err() {
                                tracing::warn!("output receiver dropped");
                                break;
                            }
                        }
                        Ok(None) => {}
                        Err(e) => {
                            tracing::error!(error = %e, "error reading stderr");
                        }
                    }
                }
            }
        }

        let status = child
            .wait()
            .await
            .map_err(|e| Error::SandboxCreation(format!("failed to wait for gemini: {}", e)))?;

        Ok(LLMResult {
            exit_status: status,
            output_lines,
            success: status.success(),
        })
    }

    fn name(&self) -> &str {
        "gemini-cli"
    }
}

impl GeminiRunner {
    /// Parses an output line to detect tool calls and file operations.
    fn parse_output_line(&self, line: &str) -> LLMOutput {
        // Detect file reads (Gemini uses different patterns)
        if line.contains("reading") && line.contains("file") {
            if let Some(path) = self.extract_path(line) {
                return LLMOutput::FileRead(path.into());
            }
        }

        // Detect file writes
        if line.contains("writing") && line.contains("file") {
            if let Some(path) = self.extract_path(line) {
                return LLMOutput::FileWrite(path.into());
            }
        }

        // Detect tool/function calls
        if line.contains("function_call") || line.contains("tool_use") {
            if let Some((tool, args)) = self.extract_function_call(line) {
                return LLMOutput::ToolCall { tool, args };
            }
        }

        LLMOutput::Stdout(line.to_string())
    }

    /// Extracts a file path from output.
    fn extract_path(&self, line: &str) -> Option<String> {
        // Look for paths in quotes
        if let Some(start) = line.find('"') {
            if let Some(end) = line[start + 1..].find('"') {
                let path = &line[start + 1..start + 1 + end];
                if path.contains('/') || path.contains('\\') {
                    return Some(path.to_string());
                }
            }
        }

        if let Some(start) = line.find('\'') {
            if let Some(end) = line[start + 1..].find('\'') {
                let path = &line[start + 1..start + 1 + end];
                if path.contains('/') || path.contains('\\') {
                    return Some(path.to_string());
                }
            }
        }

        None
    }

    /// Extracts a function call from output.
    fn extract_function_call(&self, line: &str) -> Option<(String, String)> {
        // Pattern: "function_call: name(args)" or similar
        if line.contains("function_call:") {
            let parts: Vec<&str> = line.split("function_call:").collect();
            if parts.len() > 1 {
                let rest = parts[1].trim();
                if let Some(paren) = rest.find('(') {
                    let name = rest[..paren].trim().to_string();
                    let end = rest.rfind(')').unwrap_or(rest.len());
                    let args = rest[paren + 1..end].to_string();
                    return Some((name, args));
                }
            }
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gemini_runner_builds_basic_args() {
        let runner = GeminiRunner::new();
        let config = LLMSpawnConfig {
            prompt: "test prompt".to_string(),
            working_dir: "/tmp/test".into(),
            manifest: Default::default(),
            model: None,
        };

        let args = runner.build_args(&config);

        assert!(args.contains(&"--non-interactive".to_string()));
        assert!(args.contains(&"--prompt".to_string()));
        assert!(args.contains(&"test prompt".to_string()));
        assert!(args.contains(&"--sandbox".to_string()));
        assert!(args.contains(&"strict".to_string()));
    }

    #[test]
    fn gemini_runner_includes_model_in_args() {
        let runner = GeminiRunner::new();
        let config = LLMSpawnConfig {
            prompt: "test".to_string(),
            working_dir: "/tmp".into(),
            manifest: Default::default(),
            model: Some("gemini-pro".to_string()),
        };

        let args = runner.build_args(&config);

        assert!(args.contains(&"--model".to_string()));
        assert!(args.contains(&"gemini-pro".to_string()));
    }

    #[test]
    fn gemini_runner_uses_permissive_sandbox_with_commands() {
        let runner = GeminiRunner::new();
        let mut manifest = crate::sandbox::SandboxManifest::default();
        manifest.allowed_commands = vec!["npm test".to_string()];

        let config = LLMSpawnConfig {
            prompt: "test".to_string(),
            working_dir: "/tmp".into(),
            manifest,
            model: None,
        };

        let args = runner.build_args(&config);

        assert!(args.contains(&"--sandbox".to_string()));
        assert!(args.contains(&"permissive".to_string()));
    }

    #[test]
    fn gemini_runner_parses_stdout_line() {
        let runner = GeminiRunner::new();

        let output = runner.parse_output_line("Just some regular output");
        assert!(matches!(output, LLMOutput::Stdout(_)));
    }

    #[test]
    fn gemini_runner_detects_file_read() {
        let runner = GeminiRunner::new();

        let output = runner.parse_output_line("reading file \"/src/main.rs\"");
        assert!(matches!(output, LLMOutput::FileRead(_)));
    }

    #[test]
    fn gemini_runner_detects_file_write() {
        let runner = GeminiRunner::new();

        let output = runner.parse_output_line("writing file \"/src/new.rs\"");
        assert!(matches!(output, LLMOutput::FileWrite(_)));
    }

    #[test]
    fn gemini_runner_detects_function_call() {
        let runner = GeminiRunner::new();

        let output = runner.parse_output_line("function_call: execute_code(print('hello'))");
        if let LLMOutput::ToolCall { tool, args } = output {
            assert_eq!(tool, "execute_code");
            assert_eq!(args, "print('hello')");
        } else {
            panic!("Expected ToolCall");
        }
    }

    #[test]
    fn gemini_runner_has_correct_name() {
        let runner = GeminiRunner::new();
        assert_eq!(runner.name(), "gemini-cli");
    }

    #[test]
    fn gemini_runner_with_custom_path() {
        let runner = GeminiRunner::with_cli_path("/usr/local/bin/gemini");
        assert_eq!(runner.cli_path, "/usr/local/bin/gemini");
    }
}
