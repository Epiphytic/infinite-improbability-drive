# Cruise-Control Planner Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Implement the Planner module that uses spawn-team ping-pong with phased reviews to generate dependency-aware plans as beads issues.

**Architecture:** Planner orchestrates 5 ping-pong iterations with phase-specific reviews (Security → Feasibility → Granularity → Dependencies → Polish). Primary LLM outputs JSON, validated and written as beads issues, with markdown derived view. PR created for approval.

**Tech Stack:** Rust, tokio async, serde for JSON parsing, existing spawn-team infrastructure, gh CLI for PR creation.

**Design Document:** `docs/plans/2026-02-01-cruise-planner-design.md`

---

## Task 1: Add ReviewPhase Enum

**Files:**
- Create: `core/src/cruise/planner.rs`
- Modify: `core/src/cruise/mod.rs`

**Step 1: Create planner.rs with ReviewPhase enum and tests**

Create `core/src/cruise/planner.rs`:

```rust
//! Planner for cruise-control plan generation.
//!
//! Uses spawn-team ping-pong with phased reviews to generate
//! dependency-aware plans as beads issues.

use serde::{Deserialize, Serialize};

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_phase_for_iteration_maps_correctly() {
        assert_eq!(ReviewPhase::for_iteration(1), ReviewPhase::Security);
        assert_eq!(ReviewPhase::for_iteration(2), ReviewPhase::TechnicalFeasibility);
        assert_eq!(ReviewPhase::for_iteration(3), ReviewPhase::TaskGranularity);
        assert_eq!(ReviewPhase::for_iteration(4), ReviewPhase::DependencyCompleteness);
        assert_eq!(ReviewPhase::for_iteration(5), ReviewPhase::GeneralPolish);
        assert_eq!(ReviewPhase::for_iteration(10), ReviewPhase::GeneralPolish);
    }

    #[test]
    fn review_phase_focus_descriptions_not_empty() {
        assert!(!ReviewPhase::Security.focus_description().is_empty());
        assert!(!ReviewPhase::TechnicalFeasibility.focus_description().is_empty());
        assert!(!ReviewPhase::TaskGranularity.focus_description().is_empty());
        assert!(!ReviewPhase::DependencyCompleteness.focus_description().is_empty());
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
}
```

**Step 2: Add planner module to mod.rs**

Add to `core/src/cruise/mod.rs`:

```rust
pub mod planner;
```

And add export:

```rust
pub use planner::ReviewPhase;
```

**Step 3: Run tests**

Run: `cd core && cargo test cruise::planner`
Expected: 3 tests pass

**Step 4: Commit**

```bash
git add core/src/cruise/planner.rs core/src/cruise/mod.rs
git commit -m "feat(cruise): add ReviewPhase enum for planner iterations"
```

---

## Task 2: Add Prompt Builders

**Files:**
- Create: `core/src/cruise/prompts.rs`
- Modify: `core/src/cruise/mod.rs`

**Step 1: Create prompts.rs with PlanPromptBuilder**

Create `core/src/cruise/prompts.rs`:

