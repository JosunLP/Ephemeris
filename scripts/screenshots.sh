#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Takes the screenshots the Flathub listing shows, on a virtual X server.
#
#     scripts/screenshots.sh
#
# writes docs/public/screenshots/desktop-light.png and desktop-dark.png, which
# is what packaging/linux/io.github.josunlp.ephemeris.metainfo.xml points at.
# Run it whenever the widget's look changes; a store listing showing a version
# nobody can install any more is worse than none.
#
# Needs a Linux machine with:
#
#     sudo apt-get install --no-install-recommends xvfb feh imagemagick \
#         libcairo2 libpango-1.0-0 libpangocairo-1.0-0 libx11-6 libglib2.0-0
#
# It does not need a desktop: `Xvfb` supplies the display, `EPHEMERIS_DEMO=1`
# supplies the agenda, and no account is touched. The settings file is written
# for the run, so it also does not matter what the machine's own widget is
# configured to look like — but it is the real one, in
# `$XDG_CONFIG_HOME`/`~/.config`, so this is not a script to run while using
# the widget for real.
#
# Two things here are workarounds for the runner rather than properties of the
# widget, and both are why the shots are composed rather than simply captured:
#
#  1. A virtual server has no compositing manager, so the widget draws opaque
#     and the margin it leaves for its drop shadow comes out as a black box.
#     The panel is therefore cut out at exactly the rectangle it occupies, its
#     corners are cut back to the curve it drew, and the shadow a compositor
#     would have produced is drawn underneath.
#  2. The widget leaves the root window black, so the wallpaper is set *after*
#     it has started. Painted before, it is gone by the time of the capture.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out="$root/docs/public/screenshots"

screen_w=1600
screen_h=900

# `xvfb-run` rather than starting `Xvfb` here: an X server signals its parent
# with SIGUSR1 once it is listening, and a shell that has not arranged to
# ignore that signal is killed by it. The wrapper deals with that, picks a free
# display number and takes the server down again afterwards.
if [ -z "${EPHEMERIS_SHOT_INSIDE:-}" ]; then
    command -v xvfb-run > /dev/null || {
        echo "xvfb-run is not installed — see the header of this script." >&2
        exit 2
    }
    exec xvfb-run -a --server-args="-screen 0 ${screen_w}x${screen_h}x24" \
        env EPHEMERIS_SHOT_INSIDE=1 "$0" "$@"
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"; pkill -x ephemeris 2>/dev/null || true' EXIT

binary="${EPHEMERIS_BINARY:-$root/target/debug/ephemeris}"
# The panel, in the screen's pixels. The window is placed by the settings
# below; `Metrics::with_shadow` leaves 14 device-independent pixels around the
# panel for the shadow, and the run is at scale 1.
panel_x=1144
panel_y=110
panel_w=380
panel_h=660
radius=12

for tool in feh convert import python3; do
    command -v "$tool" > /dev/null || {
        echo "$tool is not installed — see the header of this script." >&2
        exit 2
    }
done

[ -x "$binary" ] || cargo build --locked --manifest-path "$root/Cargo.toml"

# The wallpaper. Two flat gradients rather than a photograph: something has to
# be behind a widget that sits on the desktop, and anything with detail in it
# competes with the thing being shown.
python3 - "$work" <<'PY'
import struct, sys, zlib

def png(width, height, rows):
    def chunk(kind, data):
        return (struct.pack(">I", len(data)) + kind + data
                + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF))
    raw = b"".join(b"\x00" + row for row in rows)
    return (b"\x89PNG\r\n\x1a\n"
            + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw, 9))
            + chunk(b"IEND", b""))

def gradient(width, height, start, end):
    rows = []
    for y in range(height):
        row = bytearray()
        for x in range(width):
            t = (x / width + y / height) / 2
            row += bytes(round(a + (b - a) * t) for a, b in zip(start, end))
        rows.append(bytes(row))
    return rows

WIDTH, HEIGHT = 1600, 900
for name, (start, end) in {
    "dark": ((26, 30, 46), (58, 44, 74)),
    "light": ((222, 231, 244), (198, 206, 228)),
}.items():
    with open(f"{sys.argv[1]}/wall-{name}.png", "wb") as handle:
        handle.write(png(WIDTH, HEIGHT, gradient(WIDTH, HEIGHT, start, end)))
PY

config="${XDG_CONFIG_HOME:-$HOME/.config}/ephemeris"
mkdir -p "$config" "$out"

shoot() {
    local theme="$1"

    cat > "$config/config.json" <<JSON
{
  "x": $((panel_x - 14)),
  "y": $((panel_y - 14)),
  "width": $panel_w.0,
  "height": $panel_h.0,
  "locked": false,
  "sync_minutes": 30,
  "accounts": [],
  "calendar_ids": [],
  "tasklist_ids": [],
  "show_undated_tasks": false,
  "show_past_events": true,
  "hide_declined": true,
  "opacity": 1.0,
  "backdrop": "none",
  "scale": 1.0,
  "language": "en-US",
  "theme": "$theme",
  "accent": "system",
  "peek_hotkey": "Ctrl+Alt+Shift+K",
  "peek_seconds": 5,
  "undo_seconds": 4
}
JSON

    pkill -x ephemeris 2>/dev/null || true
    sleep 1
    EPHEMERIS_DEMO=1 \
        LC_ALL=en_US.UTF-8 LC_TIME=en_US.UTF-8 LANG=en_US.UTF-8 \
        "$binary" > "$work/widget-$theme.log" 2>&1 &
    sleep 7

    feh --bg-fill "$work/wall-$theme.png"
    sleep 2
    import -window root "$work/capture-$theme.png"
    pkill -x ephemeris 2>/dev/null || true

    convert "$work/capture-$theme.png" \
        -crop "${panel_w}x${panel_h}+${panel_x}+${panel_y}" +repage \
        "$work/panel-$theme.png"
    convert "$work/panel-$theme.png" \
        \( +clone -alpha transparent -background none -fill white \
           -draw "roundrectangle 0,0,$((panel_w - 1)),$((panel_h - 1)),$radius,$radius" \) \
        -alpha set -compose DstIn -composite "$work/rounded-$theme.png"

    local pad=40
    convert -size "$((panel_w + 2 * pad))x$((panel_h + 2 * pad))" xc:none \
        -fill 'rgba(0,0,0,0.55)' \
        -draw "roundrectangle $pad,$pad,$((pad + panel_w - 1)),$((pad + panel_h - 1)),$radius,$radius" \
        -blur 0x14 "$work/shadow.png"

    convert "$work/wall-$theme.png" \
        "$work/shadow.png" -geometry "+$((panel_x - pad))+$((panel_y - pad + 8))" -composite \
        "$work/rounded-$theme.png" -geometry "+${panel_x}+${panel_y}" -composite \
        -strip "$out/desktop-$theme.png"

    echo "wrote docs/public/screenshots/desktop-$theme.png"
}

shoot light
shoot dark
