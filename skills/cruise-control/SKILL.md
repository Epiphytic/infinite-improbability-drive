---
name: cruise-control
description: Autonomous development orchestrator that plans, builds, and validates applications
---

# Cruise-Control Skill

Autonomously orchestrates complete development cycles from high-level prompts through three phases: Plan, Build, and Validate.

## Usage

When you need to build an entire application or feature autonomously:

1. Provide a high-level prompt describing what to build
2. Cruise-control generates a dependency-aware plan using spawn-team ping-pong
3. A PR is created for plan approval
4. After approval, tasks execute in parallel respecting dependencies
5. Finally, the result is audited with functional tests and quality review

## Invocation

```
/cruise-control "Build a sqlite gui interface in rust with jwt authentication"
```

With options:

```
/cruise-control --max-parallel 5 "Build microservices architecture"
/cruise-control --auto-approve --test-level strict "Build and test"
```

## Three Phases

### Phase 1: Plan

Uses spawn-team in ping-pong mode:

1. Primary LLM drafts plan as beads issues
2. Reviewer critiques dependencies, identifies gaps
3. Primary refines based on feedback
4. Iterate until approved or max iterations

Output:
- Beads issues in AISP format (source of truth)
- Markdown plan document (human-readable)
- PR for approval with dependency graph

### Phase 2: Build

Uses spawn-team in sequential mode with parallelism:

1. Topological sort of tasks by dependencies
2. Execute ready tasks up to max_parallel limit
3. Create PRs per configured strategy
4. Track progress in beads issues

Handles failures by:
- Retrying once with error context
- Marking as blocked if still fails
- Continuing with non-dependent tasks

### Phase 3: Validate

Single spawn audits the result:

1. **Functional tests**: Build app, run curl tests
2. **Plan adherence**: Compare implementation to plan
3. **Quality review**: Security, performance, code quality
4. **Gap analysis**: Missing features, improvements needed

Generates comprehensive audit report.

## Approval Flow

Plans require GitHub PR approval:

1. PR created with plan documents
2. Poller checks PR status (1min -> 2min -> 4min -> ... -> 30min max)
3. On approval, build phase begins with fresh LLM context
4. Use `--auto-approve` for tests/CI to skip waiting

## Configuration

In `.infinite-probability/cruise-control.toml`:

```toml
[planning]
ping_pong_iterations = 5

[building]
max_parallel = 3
pr_strategy = "per-task"  # per-task | batch | single

[validation]
test_level = "functional"  # basic | functional | strict
```

## When to Use Cruise-Control

**Use cruise-control when:**
- Building complete applications from scratch
- Large features requiring multiple coordinated tasks
- You want automated code review at each step
- Testing spawn-team infrastructure end-to-end

**Use basic spawn when:**
- Single, focused tasks
- Quick fixes or small changes
- You need direct control over execution

## Example Workflow

```
User: /cruise-control "Build a REST API with user auth and data endpoints"

Phase 1 (Plan):
- Generates 8 tasks with dependency graph
- Creates PR #42 with plan
- Waits for approval...
- Approved!

Phase 2 (Build):
- Task 1: Set up project structure ✓
- Task 2: Implement JWT auth ✓
- Task 3: Create database models ✓
- Task 4: Implement user endpoints (depends on 2,3) ✓
- ...
- 8/8 tasks completed

Phase 3 (Validate):
- Build: ✓
- Tests: 12/12 passed
- Quality: 8.5/10
- Findings: 2 warnings (no rate limiting, missing input validation)

Result: SUCCESS
PR: https://github.com/org/repo/pull/50
Report: docs/plans/2026-02-01-api-audit-report.md
```
