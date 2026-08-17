<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Assets

`logo.svg` is the only drawn file. Everything else in here and in
`docs/public/` is rendered from it by `scripts/render-logo.py`; edit the SVG,
run the script, and commit both.

```bash
pip install cairosvg
python3 scripts/render-logo.py
python3 scripts/render-logo.py --check    # re-render and compare, don't write
python3 scripts/render-logo.py --verify   # no renderer needed; what CI runs
```

The outputs are committed because the documentation workflow runs
`vitepress build` and nothing else — there is no CairoSVG on that runner, and a
reader cloning the repository should see the README with its logo.

Committed output can go stale, so CI checks it — with `--verify` rather than
`--check`. `--check` re-renders, and PNG bytes depend on the CairoSVG version
and the Cairo underneath it, so a runner that installs either fresh would
eventually fail on an upgrade instead of on a stale file. `--verify` needs
nothing but the standard library and asks only what cannot go flaky: that
`docs/public/logo.svg` is byte-identical to the drawn file, and that every
generated file is present and structurally what the script writes — each PNG's
`IHDR`, the `.ico` directory, the `.icns` chunk lengths. Use `--check` yourself
after editing the SVG; it is the one that compares the actual pixels.

| File                             | Size    | Used by                        |
| -------------------------------- | ------- | ------------------------------ |
| `assets/logo.svg`                | vector  | the source; also the favicon, the Flatpak icon and the Linux tarball |
| `assets/logo.png`                | 512 px  | the README                     |
| `assets/logo.ico`                | 16–256  | the Windows executable, through `build.rs` |
| `assets/logo.icns`               | 16–1024 | the macOS bundle, as `Contents/Resources/Ephemeris.icns` |
| `docs/public/logo.svg`           | vector  | header logo, hero, SVG favicon |
| `docs/public/logo.png`           | 512 px  | `og:image` for link previews   |
| `docs/public/apple-touch-icon.png` | 180 px | iOS home screen                |
| `docs/public/favicon-32.png`     | 32 px   | browsers without SVG favicons  |
| `docs/public/favicon-16.png`     | 16 px   | the same, in a tab             |

`logo.ico` and `logo.icns` are containers rather than images: several sizes
each, written by hand in the script because neither format needs more than a
header in front of the renders. They are committed for the same reason the
PNGs are, and rather more firmly — the Windows build reads `logo.ico` at
compile time and fails without it, and the macOS runner copies `logo.icns`
into the bundle rather than calling `iconutil`.

The Flatpak installs the SVG itself, as
`hicolor/scalable/apps/io.github.josunlp.ephemeris.svg`; the name matches the
application ID, which is what `appstreamcli compose` looks for.

## The mark

The day rail — the bar across the top of the widget that shows the whole day
with one coloured segment per event — closed into the ring a day actually is.
The gaps are the time that is still yours, the amber segment is the event
running right now, and the hand points at it.

The ring is regular: four segments of 72 degrees and four gaps of 18, the gaps
centred on twelve, three, six and nine o'clock, so each arc is the one before
it turned a quarter and the ring reads the same whichever way up you hold it.
The hand and the amber are the only things off-centre, which is what keeps it
a clock rather than a loading spinner.

Two colours carry it: the blue of the panel (`#7CC4FF` → `#3A46D8`) and the
amber the widget uses for what is due (`#FFDC8C` → `#FFA92B`).

The panel is a superellipse, not a rounded rectangle — the curvature runs into
the straight edge instead of meeting it at a corner. `--squircle` prints the
path if you want to change the exponent.

Everything is gradients and plain shapes: no filters, no text, no external
font. That is a constraint rather than a preference — CairoSVG renders the
committed PNGs and drops most filters, so anything a filter did would be in
the SVG and missing from every raster copy of it. The glow that would sit
under the running event is the one thing this costs.
