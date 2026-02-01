---
name: spawn
description: Spawn a sandboxed LLM instance
usage: /spawn [--aisp|--passthrough] [--idle-timeout <secs>] [--total-timeout <secs>] "<prompt>"
---

# /spawn Command

Spawns a sandboxed LLM instance to work on a delegated task.

## Synopsis

```
/spawn "<prompt>"
/spawn --passthrough "<prompt>"
/spawn --aisp "<prompt>"
/spawn --idle-timeout 300 --total-timeout 3600 "<prompt>"
```

## Description

The `/spawn` command launches an isolated LLM instance in a git worktree sandbox. The spawned LLM has limited permissions and cannot access files outside its worktree or use dangerous flags.

A watcher agent monitors the spawned LLM, handles permission errors, and creates a pull request when the work is complete.

## Options

- `--aisp`: Use AISP mode (structured prompt conversion) - default
- `--passthrough`: Pass prompt directly without conversion
- `--idle-timeout <seconds>`: Idle timeout before termination (default: 120)
- `--total-timeout <seconds>`: Total wall-clock timeout (default: 1800)
- `--max-permission-escalations <n>`: Max recovery attempts (default: 1)

## Examples

```
/spawn "fix the authentication bug"
/spawn --passthrough "update the README"
/spawn --idle-timeout 300 "refactor the database layer"
```

## Output

Returns a `SpawnResult` with:

- Status (Success, Failed, TimedOut)
- Spawn ID
- Duration
- Files changed
- Commits made
- Summary
- PR URL (if created)
- Log file paths

## See Also

- `/spawn-team` - Spawn with multi-LLM coordination
