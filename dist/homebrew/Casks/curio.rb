# Homebrew cask for Curio — the unsigned macOS distribution channel.
#
# Distribution decision (docs/design/decisions.md): Apple Developer
# enrollment is skipped, so the app is neither signed nor notarized; Windows
# is deferred. This cask ships the unsigned .app from the GitHub release and
# documents the Gatekeeper right-click-open path in `caveats`.
#
# This file is a TEMPLATE that lives with the app repo. Publishing it means
# copying it into the tap repo `alexnodeland/homebrew-tap` under `Casks/` —
# see ../README.md. Do NOT expect `brew` to find it here.
cask "curio" do
  arch arm: "arm64", intel: "x64"

  # Unsigned + always-latest: the tap always installs whatever the latest
  # GitHub release published, so there is no per-release sha256 to bump.
  version :latest
  sha256 :no_check

  url "https://github.com/alexnodeland/curio-rss/releases/latest/download/Curio-#{arch}.dmg",
      verified: "github.com/alexnodeland/curio-rss/"
  name "Curio"
  desc "Local-first RSS reader"
  homepage "https://github.com/alexnodeland/curio-rss"

  app "Curio.app"

  caveats <<~EOS
    Curio is NOT signed or notarized (Apple Developer enrollment is skipped),
    so macOS Gatekeeper will refuse to open it on first launch. To run it:

      1. In Finder, open /Applications and right-click (Control-click) Curio.app
      2. Choose "Open", then "Open" again in the warning dialog

    or clear the quarantine flag from a terminal:

      xattr -dr com.apple.quarantine "/Applications/Curio.app"

    You only need to do this once per install.
  EOS

  # Full uninstall footprint (matches the app's platform paths:
  # io.github.alexnodeland.curio data dir + the curio cache dir).
  zap trash: [
    "~/Library/Application Support/io.github.alexnodeland.curio",
    "~/Library/Caches/curio",
    "~/Library/Preferences/io.github.alexnodeland.curio.plist",
  ]
end
