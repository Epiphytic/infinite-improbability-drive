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
        let prompt =
            PlanReviewPromptBuilder::new(r#"{"title": "Test"}"#, ReviewPhase::Security).build();

        assert!(prompt.contains("Plan Review Request"));
        assert!(prompt.contains("security gaps"));
        assert!(prompt.contains("authentication"));
    }

    #[test]
    fn plan_review_prompt_builder_includes_plan() {
        let prompt = PlanReviewPromptBuilder::new(
            r#"{"title": "My Plan", "tasks": []}"#,
            ReviewPhase::DependencyCompleteness,
        )
        .build();

        assert!(prompt.contains("My Plan"));
        assert!(prompt.contains("dependencies"));
        assert!(prompt.contains("parallel"));
    }
}
