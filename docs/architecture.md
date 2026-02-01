# Architecture

> **Source:** [architecture.aisp](./architecture.aisp) — LLM-optimized specification

## Overview

The infinite-improbability-drive plugin enables host LLMs to spawn isolated sub-LLMs for delegated task execution. The architecture follows a watcher-agent pattern where a supervisory agent orchestrates the entire spawn lifecycle.

## System Architecture

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

## Core Components

### SpawnCommand

The CLI and skill entry point. Accepts a prompt and optional configuration overrides, then delegates to the watcher agent.

**Location:** `core/src/spawn.rs`

### WatcherAgent

The orchestration brain. It evaluates tasks using LLM-assisted analysis, provisions sandboxes, monitors execution, handles errors, and creates pull requests.

**Location:** `core/src/watcher.rs`
**Specification:** [agents/watcher.aisp](../agents/watcher.aisp) | [agents/watcher.md](../agents/watcher.md)

### SandboxProvider

A trait that abstracts the isolation mechanism. Currently implemented using git worktrees, with Docker/Podman support planned for the future.

**Location:** `core/src/sandbox/provider.rs`

### LLMRunner

Launches target LLM CLIs (Claude Code or Gemini CLI) in streaming mode and captures output.

**Location:** `core/src/runner/`

### ProgressMonitor

Tracks file changes, commits, output lines, and detects timeouts based on activity or wall-clock time.

**Location:** `core/src/monitor.rs`

### PermissionDetector

Pattern-matches common permission errors and computes appropriate fixes for the recovery system.

**Location:** `core/src/permissions.rs`

### PRManager

Creates pull requests from worktree branches and handles merge conflicts using either auto-resolution or a repair sandbox.

**Location:** `core/src/pr.rs`

### SecretsManager

Injects secrets as environment variables and redacts them from all log output.

**Location:** `core/src/secrets.rs`

## Data Flow

### Spawn Flow

1. **Request received** — Host LLM invokes `/spawn` with a prompt
2. **Evaluation** — Watcher agent analyzes the task to create a sandbox manifest
3. **Provisioning** — Git worktree created with appropriate permissions
4. **Execution** — Target LLM launched in sandbox with streaming output
5. **Monitoring** — Progress tracked, errors detected, recovery attempted
6. **Integration** — Changes committed, PR created
7. **Reporting** — Summary returned to host LLM

### Recovery Flow

```
Permission error detected
        │
        ▼
┌─────────────────────────┐
│ Analyze error type      │
│ Match against patterns  │
└───────────┬─────────────┘
            │
            ▼
    Can we fix it?
     /          \
   Yes           No
    │             │
    ▼             ▼
Escalation    Kill sandbox
count < max?  Report to host
 /      \
Yes      No
 │        │
 ▼        ▼
Apply fix  Kill sandbox
& retry   Report to host
```

## Type System

### Core Types

| Type | Description |
|------|-------------|
| `SpawnRequest` | Input containing prompt, mode, and configuration |
| `Mode` | Either `AISP` (translated) or `Passthrough` (direct) |
| `SandboxManifest` | Permissions, paths, tools, commands, and secrets |
| `SpawnResult` | Output containing status, changes, PR URL, and logs |

### Error Types

| Type | Description |
|------|-------------|
| `PermissionErrorType` | File access, command, tool, or network denials |
| `PermissionFix` | Computed remediation for permission errors |
| `SpawnError` | Top-level error algebra for spawn operations |

## Isolation Guarantees

The sandbox provides these security guarantees:

- **No home directory access** — `$HOME` is inaccessible
- **No config file access** — User configuration files are hidden
- **No cross-repo access** — Only the target worktree is visible
- **No dangerous flags** — `--dangerously-skip-permissions` is never allowed
- **Secret redaction** — All secrets are stripped from logs
- **Restricted PATH** — Only allowed commands are available

## Metrics

### Default Timeouts

| Metric | Default | Description |
|--------|---------|-------------|
| Idle timeout | 120s | No activity triggers termination |
| Total timeout | 1800s | Maximum wall-clock time |

### Recovery Limits

| Strategy | Max Escalations |
|----------|-----------------|
| Moderate | 1 |
| Aggressive | Unlimited |
| Interactive | User-controlled |

## Related Documentation

- [Configuration Guide](./configuration.md)
- [Watcher Agent](../agents/watcher.md)
