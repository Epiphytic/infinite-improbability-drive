# E2E Test Results

This directory contains automated E2E test results for infinite-improbability-drive.

## Latest Results

| Test | Status | Date | Link |
|------|--------|------|------|
| full_web_app | ✅ PASSED | 2026-02-03 | [View](full_web_app-latest.md) |

## Test Descriptions

| Test | Description |
|------|-------------|
| `smoke_hello` | Basic smoke test - create simple file |
| `code_generation` | Generate Rust code with tests |
| `full_web_app` | Full workflow: Rust CLI with lib and integration tests |
| `full_workflow_simple` | Full plan→approve→execute workflow |
| `smoke_hello_gemini` | Smoke test using Gemini instead of Claude |

## Running Tests

```bash
# Run all E2E tests (repos deleted on success)
cargo test --test e2e_test -- --ignored

# Run specific test, keep repo for inspection
E2E_KEEP_REPOS=1 cargo test --test e2e_test full_web_app -- --ignored
```
