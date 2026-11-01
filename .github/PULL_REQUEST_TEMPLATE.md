# GPY Pull Request

## Description

What does this change do, and why? Link the issue it addresses.

Closes #ISSUE_NUMBER

## Testing

```bash
just check            # or ./scripts/quality-check.sh with GPY_GATE_STRICT=1
```

Which parts did you run, and did they pass? If you ran a narrower command
instead (e.g. `(cd gpy-agent && cargo nextest run)` or `./scripts/test-fish.sh`),
say which.

## Checklist

- [ ] Quality gates pass (`./scripts/quality-check.sh`, or the relevant
      subset — see [CONTRIBUTING.md](../CONTRIBUTING.md#quality-gates))
- [ ] Tests added or updated for behavior changes
- [ ] Docs updated for user-visible or developer-facing changes
