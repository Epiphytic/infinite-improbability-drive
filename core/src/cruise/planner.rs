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
}
