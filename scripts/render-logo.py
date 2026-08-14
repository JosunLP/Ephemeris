#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Renders the raster copies of the logo from `assets/logo.svg`.

`assets/logo.svg` is the only drawn file. Everything else — the PNG the README
shows, the favicons the documentation site links, the touch icon iOS uses when
somebody saves the page to their home screen, the two icon containers the
Windows and macOS releases are built with — is produced from it here, so there
is one shape to change rather than nine.

    python3 scripts/render-logo.py

The outputs are committed: the documentation workflow runs `vitepress build`
and nothing else, and neither it nor a reader cloning the repository has
CairoSVG. `--check` re-renders into memory and compares, which is what you want
after editing the SVG to confirm the committed PNGs are still in step.

Needs CairoSVG (`pip install cairosvg`), which is not a dependency of anything
else here; the script is run by hand when the logo changes.
"""

from __future__ import annotations

import argparse
import math
import pathlib
import shutil
import struct
import sys
import zlib

ROOT = pathlib.Path(__file__).resolve().parents[1]
SOURCE = ROOT / "assets" / "logo.svg"

# Where the drawn file is copied verbatim. VitePress serves `docs/public` from
# the site root, so `logo.svg` is reachable as `/Ephemeris/logo.svg`.
COPIES = [ROOT / "docs" / "public" / "logo.svg"]

# Size in pixels, and where it goes. The favicon sizes are the two a browser
# actually asks for; 180 is what iOS wants for a home-screen icon; 512 is the
# one the README and the documentation home page show, at half that in CSS
# pixels so it stays sharp on a HiDPI display.
RENDERS = [
    (512, ROOT / "assets" / "logo.png"),
    (512, ROOT / "docs" / "public" / "logo.png"),
    (180, ROOT / "docs" / "public" / "apple-touch-icon.png"),
    (32, ROOT / "docs" / "public" / "favicon-32.png"),
    (16, ROOT / "docs" / "public" / "favicon-16.png"),
]

# The sizes each icon container carries. Windows draws the icon at 16 in a
# title bar and a tree, 32 and 48 in Explorer's smaller views, 256 in its
# largest; 24 and 64 are the two it scales to often enough to be worth having
# drawn rather than resampled. macOS asks for every power of two from 16 to
# 512 and the retina copy of each, which is what the type codes below pair up.
ICO_SIZES = [16, 24, 32, 48, 64, 128, 256]

# `icns` is a flat sequence of typed chunks, and since 10.7 the modern types
# take a PNG as they are. Two codes can name the same pixel size — `ic08` is
# 256 and `ic13` is 128 at double density, which is also 256 — so a size can
# appear twice; `iconutil` writes both, and Finder picks by the code rather
# than by measuring.
ICNS_TYPES = [
    (b"icp4", 16),
    (b"icp5", 32),
    (b"ic11", 32),  # 16 @2x
    (b"ic12", 64),  # 32 @2x
    (b"ic07", 128),
    (b"ic13", 256),  # 128 @2x
    (b"ic08", 256),
    (b"ic14", 512),  # 256 @2x
    (b"ic09", 512),
    (b"ic10", 1024),  # 512 @2x
]


def squircle(exponent: float = 5.0, half: float = 127.5, centre: float = 128.0,
             segments: int = 32) -> str:
    """The `d` of the panel outline: |x|^n + |y|^n = 1, as cubic curves.

    Sampled by arc length rather than by parameter — an even parameter step
    crowds the corners and leaves the flat edges with too few points to fit,
    which shows as a visible wobble along them. The fit itself is Catmull-Rom
    converted to Bézier, which overshoots by around a fifth of a pixel, so the
    shape is drawn a half-pixel inside the 256 box and stays within it.
    """
    dense = 4000
    points = []
    for i in range(dense):
        t = 2 * math.pi * i / dense
        cos_t, sin_t = math.cos(t), math.sin(t)
        points.append((
            centre + half * math.copysign(abs(cos_t) ** (2 / exponent), cos_t),
            centre + half * math.copysign(abs(sin_t) ** (2 / exponent), sin_t),
        ))

    lengths = [0.0]
    for i in range(1, dense + 1):
        a, b = points[i - 1], points[i % dense]
        lengths.append(lengths[-1] + math.hypot(b[0] - a[0], b[1] - a[1]))

    picked, cursor = [], 0
    for k in range(segments):
        target = lengths[-1] * k / segments
        while lengths[cursor + 1] < target:
            cursor += 1
        picked.append(points[cursor])

    count = len(picked)
    out = [f"M{picked[0][0]:.1f} {picked[0][1]:.1f}"]
    for i in range(count):
        p0 = picked[(i - 1) % count]
        p1, p2 = picked[i], picked[(i + 1) % count]
        p3 = picked[(i + 2) % count]
        c1 = (p1[0] + (p2[0] - p0[0]) / 6, p1[1] + (p2[1] - p0[1]) / 6)
        c2 = (p2[0] - (p3[0] - p1[0]) / 6, p2[1] - (p3[1] - p1[1]) / 6)
        out.append(
            f"C{c1[0]:.1f} {c1[1]:.1f} {c2[0]:.1f} {c2[1]:.1f}"
            f" {p2[0]:.1f} {p2[1]:.1f}"
        )
    return "".join(out) + "Z"


def render(size: int) -> bytes:
    """The SVG at `size` × `size` pixels, as PNG bytes."""
    import cairosvg

    return cairosvg.svg2png(
        url=str(SOURCE), output_width=size, output_height=size
    )


def pixels(png: bytes, size: int) -> bytes:
    """The rows of a PNG this script rendered, as straight RGBA, top down.

    Only the shape CairoSVG writes is handled: eight bits a channel, RGBA,
    no interlacing, one filter byte in front of every row. Anything else is a
    changed renderer rather than a case to grow support for, so it raises.
    """
    if png[12:16] != b"IHDR":
        raise ValueError("not a PNG")
    depth, colour = png[24], png[25]
    if (depth, colour, png[28]) != (8, 6, 0):
        raise ValueError(
            f"expected 8-bit RGBA, non-interlaced; got depth {depth},"
            f" colour type {colour}, interlace {png[28]}"
        )

    compressed = bytearray()
    at = 8
    while at < len(png):
        length = int.from_bytes(png[at:at + 4], "big")
        if png[at + 4:at + 8] == b"IDAT":
            compressed += png[at + 8:at + 8 + length]
        at += 12 + length
    raw = zlib.decompress(bytes(compressed))

    stride = size * 4
    out = bytearray(size * stride)
    prior = bytes(stride)
    at = 0
    for y in range(size):
        kind, at = raw[at], at + 1
        line = bytearray(raw[at:at + stride])
        at += stride
        # Sub, Up, Average and Paeth, from the PNG specification. The byte
        # four to the left is the same channel of the previous pixel, and
        # before the first pixel it counts as zero.
        if kind == 1:
            for i in range(4, stride):
                line[i] = (line[i] + line[i - 4]) & 0xFF
        elif kind == 2:
            for i in range(stride):
                line[i] = (line[i] + prior[i]) & 0xFF
        elif kind == 3:
            for i in range(stride):
                left = line[i - 4] if i >= 4 else 0
                line[i] = (line[i] + ((left + prior[i]) >> 1)) & 0xFF
        elif kind == 4:
            for i in range(stride):
                a = line[i - 4] if i >= 4 else 0
                b = prior[i]
                c = prior[i - 4] if i >= 4 else 0
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                line[i] = (line[i] + (
                    a if pa <= pb and pa <= pc else b if pb <= pc else c
                )) & 0xFF
        elif kind != 0:
            raise ValueError(f"unexpected PNG filter {kind}")
        out[y * stride:(y + 1) * stride] = line
        prior = bytes(line)
    return bytes(out)


def dib(png: bytes, size: int) -> bytes:
    """One icon image in the form `.ico` has carried since Windows 3:

    a `BITMAPINFOHEADER` whose height is doubled because a colour image and a
    mask are stacked in it, BGRA rows from the bottom up, then the mask. The
    mask is left at zero throughout — it says "draw this pixel", and the alpha
    channel is what actually decides how much of it to draw. Windows has read
    a PNG here since Vista, but only at 256; below that the shell still expects
    this, and the icon appears blank in some views if it does not get it.
    """
    rgba = pixels(png, size)
    header = struct.pack(
        "<IiiHHIIiiII", 40, size, size * 2, 1, 32, 0, size * size * 4,
        0, 0, 0, 0,
    )
    rows = bytearray()
    for y in range(size - 1, -1, -1):
        row = rgba[y * size * 4:(y + 1) * size * 4]
        for x in range(0, len(row), 4):
            r, g, b, a = row[x:x + 4]
            rows += bytes((b, g, r, a))
    # Every mask row is padded out to a multiple of four bytes, as every row
    # in a DIB is.
    return bytes(header + rows + bytes((size + 31) // 32 * 4 * size))


def ico(rendered: dict[int, bytes]) -> bytes:
    """The Windows icon: a directory of images, then the images."""
    images = [
        # 256 goes in as the PNG it already is. It is the one size where the
        # uncompressed form is worth avoiding — a quarter of a megabyte, and
        # the resource is linked into every copy of the binary.
        rendered[size] if size == 256 else dib(rendered[size], size)
        for size in ICO_SIZES
    ]
    offset = 6 + 16 * len(images)
    directory = bytearray(struct.pack("<HHH", 0, 1, len(images)))
    for size, image in zip(ICO_SIZES, images):
        directory += struct.pack(
            # A side of 256 is written as zero: the field is one byte, and
            # 256 is the one size that does not fit in it.
            "<BBBBHHII", size % 256, size % 256, 0, 0, 1, 32,
            len(image), offset,
        )
        offset += len(image)
    return bytes(directory) + b"".join(images)


def icns(rendered: dict[int, bytes]) -> bytes:
    """The macOS icon: `icns`, a total length, then length-prefixed chunks."""
    body = b"".join(
        code + struct.pack(">I", len(rendered[size]) + 8) + rendered[size]
        for code, size in ICNS_TYPES
    )
    return b"icns" + struct.pack(">I", len(body) + 8) + body


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--check",
        action="store_true",
        help="verify the committed files match the SVG instead of writing them",
    )
    parser.add_argument(
        "--squircle",
        action="store_true",
        help="print the panel outline for pasting into the SVG, and stop",
    )
    args = parser.parse_args()

    if args.squircle:
        print(squircle())
        return 0

    try:
        import cairosvg  # noqa: F401
    except ImportError:
        print(
            "cairosvg is not installed; `pip install cairosvg` to run this.",
            file=sys.stderr,
        )
        return 2

    source = SOURCE.read_bytes()
    stale: list[pathlib.Path] = []

    for destination in COPIES:
        if args.check:
            if not destination.exists() or destination.read_bytes() != source:
                stale.append(destination)
        else:
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(SOURCE, destination)

    for size, destination in RENDERS:
        png = render(size)
        if args.check:
            if not destination.exists() or destination.read_bytes() != png:
                stale.append(destination)
        else:
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(png)

    # Both containers are built from the same renders, and the two lists
    # overlap, so each size is drawn once and handed to both.
    rendered = {
        size: render(size)
        for size in sorted({*ICO_SIZES, *(s for _, s in ICNS_TYPES)})
    }
    packed = [
        (ROOT / "assets" / "logo.ico", ico(rendered)),
        (ROOT / "assets" / "logo.icns", icns(rendered)),
    ]
    for destination, blob in packed:
        if args.check:
            if not destination.exists() or destination.read_bytes() != blob:
                stale.append(destination)
        else:
            destination.write_bytes(blob)

    if args.check:
        if stale:
            print("out of step with assets/logo.svg:", file=sys.stderr)
            for path in stale:
                print(f"  {path.relative_to(ROOT)}", file=sys.stderr)
            print("run: python3 scripts/render-logo.py", file=sys.stderr)
            return 1
        count = len(COPIES) + len(RENDERS) + len(packed)
        print(f"{count} files in step with the SVG")
        return 0

    for _, destination in RENDERS:
        print(f"wrote {destination.relative_to(ROOT)}")
    for destination, _ in packed:
        print(f"wrote {destination.relative_to(ROOT)}")
    for destination in COPIES:
        print(f"copied {destination.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
