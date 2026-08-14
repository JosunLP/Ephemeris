# Contributing

Thanks for taking the time. This document covers what you need to get a change
merged.

## Getting set up

```bash
git clone https://github.com/JosunLP/Ephemeris
cd Ephemeris
cargo test --workspace
cargo run --release
```

To see the interface without connecting a calendar account:

```bash
EPHEMERIS_DEMO=1 cargo run --release      # PowerShell: $env:EPHEMERIS_DEMO=1
```

Rust 1.90 or newer. On Windows you also need the MSVC build tools; on macOS and
Linux nothing beyond a C toolchain, because Xlib, Cairo and Pango are opened at
run time rather than linked.

**Working on the macOS front end without a Mac.** `scripts/check-macos.sh`
type-checks `src/unix/mac` from a Linux or WSL machine and runs the same lints
continuous integration denies warnings for. `cargo check` never links, so the
AppKit symbols do not have to exist. It cannot tell you whether a selector is
spelled correctly or whether anything appears on screen — see
[Porting](/development/porting) for exactly what it does and does not prove.

## The one architectural rule

The project is a workspace with a deliberate split:

| Crate            | Contains                                               | May call the operating system |
| ---------------- | ------------------------------------------------------ | ----------------------------- |
| `ephemeris-core` | model, calendar back ends, sync, localisation, palette | **no**                        |
| `ephemeris`      | the front ends: `src/win`, `src/unix`                  | yes                           |

`ephemeris-core` must compile and pass its tests on Windows, macOS and Linux.
Anything it needs from the system goes through a trait in `core/src/host.rs`,
supplied by the front end at start-up. Continuous integration checks this on
all three systems, so a stray platform call fails the build rather than
surfacing months later on somebody else's machine.

If you find yourself reaching for a platform API inside the core, that is the
signal to add a trait method instead.

The front end is split the same way. `src/main.rs` calls four functions —
`install_host`, `acquire_single_instance`, `run`, `fatal` — and a `#[cfg]`
decides which module supplies them, so platform code lives in `src/win` or
`src/unix` and nowhere else. Adding a platform is adding a directory rather
than threading conditionals through the program. What each new front end owes
the core, and which decisions have already been taken, is in
[docs/development/porting.md](docs/development/porting.md).

## Before you open a pull request

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

All three run in CI and all three have to pass.

Two generated files are checked there as well, and both need nothing but
Python, so run them too if you touched what they are generated from:

```bash
python3 packaging/linux/cargo-sources.py --check   # after Cargo.lock changes
python3 scripts/render-logo.py --verify            # after assets/logo.svg changes
```

## What good looks like here

- **Comments explain why, not what.** The code already says what it does. The
  valuable comment is the one recording the trap you fell into — see
  `model::parse_task_due` for the shape of it.
- **A bug fix comes with a test that fails without it.** Especially for date
  handling, where the failure mode is "off by one day in half the world".
- **English, in code and in documentation.**
- **Say what you could not verify.** A pull request that notes "this path was
  never run against a real server" is more useful than one that implies it was.

## Adding an interface language

One `Catalog` constant in `crates/core/src/i18n.rs`, one arm in `catalog_for`,
one entry in `CATALOGS`. Because `Catalog` is a struct of named fields rather
than a key/value map, a forgotten string is a compile error rather than a blank
label at runtime.

What a translation pull request is expected to contain:

- **Text written by someone who speaks the language.** Machine translation
  produces strings that are grammatical and wrong in tone — a desktop widget
  saying the equivalent of "Please to synchronise now" is worse than the
  English fallback, because it looks like the program does not know what it is
  saying. Say in the pull request who checked it.
- **The right plural rule.** Pick the [`Plural`] variant your language actually
  uses and fill in its forms; the variant *is* the rule, so the two cannot
  drift apart. Russian, Ukrainian, Polish and Czech each get their own variant
  because their boundaries genuinely differ — Polish puts 21 in the `many`
  form where Russian puts it in `one`. Arabic has six categories, Hebrew has a
  dual, and Chinese, Japanese, Korean and Turkish have exactly one form.
- **Strings that fit.** The panel is about 380 device-independent pixels wide
  and several labels sit in a fixed column. `cargo test -p ephemeris-core`
  enforces a width budget for those and flags anything that ran away from its
  English original; a failure means "find a shorter word", not "raise the
  budget".
- **A screenshot, if you can.** Run `EPHEMERIS_DEMO=1 cargo run --release`
  with `"language"` set to your tag. For a script Segoe UI does not cover —
  CJK, Thai, Devanagari — this is the only way to see whether font fallback
  picked something sensible.

Regional variants normally share one catalogue: `de-AT` gets `DE`, and dates,
times and the calendar system still come from the operating system. Split them
only where the text genuinely differs, as `pt` and `pt-BR` do. Chinese is
matched by script rather than by region, so `zh-TW` and `zh-HK` reach the
traditional catalogue.

[`Plural`]: https://github.com/JosunLP/Ephemeris/blob/main/crates/core/src/i18n.rs

## Reporting a bug

Please include the log. Right-click the widget and choose *Open log*, or find
it next to the settings file. It records what failed and why, which is usually
faster than a description.
