# E2E Test Result: full-web-app

**Status:** ❌ FAILED

**Date:** 2026-02-02

**Time:** 23:58:40 UTC

## Summary

| Metric | Value |
|--------|-------|
| Fixture | `full-web-app` |
| Spawn Success | true |
| Overall Passed | false |
| Repository | [`epiphytic/e2e-full-web-app-e7d8d2ff`](https://github.com/epiphytic/e2e-full-web-app-e7d8d2ff) |

## Pull Requests

- **Plan PR:** [View PR](https://github.com/Epiphytic/e2e-full-web-app-e7d8d2ff/pull/1)

## Validation

**Passed:** false

### Checks

- ❌ Missing expected file: Cargo.toml
- ❌ Missing expected file: src/main.rs
- ❌ Missing expected file: src/lib.rs
- ❌ Missing expected file: tests/integration.rs
- ✅ Build passed: cargo build --release
- ❌ Tests failed: cargo test --lib
- ❌ E2E tests failed: cargo test --test integration

