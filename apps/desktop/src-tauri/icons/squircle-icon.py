#!/usr/bin/env python3
"""Generate Curio's icon artwork.

Two variants, because the platforms disagree about who owns the tile shape:

  * ``tile`` -> icon.svg. The app tile is baked into the artwork: an 824x824
    continuous-curvature squircle (superellipse, n=5) centred on a 1024 canvas
    with 100px margins, per Apple's pre-Tahoe macOS grid. Windows (.ico) and
    Linux (.png) still want this — the shipped bitmap *is* the final icon, so
    it has to carry its own shape.

  * ``fullbleed`` -> icon-macos.svg. The gradient runs edge to edge: no baked
    shape, no transparent margin. macOS 26 (Tahoe) masks app icons to the
    system squircle itself, so artwork that already contains a tile gets a tile
    masked inside a tile — which renders in the Dock as a nested double-tile
    with a shrunken, off-grid glyph. Going full-bleed lets the system mask do
    its job. Tahoe also composites its own specular highlight, so this variant
    drops the baked sheen rather than fighting it.

    The trade-off: pre-Tahoe macOS never masked icons, so there it renders as a
    hard square. Accepted — Tahoe is the current OS and one .icns cannot be
    native on both.

The glyph is identical in both; only the tile treatment and the scale differ.
``tile`` shrinks the glyph by 824/880 so it sits on the smaller baked tile;
``fullbleed`` keeps it at native scale against the full canvas.

Deterministic: same inputs -> byte-identical file.

Usage: squircle-icon.py [tile|fullbleed] > icon.svg
"""
import math
import sys

CANVAS = 1024
CENTER = CANVAS / 2  # 512
HALF = 412.0         # 824 / 2  -> 100px margins
N = 5.0              # superellipse exponent (Apple-icon squircle)
SAMPLES = 720        # dense enough to be visually a smooth curve


def superellipse_path(cx: float, cy: float, a: float, n: float, samples: int) -> str:
    """A closed superellipse (squircle) path centred at (cx, cy), half-size a."""
    pts = []
    exp = 2.0 / n
    for i in range(samples):
        theta = 2.0 * math.pi * i / samples
        ct, st = math.cos(theta), math.sin(theta)
        x = cx + a * math.copysign(abs(ct) ** exp, ct)
        y = cy + a * math.copysign(abs(st) ** exp, st)
        pts.append((round(x, 2), round(y, 2)))
    head = f"M {pts[0][0]} {pts[0][1]}"
    body = " ".join(f"L {x} {y}" for x, y in pts[1:])
    return f"{head} {body} Z"


variant = sys.argv[1] if len(sys.argv) > 1 else "tile"
if variant not in ("tile", "fullbleed"):
    sys.exit(f"unknown variant {variant!r} (want 'tile' or 'fullbleed')")

# The baked tile is 824 wide, so its glyph shrinks to match; full-bleed art sits
# against the whole 1024 canvas and keeps the glyph at native scale.
SCALE = 824.0 / 880.0 if variant == "tile" else 1.0


def s(v: float) -> float:
    """Scale a scalar length about the centre (radii, stroke widths, dashes)."""
    return round(v * SCALE, 2)


def sp(x: float, y: float) -> tuple:
    """Scale a point about the canvas centre."""
    return (round(CENTER + (x - CENTER) * SCALE, 2), round(CENTER + (y - CENTER) * SCALE, 2))


if variant == "tile":
    tile = superellipse_path(CENTER, CENTER, HALF, N, SAMPLES)
    defs_extra = '''    <radialGradient id="sheen" cx="0.32" cy="0.26" r="0.9">
      <stop offset="0" stop-color="#ffffff" stop-opacity="0.20"/>
      <stop offset="0.5" stop-color="#ffffff" stop-opacity="0.03"/>
      <stop offset="1" stop-color="#ffffff" stop-opacity="0"/>
    </radialGradient>
'''
    tile_markup = f'''  <!-- app tile: an 824x824 continuous-curvature squircle (superellipse, n=5)
       centred on the 1024 canvas with 100px margins. On Windows/Linux the
       bitmap is the final icon, so it carries its own shape. -->
  <path d="{tile}" fill="url(#tile)"/>
  <path d="{tile}" fill="url(#sheen)"/>'''
else:
    defs_extra = ""
    tile_markup = '''  <!-- full-bleed tile: no baked shape, no margin. macOS 26 masks app icons to
       the system squircle and adds its own highlight, so anything we bake in
       here gets masked a second time and reads as a tile inside a tile. -->
  <rect x="0" y="0" width="1024" height="1024" fill="url(#tile)"/>'''

# --- inner artwork ---
# C aperture ring
ring_r = s(292)
ring_sw = s(116)
ring_dash_on = s(1265)
ring_dash_off = s(570)
ring_offset = s(285)

# RSS arcs + dot
a1_start = sp(556, 646)
a1_r = s(104)
a1_end = sp(452, 542)
a2_start = sp(644, 646)
a2_r = s(192)
a2_end = sp(452, 454)
rss_sw = s(52)
dot = sp(452, 646)
dot_r = s(42)

svg = f'''<svg width="1024" height="1024" viewBox="0 0 1024 1024" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Curio">
  <title>Curio</title>
  <defs>
    <linearGradient id="tile" x1="0" y1="0" x2="0" y2="1024" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#4A38C8"/>
      <stop offset="0.55" stop-color="#3A2AA0"/>
      <stop offset="1" stop-color="#271B6E"/>
    </linearGradient>
{defs_extra}  </defs>

{tile_markup}

  <!-- the Curio "C": a bold aperture ring opening to the right, drawn as a
       stroked circle with a dashed gap centred on 3 o'clock -->
  <circle cx="512" cy="512" r="{ring_r}" fill="none" stroke="#F6F1E7" stroke-width="{ring_sw}"
          stroke-linecap="round" stroke-dasharray="{ring_dash_on} {ring_dash_off}" stroke-dashoffset="{ring_offset}"/>

  <!-- the RSS mark tucked inside the C's mouth: dot + two broadcast arcs
       opening to the upper-right (this is a feed reader) -->
  <g fill="none" stroke="#F4B23E" stroke-width="{rss_sw}" stroke-linecap="round">
    <path d="M {a1_start[0]} {a1_start[1]} A {a1_r} {a1_r} 0 0 0 {a1_end[0]} {a1_end[1]}"/>
    <path d="M {a2_start[0]} {a2_start[1]} A {a2_r} {a2_r} 0 0 0 {a2_end[0]} {a2_end[1]}"/>
  </g>
  <circle cx="{dot[0]}" cy="{dot[1]}" r="{dot_r}" fill="#F4B23E"/>
</svg>
'''

sys.stdout.write(svg)
