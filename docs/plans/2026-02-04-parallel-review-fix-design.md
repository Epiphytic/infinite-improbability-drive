# Parallel Review/Fix Pipeline Design

**Goal:** Replace the sequential review-then-fix loop in `SpawnTeamOrchestrator` with a parallel pipeline where reviewers run concurrently and a single fixer processes comments as a queue, with proper threaded replies and conversation resolution.

## Current Problem

In `team_orchestrator.rs`, the GitHub mode review flow is strictly sequential:

```
for each domain (Security, TechFeas, TaskGran, DepComplete, GenPolish):
    1. Run reviewer (blocks until complete)
    2. Get pending comments
    3. For each comment, run fixer (blocks until complete)
    4. Next domain
```

A comment posted at minute 1 won't get fixed until the entire domain review finishes. The next domain's reviewer can't start until all fixes for the previous domain are done.

## Proposed Architecture

```
┌─────────────────────────────────────────────────────────────┐
│               SpawnTeamOrchestrator (GitHub mode)            │
├─────────────────────────────────────────────────────────────┤
│                                                              │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐                    │
│  │Reviewer 1│ │Reviewer 2│ │Reviewer 3│  (tokio tasks,     │
│  │(Security)│ │(TechFeas)│ │(TaskGran)│   max_concurrent)  │
│  └────┬─────┘ └────┬─────┘ └────┬─────┘                    │
│       │             │            │                           │
│       └─────────────┼────────────┘                          │
│                     │ mpsc channel                           │
│                     ▼                                        │
│           ┌─────────────────┐                               │
│           │  Comment Queue   │                               │
│           │  (FixerWorker)   │ ← single fixer LLM           │
│           └────────┬────────┘                               │
│                    │                                         │
│                    ▼                                         │
│           ┌─────────────────┐                               │
│           │ For each comment:│                               │
│           │ 1. Run Claude fix│                               │
│           │ 2. Commit + push │                               │
│           │ 3. Reply (thread)│                               │
│           │ 4. Resolve thread│                               │
│           └─────────────────┘                               │
└─────────────────────────────────────────────────────────────┘
```

### Key Properties

1. **Parallel reviewers** - All 5 domain reviewers run as concurrent tokio tasks, with a configurable semaphore (`max_concurrent_reviewers`, default 3)
2. **Comment queue** - `tokio::sync::mpsc` channel connects reviewers to fixer. Reviewers poll for new comments after their review finishes and send them through the channel
3. **Single fixer worker** - One dedicated tokio task consumes the queue, running Claude fixes one at a time
4. **Threaded replies** - Fixer uses `gh api repos/{owner}/{repo}/pulls/{pr}/comments/{id}/replies` to post replies as thread children
5. **Conversation resolution** - Fixer uses GitHub GraphQL API to resolve the review thread after a successful fix

## Component Design

### 1. Configuration

In `SpawnTeamConfig`:
```rust
pub struct SpawnTeamConfig {
    // ... existing fields ...
    /// Maximum number of reviewer LLMs running concurrently (default: 3)
    pub max_concurrent_reviewers: u32,
}
```

In `CruiseConfig` TOML:
```toml
[cruise-control.planning]
max_concurrent_reviewers = 3
```

### 2. Comment Queue Types

```rust
/// A review comment queued for fixing.
struct QueuedComment {
    domain: ReviewDomain,
    comment: GitHubReviewComment,
    pr_number: u64,
    repo: String,
}

/// Messages sent from reviewers to the fixer worker.
enum FixerMessage {
    /// A comment that needs fixing.
    Fix(QueuedComment),
    /// Signal that all reviewers have completed.
    AllReviewersComplete,
}
```

### 3. Parallel Review Method

New method `run_parallel_review_fix()` replaces the sequential `for` loop in `run_with_branch()`:

