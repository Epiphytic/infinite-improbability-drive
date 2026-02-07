# Design: Persistent Phase Sandbox with Async Comment Handling

**Date:** 2026-02-04
**Status:** Approved
**Author:** Claude (brainstormed with user)

## Problem Statement

The current sandbox architecture creates a new worktree per LLM invocation and cleans it up when the process exits. This breaks down for cruise-control workflows where:

1. Multiple LLMs (planner, reviewer, fixer) need to collaborate on the same files
2. Review comments arrive asynchronously over time
3. Human reviewers may add comments hours after automated review

## Solution Overview

Introduce a **persistent phase sandbox** for cruise-control that lives until PR merge/close or timeout, while preserving the existing transient sandbox behavior for `spawn` and `spawn-team` commands.

## Sandbox Modes

### Transient (existing, unchanged)

Used by `spawn` and `spawn-team` commands:
- Cleans up automatically when LLM process exits
- Current `Drop` implementation preserved

```rust
let sandbox = provider.create(manifest)?;  // Cleans up on drop
```

### Persistent (new for cruise-control)

Used by `cruise-control` phases:
- Lives until PR merge/close or 24h timeout
- Multiple LLM invocations share the same sandbox

```rust
let phase_sandbox = PhaseSandbox::new(provider, manifest, branch_name)?;
phase_sandbox.spawn_planner(prompt).await?;
phase_sandbox.spawn_reviewer(domain, prompt).await?;
phase_sandbox.spawn_fixer(comments).await?;
// ... sandbox persists between calls ...
phase_sandbox.cleanup();  // Explicit cleanup only
```

## Sandbox Lifecycle

### Creation
- Created when planning phase starts
- Branch name from extracted metadata (e.g., `feat/add-user-auth`)
- State written to `.cruise/phase-state.json` for crash recovery

### Persistence
- NOT cleaned up when LLM processes exit
- Multiple LLM invocations use the same worktree path
- State persists in git (commits pushed to remote)

### Cleanup Triggers
1. PR merged or closed → immediate cleanup
2. 24-hour inactivity timeout → cleanup + PR comment
3. Manual `cruise cleanup` command → cleanup

### Recovery
- On watcher crash/restart, reads `.cruise/phase-state.json`
- Resumes monitoring PR and spawning fixer rounds

## LLM Invocation Flow

Each LLM is a short-lived process with a specific role. All share the same sandbox path.

### Planning Phase Flow

```
1. Planner (Claude)
   - Prompt: "Create a plan for: {task}"
   - Writes: plan.md, commits, pushes
   - Exits when done

2. Reviewer (Gemini) - READ ONLY
   - CLI args enforce read-only (--allowed-tools Read,Glob,Grep,Bash)
   - Different prompt per domain (Security, TechnicalFeasibility, etc.)
   - Reads plan.md from sandbox
   - Posts comments to GitHub PR
   - Signals completion via exit or [REVIEW COMPLETE] comment

3. Fixer (Claude) - triggered per round
   - Prompt: "Address these review comments: {comments}"
   - Includes: path to plan.md, comment links/content, CLI-injected comments
   - Edits files, commits, pushes
   - Exits when done

4. Repeat 2-3 until approved or max rounds
```

### Planner vs Fixer

Same LLM (Claude), different prompts. Both write to the same sandbox. The distinction is purely about role:
- **Planner**: Creates initial artifacts
- **Fixer**: Addresses review feedback

## Watcher & Comment Monitoring

### Watcher Responsibilities
- Creates and owns the persistent sandbox
- Spawns planner, reviewer, and fixer LLM processes
- Monitors Gemini process state (running vs exited)
- Polls GitHub for new PR comments
- Triggers fixer rounds when review batches complete

### Comment Monitoring States

