class Gpy < Formula
  desc "High-performance Fish shell prompt with agent-based architecture"
  homepage "https://github.com/jpease/gpy"
  # TODO(#515): this formula is NOT installable and Homebrew is not an
  # operational channel. Two things are missing and neither can be done before
  # the repository is public and v0.1.0 is tagged (#509, #507):
  #
  #   1. sha256 below is a placeholder. Computing the real digest needs the
  #      tag tarball to exist:  curl -sL <url> | shasum -a 256
  #   2. There is no tap. Distribution is a personal tap, jpease/homebrew-tap,
  #      giving `brew install jpease/tap/gpy`. homebrew-core is a post-launch
  #      goal (it requires 90 forks / 90 watchers / 225 stars and a repo at
  #      least 30 days old) and is explicitly not a launch blocker.
  #
  # Until both are done, no installer, README, or doc may offer Homebrew as an
  # install method -- tests/bash/release_version_claims.test.bash enforces
  # that. The step-by-step procedure is in docs/dev/releasing.md.
  url "https://github.com/jpease/gpy/archive/refs/tags/v0.1.0.tar.gz"
  sha256 "REPLACE_WITH_ACTUAL_SHA256"
  license "GPL-3.0-or-later"
  head "https://github.com/jpease/gpy.git", branch: "main"

  depends_on "rust" => :build
  depends_on "fish"

  def install
    system "cargo", "build", "--release", "--manifest-path", "gpy-agent/Cargo.toml"
    bin.install "gpy-agent/target/release/gpy-agent"
    bin.install "gpy-agent/target/release/gpy"

    # Preserve the fish/{core,segments,functions,conf.d,completions} layout
    # as-is: gpy_init.fish (the Fish integration contract) expects that exact
    # tree under a single "gpy" directory, not a flattened function autoload
    # dir. Users wire it into their Fish config per the caveats below.
    pkgshare.install Dir["fish/*"]

    doc.install "README.md", "SECURITY.md"
  end

  def caveats
    <<~EOS
      GPY has been installed, but Homebrew formulas don't modify your
      dotfiles automatically. To finish Fish integration:

        mkdir -p ~/.config/fish/gpy ~/.config/fish/completions ~/.config/fish/functions
        cp -r #{opt_pkgshare}/core #{opt_pkgshare}/segments #{opt_pkgshare}/functions \\
          #{opt_pkgshare}/conf.d ~/.config/fish/gpy/
        cp #{opt_pkgshare}/completions/gpy-dynamic.fish ~/.config/fish/completions/

        # Fish only autoloads functions from ~/.config/fish/functions, so
        # this needs a symlink there alongside fish_prompt itself.
        ln -sf ~/.config/fish/gpy/functions/fish_prompt.fish ~/.config/fish/functions/fish_prompt.fish

      Then add to ~/.config/fish/config.fish:

        if status is-interactive
            source ~/.config/fish/gpy/conf.d/gpy_init.fish
        end

      Restart Fish, then run 'gpy-agent init' to finish configuration.
      Run 'gpy doctor' any time to check your setup.
    EOS
  end

  test do
    system "#{bin}/gpy-agent", "--help"
    system "#{bin}/gpy-agent", "oneshot", "git", "--cwd", testpath
    system "#{bin}/gpy-agent", "oneshot", "lang", "--cwd", testpath
    system "#{bin}/gpy", "--version"
  end
end
