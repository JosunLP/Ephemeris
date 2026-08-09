// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! The macOS front end: an `NSWindow` at the desktop level with a Core
//! Graphics renderer.
//!
//! What is here is only what AppKit, Core Graphics, Core Text and Carbon are
//! needed for. The widget's behaviour is in [`crate::unix::app`] and its
//! drawing in [`crate::unix::paint`], both shared with the Linux front end;
//! everything portable is in `tpmplaner-core`.
//!
//! | | |
//! |---|---|
//! | [`objc`] | The slice of the Objective-C runtime and the frameworks |
//! | [`window`] | The window, the classes, the event loop and the shell |
//! | [`canvas`] | Core Graphics and Core Text behind [`crate::unix::canvas::Canvas`] |
//! | [`menu`] | The right-click menu as an `NSMenu` |
//! | [`visuals`] | Appearance and accessibility settings |
//! | [`hotkey`] | The global peek shortcut, through Carbon |

pub mod canvas;
pub mod hotkey;
pub mod menu;
pub mod objc;
pub mod visuals;
pub mod window;

use objc::*;

/// Is there a window server to talk to?
///
/// `NSApplication` cannot be created in a daemon context, over a plain SSH
/// session or in a continuous-integration container, and asking for one there
/// terminates the process rather than returning nil. The session's own type
/// answers the question first, which is what lets the front end fall back to
/// printing the agenda instead of dying.
pub fn has_window_server() -> bool {
    // A GUI session sets this; `launchd`'s background sessions and an SSH
    // login do not. `CGSessionCopyCurrentDictionary` would be the thorough
    // check and needs a connection to the window server to answer, which is
    // the very thing being tested for.
    std::env::var_os("SECURITYSESSIONID").is_some()
        || std::env::var_os("Apple_PubSub_Socket_Render").is_some()
}

/// Only one widget per session.
///
/// A second one would draw an identical window over the first and synchronise
/// in parallel. `NSRunningApplication` answers this without a lock file: it
/// lists what the session is running, and a bare binary started twice appears
/// twice under the same bundle identifier or, with no bundle, the same
/// executable path.
pub fn acquire_single_instance() -> bool {
    let _pool = Pool::new();
    let Ok(exe) = std::env::current_exe() else {
        // Without a path to compare there is nothing to be sure about, and
        // refusing to start would be the worse mistake.
        return true;
    };
    let exe = std::fs::canonicalize(&exe).unwrap_or(exe);

    unsafe {
        let running: Id = send(class(c"NSRunningApplication") as Id, c"currentApplication");
        let own_pid: i32 = send(running, c"processIdentifier");

        let workspace: Id = send(class(c"NSWorkspace") as Id, c"sharedWorkspace");
        let apps: Id = send(workspace, c"runningApplications");
        let count: usize = send(apps, c"count");
        for i in 0..count {
            let app: Id = send1(apps, c"objectAtIndex:", i);
            let pid: i32 = send(app, c"processIdentifier");
            if pid == own_pid {
                continue;
            }
            let url: Id = send(app, c"executableURL");
            if url.is_null() {
                continue;
            }
            let path: Id = send(url, c"path");
            let Some(path) = nsstring_to_string(path) else {
                continue;
            };
            let other = std::path::PathBuf::from(&path);
            let other = std::fs::canonicalize(&other).unwrap_or(other);
            if other == exe {
                return false;
            }
        }
    }
    true
}

/// A Rust string from an `NSString`, via its UTF-8 representation.
fn nsstring_to_string(s: Id) -> Option<String> {
    if s.is_null() {
        return None;
    }
    let bytes: *const std::ffi::c_char = unsafe { send(s, c"UTF8String") };
    if bytes.is_null() {
        return None;
    }
    unsafe { std::ffi::CStr::from_ptr(bytes) }
        .to_str()
        .ok()
        .map(str::to_owned)
}