```rust
//! Prompt builders for cruise-control planning.

use super::planner::ReviewPhase;

/// Builder for creating primary LLM plan generation prompts.
pub struct PlanPromptBuilder {
    user_prompt: String,
    previous_plan: Option<String>,
    review_feedback: Option<String>,
}

impl PlanPromptBuilder {
    /// Creates a new plan prompt builder.
    pub fn new(user_prompt: impl Into<String>) -> Self {
        Self {
            user_prompt: user_prompt.into(),
            previous_plan: None,
            review_feedback: None,
        }
    }

    /// Sets the previous plan JSON for refinement.
    pub fn with_previous_plan(mut self, plan: impl Into<String>) -> Self {
        self.previous_plan = Some(plan.into());
        self
    }

    /// Sets the review feedback to address.
    pub fn with_review_feedback(mut self, feedback: impl Into<String>) -> Self {
        self.review_feedback = Some(feedback.into());
        self
    }

    /// Builds the prompt.
    pub fn build(&self) -> String {
        let mut prompt = String::new();

        if self.previous_plan.is_some() {
            prompt.push_str("## Plan Refinement Request\n\n");
            prompt.push_str("Refine the plan based on the review feedback.\n\n");
        } else {
            prompt.push_str("## Plan Generation Request\n\n");
            prompt.push_str("Create a dependency-aware implementation plan.\n\n");
        }

        prompt.push_str("### Original Request\n\n");
        prompt.push_str(&self.user_prompt);
        prompt.push_str("\n\n");

        if let Some(plan) = &self.previous_plan {
            prompt.push_str("### Current Plan\n\n");
            prompt.push_str("```json\n");
            prompt.push_str(plan);
            prompt.push_str("\n```\n\n");
        }

        if let Some(feedback) = &self.review_feedback {
            prompt.push_str("### Review Feedback to Address\n\n");
            prompt.push_str(feedback);
            prompt.push_str("\n\n");
        }

        prompt.push_str("### Output Format\n\n");
        prompt.push_str("Respond with a JSON object:\n");
        prompt.push_str("```json\n");
        prompt.push_str("{\n");
        prompt.push_str("  \"title\": \"Plan title\",\n");
        prompt.push_str("  \"overview\": \"2-3 sentence overview\",\n");
        prompt.push_str("  \"tasks\": [\n");
        prompt.push_str("    {\n");
        prompt.push_str("      \"id\": \"CRUISE-001\",\n");
        prompt.push_str("      \"subject\": \"Task title\",\n");
        prompt.push_str("      \"description\": \"Detailed description\",\n");
        prompt.push_str("      \"blocked_by\": [],\n");
        prompt.push_str("      \"component\": \"component-name\",\n");
        prompt.push_str("      \"complexity\": \"low|medium|high\",\n");
        prompt.push_str("      \"acceptance_criteria\": [\"criterion 1\", \"criterion 2\"]\n");
        prompt.push_str("    }\n");
        prompt.push_str("  ],\n");
        prompt.push_str("  \"risks\": [\"risk 1\", \"risk 2\"]\n");
        prompt.push_str("}\n");
        prompt.push_str("```\n");

        prompt
    }
}

/// Builder for creating plan review prompts.
pub struct PlanReviewPromptBuilder {
    plan_json: String,
    phase: ReviewPhase,
}

impl PlanReviewPromptBuilder {
    /// Creates a new plan review prompt builder.
    pub fn new(plan_json: impl Into<String>, phase: ReviewPhase) -> Self {
        Self {
            plan_json: plan_json.into(),
            phase,
        }
    }