```
┌─────────────────────────────────────────────────────────┐
│                    REVIEWER ACTIVE                       │
│  Gemini process running, comments passed directly        │
│  to fixer queue. No polling needed.                      │
└──────────────────────┬──────────────────────────────────┘
                       │ Process exits OR [REVIEW COMPLETE]
                       ▼
┌─────────────────────────────────────────────────────────┐
│                    FIXER ROUND                           │
│  Spawn Claude with pending comments. Wait for exit.      │
└──────────────────────┬──────────────────────────────────┘
                       │ Fixer exits
                       ▼
┌─────────────────────────────────────────────────────────┐
│                  BACKOFF POLLING                         │
│  Poll every 5s → 10s → 20s → ... → 5min (max)           │
│  New comment resets to 5s. `cruise fix` forces poll.    │
└──────────────────────┬──────────────────────────────────┘
                       │ New comment detected
                       ▼
                  [FIXER ROUND]
```

### Backoff Reset Conditions
- New comment detected → reset to 5s
- Manual `cruise fix` → immediate poll, reset to 5s
- PR merged/closed → exit monitoring

### Review Completion Signals
Either of these triggers a fixer round:
1. Gemini process exits
2. `[REVIEW COMPLETE]` comment appears on PR

## CLI Commands

### Existing (unchanged)
- `spawn` - transient sandbox, cleans up on exit
- `spawn-team` - transient sandbox, cleans up on exit

### New/Modified
- `cruise fix` - trigger immediate poll + fixer round if comments pending
- `cruise fix --comment "Fix X"` - inject comment directly + trigger fixer
- `cruise cleanup` - force sandbox cleanup
- `cruise resume` - resume monitoring existing sandbox after crash

## Implementation Changes

### 1. Sandbox persistence (`sandbox/worktree.rs`)
- Keep existing auto-cleanup behavior (transient mode)
- Add flag to disable auto-cleanup for persistent mode

### 2. New `PhaseSandbox` (`sandbox/phase.rs`)
- Wrapper that owns worktree for entire phase
- Tracks: PR URL, comment state, backoff timer, last activity
- Provides `spawn_llm(role, prompt)` reusing same path
- Writes state to `.cruise/phase-state.json`
- Handles cleanup triggers (PR close, timeout, manual)

### 3. Watcher changes (`watcher.rs` / `team_orchestrator.rs`)
- Add comment polling loop with exponential backoff
- Track reviewer process state (running/exited)
- Queue comments during "fast and furious" phase
- Trigger fixer rounds on batch completion

### 4. CLI additions
- `cruise fix [--comment "..."]`
- `cruise cleanup`
- `cruise resume`

## Error Handling

### Watcher crash/restart
- Read state from `.cruise/phase-state.json`
- Resume polling from current state

### Fixer fails mid-round
- Partial commits preserved in git
- Next fixer round continues from current state
- No rollback needed

### Reviewer times out or crashes
- Treat as implicit review complete
- Trigger fixer round with comments posted so far
- Log warning about incomplete review

### PR closed externally
- Polling detects PR state change
- Trigger cleanup
- Log completion status

### 24h timeout reached
- Cleanup sandbox
- Post comment: "Cruise-control session timed out after 24h of inactivity"
- Leave PR open for human action

## Data Structures

### Phase State File (`.cruise/phase-state.json`)

```json
{
  "sandbox_path": "/tmp/improbability-drive-sandboxes/feat-add-auth-abc123",
  "branch_name": "feat/add-user-auth",
  "pr_url": "https://github.com/owner/repo/pull/123",
  "pr_number": 123,
  "phase": "planning",
  "current_review_domain": "Security",
  "last_activity": "2026-02-04T10:30:00Z",
  "backoff_interval_secs": 5,
  "pending_comment_ids": [456, 457],
  "completed_rounds": 2
}
```

### PhaseSandbox struct

```rust
pub struct PhaseSandbox<P: SandboxProvider> {
    provider: P,
    worktree_path: PathBuf,
    branch_name: String,
    pr_url: Option<String>,
    pr_number: Option<u64>,
    last_activity: Instant,
    backoff: ExponentialBackoff,
    pending_comments: Vec<CommentInfo>,
    state_file: PathBuf,
}
```

## Testing Strategy

1. **Unit tests**: PhaseSandbox state management, backoff logic
2. **Integration tests**: Multi-LLM invocation on same sandbox
3. **E2E tests**: Full planning workflow with simulated review comments
