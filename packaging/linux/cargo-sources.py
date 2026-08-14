#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Turns Cargo.lock into the source list a Flatpak build needs.

A Flatpak build has no network. Every crate therefore has to be listed as a
source `flatpak-builder` fetches and verifies before the build starts, and
`cargo` has to be pointed at the result instead of at crates.io.

The upstream tool for this is `flatpak-cargo-generator.py` from
flatpak-builder-tools. This is the same output from the same input, without the
dependency: every checksum it would look up is already in `Cargo.lock`, so the
answer is a pure function of that file and no network access is needed to
produce it — which is also what lets continuous integration check that the two
are still in step.

    python3 packaging/linux/cargo-sources.py

Run it whenever `Cargo.lock` changes; `--check` verifies instead of writing,
which is what CI does.
"""

from __future__ import annotations

import argparse
import json
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
LOCK = ROOT / "Cargo.lock"
OUT = ROOT / "packaging" / "linux" / "cargo-sources.json"

# Only crates that come from the registry are fetched. The workspace's own two
# packages have no `source` and arrive with the checkout.
REGISTRY = "registry+https://github.com/rust-lang/crates.io-index"

# Where the module is unpacked inside the sandbox. `flatpak-builder` uses the
# module's name, and the manifest sets `CARGO_HOME` to match.
BUILD_DIR = "/run/build/ephemeris"

CARGO_CONFIG = f"""\
[source.crates-io]
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "{BUILD_DIR}/cargo/vendor"
"""


def packages(lock: str) -> list[tuple[str, str, str]]:
    """`(name, version, checksum)` for every crate fetched from the registry."""
    found = []
    for block in lock.split("[[package]]")[1:]:
        fields = dict(re.findall(r'^(\w+) = "([^"]*)"$', block, re.MULTILINE))
        if fields.get("source") != REGISTRY:
            continue
        name, version = fields.get("name"), fields.get("version")
        checksum = fields.get("checksum")
        if not (name and version and checksum):
            # A registry package with no checksum cannot be verified, and a
            # source that is fetched but not verified is worse than none.
            sys.exit(f"{name or '?'} has no checksum in Cargo.lock")
        found.append((name, version, checksum))
    return sorted(found)


def sources(crates: list[tuple[str, str, str]]) -> list[dict]:
    out: list[dict] = []
    for name, version, checksum in crates:
        vendor = f"cargo/vendor/{name}-{version}"
        out.append(
            {
                "type": "archive",
                "archive-type": "tar-gzip",
                "url": f"https://static.crates.io/crates/{name}/{name}-{version}.crate",
                "sha256": checksum,
                "dest": vendor,
            }
        )
        # `cargo` refuses a vendored crate without one of these. The empty
        # `files` map is what tells it not to re-verify each file individually,
        # which is what the upstream generator emits too.
        out.append(
            {
                "type": "inline",
                "contents": json.dumps({"package": checksum, "files": {}}),
                "dest": vendor,
                "dest-filename": ".cargo-checksum.json",
            }
        )
    out.append(
        {
            "type": "inline",
            "contents": CARGO_CONFIG,
            "dest": "cargo",
            "dest-filename": "config.toml",
        }
    )
    return out


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--check",
        action="store_true",
        help="fail if the file on disk is not what this would write",
    )
    args = parser.parse_args()

    crates = packages(LOCK.read_text(encoding="utf-8"))
    rendered = json.dumps(sources(crates), indent=4) + "\n"

    if args.check:
        current = OUT.read_text(encoding="utf-8") if OUT.exists() else ""
        if current != rendered:
            print(
                f"{OUT.relative_to(ROOT)} is out of step with Cargo.lock — "
                "run packaging/linux/cargo-sources.py",
                file=sys.stderr,
            )
            return 1
        print(f"{len(crates)} crates, in step with Cargo.lock")
        return 0

    OUT.write_text(rendered, encoding="utf-8")
    print(f"wrote {OUT.relative_to(ROOT)} — {len(crates)} crates")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
