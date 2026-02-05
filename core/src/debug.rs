//! Debug and fail-fast configuration for cruise-control.
//!
//! Environment variables:
//! - `CRUISE_DEBUG=1` - Enable verbose debug logging
//! - `CRUISE_FAIL_FAST=1` - Exit immediately on any error
//!
//! When CRUISE_DEBUG is enabled:
//! - All tracing::debug! messages are visible
//! - LLM command invocations are printed with full arguments
//! - Git operations show detailed output
//! - Review phases print intermediate results
//!
//! When CRUISE_FAIL_FAST is enabled:
//! - Any LLM failure immediately terminates the workflow
//! - Any git operation failure immediately terminates
//! - Review phases that fail stop the entire process
//! - Partial work is NOT preserved

use std::sync::OnceLock;

/// Global debug configuration loaded once at startup.
static DEBUG_CONFIG: OnceLock<DebugConfig> = OnceLock::new();

/// Debug and fail-fast configuration.
#[derive(Debug, Clone)]
pub struct DebugConfig {
    /// Enable verbose debug logging.
    pub debug_mode: bool,
    /// Exit immediately on any error.
    pub fail_fast: bool,
}

impl Default for DebugConfig {
    fn default() -> Self {
        Self::from_env()
    }
}

impl DebugConfig {
    /// Loads configuration from environment variables.
    pub fn from_env() -> Self {
        Self {
            debug_mode: std::env::var("CRUISE_DEBUG")
                .map(|v| v == "1" || v.to_lowercase() == "true")
                .unwrap_or(false),
            fail_fast: std::env::var("CRUISE_FAIL_FAST")
                .map(|v| v == "1" || v.to_lowercase() == "true")
                .unwrap_or(false),
        }
    }

    /// Creates a test configuration with both modes enabled.
    #[cfg(test)]
    pub fn test_config() -> Self {
        Self {
            debug_mode: true,
            fail_fast: true,
        }
    }
}

/// Gets the global debug configuration.
///
/// This is initialized once from environment variables.
pub fn get_config() -> &'static DebugConfig {
    DEBUG_CONFIG.get_or_init(DebugConfig::from_env)
}

/// Returns true if debug mode is enabled.
pub fn is_debug() -> bool {
    get_config().debug_mode
}

/// Returns true if fail-fast mode is enabled.
pub fn is_fail_fast() -> bool {
    get_config().fail_fast
}

/// Logs a debug message with source location.
///
/// Only logs if CRUISE_DEBUG is enabled.
#[macro_export]
macro_rules! cruise_debug {
    ($($arg:tt)*) => {
        if $crate::debug::is_debug() {
            tracing::debug!($($arg)*);
        }
    };
}

/// Logs an error and optionally panics in fail-fast mode.
///
/// In fail-fast mode, this macro will panic after logging.
/// In normal mode, it just logs the error.
#[macro_export]
macro_rules! cruise_error {
    ($($arg:tt)*) => {{
        tracing::error!($($arg)*);
        if $crate::debug::is_fail_fast() {
            panic!("CRUISE_FAIL_FAST enabled - aborting on error");
        }
    }};
}

/// Prints debug info about a command before running it.
pub fn debug_command(cmd: &str, args: &[&str], working_dir: &std::path::Path) {
    if is_debug() {
        eprintln!("[CRUISE_DEBUG] Running command:");
        eprintln!("  cmd: {}", cmd);
        eprintln!("  args: {:?}", args);
        eprintln!("  cwd: {}", working_dir.display());
    }
}

/// Prints debug info about an LLM invocation.
pub fn debug_llm_invocation(
    llm: &str,
    prompt_preview: &str,
    working_dir: &std::path::Path,
    iteration: u32,
    role: &str,
) {
    if is_debug() {
        eprintln!("[CRUISE_DEBUG] LLM Invocation:");
        eprintln!("  llm: {}", llm);
        eprintln!("  role: {}", role);
        eprintln!("  iteration: {}", iteration);
        eprintln!("  cwd: {}", working_dir.display());
        eprintln!(
            "  prompt (first 200 chars): {}...",
            &prompt_preview[..prompt_preview.len().min(200)]
        );
    }
}

/// Prints debug info about a review phase.
pub fn debug_review_phase(domain: &str, pr_number: u64, diff_len: usize) {
    if is_debug() {
        eprintln!("[CRUISE_DEBUG] Review Phase:");
        eprintln!("  domain: {}", domain);
        eprintln!("  pr_number: {}", pr_number);
        eprintln!("  diff_size: {} bytes", diff_len);
    }
}

/// Prints debug info about a git operation result.
pub fn debug_git_result(operation: &str, success: bool, output: &str) {
    if is_debug() {
        eprintln!("[CRUISE_DEBUG] Git Operation:");
        eprintln!("  operation: {}", operation);
        eprintln!("  success: {}", success);
        if !output.is_empty() {
            eprintln!("  output: {}", output);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_config_defaults_to_false() {
        // Clear env vars for this test
        std::env::remove_var("CRUISE_DEBUG");
        std::env::remove_var("CRUISE_FAIL_FAST");

        let config = DebugConfig::from_env();
        assert!(!config.debug_mode);
        assert!(!config.fail_fast);
    }

    #[test]
    fn test_config_has_both_enabled() {
        let config = DebugConfig::test_config();
        assert!(config.debug_mode);
        assert!(config.fail_fast);
    }
}
