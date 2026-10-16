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
Require zsh
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
#
# $PWD is the repo root only right now -- `vhs` was invoked from there and
# nothing has cd'd yet. Stash it exported so beat 2 can source the repo's
# zsh integration by path once it's several directories and one exec deep.
Hide
Type "set -gx GPY_REPO_ROOT $PWD"
Enter
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
# cd through node-app / python-app / rust-app sub-projects (each its own
# repo) -- the language segment picks up each project's version instantly,
# with no lag between hops. Also pins node-app to a specific Node via
# `mise use` while there, showing the segment follows a per-project version
# pin, not just which language is present. Ends in rust-app/ for beat 2,
# which needs a git repo.
#
# Beat headers are drawn inside a Hide block so neither the typing nor the
# echo itself is ever on camera -- only its output. `clear` runs in the same
# hidden block because Hide only stops frame capture: the typed command is
# still on screen when Show resumes.
Hide
Type "clear; echo (set_color -o 24273a -b 8aadf4)(string pad -r -w $COLUMNS '  ▸ Fast, language-aware prompt')(set_color normal); echo"
Enter
Sleep 300ms
Show
Sleep 900ms
Type "cd node-app"
Enter
Sleep 1.3s
Type "mise use node@22.22.3"
Enter
Sleep 1.5s
Type "cd ../python-app"
Enter
Sleep 1.3s
Type "cd ../rust-app"
Enter
Sleep 1.3s

# ── Beat 2: live updates via the async agent, across two shells ─────────
# Mutate a tracked file from a backgrounded, delayed job while two
# already-split, already-labelled panes -- one fish, one zsh -- sit idle.
# Both prompts refresh live -- no keypress, no Enter, in either pane --
# proving the agent, not either shell's own hooks, is what's driving it.
#
# Both panes, the zsh source, and every tmux styling call happen here,
# hidden, before the beat is revealed: setting pane-border-status/-format
# against the lone pre-split pane didn't reliably survive the layout
# recalc that split-window triggers, so the split has to exist first and
# the options get set once both panes are already in the window.
#
# The caption can't be echoed like the other beats' headers: tmux switches
# to the alternate screen buffer on attach, wiping anything echoed
# beforehand for the whole beat. It rides in tmux's own status bar instead,
# moved to the top and styled to match the other headers, which also keeps
# it pinned while the panes scroll. Each pane additionally gets its own
# label via pane-border-format driven by #{pane_current_command} -- it
# names whatever shell is actually running in that pane, so it can't drift
# out of sync or lie about which pane is which.
#
# `-f /dev/null` on every server-creating call: the fixture $HOME has no
# tmux.conf today, but one leaked XDG_CONFIG_HOME would otherwise let a
# personal config break the recording.
Hide
Type "tmux -f /dev/null -L gpydemo kill-server 2>/dev/null; true"
Enter
Sleep 300ms
Type "tmux -f /dev/null -L gpydemo new-session -d -s tour -c $PWD fish"
Enter
Sleep 500ms
Type "tmux -L gpydemo split-window -h -t tour -c $PWD 'zsh -f'"
Enter
Sleep 800ms

# `zsh -f` skips every rc file, so without this the pane would show zsh's
# own bare prompt, not GPY's -- source the repo's integration directly
# rather than depending on whatever the local ~/.zshrc happens to do.
Type "tmux -L gpydemo send-keys -t tour.1 'source $GPY_REPO_ROOT/zsh/gpy.zsh' Enter"
Enter
Sleep 1s
Type "tmux -L gpydemo send-keys -t tour.1 clear Enter; tmux -L gpydemo select-pane -t tour.0"
Enter
Sleep 500ms
Type "tmux -L gpydemo set -g status-position top; tmux -L gpydemo set -g status-style 'bg=#8aadf4,fg=#24273a,bold'"
Enter
Type "tmux -L gpydemo set -g status-left '  ▸ Live updates via the async agent -- no keypress'; tmux -L gpydemo set -g status-left-length 200"
Enter
Type "tmux -L gpydemo set -g status-right ''; tmux -L gpydemo set -g window-status-format ''; tmux -L gpydemo set -g window-status-current-format ''"
Enter
Type "tmux -L gpydemo set -g pane-border-status top; tmux -L gpydemo set -g pane-border-format ' #{pane_current_command} '"
Enter
Type "tmux -L gpydemo set -g pane-border-style 'fg=#494d64'; tmux -L gpydemo set -g pane-active-border-style 'fg=#8aadf4,bold'"
Enter
Sleep 300ms
Type "clear"
Enter
Show

Type "tmux -L gpydemo attach -t tour"
Enter
Sleep 1.5s

# The write is delayed just long enough to read as "nothing was typed" --
# enough to prove this isn't a synchronous, Enter-triggered refresh -- but
# short enough that the update lands before a viewer's attention does. The
# original 2s delay read as the segment being slow rather than as async.
Type "sh -c 'sleep 0.8; echo tour >> NOTES.md' >/dev/null 2>&1 &"
Enter
Sleep 3s
Type "tmux kill-session -t tour"
Enter
Sleep 800ms
Type "cd .."
Enter
Sleep 1s

# ── Beat 2.5: configurable segments -------------------------------------
# `gpy lang versions` toggles the version pill on the language segment;
# `gpy config set ui.directory.display` swaps how the directory segment
# renders a deep path. Both are plain, scriptable CLI commands rather than
# theme edits, so this beat stays outside the theme/palette import beats
# that follow. Dips into rust-app/src for a path deep enough that
# basename/truncated/full actually look different from each other, then
# returns to the workspace root beat 3 needs (starship-preset.toml and
# tokyo-night.yaml live there, referenced by bare filename).
Hide
Type "clear; echo (set_color -o 24273a -b 8aadf4)(string pad -r -w $COLUMNS '  ▸ Configurable segments: versions and path style')(set_color normal); echo"
Enter
Sleep 300ms
Show
Sleep 900ms
Type "cd rust-app/src"
Enter
Sleep 1s
Type "gpy lang versions off"
Enter
Sleep 1.5s
Type "gpy lang versions on"
Enter
Sleep 1.5s
Type "gpy config set ui.directory.display truncated"
Enter
Sleep 1.5s
Type "gpy config set ui.directory.display full"
Enter
Sleep 1.5s
Type "gpy config set ui.directory.display basename"
Enter
Sleep 1.5s
Type "cd ../.."
Enter
Sleep 1s

# ── Beat 3: theme system, including a real Starship preset import ──────
Hide
Type "clear; echo (set_color -o 24273a -b 8aadf4)(string pad -r -w $COLUMNS '  ▸ Theme system, with a real Starship preset import')(set_color normal); echo"
Enter
Sleep 300ms
Show
Sleep 900ms
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
Hide
Type "clear; echo (set_color -o 24273a -b 8aadf4)(string pad -r -w $COLUMNS '  ▸ Palette system, with a real base16 import')(set_color normal); echo"
Enter
Sleep 300ms
Show
Sleep 900ms
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
Hide
Type "clear; echo (set_color -o 24273a -b 8aadf4)(string pad -r -w $COLUMNS '  ▸ A clean bill of health')(set_color normal); echo"
Enter
Sleep 300ms
Show
Sleep 900ms
Type "gpy doctor"
Enter
Sleep 2.5s
