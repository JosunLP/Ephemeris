// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! The Linux front end: one binary, two window systems.
//!
//! | | |
//! |---|---|
//! | [`wayland`] | `wlr-layer-shell` where there is one, an ordinary window where there is not |
//! | [`window`] | X11, with the EWMH hints that give the widget its behaviour |
//! | [`canvas`] | Cairo and Pango behind [`crate::unix::canvas::Canvas`], shared by both |
//! | [`drawn_menu`] | The right-click menu, which Linux has none of and the widget draws |
//! | [`ffi`] / [`dl`] | Xlib, Cairo and Pango, opened at run time rather than linked |
//! | [`visuals`] | Appearance through the XDG settings portal |
//!
//! **Wayland is preferred where it is running**, and X11 is not a lesser
//! fallback: it is the older of two equal implementations, and a Wayland
//! session with XWayland could use either. Wayland wins there because it is
//! the session the user is actually in — going through XWayland means the
//! compositor treats the widget as a foreign client and the stacking request
//! is far less likely to be honoured.
//!
//! Where neither is available — a container, a server over SSH, a
//! continuous-integration runner — [`crate::unix::text`] prints the agenda
//! instead, which is decided in [`crate::unix`] rather than here.

pub mod canvas;
pub mod dl;
pub mod drawn_menu;
pub mod ffi;
pub mod menu;
pub mod visuals;
pub mod wayland;
pub mod window;

use tpmplaner_core::log;

/// Is there anything here to open a window on?
pub fn has_display() -> bool {
    wayland::available() || std::env::var_os("DISPLAY").is_some()
}

pub fn run() -> Result<(), String> {
    if wayland::available() {
        // A Wayland session that also has XWayland could take either route.
        // The native one is right: under XWayland the compositor treats the
        // widget as a foreign client, and the one request that matters —
        // "stay below everything" — is the one it is least likely to honour.
        match wayland::run() {
            Ok(()) => return Ok(()),
            Err(e) if std::env::var_os("DISPLAY").is_some() => {
                log::warn(&format!(
                    "The Wayland front end could not start ({e}) — falling back to XWayland."
                ));
            }
            Err(e) => return Err(e),
        }
    }
    window::run()
}

/// Opens a file, a folder or a URL in whatever the desktop has registered.
///
/// Shared by both back ends: `xdg-open` neither knows nor cares which window
/// system asked.
pub fn open_path(path: &std::path::Path) {
    let run = std::process::Command::new("xdg-open")
        .arg(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    match run {
        Ok(mut child) => {
            // Reaped so the widget does not collect zombies over days of
            // uptime, for the reason set out in `unix::host::open_url`.
            let _ = std::thread::Builder::new()
                .name("tpmplaner-open".into())
                .spawn(move || {
                    let _ = child.wait();
                });
        }
        Err(e) => log::warn(&format!("Could not run xdg-open: {e}")),
    }
}

/// Records what kind of X11 session this is, and what follows from it.
///
/// Only reached when the X11 back end is running. Under a Wayland session that
/// means XWayland was chosen deliberately or the native path failed, and
/// either way the stacking request is the compositor's to honour or ignore.
pub fn report_session() {
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    let session = std::env::var("XDG_SESSION_TYPE").unwrap_or_default();
    if wayland || session == "wayland" {
        log::warn(
            "Running under XWayland. Whether the widget stays below other windows is up to \
             the compositor, and several ignore _NET_WM_STATE_BELOW for X11 clients — the \
             widget may behave as an ordinary window.",
        );
    } else {
        log::info("X11 session — the widget is stacked below other windows.");
    }
}

/// The name of the socket that makes the widget single-instance, and carries
/// a message to the copy already running.
///
/// One per display rather than one per machine: two sessions on one host are
/// two desktops, and each should have its own widget.
fn socket_name() -> String {
    let display = std::env::var("WAYLAND_DISPLAY")
        .or_else(|_| std::env::var("DISPLAY"))
        .unwrap_or_default();
    format!("tpmplaner-{display}")
}

/// Only one widget per display, and a way to talk to it.
///
/// The lock is an abstract socket, which is a Linux feature worth using here:
/// it has no path, so nothing is left behind in `/tmp` for the next start to
/// trip over, and the kernel releases it when the process ends however it ends
/// — including a kill that no cleanup code would survive.
///
/// It is also the channel `--peek` uses. That is not an extra: Wayland gives
/// no client the power to grab a key, so a compositor keybinding running
/// `tpmplaner --peek` is the only way the shortcut can work there, and this is
/// what it talks to.
pub fn acquire_single_instance() -> bool {
    use std::os::linux::net::SocketAddrExt;
    use std::os::unix::net::{SocketAddr, UnixListener};

    let Ok(address) = SocketAddr::from_abstract_name(socket_name().as_bytes()) else {
        // Too long a name to be an abstract address, which means no lock — and
        // refusing to start would be the worse mistake.
        return true;
    };
    match UnixListener::bind_addr(&address) {
        Ok(listener) => {
            listener.set_nonblocking(true).ok();
            LISTENER.with(|l| *l.borrow_mut() = Some(listener));
            true
        }
        Err(_) => {
            log::info("Another copy is already on this display");
            false
        }
    }
}

thread_local! {
    /// Held for the life of the process: it is both the single-instance lock
    /// and the socket `--peek` connects to.
    static LISTENER: std::cell::RefCell<Option<std::os::unix::net::UnixListener>> =
        const { std::cell::RefCell::new(None) };
}

/// The descriptor a loop should poll for a `--peek` from another copy, if
/// there is one.
pub fn control_fd() -> Option<std::os::fd::RawFd> {
    use std::os::fd::AsRawFd;
    LISTENER.with(|l| l.borrow().as_ref().map(|s| s.as_raw_fd()))
}

/// Accepts everything waiting on the control socket; true if any of it asked
/// for a peek.
pub fn take_control_requests() -> bool {
    use std::io::Read;
    LISTENER.with(|l| {
        let borrowed = l.borrow();
        let Some(listener) = borrowed.as_ref() else {
            return false;
        };
        let mut peek = false;
        while let Ok((stream, _)) = listener.accept() {
            stream.set_nonblocking(false).ok();
            let mut message = String::new();
            // Bounded: whatever connected is not necessarily this program,
            // and an unbounded read from a stranger is a way to be held open.
            let _ = stream.take(64).read_to_string(&mut message);
            if message.trim() == PEEK {
                peek = true;
            }
        }
        peek
    })
}

/// What `--peek` writes down the socket.
const PEEK: &str = "peek";

/// Tells the running widget to come forward, and says whether anything was
/// listening.
///
/// This is what a compositor keybinding runs. It is a second, very short-lived
/// copy of the program: it connects, writes one word and exits.
pub fn send_peek() -> bool {
    use std::io::Write;
    use std::os::linux::net::SocketAddrExt;
    use std::os::unix::net::{SocketAddr, UnixStream};

    let Ok(address) = SocketAddr::from_abstract_name(socket_name().as_bytes()) else {
        return false;
    };
    match UnixStream::connect_addr(&address) {
        Ok(mut stream) => stream.write_all(PEEK.as_bytes()).is_ok(),
        Err(_) => false,
    }
}
