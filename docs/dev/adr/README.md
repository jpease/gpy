# Architecture Decision Records

This directory contains Architecture Decision Records (ADRs) for GPY.

## What is an ADR?

An Architecture Decision Record (ADR) captures an important architectural decision made along with its context and consequences.

## Format

Each ADR follows this structure:

```markdown
# ADR-NNNN: [Title]

## Status
[Proposed | Accepted | Deprecated | Superseded]

## Context
What is the issue that we're seeing that is motivating this decision or change?

## Decision
What is the change that we're proposing and/or doing?

## Consequences
What becomes easier or more difficult to do because of this change?
```

## Index

- [ADR-0001](adr-0001-hybrid-rust-fish-architecture.md) - Hybrid Rust/Fish Architecture
- [ADR-0002](adr-0002-unix-domain-sockets-for-ipc.md) - Unix Domain Sockets for IPC
- [ADR-0003](adr-0003-gix-for-git-operations.md) - Gix (Gitoxide) for Git Operations (**superseded** by the native git subprocess backend)
- [ADR-0004](adr-0004-sigusr1-live-updates.md) - SIGUSR1 for Live Prompt Updates (**superseded in part** by ADR-0007)
- [ADR-0005](adr-0005-theme-hot-reload.md) - Theme and Config Hot-Reload (**superseded in part** by ADR-0007)
- [ADR-0006](adr-0006-async-terminal-updates.md) - Async Terminal Updates
- [ADR-0007](adr-0007-sigurg-doorbell-notifications.md) - SIGURG Doorbell and Flag Files for Shell Notifications

## Creating a New ADR

1. Copy the template: `cp adr-template.md adr-NNNN-short-title.md`
2. Fill in the sections
3. Update this index
4. Create a pull request
