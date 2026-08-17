// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Ephemeris contributors
//! The system's appearance and accessibility settings, read from macOS.
//!
//! [`ephemeris_core::theme::SystemVisuals`] is what the palette is resolved
//! against, and every one of its five values has an answer here:
//!
//! | | |
//! |---|---|
//! | light or dark | `NSApp.effectiveAppearance`, matched against the two names |
//! | accent colour | `NSColor.controlAccentColor`, converted to sRGB |
//! | transparency | `NSWorkspace.accessibilityDisplayShouldReduceTransparency` |
//! | animations | `NSWorkspace.accessibilityDisplayShouldReduceMotion` |
//! | high contrast | `NSWorkspace.accessibilityDisplayShouldIncreaseContrast` |
//!
//! The three accessibility switches are settings somebody turned on because
//! they needed to, so the widget follows them rather than deciding for itself:
//! motion can trigger symptoms in people with vestibular disorders, and a
//! translucent panel over a wallpaper is exactly what "reduce transparency"
//! exists to stop.
//!
//! macOS has no equivalent of the Windows high-contrast palette — "increase
//! contrast" strengthens borders and removes translucency rather than handing
//! out five system colours — so [`SystemVisuals::contrast`] stays `None` and
//! the core falls back to its own contrast palette. That is the documented
//! behaviour of the field, not a gap.

use crate::unix::mac::objc::*;
use ephemeris_core::theme::SystemVisuals;

pub fn read() -> SystemVisuals {
    let _pool = Pool::new();
    SystemVisuals {
        light: !is_dark(),
        accent: Some(accent()),
        transparency: !workspace_flag(c"accessibilityDisplayShouldReduceTransparency"),
        animations: !workspace_flag(c"accessibilityDisplayShouldReduceMotion"),
        high_contrast: workspace_flag(c"accessibilityDisplayShouldIncreaseContrast"),
        contrast: None,
    }
}

/// Asks to be told when any of this changes.
///
/// Two notifications, because the two halves are broadcast separately: the
/// distributed centre carries the system-wide light/dark switch, and the
/// workspace centre carries the accessibility ones. The accent colour arrives
/// on the first of them.
pub fn watch(delegate: Id) {
    unsafe {
        let distributed: Id = send(
            class(c"NSDistributedNotificationCenter") as Id,
            c"defaultCenter",
        );
        send4::<Id, Sel, Id, Id, ()>(
            distributed,
            c"addObserver:selector:name:object:",
            delegate,
            sel(c"onAppearanceChanged:"),
            nsstring("AppleInterfaceThemeChangedNotification"),
            nil,
        );

        let workspace: Id = send(class(c"NSWorkspace") as Id, c"sharedWorkspace");
        let centre: Id = send(workspace, c"notificationCenter");
        send4::<Id, Sel, Id, Id, ()>(
            centre,
            c"addObserver:selector:name:object:",
            delegate,
            sel(c"onAppearanceChanged:"),
            nsstring("NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification"),
            nil,
        );
    }
}

/// Is the interface in its dark appearance?
///
/// `bestMatchFromAppearancesWithNames:` rather than comparing the name
/// directly: an appearance can be `NSAppearanceNameAccessibilityHighContrastDarkAqua`
/// among others, and asking which of the two *base* appearances it is closest
/// to is the documented way to get a yes or no out of that.
fn is_dark() -> bool {
    unsafe {
        let app: Id = send(class(c"NSApplication") as Id, c"sharedApplication");
        let appearance: Id = send(app, c"effectiveAppearance");
        if appearance.is_null() {
            return false;
        }
        let names: Id = send2(
            class(c"NSArray") as Id,
            c"arrayWithObjects:count:",
            [
                nsstring("NSAppearanceNameAqua"),
                nsstring("NSAppearanceNameDarkAqua"),
            ]
            .as_ptr(),
            2usize,
        );
        let best: Id = send1(appearance, c"bestMatchFromAppearancesWithNames:", names);
        if best.is_null() {
            return false;
        }
        let dark: i8 = send1(
            best,
            c"isEqualToString:",
            nsstring("NSAppearanceNameDarkAqua"),
        );
        dark != 0
    }
}

/// The accent colour as `0xRRGGBB`.
///
/// Converted into the sRGB space first. `controlAccentColor` is a dynamic
/// catalogue colour and asking a component of it directly raises — the
/// conversion is what turns it into something with numbers in it.
fn accent() -> u32 {
    const FALLBACK: u32 = 0x00_7AFF;
    unsafe {
        let color: Id = send(class(c"NSColor") as Id, c"controlAccentColor");
        if color.is_null() {
            return FALLBACK;
        }
        let srgb: Id = send1(color, c"colorUsingColorSpace:", {
            let space: Id = send(class(c"NSColorSpace") as Id, c"sRGBColorSpace");
            space
        });
        if srgb.is_null() {
            return FALLBACK;
        }
        let r: CGFloat = send(srgb, c"redComponent");
        let g: CGFloat = send(srgb, c"greenComponent");
        let b: CGFloat = send(srgb, c"blueComponent");
        let byte = |v: CGFloat| ((v.clamp(0.0, 1.0) * 255.0).round() as u32) & 0xFF;
        (byte(r) << 16) | (byte(g) << 8) | byte(b)
    }
}

fn workspace_flag(selector: &std::ffi::CStr) -> bool {
    unsafe {
        let workspace: Id = send(class(c"NSWorkspace") as Id, c"sharedWorkspace");
        if workspace.is_null() {
            return false;
        }
        let on: i8 = send(workspace, selector);
        on != 0
    }
}