    /// Builds the review prompt.
    pub fn build(&self) -> String {
        let mut prompt = String::new();

        prompt.push_str("## Plan Review Request\n\n");
        prompt.push_str("Review the following implementation plan.\n\n");

        prompt.push_str("### Review Focus\n\n");
        prompt.push_str(self.phase.focus_description());
        prompt.push_str("\n\n");

        prompt.push_str("### Plan to Review\n\n");
        prompt.push_str("```json\n");
        prompt.push_str(&self.plan_json);
        prompt.push_str("\n```\n\n");

        prompt.push_str("### Response Format\n\n");
        prompt.push_str("Respond with a JSON object:\n");
        prompt.push_str("```json\n");
        prompt.push_str("{\n");
        prompt.push_str("  \"verdict\": \"approved\" | \"needs_changes\",\n");
        prompt.push_str("  \"suggestions\": [\n");
        prompt.push_str("    {\n");
        prompt.push_str("      \"category\": \"security|feasibility|granularity|dependency\",\n");
        prompt.push_str("      \"task_id\": \"CRUISE-001 or null for general\",\n");
        prompt.push_str("      \"issue\": \"Description of the issue\",\n");
        prompt.push_str("      \"suggestion\": \"How to address it\"\n");
        prompt.push_str("    }\n");
        prompt.push_str("  ],\n");
        prompt.push_str("  \"summary\": \"Brief summary of review\"\n");
        prompt.push_str("}\n");
        prompt.push_str("```\n");

        prompt
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_prompt_builder_initial_prompt() {
        let prompt = PlanPromptBuilder::new("Build a REST API").build();

        assert!(prompt.contains("Plan Generation Request"));
        assert!(prompt.contains("Build a REST API"));
        assert!(prompt.contains("CRUISE-001"));
        assert!(!prompt.contains("Current Plan"));
    }

    #[test]
    fn plan_prompt_builder_refinement_prompt() {
        let prompt = PlanPromptBuilder::new("Build a REST API")
            .with_previous_plan(r#"{"title": "API Plan"}"#)
            .with_review_feedback("Add error handling task")
            .build();

        assert!(prompt.contains("Plan Refinement Request"));
        assert!(prompt.contains("Current Plan"));
        assert!(prompt.contains("API Plan"));
        assert!(prompt.contains("Add error handling task"));
    }

    #[test]
    fn plan_review_prompt_builder_includes_phase_focus() {
        let prompt = PlanReviewPromptBuilder::new(
            r#"{"title": "Test"}"#,
            ReviewPhase::Security,
        ).build();

        assert!(prompt.contains("Plan Review Request"));
        assert!(prompt.contains("security gaps"));
        assert!(prompt.contains("authentication"));
    }

    #[test]
    fn plan_review_prompt_builder_includes_plan() {
        let prompt = PlanReviewPromptBuilder::new(
            r#"{"title": "My Plan", "tasks": []}"#,
            ReviewPhase::DependencyCompleteness,
        ).build();

        assert!(prompt.contains("My Plan"));
        assert!(prompt.contains("dependencies"));
        assert!(prompt.contains("parallel"));
    }
}
```

**Step 2: Add prompts module to mod.rs**

Add to `core/src/cruise/mod.rs`:

```rust
pub mod prompts;
```

And add exports:

```rust
pub use prompts::{PlanPromptBuilder, PlanReviewPromptBuilder};
```

**Step 3: Run tests**

Run: `cd core && cargo test cruise::prompts`
Expected: 4 tests pass

**Step 4: Commit**

```bash
git add core/src/cruise/prompts.rs core/src/cruise/mod.rs
git commit -m "feat(cruise): add prompt builders for planner"
```

---

## Task 3: Add Plan JSON Parsing

**Files:**
- Modify: `core/src/cruise/planner.rs`

**Step 1: Add parse_plan_json function with tests**

Add to `core/src/cruise/planner.rs`:

```rust
use crate::error::{Error, Result};
use super::task::{CruisePlan, CruiseTask, TaskComplexity, TaskStatus};

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
    let json_str = extract_json(output)
        .ok_or_else(|| Error::Cruise("No JSON found in output".to_string()))?;

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
```

**Step 2: Add tests for JSON parsing**

Add to the tests module in `core/src/cruise/planner.rs`:

```rust
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
```

**Step 3: Run tests**

Run: `cd core && cargo test cruise::planner`
Expected: 9 tests pass (3 original + 6 new)

**Step 4: Commit**

```bash
git add core/src/cruise/planner.rs
git commit -m "feat(cruise): add plan JSON parsing"
```

---

## Task 4: Add Plan Validation

**Files:**
- Modify: `core/src/cruise/planner.rs`

**Step 1: Add validate_plan function**

Add to `core/src/cruise/planner.rs`:

```rust
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
            return Err(Error::Cruise(format!(
                "Task {} has no subject",
                task.id
            )));
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
```

**Step 2: Add validation tests**

Add to the tests module:

```rust
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
        plan.tasks = vec![
            CruiseTask::new("CRUISE-001", "Task")
                .with_blocked_by(vec!["CRUISE-999".to_string()]),
        ];

        let result = validate_plan(&plan);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unknown task"));
    }

    #[test]
    fn validate_plan_rejects_cycle() {
        let mut plan = CruisePlan::new("test");
        plan.title = "Test".to_string();
        plan.tasks = vec![
            CruiseTask::new("CRUISE-001", "A")
                .with_blocked_by(vec!["CRUISE-002".to_string()]),
            CruiseTask::new("CRUISE-002", "B")
                .with_blocked_by(vec!["CRUISE-001".to_string()]),
        ];

        let result = validate_plan(&plan);
        assert!(result.is_err());
    }
```

**Step 3: Run tests**

Run: `cd core && cargo test cruise::planner`
Expected: 15 tests pass

**Step 4: Commit**

```bash
git add core/src/cruise/planner.rs
git commit -m "feat(cruise): add plan validation"
```

---

## Task 5: Add Beads Writer

**Files:**
- Modify: `core/src/cruise/planner.rs`

**Step 1: Add plan_to_beads function**

Add to `core/src/cruise/planner.rs`:

```rust
use std::fs;
use std::path::Path;

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

    content.push_str(&format!("complexity: {}\n", format_complexity(&task.complexity)));
    content.push_str("---\n\n");

    // Body
    content.push_str(&format!("# {}\n\n", task.subject));
    content.push_str(&task.description);
    content.push_str("\n");

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
```

**Step 2: Add beads writer tests**

Add to the tests module:

```rust
    use tempfile::TempDir;

