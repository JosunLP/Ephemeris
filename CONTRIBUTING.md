# Contributing

Thanks for taking the time. This document covers what you need to get a change
merged.

## Getting set up

```bash
git clone https://github.com/JosunLP/TPMPlaner
cd TPMPlaner
cargo test --workspace
cargo run --release
```

To see the interface without connecting a calendar account:

```bash
TPMPLANER_DEMO=1 cargo run --release      # PowerShell: $env:TPMPLANER_DEMO=1
```

Rust 1.90 or newer. On Windows you also need the MSVC build tools.

## The one architectural rule

The project is a workspace with a deliberate split:

| Crate            | Contains                                               | May call the operating system |
| ---------------- | ------------------------------------------------------ | ----------------------------- |
| `tpmplaner-core` | model, calendar back ends, sync, localisation, palette | **no**                        |
| `tpmplaner`      | the front ends: `src/win`, `src/unix`                  | yes                           |

`tpmplaner-core` must compile and pass its tests on Windows, macOS and Linux.
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

## What good looks like here

- **Comments explain why, not what.** The code already says what it does. The
  valuable comment is the one recording the trap you fell into — see
  `model::parse_task_due` for the shape of it.
- **A bug fix comes with a test that fails without it.** Especially for date
  handling, where the failure mode is "off by one day in half the world".
- **English, in code and in documentation.**
- **Say what you could not verify.** A pull request that notes "this path was
  never run against a real server" is more useful than one that implies it was.

## Reporting a bug

Please include the log. Right-click the widget and choose *Open log*, or find
it next to the settings file. It records what failed and why, which is usually
faster than a description.
