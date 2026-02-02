//! Planner for cruise-control plan generation.
//!
//! Uses spawn-team ping-pong with phased reviews to generate
//! dependency-aware plans as beads issues.

use serde::{Deserialize, Serialize};

use super::task::{CruisePlan, CruiseTask, TaskComplexity};
use crate::error::{Error, Result};

/// Review phase for plan iteration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewPhase {
    /// Review for security gaps: auth, secrets, injection, validation.
    Security,
    /// Review technical approach and stack appropriateness.
    TechnicalFeasibility,
    /// Review task sizing for parallelization.
    TaskGranularity,
    /// Review dependencies and parallelization opportunities.
    DependencyCompleteness,
    /// General polish and refinement.
    GeneralPolish,
}

impl ReviewPhase {
    /// Returns the phase for a given iteration (1-indexed).
    pub fn for_iteration(iteration: u32) -> Self {
        match iteration {
            1 => ReviewPhase::Security,
            2 => ReviewPhase::TechnicalFeasibility,
            3 => ReviewPhase::TaskGranularity,
            4 => ReviewPhase::DependencyCompleteness,
            _ => ReviewPhase::GeneralPolish,
        }
    }

    /// Returns the focus description for this phase.
    pub fn focus_description(&self) -> &'static str {
        match self {
            ReviewPhase::Security => {
                "Review for security gaps: authentication, secrets management, \
                 injection vulnerabilities, input validation. Suggest mitigations."
            }
            ReviewPhase::TechnicalFeasibility => {
                "Review technical approach: Is the tech stack appropriate? \
                 Are there better alternatives? Is the approach sound?"
            }
            ReviewPhase::TaskGranularity => {
                "Review task sizing: Are tasks too large to parallelize effectively? \
                 Too small to be meaningful? Suggest splits or merges."
            }
            ReviewPhase::DependencyCompleteness => {
                "Review dependencies: Are there missing dependency links? \
                 Tasks that could run in parallel but are serialized unnecessarily?"
            }
            ReviewPhase::GeneralPolish => {
                "Final review: Any remaining issues with the plan? \
                 Clarity, completeness, feasibility concerns?"
            }
        }
    }
}

/// Intermediate struct for parsing plan JSON.
#[derive(Debug, Deserialize)]
struct PlanJson {
    title: String,
    overview: String,
    tasks: Vec<TaskJson>,
    #[serde(default)]
    risks: Vec<String>,
}

/// Intermediate struct for parsing task JSON.
#[derive(Debug, Deserialize)]
struct TaskJson {
    id: String,
    subject: String,
    description: String,
    #[serde(default)]
    blocked_by: Vec<String>,
    #[serde(default)]
    component: Option<String>,
    #[serde(default = "default_complexity")]
    complexity: String,
    #[serde(default)]
    acceptance_criteria: Vec<String>,
}

fn default_complexity() -> String {
    "medium".to_string()
}

/// Parses plan JSON from LLM output.
///
/// Extracts JSON from the output (may be wrapped in markdown code blocks)
/// and parses it into a CruisePlan.
pub fn parse_plan_json(output: &str) -> Result<CruisePlan> {
    // Try to find JSON in the output
    let json_str =
        extract_json(output).ok_or_else(|| Error::Cruise("No JSON found in output".to_string()))?;

    // Parse the JSON
    let parsed: PlanJson = serde_json::from_str(json_str)
        .map_err(|e| Error::Cruise(format!("Failed to parse plan JSON: {}", e)))?;

    // Convert to CruisePlan
    let mut plan = CruisePlan::new("");
    plan.title = parsed.title;
    plan.overview = parsed.overview;
    plan.risks = parsed.risks;

    for task_json in parsed.tasks {
        let complexity = match task_json.complexity.to_lowercase().as_str() {
            "low" => TaskComplexity::Low,
            "high" => TaskComplexity::High,
            _ => TaskComplexity::Medium,
        };

        let mut task = CruiseTask::new(&task_json.id, &task_json.subject)
            .with_description(&task_json.description)
            .with_blocked_by(task_json.blocked_by)
            .with_complexity(complexity);

        task.component = task_json.component;
        task.acceptance_criteria = task_json.acceptance_criteria;

        plan.tasks.push(task);
    }

    Ok(plan)
}

