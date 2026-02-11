//! Task representation for cruise-control plans.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Status of a cruise task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    /// Task is waiting to be started.
    #[default]
    Pending,
    /// Task is currently being executed.
    InProgress,
    /// Task completed successfully.
    Completed,
    /// Task failed and is blocked.
    Blocked,
    /// Task was skipped.
    Skipped,
}

/// Complexity estimate for a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskComplexity {
    /// Simple task (< 30 min).
    Low,
    /// Moderate task (30-60 min).
    #[default]
    Medium,
    /// Complex task (> 60 min).
    High,
}

/// A single task in a cruise-control plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CruiseTask {
    /// Unique task identifier (e.g., "CRUISE-001").
    pub id: String,
    /// Task subject/title.
    pub subject: String,
    /// Detailed description.
    pub description: String,
    /// Current status.
    #[serde(default)]
    pub status: TaskStatus,
    /// IDs of tasks this depends on.
    #[serde(default)]
    pub blocked_by: Vec<String>,
    /// Component this task belongs to.
    #[serde(default)]
    pub component: Option<String>,
    /// Estimated complexity.
    #[serde(default)]
    pub complexity: TaskComplexity,
    /// Parallel execution group.
    #[serde(default)]
    pub parallel_group: Option<u32>,
    /// Acceptance criteria.
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
    /// Start timestamp (when moved to InProgress).
    #[serde(default)]
    pub started_at: Option<u64>,
    /// End timestamp (when completed/blocked).
    #[serde(default)]
    pub finished_at: Option<u64>,
    /// Error message if blocked.
    #[serde(default)]
    pub error: Option<String>,
    /// Required permissions for the LLM to execute this task.
    /// e.g., ["Read", "Write", "Edit", "Bash"]
    #[serde(default)]
    pub permissions: Vec<String>,
    /// CLI parameters for launching the LLM for this task.
    /// e.g., "--model opus --allowedTools Read,Write,Edit"
    #[serde(default)]
    pub cli_params: Option<String>,
    /// Which spawn/spawn-team instance this task belongs to.
    /// Tasks with the same spawn_instance will be executed together.
    #[serde(default)]
    pub spawn_instance: Option<String>,
}

impl CruiseTask {
    /// Creates a new task with the given ID and subject.
    pub fn new(id: impl Into<String>, subject: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            subject: subject.into(),
            description: String::new(),
            status: TaskStatus::Pending,
            blocked_by: Vec::new(),
            component: None,
            complexity: TaskComplexity::Medium,
            parallel_group: None,
            acceptance_criteria: Vec::new(),
            started_at: None,
            finished_at: None,
            error: None,
            permissions: Vec::new(),
            cli_params: None,
            spawn_instance: None,
        }
    }

    /// Sets the description.
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = desc.into();
        self
    }

    /// Sets the dependencies.
    pub fn with_blocked_by(mut self, deps: Vec<String>) -> Self {
        self.blocked_by = deps;
        self
    }

    /// Sets the component.
    pub fn with_component(mut self, component: impl Into<String>) -> Self {
        self.component = Some(component.into());
        self
    }

    /// Sets the complexity.
    pub fn with_complexity(mut self, complexity: TaskComplexity) -> Self {
        self.complexity = complexity;
        self
    }

    /// Sets the required permissions.
    pub fn with_permissions(mut self, permissions: Vec<String>) -> Self {
        self.permissions = permissions;
        self
    }

    /// Sets the CLI parameters.
    pub fn with_cli_params(mut self, cli_params: impl Into<String>) -> Self {
        self.cli_params = Some(cli_params.into());
        self
    }

    /// Sets the spawn instance.
    pub fn with_spawn_instance(mut self, instance: impl Into<String>) -> Self {
        self.spawn_instance = Some(instance.into());
        self
    }

    /// Checks if this task is ready to execute (all dependencies completed).
    pub fn is_ready(&self, completed_tasks: &HashSet<String>) -> bool {
        self.status == TaskStatus::Pending
            && self
                .blocked_by
                .iter()
                .all(|dep| completed_tasks.contains(dep))
    }
}

