#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Type-checks the macOS front end from a Linux or WSL machine, without a Mac.
#
# `cargo check` and `cargo clippy` never link, so the AppKit, Core Graphics,
# Core Text and Carbon symbols do not have to exist — only the Rust has to be
# well typed. Two edits to a scratch copy of the tree are enough to make the
# compiler look at `src/unix/mac` on a Linux host:
#
#   * compile the module unconditionally rather than behind
#     `#[cfg(target_os = "macos")]`, and
#   * change `kind = "framework"` to `kind = "dylib"`, because rustc rejects the
#     framework link kind off Apple targets.
#
# The working tree is never touched: everything happens in a copy under /tmp.
#
# Three passes, because a single one would miss two things. `send_rect` has an
# `aarch64` branch that an x86_64 build never looks at, and `autostart.rs`,
# `locale.rs` and `secure.rs` each carry a macOS branch that a Linux host never
# compiles. Both are reached by flipping the relevant `cfg`s.
#
# A deliberate type error is injected first and the run is only trusted if the
# compiler reports it. It is easy to build a green harness that checks nothing.
#
# What this does not prove: that the selectors exist, that the message
# signatures match the ones AppKit really has, or that anything appears on
# screen. Those need a Mac. What it does prove is that the code compiles and
# passes the same lints continuous integration denies warnings for — which is
# the failure this repository would otherwise only discover on a runner.
set -uo pipefail

SRC=$(cd "$(dirname "$0")/.." && pwd)
WORK=${TMPDIR:-/tmp}/tpmplaner-macos-check
export CARGO_TARGET_DIR=${TMPDIR:-/tmp}/tpmplaner-macos-target

rm -rf "$WORK"
mkdir -p "$WORK"
cp -r "$SRC/src" "$SRC/crates" "$SRC/Cargo.toml" "$SRC/Cargo.lock" \
      "$SRC/README.md" "$SRC/LICENSE" "$WORK/"
cd "$WORK" || exit 1

perl -0pi -e 's/#\[cfg\(target_os = "macos"\)\]\nmod cf;/mod cf;/' src/unix/mod.rs
perl -0pi -e 's/#\[cfg\(target_os = "macos"\)\]\nmod mac;/mod mac;/' src/unix/mod.rs
grep -rl 'kind = "framework"' src | xargs sed -i 's/kind = "framework"/kind = "dylib"/g'
# Everything in these modules is unreachable on this host, which is not what is
# being tested.
sed -i '1i #![allow(dead_code)]' src/unix/cf.rs
for f in src/unix/mac/*.rs; do sed -i '1i #![allow(dead_code)]' "$f"; done

echo "=== canary: the harness must report a deliberate error ==="
printf '\nconst _HARNESS_CANARY: u32 = "not a number";\n' >> src/unix/mac/window.rs
cargo check > "$WORK/canary.txt" 2>&1
if grep -q "_HARNESS_CANARY" "$WORK/canary.txt"; then
  echo "canary seen — src/unix/mac really is being compiled"
else
  echo "CANARY NOT SEEN — the harness is checking nothing" >&2
  tail -n 40 "$WORK/canary.txt"
  exit 1
fi
perl -0pi -e 's/\nconst _HARNESS_CANARY: u32 = "not a number";\n//' src/unix/mac/window.rs

status=0
check() {
  echo
  echo "=== $1 ==="
  # Exactly what continuous integration runs, so a lint that would fail there
  # fails here too.
  out=$(cargo clippy --all-targets -- -D warnings 2>&1)
  count=$(printf '%s\n' "$out" | grep -cE '^error')
  echo "errors: $count"
  if [ "$count" != "0" ]; then
    printf '%s\n' "$out" | grep -E '^error' -A 12 | head -n 80
    status=1
  fi
}

check "as built on x86_64 — the objc_msgSend_stret path"

# The Apple-silicon branch of `send_rect`, compiled on this host instead. An
# aarch64 target cannot simply be added: `ring` needs a cross C compiler.
python3 - <<'EDIT'
import io

p = "src/unix/mac/objc.rs"
s = io.open(p, encoding="utf-8").read()
s = s.replace('#[cfg(target_arch = "x86_64")]', '#[cfg(any())]')
s = s.replace('#[cfg(target_arch = "aarch64")]', '#[cfg(target_arch = "x86_64")]')
io.open(p, "w", encoding="utf-8", newline="").write(s)
EDIT
check "with the aarch64 branch of send_rect compiled instead"

# The macOS halves of the modules that serve both systems: the launch agent,
# the Core Foundation date formatter and the Keychain.
python3 - <<'EDIT'
import io

for p in ("src/unix/autostart.rs", "src/unix/locale.rs", "src/unix/secure.rs"):
    s = io.open(p, encoding="utf-8").read()
    # Longest forms first, so a shorter one does not eat part of a longer. The
    # replacements are cfgs that are simply true or false on this host, rather
    # than `all()` and `any()`, which clippy rightly complains about.
    s = s.replace(
        '#[cfg(all(test, not(target_os = "macos")))]',
        '#[cfg(all(test, target_os = "windows"))]',
    )
    s = s.replace(
        '#[cfg(all(test, target_os = "macos"))]',
        '#[cfg(all(test, target_os = "linux"))]',
    )
    s = s.replace('#[cfg(not(target_os = "macos"))]', '#[cfg(target_os = "windows")]')
    s = s.replace('#[cfg(target_os = "macos")]', '#[cfg(target_os = "linux")]')
    io.open(p, "w", encoding="utf-8", newline="").write(s)
EDIT
check "with the macOS branches of autostart, locale and secure compiled"

exit "$status"
