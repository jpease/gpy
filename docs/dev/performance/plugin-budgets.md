# Plugin Performance Budgets

This document defines the baseline expectations for plugin-framework overhead.

## Budgets

Discovery budgets (measured in `gpy-agent/tests/plugin_performance_tests.rs`):
- 0 plugins: `< 50ms`
- 10 plugins: `< 120ms`
- 50 plugins: `< 300ms`

These tests are intentionally conservative so they remain stable across CI hosts while still catching major regressions.

## Non-Regression Policy

- Any PR that materially increases plugin discovery latency should include:
  - root-cause analysis,
  - before/after timings,
  - justification and mitigation plan.
- Regressions should fail tests unless explicitly approved by maintainers.

## How To Run

```bash
(cd gpy-agent && RUSTC_WRAPPER= cargo test --test plugin_performance_tests)
```

For full release checks, also run:

```bash
./scripts/quality-check.sh
```
