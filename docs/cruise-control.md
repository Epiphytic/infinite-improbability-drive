# Cruise-Control Architecture

> **Design Documents:**
> - [Main Design](./plans/2026-02-01-cruise-control-design.md)
> - [Planner Design](./plans/2026-02-01-cruise-planner-design.md)
> - [E2E Testing Design](./plans/2026-02-02-e2e-testing-design.md)

## Overview

**cruise-control** is an autonomous development orchestrator that executes a three-phase workflow: **Plan → Build → Validate**. It serves dual purposes:
1. A real skill for autonomous multi-LLM development
2. An E2E test harness for the spawn infrastructure

## System Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                        cruise-control                           │
├─────────────────────────────────────────────────────────────────┤
│  PHASE 1: PLAN (spawn-team ping-pong/github)                   │
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐        │
│  │ Generate    │ → │ Create PR   │ → │ Review via  │        │
│  │ beads+AISP  │    │ with plan   │    │ GitHub/stdout│        │
│  └─────────────┘    └─────────────┘    └─────────────┘        │
├─────────────────────────────────────────────────────────────────┤
│  PHASE 2: BUILD (spawn-team, dependency-aware parallel tasks)  │
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐        │
│  │ Topo-sort   │ → │ Execute     │ → │ Create PRs  │        │
│  │ dependencies│    │ with limits │    │ per config  │        │
│  └─────────────┘    └─────────────┘    └─────────────┘        │
├─────────────────────────────────────────────────────────────────┤
│  PHASE 3: VALIDATE (single spawn)                               │
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐        │
│  │ Run tests   │ → │ Audit code  │ → │ Generate    │        │
│  │ (curl, etc) │    │ vs plan     │    │ report      │        │
│  └─────────────┘    └─────────────┘    └─────────────┘        │
└─────────────────────────────────────────────────────────────────┘
```

## Core Components

### CruiseRunner (`core/src/cruise/runner.rs`)

The high-level orchestrator managing the full lifecycle:
- **Simple workflow**: Single spawn for straightforward tasks
- **Full workflow**: Plan → Approve → Execute with PR integration and beads tracking

```rust
pub struct CruiseRunner<P: SandboxProvider + Clone> {
    config: CruiseConfig,
    provider: P,
    logs_dir: PathBuf,
    auto_approve: bool,
    use_spawn_team: bool,
    team_mode: CoordinationMode,
}
```

### Planner (`core/src/cruise/planner.rs`)

Orchestrates plan generation using spawn-team with phased reviews:

| Iteration | Phase | Focus |
|-----------|-------|-------|
| 1 | Security | Auth, secrets, injection, validation |
| 2 | Technical Feasibility | Tech stack appropriateness |
| 3 | Task Granularity | Right-sizing for parallelization |
| 4 | Dependency Completeness | Missing links, parallelization |
| 5+ | General Polish | Open-ended refinement |

### SpawnTeamOrchestrator (`core/src/team_orchestrator.rs`)

Implements the actual ping-pong/GitHub loop between primary and reviewer LLMs with full observability.

## Coordination Modes

Three modes are supported via `CoordinationMode`:

### Sequential Mode
```
Primary → Review → Fix (once)
```
Simple single-pass with one review cycle.

### PingPong Mode
```
Primary → Review → Fix → Review → ... (until approved or max iterations)
```
Iterative back-and-forth with review feedback appended to PR body.

### GitHub Mode (Default)
```
Primary → PR Created → GitHub Review → Fix with Commits → Reply to Comments
```
PR-based coordination where:
- PR is created on first commit
- Reviewer LLMs create PR comments (can't use request-changes on own PRs)
- Coder LLM resolves comments with commits
- Full traceability on the PR

## Review Domains (`ReviewDomain`)

For specialized review passes:

| Domain | Focus |
|--------|-------|
| `Security` | Auth, injection, OWASP Top 10 |
| `TechnicalFeasibility` | Architecture, performance |
| `TaskGranularity` | Appropriate task sizing |
| `DependencyCompleteness` | All dependencies identified |
| `GeneralPolish` | Code quality, documentation |

Each domain has specific instructions for the reviewer in `domain.instructions()`.

## Data Flow

### Plan Phase Flow

```
User Prompt
    │
    ▼
