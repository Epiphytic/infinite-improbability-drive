# Cruise-Control Design

> Autonomous development orchestrator with E2E test framework

## Overview

**cruise-control** is an autonomous development orchestrator that takes a high-level prompt and produces a working application through three phases: Plan, Build, Validate. It serves dual purposes: a real skill for autonomous development and an E2E test harness for the spawn-team infrastructure.

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                        cruise-control                           │
├─────────────────────────────────────────────────────────────────┤
│  PART 1: PLAN (spawn-team ping-pong)                           │
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐        │
│  │ Generate    │ → │ Create PR   │ → │ Poll for    │        │
│  │ beads+AISP  │    │ with plan   │    │ approval    │        │
│  └─────────────┘    └─────────────┘    └─────────────┘        │
├─────────────────────────────────────────────────────────────────┤
│  PART 2: BUILD (spawn-team sequential, parallel tasks)         │
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐        │
│  │ Topo-sort   │ → │ Execute     │ → │ Create PRs  │        │
│  │ dependencies│    │ with limits │    │ per config  │        │
│  └─────────────┘    └─────────────┘    └─────────────┘        │
├─────────────────────────────────────────────────────────────────┤
│  PART 3: VALIDATE (single spawn)                               │
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐        │
│  │ Run tests   │ → │ Audit code  │ → │ Generate    │        │
│  │ (curl, etc) │    │ vs plan     │    │ report      │        │
│  └─────────────┘    └─────────────┘    └─────────────┘        │
└─────────────────────────────────────────────────────────────────┘
```

## Key Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Plan format | Beads issues (AISP) → derived markdown | Machine-readable source of truth, human-readable view |
| Approval gate | GitHub PR approval with exponential backoff | No custom infrastructure, uses existing workflow |
| Parallelization | Configurable concurrency, dependency-aware | Balance speed with resource control |
| PR strategy | Configurable: per-task, batch, single | Different teams have different preferences |
| Validation depth | Full audit with report | Comprehensive feedback for improvement |
| Test success | Tiered: basic, functional, strict | Fast CI feedback, thorough validation when needed |
| Repo lifecycle | Configurable: ephemeral, persistent, accumulating | Flexibility for different use cases |

---

## Part 1: Plan Phase

### Objective

Generate a comprehensive, dependency-aware plan from a high-level prompt using spawn-team in ping-pong mode.

### Workflow

```
User prompt
    │
    ▼
┌─────────────────────────────────────────────────────────┐
│ spawn-team --ping-pong --max-iterations 5               │
│                                                         │
│   Primary (claude-code): Draft plan as beads issues     │
│         ↓                                               │
│   Reviewer (gemini-cli): Critique dependencies,         │
│                          identify gaps, suggest splits  │
│         ↓                                               │
│   Primary: Refine based on feedback                     │
│         ↓                                               │
│   (iterate until approved or max iterations)            │
└─────────────────────────────────────────────────────────┘
    │
    ▼
Beads issues created (AISP format)
    │
    ▼
Convert AISP → Markdown plan document
    │
    ▼
Create PR with:
  - docs/plans/YYYY-MM-DD-<topic>-plan.md
  - .beads/ directory with issue files
    │
    ▼
Poll for PR approval (1min → 2min → 4min → ... → 30min max)
    │
    ▼
On approval: trigger Part 2 with fresh LLM context
```

### Beads Issue Structure

Each task becomes a beads issue with:

```yaml
id: CRUISE-001
subject: "Implement JWT authentication module"
status: pending
blockedBy: []  # or ["CRUISE-002", "CRUISE-003"]
metadata:
  component: "auth"
  estimated_complexity: "medium"
  parallel_group: 1
