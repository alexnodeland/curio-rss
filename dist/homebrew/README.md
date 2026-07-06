# Homebrew distribution

Curio's macOS distribution channel is an **unsigned Homebrew cask** in the tap
[`alexnodeland/homebrew-tap`](https://github.com/alexnodeland/homebrew-tap).
Apple Developer enrollment is skipped, so the app is not signed or notarized;
[`Casks/curio.rb`](Casks/curio.rb) documents the Gatekeeper right-click-open
step in its `caveats`.

## Installing (end users)

```sh
brew tap alexnodeland/homebrew-tap
brew install --cask curio
```

Then open once via **right-click → Open** (or `xattr -dr com.apple.quarantine
/Applications/Curio.app`), because the build is unsigned.

## Publishing the cask (maintainer)

The cask here is the **source of truth template**; the tap is a thin mirror.
This repo never pushes to the tap. To publish an update:

1. Cut a GitHub release (see [`docs/release/macos-checklist.md`](../../docs/release/macos-checklist.md))
   that uploads the two stably-named DMGs the cask URL expects:
   - `Curio-arm64.dmg`
   - `Curio-x64.dmg`

   Because the cask uses `version :latest` + `sha256 :no_check`, the download
   URL (`releases/latest/download/Curio-<arch>.dmg`) resolves without editing
   the formula — nothing to bump per release.

2. Copy this template into the tap repo and push it:

   ```sh
   git clone https://github.com/alexnodeland/homebrew-tap
   cp dist/homebrew/Casks/curio.rb homebrew-tap/Casks/curio.rb
   cd homebrew-tap && git add Casks/curio.rb \
     && git commit -m "curio: update cask" && git push
   ```

3. Verify:

   ```sh
   brew update
   brew install --cask curio
   brew audit --cask --online curio    # optional lint
   ```

The formula only ever changes when the URL scheme, app name, or uninstall
footprint changes — routine releases need no cask edit at all.
