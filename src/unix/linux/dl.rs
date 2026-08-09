// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! Opening the desktop's libraries at run time rather than linking them.
//!
//! Xlib, Cairo and Pango are all on any Linux machine with a desktop, and on
//! none of the machines the widget also has to work on: a container, a
//! continuous-integration runner, a headless server reached over SSH. Linking
//! them would make the binary refuse to *start* in all three, where what it
//! should do is print the agenda and exit — which is exactly what
//! [`crate::unix::text`] is for.
//!
//! `dlopen` also removes the build-time dependency entirely. The tarball is
//! one file that runs on any distribution with a desktop, and continuous
//! integration builds it on a runner with no `-dev` package installed at all.
//! The cost is this module and one `Option<fn>` per symbol, checked once when
//! the library is loaded rather than at every call.
//!
//! The versioned file names (`libX11.so.6`, not `libX11.so`) are deliberate:
//! the unversioned link belongs to the development package and is exactly what
//! is not installed on the machines this runs on.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use tpmplaner_core::log;

unsafe extern "C" {
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlerror() -> *const c_char;
}

/// `RTLD_LAZY | RTLD_GLOBAL`.
///
/// Global because Cairo and Pango find each other's symbols through the
/// process's own namespace: `pango_cairo_create_layout` needs Cairo, and if
/// Cairo were loaded privately it would be a second copy with its own state.
const RTLD_LAZY_GLOBAL: c_int = 0x0001 | 0x0100;

/// A loaded shared library.
pub struct Library {
    handle: *mut c_void,
    name: &'static str,
}

impl Library {
    /// Opens the first of `names` that is present.
    ///
    /// Several names because the same library is not always called the same
    /// thing: Pango ships as `libpango-1.0.so.0` everywhere, but Cairo's Xlib
    /// surface lives in `libcairo.so.2` on most distributions and is split out
    /// on a few.
    pub fn open(names: &[&'static str]) -> Option<Self> {
        for name in names {
            let Ok(c) = CString::new(*name) else { continue };
            let handle = unsafe { dlopen(c.as_ptr(), RTLD_LAZY_GLOBAL) };
            if !handle.is_null() {
                return Some(Self { handle, name });
            }
        }
        log::warn(&format!(
            "Could not load {} ({}) — the graphical widget is unavailable",
            names.join(" or "),
            last_error().unwrap_or_else(|| "no reason given".into())
        ));
        None
    }

    /// Looks a symbol up, or reports which one was missing.
    ///
    /// A missing symbol means the library is present but older than this
    /// program expects, which is worth naming: "the widget did not start" is
    /// not something anybody can act on, and "libcairo.so.2 has no
    /// cairo_xlib_surface_create" is.
    ///
    /// # Safety
    ///
    /// `F` must be the symbol's real signature. Nothing checks it, so every
    /// call site is a place to be careful — which is why they all live in the
    /// one `ffi` module beside the declarations they are copied from.
    pub unsafe fn symbol<F: Copy>(&self, name: &CStr) -> Option<F> {
        debug_assert_eq!(
            std::mem::size_of::<F>(),
            std::mem::size_of::<*mut c_void>(),
            "a symbol is a pointer"
        );
        let raw = unsafe { dlsym(self.handle, name.as_ptr()) };
        if raw.is_null() {
            log::warn(&format!("{} has no {}", self.name, name.to_string_lossy()));
            return None;
        }
        // Transmuting a `*mut c_void` into a function pointer of the same
        // size, which is what `dlsym` is for and what POSIX guarantees.
        Some(unsafe { std::mem::transmute_copy::<*mut c_void, F>(&raw) })
    }
}

// The handle is only ever used to resolve symbols, which `dlsym` does without
// mutating anything, and it is never closed.
unsafe impl Send for Library {}
unsafe impl Sync for Library {}

fn last_error() -> Option<String> {
    let raw = unsafe { dlerror() };
    (!raw.is_null()).then(|| {
        unsafe { CStr::from_ptr(raw) }
            .to_string_lossy()
            .into_owned()
    })
}
