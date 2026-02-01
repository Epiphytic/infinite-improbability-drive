//! Infinite Improbability Drive - Claude Code plugin for spawning sandboxed LLMs
//!
//! This library provides the core functionality for launching isolated LLM instances
//! in git worktree sandboxes with intelligent resource provisioning and lifecycle management.

pub mod error;
pub mod sandbox;
pub mod spawn;

pub use error::Error;
pub use sandbox::{Sandbox, SandboxManifest, SandboxProvider};
pub use spawn::{SpawnConfig, SpawnResult, SpawnStatus};
