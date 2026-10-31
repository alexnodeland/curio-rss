#!/usr/bin/env bash
# MSRV gate: every workspace crate must build with the exact toolchain its
# Cargo.toml `rust-version` declares. The versions are read from
# `cargo metadata`, never restated here, so this check matches Cargo.toml by
# construction — bump `rust-version` and the next run checks the new number.
#
# Crates are grouped by declared version: the engine crates (curio-core,
# curio-types, curio-cli, ...) inherit the workspace `rust-version`, while
# curio-desktop declares its own, higher one (tauri/specta need a newer
# compiler than the headless core does). Each group gets one
# `cargo +<version> check --locked --all-targets`, so tests and dev-deps are
# held to the floor too.
#
# Installs the toolchains it needs through rustup (minimal profile). CI job
# `msrv` runs this; locally it is `just msrv` (opt-in — it downloads one
# toolchain per distinct version, so it is not part of `just ci`). On Linux
# the curio-desktop group needs the webview system packages (see ci.yml).
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

# "<rust_version> <package>" for every workspace member.
pairs="$(cargo metadata --no-deps --format-version 1 --locked \
    | jq -r '.packages[] | "\(.rust_version // "") \(.name)"')"

if grep -qE '^ ' <<<"$pairs"; then
    echo "MSRV: workspace crates without a rust-version:" >&2
    grep -E '^ ' <<<"$pairs" >&2
    exit 1
fi

for version in $(cut -d' ' -f1 <<<"$pairs" | sort -uV); do
    names=()
    pkgs=()
    while read -r v name; do
        if [ "$v" = "$version" ]; then
            names+=("$name")
            pkgs+=(-p "$name")
        fi
    done <<<"$pairs"
    # `rust-version = "1.88"` means 1.88.0: pin the .0 release, not the
    # latest 1.88.x patch rustup would otherwise resolve "1.88" to.
    toolchain="$version"
    [[ "$toolchain" =~ ^[0-9]+\.[0-9]+$ ]] && toolchain="$toolchain.0"
    echo "── MSRV $toolchain: ${names[*]} ──"
    rustup toolchain install "$toolchain" --profile minimal --no-self-update
    cargo "+$toolchain" check --locked --all-targets "${pkgs[@]}"
done

echo "msrv: OK — every crate builds on its declared rust-version"
