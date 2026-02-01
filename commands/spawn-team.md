---
name: spawn-team
description: Spawn with multi-LLM coordination (primary + reviewer)
usage: /spawn-team [--sequential|--ping-pong] [--max-iterations <n>] [--reviewer <llm>] "<prompt>"
---

# /spawn-team Command

Spawns a sandboxed LLM with multi-LLM coordination for code review.

## Synopsis

```
/spawn-team "<prompt>"
/spawn-team --ping-pong "<prompt>"
/spawn-team --sequential --reviewer gemini-cli "<prompt>"
/spawn-team --max-iterations 5 "<prompt>"
```

## Description

The `/spawn-team` command extends `/spawn` with multi-LLM coordination. After the primary LLM completes its task, a reviewer LLM evaluates the changes and provides feedback.

Two coordination modes are supported:

- **Sequential** (default): Primary completes → Reviewer reviews once → Primary fixes
- **Ping-pong**: Primary and reviewer alternate until approval or max iterations

## Options

- `--sequential`: Use sequential coordination (default)
- `--ping-pong`: Use iterative ping-pong coordination
- `--max-iterations <n>`: Maximum review iterations for ping-pong mode (default: 3)
- `--reviewer <llm>`: Reviewer LLM identifier (default: gemini-cli)
- `--primary <llm>`: Primary LLM identifier (default: claude-code)

All `/spawn` options are also supported:

- `--aisp`: Use AISP mode (structured prompt conversion)
- `--passthrough`: Pass prompt directly without conversion
- `--idle-timeout <seconds>`: Idle timeout (default: 120)
- `--total-timeout <seconds>`: Total timeout (default: 1800)

## Examples

```
# Basic spawn-team with defaults (sequential, claude-code primary, gemini-cli reviewer)
/spawn-team "implement the new authentication flow"

# Ping-pong mode with more iterations
/spawn-team --ping-pong --max-iterations 5 "refactor the payment module"

# Use specific LLMs
/spawn-team --primary gemini-cli --reviewer claude-code "optimize database queries"

# Combined with spawn options
/spawn-team --passthrough --total-timeout 3600 "major refactoring task"
```

## Coordination Modes

### Sequential Mode

```
Primary LLM → completes task
     ↓
Reviewer LLM → provides feedback
     ↓
Primary LLM → applies fixes
     ↓
PR created
```

Best for: Straightforward tasks where one review pass is sufficient.

### Ping-Pong Mode

```
Primary LLM → completes task
     ↓
Reviewer LLM → approves or requests changes
     ↓ (if changes needed)
Primary LLM → applies fixes
     ↓
Reviewer LLM → approves or requests changes
     ↓ (repeat until approved or max iterations)
PR created
```

Best for: Complex tasks requiring iterative refinement.

## Output

Returns a `SpawnTeamResult` with:

- Success status
- Number of iterations performed
- Final review verdict (Approved, NeedsChanges, Failed)
- All review results with suggestions
- Summary of the team operation
- PR URL (if created)

## Review Format

Reviewers respond with structured JSON:

```json
{
  "verdict": "approved" | "needs_changes",
  "suggestions": [
    {
      "file": "path/to/file",
      "line": 42,
      "issue": "description of issue",
      "suggestion": "how to fix it"
    }
  ]
}
```

## Configuration

Settings in `.infinite-probability/improbability-drive.toml`:

```toml
[spawn-team]
coordination = "sequential"
max_iterations = 3
reviewer_llm = "gemini-cli"
```

## See Also

- `/spawn` - Basic spawn without multi-LLM coordination
