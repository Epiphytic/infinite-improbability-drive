//! Progress monitoring for spawned LLM instances.
//!
//! Tracks file changes, commits, output lines, and detects timeouts
//! based on activity or wall-clock time.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Information about a commit made during spawn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitInfo {
    /// The commit hash.
    pub hash: String,
    /// The commit message.
    pub message: String,
}

/// Timeout configuration for progress monitoring.
#[derive(Debug, Clone, Copy)]
pub struct TimeoutConfig {
    /// Maximum time without any activity before termination.
    pub idle_timeout: Duration,
    /// Maximum total wall-clock time before termination.
    pub total_timeout: Duration,
}

impl Default for TimeoutConfig {
    fn default() -> Self {
        Self {
            idle_timeout: Duration::from_secs(120),
            total_timeout: Duration::from_secs(1800),
        }
    }
}

/// Reason for a timeout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeoutReason {
    /// No activity for too long.
    Idle,
    /// Total time exceeded.
    Total,
}

/// Tracks progress of a spawned LLM instance.
pub struct ProgressMonitor {
    /// Files that have been read.
    files_read: HashSet<PathBuf>,
    /// Files that have been written.
    files_written: HashSet<PathBuf>,
    /// Commits made during the spawn.
    commits: Vec<CommitInfo>,
    /// Number of output lines captured.
    output_lines: usize,
    /// Time of last activity.
    last_activity: Instant,
    /// Time when monitoring started.
    start_time: Instant,
    /// Timeout configuration.
    timeout_config: TimeoutConfig,
}

impl ProgressMonitor {
    /// Creates a new progress monitor with the given timeout configuration.
    pub fn new(timeout_config: TimeoutConfig) -> Self {
        let now = Instant::now();
        Self {
            files_read: HashSet::new(),
            files_written: HashSet::new(),
            commits: Vec::new(),
            output_lines: 0,
            last_activity: now,
            start_time: now,
            timeout_config,
        }
    }

    /// Records that a file was read.
    pub fn record_file_read(&mut self, path: PathBuf) {
        self.files_read.insert(path);
        self.last_activity = Instant::now();
    }

    /// Records that a file was written.
    pub fn record_file_write(&mut self, path: PathBuf) {
        self.files_written.insert(path);
        self.last_activity = Instant::now();
    }

    /// Records a commit.
    pub fn record_commit(&mut self, info: CommitInfo) {
        self.commits.push(info);
        self.last_activity = Instant::now();
    }

    /// Records output lines.
    pub fn record_output(&mut self, lines: usize) {
        self.output_lines += lines;
        self.last_activity = Instant::now();
    }

    /// Touches the activity timer without recording any specific event.
    pub fn touch(&mut self) {
        self.last_activity = Instant::now();
    }

    /// Returns files that have been read.
    pub fn files_read(&self) -> &HashSet<PathBuf> {
        &self.files_read
    }

    /// Returns files that have been written.
    pub fn files_written(&self) -> &HashSet<PathBuf> {
        &self.files_written
    }

    /// Returns commits made.
    pub fn commits(&self) -> &[CommitInfo] {
        &self.commits
    }

    /// Returns number of output lines.
    pub fn output_lines(&self) -> usize {
        self.output_lines
    }

    /// Returns time since last activity.
    pub fn idle_duration(&self) -> Duration {
        self.last_activity.elapsed()
    }

    /// Returns total elapsed time.
    pub fn total_duration(&self) -> Duration {
        self.start_time.elapsed()
    }

    /// Checks if a timeout has occurred.
    ///
    /// Returns `Some(reason)` if a timeout has occurred, `None` otherwise.
    pub fn check_timeout(&self) -> Option<TimeoutReason> {
        if self.idle_duration() >= self.timeout_config.idle_timeout {
            Some(TimeoutReason::Idle)
        } else if self.total_duration() >= self.timeout_config.total_timeout {
            Some(TimeoutReason::Total)
        } else {
            None
        }
    }

    /// Returns whether there has been any activity.
    pub fn has_activity(&self) -> bool {
        !self.files_read.is_empty()
            || !self.files_written.is_empty()
            || !self.commits.is_empty()
            || self.output_lines > 0
    }
}

/// Summary of progress state for serialization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressSummary {
    pub files_read: Vec<PathBuf>,
    pub files_written: Vec<PathBuf>,
    pub commits: Vec<CommitInfo>,
    pub output_lines: usize,
    pub total_duration_secs: f64,
}

