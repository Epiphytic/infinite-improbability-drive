//! GitHub PR approval polling.

use std::process::Command;
use std::time::{Duration, Instant};

use crate::error::{Error, Result};
use super::config::ApprovalConfig;

/// Status of a PR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrStatus {
    /// PR is open and awaiting review.
    Open,
    /// PR has been approved.
    Approved,
    /// PR has been merged.
    Merged,
    /// PR has been closed without merging.
    Closed,
}

/// Approval poller for GitHub PRs.
pub struct ApprovalPoller {
    config: ApprovalConfig,
}

impl ApprovalPoller {
    /// Creates a new approval poller with the given configuration.
    pub fn new(config: ApprovalConfig) -> Self {
        Self { config }
    }

    /// Creates a poller with default configuration.
    pub fn with_defaults() -> Self {
        Self::new(ApprovalConfig::default())
    }

    /// Checks the status of a PR using gh CLI.
    pub fn check_pr_status(&self, pr_url: &str) -> Result<PrStatus> {
        let output = Command::new("gh")
            .args(["pr", "view", pr_url, "--json", "state,reviewDecision"])
            .output()
            .map_err(|e| Error::GitHub(format!("failed to run gh: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::GitHub(format!("gh pr view failed: {}", stderr)));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: serde_json::Value = serde_json::from_str(&stdout)
            .map_err(|e| Error::GitHub(format!("failed to parse gh output: {}", e)))?;

        let state = json["state"].as_str().unwrap_or("UNKNOWN");
        let review_decision = json["reviewDecision"].as_str();

        match state {
            "MERGED" => Ok(PrStatus::Merged),
            "CLOSED" => Ok(PrStatus::Closed),
            "OPEN" => {
                if review_decision == Some("APPROVED") {
                    Ok(PrStatus::Approved)
                } else {
                    Ok(PrStatus::Open)
                }
            }
            _ => Ok(PrStatus::Open),
        }
    }

    /// Approves a PR using gh CLI (for test mode).
    pub fn approve_pr(&self, pr_url: &str) -> Result<()> {
        let output = Command::new("gh")
            .args(["pr", "review", pr_url, "--approve"])
            .output()
            .map_err(|e| Error::GitHub(format!("failed to run gh: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::GitHub(format!("gh pr review failed: {}", stderr)));
        }

        Ok(())
    }

    /// Merges a PR using gh CLI.
    pub fn merge_pr(&self, pr_url: &str) -> Result<()> {
        let output = Command::new("gh")
            .args(["pr", "merge", pr_url, "--merge", "--delete-branch"])
            .output()
            .map_err(|e| Error::GitHub(format!("failed to run gh: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::GitHub(format!("gh pr merge failed: {}", stderr)));
        }

        Ok(())
    }

    /// Calculates the next poll interval using exponential backoff.
    pub fn next_interval(&self, current: Duration) -> Duration {
        let next = Duration::from_secs_f64(current.as_secs_f64() * self.config.poll_backoff);
        next.min(self.config.poll_max)
    }

    /// Polls for PR approval with exponential backoff.
    /// Returns Ok(()) when approved, Err on timeout or other error.
    pub async fn poll_for_approval(&self, pr_url: &str, timeout: Duration) -> Result<()> {
        let start = Instant::now();
        let mut interval = self.config.poll_initial;

        loop {
            // Check if we've exceeded the timeout
            if start.elapsed() >= timeout {
                return Err(Error::ApprovalTimeout(timeout.as_secs()));
            }

            // Check PR status
            match self.check_pr_status(pr_url)? {
                PrStatus::Approved | PrStatus::Merged => return Ok(()),
                PrStatus::Closed => {
                    return Err(Error::GitHub("PR was closed without approval".to_string()));
                }
                PrStatus::Open => {
                    // Wait and try again
                    tokio::time::sleep(interval).await;
                    interval = self.next_interval(interval);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_interval_applies_backoff() {
        let config = ApprovalConfig {
            poll_initial: Duration::from_secs(60),
            poll_max: Duration::from_secs(1800),
            poll_backoff: 2.0,
        };
        let poller = ApprovalPoller::new(config);

        let next = poller.next_interval(Duration::from_secs(60));
        assert_eq!(next, Duration::from_secs(120));

        let next = poller.next_interval(Duration::from_secs(120));
        assert_eq!(next, Duration::from_secs(240));
    }

    #[test]
    fn next_interval_caps_at_max() {
        let config = ApprovalConfig {
            poll_initial: Duration::from_secs(60),
            poll_max: Duration::from_secs(300),
            poll_backoff: 2.0,
        };
        let poller = ApprovalPoller::new(config);

        let next = poller.next_interval(Duration::from_secs(200));
        assert_eq!(next, Duration::from_secs(300)); // Capped at max
    }

    #[test]
    fn pr_status_equality() {
        assert_eq!(PrStatus::Open, PrStatus::Open);
        assert_ne!(PrStatus::Open, PrStatus::Approved);
    }
}