/// Extracts JSON from output that may contain markdown code blocks.
fn extract_json(output: &str) -> Option<&str> {
    // Try to find JSON in code block
    if let Some(start) = output.find("```json") {
        let json_start = start + 7;
        if let Some(end) = output[json_start..].find("```") {
            return Some(output[json_start..json_start + end].trim());
        }
    }

    // Try to find raw JSON
    let json_start = output.find('{')?;
    let json_end = output.rfind('}')?;
    if json_start < json_end {
        Some(&output[json_start..=json_end])
    } else {
        None
    }
}

/// Validates a parsed plan for completeness and correctness.
pub fn validate_plan(plan: &CruisePlan) -> Result<()> {
    // Check for empty plan
    if plan.tasks.is_empty() {
        return Err(Error::Cruise("Plan produced no tasks".to_string()));
    }

    // Check for empty title
    if plan.title.trim().is_empty() {
        return Err(Error::Cruise("Plan has no title".to_string()));
    }

    // Check for dependency cycles
    if let Some(cycle) = plan.has_cycle() {
        return Err(Error::DependencyCycle(cycle));
    }

    // Validate each task
    for task in &plan.tasks {
        // Check ID format
        if !task.id.starts_with("CRUISE-") {
            return Err(Error::Cruise(format!(
                "Task ID '{}' must use CRUISE-XXX format",
                task.id
            )));
        }

        // Check for empty subject
        if task.subject.trim().is_empty() {
            return Err(Error::Cruise(format!("Task {} has no subject", task.id)));
        }

        // Check for unknown dependencies
        for dep in &task.blocked_by {
            if !plan.tasks.iter().any(|t| &t.id == dep) {
                return Err(Error::Cruise(format!(
                    "Task {} depends on unknown task {}",
                    task.id, dep
                )));
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_phase_for_iteration_maps_correctly() {
        assert_eq!(ReviewPhase::for_iteration(1), ReviewPhase::Security);
        assert_eq!(
            ReviewPhase::for_iteration(2),
            ReviewPhase::TechnicalFeasibility
        );
        assert_eq!(ReviewPhase::for_iteration(3), ReviewPhase::TaskGranularity);
        assert_eq!(
            ReviewPhase::for_iteration(4),
            ReviewPhase::DependencyCompleteness
        );
        assert_eq!(ReviewPhase::for_iteration(5), ReviewPhase::GeneralPolish);
        assert_eq!(ReviewPhase::for_iteration(10), ReviewPhase::GeneralPolish);
    }

    #[test]
    fn review_phase_focus_descriptions_not_empty() {
        assert!(!ReviewPhase::Security.focus_description().is_empty());
        assert!(!ReviewPhase::TechnicalFeasibility
            .focus_description()
            .is_empty());
        assert!(!ReviewPhase::TaskGranularity.focus_description().is_empty());
        assert!(!ReviewPhase::DependencyCompleteness
            .focus_description()
            .is_empty());
        assert!(!ReviewPhase::GeneralPolish.focus_description().is_empty());
    }

    #[test]
    fn review_phase_serializes_correctly() {
        assert_eq!(
            serde_json::to_string(&ReviewPhase::Security).unwrap(),
            "\"security\""
        );
        assert_eq!(
            serde_json::to_string(&ReviewPhase::TechnicalFeasibility).unwrap(),
            "\"technical_feasibility\""
        );
    }

    #[test]
    fn parse_plan_json_extracts_from_code_block() {
        let output = r#"
Here's my plan:
```json
{
    "title": "REST API",
    "overview": "Build a REST API",
    "tasks": [
        {
            "id": "CRUISE-001",
            "subject": "Setup project",
            "description": "Create initial structure",
            "blocked_by": [],
            "component": "infrastructure",
            "complexity": "low",
            "acceptance_criteria": ["Cargo.toml exists"]
        }
    ],
    "risks": ["Tight deadline"]
}
```
"#;

        let plan = parse_plan_json(output).unwrap();
        assert_eq!(plan.title, "REST API");
        assert_eq!(plan.tasks.len(), 1);
        assert_eq!(plan.tasks[0].id, "CRUISE-001");
        assert_eq!(plan.tasks[0].complexity, TaskComplexity::Low);
        assert_eq!(plan.risks, vec!["Tight deadline"]);
    }

    #[test]
    fn parse_plan_json_extracts_raw_json() {
        let output = r#"{"title": "Test", "overview": "Test plan", "tasks": []}"#;

        let plan = parse_plan_json(output).unwrap();
        assert_eq!(plan.title, "Test");
        assert!(plan.tasks.is_empty());
    }

    #[test]
    fn parse_plan_json_handles_missing_optional_fields() {
        let output = r#"{
            "title": "Minimal",
            "overview": "Minimal plan",
            "tasks": [
                {
                    "id": "CRUISE-001",
                    "subject": "Task",
                    "description": "Do something"
                }
            ]
        }"#;

        let plan = parse_plan_json(output).unwrap();
        assert_eq!(plan.tasks[0].complexity, TaskComplexity::Medium);
        assert!(plan.tasks[0].blocked_by.is_empty());
        assert!(plan.risks.is_empty());
    }

    #[test]
    fn parse_plan_json_returns_error_for_invalid_json() {
        let output = "not json at all";
        let result = parse_plan_json(output);
        assert!(result.is_err());
    }

    #[test]
    fn extract_json_finds_code_block() {
        let output = "text ```json\n{\"a\": 1}\n``` more";
        assert_eq!(extract_json(output), Some("{\"a\": 1}"));
    }

    #[test]
    fn extract_json_finds_raw_json() {
        let output = "prefix {\"a\": 1} suffix";
        assert_eq!(extract_json(output), Some("{\"a\": 1}"));
    }

    #[test]
    fn validate_plan_accepts_valid_plan() {
        let mut plan = CruisePlan::new("test");
        plan.title = "Valid Plan".to_string();
        plan.tasks = vec![
            CruiseTask::new("CRUISE-001", "First task"),
            CruiseTask::new("CRUISE-002", "Second task")
                .with_blocked_by(vec!["CRUISE-001".to_string()]),
        ];

        assert!(validate_plan(&plan).is_ok());
    }

    #[test]
    fn validate_plan_rejects_empty_plan() {
        let mut plan = CruisePlan::new("test");
        plan.title = "Empty".to_string();

        let result = validate_plan(&plan);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("no tasks"));
    }

    #[test]
    fn validate_plan_rejects_empty_title() {
        let mut plan = CruisePlan::new("test");
        plan.title = "   ".to_string();
        plan.tasks = vec![CruiseTask::new("CRUISE-001", "Task")];

        let result = validate_plan(&plan);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("no title"));
    }

    #[test]
    fn validate_plan_rejects_invalid_id_format() {
        let mut plan = CruisePlan::new("test");
        plan.title = "Test".to_string();
        plan.tasks = vec![CruiseTask::new("TASK-001", "Bad ID")];

        let result = validate_plan(&plan);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("CRUISE-XXX"));
    }

    #[test]
    fn validate_plan_rejects_unknown_dependency() {
        let mut plan = CruisePlan::new("test");
        plan.title = "Test".to_string();
        plan.tasks =
            vec![CruiseTask::new("CRUISE-001", "Task")
                .with_blocked_by(vec!["CRUISE-999".to_string()])];

        let result = validate_plan(&plan);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unknown task"));
    }

    #[test]
    fn validate_plan_rejects_cycle() {
        let mut plan = CruisePlan::new("test");
        plan.title = "Test".to_string();
        plan.tasks = vec![
            CruiseTask::new("CRUISE-001", "A").with_blocked_by(vec!["CRUISE-002".to_string()]),
            CruiseTask::new("CRUISE-002", "B").with_blocked_by(vec!["CRUISE-001".to_string()]),
        ];

        let result = validate_plan(&plan);
        assert!(result.is_err());
    }
}
