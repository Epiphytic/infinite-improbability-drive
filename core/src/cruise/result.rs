//! Result types for cruise-control phases.

use serde::{Deserialize, Serialize};
use std::time::Duration;

use super::task::TaskStatus;

/// Result of the planning phase.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanResult {
    /// Whether planning succeeded.
    pub success: bool,
    /// Number of ping-pong iterations.
    pub iterations: u32,
    /// Number of tasks in the plan.
    pub task_count: usize,
    /// PR URL for the plan.
    pub pr_url: Option<String>,
    /// Duration of planning phase.
    pub duration: Duration,
    /// Path to the generated plan file.
    pub plan_file: Option<String>,
    /// Error message if failed.
    pub error: Option<String>,
}

/// Result of a single task execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResult {
    /// Task ID.
    pub task_id: String,
    /// Final status.
    pub status: TaskStatus,
    /// PR URL if created.
    pub pr_url: Option<String>,
    /// Duration of task execution.
    pub duration: Duration,
    /// Error message if failed.
    pub error: Option<String>,
}

/// Result of the build phase.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildResult {
    /// Whether build succeeded.
    pub success: bool,
    /// Results for each task.
    pub task_results: Vec<TaskResult>,
    /// Maximum parallelism achieved.
    pub max_parallelism: usize,
    /// Total duration of build phase.
    pub duration: Duration,
    /// Count of completed tasks.
    pub completed_count: usize,
    /// Count of blocked tasks.
    pub blocked_count: usize,
}

impl BuildResult {
    /// Returns the success rate as a percentage.
    pub fn success_rate(&self) -> f64 {
        let total = self.task_results.len();
        if total == 0 {
            return 100.0;
        }
        let completed = self
            .task_results
            .iter()
            .filter(|r| r.status == TaskStatus::Completed)
            .count();
        (completed as f64 / total as f64) * 100.0
    }
}

/// Severity of an audit finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FindingSeverity {
    /// Critical issue that must be fixed.
    Critical,
    /// Warning that should be addressed.
    Warning,
    /// Informational note.
    Info,
}

/// A single audit finding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditFinding {
    /// Severity level.
    pub severity: FindingSeverity,
    /// Category (security, performance, quality).
    pub category: String,
    /// Description of the finding.
    pub description: String,
    /// File path if applicable.
    pub file: Option<String>,
    /// Line number if applicable.
    pub line: Option<u32>,
    /// Suggested fix.
    pub suggestion: Option<String>,
}

/// Result of a functional test.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionalTestResult {
    /// Test name/endpoint.
    pub name: String,
    /// HTTP method if applicable.
    pub method: Option<String>,
    /// Expected result.
    pub expected: String,
    /// Actual result.
    pub actual: String,
    /// Whether the test passed.
    pub passed: bool,
}

/// Plan adherence status for a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AdherenceStatus {
    /// Fully implemented as planned.
    Implemented,
    /// Partially implemented.
    Partial,
    /// Not implemented.
    Missing,
    /// Implemented differently than planned.
    Deviated,
}

/// Plan adherence check for a task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdherenceCheck {
    /// Task ID.
    pub task_id: String,
    /// Task subject.
    pub subject: String,
    /// Adherence status.
    pub status: AdherenceStatus,
    /// Notes about the implementation.
    pub notes: Option<String>,
}

/// Result of the validation phase.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResult {
    /// Whether validation passed at the configured level.
    pub success: bool,
    /// Functional test results.
    pub functional_tests: Vec<FunctionalTestResult>,
    /// Plan adherence checks.
    pub adherence_checks: Vec<AdherenceCheck>,
    /// Audit findings.
    pub findings: Vec<AuditFinding>,
    /// Overall quality score (0-10).
    pub quality_score: f64,
    /// Duration of validation phase.
    pub duration: Duration,
    /// Path to the audit report.
    pub report_file: Option<String>,
}

impl ValidationResult {
    /// Returns count of critical findings.
    pub fn critical_count(&self) -> usize {
        self.findings
            .iter()
            .filter(|f| f.severity == FindingSeverity::Critical)
            .count()
    }

