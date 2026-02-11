# E2E Sandbox Preservation Fix Plan

**Problem:** E2E tests fail validation because the watcher cleans up the sandbox worktree before changes can be validated. LLM work is lost.

**Root Cause:** `WatcherAgent::run()` calls `sandbox.cleanup()` after the LLM completes, destroying all changes made by the LLM.

## Fix Options

### Option A: Delay Cleanup (Recommended for E2E)
Modify watcher to return the sandbox without cleaning up on success. Let caller decide when to cleanup.

**Changes:**
1. Add `sandbox_path: Option<PathBuf>` to `WatcherResult`
2. Don't cleanup on success - return sandbox path instead
3. Caller (spawner) decides whether to cleanup or validate first

### Option B: Commit Before Cleanup
LLM should commit changes, which persist even after worktree removal.

**Changes:**
1. Add "commit all changes" step before cleanup
2. Changes survive on the branch even after worktree removal
3. Validation can checkout the branch or check commits

### Option C: Merge to Main Before Cleanup
Merge worktree branch to main before cleanup.

**Changes:**
1. After LLM success, merge worktree branch to main
2. Then cleanup worktree
3. Validation runs against main (which now has changes)

## Implementation: Option A

### Task 1: Return Sandbox from WatcherResult

**File:** `core/src/watcher.rs`

Add to WatcherResult:
```rust
pub struct WatcherResult {
    // ... existing fields ...
    /// Path to sandbox if not cleaned up (success case).
    pub sandbox_path: Option<PathBuf>,
}
```

### Task 2: Don't Cleanup on Success

**File:** `core/src/watcher.rs`

In `run()` method, don't cleanup on success:
```rust
Ok((progress, None)) => {
    // Success - return sandbox path for caller to validate
    return Ok(WatcherResult {
        success: true,
        progress,
        permission_errors,
        applied_fixes,
        termination_reason: Some(TerminationReason::Success),
        sandbox_path: Some(sandbox.path().clone()),
    });
}
```

### Task 3: Update Spawner to Handle Sandbox Lifecycle

**File:** `core/src/spawn.rs`

After watcher completes, spawner can:
1. Validate against sandbox path if needed
2. Cleanup sandbox after validation

### Task 4: Update E2E Harness

**File:** `core/src/e2e/harness.rs`

Validate against `spawn_result.sandbox_path` instead of `repo.path()`.

## Verification

Run E2E smoke test:
```bash
cargo test --test e2e_test smoke_hello -- --ignored --nocapture
```

Expected: Test passes with hello.txt found in sandbox.
