# infinite-improbability-drive

A Claude Code plugin for spawning sandboxed coding LLMs with intelligent resource provisioning and lifecycle management.

## What It Does

The infinite-improbability-drive enables you to delegate complex coding tasks to isolated LLM instances. When you spawn a sub-LLM:

- It runs in an isolated git worktree (no access to your home directory or other repos)
- A watcher agent monitors progress and recovers from permission errors
- Changes are automatically committed and a PR is created
- You get a summary with all changes made

## Spawn Modes

### Single Spawn
Delegate a task to a single LLM instance running in isolation.

### Spawn-Team (PingPong)
Multi-LLM coordination with Claude as primary coder and Gemini as reviewer, iterating until approval.

### Spawn-Team (GitHub)
PR-based coordination where reviews are posted as GitHub comments with full traceability.

## Cruise-Control

An autonomous development orchestrator using a three-phase workflow:

1. **Plan** - Generate dependency-aware plans using spawn-team ping-pong
2. **Build** - Execute plans with parallel task execution
3. **Validate** - Audit implementation against the plan

See [Cruise-Control Architecture](./docs/cruise-control.md) for details.

## Quick Start

```bash
# Basic spawn
infinite-improbability-drive spawn "fix the auth bug"

# Use AISP mode for clearer LLM communication
infinite-improbability-drive spawn --aisp "implement user authentication"

# Spawn a team (primary + reviewer)
infinite-improbability-drive spawn-team "implement feature X"

# Full cruise-control workflow
infinite-improbability-drive cruise "build a REST API with auth"
```

## Documentation

### For Humans

| Document | Description |
|----------|-------------|
| [Architecture](./docs/architecture.md) | Spawn system design and component overview |
| [Spawn-Team](./docs/spawn-team.md) | Multi-LLM coordination modes |
| [Cruise-Control](./docs/cruise-control.md) | Autonomous orchestrator architecture |
| [Configuration](./docs/configuration.md) | All configuration options explained |
| [Watcher Agent](./agents/watcher.md) | How the orchestration agent works |
| [Agents](./AGENTS.md) | Multi-agent coordination patterns |

### Design Documents

| Document | Description |
|----------|-------------|
| [Cruise-Control Design](./docs/plans/2026-02-01-cruise-control-design.md) | Main cruise-control design |
| [Planner Design](./docs/plans/2026-02-01-cruise-planner-design.md) | Plan phase design |
| [E2E Testing Design](./docs/plans/2026-02-02-e2e-testing-design.md) | End-to-end testing infrastructure |

### For LLMs (AISP Format)

| Document | Description |
|----------|-------------|
| [CLAUDE.aisp](./CLAUDE.aisp) | Development instructions |
| [architecture.aisp](./docs/architecture.aisp) | Type system and data flow |
| [configuration.aisp](./docs/configuration.aisp) | Config schema specification |
| [watcher.aisp](./agents/watcher.aisp) | Agent behavior specification |

## How It Works

```
┌─────────────────────────────────────────────────────────────┐
│                     Host LLM Session                        │
│  (receives spawn request, gets summary results)             │
└─────────────────────┬───────────────────────────────────────┘
                      │ spawn command
                      ▼
┌─────────────────────────────────────────────────────────────┐
│                   LLM Watcher Agent                         │
│  - Evaluates task requirements (LLM-assisted)               │
│  - Provisions sandbox (git worktree)                        │
│  - Monitors progress (files, commits, output)               │
│  - Detects errors & manages recovery                        │
│  - Creates PR & summary on completion                       │
└─────────────────────┬───────────────────────────────────────┘
                      │ provisions & monitors
                      ▼
┌─────────────────────────────────────────────────────────────┐
│                  Sandboxed LLM Instance                     │
│  - Runs in isolated git worktree                            │
│  - Headless, streaming mode, no session history             │
│  - Limited permissions (no --dangerously-skip-permissions)  │
│  - Output piped to debug files                              │
└─────────────────────────────────────────────────────────────┘
```

## Configuration

Create `.infinite-probability/improbability-drive.toml` in your project:

```toml
[spawn]
mode = "aisp"                    # or "passthrough"
recovery_strategy = "moderate"   # "moderate", "aggressive", "interactive"
idle_timeout = 120               # 2 minutes no activity
total_timeout = 1800             # 30 minutes max
default_llm = "claude-code"      # or "gemini-cli"

[spawn.permissions]
allowed_tools = ["Read", "Write", "Edit", "Glob", "Grep", "Bash"]
denied_tools = ["Task"]
max_permission_escalations = 1

[spawn-team]
coordination = "sequential"      # or "ping-pong"
max_iterations = 3
reviewer_llm = "gemini-cli"
```

See [Configuration Guide](./docs/configuration.md) for all options.

## Security

The sandbox enforces strict isolation:

- **No home directory access** — `$HOME` is inaccessible
- **No config file access** — Your dotfiles are protected
- **No cross-repo access** — Only the target worktree is visible
- **No dangerous flags** — `--dangerously-skip-permissions` blocked
- **Secret redaction** — All secrets stripped from logs
- **Restricted PATH** — Only allowed commands available

## Recovery Strategies

| Strategy | Behavior |
|----------|----------|
| **Moderate** | Attempt 1 recovery before failing |
| **Aggressive** | Keep trying until unfixable error |
| **Interactive** | Ask user for each recovery decision |

## Spawn-Team Modes

| Mode | Description |
|------|-------------|
| **Sequential** | Primary LLM completes, reviewer evaluates once |
| **PingPong** | Primary and reviewer alternate until approval |
| **GitHub** | PR-based coordination with GitHub reviews (default) |

### GitHub Mode

The default coordination mode uses GitHub PRs for communication:

1. PR is created on first commit
2. Reviewer LLMs post PR comments with findings
3. Coder LLM resolves comments with commits
4. Full traceability on the PR

## Development

See [CLAUDE.md](./CLAUDE.md) for development instructions, or [CLAUDE.aisp](./CLAUDE.aisp) for LLM-optimized instructions.

### Test-Driven Development (Required)

**All development on this project MUST follow Test-Driven Development:**

1. **Write failing tests first** - Define expected behavior before implementation
2. **Implement minimal code to pass** - Only write enough code to make tests green
3. **Refactor while maintaining green tests** - Improve code quality with test safety net

**No feature is considered complete until it has provable, verifiable tests that it works.**

### Verification Before Completion

Before any work is considered done:

- [ ] Unit tests exist and pass
- [ ] Integration tests verify component interactions
- [ ] E2E tests validate full workflows (where applicable)
- [ ] `cargo test` passes with no failures
- [ ] Documentation updated if behavior changed

```bash
# Build
cargo build --release

# Test (required before any PR)
cargo test

# Run E2E tests
cargo test --test e2e

# Run
./target/release/infinite-improbability-drive spawn "your task"
```

## License

MIT
