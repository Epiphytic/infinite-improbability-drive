# Spawn-Team Architecture

> Multi-LLM coordination for autonomous development workflows

## Overview

Spawn-team enables coordination between multiple LLMs (typically Claude as primary coder and Gemini as reviewer) to produce higher-quality code through iterative review cycles.

## Coordination Modes

### Sequential Mode

The simplest coordination pattern:

```
Primary LLM ──► Reviewer LLM ──► Fix (once)
```

1. Primary completes the task
2. Reviewer evaluates once
3. Primary fixes issues (if any)
4. Done

**Use case:** Simple tasks that don't need multiple review rounds.

### PingPong Mode

Iterative back-and-forth until approval:

```
┌────────────────────────────────────────────────────┐
│                                                    │
│   Primary ──► Reviewer ──► Fix ──► Reviewer ──►   │
│       ▲                              │             │
│       └──────────── loop ◄───────────┘             │
│           (until approved or max iterations)       │
│                                                    │
└────────────────────────────────────────────────────┘
```

Review feedback is captured and appended to PR body for traceability.

**Use case:** Complex tasks requiring multiple refinement rounds.

### GitHub Mode (Default)

PR-based coordination using GitHub as the communication medium:

```
┌─────────────────────────────────────────────────────────────────┐
│                         GitHub Mode Flow                         │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  1. Primary creates initial work                                │
│     └──► Watcher commits and pushes                             │
│                                                                  │
│  2. PR created on first commit                                  │
│     └──► Initial PR body with prompt and observability          │
│                                                                  │
│  3. Review phases run sequentially:                             │
│     ┌─────────────────────────────────────────────────────┐     │
│     │  Security ──► TechnicalFeasibility ──► Granularity  │     │
│     │      └──► DependencyCompleteness ──► GeneralPolish  │     │
│     └─────────────────────────────────────────────────────┘     │
│                                                                  │
│  4. For each phase with issues:                                 │
│     a. Reviewer posts PR comment: [DOMAIN REVIEW - NEEDS CHANGES]│
│     b. Coder reads file and edits to address feedback           │
│     c. Commit with message referencing comment                   │
│     d. Reply to comment with commit hash                         │
│                                                                  │
│  5. Final PR ready for human review or auto-merge               │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

**Key features:**
- Full traceability on the PR
- Line-specific comments where possible
- Each review phase is distinct and focused
- Human can intervene at any point

**Use case:** Production workflows, human oversight required, audit trails needed.

## Review Domains

Each review phase focuses on a specific domain:

| Phase | Domain | Focus |
|-------|--------|-------|
| 1 | Security | Auth, injection, secrets, OWASP Top 10 |
| 2 | TechnicalFeasibility | Architecture, performance, tech stack |
| 3 | TaskGranularity | Task sizing for parallelization |
| 4 | DependencyCompleteness | Missing dependencies, parallelization opportunities |
| 5 | GeneralPolish | Code quality, documentation, final fixes |

Each domain has specific review instructions in `ReviewDomain::instructions()`.

## GitHub Mode Details

### PR Creation

PR is created on the first commit pushed:
- Title from prompt (truncated to 70 chars)
- Body includes original prompt in accordion
- Observability data appended after reviews

### Review Comments

Since the same user can't use `gh pr review --request-changes` on their own PRs, reviewers use PR comments instead:

```bash
# Review needs changes
gh pr comment {PR_NUMBER} --repo {REPO} --body "[SECURITY REVIEW - NEEDS CHANGES]

{detailed findings}"

# Review approved
gh pr comment {PR_NUMBER} --repo {REPO} --body "[SECURITY REVIEW - APPROVED]

Security review passed with no issues."
```

### Line-Specific Comments

For line-specific feedback:

```bash
gh api repos/{REPO}/pulls/{PR}/comments --method POST \
  -f body="Your comment" \
  -f path="path/to/file" \
  -F line=42 \
  -f commit_id="$(gh pr view --repo {REPO} {PR} --json headRefOid --jq .headRefOid)"
```

### Comment Resolution

When the coder resolves a comment:
1. Read the file mentioned in the comment
2. Edit the file to address the feedback
3. Commit changes
4. Reply to the comment with commit hash

## SpawnTeamOrchestrator

The `SpawnTeamOrchestrator` (`core/src/team_orchestrator.rs`) manages the full workflow:

```rust
pub struct SpawnTeamOrchestrator<P: SandboxProvider + Clone> {
    config: SpawnTeamConfig,
    provider: P,
    logs_dir: PathBuf,
    observability: SpawnObservability,
    env_vars: HashMap<String, String>,
}
```

### Key Methods

- `run()` - Main entry point
- `run_with_branch()` - Run with explicit branch name
- `run_primary()` - Execute primary LLM
- `run_reviewer()` - Execute reviewer for PingPong mode
- `run_github_reviewer()` - Execute reviewer for GitHub mode
- `resolve_github_comment()` - Have coder resolve a PR comment

## Observability

Comprehensive data captured during execution:

```rust
pub struct SpawnObservability {
    pub command_lines: Vec<CommandLineRecord>,
    pub permissions_requested: Vec<PermissionRecord>,
    pub permissions_granted: Vec<PermissionRecord>,
    pub review_feedback: Vec<ReviewFeedbackRecord>,
    pub security_findings: Vec<SecurityFinding>,
    pub sandbox_path: Option<PathBuf>,
    pub commits: Vec<CommitRecord>,
    pub pr_url: Option<String>,
    pub github_reviews: Vec<GitHubReview>,
    pub resolved_comments: Vec<ResolvedCommentRecord>,
}
```

This data is formatted as markdown and appended to PRs.

## Configuration

```toml
[spawn-team]
mode = "github"           # sequential | pingpong | github
max_iterations = 3        # Max review cycles (PingPong) or phases (GitHub)
primary_llm = "claude-code"
reviewer_llm = "gemini-cli"
max_escalations = 5       # Permission escalations per spawn
```

## Prompt Builders

### ReviewPromptBuilder

For PingPong mode reviews:
- Includes original prompt
- Includes git diff
- Requests JSON response with verdict and suggestions

### GitHubReviewPromptBuilder

For GitHub mode reviews:
- Includes domain-specific instructions
- Includes PR number and repo
- Instructs use of `gh pr comment` for posting

### ResolveCommentPromptBuilder

For resolving GitHub comments:
- Includes comment details (file, line, body)
- Instructs to use Read and Edit tools
- Commit and reply handled automatically

## Error Handling

- If primary fails: Return failure immediately
- If reviewer fails: Continue with approval (don't block on review)
- If comment resolution fails: Log warning, continue to next comment
- If PR creation fails: Return failure with error message

## Related Documentation

- [Cruise-Control Architecture](./cruise-control.md)
- [Spawn Architecture](./architecture.md)
- [AGENTS.md](../AGENTS.md)