```

### Plan Document Sections

Generated from beads issues:

1. **Overview** — What we're building
2. **Dependency Graph** — Mermaid diagram from blockedBy relationships
3. **Task Breakdown** — Each beads issue with acceptance criteria
4. **Parallel Execution Groups** — Which tasks can run concurrently
5. **Risk Areas** — Identified during ping-pong review

### Test Mode Behavior

When `--auto-approve` is set:
- PR is created and immediately approved via `gh pr merge`
- No polling wait
- Proceeds directly to Part 2

---

## Part 2: Build Phase

### Objective

Execute the approved plan by running tasks with configurable parallelism, respecting dependencies, using spawn-team in sequential mode for each task.

### Workflow

```
Approved plan (beads issues)
    │
    ▼
┌─────────────────────────────────────────────────────────┐
│ Dependency Resolution                                   │
│                                                         │
│   1. Topological sort of beads issues                   │
│   2. Identify ready tasks (no pending blockers)         │
│   3. Group by parallel execution capability             │
└─────────────────────────────────────────────────────────┘
    │
    ▼
┌─────────────────────────────────────────────────────────┐
│ Parallel Executor (respects --max-parallel N)           │
│                                                         │
│   while tasks_remain:                                   │
│     ready = tasks where all blockedBy are completed     │
│     running = min(len(ready), max_parallel - active)    │
│                                                         │
│     for task in ready[:running]:                        │
│       spawn-team --sequential \                         │
│         --primary claude-code \                         │
│         --reviewer gemini-cli \                         │
│         "Implement {task.subject}. Context: {plan}"     │
│                                                         │
│     await any_completion()                              │
│     update beads issue status                           │
│     create PR per strategy                              │
└─────────────────────────────────────────────────────────┘
    │
    ▼
All tasks completed → PRs created per strategy
```

### PR Strategies

| Strategy | Behavior |
|----------|----------|
| `per-task` | One PR per beads issue, merged as completed |
| `batch` | Group by component or dependency wave |
| `single` | One growing PR, each task is a commit |

Default: `per-task`

### Task Execution Context

Each spawn-team invocation receives:
- The specific beads issue to implement
- The full plan document for context
- List of already-completed tasks (for reference)
- Relevant code from completed dependencies

### Failure Handling

```
Task fails
    │
    ├─→ Retry once with error context
    │
    ├─→ If still fails: mark beads issue as "blocked"
    │   └─→ Continue with non-dependent tasks
    │
    └─→ At end: report blocked tasks in summary
```

### Progress Tracking

- Beads issues updated in real-time: `pending` → `in_progress` → `completed`
- Profiling: start/end timestamps per task stored in issue metadata
- Summary written to `docs/plans/YYYY-MM-DD-<topic>-execution-log.md`

---

## Part 3: Validate Phase

### Objective

Validate the built application works, audit implementation against plan, and generate a comprehensive report identifying gaps and improvements.

### Workflow

```
All Part 2 tasks completed
    │
    ▼
┌─────────────────────────────────────────────────────────┐
│ spawn (single instance, fresh context)                  │
│                                                         │
│   Inputs:                                               │
│   - Original prompt                                     │
│   - Plan document                                       │
│   - Beads issues with completion status                 │
│   - Full codebase                                       │
│   - Execution log with timings                          │
└─────────────────────────────────────────────────────────┘
    │
    ▼
┌─────────────────────────────────────────────────────────┐
│ Validation Steps                                        │
│                                                         │
│ 1. FUNCTIONAL TESTS                                     │
│    - Build the application                              │
│    - Start services (docker-compose or direct)          │
│    - Execute curl tests against endpoints               │
│    - Verify expected responses                          │
│                                                         │
│ 2. PLAN ADHERENCE                                       │
│    - Compare each beads issue to actual implementation  │
│    - Flag: implemented / partial / missing / deviated   │
│    - Identify undocumented additions                    │
│                                                         │
│ 3. QUALITY REVIEW                                       │
│    - Security: auth, input validation, secrets handling │
│    - Performance: obvious inefficiencies, N+1 queries   │
│    - Code quality: error handling, logging, tests       │
│                                                         │
│ 4. GAP ANALYSIS                                         │
│    - What's missing from original requirements?         │
│    - What was added that wasn't planned?                │
│    - What should be improved?                           │
└─────────────────────────────────────────────────────────┘
    │
    ▼
