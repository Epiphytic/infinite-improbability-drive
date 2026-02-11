//! Claude Code CLI runner.

use std::process::Stdio;

use async_trait::async_trait;
use serde_json;
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
            // Skip permissions since we're in a sandbox - the sandbox provides isolation
            "--dangerously-skip-permissions".to_string(),
            // Use stream-json for structured output that shows tool calls
            "--output-format".to_string(),
            "stream-json".to_string(),
            "--verbose".to_string(), // Required for stream-json
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

        // Add the prompt via -p flag (required for --print mode)
        args.push("-p".to_string());
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
            // Disable fork-join plugin to avoid conflicts with cruise-control
            .env("FORK_JOIN_DISABLED", "1")
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

                            // Debug log to see what we're receiving
                            tracing::debug!(line = %line, "claude stdout");

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
    /// Parses an output line (JSON from stream-json format) to detect tool calls and file operations.
    fn parse_output_line(&self, line: &str) -> LLMOutput {
        // Try to parse as JSON first (stream-json format)
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(line) {
            // Check for tool use in assistant messages
            if json.get("type").and_then(|t| t.as_str()) == Some("assistant") {
                if let Some(content) = json
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_array())
                {
                    for item in content {
                        if item.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                            let tool_name = item
                                .get("name")
                                .and_then(|n| n.as_str())
                                .unwrap_or("")
                                .to_string();

                            // Extract file path from input if present
                            if let Some(input) = item.get("input") {
                                let file_path = input
                                    .get("file_path")
                                    .and_then(|p| p.as_str())
                                    .map(|s| s.to_string());

                                match tool_name.as_str() {
                                    "Read" => {
                                        if let Some(path) = file_path {
                                            return LLMOutput::FileRead(path.into());
                                        }
                                    }
                                    "Write" | "Edit" | "NotebookEdit" => {
                                        if let Some(path) = file_path {
                                            return LLMOutput::FileWrite(path.into());
                                        }
                                    }
                                    _ => {
                                        let args = input.to_string();
                                        return LLMOutput::ToolCall {
                                            tool: tool_name,
                                            args,
                                        };
                                    }
                                }
                            }

                            return LLMOutput::ToolCall {
                                tool: tool_name,
                                args: String::new(),
                            };
                        }
                    }
                }
            }

            // Check for tool_use_result (shows completed file operations)
            if json.get("type").and_then(|t| t.as_str()) == Some("user") {
                if let Some(result) = json.get("tool_use_result") {
                    let result_type = result
                        .get("type")
                        .and_then(|t| t.as_str())
                        .unwrap_or("");

                    if let Some(file_path) = result.get("filePath").and_then(|p| p.as_str()) {
                        match result_type {
                            "create" | "update" => {
                                return LLMOutput::FileWrite(file_path.into());
                            }
                            "read" => {
                                return LLMOutput::FileRead(file_path.into());
                            }
                            _ => {}
                        }
                    }
                }
            }

            // For other JSON messages, just return as stdout
            return LLMOutput::Stdout(line.to_string());
        }

        // Fallback: not JSON, return as raw output
        LLMOutput::Stdout(line.to_string())
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
    fn claude_runner_parses_non_json_as_stdout() {
        let runner = ClaudeRunner::new();

        let output = runner.parse_output_line("Just some regular output");
        assert!(matches!(output, LLMOutput::Stdout(_)));
    }

    #[test]
    fn claude_runner_parses_json_write_tool_use() {
        let runner = ClaudeRunner::new();

        let json = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Write","input":{"file_path":"/src/main.rs","content":"test"}}]}}"#;
        let output = runner.parse_output_line(json);
        if let LLMOutput::FileWrite(path) = output {
            assert_eq!(path.to_string_lossy(), "/src/main.rs");
        } else {
            panic!("Expected FileWrite, got {:?}", output);
        }
    }

    #[test]
    fn claude_runner_parses_json_read_tool_use() {
        let runner = ClaudeRunner::new();

        let json = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"/src/lib.rs"}}]}}"#;
        let output = runner.parse_output_line(json);
        if let LLMOutput::FileRead(path) = output {
            assert_eq!(path.to_string_lossy(), "/src/lib.rs");
        } else {
            panic!("Expected FileRead, got {:?}", output);
        }
    }

    #[test]
    fn claude_runner_parses_json_tool_use_result_create() {
        let runner = ClaudeRunner::new();

        let json = r#"{"type":"user","tool_use_result":{"type":"create","filePath":"/test/file.txt"}}"#;
        let output = runner.parse_output_line(json);
        if let LLMOutput::FileWrite(path) = output {
            assert_eq!(path.to_string_lossy(), "/test/file.txt");
        } else {
            panic!("Expected FileWrite, got {:?}", output);
        }
    }

    #[test]
    fn claude_runner_parses_json_generic_tool_call() {
        let runner = ClaudeRunner::new();

        let json = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"ls -la"}}]}}"#;
        let output = runner.parse_output_line(json);
        if let LLMOutput::ToolCall { tool, args } = output {
            assert_eq!(tool, "Bash");
            assert!(args.contains("ls -la"));
        } else {
            panic!("Expected ToolCall, got {:?}", output);
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