    #[test]
    fn plan_to_beads_creates_files() {
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
```

**Step 3: Run tests**

Run: `cd core && cargo test cruise::planner`
Expected: 18 tests pass

**Step 4: Commit**

```bash
git add core/src/cruise/planner.rs
git commit -m "feat(cruise): add beads issue writer"
```

---

## Task 6: Add Markdown Generator

**Files:**
- Modify: `core/src/cruise/planner.rs`

**Step 1: Add generate_plan_markdown function**

Add to `core/src/cruise/planner.rs`:

```rust
use std::collections::{HashMap, HashSet};

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
            md.push_str(&format!("- **Dependencies**: {}\n", task.blocked_by.join(", ")));
        }
        md.push_str("\n");
        md.push_str(&task.description);
        md.push_str("\n\n");
    }

    // Parallel execution groups
    md.push_str("## Parallel Execution Groups\n\n");
    let waves = compute_execution_waves(plan);
    for (i, wave) in waves.iter().enumerate() {
        let task_ids: Vec<&str> = wave.iter().map(|s| s.as_str()).collect();
        if wave.len() > 1 {
            md.push_str(&format!("- **Wave {}**: {} *(parallel)*\n", i + 1, task_ids.join(", ")));
        } else {
            md.push_str(&format!("- **Wave {}**: {}\n", i + 1, task_ids.join(", ")));
        }
    }
    md.push_str("\n");

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
```

**Step 2: Add markdown generator tests**

Add to the tests module:

```rust
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
            CruiseTask::new("CRUISE-002", "B")
                .with_blocked_by(vec!["CRUISE-001".to_string()]),
            CruiseTask::new("CRUISE-003", "C")
                .with_blocked_by(vec!["CRUISE-001".to_string()]),
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
```

**Step 3: Run tests**

Run: `cd core && cargo test cruise::planner`
Expected: 20 tests pass

**Step 4: Commit**

```bash
git add core/src/cruise/planner.rs
git commit -m "feat(cruise): add plan markdown generator"
```

---

## Task 7: Add PR Body Generator

**Files:**
- Modify: `core/src/cruise/planner.rs`

**Step 1: Add generate_pr_body function**

Add to `core/src/cruise/planner.rs`:

```rust
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
    body.push_str("\n");

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
            body.push_str(&format!("- **Wave {}**: {} *(parallel)*\n", i + 1, wave.join(", ")));
        } else {
            body.push_str(&format!("- **Wave {}**: {}\n", i + 1, wave.join(", ")));
        }
    }
    body.push_str("\n");

    // Planning stats
    body.push_str("## Planning Stats\n\n");
    body.push_str(&format!("- **Iterations**: {}\n", iterations));
    body.push_str("- **Review phases**: Security ✓, Feasibility ✓, Granularity ✓, Dependencies ✓\n");

    body
}

