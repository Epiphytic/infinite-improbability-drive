# infinite-improbability-drive

> **LLM Specification:** [CLAUDE.aisp](./CLAUDE.aisp) — Optimized for AI consumption

A Claude Code plugin for spawning sandboxed coding LLMs with intelligent resource provisioning and lifecycle management.

## Project Overview

This plugin provides a `spawn` skill that enables host LLMs to delegate complex tasks to isolated sub-LLMs without context pollution. The architecture uses a watcher agent pattern for orchestration, with git worktrees providing sandbox isolation.

## Documentation

| Document | Human | LLM (AISP) |
|----------|-------|------------|
| Architecture | [docs/architecture.md](./docs/architecture.md) | [docs/architecture.aisp](./docs/architecture.aisp) |
| Spawn-Team | [docs/spawn-team.md](./docs/spawn-team.md) | - |
| Cruise-Control | [docs/cruise-control.md](./docs/cruise-control.md) | - |
| Configuration | [docs/configuration.md](./docs/configuration.md) | [docs/configuration.aisp](./docs/configuration.aisp) |
| Watcher Agent | [agents/watcher.md](./agents/watcher.md) | [agents/watcher.aisp](./agents/watcher.aisp) |
| Agents | [AGENTS.md](./AGENTS.md) | - |

### Design Documents

| Document | Description |
|----------|-------------|
| [Cruise-Control Design](./docs/plans/2026-02-01-cruise-control-design.md) | Main cruise-control design |
| [Planner Design](./docs/plans/2026-02-01-cruise-planner-design.md) | Plan phase design |
| [E2E Testing Design](./docs/plans/2026-02-02-e2e-testing-design.md) | End-to-end testing infrastructure |

## Architecture

```
Host LLM Session → spawn command → Watcher Agent → Sandboxed LLM Instance
                                        ↓
                              (provisions, monitors,
                               recovers, creates PR)
```

### Spawn-Team Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                    SpawnTeamOrchestrator                        │
├─────────────────────────────────────────────────────────────────┤
│  Modes: Sequential | PingPong | GitHub (default)               │
│                                                                 │
│  Primary (Claude) ◄──► Reviewer (Gemini)                       │
│         │                      │                                │
│         ▼                      ▼                                │
│  Commits + Push        PR Comments / Review Feedback            │
└─────────────────────────────────────────────────────────────────┘
```

### Cruise-Control Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                        cruise-control                           │
├─────────────────────────────────────────────────────────────────┤
│  PHASE 1: PLAN (spawn-team with phased reviews)                │
│  PHASE 2: BUILD (parallel task execution)                      │
│  PHASE 3: VALIDATE (audit against plan)                        │
└─────────────────────────────────────────────────────────────────┘
```

See [docs/cruise-control.md](./docs/cruise-control.md) for full details.

### Key Components

| Component | File | Responsibility |
|-----------|------|----------------|
| `SpawnCommand` | `core/src/spawn.rs` | CLI/skill entry point |
| `WatcherAgent` | `core/src/watcher.rs` | Orchestrates spawn lifecycle |
| `SandboxProvider` | `core/src/sandbox/provider.rs` | Trait for isolation (worktree now, Docker later) |
| `WorktreeSandbox` | `core/src/sandbox/worktree.rs` | Git worktree implementation |
| `LLMRunner` | `core/src/runner/` | Launches and streams from target CLI |
| `ProgressMonitor` | `core/src/monitor.rs` | Tracks activity, detects hangs |
| `PermissionDetector` | `core/src/permissions.rs` | Pattern-matches permission errors |
| `PRManager` | `core/src/pr.rs` | Creates PRs, handles merge conflicts |
| `SecretsManager` | `core/src/secrets.rs` | Secret injection & log redaction |
| `CruiseRunner` | `core/src/cruise/runner.rs` | Orchestrates cruise-control workflow |
| `Planner` | `core/src/cruise/planner.rs` | Plan generation with phased reviews |
| `SpawnTeamOrchestrator` | `core/src/team_orchestrator.rs` | Multi-LLM coordination |

## Directory Structure

```
infinite-improbability-drive/
├── .claude-plugin/
│   └── plugin.json              # Plugin metadata
├── CLAUDE.md                    # This file (human instructions)
├── CLAUDE.aisp                  # LLM instructions
├── README.md                    # User documentation
├── commands/
│   ├── spawn.md                 # /spawn command
│   └── spawn-team.md            # /spawn-team command
├── skills/
│   └── spawn/
│       └── SKILL.md             # spawn skill definition
├── agents/
│   ├── watcher.md               # Watcher agent (human)
│   └── watcher.aisp             # Watcher agent (LLM)
├── hooks/
│   └── hooks.json               # Event hooks
├── core/                        # Rust implementation
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── main.rs              # CLI binary
│       ├── spawn.rs
│       ├── watcher.rs
│       ├── sandbox/
│       ├── runner/
│       ├── monitor.rs
│       ├── permissions.rs
│       ├── secrets.rs
│       └── pr.rs
├── docs/
│   ├── architecture.md          # Human readable
│   ├── architecture.aisp        # LLM optimized
│   ├── configuration.md         # Human readable
│   └── configuration.aisp       # LLM optimized
└── tests/
```

## Development Guidelines

### Test-Driven Development (REQUIRED)

**All development on this project MUST follow Test-Driven Development (TDD).**

#### TDD Workflow

1. **Write failing tests first** - Define expected behavior before implementation
2. **Implement minimal code to pass** - Only write enough code to make tests green
3. **Refactor while maintaining green tests** - Improve code quality with test safety net

#### Verification Standards

**No feature is considered complete until it has provable, verifiable tests that it works.**

