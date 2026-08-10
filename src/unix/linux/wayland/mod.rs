// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! The Wayland back end.
//!
//! | | |
//! |---|---|
//! | [`ffi`] | libwayland's symbols, and the protocols it does not ship |
//! | [`window`] | The surface, the buffers, the input and the loop |
//! | [`menu`] | The right-click menu as an `xdg_popup` |
//! | [`cursor`] | The pointer, which a Wayland client has to draw itself |
//!
//! What this back end can and cannot promise depends entirely on the
//! compositor, and it says which at every start rather than leaving the user
//! to work it out — see [`report_shape`].

pub mod cursor;
pub mod ffi;
pub mod menu;
pub mod window;

use tpmplaner_core::log;
use window::Shape;

pub fn run() -> Result<(), String> {
    window::run()
}

/// Is there a Wayland compositor to talk to?
pub fn available() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
}

/// Records what the compositor turned out to support, and what follows.
///
/// The decision in `docs/development/porting.md` was that where the widget
/// cannot sit below other windows it says so rather than pretending, and this
/// is where it says it. A user whose widget is behaving like an ordinary
/// window gets the reason in the file bug reports ask for.
pub fn report_shape(shape: Shape) {
    match shape {
        Shape::Layer => log::info(
            "Wayland with wlr-layer-shell — the widget is on the bottom layer, \
             below every other window.",
        ),
        Shape::Toplevel => log::warn(
            "This compositor has no wlr-layer-shell, so the widget is an ordinary window: \
             it sits among your windows rather than behind them, and the compositor \
             places it rather than the saved position. GNOME's Mutter is the usual \
             reason. Everything else works.",
        ),
    }
}

/// Says how the peek shortcut works here, which is not the way it works
/// anywhere else.
///
/// Wayland gives no client the power to grab a key: the compositor owns every
/// shortcut, which is a deliberate improvement on X11 and means the widget
/// cannot register one for itself. What it can do is answer, so a second copy
/// started with `--peek` tells the running one to come forward, and a line in
/// the compositor's configuration is the shortcut.
pub fn report_hotkey(configured: &str) {
    if configured.trim().is_empty() {
        return;
    }
    log::info(
        "Wayland compositors own every keyboard shortcut, so peek_hotkey does nothing here. \
         Bind a key to `tpmplaner --peek` in your compositor's configuration instead — \
         for example `bindsym $mod+k exec tpmplaner --peek` in Sway.",
    );
}
