# E2E Test Result: full-web-app-github

**Status:** ❌ FAILED

**Date:** 2026-02-03

**Time:** 10:56:54 UTC

## Summary

| Metric | Value |
|--------|-------|
| Fixture | `full-web-app-github` |
| Spawn Success | true |
| Overall Passed | false |
| Repository | [`epiphytic/e2e-full-web-app-github-1b24b8ff`](https://github.com/epiphytic/e2e-full-web-app-github-1b24b8ff) |

## Pull Requests

- **Plan PR:** [View PR](https://github.com/Epiphytic/e2e-full-web-app-github-1b24b8ff/pull/1)

## Validation

**Passed:** false

### Checks

- ✅ Found expected file: Cargo.toml
- ✅ Found expected file: src/main.rs
- ❌ Missing expected file: src/lib.rs
- ❌ Missing expected file: static/index.html
- ✅ Found expected file: .github/workflows/lint.yml
- ✅ Found expected file: .github/workflows/dependency-review.yml
- ✅ Build passed: cargo build --release
- ❌ Tests failed: cargo test --lib
- ❌ E2E tests failed: cargo test --test e2e

