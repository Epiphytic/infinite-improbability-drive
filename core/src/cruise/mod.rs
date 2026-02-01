//! Cruise-control: autonomous development orchestrator.
//!
//! Three-phase workflow: Plan → Build → Validate

pub mod config;

pub use config::{
    ApprovalConfig, BuildingConfig, CruiseConfig, PlanningConfig, PrStrategy, RepoLifecycle,
    TestConfig, TestLevel, ValidationConfig,
};
