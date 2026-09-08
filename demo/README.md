# GPY Demo GIF

Records `demo.tape` with [VHS](https://github.com/charmbracelet/vhs) into
`../assets/demo.gif`, which is embedded in the root `README.md`.

## Prerequisites

```bash
brew install vhs tmux
```

VHS also needs `ttyd` and `ffmpeg` (pulled in automatically by the `vhs`
Homebrew formula). For icons to render, install a [Nerd
Font](https://www.nerdfonts.com) and make sure `JetBrainsMono Nerd Font
Mono` (or update `Set FontFamily` in `demo.tape` to match whatever you have)
is available to your system's font renderer.

## Recording

```bash
just demo
```

This runs `setup.fish`, which:

1. Builds a release binary and installs it into an isolated fixture `$HOME`
   at `.fixture-home/` (gitignored, wiped and rebuilt every run) -- your
   real `~/.config/gpy`, `~/.local/bin`, and default tmux server are never
   touched.
2. Seeds a small multi-language "tour" workspace (`workspace/gpy-tour/`,
   with `api/`/`service/`/`engine/` as three independent Node/Python/Rust
   git repos -- separate repos so language detection actually changes as
   the tape moves between them) plus two real fixtures already used by the
   Rust test suite: a Starship preset
   (`gpy-agent/tests/fixtures/starship/preset_gruvbox_rainbow.toml`) and a
   base16 scheme (`gpy-agent/tests/fixtures/base16/tokyo-night-dark.yaml`),
   for the theme/palette import beats.
3. Runs `vhs demo.tape`, which inherits that fixture environment.

To iterate quickly without rebuilding the agent every time:

```bash
./demo/setup.fish --no-build
```

(requires a `gpy-agent/target/release/` binary from a previous full run).

## Editing the tape

`demo.tape` references the fixture only through `~` (i.e. `$HOME`), so it
never hardcodes an absolute path -- it works regardless of where the repo is
checked out. See VHS's own reference (`vhs manual`, or the [project
README](https://github.com/charmbracelet/vhs#vhs-command-reference)) for the
tape syntax.

If you change the tour repo's shape in `setup.fish` (new sub-project, renamed
file, etc.), update `demo.tape` to match -- there's no dependency-checking
between the two.
