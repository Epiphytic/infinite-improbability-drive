---
name: spawn-team
description: Spawn a sandboxed LLM with multi-LLM coordination for automated code review
---

# Spawn-Team Skill

Launches an isolated LLM instance with coordinated review from a secondary LLM. This enables automated code review workflows where one LLM implements changes and another reviews them.

## Usage

When you need delegated task execution with automated code review:

1. Describe the task in the prompt
2. The primary LLM (default: claude-code) implements the changes
3. The reviewer LLM (default: gemini-cli) evaluates the changes
4. Based on coordination mode, either:
   - **Sequential**: Primary applies fixes once, then PR is created
   - **Ping-pong**: Primary and reviewer iterate until approval

## Invocation

```
/spawn-team "implement the new authentication flow"
```

With coordination mode:

```
/spawn-team --sequential "straightforward feature"
/spawn-team --ping-pong "complex refactoring"
```

With custom LLMs:

```
/spawn-team --reviewer claude-code "use Claude as reviewer"
/spawn-team --primary gemini-cli --reviewer claude-code "swap roles"
```

## Coordination Modes

### Sequential (Default)

Primary LLM completes the task, reviewer provides feedback, primary applies fixes once. Best for straightforward tasks.

```
Primary → Reviewer → Primary (fixes) → PR
```

### Ping-Pong

Primary and reviewer alternate until the reviewer approves or max iterations reached. Best for complex tasks requiring iteration.

```
Primary → Reviewer → Primary → Reviewer → ... → PR
```

Control iterations:

```
/spawn-team --ping-pong --max-iterations 5 "complex task"
```

## Supported LLMs

| Identifier | Description |
|------------|-------------|
| `claude-code` | Claude Code CLI (default primary) |
| `gemini-cli` | Gemini CLI (default reviewer) |

## Configuration

Settings in `.infinite-probability/improbability-drive.toml`:

```toml
[spawn-team]
coordination = "sequential"  # or "ping-pong"
max_iterations = 3
reviewer_llm = "gemini-cli"

[spawn]
default_llm = "claude-code"
```

## Review Process

The reviewer LLM receives:

1. The original task prompt
2. A git diff of the changes made
3. Instructions to respond with structured JSON

Review response format:

```json
{
  "verdict": "approved",
  "suggestions": []
}
```

Or with suggestions:

```json
{
  "verdict": "needs_changes",
  "suggestions": [
    {
      "file": "src/auth.rs",
      "line": 42,
      "issue": "Missing error handling",
      "suggestion": "Add Result return type"
    }
  ]
}
```

## Fix Process

When the reviewer returns `needs_changes`, the primary LLM receives:

1. The original task prompt
2. The list of issues to fix with file paths and suggestions
3. Instructions to address each issue

## Output

Returns a `SpawnTeamResult` containing:

- `success`: Whether the team operation succeeded
- `iterations`: Number of review iterations performed
- `final_verdict`: Approved, NeedsChanges, or Failed
- `reviews`: All review results with suggestions
- `summary`: Human-readable summary

## When to Use Spawn-Team

**Use spawn-team when:**
- Task requires validation beyond basic completion
- You want automated code review before PR creation
- Task is complex enough to benefit from a second perspective
- You want to catch issues before human review

**Use basic spawn when:**
- Task is simple and well-defined
- Speed is more important than review
- You'll do manual code review anyway

## Safety

Same isolation guarantees as `/spawn`:

- Sandbox runs in isolated git worktree
- No access to `$HOME` or config files
- `--dangerously-skip-permissions` never allowed
- Secrets injected as env vars, never logged
- Both primary and reviewer run in sandboxes

## Example Workflow

```
User: /spawn-team --ping-pong "add rate limiting to the API"

Spawn-team:
1. Primary (claude-code) implements rate limiting
2. Reviewer (gemini-cli) finds edge case: "No handling for burst traffic"
3. Primary addresses feedback, adds burst handling
4. Reviewer approves changes
5. PR created with 2 review iterations documented

Result:
- Status: Success
- Iterations: 2
- Final verdict: Approved
- PR: https://github.com/repo/pull/123
```
