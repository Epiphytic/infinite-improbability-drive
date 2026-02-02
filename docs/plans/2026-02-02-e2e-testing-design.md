# E2E Testing Infrastructure Design

> **Goal:** Enable real end-to-end testing of the spawn framework using actual Claude and Gemini CLIs against ephemeral GitHub repositories.

## Overview

The E2E test infrastructure will:
1. Create ephemeral GitHub repos in the Epiphytic org
2. Run spawn with configurable LLM runners (Claude/Gemini)
3. Validate results at multiple levels (file existence → full test suite)
4. Clean up repos after tests complete

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                         E2E Test Harness                        │
├─────────────────────────────────────────────────────────────────┤
│  1. Load fixture (prompt, validation config)                    │
│  2. Create ephemeral GitHub repo                                │
│  3. Run spawn with configured LLM runner                        │
│  4. Validate results (files, build, tests)                      │
│  5. Cleanup (delete repo)                                       │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                      Spawner (async)                            │
├─────────────────────────────────────────────────────────────────┤
│  spawn() ──► WatcherAgent::run() ──► LLMRunner::spawn()        │
│                    │                        │                   │
│                    ▼                        ▼                   │
│            ProgressMonitor           Claude/Gemini CLI          │
│            PermissionDetector        (streaming output)         │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                      Sandbox (WorktreeSandbox)                  │
├─────────────────────────────────────────────────────────────────┤
│  Git worktree isolation with manifest-based permissions         │
└─────────────────────────────────────────────────────────────────┘
```

## Key Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| LLM Runners | Configurable (Claude default, Gemini optional) | Flexibility without blocking progress |
| Test Tasks | Configurable YAML fixtures | Version-controlled, reproducible, graduated complexity |
| Repo Lifecycle | Ephemeral (create/delete per test) | Clean isolation, no state leakage |
| Validation | Full with configurable levels | Thorough E2E with fast smoke test option |
| Runtime | Async (tokio) | Matches LLM streaming nature |

## Fixture Format

```yaml
# tests/e2e/fixtures/smoke-hello.yaml
name: "smoke-hello"
description: "Minimal smoke test - create a single file"
runner: "claude"  # or "gemini", defaults to "claude"

prompt: "Create a file called hello.txt containing 'Hello, World!'"

validation:
  level: "file_exists"
  expected_files:
    - "hello.txt"
  expected_content:
    "hello.txt": "Hello, World!"

timeout: 60  # seconds
```

```yaml
# tests/e2e/fixtures/full-web-app.yaml
name: "full-web-app"
description: "Complete web application with auth and tests"
runner: "claude"

prompt: |
  Build a web application that is a simple web UI to an SQLite database.
  It should incorporate JWT authentication using a locally generated
  private key and CA. Must have unit tests, end-to-end tests that show
  the ability to add and delete tables through the API, and a test of
  log in to the web UI using Playwright.

validation:
  level: "full"
  build_command: "cargo build --release"
  test_command: "cargo test"
  e2e_command: "cargo test --test e2e"
  expected_files:
    - "Cargo.toml"
    - "src/main.rs"
    - "tests/"

timeout: 1800  # 30 minutes for complex builds
```

### Validation Levels

| Level | Checks | Use Case |
|-------|--------|----------|
| `file_exists` | Expected files created | Smoke tests |
| `build` | Files + build succeeds | Code generation tests |
| `test` | Build + unit tests pass | Feature tests |
| `full` | Build + tests + e2e tests | Complete app tests |

## GitHub Repo Lifecycle

```
Test Start
    │
    ▼
┌─────────────────────────────────────────┐
│  gh repo create epiphytic/e2e-{uuid}   │
│  --public --clone                       │
└─────────────────────────────────────────┘
    │
    ▼
┌─────────────────────────────────────────┐
│  Initialize repo with minimal files:    │
│  - .gitignore                           │
│  - README.md (test metadata)            │
│  - Initial commit                       │
└─────────────────────────────────────────┘
    │
    ▼
