---
name: cruise-control
description: Autonomous development orchestrator - plan, build, validate
usage: /cruise-control [options] "<prompt>"
---

# /cruise-control Command

Autonomously plans, builds, and validates applications from a high-level prompt.

## Synopsis

```
/cruise-control "<prompt>"
/cruise-control --plan-only "<prompt>"
/cruise-control --build-only --plan-file <path>
/cruise-control --validate-only
/cruise-control --auto-approve "<prompt>"
```

## Description

The `/cruise-control` command orchestrates a complete development cycle:

1. **Plan Phase**: Uses spawn-team ping-pong to generate a dependency-aware plan as beads issues, converts to markdown, creates PR for approval
2. **Build Phase**: Executes tasks with configurable parallelism using spawn-team sequential, respecting dependencies
3. **Validate Phase**: Audits the result with functional tests, plan adherence checks, and quality review

## Options

### Phase Control

- `--plan-only`: Only run the planning phase
- `--build-only`: Only run the build phase (requires `--plan-file`)
- `--validate-only`: Only run the validation phase

### Planning Options

- `--plan-file <path>`: Use existing plan instead of generating
- `--ping-pong-iterations <n>`: Max ping-pong iterations (default: 5)

### Building Options

- `--max-parallel <n>`: Max concurrent spawn-team instances (default: 3)
- `--pr-strategy <strategy>`: PR strategy - per-task, batch, or single (default: per-task)

### Approval Options

- `--auto-approve`: Skip PR approval wait (for tests/CI)

### Validation Options

- `--test-level <level>`: Success level - basic, functional, or strict (default: functional)

### Test Options

- `--repo <org/name>`: Target repository (for E2E tests)
- `--cleanup`: Delete test repo on completion

## Examples

```bash
# Full autonomous run
/cruise-control "Build a REST API with SQLite and JWT auth"

# Planning only
/cruise-control --plan-only "Design a CLI tool for data processing"

# Build from existing plan
/cruise-control --build-only --plan-file docs/plans/2026-02-01-api-plan.md

# With options
/cruise-control --max-parallel 5 --pr-strategy batch "Build microservices"

# E2E test mode
/cruise-control --auto-approve --test-level strict "Build test app"
```

## Output

Returns a `CruiseResult` with:

- Overall success status
- Plan result (iterations, task count, PR URL)
- Build result (task results, parallelism achieved)
- Validation result (test results, findings, quality score)
- Total duration
- Summary message

## Configuration

Settings in `.infinite-probability/cruise-control.toml`:

```toml
[planning]
ping_pong_iterations = 5
reviewer_llm = "gemini-cli"

[building]
max_parallel = 3
pr_strategy = "per-task"

[validation]
test_level = "functional"

[approval]
poll_initial = "1m"
poll_max = "30m"
poll_backoff = 2.0
```

## See Also

- `/spawn` - Basic spawn without orchestration
- `/spawn-team` - Multi-LLM coordination
