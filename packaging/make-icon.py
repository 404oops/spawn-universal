#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Draw the application icon.

The mark is a keycap with the actuation point marked beneath it: a key and
the depth it triggers at, which is what this application is for. It is drawn
here rather than stored as a binary so the source of truth is readable, and
so every size is rendered rather than resampled.

Deliberately not the manufacturer's logo. That artwork is theirs, this
project is GPL, and an icon carrying their mark would imply their software.

    python3 packaging/make-icon.py
"""

import struct
import zlib
from pathlib import Path

OUT = Path(__file__).parent / "icons"
SIZES = [16, 32, 64, 128, 256, 512, 1024]
SS = 4  # supersampling factor, for antialiased edges

# The interface's own palette: a violet hue lit from above.
TILE_TOP = (0x22, 0x28, 0x3A)
TILE_BOTTOM = (0x0E, 0x11, 0x18)
CAP_TOP = (0x4A, 0x56, 0x84)
CAP_BOTTOM = (0x2E, 0x37, 0x5A)
CAP_EDGE = (0x6B, 0x79, 0xAD)
ACCENT = (0xA9, 0x9B, 0xF5)


def rounded_rect(px, py, x, y, w, h, r):
    """Signed distance from a rounded rectangle: negative inside."""
    cx, cy = abs(px - x) - (w / 2 - r), abs(py - y) - (h / 2 - r)
    outside = ((max(cx, 0.0)) ** 2 + (max(cy, 0.0)) ** 2) ** 0.5
    return outside + min(max(cx, cy), 0.0) - r


def mix(a, b, t):
    t = max(0.0, min(1.0, t))
    return tuple(round(a[i] + (b[i] - a[i]) * t) for i in range(3))


def over(dst, src, alpha):
    return tuple(round(dst[i] + (src[i] - dst[i]) * alpha) for i in range(3))


def sample(px, py, n):
    """Colour and alpha at one point, in a unit square scaled to `n`."""
    u, v = px / n, py / n

    # The tile itself, lit from the top.
    tile = rounded_rect(u, v, 0.5, 0.5, 1.0, 1.0, 0.225)
    if tile > 0:
        return (0, 0, 0), 0.0
    color = mix(TILE_TOP, TILE_BOTTOM, v)

    # The keycap, sitting a little above centre to leave room beneath it.
    cap = rounded_rect(u, v, 0.5, 0.42, 0.46, 0.44, 0.085)
    if cap < 0:
        # Its own gradient, and a lit top edge.
        local = (v - 0.20) / 0.44
        color = mix(CAP_TOP, CAP_BOTTOM, local)
        edge = cap + 0.014
        if edge > 0 and v < 0.42:
            color = CAP_EDGE

    # The actuation point: where the key triggers, under the cap.
    bar = rounded_rect(u, v, 0.5, 0.78, 0.46, 0.075, 0.0375)
    if bar < 0:
        color = ACCENT

    return color, 1.0


def render(n):
    big = n * SS
    rows = []
    for y in range(n):
        row = bytearray()
        for x in range(n):
            r = g = b = a = 0.0
            for sy in range(SS):
                for sx in range(SS):
                    px = x * SS + sx + 0.5
                    py = y * SS + sy + 0.5
                    c, alpha = sample(px, py, big)
                    r += c[0] * alpha
                    g += c[1] * alpha
                    b += c[2] * alpha
                    a += alpha
            count = SS * SS
            if a > 0:
                row += bytes(
                    (round(r / a), round(g / a), round(b / a), round(255 * a / count))
                )
            else:
                row += b"\0\0\0\0"
        rows.append(bytes(row))
    return rows


def write_png(path, n, rows):
    raw = b"".join(b"\0" + row for row in rows)

    def chunk(tag, data):
        return (
            struct.pack(">I", len(data))
            + tag
            + data
            + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
        )

    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", n, n, 8, 6, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(raw, 9))
    png += chunk(b"IEND", b"")
    path.write_bytes(png)


def write_ico(path, pngs):
    """A Windows icon, as PNG entries: supported since Vista and far smaller
    than the uncompressed bitmaps the old format wants."""
    entries, blobs, offset = [], [], 6 + 16 * len(pngs)
    for size, data in pngs:
        entries.append(
            struct.pack(
                "<BBBBHHII",
                0 if size >= 256 else size,
                0 if size >= 256 else size,
                0,
                0,
                1,
                32,
                len(data),
                offset,
            )
        )
        blobs.append(data)
        offset += len(data)
    path.write_bytes(
        struct.pack("<HHH", 0, 1, len(pngs)) + b"".join(entries) + b"".join(blobs)
    )


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    for n in SIZES:
        write_png(OUT / f"icon-{n}.png", n, render(n))
        print(f"icon-{n}.png")

    # Windows wants the small sizes in one file.
    write_ico(
        OUT / "icon.ico",
        [(n, (OUT / f"icon-{n}.png").read_bytes()) for n in (16, 32, 64, 128, 256)],
    )
    print("icon.ico")

    (OUT / "icon.png").write_bytes((OUT / "icon-512.png").read_bytes())
    print("icon.png (512, for Linux)")


if __name__ == "__main__":
    main()
