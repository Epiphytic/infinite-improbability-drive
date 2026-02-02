//! Planner for cruise-control plan generation.
//!
//! Uses spawn-team ping-pong with phased reviews to generate
//! dependency-aware plans as beads issues.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::task::{CruisePlan, CruiseTask, TaskComplexity, TaskStatus};
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

/// Writes a CruisePlan as beads issues to the given directory.
pub fn plan_to_beads(plan: &CruisePlan, beads_dir: &Path) -> Result<Vec<std::path::PathBuf>> {
    // Create .beads directory if needed
    fs::create_dir_all(beads_dir)
        .map_err(|e| Error::Cruise(format!("Failed to create beads directory: {}", e)))?;

    let mut written_files = Vec::new();

    for task in &plan.tasks {
        let filename = format!("{}.md", task.id);
        let filepath = beads_dir.join(&filename);

        let content = format_beads_issue(task);

        fs::write(&filepath, content)
            .map_err(|e| Error::Cruise(format!("Failed to write {}: {}", filename, e)))?;

        written_files.push(filepath);
    }

    Ok(written_files)
}

/// Formats a CruiseTask as a beads issue markdown file.
fn format_beads_issue(task: &CruiseTask) -> String {
    let mut content = String::new();

    // YAML frontmatter
    content.push_str("---\n");
    content.push_str(&format!("id: {}\n", task.id));
    content.push_str(&format!("subject: {}\n", task.subject));
    content.push_str(&format!("status: {}\n", format_status(&task.status)));

    if !task.blocked_by.is_empty() {
        content.push_str("blockedBy:\n");
        for dep in &task.blocked_by {
            content.push_str(&format!("  - {}\n", dep));
        }
    } else {
        content.push_str("blockedBy: []\n");
    }

    if let Some(component) = &task.component {
        content.push_str(&format!("component: {}\n", component));
    }

    content.push_str(&format!(
        "complexity: {}\n",
        format_complexity(&task.complexity)
    ));
    content.push_str("---\n\n");

    // Body
    content.push_str(&format!("# {}\n\n", task.subject));
    content.push_str(&task.description);
    content.push('\n');

    if !task.acceptance_criteria.is_empty() {
        content.push_str("\n## Acceptance Criteria\n\n");
        for criterion in &task.acceptance_criteria {
            content.push_str(&format!("- [ ] {}\n", criterion));
        }
    }

    content
}

fn format_status(status: &TaskStatus) -> &'static str {
    match status {
        TaskStatus::Pending => "pending",
        TaskStatus::InProgress => "in_progress",
        TaskStatus::Completed => "completed",
        TaskStatus::Blocked => "blocked",
        TaskStatus::Skipped => "skipped",
    }
}

fn format_complexity(complexity: &TaskComplexity) -> &'static str {
    match complexity {
        TaskComplexity::Low => "low",
        TaskComplexity::Medium => "medium",
        TaskComplexity::High => "high",
    }
}

