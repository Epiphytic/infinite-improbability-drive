# infinite-improbability-drive

A Claude Code plugin for spawning sandboxed coding LLMs with intelligent resource provisioning and lifecycle management.

## What It Does

The infinite-improbability-drive enables you to delegate complex coding tasks to isolated LLM instances. When you spawn a sub-LLM:

- It runs in an isolated git worktree (no access to your home directory or other repos)
- A watcher agent monitors progress and recovers from permission errors
- Changes are automatically committed and a PR is created
- You get a summary with all changes made

## Quick Start

```bash
# Basic spawn
infinite-improbability-drive spawn "fix the auth bug"

# Use AISP mode for clearer LLM communication
infinite-improbability-drive spawn --aisp "implement user authentication"

# Spawn a team (primary + reviewer)
infinite-improbability-drive spawn-team "implement feature X"
```

## Documentation

### For Humans

| Document | Description |
|----------|-------------|
| [Architecture](./docs/architecture.md) | System design and component overview |
| [Configuration](./docs/configuration.md) | All configuration options explained |
| [Watcher Agent](./agents/watcher.md) | How the orchestration agent works |

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
| **Ping-pong** | Primary and reviewer alternate until approval |

## Development

See [CLAUDE.md](./CLAUDE.md) for development instructions, or [CLAUDE.aisp](./CLAUDE.aisp) for LLM-optimized instructions.

```bash
# Build
cargo build --release

# Test
cargo test

# Run
./target/release/infinite-improbability-drive spawn "your task"
```

## License

MIT
