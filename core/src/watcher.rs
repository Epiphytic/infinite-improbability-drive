//! Watcher agent for orchestrating spawn lifecycle.
//!
//! The watcher agent monitors spawned LLM instances, handles permission errors,
//! and manages the recovery process.

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::mpsc;

use crate::error::Result;
use crate::monitor::{ProgressMonitor, ProgressSummary, TimeoutConfig, TimeoutReason};
use crate::permissions::{PermissionDetector, PermissionError, PermissionFix};
use crate::runner::{LLMOutput, LLMRunner, LLMSpawnConfig};
use crate::sandbox::{Sandbox, SandboxManifest, SandboxProvider};

/// Recovery strategy for permission errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RecoveryStrategy {
    /// Moderate recovery - limited escalation attempts.
    #[default]
    Moderate,
    /// Aggressive recovery - keep trying until CannotFix.
    Aggressive,
    /// Interactive recovery - pause and ask user.
    Interactive,
}

/// Configuration for the watcher agent.
#[derive(Debug, Clone)]
pub struct WatcherConfig {
    /// Timeout configuration.
    pub timeout: TimeoutConfig,
    /// Recovery strategy.
    pub recovery_strategy: RecoveryStrategy,
    /// Maximum permission escalations for moderate mode.
    pub max_escalations: u32,
}

impl Default for WatcherConfig {
    fn default() -> Self {
        Self {
            timeout: TimeoutConfig::default(),
            recovery_strategy: RecoveryStrategy::Moderate,
            max_escalations: 1,
        }
    }
}

/// Result of a watcher-managed spawn.
#[derive(Debug)]
pub struct WatcherResult {
    /// Whether the spawn completed successfully.
    pub success: bool,
    /// Progress summary.
    pub progress: ProgressSummary,
    /// Permission errors encountered.
    pub permission_errors: Vec<PermissionError>,
    /// Applied fixes.
    pub applied_fixes: Vec<PermissionFix>,
    /// Reason for termination, if any.
    pub termination_reason: Option<TerminationReason>,
}

/// Reason the watcher terminated the spawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminationReason {
    /// Completed successfully.
    Success,
    /// LLM exited with error.
    LLMError(String),
    /// Timeout occurred.
    Timeout(TimeoutReason),
    /// Unrecoverable permission error.
    PermissionError(String),
    /// Escalation limit reached.
    EscalationLimitReached,
}

/// The watcher agent that orchestrates spawn lifecycle.
pub struct WatcherAgent<P: SandboxProvider, R: LLMRunner> {
    /// Sandbox provider.
    provider: Arc<P>,
    /// LLM runner.
    runner: Arc<R>,
    /// Permission detector.
    detector: PermissionDetector,
    /// Configuration.
    config: WatcherConfig,
}

impl<P: SandboxProvider + 'static, R: LLMRunner + 'static> WatcherAgent<P, R> {
    /// Creates a new watcher agent.
    pub fn new(provider: P, runner: R, config: WatcherConfig) -> Self {
        Self {
            provider: Arc::new(provider),
            runner: Arc::new(runner),
            detector: PermissionDetector::new(),
            config,
        }
    }

    /// Runs a spawn with full lifecycle management.
    pub async fn run(
        &self,
        prompt: String,
        initial_manifest: SandboxManifest,
    ) -> Result<WatcherResult> {
        let mut manifest = initial_manifest;
        let mut permission_errors = Vec::new();
        let mut applied_fixes = Vec::new();
        let mut escalation_count = 0;

        loop {
            // Create sandbox
            let mut sandbox = self.provider.create(manifest.clone())?;

            // Run LLM with monitoring
            let result = self
                .run_with_monitoring(&prompt, sandbox.path().clone(), &manifest)
                .await;

            // Cleanup sandbox
            sandbox.cleanup()?;

            match result {
                Ok((progress, None)) => {
                    // Success!
                    return Ok(WatcherResult {
                        success: true,
                        progress,
                        permission_errors,
                        applied_fixes,
                        termination_reason: Some(TerminationReason::Success),
                    });
                }
                Ok((progress, Some(timeout_reason))) => {
                    // Timeout
                    return Ok(WatcherResult {
                        success: false,
                        progress,
                        permission_errors,
                        applied_fixes,
                        termination_reason: Some(TerminationReason::Timeout(timeout_reason)),
                    });
                }
                Err(WatcherError::PermissionErrors(errors, progress)) => {
                    // Handle permission errors based on strategy
                    for error in &errors {
                        permission_errors.push(error.clone());

                        match &error.fix {
                            PermissionFix::CannotFix(reason) => {
                                return Ok(WatcherResult {
                                    success: false,
                                    progress,
                                    permission_errors,
                                    applied_fixes,
                                    termination_reason: Some(TerminationReason::PermissionError(
                                        reason.clone(),
                                    )),
                                });
                            }
                            fix => {
                                // Check escalation limit for moderate mode
                                if self.config.recovery_strategy == RecoveryStrategy::Moderate
                                    && escalation_count >= self.config.max_escalations
                                {
                                    return Ok(WatcherResult {
                                        success: false,
                                        progress,
                                        permission_errors,
                                        applied_fixes,
                                        termination_reason: Some(
                                            TerminationReason::EscalationLimitReached,
                                        ),
                                    });
                                }

                                // Apply fix
                                self.apply_fix(&mut manifest, fix);
                                applied_fixes.push(fix.clone());
                                escalation_count += 1;
                            }
                        }
                    }
                    // Continue loop with updated manifest
                }
                Err(WatcherError::LLMError(msg, progress)) => {
                    return Ok(WatcherResult {
                        success: false,
                        progress,
                        permission_errors,
                        applied_fixes,
                        termination_reason: Some(TerminationReason::LLMError(msg)),
                    });
                }
            }
        }
    }