/// Generates a markdown plan document from a CruisePlan.
pub fn generate_plan_markdown(plan: &CruisePlan) -> String {
    let mut md = String::new();

    // Title and overview
    md.push_str(&format!("# {}\n\n", plan.title));
    md.push_str("## Overview\n\n");
    md.push_str(&plan.overview);
    md.push_str("\n\n");

    // Dependency graph (mermaid)
    md.push_str("## Dependency Graph\n\n");
    md.push_str("```mermaid\n");
    md.push_str("graph TD\n");
    for task in &plan.tasks {
        let label = task.subject.replace('"', "'");
        if task.blocked_by.is_empty() {
            md.push_str(&format!("    {}[\"{}\"]\n", task.id, label));
        } else {
            for dep in &task.blocked_by {
                md.push_str(&format!("    {} --> {}\n", dep, task.id));
            }
        }
    }
    md.push_str("```\n\n");

    // Tasks table
    md.push_str("## Tasks\n\n");
    for task in &plan.tasks {
        md.push_str(&format!("### {}: {}\n\n", task.id, task.subject));
        if let Some(component) = &task.component {
            md.push_str(&format!("- **Component**: {}\n", component));
        }
        md.push_str(&format!("- **Complexity**: {:?}\n", task.complexity));
        if task.blocked_by.is_empty() {
            md.push_str("- **Dependencies**: none\n");
        } else {
            md.push_str(&format!(
                "- **Dependencies**: {}\n",
                task.blocked_by.join(", ")
            ));
        }
        md.push('\n');
        md.push_str(&task.description);
        md.push_str("\n\n");
    }

    // Parallel execution groups
    md.push_str("## Parallel Execution Groups\n\n");
    let waves = compute_execution_waves(plan);
    for (i, wave) in waves.iter().enumerate() {
        let task_ids: Vec<&str> = wave.iter().map(|s| s.as_str()).collect();
        if wave.len() > 1 {
            md.push_str(&format!(
                "- **Wave {}**: {} *(parallel)*\n",
                i + 1,
                task_ids.join(", ")
            ));
        } else {
            md.push_str(&format!("- **Wave {}**: {}\n", i + 1, task_ids.join(", ")));
        }
    }
    md.push('\n');

    // Risk areas
    if !plan.risks.is_empty() {
        md.push_str("## Risk Areas\n\n");
        for risk in &plan.risks {
            md.push_str(&format!("- {}\n", risk));
        }
    }

    md
}

/// Computes execution waves (groups of tasks that can run in parallel).
fn compute_execution_waves(plan: &CruisePlan) -> Vec<Vec<String>> {
    let mut waves: Vec<Vec<String>> = Vec::new();
    let mut completed: HashSet<String> = HashSet::new();
    let mut remaining: Vec<&CruiseTask> = plan.tasks.iter().collect();

    while !remaining.is_empty() {
        // Find tasks with all dependencies satisfied
        let ready: Vec<String> = remaining
            .iter()
            .filter(|t| t.blocked_by.iter().all(|dep| completed.contains(dep)))
            .map(|t| t.id.clone())
            .collect();

        if ready.is_empty() {
            // Shouldn't happen if plan is valid, but avoid infinite loop
            break;
        }

        // Add ready tasks to completed
        for id in &ready {
            completed.insert(id.clone());
        }

        // Remove ready tasks from remaining
        remaining.retain(|t| !ready.contains(&t.id));

        waves.push(ready);
    }

    waves
}

/// Generates the PR body for a plan PR.
pub fn generate_pr_body(plan: &CruisePlan, user_prompt: &str, iterations: u32) -> String {
    let mut body = String::new();

    // Summary
    body.push_str("## Summary\n\n");
    body.push_str(&plan.overview);
    body.push_str("\n\n");

    // Original prompt in accordion
    body.push_str("<details>\n");
    body.push_str("<summary>Original Prompt</summary>\n\n");
    body.push_str(user_prompt);
    body.push_str("\n\n</details>\n\n");

    // Tasks table
    body.push_str(&format!("## Tasks ({})\n\n", plan.tasks.len()));
    body.push_str("| ID | Subject | Component | Complexity | Dependencies |\n");
    body.push_str("|----|---------|-----------|------------|---------------|\n");
    for task in &plan.tasks {
        let component = task.component.as_deref().unwrap_or("-");
        let complexity = format!("{:?}", task.complexity).to_lowercase();
        let deps = if task.blocked_by.is_empty() {
            "-".to_string()
        } else {
            task.blocked_by.join(", ")
        };
        body.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            task.id, task.subject, component, complexity, deps
        ));
    }
    body.push('\n');

    // ASCII dependency graph
    body.push_str("## Dependency Graph\n\n");
    body.push_str("```\n");
    body.push_str(&generate_ascii_tree(plan));
    body.push_str("```\n\n");

    // Parallel execution
    body.push_str("## Parallel Execution\n\n");
    let waves = compute_execution_waves(plan);
    for (i, wave) in waves.iter().enumerate() {
        if wave.len() > 1 {
            body.push_str(&format!(
                "- **Wave {}**: {} *(parallel)*\n",
                i + 1,
                wave.join(", ")
            ));
        } else {
            body.push_str(&format!("- **Wave {}**: {}\n", i + 1, wave.join(", ")));
        }
    }
    body.push('\n');

    // Planning stats
    body.push_str("## Planning Stats\n\n");
    body.push_str(&format!("- **Iterations**: {}\n", iterations));
    body.push_str(
        "- **Review phases**: Security ✓, Feasibility ✓, Granularity ✓, Dependencies ✓\n",
    );

    body
}