┌─────────────────────────────────────────────────┐
│  Spawn-Team (configurable mode)                 │
│                                                 │
│  Iter 1: Primary drafts → Security review       │
│  Iter 2: Primary refines → Feasibility review   │
│  Iter 3: Primary refines → Granularity review   │
│  Iter 4: Primary refines → Dependency review    │
│  Iter 5: Primary refines → General polish       │
└─────────────────────────────────────────────────┘
    │
    ▼
JSON Plan Output
    │
    ▼
┌─────────────────┐     ┌─────────────────┐
│ Write Beads     │ ──→ │ Generate MD     │
│ .beads/CRUISE-* │     │ plan.md         │
└─────────────────┘     └─────────────────┘
    │
    ▼
Create PR → Poll for Approval → PlanResult
```

### GitHub Mode PR Lifecycle

```
1. Primary creates initial work
2. Watcher commits and pushes
3. PR created on first commit
4. For each review domain:
   a. Reviewer posts PR comment with findings
   b. If NEEDS CHANGES:
      - Coder reads comment
      - Coder edits files
      - Commits with reference to comment
      - Replies to comment with commit hash
5. Final PR ready for merge
```

## Observability

`SpawnObservability` captures comprehensive execution data:

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

This data is formatted as markdown and appended to PRs for full traceability.

## Configuration

```toml
[planning]
ping_pong_iterations = 5
reviewer_llm = "gemini-cli"

[building]
max_parallel = 3
pr_strategy = "per-task"  # per-task | batch | single
sequential_reviewer = "gemini-cli"

[validation]
test_level = "functional"  # basic | functional | strict
curl_timeout = 30

[approval]
poll_initial = "1m"
poll_max = "30m"
poll_backoff = 2.0

[timeouts]
idle_timeout_secs = 300      # 5 minutes
total_timeout_secs = 3600    # 1 hour
planning_idle_timeout_secs = 600  # 10 minutes
```

## PR Strategies

| Strategy | Behavior |
|----------|----------|
| `per-task` | One PR per beads issue, merged as completed |
| `batch` | Group by component or dependency wave |
| `single` | One growing PR, each task is a commit |

## Test Success Criteria

| Level | Requirements |
|-------|--------------|
| `basic` | All phases complete |
| `functional` | All phases + app passes tests |
| `strict` | All phases, app works, no critical findings |

## Key Types

### CruisePlan (`core/src/cruise/task.rs`)

```rust
pub struct CruisePlan {
    pub title: String,
    pub overview: String,
    pub tasks: Vec<CruiseTask>,
    pub risks: Vec<String>,
    pub spawn_instances: Vec<SpawnInstance>,
}
```

### CruiseTask

```rust
pub struct CruiseTask {
    pub id: String,              // CRUISE-XXX format
    pub subject: String,
    pub description: String,
    pub blocked_by: Vec<String>,
    pub status: TaskStatus,
    pub complexity: TaskComplexity,
    pub component: Option<String>,
    pub acceptance_criteria: Vec<String>,
    pub permissions: Vec<String>,
    pub cli_params: Option<String>,
    pub spawn_instance: Option<String>,
}
```

### SpawnInstance

```rust
pub struct SpawnInstance {
    pub id: String,              // SPAWN-XXX format
    pub name: String,
    pub use_spawn_team: bool,
    pub cli_params: String,
    pub permissions: Vec<String>,
    pub task_ids: Vec<String>,
}
```

## Integration with Beads

Plans generate beads issues as source of truth:
- Each `CruiseTask` becomes `.beads/CRUISE-XXX.md`
- Dependencies tracked via `blockedBy` relationships
- Status updated: `pending` → `in_progress` → `completed`
- Individual commits for each issue closure

## Related Documentation

- [Spawn Architecture](./architecture.md)
- [Configuration Guide](./configuration.md)
- [Design Plans](./plans/)
