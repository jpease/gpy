# Build a Segment Plugin

How to create a custom prompt segment plugin that runs across Bash, Zsh, and Fish.

For complete specification and contract details, see [Plugin Framework](plugins.md).

## Overview

GPY script plugins (`entry_type = "script"`) provide segments implemented as POSIX `sh` scripts. They run consistently across all supported shells (Bash, Zsh, and Fish) without requiring shell-specific prompt code.

Each script plugin consists of:
- `plugin.toml`: Top-level plugin package manifest
- `segments/<name>.toml`: Segment manifest declaring inputs, triggers, cache TTL, and settings
- `bin/<script>.sh`: POSIX `sh` executable script rendering segment output

Reference implementations are available in [`docs/dev/examples/plugins/`](examples/plugins/):
- `k8s-context`: Demonstrates environment inspection (`KUBECONFIG`) and typed settings
- `hostname-example` and `username-example`: Session-scoped segments with variant styling

## Scaffolding a New Plugin

Use `gpy plugin new` to scaffold the directory structure and starter files:

```bash
gpy plugin new acme-tools --segment acme-status
```

This creates:

```text
acme-tools/
├── plugin.toml
├── segments/
│   └── acme-status.toml
└── bin/
    └── acme-status.sh
```

### 1. Plugin Manifest (`plugin.toml`)

The top-level manifest defines the plugin identity and provided segments:

```toml
id = "acme-tools"
name = "Acme Tools"
version = "0.1.0"
api_version = "v1"
entry_type = "script"
provided_segments = ["acme-status"]
description = "Acme tooling prompt status"

[compatibility_constraints]
min_gpy_version = "0.1.0"
```

### 2. Segment Manifest (`segments/<name>.toml`)

Configure runtime behavior, cache rules, triggers, and settings:

```toml
description = "Acme status indicator"
command = "bin/acme-status.sh"
inputs = ["is_ssh"]
ttl_secs = 5
timeout_ms = 300
require_command = "bin/acme-status.sh"

[settings.style]
type = "string"
default = "normal"
choices = ["normal", "bold"]
description = "Display style"
```

- **Inputs**: Pass shell state such as `is_ssh` or `pwd`. See [Inputs: public shell state](plugins.md#inputs-public-shell-state).
- **Settings**: Declare typed options users can configure under `[plugin.<segment>]`. See [Settings](plugins.md#settings).

### 3. Segment Script (`bin/<script>.sh`)

The script runs with `/bin/sh` and outputs up to two lines:
1. Segment text to display (empty output hides the segment)
2. Optional style variant name matching `^[a-z0-9_]{1,32}$`

```sh
#!/bin/sh
if ! command -v acme >/dev/null 2>&1; then
    exit 0
fi

status=$(acme status 2>/dev/null)
if [ -n "$status" ]; then
    printf '%s\n' "$status"
    # Optional line 2 variant:
    # printf '%s\n' "active"
fi
```

See [Output protocol](plugins.md#output-protocol) for formatting and sanitization rules.

## Installation and Trust Checks

Script segments run under strict ownership and permissions requirements to ensure security:
- The script and every parent directory up to the plugin root must be owned by you or root.
- No directory or file in the path may be group- or world-writable.

See [Trust](plugins.md#trust) for the full trust model.

Use `gpy plugin install` to copy the plugin to `$XDG_CONFIG_HOME/gpy/plugins/` with the appropriate file modes and permissions:

```bash
gpy plugin install acme-tools/
```

## Validation and Discovery

Validate the plugin against the contract schema and trust constraints:

```bash
gpy plugin validate ~/.config/gpy/plugins/acme-tools
```

Check that GPY discovers the plugin and its segment:

```bash
gpy plugin list
gpy segments
```

## Enabling the Segment

Enable the segment in your active prompt configuration:

```bash
gpy enable acme-status
```

Verify that the segment appears in `ui.enabled_segments`:

```bash
gpy config get ui.enabled_segments
```

## Styling in Themes

Define styling for your segment in your theme file (`~/.config/gpy/themes/<theme>.toml`):

```toml
[segments.acme-status]
icon = "A"
bg_color = "#1d3557"
text_color = "#f1faee"
```

For variants emitted on line 2 (e.g. `active`), define variant styles:

```toml
[segments.acme-status]
icon = "A"
bg_color = "#1d3557"
text_color = "#f1faee"
active_bg_color = "#457b9d"
active_text_color = "#ffffff"
```

Validate and apply the theme:

```bash
gpy theme validate
gpy theme use <theme>
```

For full theme documentation, see [Create a Theme](create-theme.md).

## Performance and Best Practices

- **Keep execution fast**: Script timeouts are capped at 500 ms (default 300 ms).
- **Use triggers**: Use `require_env`, `require_files`, or `require_command` so scripts only run when necessary.
- **Cache appropriately**: Configure `ttl_secs` to avoid running scripts on every single prompt.
- Discovery budgets are documented in [Plugin Budgets](performance/plugin-budgets.md).
