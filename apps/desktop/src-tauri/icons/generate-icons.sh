#!/usr/bin/env bash
# Render the committed Curio glyph (icon.svg) into the full bundle icon set:
# the PNG sizes Tauri references, plus icon.icns (macOS) and icon.ico
# (Windows). Committed and deterministic — re-run whenever icon.svg changes,
# then commit the regenerated bitmaps.
#
# Toolchain (all present on the macOS build host):
#   * rsvg-convert  — SVG -> high-res PNG master (brew install librsvg)
#   * tauri icon    — PNG master -> the platform icon set (.png/.icns/.ico)
#                     (the @tauri-apps/cli devDependency; no extra install)
#
# Every emitted file must stay under the repo's 1 MB blob guard; the script
# asserts it before finishing.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
desktop_dir="$(cd "$here/../.." && pwd)"      # apps/desktop
svg="$here/icon.svg"
master="$(mktemp -t curio-icon-master).png"
trap 'rm -f "$master"' EXIT

command -v rsvg-convert >/dev/null || { echo "need rsvg-convert (brew install librsvg)"; exit 1; }

echo "rendering $svg -> 1024px master"
rsvg-convert -w 1024 -h 1024 "$svg" -o "$master"

echo "generating the icon set via tauri icon"
( cd "$desktop_dir" && npm exec --yes -- tauri icon "$master" --output src-tauri/icons )

echo "enforcing the 1 MB blob guard on every emitted icon"
limit=1048576
status=0
for f in "$here"/*.png "$here"/icon.icns "$here"/icon.ico; do
    [ -e "$f" ] || continue
    size=$(wc -c < "$f")
    if [ "$size" -gt "$limit" ]; then
        echo "  OVERSIZE: $(basename "$f") is $size bytes (> 1 MB)"
        status=1
    fi
done
[ "$status" -eq 0 ] && echo "icons OK — all under 1 MB"
exit "$status"
