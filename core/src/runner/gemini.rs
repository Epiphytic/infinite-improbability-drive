//! Gemini CLI runner.

use std::process::Stdio;

use async_trait::async_trait;
use serde_json;
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
            // Use plan approval mode for safe execution with auto-approved plans
            "--approval-mode".to_string(),
            "plan".to_string(),
            // Use stream-json for structured output that shows tool calls
            "--output-format".to_string(),
            "stream-json".to_string(),
        ];

        // Add model if specified
        if let Some(model) = &config.model {
            args.push("--model".to_string());
            args.push(model.clone());
        }

        // Add the prompt using --prompt flag
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
            // Disable fork-join plugin to avoid conflicts with cruise-control
            .env("FORK_JOIN_DISABLED", "1")
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
    /// Parses an output line (JSON from stream-json format) to detect tool calls and file operations.
    fn parse_output_line(&self, line: &str) -> LLMOutput {
        // Try to parse as JSON first (stream-json format)
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(line) {
            // Check for tool use in assistant messages
            // Gemini may use different JSON structure - check for common patterns
            if let Some(tool_call) = json.get("tool_call").or(json.get("function_call")) {
                let tool_name = tool_call
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string();

                if let Some(args) = tool_call.get("args").or(tool_call.get("arguments")) {
                    let file_path = args.get("path").or(args.get("file_path")).and_then(|p| p.as_str());

                    match tool_name.as_str() {
                        "read_file" | "ReadFile" | "Read" => {
                            if let Some(path) = file_path {
                                return LLMOutput::FileRead(path.into());
                            }
                        }
                        "write_file" | "WriteFile" | "Write" | "edit_file" | "EditFile" | "Edit" => {
                            if let Some(path) = file_path {
                                return LLMOutput::FileWrite(path.into());
                            }
                        }
                        _ => {
                            return LLMOutput::ToolCall {
                                tool: tool_name,
                                args: args.to_string(),
                            };
                        }
                    }
                }

                return LLMOutput::ToolCall {
                    tool: tool_name,
                    args: String::new(),
                };
            }

            // Check for tool results (file operations completed)
            if let Some(result) = json.get("tool_result").or(json.get("function_result")) {
                if let Some(file_path) = result.get("file_path").and_then(|p| p.as_str()) {
                    let result_type = result.get("type").and_then(|t| t.as_str()).unwrap_or("");
                    match result_type {
                        "write" | "create" | "edit" => {
                            return LLMOutput::FileWrite(file_path.into());
                        }
                        "read" => {
                            return LLMOutput::FileRead(file_path.into());
                        }
                        _ => {}
                    }
                }
            }

            // For other JSON messages, return as stdout
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
    fn gemini_runner_builds_basic_args() {
        let runner = GeminiRunner::new();
        let config = LLMSpawnConfig {
            prompt: "test prompt".to_string(),
            working_dir: "/tmp/test".into(),
            manifest: Default::default(),
            model: None,
        };

        let args = runner.build_args(&config);

        assert!(args.contains(&"--approval-mode".to_string()));
        assert!(args.contains(&"plan".to_string()));
        assert!(args.contains(&"--output-format".to_string()));
        assert!(args.contains(&"stream-json".to_string()));
        assert!(args.contains(&"--prompt".to_string()));
        assert!(args.contains(&"test prompt".to_string()));
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
    fn gemini_runner_parses_non_json_as_stdout() {
        let runner = GeminiRunner::new();

        let output = runner.parse_output_line("Just some regular output");
        assert!(matches!(output, LLMOutput::Stdout(_)));
    }

    #[test]
    fn gemini_runner_parses_json_write_tool_call() {
        let runner = GeminiRunner::new();

        let json = r#"{"tool_call":{"name":"write_file","args":{"path":"/src/main.rs","content":"test"}}}"#;
        let output = runner.parse_output_line(json);
        if let LLMOutput::FileWrite(path) = output {
            assert_eq!(path.to_string_lossy(), "/src/main.rs");
        } else {
            panic!("Expected FileWrite, got {:?}", output);
        }
    }

    #[test]
    fn gemini_runner_parses_json_read_tool_call() {
        let runner = GeminiRunner::new();

        let json = r#"{"tool_call":{"name":"read_file","args":{"path":"/src/lib.rs"}}}"#;
        let output = runner.parse_output_line(json);
        if let LLMOutput::FileRead(path) = output {
            assert_eq!(path.to_string_lossy(), "/src/lib.rs");
        } else {
            panic!("Expected FileRead, got {:?}", output);
        }
    }

    #[test]
    fn gemini_runner_parses_json_generic_tool_call() {
        let runner = GeminiRunner::new();

        let json = r#"{"tool_call":{"name":"execute_shell","args":{"command":"ls -la"}}}"#;
        let output = runner.parse_output_line(json);
        if let LLMOutput::ToolCall { tool, args } = output {
            assert_eq!(tool, "execute_shell");
            assert!(args.contains("ls -la"));
        } else {
            panic!("Expected ToolCall, got {:?}", output);
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
