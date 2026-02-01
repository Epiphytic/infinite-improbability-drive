# Watcher Agent

> **Source:** [watcher.aisp](./watcher.aisp) — LLM-optimized specification

## Overview

The Watcher Agent is the orchestration brain of the spawn system. It manages the entire lifecycle of a spawned LLM instance, from task evaluation through PR creation.

## Responsibilities

- **Evaluate** task requirements using LLM-assisted analysis
- **Provision** isolated sandbox environments (git worktrees)
- **Execute** target LLM CLI in streaming mode
- **Monitor** progress, detect hangs, and track file changes
- **Recover** from permission errors using configurable strategies
- **Integrate** changes via PR creation
- **Report** results back to the host LLM

## Lifecycle Phases

```
Evaluate → Provision → Execute → Monitor → Recover → Integrate → Report
```

### Phase 1: Evaluate

The watcher analyzes the spawn request to create a `SandboxManifest`:

```rust
SandboxManifest {
    readable_paths: Set<PathPattern>,   // Files the LLM can read
    writable_paths: Set<PathPattern>,   // Files the LLM can modify
    allowed_tools: Set<ToolName>,       // Tools available
    allowed_commands: Set<CommandPattern>, // Bash commands allowed
    environment: Map<String, String>,   // Environment variables
    secrets: Set<SecretRef>,            // Secrets to inject
    complexity: TaskComplexity,         // For timeout tuning
}
```

The evaluation uses LLM-assisted inference to:
- Assess task complexity (Low, Medium, High)
- Identify required file paths based on the prompt
- Determine necessary tools and commands
- Detect secrets that need injection

### Phase 2: Provision

Creates the isolated execution environment:

1. Create a new branch: `spawn/{uuid}`
2. Create a git worktree from that branch
3. Apply the manifest permissions
4. Inject secrets as environment variables

### Phase 3: Execute

Launches the target LLM in the sandbox:

```rust
LLMSpawnConfig {
    cli: LLMTarget,          // ClaudeCode or GeminiCli
    prompt: String,          // Original or AISP-converted
    worktree_path: PathBuf,  // Isolated directory
    manifest: SandboxManifest,
    session_history: false,  // No persistence
    headless: true,          // No interactive prompts
    streaming: true,         // Real-time output capture
}
```

### Phase 4: Monitor

Tracks execution state in real-time:

```rust
ProgressState {
    files_read: Set<Path>,
    files_written: Set<Path>,
    commits_made: Vec<CommitInfo>,
    output_lines: usize,
    last_activity: Instant,
    start_time: Instant,
    permission_errors: Vec<PermissionError>,
    other_errors: Vec<String>,
}
```

**Timeout Detection:**
- **Idle timeout** — No activity for `idle_timeout` seconds
- **Total timeout** — Wall-clock time exceeds `total_timeout`

### Phase 5: Recover

When permission errors are detected, the watcher attempts recovery based on the configured strategy.

#### Moderate Strategy (Default)

```
if escalation_count >= max_permission_escalations:
    return Error
else:
    fix = compute_fix(error)
    if fix == CannotFix:
        return Error
    else:
        apply_fix(fix, manifest)
        retry_execution()
```

#### Aggressive Strategy

Same as moderate but ignores escalation limits—keeps trying until `CannotFix`.

#### Interactive Strategy

Pauses execution and prompts the user for each recovery decision.

### Phase 6: Integrate

After successful execution:

1. Detect uncommitted changes and commit them
2. Push the branch to origin
3. Create a PR targeting the original branch
4. Handle merge conflicts:
   - **Small conflicts** — Auto-resolve
   - **Large conflicts** — Spawn a repair sandbox

### Phase 7: Report

Generate a summary for the host LLM:

```markdown
## Spawn Complete: fix-auth-bug (spawn-id: abc123)

**Status:** Success (3m 42s)
**PR:** #147 - https://github.com/org/repo/pull/147

### Changes
- `src/auth/token.rs` (+45, -12) - Added token expiry validation
- `src/auth/middleware.rs` (+8, -2) - Updated error handling
- `tests/auth_test.rs` (+67, -0) - Added expiry test cases

### Summary
Implemented token expiry checking in the auth middleware. Added
TokenExpired error variant and corresponding test coverage.

### Logs
Debug logs: `.improbability-drive/spawns/abc123/`
```

## Events

The watcher emits structured events throughout execution:

| Event | Description |
|-------|-------------|
| `SandboxCreated` | Worktree provisioned successfully |
| `LLMStarted` | Target LLM process launched |
| `ProgressUpdate` | Periodic state snapshot |
| `PermissionError` | Error detected with computed fix |
| `RecoveryAttempt` | Fix applied, retrying execution |
| `LLMCompleted` | Target LLM finished (success/failure) |
| `PRCreated` | Pull request created |

## Available Tools

### Default Allowed

- `Read` — Read file contents
- `Write` — Create new files
- `Edit` — Modify existing files
- `Glob` — Find files by pattern
- `Grep` — Search file contents
- `Bash` — Execute shell commands

### Default Denied

- `Task` — Prevents recursive spawning

## Performance Targets

| Metric | Target |
|--------|--------|
| Evaluation time | < 5 seconds |
| Provision time | < 10 seconds |
| PR creation time | < 30 seconds |
| Success rate | ≥ 85% |

## Related Documentation

- [Architecture](../docs/architecture.md)
- [Configuration](../docs/configuration.md)