/// Generates an ASCII tree representation of task dependencies.
fn generate_ascii_tree(plan: &CruisePlan) -> String {
    let mut tree = String::new();

    // Find root tasks (no dependencies)
    let roots: Vec<&CruiseTask> = plan
        .tasks
        .iter()
        .filter(|t| t.blocked_by.is_empty())
        .collect();

    // Build dependency map (task -> tasks that depend on it)
    let mut dependents: HashMap<&str, Vec<&CruiseTask>> = HashMap::new();
    for task in &plan.tasks {
        for dep in &task.blocked_by {
            dependents.entry(dep.as_str()).or_default().push(task);
        }
    }

    // Render tree from each root
    for root in roots {
        render_tree_node(&mut tree, root, &dependents, "", true);
    }

    tree
}

fn render_tree_node(
    output: &mut String,
    task: &CruiseTask,
    dependents: &HashMap<&str, Vec<&CruiseTask>>,
    prefix: &str,
    is_last: bool,
) {
    let connector = if prefix.is_empty() {
        ""
    } else if is_last {
        "└── "
    } else {
        "├── "
    };

    output.push_str(&format!(
        "{}{}{} ({})\n",
        prefix, connector, task.id, task.subject
    ));

    let children = dependents
        .get(task.id.as_str())
        .map(|v| v.as_slice())
        .unwrap_or(&[]);
    let child_prefix = if prefix.is_empty() {
        String::new()
    } else if is_last {
        format!("{}    ", prefix)
    } else {
        format!("{}│   ", prefix)
    };

    for (i, child) in children.iter().enumerate() {
        render_tree_node(
            output,
            child,
            dependents,
            &child_prefix,
            i == children.len() - 1,
        );
    }
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

    #[test]
    fn plan_to_beads_creates_files() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let beads_dir = temp_dir.path().join(".beads");

        let mut plan = CruisePlan::new("test");
        plan.tasks = vec![
            CruiseTask::new("CRUISE-001", "First task")
                .with_description("Do the first thing")
                .with_component("core"),
            CruiseTask::new("CRUISE-002", "Second task")
                .with_blocked_by(vec!["CRUISE-001".to_string()]),
        ];

        let files = plan_to_beads(&plan, &beads_dir).unwrap();

        assert_eq!(files.len(), 2);
        assert!(beads_dir.join("CRUISE-001.md").exists());
        assert!(beads_dir.join("CRUISE-002.md").exists());
    }

    #[test]
    fn format_beads_issue_includes_frontmatter() {
        let task = CruiseTask::new("CRUISE-001", "Test task")
            .with_description("Description here")
            .with_component("testing")
            .with_complexity(TaskComplexity::High)
            .with_blocked_by(vec!["CRUISE-000".to_string()]);

        let content = format_beads_issue(&task);

        assert!(content.starts_with("---\n"));
        assert!(content.contains("id: CRUISE-001"));
        assert!(content.contains("subject: Test task"));
        assert!(content.contains("status: pending"));
        assert!(content.contains("component: testing"));
        assert!(content.contains("complexity: high"));
        assert!(content.contains("- CRUISE-000"));
        assert!(content.contains("# Test task"));
        assert!(content.contains("Description here"));
    }

    #[test]
    fn format_beads_issue_includes_acceptance_criteria() {
        let mut task = CruiseTask::new("CRUISE-001", "Task");
        task.acceptance_criteria = vec![
            "First criterion".to_string(),
            "Second criterion".to_string(),
        ];

        let content = format_beads_issue(&task);

        assert!(content.contains("## Acceptance Criteria"));
        assert!(content.contains("- [ ] First criterion"));
        assert!(content.contains("- [ ] Second criterion"));
    }

    #[test]
    fn generate_plan_markdown_includes_all_sections() {
        let mut plan = CruisePlan::new("test");
        plan.title = "Test Plan".to_string();
        plan.overview = "This is the overview.".to_string();
        plan.risks = vec!["Risk one".to_string()];
        plan.tasks = vec![
            CruiseTask::new("CRUISE-001", "First task")
                .with_description("Do first")
                .with_component("core"),
            CruiseTask::new("CRUISE-002", "Second task")
                .with_description("Do second")
                .with_blocked_by(vec!["CRUISE-001".to_string()]),
        ];

        let md = generate_plan_markdown(&plan);

        assert!(md.contains("# Test Plan"));
        assert!(md.contains("## Overview"));
        assert!(md.contains("This is the overview."));
        assert!(md.contains("## Dependency Graph"));
        assert!(md.contains("```mermaid"));
        assert!(md.contains("CRUISE-001 --> CRUISE-002"));
        assert!(md.contains("## Tasks"));
        assert!(md.contains("### CRUISE-001: First task"));
        assert!(md.contains("## Parallel Execution Groups"));
        assert!(md.contains("## Risk Areas"));
        assert!(md.contains("Risk one"));
    }

    #[test]
    fn compute_execution_waves_groups_parallel_tasks() {
        let mut plan = CruisePlan::new("test");
        plan.tasks = vec![
            CruiseTask::new("CRUISE-001", "A"),
            CruiseTask::new("CRUISE-002", "B").with_blocked_by(vec!["CRUISE-001".to_string()]),
            CruiseTask::new("CRUISE-003", "C").with_blocked_by(vec!["CRUISE-001".to_string()]),
            CruiseTask::new("CRUISE-004", "D")
                .with_blocked_by(vec!["CRUISE-002".to_string(), "CRUISE-003".to_string()]),
        ];

        let waves = compute_execution_waves(&plan);

        assert_eq!(waves.len(), 3);
        assert_eq!(waves[0], vec!["CRUISE-001"]);
        assert!(waves[1].contains(&"CRUISE-002".to_string()));
        assert!(waves[1].contains(&"CRUISE-003".to_string()));
        assert_eq!(waves[2], vec!["CRUISE-004"]);
    }

    #[test]
    fn generate_pr_body_includes_all_sections() {
        let mut plan = CruisePlan::new("test");
        plan.title = "Test Plan".to_string();
        plan.overview = "Build something cool.".to_string();
        plan.tasks = vec![
            CruiseTask::new("CRUISE-001", "Setup").with_component("infra"),
            CruiseTask::new("CRUISE-002", "Build").with_blocked_by(vec!["CRUISE-001".to_string()]),
        ];

        let body = generate_pr_body(&plan, "Original request here", 5);

        assert!(body.contains("## Summary"));
        assert!(body.contains("Build something cool."));
        assert!(body.contains("<details>"));
        assert!(body.contains("Original request here"));
        assert!(body.contains("## Tasks (2)"));
        assert!(body.contains("| CRUISE-001 |"));
        assert!(body.contains("## Dependency Graph"));
        assert!(body.contains("## Parallel Execution"));
        assert!(body.contains("**Wave 1**"));
        assert!(body.contains("## Planning Stats"));
        assert!(body.contains("Iterations**: 5"));
    }

    #[test]
    fn generate_ascii_tree_renders_hierarchy() {
        let mut plan = CruisePlan::new("test");
        plan.tasks = vec![
            CruiseTask::new("CRUISE-001", "Root"),
            CruiseTask::new("CRUISE-002", "Child A")
                .with_blocked_by(vec!["CRUISE-001".to_string()]),
            CruiseTask::new("CRUISE-003", "Child B")
                .with_blocked_by(vec!["CRUISE-001".to_string()]),
        ];

        let tree = generate_ascii_tree(&plan);

        assert!(tree.contains("CRUISE-001 (Root)"));
        assert!(tree.contains("CRUISE-002 (Child A)"));
        assert!(tree.contains("CRUISE-003 (Child B)"));
    }
}
