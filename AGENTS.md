# Agents

This document describes the agent architecture used in infinite-improbability-drive.

## Overview

The system uses multiple specialized agents that work together to accomplish autonomous development tasks. Agents communicate through standardized interfaces and can coordinate via several mechanisms.

## Agent Types

### Watcher Agent

**Location:** `core/src/watcher.rs` | [agents/watcher.md](./agents/watcher.md)

The orchestration brain that supervises sandboxed LLM execution:
- Evaluates tasks using LLM-assisted analysis
- Provisions sandboxes via git worktrees
- Monitors execution progress
- Handles permission errors and recovery
- Creates pull requests for changes

### Spawn Agent

**Location:** `core/src/spawn.rs`

The entry point for delegating tasks to isolated LLM instances:
- Configures sandbox manifests
- Invokes watcher agent
- Returns results to host LLM

### SpawnTeam Orchestrator

**Location:** `core/src/team_orchestrator.rs`

Coordinates multi-LLM workflows with full observability:
- Manages ping-pong iterations between primary and reviewer
- Handles GitHub-based PR review workflows
- Tracks permissions, commits, and review feedback
- Captures security findings

## Coordination Modes

### Sequential

```
Primary LLM ──► Reviewer LLM ──► Fix (once)
```

Single-pass coordination with one review cycle.

### PingPong

```
Primary ◄──► Reviewer (iterate until approved or max iterations)
```

Iterative back-and-forth where review feedback is incorporated until approval.

### GitHub (Default)

```
Primary ──► PR Created ──► GitHub Reviews ──► Commit Fixes ──► Reply to Comments
```

PR-based coordination using GitHub as the communication medium:
1. PR created on first commit
2. Reviewer posts PR comments with findings
3. Coder resolves comments with commits
4. Full traceability on the PR

## Review Domains

For specialized review passes during planning and execution:

| Domain | Focus |
|--------|-------|
| Security | Auth, injection, OWASP Top 10 |
| TechnicalFeasibility | Architecture, performance |
| TaskGranularity | Appropriate task sizing |
| DependencyCompleteness | All dependencies identified |
| GeneralPolish | Code quality, documentation |

## Agent Communication

### Prompt-Based

Agents communicate through structured prompts:
- `ReviewPromptBuilder` - Constructs review requests
- `FixPromptBuilder` - Constructs fix requests from suggestions
- `GitHubReviewPromptBuilder` - Creates GitHub review prompts
- `ResolveCommentPromptBuilder` - Creates comment resolution prompts

### File-Based

Agents share context through:
- Plan files in `docs/plans/`
- Beads issues in `.beads/`
- PR descriptions and comments

### GitHub-Based

In GitHub mode, agents communicate via:
- PR comments for review feedback
- Commit messages for fix explanations
- PR body updates for observability

## Observability

All agent interactions are captured in `SpawnObservability`:

```rust
pub struct SpawnObservability {
    pub command_lines: Vec<CommandLineRecord>,
    pub permissions_requested: Vec<PermissionRecord>,
    pub permissions_granted: Vec<PermissionRecord>,
    pub review_feedback: Vec<ReviewFeedbackRecord>,
    pub security_findings: Vec<SecurityFinding>,
    pub sandbox_path: Option<PathBuf>,
    pub commits: Vec<CommitRecord>,
    pub pr_url: Option<String>,
    pub github_reviews: Vec<GitHubReview>,
    pub resolved_comments: Vec<ResolvedCommentRecord>,
}
```

## LLM Runners

### ClaudeRunner

Primary coder using Claude Code CLI:
- File operations (Read, Write, Edit)
- Code generation and modification
- Test execution

### GeminiRunner

Reviewer using Gemini CLI:
- Code review analysis
- Security assessment
- Technical feasibility evaluation

## Configuration

Agent behavior is configured via:

```toml
[spawn-team]
mode = "github"           # sequential | pingpong | github
max_iterations = 3
primary_llm = "claude-code"
reviewer_llm = "gemini-cli"
max_escalations = 5
```

## Development Requirements

### Test-Driven Development

**All agent development MUST follow TDD:**

1. Write failing tests first
2. Implement minimal code to pass
3. Refactor while maintaining green tests
4. No feature is complete without tests

### Verification Standards

Before any agent change is considered complete:

- [ ] Unit tests exist and pass
- [ ] Integration tests verify agent interactions
- [ ] E2E tests validate full workflows
- [ ] Documentation updated

## Related Documentation

- [Cruise-Control Architecture](./docs/cruise-control.md)
- [Spawn Architecture](./docs/architecture.md)
- [Watcher Agent Spec](./agents/watcher.md)
- [Configuration Guide](./docs/configuration.md)
