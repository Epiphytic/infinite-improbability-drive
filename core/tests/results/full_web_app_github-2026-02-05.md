# E2E Test Result: full-web-app-github

**Status:** ❌ FAILED

**Date:** 2026-02-05

**Time:** 18:07:25 UTC

## Summary

| Metric | Value |
|--------|-------|
| Fixture | `full-web-app-github` |
| Spawn Success | true |
| Overall Passed | false |
| Repository | [`epiphytic/e2e-full-web-app-github-df0fcda5`](https://github.com/epiphytic/e2e-full-web-app-github-df0fcda5) |

## Pull Requests

- **Plan PR:** [View PR](https://github.com/Epiphytic/e2e-full-web-app-github-df0fcda5/pull/1)
- **Implementation PR:** [View PR](https://github.com/Epiphytic/e2e-full-web-app-github-df0fcda5/pull/2)

## Validation

**Passed:** false

### Checks

- ✅ Found expected file: Cargo.toml
- ✅ Found expected file: src/main.rs
- ✅ Found expected file: templates/
- ❌ Missing expected file: .github/workflows/lint.yml
- ❌ Missing expected file: .github/workflows/dependency-review.yml
- ✅ Build passed: cargo build --release
- ❌ Tests failed: cargo test