Generate Audit Report
```

### Audit Report Structure

```markdown
# Cruise-Control Audit Report
Generated: YYYY-MM-DD HH:MM

## Summary
- Overall Status: ✅ PASS / ⚠️ PARTIAL / ❌ FAIL
- Functional Tests: X/Y passed
- Plan Adherence: X/Y tasks fully implemented
- Quality Score: X/10

## Functional Test Results
| Endpoint | Method | Expected | Actual | Status |
|----------|--------|----------|--------|--------|
| /auth/login | POST | 200 | 200 | ✅ |
| /api/data | GET | 200 | 500 | ❌ |

## Plan Adherence
| Task ID | Subject | Status | Notes |
|---------|---------|--------|-------|
| CRUISE-001 | JWT auth | ✅ Implemented | |
| CRUISE-002 | SQLite GUI | ⚠️ Partial | Missing export feature |

## Quality Findings
### Critical
- [ ] SQL injection vulnerability in /api/query

### Warnings
- [ ] No rate limiting on auth endpoints
- [ ] Missing input validation on user fields

## Gaps & Improvements
### Missing from Requirements
1. ...

### Recommended Improvements
1. ...

## Execution Metrics
- Total duration: X minutes
- Part 1 (Plan): X min
- Part 2 (Build): X min
- Part 3 (Validate): X min
- Tasks parallelism achieved: X concurrent avg
```

### Output Artifacts

- `docs/plans/YYYY-MM-DD-<topic>-audit-report.md`
- Updated beads issues with validation notes
- PR with audit report (if findings require action)

---

## E2E Test Framework

### Test Configuration

```toml
# tests/e2e/cruise-control.toml

[test]
name = "cruise-control-e2e"
prompt = """
Build a sqlite gui interface in rust with jwt authentication
using a locally generated ca and private key. Ensure that the
plan shows dependencies between the pieces so that multiple
teams can work on it in parallel.
"""

[repository]
org = "epiphytic"
name_prefix = "cruise-control-test"
lifecycle = "ephemeral"  # ephemeral | persistent | accumulating
cleanup_on_success = true
cleanup_on_failure = false  # keep for debugging

[execution]
max_parallel = 3
pr_strategy = "per-task"
plan_approval = "auto"  # auto | manual | timeout:300

[timeouts]
part1_max = "30m"
part2_max = "2h"
part3_max = "30m"

[success_criteria]
level = "functional"  # basic | functional | strict
```

### Test Workflow

```
TEST SETUP
├── Verify gh cli authenticated
├── Create test repository (epiphytic/cruise-control-test-YYYYMMDD-HHMMSS)
├── Initialize with README, .gitignore
└── Clone locally to temp directory

PART 1 TEST
├── Run cruise-control --plan-only
├── Assertions:
│   ├── Beads issues created (count > 0)
│   ├── Plan markdown generated
│   ├── PR created with plan
│   └── Dependencies form valid DAG (no cycles)
├── Auto-approve PR
└── Record: duration, issue count, iteration count

PART 2 TEST
├── Run cruise-control --build-only (or use cached plan)
├── Assertions:
│   ├── All beads issues marked completed or blocked
│   ├── PRs created per strategy
│   ├── Code compiles (cargo build succeeds)
│   └── Expected files exist
└── Record: duration per task, parallelism achieved

PART 3 TEST
├── Run cruise-control --validate-only
├── Assertions (by level):
│   ├── basic: Audit report generated
│   ├── functional: App starts, curl tests pass
│   └── strict: No critical findings, all tasks implemented
└── Record: test results, quality score

