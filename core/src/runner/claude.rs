//! Claude Code CLI runner.

use std::process::Stdio;

use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

use crate::error::{Error, Result};

use super::{LLMOutput, LLMResult, LLMRunner, LLMSpawnConfig};

/// Runner for Claude Code CLI.
pub struct ClaudeRunner {
    /// Path to the claude CLI binary.
    cli_path: String,
}

impl Default for ClaudeRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl ClaudeRunner {
    /// Creates a new Claude runner using the default `claude` command.
    pub fn new() -> Self {
        Self {
            cli_path: "claude".to_string(),
        }
    }

    /// Creates a new Claude runner with a custom CLI path.
    pub fn with_cli_path(cli_path: impl Into<String>) -> Self {
        Self {
            cli_path: cli_path.into(),
        }
    }

    /// Builds the command arguments for spawning Claude.
    fn build_args(&self, config: &LLMSpawnConfig) -> Vec<String> {
        let mut args = vec![
            "--print".to_string(), // Non-interactive mode
        ];

        // Add model if specified
        if let Some(model) = &config.model {
            args.push("--model".to_string());
            args.push(model.clone());
        }

        // Add allowed tools from manifest
        if !config.manifest.allowed_tools.is_empty() {
            args.push("--allowedTools".to_string());
            args.push(config.manifest.allowed_tools.join(","));
        }

        // Add the prompt
        args.push(config.prompt.clone());

        args
    }
}

#[async_trait]
impl LLMRunner for ClaudeRunner {
    async fn spawn(
        &self,
        config: LLMSpawnConfig,
        output_tx: mpsc::Sender<LLMOutput>,
    ) -> Result<LLMResult> {
        let args = self.build_args(&config);

        tracing::info!(
            cli = %self.cli_path,
            working_dir = ?config.working_dir,
            "spawning Claude CLI"
        );

        let mut child = Command::new(&self.cli_path)
            .args(&args)
            .current_dir(&config.working_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null())
            .spawn()
            .map_err(|e| Error::SandboxCreation(format!("failed to spawn claude: {}", e)))?;

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
            .map_err(|e| Error::SandboxCreation(format!("failed to wait for claude: {}", e)))?;

        Ok(LLMResult {
            exit_status: status,
            output_lines,
            success: status.success(),
        })
    }

    fn name(&self) -> &str {
        "claude-code"
    }
}

impl ClaudeRunner {
    /// Parses an output line to detect tool calls and file operations.
    fn parse_output_line(&self, line: &str) -> LLMOutput {
        // Detect Read tool calls
        if line.contains("Read(") || line.contains("reading file") {
            if let Some(path) = self.extract_path(line) {
                return LLMOutput::FileRead(path.into());
            }
        }

        // Detect Write/Edit tool calls
        if line.contains("Write(") || line.contains("Edit(") || line.contains("writing file") {
            if let Some(path) = self.extract_path(line) {
                return LLMOutput::FileWrite(path.into());
            }
        }

        // Detect generic tool calls
        if line.contains("Tool:") || line.contains("using tool") {
            if let Some((tool, args)) = self.extract_tool_call(line) {
                return LLMOutput::ToolCall { tool, args };
            }
        }

        LLMOutput::Stdout(line.to_string())
    }

    /// Extracts a file path from output.
    fn extract_path(&self, line: &str) -> Option<String> {
        // Look for paths in quotes or after known patterns
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

    /// Extracts a tool call from output.
    fn extract_tool_call(&self, line: &str) -> Option<(String, String)> {
        // Pattern: "Tool: ToolName(args)" or "using tool ToolName"
        if line.contains("Tool:") {
            let parts: Vec<&str> = line.split("Tool:").collect();
            if parts.len() > 1 {
                let rest = parts[1].trim();
                if let Some(paren) = rest.find('(') {
                    let tool = rest[..paren].trim().to_string();
                    let end = rest.rfind(')').unwrap_or(rest.len());
                    let args = rest[paren + 1..end].to_string();
                    return Some((tool, args));
                } else {
                    return Some((rest.to_string(), String::new()));
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
    fn claude_runner_builds_basic_args() {
        let runner = ClaudeRunner::new();
        let config = LLMSpawnConfig {
            prompt: "test prompt".to_string(),
            working_dir: "/tmp/test".into(),
            manifest: Default::default(),
            model: None,
        };

        let args = runner.build_args(&config);

        assert!(args.contains(&"--print".to_string()));
        assert!(args.contains(&"test prompt".to_string()));
    }

    #[test]
    fn claude_runner_includes_model_in_args() {
        let runner = ClaudeRunner::new();
        let config = LLMSpawnConfig {
            prompt: "test".to_string(),
            working_dir: "/tmp".into(),
            manifest: Default::default(),
            model: Some("haiku".to_string()),
        };

        let args = runner.build_args(&config);

        assert!(args.contains(&"--model".to_string()));
        assert!(args.contains(&"haiku".to_string()));
    }

    #[test]
    fn claude_runner_includes_allowed_tools() {
        let runner = ClaudeRunner::new();
        let mut manifest = crate::sandbox::SandboxManifest::default();
        manifest.allowed_tools = vec!["Read".to_string(), "Write".to_string()];

        let config = LLMSpawnConfig {
            prompt: "test".to_string(),
            working_dir: "/tmp".into(),
            manifest,
            model: None,
        };

        let args = runner.build_args(&config);

        assert!(args.contains(&"--allowedTools".to_string()));
        assert!(args.contains(&"Read,Write".to_string()));
    }

    #[test]
    fn claude_runner_parses_stdout_line() {
        let runner = ClaudeRunner::new();

        let output = runner.parse_output_line("Just some regular output");
        assert!(matches!(output, LLMOutput::Stdout(_)));
    }

    #[test]
    fn claude_runner_detects_file_read() {
        let runner = ClaudeRunner::new();

        let output = runner.parse_output_line("Read(\"/src/main.rs\")");
        assert!(matches!(output, LLMOutput::FileRead(_)));
    }

    #[test]
    fn claude_runner_detects_file_write() {
        let runner = ClaudeRunner::new();

        let output = runner.parse_output_line("Write(\"/src/new_file.rs\")");
        assert!(matches!(output, LLMOutput::FileWrite(_)));
    }

    #[test]
    fn claude_runner_detects_tool_call() {
        let runner = ClaudeRunner::new();

        let output = runner.parse_output_line("Tool: Bash(ls -la)");
        if let LLMOutput::ToolCall { tool, args } = output {
            assert_eq!(tool, "Bash");
            assert_eq!(args, "ls -la");
        } else {
            panic!("Expected ToolCall");
        }
    }

    #[test]
    fn claude_runner_has_correct_name() {
        let runner = ClaudeRunner::new();
        assert_eq!(runner.name(), "claude-code");
    }

    #[test]
    fn claude_runner_with_custom_path() {
        let runner = ClaudeRunner::with_cli_path("/usr/local/bin/claude");
        assert_eq!(runner.cli_path, "/usr/local/bin/claude");
    }
}
