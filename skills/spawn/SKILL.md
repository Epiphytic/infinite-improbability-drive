---
name: spawn
description: Spawn a sandboxed LLM instance to work on a delegated task
---

# Spawn Skill

Launches an isolated LLM instance in a git worktree sandbox to handle complex tasks without polluting the host LLM's context.

## Usage

When you need to delegate a task to an isolated LLM:

1. Describe the task in the prompt
2. The watcher agent will evaluate the task and provision appropriate resources
3. A sandboxed LLM will execute the task
4. Results are returned as a PR with a summary

## Invocation

```
/spawn "fix the authentication bug in src/auth.rs"
```

Or with mode override:

```
/spawn --passthrough "simple typo fix"
/spawn --aisp "complex refactoring task"
```

## Modes

- **AISP** (default): Converts prompt to structured AISP format for better LLM comprehension
- **Passthrough**: Sends prompt directly without conversion

## Configuration

Settings in `.infinite-probability/improbability-drive.toml`:

```toml
[spawn]
mode = "aisp"
recovery_strategy = "moderate"
idle_timeout = 120
total_timeout = 1800
default_llm = "claude-code"
```

## What Happens

1. **Evaluation**: Watcher agent analyzes the task to determine required permissions
2. **Provisioning**: Git worktree created with appropriate isolation
3. **Execution**: Target LLM runs in sandbox with limited permissions
4. **Monitoring**: Progress tracked, errors detected, recovery attempted
5. **Integration**: Changes committed and PR created
6. **Reporting**: Summary returned to host LLM

## Safety

- Sandbox runs in isolated git worktree
- No access to `$HOME` or config files
- `--dangerously-skip-permissions` is never allowed
- Secrets injected as env vars, never logged
- Restricted `$PATH` for commands