CLEANUP & REPORTING
├── Generate test report
└── Based on lifecycle: delete / reset / keep repository
```

### Cached Plan Fallback

```
tests/e2e/fixtures/
├── cached-plan/
│   ├── .beads/
│   │   ├── CRUISE-001.md
│   │   ├── CRUISE-002.md
│   │   └── ...
│   └── plan.md
```

If Part 1 fails or `--skip-planning` is set, Part 2 uses the cached plan.

### Test Invocation

```bash
# Full E2E test
cargo test --test cruise_control_e2e -- --ignored

# Individual phases
cargo test --test cruise_control_e2e plan_phase
cargo test --test cruise_control_e2e build_phase
cargo test --test cruise_control_e2e validate_phase

# With options
CRUISE_TEST_LEVEL=strict \
CRUISE_REPO_LIFECYCLE=persistent \
cargo test --test cruise_control_e2e
```

### Tiered Success Criteria

| Level | Requirements |
|-------|--------------|
| `basic` | All phases complete (regardless of validation findings) |
| `functional` | All phases complete AND app passes curl tests |
| `strict` | All phases complete, app works, AND no critical audit findings |

---

## CLI Interface

### Skill Invocation

```bash
# Full autonomous run
/cruise-control "Build a sqlite gui interface in rust with jwt authentication..."

# Phase-specific
/cruise-control --plan-only "..."
/cruise-control --build-only --plan-file docs/plans/2026-02-01-sqlite-gui-plan.md
/cruise-control --validate-only

# With options
/cruise-control \
  --max-parallel 5 \
  --pr-strategy batch \
  --auto-approve \
  "..."
```

### CLI Options

| Option | Default | Description |
|--------|---------|-------------|
| `--plan-only` | false | Only run Part 1 (planning) |
| `--build-only` | false | Only run Part 2 (building) |
| `--validate-only` | false | Only run Part 3 (validation) |
| `--plan-file <path>` | - | Use existing plan instead of generating |
| `--max-parallel <n>` | 3 | Max concurrent spawn-team instances |
| `--pr-strategy <s>` | per-task | per-task, batch, or single |
| `--auto-approve` | false | Skip PR approval wait (for tests/CI) |
| `--test-level <l>` | functional | basic, functional, or strict |
| `--repo <org/name>` | - | Target repository (for E2E tests) |
| `--cleanup` | false | Delete test repo on completion |

---

## Configuration

```toml
# .infinite-probability/cruise-control.toml

[planning]
ping_pong_iterations = 5
reviewer_llm = "gemini-cli"

[building]
max_parallel = 3
pr_strategy = "per-task"
sequential_reviewer = "gemini-cli"

[validation]
test_level = "functional"
curl_timeout = 30

[approval]
poll_initial = "1m"
poll_max = "30m"
poll_backoff = 2.0  # exponential multiplier

[test]
default_org = "epiphytic"
repo_lifecycle = "ephemeral"
```

---

## Implementation Structure

### Rust Modules

```
core/src/
├── cruise/
│   ├── mod.rs           # Public API
│   ├── planner.rs       # Part 1: Plan generation
│   ├── builder.rs       # Part 2: Parallel execution
│   ├── validator.rs     # Part 3: Audit & validation
│   ├── approval.rs      # GitHub PR polling
│   ├── config.rs        # Configuration loading
│   └── report.rs        # Report generation
```

### Integration Points

| Component | Uses |
|-----------|------|
| `planner.rs` | `spawn-team` (ping-pong), `beads` |
| `builder.rs` | `spawn-team` (sequential), `beads`, `PRManager` |
| `validator.rs` | `spawn` (single), report generation |
| `approval.rs` | `gh` CLI for PR status polling |

### Artifacts to Create

| File | Purpose |
|------|---------|
| `commands/cruise-control.md` | Slash command definition |
| `skills/cruise-control/SKILL.md` | Skill documentation |
| `core/src/cruise/*.rs` | Rust implementation |
| `tests/e2e/cruise_control_e2e.rs` | E2E test |
| `tests/e2e/fixtures/cached-plan/` | Fallback plan for testing |
| `tests/e2e/cruise-control.toml` | Test configuration |
