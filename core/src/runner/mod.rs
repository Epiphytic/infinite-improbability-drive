//! LLM runner implementations for spawning CLI-based LLMs.
//!
//! Supports Claude Code and Gemini CLI in headless streaming mode.

mod claude;
mod gemini;

pub use claude::ClaudeRunner;
pub use gemini::GeminiRunner;

use std::path::PathBuf;
use std::process::ExitStatus;

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::error::Result;
use crate::sandbox::SandboxManifest;

/// Output from an LLM during execution.
#[derive(Debug, Clone)]
pub enum LLMOutput {
    /// Standard output line.
    Stdout(String),
    /// Standard error line.
    Stderr(String),
    /// Tool call detected.
    ToolCall { tool: String, args: String },
    /// File read detected.
    FileRead(PathBuf),
    /// File write detected.
    FileWrite(PathBuf),
}

/// Configuration for spawning an LLM.
#[derive(Debug, Clone)]
pub struct LLMSpawnConfig {
    /// The prompt to send.
    pub prompt: String,
    /// Working directory (sandbox path).
    pub working_dir: PathBuf,
    /// Sandbox manifest with permissions.
    pub manifest: SandboxManifest,
    /// Model to use (e.g., "sonnet", "haiku", "opus").
    pub model: Option<String>,
}

/// Result of an LLM execution.
#[derive(Debug)]
pub struct LLMResult {
    /// Exit status of the process.
    pub exit_status: ExitStatus,
    /// Total output lines.
    pub output_lines: usize,
    /// Whether the LLM completed successfully.
    pub success: bool,
}

/// Trait for LLM runners.
#[async_trait]
pub trait LLMRunner: Send + Sync {
    /// Spawns the LLM with the given configuration.
    ///
    /// Output is streamed to the provided channel.
    async fn spawn(
        &self,
        config: LLMSpawnConfig,
        output_tx: mpsc::Sender<LLMOutput>,
    ) -> Result<LLMResult>;

    /// Returns the name of this runner.
    fn name(&self) -> &str;
}