    /// Runs the LLM with progress monitoring.
    async fn run_with_monitoring(
        &self,
        prompt: &str,
        working_dir: PathBuf,
        manifest: &SandboxManifest,
    ) -> std::result::Result<(ProgressSummary, Option<TimeoutReason>), WatcherError> {
        let mut monitor = ProgressMonitor::new(self.config.timeout);
        let mut detected_errors = Vec::new();

        // Create output channel
        let (tx, mut rx) = mpsc::channel::<LLMOutput>(100);

        // Build spawn config
        let spawn_config = LLMSpawnConfig {
            prompt: prompt.to_string(),
            working_dir,
            manifest: manifest.clone(),
            model: None,
        };

        // Spawn LLM in background
        let runner = self.runner.clone();
        let llm_handle = tokio::spawn(async move { runner.spawn(spawn_config, tx).await });

        // Process output with monitoring
        while let Some(output) = rx.recv().await {
            // Check for timeout
            if let Some(reason) = monitor.check_timeout() {
                // Cancel LLM
                llm_handle.abort();
                return Ok((ProgressSummary::from(&monitor), Some(reason)));
            }

            // Process output
            match &output {
                LLMOutput::Stdout(line) => {
                    monitor.record_output(1);

                    // Check for permission errors
                    if let Some(error) = self.detector.analyze(line) {
                        detected_errors.push(error);
                    }
                }
                LLMOutput::Stderr(line) => {
                    monitor.record_output(1);

                    // Check for permission errors
                    if let Some(error) = self.detector.analyze(line) {
                        detected_errors.push(error);
                    }
                }
                LLMOutput::FileRead(path) => {
                    monitor.record_file_read(path.clone());
                }
                LLMOutput::FileWrite(path) => {
                    monitor.record_file_write(path.clone());
                }
                LLMOutput::ToolCall { .. } => {
                    monitor.touch();
                }
            }
        }

        // Wait for LLM to finish
        let llm_result = llm_handle.await.map_err(|e| {
            WatcherError::LLMError(format!("LLM task panicked: {}", e), ProgressSummary::from(&monitor))
        })?.map_err(|e| {
            WatcherError::LLMError(format!("LLM error: {}", e), ProgressSummary::from(&monitor))
        })?;

        // Check for permission errors
        if !detected_errors.is_empty() {
            return Err(WatcherError::PermissionErrors(
                detected_errors,
                ProgressSummary::from(&monitor),
            ));
        }

        // Check exit status
        if !llm_result.success {
            return Err(WatcherError::LLMError(
                "LLM exited with non-zero status".to_string(),
                ProgressSummary::from(&monitor),
            ));
        }

        Ok((ProgressSummary::from(&monitor), None))
    }

    /// Applies a permission fix to the manifest.
    fn apply_fix(&self, manifest: &mut SandboxManifest, fix: &PermissionFix) {
        match fix {
            PermissionFix::AddReadPath(pattern) => {
                if !manifest.readable_paths.contains(pattern) {
                    manifest.readable_paths.push(pattern.clone());
                }
            }
            PermissionFix::AddWritePath(pattern) => {
                if !manifest.writable_paths.contains(pattern) {
                    manifest.writable_paths.push(pattern.clone());
                }
            }
            PermissionFix::AllowCommand(cmd) => {
                if !manifest.allowed_commands.contains(cmd) {
                    manifest.allowed_commands.push(cmd.clone());
                }
            }
            PermissionFix::EnableTool(tool) => {
                if !manifest.allowed_tools.contains(tool) {
                    manifest.allowed_tools.push(tool.clone());
                }
            }
            PermissionFix::InjectEnvVar(var) => {
                // Mark as needing injection (actual value comes from secrets manager)
                manifest
                    .environment
                    .insert(var.clone(), format!("${{{}}}", var));
            }
            PermissionFix::InjectSecret(secret) => {
                if !manifest.secrets.contains(secret) {
                    manifest.secrets.push(secret.clone());
                }
            }
            PermissionFix::CannotFix(_) => {
                // Cannot apply - caller handles this
            }
        }
    }
}

