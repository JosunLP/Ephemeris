// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Ephemeris contributors

//! Puts the icon and the version block into the Windows executable.
//!
//! On Windows there is nowhere else for an icon to live. Explorer, the
//! download bar of every browser, the properties dialogue and the Start menu
//! shortcut the installer creates all read it out of the binary's own resource
//! section — the shortcut deliberately sets no `IconLocation`, so whatever is
//! embedded here is what it shows. The version block that comes with it is
//! what the properties dialogue lists under Details.
//!
//! Everything is behind `cfg(windows)`, which in a build script is the machine
//! doing the building rather than the machine being built for. That is the
//! same condition `Cargo.toml` declares the dependency under, so on Linux and
//! macOS this file compiles to an empty `main` and the crate is never fetched.
//! Cross-compiling to Windows from either would fall through it and produce a
//! binary with no icon rather than a build failure; the release builds every
//! Windows target on a Windows runner, so that path is not one taken here.

fn main() {
    #[cfg(windows)]
    {
        // Only this file matters — the rest of the crate is the compiler's
        // business, and without this the resource is rebuilt on every change
        // to anything.
        println!("cargo:rerun-if-changed=assets/logo.ico");
        println!("cargo:rerun-if-changed=build.rs");

        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("assets/logo.ico");
        // A failure here is the resource compiler missing or the icon being
        // unreadable, and both would otherwise ship as a binary that quietly
        // has no icon. CI builds this target on every push, so it is found
        // long before a release is cut.
        resource
            .compile()
            .expect("embedding assets/logo.ico into the executable");
    }
}
