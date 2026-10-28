# Plugin and Theme Authoring

Use the focused guides instead of treating this page as the primary reference:

- [Build a Segment Plugin](build-segment-plugin.md)
- [Create a Theme](create-theme.md)
- [Plugin Framework](plugins.md)
- [Plugin Manifest Template](templates/plugin.toml)
- [Theme Template with Plugin Segment Styling](templates/theme.plugin-segment.toml)

Plugin segments from v1 `.fish` files load in Fish only; see [Plugin Framework](plugins.md#segment-runtime-contract).

## Which Doc to Read

Read [Build a Segment Plugin](build-segment-plugin.md) if you are:

- creating a new community segment
- writing a `plugin.toml`
- implementing Fish `detect` and `render` functions

Read [Create a Theme](create-theme.md) if you are:

- designing a new theme
- styling built-in or plugin segments
- validating and shipping a theme

Read [Plugin Framework](plugins.md) if you need:

- the manifest contract
- precedence and discovery rules
- compatibility guarantees
- versioning policy

## Themes Shipped in a Plugin

A plugin's `themes/` directory may hold full themes or overlays. A plugin overlay must extend a builtin theme (`extends = "starship"`) and always gets the embedded builtin, never a user or another plugin's theme of that name; `gpy plugin validate` checks it.

Users who customize a plugin theme with `gpy theme new <name> --from <plugin-theme>`, `gpy theme save` or `gpy config wizard` get an overlay that extends it, so their theme depends on your plugin: `gpy` names the plugin when it writes the file, and the theme stops loading if the plugin is removed or disabled. Renaming or removing a shipped theme therefore breaks such overlays. `--full` gives users an independent copy instead.
