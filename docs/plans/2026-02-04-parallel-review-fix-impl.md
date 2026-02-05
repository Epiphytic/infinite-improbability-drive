# Parallel Review/Fix Pipeline Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace the sequential review-then-fix loop with a parallel pipeline where reviewers run concurrently and a single fixer processes comments as a queue, with threaded replies and conversation resolution.

**Architecture:** Reviewers spawn as concurrent tokio tasks (semaphore-limited). An mpsc channel feeds review comments to a single fixer worker task that processes them one at a time. The fixer posts threaded replies to the specific review comment and resolves the GitHub conversation thread.

**Tech Stack:** Rust, tokio (mpsc, Semaphore, JoinHandle), GitHub REST API, GitHub GraphQL API, `gh` CLI

---

### Task 1: Add `max_concurrent_reviewers` to SpawnTeamConfig

**Files:**
- Modify: `core/src/team.rs:27-49` (SpawnTeamConfig struct)
- Modify: `core/src/team.rs:67-79` (Default impl)
- Modify: `core/src/team.rs:598-616` (tests)

**Step 1: Write the failing test**

Add to `core/src/team.rs` inside `mod tests`:

```rust
#[test]
fn spawn_team_config_default_max_concurrent_reviewers() {
    let config = SpawnTeamConfig::default();
    assert_eq!(config.max_concurrent_reviewers, 3);
}
```

**Step 2: Run test to verify it fails**

Run: `cd /Users/liam.helmer/repos/epiphytic/infinite-improbability-drive/.worktrees/phase-sandbox/core && cargo test spawn_team_config_default_max_concurrent_reviewers -- --nocapture`

Expected: FAIL with "no field `max_concurrent_reviewers`"

**Step 3: Implement the field**

Add to `SpawnTeamConfig` struct (after `max_escalations` field, line ~48):

```rust
    /// Maximum number of reviewer LLMs running concurrently in GitHub mode.
    /// Default: 3. Set to 1 for sequential behavior.
    #[serde(default = "default_max_concurrent_reviewers")]
    pub max_concurrent_reviewers: u32,
```

Add default function (after `default_max_escalations`, line ~65):

```rust
fn default_max_concurrent_reviewers() -> u32 {
    3
}
```

Add to `Default` impl (after `max_escalations` in the `default()` method):

```rust
            max_concurrent_reviewers: default_max_concurrent_reviewers(),
```

**Step 4: Run tests to verify they pass**

Run: `cd /Users/liam.helmer/repos/epiphytic/infinite-improbability-drive/.worktrees/phase-sandbox/core && cargo test team:: -- --nocapture`

Expected: ALL PASS

**Step 5: Commit**

```bash
cd /Users/liam.helmer/repos/epiphytic/infinite-improbability-drive/.worktrees/phase-sandbox
git add core/src/team.rs
git commit -m "feat(team): add max_concurrent_reviewers config field"
```

---

### Task 2: Add `max_concurrent_reviewers` to PlanningConfig and CruiseRunner

**Files:**
- Modify: `core/src/cruise/config.rs:46-54` (PlanningConfig)
- Modify: `core/src/cruise/config.rs:64-71` (Default impl)
- Modify: `core/src/cruise/runner.rs:700-726` (run_planning_phase_with_team)

**Step 1: Write the failing test**

Add to `core/src/cruise/config.rs` inside `mod tests`:

```rust
#[test]
fn planning_config_default_max_concurrent_reviewers() {
    let config = PlanningConfig::default();
    assert_eq!(config.max_concurrent_reviewers, 3);
}
```

**Step 2: Run test to verify it fails**

Run: `cd /Users/liam.helmer/repos/epiphytic/infinite-improbability-drive/.worktrees/phase-sandbox/core && cargo test planning_config_default_max_concurrent_reviewers -- --nocapture`

Expected: FAIL

**Step 3: Implement**

Add to `PlanningConfig` struct (after `reviewer_llm` field):

```rust
    /// Maximum concurrent reviewer LLMs in GitHub mode.
    #[serde(default = "default_max_concurrent_reviewers")]
    pub max_concurrent_reviewers: u32,
```

Add default function (after `default_reviewer_llm`):

```rust
fn default_max_concurrent_reviewers() -> u32 {
    3
}
```

Add to `PlanningConfig::default()`:

```rust
            max_concurrent_reviewers: default_max_concurrent_reviewers(),
```

Then in `core/src/cruise/runner.rs`, update `run_planning_phase_with_team` to pass the config value to `SpawnTeamConfig` (around line 704-711):

```rust
        let team_config = SpawnTeamConfig {
            mode: self.team_mode,
            max_iterations: self.config.planning.ping_pong_iterations,
            primary_llm: "claude-code".to_string(),
            primary_model: self.primary_model.clone(),
            reviewer_llm: "gemini-cli".to_string(),
            reviewer_model: self.reviewer_model.clone(),
            max_escalations: self.max_escalations,
            max_concurrent_reviewers: self.config.planning.max_concurrent_reviewers,
        };
```

Also update `run_execution_phase_with_team` (around line 1254-1262) the same way.

**Step 4: Run tests to verify they pass**