impl From<&ProgressMonitor> for ProgressSummary {
    fn from(monitor: &ProgressMonitor) -> Self {
        Self {
            files_read: monitor.files_read.iter().cloned().collect(),
            files_written: monitor.files_written.iter().cloned().collect(),
            commits: monitor.commits.clone(),
            output_lines: monitor.output_lines,
            total_duration_secs: monitor.total_duration().as_secs_f64(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn progress_monitor_starts_empty() {
        let monitor = ProgressMonitor::new(TimeoutConfig::default());

        assert!(monitor.files_read().is_empty());
        assert!(monitor.files_written().is_empty());
        assert!(monitor.commits().is_empty());
        assert_eq!(monitor.output_lines(), 0);
        assert!(!monitor.has_activity());
    }

    #[test]
    fn progress_monitor_tracks_file_reads() {
        let mut monitor = ProgressMonitor::new(TimeoutConfig::default());

        monitor.record_file_read(PathBuf::from("src/main.rs"));
        monitor.record_file_read(PathBuf::from("src/lib.rs"));
        monitor.record_file_read(PathBuf::from("src/main.rs")); // duplicate

        assert_eq!(monitor.files_read().len(), 2);
        assert!(monitor.files_read().contains(&PathBuf::from("src/main.rs")));
        assert!(monitor.files_read().contains(&PathBuf::from("src/lib.rs")));
        assert!(monitor.has_activity());
    }

    #[test]
    fn progress_monitor_tracks_file_writes() {
        let mut monitor = ProgressMonitor::new(TimeoutConfig::default());

        monitor.record_file_write(PathBuf::from("src/new.rs"));

        assert_eq!(monitor.files_written().len(), 1);
        assert!(monitor
            .files_written()
            .contains(&PathBuf::from("src/new.rs")));
        assert!(monitor.has_activity());
    }

    #[test]
    fn progress_monitor_tracks_commits() {
        let mut monitor = ProgressMonitor::new(TimeoutConfig::default());

        monitor.record_commit(CommitInfo {
            hash: "abc123".to_string(),
            message: "Fix bug".to_string(),
        });

        assert_eq!(monitor.commits().len(), 1);
        assert_eq!(monitor.commits()[0].hash, "abc123");
        assert!(monitor.has_activity());
    }

    #[test]
    fn progress_monitor_tracks_output_lines() {
        let mut monitor = ProgressMonitor::new(TimeoutConfig::default());

        monitor.record_output(10);
        monitor.record_output(5);

        assert_eq!(monitor.output_lines(), 15);
        assert!(monitor.has_activity());
    }

    #[test]
    fn progress_monitor_detects_idle_timeout() {
        let config = TimeoutConfig {
            idle_timeout: Duration::from_millis(50),
            total_timeout: Duration::from_secs(3600),
        };
        let monitor = ProgressMonitor::new(config);

        // Initially no timeout
        assert_eq!(monitor.check_timeout(), None);

        // Wait for idle timeout
        thread::sleep(Duration::from_millis(60));

        assert_eq!(monitor.check_timeout(), Some(TimeoutReason::Idle));
    }

    #[test]
    fn progress_monitor_detects_total_timeout() {
        let config = TimeoutConfig {
            idle_timeout: Duration::from_secs(3600),
            total_timeout: Duration::from_millis(50),
        };
        let monitor = ProgressMonitor::new(config);

        // Initially no timeout
        assert_eq!(monitor.check_timeout(), None);

        // Wait for total timeout
        thread::sleep(Duration::from_millis(60));

        assert_eq!(monitor.check_timeout(), Some(TimeoutReason::Total));
    }

    #[test]
    fn progress_monitor_activity_resets_idle_timer() {
        let config = TimeoutConfig {
            idle_timeout: Duration::from_millis(100),
            total_timeout: Duration::from_secs(3600),
        };
        let mut monitor = ProgressMonitor::new(config);

        // Wait a bit but not enough for timeout
        thread::sleep(Duration::from_millis(60));

        // Record activity - this resets the idle timer
        monitor.touch();

        // Wait same amount again
        thread::sleep(Duration::from_millis(60));

        // Should not timeout because we reset the timer
        assert_eq!(monitor.check_timeout(), None);
    }

    #[test]
    fn progress_summary_captures_state() {
        let mut monitor = ProgressMonitor::new(TimeoutConfig::default());

        monitor.record_file_read(PathBuf::from("a.rs"));
        monitor.record_file_write(PathBuf::from("b.rs"));
        monitor.record_commit(CommitInfo {
            hash: "def456".to_string(),
            message: "Add feature".to_string(),
        });
        monitor.record_output(42);

        let summary = ProgressSummary::from(&monitor);

        assert_eq!(summary.files_read.len(), 1);
        assert_eq!(summary.files_written.len(), 1);
        assert_eq!(summary.commits.len(), 1);
        assert_eq!(summary.output_lines, 42);
        assert!(summary.total_duration_secs >= 0.0);
    }
}