/// A spawn/spawn-team instance that groups related tasks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnInstance {
    /// Unique instance identifier (e.g., "SPAWN-001").
    pub id: String,
    /// Human-readable name for the instance.
    pub name: String,
    /// Whether this uses spawn-team (ping-pong mode) or single spawn.
    #[serde(default)]
    pub use_spawn_team: bool,
    /// CLI parameters for launching this instance.
    pub cli_params: String,
    /// Required permissions for this instance.
    pub permissions: Vec<String>,
    /// Task IDs that belong to this instance.
    pub task_ids: Vec<String>,
}

impl SpawnInstance {
    /// Creates a new spawn instance.
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            use_spawn_team: false,
            cli_params: String::new(),
            permissions: Vec::new(),
            task_ids: Vec::new(),
        }
    }
}

/// A complete cruise-control plan.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CruisePlan {
    /// Original prompt that generated this plan.
    pub prompt: String,
    /// Plan title.
    pub title: String,
    /// Plan overview/summary.
    pub overview: String,
    /// All tasks in the plan.
    pub tasks: Vec<CruiseTask>,
    /// Risk areas identified during planning.
    #[serde(default)]
    pub risks: Vec<String>,
    /// Number of ping-pong iterations used to create plan.
    #[serde(default)]
    pub planning_iterations: u32,
    /// Spawn instances that group tasks for execution.
    #[serde(default)]
    pub spawn_instances: Vec<SpawnInstance>,
}

impl CruisePlan {
    /// Creates a new plan with the given prompt.
    pub fn new(prompt: impl Into<String>) -> Self {
        Self {
            prompt: prompt.into(),
            title: String::new(),
            overview: String::new(),
            tasks: Vec::new(),
            risks: Vec::new(),
            planning_iterations: 0,
            spawn_instances: Vec::new(),
        }
    }

    /// Returns tasks that are ready to execute.
    pub fn ready_tasks(&self) -> Vec<&CruiseTask> {
        let completed: HashSet<String> = self
            .tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Completed)
            .map(|t| t.id.clone())
            .collect();

