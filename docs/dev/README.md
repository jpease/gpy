# GPY Developer Documentation

Welcome to the developer documentation for GPY!

## 🏗️ Architecture & Design

- [Architecture Overview](architecture.md) - High-level system design.
- [Background Behavior](background-behavior.md) - Agent discoverability and idle behavior.
- [Design Decisions](design-decisions.md) - Core principles and rationale.
- [Architectural Decision Records (ADR)](adr/README.md) - Historical record of key technical choices.

## 🛠️ Development Guides

- [Segment Development](segment-development.md) - How to write new prompt segments.
- [Adding a New Segment](add-new-segment.md) - Step-by-step tactic for adding segments.
- [Plugin Framework](plugins.md) - Manifest, discovery, and runtime segment loading contract.
- [Build a Segment Plugin](build-segment-plugin.md) - Step-by-step guide for community segment plugins.
- [Create a Theme](create-theme.md) - Step-by-step guide for community theme authors.
- [Plugin + Theme Authoring](plugin-theme-authoring.md) - Short index for extension authoring docs.

## ✅ Testing

- [Testing Tiers](testing.md) - Every test tier (Rust, CLI, Fish content and PTY sessions, Bash/Zsh live daemon, installers, release smoke, container fresh install, Windows, coverage), what each proves, and the command that runs it.

## 🚢 Releasing

- [Releasing GPY](releasing.md) - Cutting a release, the version-bearing files, and which distribution channels are operational.

## 🔒 Security & Audits

- [Security Audit Notes](security-audit.md) - Dependency analysis and vulnerability tracking.

## ⚡ Performance

- [Benchmarking Methodology](performance/benchmarking.md) - Repeatable benchmark workflow.
- [Performance Baselines](performance/performance-baselines.md) - How to run and update baselines.
- [Plugin Budgets](performance/plugin-budgets.md) - Plugin discovery performance budgets and checks.
- [Optimizations (gix era, archived)](../archive/gix-era-optimizations.md) - Superseded by the native git subprocess backend; kept for benchmark methodology and history.

## 🏠 Back to Project
- [Project Root](../../README.md)
