# E2E Test Result: full-web-app

**Status:** ❌ FAILED

**Date:** 2026-02-03

**Time:** 05:07:54 UTC

## Summary

| Metric | Value |
|--------|-------|
| Fixture | `full-web-app` |
| Spawn Success | true |
| Overall Passed | false |
| Repository | [`epiphytic/e2e-full-web-app-24320728`](https://github.com/epiphytic/e2e-full-web-app-24320728) |

## Pull Requests

- **Plan PR:** [View PR](https://github.com/Epiphytic/e2e-full-web-app-24320728/pull/1)

## Validation

**Passed:** false

### Checks

- ❌ Missing expected file: Cargo.toml
- ❌ Missing expected file: src/main.rs
- ❌ Missing expected file: src/lib.rs
- ❌ Missing expected file: static/index.html
- ❌ Missing expected file: .github/workflows/lint.yml
- ❌ Missing expected file: .github/workflows/dependency-review.yml
- ✅ Build passed: cargo build --release
- ✅ Tests passed: cargo test --lib
- ❌ E2E tests failed: cargo test --test e2e

