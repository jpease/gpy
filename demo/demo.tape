# GPY feature walkthrough.
#
# Generates ../assets/demo.gif. Don't run `vhs demo/demo.tape` directly --
# invoke `just demo` (or `./demo/setup.fish`) instead. That builds an
# isolated fixture $HOME first and execs this tape from the repo root with
# that environment already exported, so the recording never touches your
# real ~/.config/gpy, ~/.local/bin, or default tmux server.

Output "assets/demo.gif"

Require fish
Require tmux
Require gpy

Set Shell "fish"
Set FontFamily "JetBrainsMono Nerd Font Mono"
Set FontSize 16
Set Width 1200
Set Height 700
Set Padding 20
Set Theme "Catppuccin Mocha"
Set TypingSpeed 40ms

# The very first fish session against a fresh install needs the restart
# install-dev.fish itself tells you to do ("Restart Fish: exec fish") before
# segments populate (fish universal vars written by GPY's init aren't picked
# up mid-session) -- do that restart here, hidden, before the beats that
# matter begin. Later panes/sessions in this recording don't need their own
# restart: those universal vars are already on disk once this one is done.
Hide
Type "cd ~/workspace/gpy-tour"
Enter
Sleep 500ms
Type "exec fish"
Enter
Sleep 2s
Type "clear"
Enter
Sleep 500ms
Show

# ── Beat 1: fast, language-aware prompt ────────────────────────────────
# cd through Node / Python / Rust sub-projects (each its own repo) -- the
# language segment picks up each project's version instantly, with no lag
# between hops. Ends in engine/ for beat 2, which needs a git repo.
Type "cd api"
Enter
Sleep 1.3s
Type "cd ../service"
Enter
Sleep 1.3s
Type "cd ../engine"
Enter
Sleep 1.3s

# ── Beat 2: live updates via the async agent, across two panes ─────────
# Split the terminal, mutate a tracked file from a backgrounded, delayed
# job, then just sit idle. Both prompts refresh live -- no keypress, no
# Enter, in either pane.
Hide
Type "tmux -L gpydemo kill-server 2>/dev/null; true"
Enter
Sleep 300ms
Show

Type "tmux -L gpydemo new-session -s tour fish"
Enter
Sleep 1s
Type "tmux -L gpydemo split-window -h -c '#{pane_current_path}' fish"
Enter
Sleep 1s
Type "sh -c 'sleep 2; echo tour >> NOTES.md' >/dev/null 2>&1 &"
Enter
Sleep 4s
Type "tmux -L gpydemo kill-session -t tour"
Enter
Sleep 800ms
Type "cd .."
Enter
Sleep 1s

# ── Beat 3: theme system, including a real Starship preset import ──────
Type "gpy theme list"
Enter
Sleep 1.8s
Type "gpy theme import starship-preset.toml --name tour-starship 2>/dev/null"
Enter
Sleep 1.8s
Type "gpy theme use tour-starship"
Enter
Sleep 2s

# ── Beat 4: palette system, including a real base16 (Tokyo Night) import ─
# Back to the default theme AND palette first: `theme use tour-starship`
# switched the palette too (its recommended pairing), and tour-starship's
# segment templates reference color names that only that paired palette
# defines -- not a fair pairing for a base16 palette swap.
Type "gpy theme use default"
Enter
Sleep 800ms
Type "gpy palette use default"
Enter
Sleep 1s
Type "gpy palette list"
Enter
Sleep 1.8s
Type "gpy palette import tokyo-night.yaml"
Enter
Sleep 1.8s
Type "gpy palette use tokyo-night-dark"
Enter
Sleep 2s

# ── Close on a clean bill of health ─────────────────────────────────────
Type "gpy doctor"
Enter
Sleep 2.5s
