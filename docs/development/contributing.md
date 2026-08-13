# Contributing

The full text lives in
[CONTRIBUTING.md](https://github.com/JosunLP/TPMPlaner/blob/main/CONTRIBUTING.md).
The essentials:

## Build

```bash
git clone https://github.com/JosunLP/TPMPlaner
cd TPMPlaner
cargo test --workspace
cargo run --release
```

Rust 1.90 or newer, plus the MSVC build tools on Windows. Nothing beyond a C
toolchain on macOS or Linux: Xlib, Cairo and Pango are opened at run time
rather than linked.

Working on the macOS front end without a Mac? `scripts/check-macos.sh`
type-checks `src/unix/mac` from a Linux or WSL machine under the same lints
continuous integration denies warnings for — `cargo check` never links, so the
AppKit symbols do not have to exist. What it cannot tell you is in
[Porting](/development/porting).

Preview the interface without connecting an account:

```powershell
$env:TPMPLANER_DEMO = "1"; cargo run --release
```

## Before a pull request

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## The rule that matters

`tpmplaner-core` may not call an operating system API. If it needs one, add a
method to a trait in `core/src/host.rs` and implement it in the front end. CI
builds the core on Ubuntu, macOS and Windows, so a violation fails the build.

## What is wanted

The largest open piece is a **native Wayland back end** through
`wlr-layer-shell`: the protocol spoken directly against a `libwayland-client`
opened the way Xlib already is, and a Cairo image surface behind a
shared-memory buffer. Everything above the `Canvas` and `Shell` traits already
works and would not change. See [Porting](/development/porting).

Also useful, and smaller:

- Additional interface languages — one `Catalog` constant in `i18n.rs`, plus a
  line in `catalog_for` and `CATALOGS`. See
  [CONTRIBUTING.md](https://github.com/JosunLP/TPMPlaner/blob/main/CONTRIBUTING.md#adding-an-interface-language)
  for what a translation pull request should contain: who checked the text, the
  right plural variant, and strings that fit the column.
- Verification of the Microsoft and CalDAV back ends against real servers
- The Direct2D renderer moved onto the `Canvas` trait, so there is one
  description of the interface rather than two
- A Flatpak, and a signed and notarised macOS `.app`

## House style

Comments explain **why**, not what. The valuable comment is the one recording
the trap — `model::parse_task_due` shows the shape of it. A bug fix comes with
a test that fails without it. English throughout.

And say what you could not verify. A pull request noting "never run against a
real tenant" is more useful than one that quietly implies it was.