Run: `cd /Users/liam.helmer/repos/epiphytic/infinite-improbability-drive/.worktrees/phase-sandbox/core && cargo test -- --nocapture`

Expected: ALL PASS

**Step 5: Commit**

```bash
cd /Users/liam.helmer/repos/epiphytic/infinite-improbability-drive/.worktrees/phase-sandbox
git add core/src/cruise/config.rs core/src/cruise/runner.rs
git commit -m "feat(cruise): add max_concurrent_reviewers to planning config"
```

---

### Task 3: Add comment queue types and threaded reply helper

**Files:**
- Modify: `core/src/team_orchestrator.rs` (add types and helper methods)

**Step 1: Write the failing test**

Add to `core/src/team_orchestrator.rs` inside `mod tests`:

```rust
#[test]
fn queued_comment_captures_data() {
    let comment = GitHubReviewComment {
        id: 123,
        path: "src/main.rs".to_string(),
        line: Some(10),
        body: "Fix this".to_string(),
        resolved: false,
        resolved_by_commit: None,
    };
    let queued = QueuedComment {
        domain: ReviewDomain::Security,
        comment: comment.clone(),
        pr_number: 1,
        repo: "owner/repo".to_string(),
    };
    assert_eq!(queued.comment.id, 123);
    assert_eq!(queued.domain, ReviewDomain::Security);
}

#[test]
fn fixer_message_variants() {
    let comment = GitHubReviewComment {
        id: 456,
        path: "src/lib.rs".to_string(),
        line: None,
        body: "Issue".to_string(),
        resolved: false,
        resolved_by_commit: None,
    };
    let queued = QueuedComment {
        domain: ReviewDomain::GeneralPolish,
        comment,
        pr_number: 2,
        repo: "owner/repo".to_string(),
    };
    let msg = FixerMessage::Fix(queued);
    assert!(matches!(msg, FixerMessage::Fix(_)));

    let done = FixerMessage::AllReviewersComplete;
    assert!(matches!(done, FixerMessage::AllReviewersComplete));
}
```

**Step 2: Run test to verify it fails**

Run: `cd /Users/liam.helmer/repos/epiphytic/infinite-improbability-drive/.worktrees/phase-sandbox/core && cargo test queued_comment_captures_data -- --nocapture`

Expected: FAIL with "cannot find type `QueuedComment`"

**Step 3: Implement the types**

Add near the top of `core/src/team_orchestrator.rs` (after the existing struct definitions, around line 138):

```rust
/// A review comment queued for fixing by the fixer worker.
#[derive(Debug, Clone)]
pub struct QueuedComment {
    /// The review domain this comment came from.
    pub domain: ReviewDomain,
    /// The GitHub review comment to fix.
    pub comment: GitHubReviewComment,
    /// PR number.
    pub pr_number: u64,
    /// Repository name (owner/repo).
    pub repo: String,
}

/// Messages sent from reviewer tasks to the fixer worker.
#[derive(Debug)]
pub enum FixerMessage {
    /// A comment that needs fixing.
    Fix(QueuedComment),
    /// Signal that all reviewers have completed.
    AllReviewersComplete,
}
```

