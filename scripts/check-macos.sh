#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Type-checks the macOS front end from a Linux or WSL machine, without a Mac.
#
# `cargo check` and `cargo clippy` never link, so the AppKit, Core Graphics,
# Core Text and Carbon symbols do not have to exist — only the Rust has to be
# well typed. Two edits to a scratch copy of the tree are enough:
#
#   * swap the `target_os` gates so a Linux host compiles the *macOS* shape of
#     the program — `src/unix/mac` in, `src/unix/linux` out, and the macOS half
#     of every module that has one — and
#   * change `kind = "framework"` to `kind = "dylib"`, because rustc rejects the
#     framework link kind off Apple targets.
#
# Swapping rather than merely ungating matters. An earlier version of this
# script compiled the macOS modules *beside* the Linux ones and silenced
# `dead_code` to keep the unreachable ones quiet — which also silenced six
# genuinely unused items that continuous integration then found on a real Mac.
# Compiling the same shape a Mac would means dead code is dead here too.
#
# The working tree is never touched. A deliberate type error is injected first
# and the run is only trusted if the compiler reports it: it is easy to build a
# green harness that checks nothing.
#
# What this does not prove: that the selectors exist, that the message
# signatures match the ones AppKit really has, or that anything appears on
# screen. Those need a Mac.
set -uo pipefail

SRC=$(cd "$(dirname "$0")/.." && pwd)
WORK=${TMPDIR:-/tmp}/tpmplaner-macos-check
export CARGO_TARGET_DIR=${TMPDIR:-/tmp}/tpmplaner-macos-target

rm -rf "$WORK"
mkdir -p "$WORK"
cp -r "$SRC/src" "$SRC/crates" "$SRC/Cargo.toml" "$SRC/Cargo.lock" \
      "$SRC/README.md" "$SRC/LICENSE" "$WORK/"
cd "$WORK" || exit 1

# `framework` is an Apple-only link kind; nothing is linked here anyway.
grep -rl 'kind = "framework"' src | xargs sed -i 's/kind = "framework"/kind = "dylib"/g'

# Every gate that says "macOS" now says "the host", and every gate that says
# "Linux" says "Windows" — false here. Real `target_os` values rather than
# `all()` and `any()`, which clippy rightly complains about.
swap() {
  python3 - "$@" <<'EDIT'
import io
import sys

for path in sys.argv[1:]:
    s = io.open(path, encoding="utf-8").read()
    # Longest forms first, so a shorter one cannot eat part of a longer.
    for a, b in (
        ('#[cfg(all(test, not(target_os = "macos")))]', '#[cfg(all(test, target_os = "windows"))]'),
        ('#[cfg(all(test, target_os = "macos"))]', '#[cfg(all(test, target_os = "linux"))]'),
        ('#[cfg(not(any(target_os = "macos", target_os = "linux")))]', '#[cfg(target_os = "windows")]'),
        ('#[cfg(not(target_os = "macos"))]', '#[cfg(target_os = "windows")]'),
        ('#[cfg(not(target_os = "linux"))]', '#[cfg(not(target_os = "windows"))]'),
        ('#[cfg(target_os = "linux")]', '#[cfg(target_os = "windows")]'),
        ('#[cfg(target_os = "macos")]', '#[cfg(target_os = "linux")]'),
    ):
        s = s.replace(a, b)
    io.open(path, "w", encoding="utf-8", newline="").write(s)
EDIT
}

swap src/unix/mod.rs src/unix/autostart.rs src/unix/locale.rs src/unix/secure.rs

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
    printf '%s\n' "$out" | grep -E '^error' -A 12 | head -n 100
    status=1
  fi
}

check "the macOS build, on x86_64 — the objc_msgSend_stret path"

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

exit "$status"
