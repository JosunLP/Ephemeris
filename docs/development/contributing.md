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

Rust 1.90 or newer, plus the MSVC build tools on Windows.

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

The largest open piece is the **cross-platform interface**. The portable core
already builds and tests on all three systems; `render.rs` and `window.rs` are
what remains Windows-only.

Also useful, and smaller:

- Additional interface languages — one `Catalog` constant in `i18n.rs`, plus a
  line in `catalog_for` and `CATALOGS`. See
  [CONTRIBUTING.md](https://github.com/JosunLP/TPMPlaner/blob/main/CONTRIBUTING.md#adding-an-interface-language)
  for what a translation pull request should contain: who checked the text, the
  right plural variant, and strings that fit the column.
- Verification of the Microsoft and CalDAV back ends against real servers
- Keychain and Secret Service backends for `Host::protect`

## House style

Comments explain **why**, not what. The valuable comment is the one recording
the trap — `model::parse_task_due` shows the shape of it. A bug fix comes with
a test that fails without it. English throughout.

And say what you could not verify. A pull request noting "never run against a
real tenant" is more useful than one that quietly implies it was.
