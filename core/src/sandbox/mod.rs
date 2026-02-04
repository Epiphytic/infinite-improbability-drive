//! Sandbox module for isolated LLM execution environments.
//!
//! This module provides the [`SandboxProvider`] trait for creating isolated
//! sandboxes and the [`WorktreeSandbox`] implementation using git worktrees.

mod phase;
mod phase_state;
mod provider;
mod worktree;

pub use phase::PhaseSandbox;
pub use phase_state::{CommentInfo, PhaseState};
pub use provider::{Sandbox, SandboxManifest, SandboxProvider};
pub use worktree::WorktreeSandbox;