    /// Returns count of passed functional tests.
    pub fn tests_passed(&self) -> usize {
        self.functional_tests.iter().filter(|t| t.passed).count()
    }

    /// Returns count of fully implemented tasks.
    pub fn fully_implemented(&self) -> usize {
        self.adherence_checks
            .iter()
            .filter(|c| c.status == AdherenceStatus::Implemented)
            .count()
    }
}

/// Overall result of a cruise-control run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CruiseResult {
    /// Whether the overall run succeeded.
    pub success: bool,
    /// Original prompt.
    pub prompt: String,
    /// Plan phase result.
    pub plan_result: Option<PlanResult>,
    /// Build phase result.
    pub build_result: Option<BuildResult>,
    /// Validation phase result.
    pub validation_result: Option<ValidationResult>,
    /// Total duration.
    pub total_duration: Duration,
    /// Summary message.
    pub summary: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_result_success_rate() {
        let result = BuildResult {
            success: true,
            task_results: vec![
                TaskResult {
                    task_id: "1".to_string(),
                    status: TaskStatus::Completed,
                    pr_url: None,
                    duration: Duration::from_secs(60),
                    error: None,
                },
                TaskResult {
                    task_id: "2".to_string(),
                    status: TaskStatus::Blocked,
                    pr_url: None,
                    duration: Duration::from_secs(30),
                    error: Some("failed".to_string()),
                },
            ],
            max_parallelism: 2,
            duration: Duration::from_secs(90),
            completed_count: 1,
            blocked_count: 1,
        };

        assert_eq!(result.success_rate(), 50.0);
    }

    #[test]
    fn build_result_success_rate_empty() {
        let result = BuildResult {
            success: true,
            task_results: vec![],
            max_parallelism: 0,
            duration: Duration::from_secs(0),
            completed_count: 0,
            blocked_count: 0,
        };

        assert_eq!(result.success_rate(), 100.0);
    }

    #[test]
    fn validation_result_critical_count() {
        let result = ValidationResult {
            success: false,
            functional_tests: vec![],
            adherence_checks: vec![],
            findings: vec![
                AuditFinding {
                    severity: FindingSeverity::Critical,
                    category: "security".to_string(),
                    description: "SQL injection".to_string(),
                    file: None,
                    line: None,
                    suggestion: None,
                },
                AuditFinding {
                    severity: FindingSeverity::Warning,
                    category: "performance".to_string(),
                    description: "N+1 query".to_string(),
                    file: None,
                    line: None,
                    suggestion: None,
                },
            ],
            quality_score: 5.0,
            duration: Duration::from_secs(300),
            report_file: None,
        };

        assert_eq!(result.critical_count(), 1);
    }

    #[test]
    fn validation_result_tests_passed() {
        let result = ValidationResult {
            success: true,
            functional_tests: vec![
                FunctionalTestResult {
                    name: "/api/health".to_string(),
                    method: Some("GET".to_string()),
                    expected: "200".to_string(),
                    actual: "200".to_string(),
                    passed: true,
                },
                FunctionalTestResult {
                    name: "/api/data".to_string(),
                    method: Some("GET".to_string()),
                    expected: "200".to_string(),
                    actual: "500".to_string(),
                    passed: false,
                },
            ],
            adherence_checks: vec![],
            findings: vec![],
            quality_score: 8.0,
            duration: Duration::from_secs(60),
            report_file: None,
        };

        assert_eq!(result.tests_passed(), 1);
    }

    #[test]
    fn finding_severity_serializes() {
        assert_eq!(
            serde_json::to_string(&FindingSeverity::Critical).unwrap(),
            "\"critical\""
        );
    }

    #[test]
    fn adherence_status_serializes() {
        assert_eq!(
            serde_json::to_string(&AdherenceStatus::Implemented).unwrap(),
            "\"implemented\""
        );
        assert_eq!(
            serde_json::to_string(&AdherenceStatus::Deviated).unwrap(),
            "\"deviated\""
        );
    }
}