/// Internal error type for watcher operations.
enum WatcherError {
    PermissionErrors(Vec<PermissionError>, ProgressSummary),
    LLMError(String, ProgressSummary),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watcher_config_has_sensible_defaults() {
        let config = WatcherConfig::default();

        assert_eq!(config.recovery_strategy, RecoveryStrategy::Moderate);
        assert_eq!(config.max_escalations, 1);
    }

    #[test]
    fn recovery_strategy_default_is_moderate() {
        assert_eq!(RecoveryStrategy::default(), RecoveryStrategy::Moderate);
    }

    #[test]
    fn termination_reason_equality() {
        assert_eq!(TerminationReason::Success, TerminationReason::Success);
        assert_eq!(
            TerminationReason::Timeout(TimeoutReason::Idle),
            TerminationReason::Timeout(TimeoutReason::Idle)
        );
        assert_ne!(
            TerminationReason::Timeout(TimeoutReason::Idle),
            TerminationReason::Timeout(TimeoutReason::Total)
        );
    }

    #[test]
    fn apply_fix_adds_read_path() {
        // We can't easily create a WatcherAgent without real providers,
        // so test the fix application logic directly through a helper

        let mut manifest = SandboxManifest::default();
        assert!(manifest.readable_paths.is_empty());

        // Simulate applying fix
        let fix = PermissionFix::AddReadPath("src/**".to_string());
        apply_fix_to_manifest(&mut manifest, &fix);

        assert_eq!(manifest.readable_paths, vec!["src/**"]);
    }

    #[test]
    fn apply_fix_adds_write_path() {
        let mut manifest = SandboxManifest::default();

        let fix = PermissionFix::AddWritePath("tests/**".to_string());
        apply_fix_to_manifest(&mut manifest, &fix);

        assert_eq!(manifest.writable_paths, vec!["tests/**"]);
    }

    #[test]
    fn apply_fix_enables_tool() {
        let mut manifest = SandboxManifest::default();

        let fix = PermissionFix::EnableTool("Bash".to_string());
        apply_fix_to_manifest(&mut manifest, &fix);

        assert_eq!(manifest.allowed_tools, vec!["Bash"]);
    }

    #[test]
    fn apply_fix_allows_command() {
        let mut manifest = SandboxManifest::default();

        let fix = PermissionFix::AllowCommand("npm test".to_string());
        apply_fix_to_manifest(&mut manifest, &fix);

        assert_eq!(manifest.allowed_commands, vec!["npm test"]);
    }

    #[test]
    fn apply_fix_injects_secret() {
        let mut manifest = SandboxManifest::default();

        let fix = PermissionFix::InjectSecret("API_KEY".to_string());
        apply_fix_to_manifest(&mut manifest, &fix);

        assert_eq!(manifest.secrets, vec!["API_KEY"]);
    }

    #[test]
    fn apply_fix_does_not_duplicate() {
        let mut manifest = SandboxManifest::default();
        manifest.allowed_tools.push("Read".to_string());

        let fix = PermissionFix::EnableTool("Read".to_string());
        apply_fix_to_manifest(&mut manifest, &fix);

        // Should not add duplicate
        assert_eq!(manifest.allowed_tools, vec!["Read"]);
    }

    /// Helper function to apply fixes (mirrors WatcherAgent::apply_fix)
    fn apply_fix_to_manifest(manifest: &mut SandboxManifest, fix: &PermissionFix) {
        match fix {
            PermissionFix::AddReadPath(pattern) => {
                if !manifest.readable_paths.contains(pattern) {
                    manifest.readable_paths.push(pattern.clone());
                }
            }
            PermissionFix::AddWritePath(pattern) => {
                if !manifest.writable_paths.contains(pattern) {
                    manifest.writable_paths.push(pattern.clone());
                }
            }
            PermissionFix::AllowCommand(cmd) => {
                if !manifest.allowed_commands.contains(cmd) {
                    manifest.allowed_commands.push(cmd.clone());
                }
            }
            PermissionFix::EnableTool(tool) => {
                if !manifest.allowed_tools.contains(tool) {
                    manifest.allowed_tools.push(tool.clone());
                }
            }
            PermissionFix::InjectEnvVar(var) => {
                manifest
                    .environment
                    .insert(var.clone(), format!("${{{}}}", var));
            }
            PermissionFix::InjectSecret(secret) => {
                if !manifest.secrets.contains(secret) {
                    manifest.secrets.push(secret.clone());
                }
            }
            PermissionFix::CannotFix(_) => {}
        }
    }
}