Before any work is considered done:
- [ ] Unit tests exist and pass for all new code
- [ ] Integration tests verify component interactions
- [ ] E2E tests validate full workflows (where applicable)
- [ ] `cargo test` passes with no failures
- [ ] Documentation updated if behavior changed

#### Test Commands

```bash
# Run all tests (required before any commit)
cargo test

# Run specific test module
cargo test cruise::

# Run E2E tests (slower, requires real LLMs)
cargo test --test e2e

# Run tests with output
cargo test -- --nocapture
```

### Rust Conventions

- Use `thiserror` for custom error types
- Use `tokio` for async runtime
- Use `tracing` for structured logging
- Prefer `Result<T, E>` over panics
- All public APIs must be documented

### Error Handling Pattern

```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum SpawnError {
    #[error("Sandbox creation failed: {0}")]
    SandboxCreation(String),

    #[error("Permission denied: {kind}")]
    Permission { kind: PermissionErrorType },

    #[error("Timeout after {duration:?}")]
    Timeout { duration: Duration },
}
```

### Testing

- Unit tests in same file as implementation (`#[cfg(test)]`)
- Integration tests in `tests/` directory
- Use `tempfile` for filesystem tests
- Mock LLM responses for deterministic tests

## Configuration

See [docs/configuration.md](./docs/configuration.md) for full details.

Quick reference:

```toml
[spawn]
mode = "aisp"                    # "aisp" or "passthrough"
recovery_strategy = "moderate"   # "moderate", "aggressive", "interactive"
idle_timeout = 120               # seconds
total_timeout = 1800             # seconds
default_llm = "claude-code"      # or "gemini-cli"

[spawn.permissions]
allowed_tools = ["Read", "Write", "Edit", "Glob", "Grep", "Bash"]
denied_tools = ["Task"]          # No recursive spawning
max_permission_escalations = 1

[spawn-team]
mode = "github"                  # "sequential", "pingpong", or "github"
max_iterations = 3
primary_llm = "claude-code"
reviewer_llm = "gemini-cli"
max_escalations = 5

[cruise-control.planning]
ping_pong_iterations = 5
reviewer_llm = "gemini-cli"

[cruise-control.building]
max_parallel = 3
pr_strategy = "per-task"         # "per-task", "batch", or "single"

[cruise-control.validation]
test_level = "functional"        # "basic", "functional", or "strict"

[cruise-control.timeouts]
idle_timeout_secs = 300          # 5 minutes
total_timeout_secs = 3600        # 1 hour
planning_idle_timeout_secs = 600 # 10 minutes
```

## Key Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Sandbox isolation | Git worktree | Lightweight, leverages existing git infrastructure |
| Task evaluation | LLM-assisted | Accurate resource provisioning |
| Recovery strategy | Configurable | Flexibility for different use cases |
| Timeout strategy | Activity-based | Won't kill thinking LLMs, catches hangs |
| Change integration | PR-based | Host retains control over merges |
| Logging | Tiered | Disk efficiency with debug capability |
| Spawn-team default | GitHub mode | Full traceability, human-readable reviews on PRs |
| Review phases | 5 specialized domains | Security, feasibility, granularity, dependencies, polish |
| Cruise-control plans | Beads issues + markdown | Machine-readable source of truth, human-readable view |
| Development process | Test-Driven Development | Provable correctness, prevents regressions |

## Implementation Phases

### Phase 1: Foundation ✓
- [x] Create plugin structure
- [x] Implement SandboxProvider trait + worktree
- [x] Basic spawn command

### Phase 2: Watcher Agent ✓
- [x] LLM-assisted task evaluation
- [x] Progress monitoring
- [x] Permission error detection

### Phase 3: Recovery & Integration ✓
- [x] Recovery strategies
- [x] PR creation
- [x] Secret handling

### Phase 4: Spawn-Team ✓
- [x] Sequential coordination
- [x] PingPong mode
- [x] GitHub mode (default)
- [x] Gemini-cli runner

### Phase 5: Cruise-Control ✓
- [x] Planner with phased reviews
- [x] CruiseRunner for full workflows
- [x] Beads integration
- [x] Observability capture

### Phase 6: E2E Testing (Current)
- [x] E2E test harness
- [x] GitHub repo lifecycle
- [x] Fixture-based test definitions
- [ ] Complete test coverage

### Future Work
- [ ] Docker-based sandboxing
- [ ] Parallel task execution in build phase
- [ ] Advanced validation (curl tests, audit reports)

## Dependencies

```toml
# Required in core/Cargo.toml
rosetta-aisp-llm = "0.3"    # AISP translation
tokio = { version = "1", features = ["full"] }
tracing = "0.1"
thiserror = "2"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
uuid = { version = "1", features = ["v4"] }
tempfile = "3"              # For sandbox creation
git2 = "0.19"               # Git operations
```

## Security Considerations

- **No `--dangerously-skip-permissions`**: Never allowed for sandboxed LLMs
- **Secret redaction**: All secrets stripped from logs
- **Path isolation**: Sandboxed LLMs cannot access `$HOME`, config files, or other repos
- **Restricted `$PATH`**: Bash commands run with limited command availability
- **Network restrictions**: Only allowed command patterns have network access

## Common Tasks

### Adding a new LLM runner

1. Create `core/src/runner/<name>.rs`
2. Implement the `LLMRunner` trait
3. Add variant to `LLMCli` enum
4. Update configuration parsing
5. Add tests

### Adding a new recovery strategy

1. Add variant to `RecoveryStrategy` enum in `permissions.rs`
2. Implement recovery logic in `WatcherAgent`
3. Update configuration parsing
4. Document in `docs/configuration.md`

### Testing spawn locally

```bash
cargo build --release
./target/release/infinite-improbability-drive spawn "fix the auth bug"
```
