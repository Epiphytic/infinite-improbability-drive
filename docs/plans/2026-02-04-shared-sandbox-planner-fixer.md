# Design: Shared Sandbox and Planner/Fixer Separation

**Date:** 2026-02-04
**Status:** In Progress (Task 1 Complete)
**Author:** Claude

## Problem Statement

The current spawn-team architecture has two issues:

1. **Sandbox isolation problem**: Claude and Gemini each get their own sandbox/worktree. When Claude creates a plan file and Gemini tries to review it, they're looking at different filesystems. This causes confusion where they argue about file presence.

2. **PR metadata problem**: The PR title and branch name are derived from the raw task description, resulting in ugly/meaningless names like `spawn-team-fix-the-auth-bug-1234abcd`.

3. **Context inefficiency**: The planner LLM is invoked multiple times with accumulated context, when it would be cleaner to have:
   - A planner that creates the initial plan and exits
   - A fixer that responds to review feedback with fresh context

## Proposed Solution

### 1. LLM-Extracted PR Metadata

Before starting the spawn-team workflow, use an LLM to extract:
- A concise, meaningful PR title (max 72 chars)
- A branch name (kebab-case, max 50 chars)

```rust
pub struct ExtractedMetadata {
    pub pr_title: String,
    pub branch_name: String,
}

impl SpawnTeamOrchestrator {
    async fn extract_metadata(&self, task: &str) -> Result<ExtractedMetadata> {
        // Use lightweight LLM call to extract PR title and branch name
        let prompt = format!(
            "Extract a concise PR title and branch name from this task:\n\n{}\n\n\
             Respond in JSON format:\n\
             {{\"pr_title\": \"...\", \"branch_name\": \"...\"}}",
            task
        );
        // ... run extraction
    }
}
```

### 2. Shared Sandbox Architecture

All LLM instances in a phase share the same sandbox:

```
Before (broken):
┌──────────────────┐     ┌──────────────────┐
│ Claude Sandbox   │     │ Gemini Sandbox   │
│ /tmp/worktree-1  │     │ /tmp/worktree-2  │
│ - creates plan   │ --> │ - can't see plan │
└──────────────────┘     └──────────────────┘

After (fixed):
┌─────────────────────────────────────────────┐
│            Shared Phase Sandbox              │
│            /tmp/worktree-phase-1             │
│                                              │
│  Claude writes → plan.md → Gemini reads     │
└─────────────────────────────────────────────┘
```

Changes needed:
- `SpawnTeamOrchestrator` creates ONE sandbox for the entire phase
- Pass the same `worktree_path` to all LLM invocations
- Remove per-LLM sandbox creation

### 3. Planner/Fixer Separation

Split the planning workflow into two distinct LLM roles:

```
┌─────────────────────────────────────────────────────────────────────┐
│                         PLANNING PHASE                              │
├─────────────────────────────────────────────────────────────────────┤
│                                                                     │
│  ┌─────────────┐     Creates plan.md      ┌─────────────────────┐  │
│  │   Planner   │ ────────────────────────>│   Shared Sandbox    │  │
│  │   (Claude)  │     Exits                │   /tmp/worktree     │  │
│  └─────────────┘                          └─────────────────────┘  │
│                                                    │                │
│                                                    ▼                │
│  ┌─────────────┐     Reviews plan.md      ┌─────────────────────┐  │
│  │  Reviewer   │ <───────────────────────│   PR + Comments     │  │
│  │  (Gemini)   │     Posts comments       │   on GitHub         │  │
│  └─────────────┘                          └─────────────────────┘  │
│                                                    │                │
│                            Watcher monitors for comments           │
│                                                    │                │
│                                                    ▼                │
│  ┌─────────────┐     Reads plan + comments ┌─────────────────────┐ │
│  │   Fixer     │ <────────────────────────│   Comment Link      │ │
│  │  (Claude)   │     Addresses feedback    │   (from Watcher)    │ │
│  │  [FRESH]    │                          └─────────────────────┘  │
│  └─────────────┘                                                   │
│        │                                                           │
│        │ Stays active, receives new comments via Watcher           │
│        ▼                                                           │
│  ┌─────────────┐                                                   │
│  │  Iteration  │ Loop until approved or max iterations             │
│  └─────────────┘                                                   │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
```

#### Fixer LLM Context

The fixer receives:
1. Path to `plan.md` in the shared sandbox
2. Link to the first review comment from Gemini
3. Fresh context (no accumulated conversation history)

The watcher:
1. Monitors PR for new comments
2. Feeds comments to fixer asynchronously
3. Fixer addresses each comment and pushes changes

### 4. Implementation Plan

#### Task 1: Extract PR Metadata ✅ COMPLETE
- ✅ Added `ExtractedMetadata` struct to `team.rs`
- ✅ Added `extract_metadata()` method to `SpawnTeamOrchestrator`
- ✅ Call before creating sandbox/branch in `run_with_branch()`
- ✅ Use extracted values for PR creation in `create_pr_on_first_commit()`
- ✅ Updated tests in `config.rs` for new struct fields
- ✅ All 217 tests passing

#### Task 2: Shared Sandbox
- Modify `SpawnTeamOrchestrator::run_github_mode()` to create ONE sandbox
- Pass `worktree_path` to all LLM invocations
- Ensure both Claude and Gemini see the same filesystem

#### Task 3: Planner/Fixer Split
- Extract planner logic into `run_planner()` method
- Create new `run_fixer()` method with fresh context
- Fixer receives: plan file path, first comment link
- Watcher feeds subsequent comments to fixer

#### Task 4: Asynchronous Comment Handling
- Implement comment polling in watcher
- Create channel for watcher → fixer communication
- Fixer processes comments as they arrive

### 5. File Changes

| File | Change |
|------|--------|
| `core/src/team_orchestrator.rs` | Add metadata extraction, shared sandbox, planner/fixer split |
| `core/src/team.rs` | Add `ExtractedMetadata` struct |
| `core/src/watcher.rs` | Add comment monitoring for fixer mode |
| `core/src/runner/mod.rs` | Ensure runners can use external sandbox path |

### 6. Open Questions

1. **Metadata extraction model**: Should we use the same model as the primary LLM, or a lightweight model like Haiku?

2. **Fixer persistence**: How long should the fixer stay active waiting for comments? Use idle timeout?

3. **Comment batching**: Should the fixer wait for multiple comments before responding, or address each immediately?

4. **Sandbox cleanup**: When does the shared sandbox get cleaned up? After phase completion or PR merge?

## Testing

1. E2E test verifying Claude and Gemini see the same files
2. Unit test for metadata extraction
3. Test fixer receives and addresses review comments
4. Test watcher → fixer comment delivery
