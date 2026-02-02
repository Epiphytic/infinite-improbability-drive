//! Cruise-control: autonomous development orchestrator.
//!
//! Three-phase workflow: Plan → Build → Validate

pub mod approval;
pub mod config;
pub mod planner;
pub mod result;
pub mod task;

pub use config::{
    ApprovalConfig, BuildingConfig, CruiseConfig, PlanningConfig, PrStrategy, RepoLifecycle,
    TestConfig, TestLevel, ValidationConfig,
};
pub use result::{
    AdherenceCheck, AdherenceStatus, AuditFinding, BuildResult, CruiseResult, FindingSeverity,
    FunctionalTestResult, PlanResult, TaskResult, ValidationResult,
};
pub use task::{CruisePlan, CruiseTask, TaskComplexity, TaskStatus};
pub use approval::{ApprovalPoller, PrStatus};
pub use planner::ReviewPhase;
