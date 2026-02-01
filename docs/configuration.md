# Configuration Guide

> **Source:** [configuration.aisp](./configuration.aisp) — LLM-optimized specification

## Overview

Configuration is stored in `.infinite-probability/improbability-drive.toml` at the project root. CLI flags override file-based configuration.

## Configuration File

### Full Example

```toml
[spawn]
# Mode: use --aisp or --passthrough CLI flags to override
mode = "aisp"

# Recovery strategy: "moderate", "aggressive", "interactive"
recovery_strategy = "moderate"

# Timeouts in seconds
idle_timeout = 120      # 2 minutes no activity
total_timeout = 1800    # 30 minutes wall clock

# Default LLM for spawned instances
default_llm = "claude-code"  # or "gemini-cli"

# Logging
log_level = "standard"  # Success: standard, Failure: auto-escalates to full

[spawn.permissions]
# Default permission set for sandboxed LLMs
allowed_tools = ["Read", "Write", "Edit", "Glob", "Grep", "Bash"]
denied_tools = ["Task"]  # No recursive spawning by default
max_permission_escalations = 1  # For moderate mode

[spawn-team]
# Coordination mode: "sequential" or "ping-pong"
coordination = "sequential"

# Max iterations for ping-pong mode
max_iterations = 3

# Reviewer LLM (used after primary completes)
reviewer_llm = "gemini-cli"
```

## Spawn Section

### mode

Controls how prompts are processed before sending to the sandboxed LLM.

| Value | Description |
|-------|-------------|
| `"aisp"` | Translate prompt to AISP format for reduced ambiguity |
| `"passthrough"` | Send prompt as-is without translation |

**Default:** `"aisp"`

### recovery_strategy

Determines how permission errors are handled.

| Value | Description |
|-------|-------------|
| `"moderate"` | Attempt recovery up to `max_permission_escalations` times |
| `"aggressive"` | Keep attempting recovery until `CannotFix` |
| `"interactive"` | Pause and prompt user for each recovery decision |

**Default:** `"moderate"`

### idle_timeout

Seconds of inactivity before the spawn is terminated. This prevents hung processes while allowing thinking time.

**Default:** `120` (2 minutes)

### total_timeout

Maximum wall-clock seconds before the spawn is terminated, regardless of activity.

**Default:** `1800` (30 minutes)

### default_llm

Which LLM CLI to use for spawned instances.

| Value | Description |
|-------|-------------|
| `"claude-code"` | Use Claude Code CLI |
| `"gemini-cli"` | Use Gemini CLI |

**Default:** `"claude-code"`

### log_level

Controls log verbosity.

| Value | Description |
|-------|-------------|
| `"standard"` | Normal logging; auto-escalates to full on failure |
| `"full"` | Complete execution trace always captured |

**Default:** `"standard"`

## Permissions Section

### allowed_tools

List of tools the sandboxed LLM can use.

**Default:** `["Read", "Write", "Edit", "Glob", "Grep", "Bash"]`

### denied_tools

List of tools explicitly blocked. Takes precedence over `allowed_tools`.

**Default:** `["Task"]` (prevents recursive spawning)

### max_permission_escalations

Maximum number of times the watcher will attempt to fix permission errors and retry (for moderate strategy).

**Default:** `1`

## Spawn-Team Section

### coordination

How primary and reviewer LLMs interact.

| Value | Description |
|-------|-------------|
| `"sequential"` | Primary completes, then reviewer evaluates once |
| `"ping-pong"` | Primary and reviewer alternate until approval or max iterations |

**Default:** `"sequential"`

### max_iterations

Maximum rounds for ping-pong coordination.

**Default:** `3`

### reviewer_llm

Which LLM to use for code review in spawn-team mode.

**Default:** `"gemini-cli"`

## CLI Options

CLI flags override configuration file values.

### Mode Override

```bash
# Use AISP mode
infinite-improbability-drive spawn --aisp "implement feature X"

# Use passthrough mode
infinite-improbability-drive spawn --passthrough "simple fix"
```

### Timeout Override

```bash
infinite-improbability-drive spawn --idle-timeout 300 --total-timeout 3600 "big refactor"
```

### Recovery Override

```bash
infinite-improbability-drive spawn --max-permission-escalations 3 "complex task"
```

### Coordination Override

```bash
infinite-improbability-drive spawn-team --coordination ping-pong "implement feature X"
```

## Precedence

Configuration values are resolved in this order (highest priority first):

1. **CLI flags** — Always win
2. **Project config** — `.infinite-probability/improbability-drive.toml` in repo
3. **User config** — `~/.config/infinite-improbability-drive/config.toml`
4. **Defaults** — Built-in values

## Validation Rules

The configuration system enforces these constraints:

- `idle_timeout` must be greater than 0
- `total_timeout` must be greater than `idle_timeout`
- `max_permission_escalations` must be non-negative
- `max_iterations` must be at least 1
- `allowed_tools` and `denied_tools` must not overlap
- All tool names must be valid

## Related Documentation

- [Architecture](./architecture.md)
- [Watcher Agent](../agents/watcher.md)