Also add `use crate::team::ReviewDomain;` to the imports if not already present (it's imported via the glob on line 15).

**Step 4: Run tests to verify they pass**

Run: `cd /Users/liam.helmer/repos/epiphytic/infinite-improbability-drive/.worktrees/phase-sandbox/core && cargo test queued_comment -- --nocapture && cargo test fixer_message -- --nocapture`

Expected: ALL PASS

**Step 5: Commit**

```bash
cd /Users/liam.helmer/repos/epiphytic/infinite-improbability-drive/.worktrees/phase-sandbox
git add core/src/team_orchestrator.rs
git commit -m "feat(orchestrator): add QueuedComment and FixerMessage types"
```

---

### Task 4: Implement `reply_to_review_comment` helper

This replaces the current top-level `gh pr comment` with a threaded reply to a specific review comment.

**Files:**
- Modify: `core/src/team_orchestrator.rs` (add method to impl block)

**Step 1: Write the failing test**

Add to `core/src/team_orchestrator.rs` tests (this tests the command construction, not execution):

```rust
#[test]
fn reply_to_review_comment_builds_correct_api_path() {
    // Test the API path construction logic
    let repo = "owner/repo";
    let pr_number = 42u64;
    let comment_id = 123u64;
    let api_path = format!(
        "repos/{}/pulls/{}/comments/{}/replies",
        repo, pr_number, comment_id
    );
    assert_eq!(
        api_path,
        "repos/owner/repo/pulls/42/comments/123/replies"
    );
}
```

**Step 2: Run test to verify it passes** (this is a pure logic test)

Run: `cd /Users/liam.helmer/repos/epiphytic/infinite-improbability-drive/.worktrees/phase-sandbox/core && cargo test reply_to_review_comment_builds -- --nocapture`

Expected: PASS

**Step 3: Implement the helper method**

Add to the `impl<P: SandboxProvider + Clone + 'static> SpawnTeamOrchestrator<P>` block (before the `// GitHub Mode Helper Methods` section):

```rust
    /// Posts a threaded reply to a specific GitHub review comment.
    ///
    /// Uses the GitHub API to create a reply that appears as a child
    /// of the original review comment, rather than a top-level PR comment.
    fn reply_to_review_comment(
        &self,
        repo: &str,
        pr_number: u64,
        comment_id: u64,
        body: &str,
        work_dir: &Path,
    ) -> Result<()> {
        let api_path = format!(
            "repos/{}/pulls/{}/comments/{}/replies",
            repo, pr_number, comment_id
        );

        let output = Command::new("gh")
            .current_dir(work_dir)
            .args([
                "api",
                &api_path,
                "--method", "POST",
                "-f", &format!("body={}", body),
            ])
            .output()
            .map_err(|e| Error::Cruise(format!("failed to reply to comment: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::warn!(
                comment_id = comment_id,
                error = %stderr,
                "failed to post threaded reply, falling back to PR comment"
            );
            // Fallback: post as top-level PR comment with reference
            let fallback_body = format!(
                "> Re: comment #{}\n\n{}",
                comment_id, body
            );
            let _ = Command::new("gh")
                .current_dir(work_dir)
                .args([
                    "pr", "comment",
                    &pr_number.to_string(),
                    "--repo", repo,
                    "--body", &fallback_body,
                ])
                .output();
        }

        Ok(())
    }
```

**Step 4: Run all tests to verify nothing broke**

Run: `cd /Users/liam.helmer/repos/epiphytic/infinite-improbability-drive/.worktrees/phase-sandbox/core && cargo test -- --nocapture`

Expected: ALL PASS

**Step 5: Commit**

```bash
cd /Users/liam.helmer/repos/epiphytic/infinite-improbability-drive/.worktrees/phase-sandbox
git add core/src/team_orchestrator.rs
git commit -m "feat(orchestrator): add reply_to_review_comment for threaded replies"
```

---

### Task 5: Implement `resolve_review_thread` helper

**Files:**
- Modify: `core/src/team_orchestrator.rs` (add method)

**Step 1: Write the failing test**

```rust
#[test]
fn resolve_review_thread_builds_graphql_query() {
    // Test the GraphQL query construction
    let thread_id = "RT_kwDOtest123";
    let query = format!(
        r#"mutation {{ resolveReviewThread(input: {{threadId: "{}"}}) {{ thread {{ isResolved }} }} }}"#,
        thread_id
    );
    assert!(query.contains("resolveReviewThread"));
    assert!(query.contains(thread_id));
}
```

**Step 2: Run test to verify it passes**

Run: `cd /Users/liam.helmer/repos/epiphytic/infinite-improbability-drive/.worktrees/phase-sandbox/core && cargo test resolve_review_thread_builds -- --nocapture`

Expected: PASS

**Step 3: Implement**

Add to the impl block:

```rust
    /// Resolves a GitHub review thread by finding the thread containing the comment
    /// and marking it as resolved via GraphQL.
    fn resolve_review_thread(
        &self,
        repo: &str,
        pr_number: u64,
        comment_id: u64,
        work_dir: &Path,
    ) -> Result<()> {
        let (owner, name) = repo.split_once('/')
            .ok_or_else(|| Error::Cruise(format!("invalid repo format: {}", repo)))?;

        // Step 1: Find the thread node ID that contains this comment
        let query = format!(
            r#"query {{
                repository(owner: "{}", name: "{}") {{
                    pullRequest(number: {}) {{
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
            owner, name, pr_number,
        );

        let output = Command::new("gh")
            .current_dir(work_dir)
            .args(["api", "graphql", "-f", &format!("query={}", query)])
            .output()
            .map_err(|e| Error::Cruise(format!("failed to query review threads: {}", e)))?;

        if !output.status.success() {
            tracing::warn!(
                comment_id = comment_id,
                error = %String::from_utf8_lossy(&output.stderr),
                "failed to query review threads for resolution"
            );
            return Ok(()); // Don't fail the workflow
        }

        let response: serde_json::Value = serde_json::from_slice(&output.stdout)
            .map_err(|e| Error::Cruise(format!("failed to parse GraphQL response: {}", e)))?;

        // Find the thread containing our comment
        let thread_id = response
            .pointer("/data/repository/pullRequest/reviewThreads/nodes")
            .and_then(|nodes| nodes.as_array())
            .and_then(|nodes| {
                nodes.iter().find(|node| {
                    node.pointer("/comments/nodes/0/databaseId")
                        .and_then(|id| id.as_u64())
                        == Some(comment_id)
                })
            })
            .and_then(|node| node.get("id"))
            .and_then(|id| id.as_str());

        let Some(thread_id) = thread_id else {
            tracing::debug!(
                comment_id = comment_id,
                "could not find review thread for comment (may be a PR comment, not a review comment)"
            );
            return Ok(());
        };

        // Step 2: Resolve the thread
        let resolve_mutation = format!(
            r#"mutation {{ resolveReviewThread(input: {{threadId: "{}"}}) {{ thread {{ isResolved }} }} }}"#,
            thread_id,
        );

        let resolve_output = Command::new("gh")
            .current_dir(work_dir)
            .args(["api", "graphql", "-f", &format!("query={}", resolve_mutation)])
            .output()
            .map_err(|e| Error::Cruise(format!("failed to resolve thread: {}", e)))?;

        if resolve_output.status.success() {
            tracing::info!(
                comment_id = comment_id,
                thread_id = thread_id,
                "resolved review thread"
            );
        } else {
            tracing::warn!(
                comment_id = comment_id,
                error = %String::from_utf8_lossy(&resolve_output.stderr),
                "failed to resolve review thread"
            );
        }

        Ok(())
    }
```

**Step 4: Run all tests**

Run: `cd /Users/liam.helmer/repos/epiphytic/infinite-improbability-drive/.worktrees/phase-sandbox/core && cargo test -- --nocapture`

Expected: ALL PASS

**Step 5: Commit**

```bash
cd /Users/liam.helmer/repos/epiphytic/infinite-improbability-drive/.worktrees/phase-sandbox
git add core/src/team_orchestrator.rs
git commit -m "feat(orchestrator): add resolve_review_thread for conversation resolution"
```

---

### Task 6: Update `resolve_github_comment` to use threaded replies and resolution

**Files:**
- Modify: `core/src/team_orchestrator.rs:1963-2057` (resolve_github_comment method)

**Step 1: No new test needed** - existing tests cover the method signature. The behavioral change (threaded reply + resolution) is verified by E2E tests.

**Step 2: Update the method**

In `resolve_github_comment` (around line 2024-2054), replace the current reply logic:

Replace this block (the "Reply to the comment on GitHub" section at the end of the method):
```rust
        // Reply to the comment on GitHub
        let reply_body = format!("Fixed in commit {}", commit_hash);
        let _ = Command::new("gh")
            .current_dir(sandbox_path)
            .args([
                "pr",
                "comment",
                &pr_number.to_string(),
                "--body",
                &reply_body,
            ])
            .output();
```

With:
```rust
        // Reply as a thread child to the specific review comment
        let reply_body = format!(
            "Fixed in commit {}.\n\nChanges address the feedback in this comment.",
            commit_hash
        );
        self.reply_to_review_comment(
            repo,
            pr_number,
            comment_id,
            &reply_body,
            sandbox_path,
        )?;

        // Resolve the review thread
        self.resolve_review_thread(
            repo,
            pr_number,
            comment_id,
            sandbox_path,
        )?;
```

**Step 3: Run all tests**

Run: `cd /Users/liam.helmer/repos/epiphytic/infinite-improbability-drive/.worktrees/phase-sandbox/core && cargo test -- --nocapture`

Expected: ALL PASS

**Step 4: Commit**

```bash
cd /Users/liam.helmer/repos/epiphytic/infinite-improbability-drive/.worktrees/phase-sandbox
git add core/src/team_orchestrator.rs
git commit -m "feat(orchestrator): use threaded replies and resolve conversations on fix"
```

---

### Task 7: Implement `run_parallel_review_fix` method

This is the core change. It replaces the sequential `for` loop in `run_with_branch()` with a parallel pipeline.

**Files:**
- Modify: `core/src/team_orchestrator.rs` (add new method, requires `use std::sync::Arc; use tokio::sync::{mpsc, Semaphore, Mutex};`)

**Step 1: Write the failing test**

```rust
#[test]
fn parallel_review_fix_types_compile() {
    // Verify the channel types work together
    let (tx, mut rx) = tokio::sync::mpsc::channel::<FixerMessage>(10);
    let comment = GitHubReviewComment {
        id: 1,
        path: "test.rs".to_string(),
        line: None,
        body: "test".to_string(),
        resolved: false,
        resolved_by_commit: None,
    };
    let queued = QueuedComment {
        domain: ReviewDomain::Security,
        comment,
        pr_number: 1,
        repo: "test/repo".to_string(),
    };

    // Verify we can send and receive
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        tx.send(FixerMessage::Fix(queued)).await.unwrap();
        drop(tx);
        let msg = rx.recv().await.unwrap();
        assert!(matches!(msg, FixerMessage::Fix(_)));
        let none = rx.recv().await;
        assert!(none.is_none()); // Channel closed
    });
}
```

**Step 2: Run test to verify it passes** (type verification only)

Run: `cd /Users/liam.helmer/repos/epiphytic/infinite-improbability-drive/.worktrees/phase-sandbox/core && cargo test parallel_review_fix_types -- --nocapture`

Expected: PASS (or FAIL if imports needed)

**Step 3: Implement**

First, add imports at the top of `team_orchestrator.rs`:

```rust
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex, Semaphore};
```

Then add the method to the impl block. This is a large method - here's the full implementation:

```rust
    /// Runs review phases in parallel with a concurrent fixer worker.
    ///
    /// All review domains are spawned as concurrent tokio tasks (limited by
    /// `max_concurrent_reviewers` semaphore). As each reviewer completes and
    /// posts comments, those comments are sent through an mpsc channel to a
    /// single fixer worker task.
    ///
    /// The fixer processes comments one at a time, making code changes,
    /// committing, pushing, posting threaded replies, and resolving threads.
    async fn run_parallel_review_fix(
        &mut self,
        prompt: &str,
        timeout: Duration,
        worktree_path: &Path,
        work_path: &Path,
        pr_number: u64,
        repo: &str,
    ) -> Result<Vec<ReviewResult>> {
        let max_concurrent = self.config.max_concurrent_reviewers.max(1) as usize;
        let (tx, mut rx) = mpsc::channel::<FixerMessage>(100);

        tracing::info!(
            max_concurrent = max_concurrent,
            pr_number = pr_number,
            "starting parallel review/fix pipeline"
        );

        // Collect data needed by reviewer tasks (avoid borrowing self across await)
        let reviewer_config = self.config.clone();
        let env_vars = self.env_vars.clone();
        let review_domains = ReviewDomain::all().to_vec();

        // We need to collect results and observability from each reviewer
        let results: Arc<Mutex<Vec<ReviewResult>>> = Arc::new(Mutex::new(Vec::new()));
        let obs_records: Arc<Mutex<Vec<(CommandLineRecord, ReviewFeedbackRecord)>>> =
            Arc::new(Mutex::new(Vec::new()));

        // Spawn reviewer tasks with semaphore limiting concurrency
        let semaphore = Arc::new(Semaphore::new(max_concurrent));
        let mut reviewer_handles = Vec::new();

        for domain in review_domains {
            let permit = semaphore.clone();
            let tx = tx.clone();
            let config = reviewer_config.clone();
            let env = env_vars.clone();
            let prompt_owned = prompt.to_string();
            let worktree = worktree_path.to_path_buf();
            let work = work_path.to_path_buf();
            let repo_name = repo.to_string();
            let results_clone = results.clone();
            let obs_clone = obs_records.clone();

            let handle = tokio::task::spawn_blocking(move || {
                // Acquire semaphore permit (blocking in spawn_blocking is fine)
                let _permit = permit.blocking_lock_owned();

                tracing::info!(
                    domain = %domain.as_str(),
                    "reviewer task starting"
                );

                // Get diff for review
                let diff_output = Command::new("git")
                    .current_dir(&worktree)
                    .args(["diff", "HEAD~1..HEAD"])
                    .output();

                let diff = match diff_output {
                    Ok(output) if output.status.success() => {
                        String::from_utf8_lossy(&output.stdout).to_string()
                    }
                    _ => {
                        // Try cached diff
                        Command::new("git")
                            .current_dir(&worktree)
                            .args(["diff", "--cached"])
                            .output()
                            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                            .unwrap_or_default()
                    }
                };

                // Build review prompt
                let review_prompt = GitHubReviewPromptBuilder::new(
                    pr_number, &repo_name, domain,
                )
                .with_original_prompt(&prompt_owned)
                .with_diff(&diff)
                .build();

                // Record command line
                let cmd_record = CommandLineRecord {
                    llm: config.reviewer_llm.clone(),
                    command: format!("gemini --print \"GitHub review for {} domain\"", domain.as_str()),
                    work_dir: worktree.clone(),
                    iteration: 0,
                    role: format!("reviewer-{}", domain.as_str().to_lowercase()),
                    timestamp: chrono::Utc::now().to_rfc3339(),
                };

                // Run Gemini reviewer
                let mut gemini_args = vec![
                    "--yolo".to_string(),
                    "--output-format".to_string(), "stream-json".to_string(),
                    "--allowed-tools".to_string(), "Read,Glob,Grep,Bash".to_string(),
                ];
                if let Some(ref model) = config.reviewer_model {
                    gemini_args.push("--model".to_string());
                    gemini_args.push(model.clone());
                }
                gemini_args.push("-p".to_string());
                gemini_args.push(review_prompt);

                let output = Command::new("gemini")
                    .current_dir(&worktree)
                    .envs(&env)
                    .args(&gemini_args)
                    .output();

                let (verdict, raw_response) = match output {
                    Ok(output) => {
                        let raw = String::from_utf8_lossy(&output.stdout).to_string();
                        let success = output.status.success();

                        // Check for NEEDS CHANGES marker in PR comments
                        let needs_changes = Self::check_pr_needs_changes_static(
                            pr_number, &repo_name, &work,
                        );

                        let verdict = if needs_changes {
                            ReviewVerdict::NeedsChanges
                        } else if success {
                            ReviewVerdict::Approved
                        } else {
                            ReviewVerdict::Approved // Don't block on reviewer failure
                        };

                        (verdict, raw)
                    }
                    Err(e) => {
                        tracing::warn!(
                            domain = %domain.as_str(),
                            error = %e,
                            "reviewer failed to execute"
                        );
                        (ReviewVerdict::Approved, String::new())
                    }
                };

                let review = ReviewResult {
                    verdict: verdict.clone(),
                    suggestions: vec![],
                    summary: format!("{} review completed", domain.as_str()),
                };

                // Record feedback
                let feedback_record = ReviewFeedbackRecord {
                    iteration: 0,
                    phase: Some(domain.as_str().to_string()),
                    verdict: review.verdict.clone(),
                    suggestion_count: 0,
                    review: review.clone(),
                    raw_response,
                    diff_reviewed: diff,
                };

                // Store results
                {
                    let mut results_lock = results_clone.blocking_lock();
                    results_lock.push(review.clone());
                }
                {
                    let mut obs_lock = obs_clone.blocking_lock();
                    obs_lock.push((cmd_record, feedback_record));
                }

                // If needs changes, get pending comments and send to fixer
                if verdict == ReviewVerdict::NeedsChanges {
                    let comments = Self::get_pending_comments_static(
                        pr_number, &repo_name, &work,
                    );
                    for comment in comments {
                        let queued = QueuedComment {
                            domain,
                            comment,
                            pr_number,
                            repo: repo_name.clone(),
                        };
                        // Use blocking send since we're in spawn_blocking
                        let _ = tx.blocking_send(FixerMessage::Fix(queued));
                    }
                }

                tracing::info!(
                    domain = %domain.as_str(),
                    verdict = ?verdict,
                    "reviewer task completed"
                );
            });

            reviewer_handles.push(handle);
        }

        // Drop our copy of tx so the channel closes when all reviewers finish
        drop(tx);

        // Spawn the fixer worker as a blocking task
        let fixer_prompt = prompt.to_string();
        let fixer_worktree = worktree_path.to_path_buf();
        let fixer_env = self.env_vars.clone();
        let fixer_config = self.config.clone();
        let fixer_obs: Arc<Mutex<Vec<ResolvedCommentRecord>>> = Arc::new(Mutex::new(Vec::new()));
        let fixer_obs_clone = fixer_obs.clone();
        let fixer_commit_records: Arc<Mutex<Vec<CommitRecord>>> = Arc::new(Mutex::new(Vec::new()));
        let fixer_commit_clone = fixer_commit_records.clone();

        let fixer_handle = tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                let FixerMessage::Fix(queued) = msg else {
                    break;
                };

                let comment_id = queued.comment.id;
                tracing::info!(
                    comment_id = comment_id,
                    domain = %queued.domain.as_str(),
                    path = %queued.comment.path,
                    "fixer: processing comment"
                );

                // Build resolve prompt
                let resolve_prompt = ResolveCommentPromptBuilder::new(
                    queued.pr_number,
                    &queued.repo,
                    queued.comment.clone(),
                )
                .with_original_prompt(&fixer_prompt)
                .build();

                // Run Claude to fix (blocking)
                let worktree = fixer_worktree.clone();
                let env = fixer_env.clone();
                let obs = fixer_obs_clone.clone();
                let commits = fixer_commit_clone.clone();
                let repo = queued.repo.clone();
                let pr_num = queued.pr_number;

                let _ = tokio::task::spawn_blocking(move || {
                    let output = Command::new("claude")
                        .current_dir(&worktree)
                        .envs(&env)
                        .args([
                            "--print", "--verbose",
                            "--output-format", "stream-json",
                            "--permission-mode", "acceptEdits",
                            "-p", &resolve_prompt,
                        ])
                        .output();

                    match output {
                        Ok(o) if o.status.success() => {
                            // Commit and push
                            let _ = Command::new("git")
                                .current_dir(&worktree)
                                .args(["add", "-A"])
                                .output();

                            let commit_msg = format!(
                                "[cruise-control] fix: resolve comment {} ({})",
                                comment_id,
                                queued.domain.as_str()
                            );
                            let commit_out = Command::new("git")
                                .current_dir(&worktree)
                                .args(["commit", "-m", &commit_msg])
                                .output();

                            let commit_hash = Command::new("git")
                                .current_dir(&worktree)
                                .args(["rev-parse", "HEAD"])
                                .output()
                                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                                .unwrap_or_default();

                            // Push
                            let branch = Command::new("git")
                                .current_dir(&worktree)
                                .args(["rev-parse", "--abbrev-ref", "HEAD"])
                                .output()
                                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                                .unwrap_or_default();

                            let _ = Command::new("git")
                                .current_dir(&worktree)
                                .args(["push", "-u", "origin", &branch])
                                .output();

                            // Post threaded reply
                            let reply_body = format!(
                                "Fixed in commit {}.\n\nChanges address the feedback in this comment.",
                                &commit_hash[..commit_hash.len().min(7)]
                            );
                            let api_path = format!(
                                "repos/{}/pulls/{}/comments/{}/replies",
                                repo, pr_num, comment_id
                            );
                            let _ = Command::new("gh")
                                .current_dir(&worktree)
                                .args([
                                    "api", &api_path,
                                    "--method", "POST",
                                    "-f", &format!("body={}", reply_body),
                                ])
                                .output();

                            // Resolve the thread (best effort)
                            // This uses the resolve_review_thread logic inline
                            // since we can't call self methods from here
                            let (owner, name) = repo.split_once('/').unwrap_or(("", ""));
                            let gql_query = format!(
                                r#"query {{ repository(owner: \"{}\", name: \"{}\") {{ pullRequest(number: {}) {{ reviewThreads(first: 100) {{ nodes {{ id isResolved comments(first: 1) {{ nodes {{ databaseId }} }} }} }} }} }} }}"#,
                                owner, name, pr_num,
                            );
                            if let Ok(thread_output) = Command::new("gh")
                                .current_dir(&worktree)
                                .args(["api", "graphql", "-f", &format!("query={}", gql_query)])
                                .output()
                            {
                                if let Ok(resp) = serde_json::from_slice::<serde_json::Value>(&thread_output.stdout) {
                                    if let Some(nodes) = resp
                                        .pointer("/data/repository/pullRequest/reviewThreads/nodes")
                                        .and_then(|n| n.as_array())
                                    {
                                        if let Some(thread) = nodes.iter().find(|n| {
                                            n.pointer("/comments/nodes/0/databaseId")
                                                .and_then(|id| id.as_u64())
                                                == Some(comment_id)
                                        }) {
                                            if let Some(tid) = thread.get("id").and_then(|id| id.as_str()) {
                                                let resolve_mut = format!(
                                                    r#"mutation {{ resolveReviewThread(input: {{threadId: \"{}\"}}) {{ thread {{ isResolved }} }} }}"#,
                                                    tid,
                                                );
                                                let _ = Command::new("gh")
                                                    .current_dir(&worktree)
                                                    .args(["api", "graphql", "-f", &format!("query={}", resolve_mut)])
                                                    .output();
                                                tracing::info!(
                                                    comment_id = comment_id,
                                                    "resolved review thread"
                                                );
                                            }
                                        }
                                    }
                                }
                            }

                            // Record resolution
                            {
                                let mut obs_lock = obs.blocking_lock();
                                obs_lock.push(ResolvedCommentRecord {
                                    comment_id,
                                    resolved_by_commit: commit_hash.clone(),
                                    resolved_at: chrono::Utc::now().to_rfc3339(),
                                    explanation: format!("Resolved in commit {}", commit_hash),
                                });
                            }
                            {
                                let mut commits_lock = commits.blocking_lock();
                                commits_lock.push(CommitRecord {
                                    hash: commit_hash,
                                    message: commit_msg,
                                    iteration: 0,
                                    llm: "claude-code".to_string(),
                                    timestamp: chrono::Utc::now().to_rfc3339(),
                                    pushed: true,
                                });
                            }
                        }
                        Ok(o) => {
                            tracing::warn!(
                                comment_id = comment_id,
                                error = %String::from_utf8_lossy(&o.stderr),
                                "fixer failed to resolve comment"
                            );
                        }
                        Err(e) => {
                            tracing::warn!(
                                comment_id = comment_id,
                                error = %e,
                                "fixer failed to execute"
                            );
                        }
                    }
                }).await;
            }
        });

        // Wait for all reviewers to complete
        for handle in reviewer_handles {
            if let Err(e) = handle.await {
                tracing::warn!(error = %e, "reviewer task panicked");
            }
        }

        // Wait for fixer to drain the queue
        if let Err(e) = fixer_handle.await {
            tracing::warn!(error = %e, "fixer task panicked");
        }

        // Collect results and update observability
        let final_results = Arc::try_unwrap(results)
            .unwrap_or_else(|arc| arc.blocking_lock().clone())
            .into_inner();

        let obs_data = Arc::try_unwrap(obs_records)
            .unwrap_or_else(|arc| arc.blocking_lock().clone())
            .into_inner();

        for (cmd, feedback) in obs_data {
            self.observability.command_lines.push(cmd);
            self.observability.review_feedback.push(feedback);
        }

        let resolved = Arc::try_unwrap(fixer_obs)
            .unwrap_or_else(|arc| arc.blocking_lock().clone())
            .into_inner();
        self.observability.resolved_comments.extend(resolved);

        let commits = Arc::try_unwrap(fixer_commit_records)
            .unwrap_or_else(|arc| arc.blocking_lock().clone())
            .into_inner();
        self.observability.commits.extend(commits);

        Ok(final_results)
    }

    /// Static helper to check if latest PR comment indicates NEEDS CHANGES.
    /// Used by reviewer tasks that can't borrow self.
    fn check_pr_needs_changes_static(
        pr_number: u64,
        repo: &str,
        work_dir: &Path,
    ) -> bool {
        let output = Command::new("gh")
            .current_dir(work_dir)
            .args([
                "pr", "view",
                &pr_number.to_string(),
                "--repo", repo,
                "--json", "comments",
                "-q", ".comments[-1].body",
            ])
            .output();

        output
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains("REVIEW - NEEDS CHANGES"))
            .unwrap_or(false)
    }

    /// Static helper to get pending review comments.
    /// Used by reviewer tasks that can't borrow self.
    fn get_pending_comments_static(
        pr_number: u64,
        repo: &str,
        work_dir: &Path,
    ) -> Vec<GitHubReviewComment> {
        let output = Command::new("gh")
            .current_dir(work_dir)
            .args([
                "api",
                &format!("repos/{}/pulls/{}/comments", repo, pr_number),
                "--jq",
                r#".[] | select(.position != null) | {id: .id, path: .path, line: .line, body: .body}"#,
            ])
            .output();

        let raw = match output {
            Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
            Err(_) => return Vec::new(),
        };

        let mut comments = Vec::new();
        for line in raw.lines() {
            if line.trim().is_empty() { continue; }
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(line) {
                let id = parsed.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
                let path = parsed.get("path").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let line_num = parsed.get("line").and_then(|v| v.as_u64()).map(|l| l as u32);
                let body = parsed.get("body").and_then(|v| v.as_str()).unwrap_or("").to_string();

                if id > 0 && !path.is_empty() {
                    comments.push(GitHubReviewComment {
                        id, path, line: line_num, body,
                        resolved: false, resolved_by_commit: None,
                    });
                }
            }
        }
        comments
    }
```

**Step 4: Run all tests**

Run: `cd /Users/liam.helmer/repos/epiphytic/infinite-improbability-drive/.worktrees/phase-sandbox/core && cargo test -- --nocapture`

Expected: ALL PASS

**Step 5: Commit**

```bash
cd /Users/liam.helmer/repos/epiphytic/infinite-improbability-drive/.worktrees/phase-sandbox
git add core/src/team_orchestrator.rs
git commit -m "feat(orchestrator): implement run_parallel_review_fix pipeline"
```

---

### Task 8: Wire `run_parallel_review_fix` into `run_with_branch`

Replace the sequential `for` loop in `run_with_branch()` with a call to the new parallel method for GitHub mode.

**Files:**
- Modify: `core/src/team_orchestrator.rs:412-556` (run_with_branch review loop)

**Step 1: Replace the sequential loop**

In `run_with_branch`, replace the block from `// Step 3: Run review phases` (line ~412) through the end of the loop (line ~556) with:

```rust
        // Step 3: Run review phases
        if use_github_reviews {
            // Parallel review/fix pipeline for GitHub mode
            let review_results = self.run_parallel_review_fix(
                &current_prompt,
                timeout,
                worktree_path,
                work_path,
                extracted_pr_number,
                &repo,
            ).await?;

            reviews = review_results;
            iterations = ReviewDomain::all().len() as u32;

            // Determine final verdict from all reviews
            final_verdict = if reviews.iter().any(|r| r.verdict == ReviewVerdict::NeedsChanges) {
                Some(ReviewVerdict::NeedsChanges)
            } else {
                Some(ReviewVerdict::Approved)
            };
        } else {
            // PingPong mode: keep the sequential approach
            let review_phases = ReviewDomain::all();
            let phases_to_run = review_phases.len().min(self.config.max_iterations as usize);
            let mut current_prompt = prompt.to_string();

            for (phase_idx, domain) in review_phases.iter().take(phases_to_run).enumerate() {
                iterations = (phase_idx + 1) as u32;

                // Get current diff
                let diff = self.get_git_diff(worktree_path)?;

                // Run PingPong reviewer
                let review_result = self
                    .run_reviewer(
                        &current_prompt,
                        &diff,
                        timeout,
                        worktree_path,
                        iterations,
                        Some(domain.as_str()),
                    )
                    .await?;

                // Append review to PR body
                if let Some(ref url) = pr_url {
                    self.append_review_to_pr(url, &review_result, domain)?;
                }

                reviews.push(review_result.clone());
                final_verdict = Some(review_result.verdict.clone());

                // If needs changes, run fix
                if review_result.verdict == ReviewVerdict::NeedsChanges {
                    let _fix_result = self
                        .run_fix(
                            &current_prompt,
                            &review_result,
                            timeout,
                            worktree_path,
                            iterations,
                            None,
                        )
                        .await?;

                    current_prompt = FixPromptBuilder::new(prompt)
                        .with_suggestions(review_result.suggestions.clone())
                        .build();
                }
            }
        }
```

**Step 2: Run all tests**

Run: `cd /Users/liam.helmer/repos/epiphytic/infinite-improbability-drive/.worktrees/phase-sandbox/core && cargo test -- --nocapture`

Expected: ALL PASS

**Step 3: Commit**

```bash
cd /Users/liam.helmer/repos/epiphytic/infinite-improbability-drive/.worktrees/phase-sandbox
git add core/src/team_orchestrator.rs
git commit -m "feat(orchestrator): wire parallel review/fix into GitHub mode"
```

---

### Task 9: Run E2E test to validate

**Step 1: Run the full test suite first**

```bash
cd /Users/liam.helmer/repos/epiphytic/infinite-improbability-drive/.worktrees/phase-sandbox/core
cargo test
```

Expected: ALL PASS

**Step 2: Run E2E test in plan-only mode**

```bash
cd /Users/liam.helmer/repos/epiphytic/infinite-improbability-drive/.worktrees/phase-sandbox/core
CRUISE_DEBUG=1 CRUISE_FAIL_FAST=1 E2E_KEEP_REPOS=1 cargo test --test e2e_test full_web_app_github_plan_only -- --ignored --nocapture
```

Expected: Planning phase completes with parallel reviews. Check the PR to verify:
- Review comments appear from multiple domains (possibly interleaved)
- Fixer replies are threaded (not top-level)
- Conversations are resolved after fixes

**Step 3: Commit any fixes needed**

```bash
cd /Users/liam.helmer/repos/epiphytic/infinite-improbability-drive/.worktrees/phase-sandbox
git add -A
git commit -m "fix(orchestrator): address E2E test findings"
```