```rust
async fn run_parallel_review_fix(
    &self,  // Note: self is shared read-only, observability collected per-task
    prompt: &str,
    timeout: Duration,
    worktree_path: &Path,
    work_path: &Path,
    pr_number: u64,
    repo: &str,
) -> Result<(Vec<ReviewResult>, SpawnObservability)> {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<FixerMessage>(100);
    let semaphore = Arc::new(Semaphore::new(
        self.config.max_concurrent_reviewers as usize
    ));

    // Shared state for collecting results
    let results = Arc::new(Mutex::new(Vec::new()));
    let observability = Arc::new(Mutex::new(SpawnObservability::default()));

    // Spawn all reviewer tasks concurrently
    let mut reviewer_handles = Vec::new();
    for domain in ReviewDomain::all() {
        let permit = semaphore.clone();
        let tx = tx.clone();
        let config = self.config.clone();
        // ... clone what each reviewer needs ...

        let handle = tokio::spawn(async move {
            let _permit = permit.acquire().await.unwrap();

            // 1. Get diff
            // 2. Run reviewer for this domain
            // 3. Poll for new comments on PR
            // 4. Send each comment through tx
            // 5. Return ReviewResult
        });

        reviewer_handles.push(handle);
    }
    drop(tx); // Drop sender so fixer knows when all reviewers are done

    // Spawn single fixer worker
    let fixer_handle = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            match msg {
                FixerMessage::Fix(queued) => {
                    // 1. Run Claude to fix the comment
                    // 2. Commit + push
                    // 3. Post threaded reply to comment
                    // 4. Resolve the conversation thread
                }
                FixerMessage::AllReviewersComplete => break,
            }
        }
    });

    // Wait for all reviewers, then fixer
    for handle in reviewer_handles {
        if let Ok(result) = handle.await {
            // Collect review results
        }
    }
    // Channel closes when all tx clones are dropped
    fixer_handle.await?;
}
```

### 4. Threaded Replies

Current code posts a top-level PR comment (wrong):
```rust
let _ = Command::new("gh")
    .args(["pr", "comment", &pr_number.to_string(), "--body", &reply_body])
    .output();
```

New approach posts a threaded reply to the specific review comment:
```rust
fn reply_to_review_comment(
    repo: &str,
    pr_number: u64,
    comment_id: u64,
    body: &str,
    work_dir: &Path,
) -> Result<()> {
    let reply_json = serde_json::json!({ "body": body });

    let output = Command::new("gh")
        .current_dir(work_dir)
        .args([
            "api",
            &format!("repos/{}/pulls/{}/comments/{}/replies",
                repo, pr_number, comment_id),
            "--method", "POST",
            "--input", "-",
        ])
        .stdin(/* pipe reply_json */)
        .output()?;

    Ok(())
}
```

### 5. Conversation Resolution

After posting the threaded reply, resolve the review thread:

```rust
fn resolve_review_thread(
    repo: &str,
    pr_number: u64,
    comment_id: u64,
    work_dir: &Path,
) -> Result<()> {
    // Step 1: Find the thread node ID for this comment
    let thread_query = format!(
        r#"query {{
            repository(owner: "{owner}", name: "{name}") {{
                pullRequest(number: {pr}) {{
                    reviewThreads(first: 100) {{
                        nodes {{
                            id
                            isResolved
                            comments(first: 1) {{
                                nodes {{ databaseId }}
                            }}
                        }}
                    }}
                }}
            }}
        }}"#,
        owner = repo.split('/').next().unwrap(),
        name = repo.split('/').nth(1).unwrap(),
        pr = pr_number,
    );

    // Step 2: Find thread matching our comment_id
    // Step 3: Resolve it
    let resolve_mutation = format!(
        r#"mutation {{ resolveReviewThread(input: {{threadId: "{}"}}) {{
            thread {{ isResolved }}
        }} }}"#,
        thread_id,
    );

    Command::new("gh")
        .current_dir(work_dir)
        .args(["api", "graphql", "-f", &format!("query={}", resolve_mutation)])
        .output()?;

    Ok(())
}
```

### 6. Concurrency Safety

- **Fixer is single-threaded**: No concurrent git commits
- **Reviewers are read-only**: They only post comments via GitHub API, no worktree modifications
- **Git push retries**: If push fails (unlikely since only fixer commits), retry with `git pull --rebase`
- **Observability collection**: Each reviewer/fixer task collects its own records, merged at the end

## Files to Modify

| File | Change |
|------|--------|
| `core/src/team.rs` | Add `max_concurrent_reviewers` to `SpawnTeamConfig` |
| `core/src/team_orchestrator.rs` | Add `run_parallel_review_fix()`, `reply_to_review_comment()`, `resolve_review_thread()`. Modify `run_with_branch()` to call parallel method for GitHub mode. Update `resolve_github_comment()` |
| `core/src/cruise/config.rs` | Add `max_concurrent_reviewers` to planning config |
| `core/src/cruise/runner.rs` | Pass new config to orchestrator |

## Risks

1. **GitHub API rate limiting** - Multiple concurrent reviewers + fixer all calling GitHub API. Mitigated by semaphore limiting max concurrent reviewers.
2. **Thread resolution API** - Requires GraphQL to look up thread node ID from comment database ID. Extra API call per fix.
3. **Git push timing** - Fixer pushes while reviewers may still be running. No conflict since reviewers don't touch the worktree.
4. **Gemini quota exhaustion** - Multiple concurrent Gemini calls may hit rate limits faster. Mitigated by configurable `max_concurrent_reviewers`.
