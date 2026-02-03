# E2E Test Result: full-web-app-github

**Status:** ❌ FAILED

**Date:** 2026-02-03

**Time:** 10:04:35 UTC

## Summary

| Metric | Value |
|--------|-------|
| Fixture | `full-web-app-github` |
| Spawn Success | true |
| Overall Passed | false |
| Repository | [`epiphytic/e2e-full-web-app-github-8b4ac3f4`](https://github.com/epiphytic/e2e-full-web-app-github-8b4ac3f4) |

## Pull Requests

- **Plan PR:** [View PR](https://github.com/Epiphytic/e2e-full-web-app-github-8b4ac3f4/pull/1)

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
- ❌ Tests failed: cargo test --lib
- ❌ E2E tests failed: cargo test --test e2e