/// Generates an ASCII tree representation of task dependencies.
fn generate_ascii_tree(plan: &CruisePlan) -> String {
    let mut tree = String::new();

    // Find root tasks (no dependencies)
    let roots: Vec<&CruiseTask> = plan.tasks
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

    output.push_str(&format!("{}{}{} ({})\n", prefix, connector, task.id, task.subject));

    let children = dependents.get(task.id.as_str()).map(|v| v.as_slice()).unwrap_or(&[]);
    let child_prefix = if prefix.is_empty() {
        "".to_string()
    } else if is_last {
        format!("{}    ", prefix)
    } else {
        format!("{}│   ", prefix)
    };

    for (i, child) in children.iter().enumerate() {
        render_tree_node(output, child, dependents, &child_prefix, i == children.len() - 1);
    }
}
```

**Step 2: Add PR body tests**

Add to the tests module:

```rust
    #[test]
    fn generate_pr_body_includes_all_sections() {
        let mut plan = CruisePlan::new("test");
        plan.title = "Test Plan".to_string();
        plan.overview = "Build something cool.".to_string();
        plan.tasks = vec![
            CruiseTask::new("CRUISE-001", "Setup")
                .with_component("infra"),
            CruiseTask::new("CRUISE-002", "Build")
                .with_blocked_by(vec!["CRUISE-001".to_string()]),
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
```

**Step 3: Run tests**

Run: `cd core && cargo test cruise::planner`
Expected: 22 tests pass

**Step 4: Commit**

```bash
git add core/src/cruise/planner.rs
git commit -m "feat(cruise): add PR body generator"
```

---

## Task 8: Add Planner Struct

**Files:**
- Modify: `core/src/cruise/planner.rs`
- Modify: `core/src/cruise/mod.rs`
- Modify: `core/src/lib.rs`

**Step 1: Add Planner struct**

Add to `core/src/cruise/planner.rs`:

```rust
use std::time::Instant;
use super::config::PlanningConfig;
use super::result::PlanResult;

/// Planner for cruise-control plan generation.
pub struct Planner {
    config: PlanningConfig,
}

impl Planner {
    /// Creates a new planner with the given configuration.
    pub fn new(config: PlanningConfig) -> Self {
        Self { config }
    }

    /// Creates a planner with default configuration.
    pub fn with_defaults() -> Self {
        Self::new(PlanningConfig::default())
    }

    /// Returns the planning configuration.
    pub fn config(&self) -> &PlanningConfig {
        &self.config
    }

    /// Runs planning in dry-run mode (no PR creation).
    ///
    /// This is useful for testing the planning logic without
    /// creating actual PRs or writing files.
    pub fn plan_dry_run(&self, prompt: &str) -> Result<CruisePlan> {
        // For now, just validate that we can parse a mock response
        // Full implementation will integrate with spawn-team
        let _ = prompt;
        Err(Error::Cruise("Planner not yet integrated with spawn-team".to_string()))
    }

    /// Runs the full planning phase.
    ///
    /// This orchestrates spawn-team ping-pong iterations,
    /// validates the result, writes beads issues, generates
    /// markdown, and creates a PR for approval.
    pub async fn plan(&self, prompt: &str, work_dir: &Path) -> Result<PlanResult> {
        let start = Instant::now();

        // TODO: Integrate with spawn-team ping-pong
        // For now, return a placeholder result
        let _ = prompt;
        let _ = work_dir;

        Ok(PlanResult {
            success: false,
            iterations: 0,
            task_count: 0,
            pr_url: None,
            duration: start.elapsed(),
            plan_file: None,
            error: Some("Planner not yet integrated with spawn-team".to_string()),
        })
    }
}
```

**Step 2: Update mod.rs exports**

Update `core/src/cruise/mod.rs` to export Planner:

```rust
pub use planner::{
    Planner, ReviewPhase, generate_plan_markdown, generate_pr_body,
    parse_plan_json, plan_to_beads, validate_plan,
};
```

**Step 3: Update lib.rs exports**

Add to `core/src/lib.rs`:

```rust
pub use cruise::{
    // ... existing exports ...
    Planner, ReviewPhase, generate_plan_markdown, generate_pr_body,
    parse_plan_json, plan_to_beads, validate_plan,
};
```

**Step 4: Add Planner struct tests**

Add to the tests module:

```rust
    #[test]
    fn planner_can_be_created() {
        let planner = Planner::with_defaults();
        assert_eq!(planner.config().ping_pong_iterations, 5);
    }

    #[test]
    fn planner_dry_run_returns_error_until_integrated() {
        let planner = Planner::with_defaults();
        let result = planner.plan_dry_run("test prompt");
        assert!(result.is_err());
    }
```

**Step 5: Run tests**

Run: `cd core && cargo test cruise::planner`
Expected: 24 tests pass

**Step 6: Commit**

```bash
git add core/src/cruise/planner.rs core/src/cruise/mod.rs core/src/lib.rs
git commit -m "feat(cruise): add Planner struct"
```

---

## Summary

This implementation plan covers:

1. **Task 1**: ReviewPhase enum for iteration phases
2. **Task 2**: Prompt builders for primary and reviewer LLMs
3. **Task 3**: Plan JSON parsing from LLM output
4. **Task 4**: Plan validation (cycles, fields, format)
5. **Task 5**: Beads issue writer
6. **Task 6**: Markdown plan generator
7. **Task 7**: PR body generator
8. **Task 8**: Planner struct (shell for spawn-team integration)

**Total: 8 tasks with ~24 tests**

The actual spawn-team integration requires the spawn-team executor to be implemented, which is a separate concern. This plan establishes all the building blocks needed for that integration.

---

**Next Steps After This Plan:**

- Task 9+: Spawn-team executor integration (depends on spawn-team implementation)
- Full async planning with LLM calls
- PR creation via gh CLI
