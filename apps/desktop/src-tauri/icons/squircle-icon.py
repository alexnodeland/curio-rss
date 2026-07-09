#!/usr/bin/env python3
"""Generate Curio's icon.svg to the macOS app-icon grid.

The v0.1 tile was an 880x880 rounded rect with rx=212 circular-arc corners on
a 1024 canvas — ~7% oversized with non-native corners next to Dock neighbours.
Apple's macOS grid wants an 824x824 body (100px margins) with a continuous-
curvature squircle (a superellipse), not a single-radius arc.

This emits:
  * the tile as a superellipse squircle path, half-size 412, centred at 512,
    exponent n=5 (the accepted Apple-icon approximation), sampled densely so
    the curve is smooth at every rasterised size;
  * the inner artwork (the "C" aperture ring + the RSS mark) scaled by
    824/880 = 0.9364 about the canvas centre, so composition is preserved but
    the whole glyph shrinks with the tile.
Deterministic: same inputs -> byte-identical file.
"""
import math

CANVAS = 1024
CENTER = CANVAS / 2  # 512
HALF = 412.0         # 824 / 2  -> 100px margins
N = 5.0              # superellipse exponent (Apple-icon squircle)
SAMPLES = 720        # dense enough to be visually a smooth curve
SCALE = 824.0 / 880.0  # 0.93636...


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


def s(v: float) -> float:
    """Scale a scalar length about the centre (radii, stroke widths, dashes)."""
    return round(v * SCALE, 2)


def sp(x: float, y: float) -> tuple:
    """Scale a point about the canvas centre."""
    return (round(CENTER + (x - CENTER) * SCALE, 2), round(CENTER + (y - CENTER) * SCALE, 2))


tile = superellipse_path(CENTER, CENTER, HALF, N, SAMPLES)

# --- inner artwork, scaled about (512, 512) ---
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
    <radialGradient id="sheen" cx="0.32" cy="0.26" r="0.9">
      <stop offset="0" stop-color="#ffffff" stop-opacity="0.20"/>
      <stop offset="0.5" stop-color="#ffffff" stop-opacity="0.03"/>
      <stop offset="1" stop-color="#ffffff" stop-opacity="0"/>
    </radialGradient>
  </defs>

  <!-- app tile: an 824x824 continuous-curvature squircle (superellipse, n=5)
       centred on the 1024 canvas with 100px margins, matching Apple's macOS
       app-icon grid. Transparent canvas so the corners read as native. -->
  <path d="{tile}" fill="url(#tile)"/>
  <path d="{tile}" fill="url(#sheen)"/>

  <!-- the Curio "C": a bold aperture ring opening to the right, drawn as a
       stroked circle with a dashed gap centred on 3 o'clock (scaled 0.936) -->
  <circle cx="512" cy="512" r="{ring_r}" fill="none" stroke="#F6F1E7" stroke-width="{ring_sw}"
          stroke-linecap="round" stroke-dasharray="{ring_dash_on} {ring_dash_off}" stroke-dashoffset="{ring_offset}"/>

  <!-- the RSS mark tucked inside the C's mouth: dot + two broadcast arcs
       opening to the upper-right (this is a feed reader), scaled 0.936. -->
  <g fill="none" stroke="#F4B23E" stroke-width="{rss_sw}" stroke-linecap="round">
    <path d="M {a1_start[0]} {a1_start[1]} A {a1_r} {a1_r} 0 0 0 {a1_end[0]} {a1_end[1]}"/>
    <path d="M {a2_start[0]} {a2_start[1]} A {a2_r} {a2_r} 0 0 0 {a2_end[0]} {a2_end[1]}"/>
  </g>
  <circle cx="{dot[0]}" cy="{dot[1]}" r="{dot_r}" fill="#F4B23E"/>
</svg>
'''

import sys
sys.stdout.write(svg)
