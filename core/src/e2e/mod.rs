//! End-to-end testing infrastructure.
//!
//! Provides fixture-driven E2E tests using real LLM runners
//! against ephemeral GitHub repositories.

pub mod fixture;
pub mod harness;
pub mod repo;
pub mod validator;

pub use fixture::{ExecutionPhase, Fixture, RunnerType, ValidationConfig, ValidationLevel, WorkflowType};
pub use harness::{E2EConfig, E2EHarness, E2EResult};
pub use repo::EphemeralRepo;
pub use validator::Validator;
