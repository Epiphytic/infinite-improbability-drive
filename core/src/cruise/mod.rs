//! Cruise-control: autonomous development orchestrator.
//!
//! Three-phase workflow: Plan → Build → Validate

pub mod config;
pub mod task;

pub use config::{
    ApprovalConfig, BuildingConfig, CruiseConfig, PlanningConfig, PrStrategy, RepoLifecycle,
    TestConfig, TestLevel, ValidationConfig,
};
pub use task::{CruisePlan, CruiseTask, TaskComplexity, TaskStatus};