        self.tasks
            .iter()
            .filter(|t| t.is_ready(&completed))
            .collect()
    }

    /// Returns the count of tasks by status.
    pub fn task_counts(&self) -> (usize, usize, usize, usize) {
        let pending = self
            .tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Pending)
            .count();
        let in_progress = self
            .tasks
            .iter()
            .filter(|t| t.status == TaskStatus::InProgress)
            .count();
        let completed = self
            .tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Completed)
            .count();
        let blocked = self
            .tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Blocked)
            .count();
        (pending, in_progress, completed, blocked)
    }

    /// Checks for dependency cycles using DFS.
    pub fn has_cycle(&self) -> Option<String> {
        use std::collections::HashMap;

        let task_map: HashMap<&str, &CruiseTask> =
            self.tasks.iter().map(|t| (t.id.as_str(), t)).collect();

        #[derive(Clone, Copy, PartialEq)]
        enum Color {
            White,
            Gray,
            Black,
        }

        let mut colors: HashMap<&str, Color> =
            task_map.keys().map(|&k| (k, Color::White)).collect();

        fn dfs<'a>(
            node: &'a str,
            task_map: &'a HashMap<&'a str, &'a CruiseTask>,
            colors: &mut HashMap<&'a str, Color>,
            path: &mut Vec<&'a str>,
        ) -> Option<String> {
            colors.insert(node, Color::Gray);
            path.push(node);

            if let Some(task) = task_map.get(node) {
                for dep in &task.blocked_by {
                    match colors.get(dep.as_str()) {
                        Some(Color::Gray) => {
                            // Found cycle
                            path.push(dep.as_str());
                            return Some(path.join(" -> "));
                        }
                        Some(Color::White) | None => {
                            if let Some(cycle) = dfs(dep.as_str(), task_map, colors, path) {
                                return Some(cycle);
                            }
                        }
                        Some(Color::Black) => {}
                    }
                }
            }

            colors.insert(node, Color::Black);
            path.pop();
            None
        }

        for &task_id in task_map.keys() {
            if colors.get(task_id) == Some(&Color::White) {
                let mut path = Vec::new();
                if let Some(cycle) = dfs(task_id, &task_map, &mut colors, &mut path) {
                    return Some(cycle);
                }
            }
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cruise_task_builder_works() {
        let task = CruiseTask::new("CRUISE-001", "Implement auth")
            .with_description("Add JWT authentication")
            .with_component("auth")
            .with_complexity(TaskComplexity::High)
            .with_blocked_by(vec!["CRUISE-002".to_string()]);

        assert_eq!(task.id, "CRUISE-001");
        assert_eq!(task.subject, "Implement auth");
        assert_eq!(task.description, "Add JWT authentication");
        assert_eq!(task.component, Some("auth".to_string()));
        assert_eq!(task.complexity, TaskComplexity::High);
        assert_eq!(task.blocked_by, vec!["CRUISE-002"]);
        assert_eq!(task.status, TaskStatus::Pending);
    }

    #[test]
    fn task_is_ready_when_no_dependencies() {
        let task = CruiseTask::new("CRUISE-001", "First task");
        let completed = HashSet::new();
        assert!(task.is_ready(&completed));
    }

    #[test]
    fn task_is_ready_when_dependencies_completed() {
        let task = CruiseTask::new("CRUISE-002", "Second task")
            .with_blocked_by(vec!["CRUISE-001".to_string()]);

        let mut completed = HashSet::new();
        assert!(!task.is_ready(&completed));

        completed.insert("CRUISE-001".to_string());
        assert!(task.is_ready(&completed));
    }

    #[test]
    fn task_not_ready_when_already_in_progress() {
        let mut task = CruiseTask::new("CRUISE-001", "Task");
        task.status = TaskStatus::InProgress;
        let completed = HashSet::new();
        assert!(!task.is_ready(&completed));
    }

    #[test]
    fn cruise_plan_ready_tasks() {
        let mut plan = CruisePlan::new("Build app");
        plan.tasks = vec![
            CruiseTask::new("CRUISE-001", "Task 1"),
            CruiseTask::new("CRUISE-002", "Task 2").with_blocked_by(vec!["CRUISE-001".to_string()]),
            CruiseTask::new("CRUISE-003", "Task 3"),
        ];

        let ready = plan.ready_tasks();
        assert_eq!(ready.len(), 2);
        assert!(ready.iter().any(|t| t.id == "CRUISE-001"));
        assert!(ready.iter().any(|t| t.id == "CRUISE-003"));
    }

    #[test]
    fn cruise_plan_task_counts() {
        let mut plan = CruisePlan::new("Build app");
        plan.tasks = vec![
            {
                let mut t = CruiseTask::new("CRUISE-001", "Task 1");
                t.status = TaskStatus::Completed;
                t
            },
            CruiseTask::new("CRUISE-002", "Task 2"),
            {
                let mut t = CruiseTask::new("CRUISE-003", "Task 3");
                t.status = TaskStatus::InProgress;
                t
            },
        ];

        let (pending, in_progress, completed, blocked) = plan.task_counts();
        assert_eq!(pending, 1);
        assert_eq!(in_progress, 1);
        assert_eq!(completed, 1);
        assert_eq!(blocked, 0);
    }

    #[test]
    fn cruise_plan_detects_cycle() {
        let mut plan = CruisePlan::new("Build app");
        plan.tasks = vec![
            CruiseTask::new("A", "Task A").with_blocked_by(vec!["B".to_string()]),
            CruiseTask::new("B", "Task B").with_blocked_by(vec!["C".to_string()]),
            CruiseTask::new("C", "Task C").with_blocked_by(vec!["A".to_string()]),
        ];

        assert!(plan.has_cycle().is_some());
    }

    #[test]
    fn cruise_plan_no_cycle_for_valid_dag() {
        let mut plan = CruisePlan::new("Build app");
        plan.tasks = vec![
            CruiseTask::new("A", "Task A"),
            CruiseTask::new("B", "Task B").with_blocked_by(vec!["A".to_string()]),
            CruiseTask::new("C", "Task C").with_blocked_by(vec!["A".to_string(), "B".to_string()]),
        ];

        assert!(plan.has_cycle().is_none());
    }

    #[test]
    fn task_status_serializes_correctly() {
        assert_eq!(
            serde_json::to_string(&TaskStatus::Pending).unwrap(),
            "\"pending\""
        );
        assert_eq!(
            serde_json::to_string(&TaskStatus::Completed).unwrap(),
            "\"completed\""
        );
    }

    #[test]
    fn task_complexity_serializes_correctly() {
        assert_eq!(
            serde_json::to_string(&TaskComplexity::Low).unwrap(),
            "\"low\""
        );
        assert_eq!(
            serde_json::to_string(&TaskComplexity::High).unwrap(),
            "\"high\""
        );
    }
}