┌─────────────────────────────────────────┐
│  Run spawn in repo directory            │
│  (LLM works in this repo)               │
└─────────────────────────────────────────┘
    │
    ▼
┌─────────────────────────────────────────┐
│  Validate results                       │
│  (check files, run build/tests)         │
└─────────────────────────────────────────┘
    │
    ▼
┌─────────────────────────────────────────┐
│  gh repo delete epiphytic/e2e-{uuid}   │
│  --yes                                  │
└─────────────────────────────────────────┘
    │
    ▼
Test Complete (pass/fail)
```

**Error Handling:**
- Repo creation fails → test fails immediately
- Spawn fails → still cleanup repo, report spawn error
- Cleanup fails → log warning, don't mask real failure

## Code Changes Required

### 1. `core/src/main.rs` - Make async

```rust
#[tokio::main]
async fn main() {
    // ... existing setup ...

    // Runner selection from env
    let runner: Box<dyn LLMRunner> = match std::env::var("SPAWN_RUNNER") {
        Ok(r) if r == "gemini" => Box::new(GeminiRunner::new()),
        _ => Box::new(ClaudeRunner::new()),
    };

    // Async spawn
    match spawner.spawn(config, manifest, runner).await {
        // ... handle result ...
    }
}
```

### 2. `core/src/spawn.rs` - Wire WatcherAgent

```rust
pub async fn spawn<R: LLMRunner + 'static>(
    &self,
    config: SpawnConfig,
    manifest: SandboxManifest,
    runner: R,
) -> Result<SpawnResult> {
    // ... existing sandbox creation ...

    // Create and run watcher
    let watcher_config = WatcherConfig::from(&config);
    let watcher = WatcherAgent::new(
        self.provider.clone(),
        runner,
        watcher_config,
    );

    let watcher_result = watcher.run(llm_config).await?;

    // Convert WatcherResult to SpawnResult
    // ...
}
```

### 3. `core/Cargo.toml` - Add tokio runtime feature

```toml
[dependencies]
tokio = { version = "1", features = ["full", "rt-multi-thread"] }
```

## New Module Structure

```
core/
├── src/
│   └── e2e/
│       ├── mod.rs           # Module exports
│       ├── fixture.rs       # Fixture loading and parsing
│       ├── repo.rs          # GitHub repo lifecycle
│       ├── validator.rs     # Result validation engine
│       └── harness.rs       # Main test orchestrator
└── tests/
    └── e2e/
        ├── fixtures/
        │   ├── smoke-hello.yaml
        │   ├── code-generation.yaml
        │   └── full-web-app.yaml
        └── e2e_test.rs      # Integration test entry point
```

### Key Types

```rust
// fixture.rs
pub struct Fixture {
    pub name: String,
    pub prompt: String,
    pub runner: RunnerType,  // Claude | Gemini
    pub validation: ValidationConfig,
    pub timeout: Duration,
}

// harness.rs
pub struct E2EHarness {
    pub org: String,  // "epiphytic"
}

impl E2EHarness {
    pub async fn run_fixture(&self, fixture: &Fixture) -> E2EResult;
}

// validator.rs
pub enum ValidationLevel {
    FileExists,
    Build,
    Test,
    Full,
}
```

## Implementation Phases

| Phase | Duration | Deliverable |
|-------|----------|-------------|
| 1. Async Foundation | 1-2 days | Async CLI and Spawner |
| 2. Spawner-Watcher Integration | 2-3 days | Working spawn with real LLM |
| 3. E2E Infrastructure | 2-3 days | Test harness with repo lifecycle |
| 4. Test Fixtures | 1-2 days | Graduated test cases |
| 5. Polish & CI | 1 day | Docs, CI workflow |

**Total Estimate: 7-11 days**

## Success Criteria

The implementation is complete when:

1. `cargo test --test e2e smoke_hello` creates a repo, runs Claude, verifies `hello.txt`, deletes repo
2. `SPAWN_RUNNER=gemini cargo test --test e2e smoke_hello` does the same with Gemini
3. `cargo test --test e2e full_web_app` builds a complete web application with auth and passing tests
4. All E2E tests clean up their repos even on failure
