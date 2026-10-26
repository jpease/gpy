# Plugin Performance Budgets

This document defines the baseline expectations for plugin-framework overhead.

## Budgets

Discovery budgets, measured by the Criterion bench `gpy-agent/benches/plugin_bench.rs`:
- 0 plugins: `< 50ms`
- 10 plugins: `< 120ms`
- 50 plugins: `< 300ms`

The bench reports timings; it does not fail on them, because a wall-clock bound in a test measures the machine as much as the code.

## Non-Regression Policy

- Any PR that increases plugin discovery latency should include:
  - root-cause analysis,
  - before/after `plugin_bench` timings,
  - justification and mitigation plan.
- Regressions past a budget need explicit maintainer approval.

## How To Run

```bash
(cd gpy-agent && RUSTC_WRAPPER= cargo bench --bench plugin_bench)
```

For full release checks, also run:

```bash
./scripts/quality-check.sh
```
