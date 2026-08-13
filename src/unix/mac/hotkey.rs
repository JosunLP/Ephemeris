// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! The global shortcut that brings the widget forward for a few seconds.
//!
//! `RegisterEventHotKey` rather than an `NSEvent` global monitor, and
//! deliberately: a global monitor needs the accessibility permission, which
//! means a system dialogue, a trip to System Settings and a widget that does
//! nothing at all until somebody comes back and grants it. `RegisterEventHotKey`
//! is Carbon, it is not deprecated, and it needs no permission whatsoever.
//!
//! The key codes are the awkward part. Carbon's are *physical* positions on an
//! ANSI keyboard rather than characters, so `kVK_ANSI_A` is 0 and `kVK_ANSI_B`
//! is 11, and there is no arithmetic that produces them — hence the table.
//! They describe where the key sits, so `Ctrl+Alt+Shift+K` is the same physical
//! key on a German keyboard as on a British one, which for a shortcut is the
//! behaviour wanted.

use crate::unix::mac::objc::*;
use crate::unix::mac::window;
use std::cell::Cell;
use std::ffi::c_void;
use tpmplaner_core::hotkey::{Combination, Key};
use tpmplaner_core::log;

thread_local! {
    /// The registration, kept so it can be undone at shutdown.
    static REGISTERED: Cell<EventHotKeyRef> = const { Cell::new(std::ptr::null_mut()) };
    /// Installed once; a second handler would fire the peek twice.
    static HANDLER: Cell<bool> = const { Cell::new(false) };
}

/// `'TPMP'` as the four-character signature Carbon identifies the hotkey by.
const SIGNATURE: u32 = u32::from_be_bytes(*b"TPMP");

/// Registers the shortcut, falling back through the shared list when the
/// configured one is already taken.
///
/// An empty setting turns the feature off, which is the documented way to do
/// that and not a failure worth a warning.
pub fn install(configured: &str) {
    if configured.trim().is_empty() {
        return;
    }
    if !install_handler() {
        return;
    }

    for spec in tpmplaner_core::hotkey::candidates(configured) {
        let Some(combo) = tpmplaner_core::hotkey::parse(spec) else {
            log::warn(&format!("peek_hotkey '{spec}' is not a usable combination"));
            continue;
        };
        let Some(code) = key_code(combo.key) else {
            log::warn(&format!("peek_hotkey '{spec}' has no key code on macOS"));
            continue;
        };
        if try_register(code, modifiers(combo)) {
            if spec == configured.trim() {
                log::info(&format!("Peek hotkey: {spec}"));
            } else {
                log::warn(&format!(
                    "peek_hotkey '{configured}' is taken by another application; \
                     using {spec} instead"
                ));
            }
            return;
        }
    }
    log::warn("No peek hotkey could be registered; set peek_hotkey in config.json");
}

pub fn remove() {
    REGISTERED.with(|r| {
        let raw = r.replace(std::ptr::null_mut());
        if !raw.is_null() {
            unsafe { UnregisterEventHotKey(raw) };
        }
    });
}

fn try_register(code: u32, modifiers: u32) -> bool {
    let mut reference: EventHotKeyRef = std::ptr::null_mut();
    let status = unsafe {
        RegisterEventHotKey(
            code,
            modifiers,
            EventHotKeyID {
                signature: SIGNATURE,
                id: 1,
            },
            GetApplicationEventTarget(),
            0,
            &mut reference,
        )
    };
    if status != 0 || reference.is_null() {
        return false;
    }
    remove();
    REGISTERED.with(|r| r.set(reference));
    true
}

/// Installs the handler that turns a hotkey event into a peek.
///
/// Once per process: Carbon would happily install a second one, and the widget
/// would then rise, sink and rise again on one key press.
fn install_handler() -> bool {
    if HANDLER.with(|h| h.get()) {
        return true;
    }
    let spec = EventTypeSpec {
        eventClass: kEventClassKeyboard,
        eventKind: kEventHotKeyPressed,
    };
    let mut handler: EventHandlerRef = std::ptr::null_mut();
    let status = unsafe {
        InstallEventHandler(
            GetApplicationEventTarget(),
            on_hotkey as *const c_void,
            1,
            &spec,
            std::ptr::null_mut(),
            &mut handler,
        )
    };
    if status != 0 {
        log::warn(&format!(
            "Could not install the hotkey handler (OSStatus {status}) — \
             the peek shortcut is unavailable"
        ));
        return false;
    }
    HANDLER.with(|h| h.set(true));
    true
}

/// `noErr`: the event is ours and has been dealt with.
extern "C" fn on_hotkey(
    _call: EventHandlerCallRef,
    _event: EventRef,
    _user_data: *mut c_void,
) -> i32 {
    // Only one hotkey is ever registered, so there is nothing to tell apart
    // and no need to read the identifier back out of the event.
    let _pool = Pool::new();
    window::on_hotkey();
    0
}

/// Carbon's modifier mask for a parsed combination.
///
/// The Command key is what `meta` means here. The settings file calls it
/// `Win`, `Cmd` or `Super` and [`tpmplaner_core::hotkey`] accepts all three,
/// precisely so one file can be carried between machines.
fn modifiers(combo: Combination) -> u32 {
    let mut mask = 0;
    if combo.ctrl {
        mask |= controlKey;
    }
    if combo.alt {
        mask |= optionKey;
    }
    if combo.shift {
        mask |= shiftKey;
    }
    if combo.meta {
        mask |= cmdKey;
    }
    mask
}

/// The virtual key code for a key, from `HIToolbox/Events.h`.
fn key_code(key: Key) -> Option<u32> {
    // `kVK_ANSI_A` … `kVK_ANSI_Z`, in alphabetical order rather than in the
    // order of the numbers, which have none.
    const LETTERS: [u32; 26] = [
        0, 11, 8, 2, 14, 3, 5, 4, 34, 38, 40, 37, 46, 45, 31, 35, 12, 15, 1, 17, 32, 9, 13, 7, 16,
        6,
    ];
    // `kVK_ANSI_0` … `kVK_ANSI_9`, likewise.
    const DIGITS: [u32; 10] = [29, 18, 19, 20, 21, 23, 22, 26, 28, 25];
    // `kVK_F1` … `kVK_F12`.
    const FUNCTION: [u32; 12] = [122, 120, 99, 118, 96, 97, 98, 100, 101, 109, 103, 111];

    match key {
        Key::Letter(c) if c.is_ascii_uppercase() => Some(LETTERS[(c - b'A') as usize]),
        Key::Digit(d) if d < 10 => Some(DIGITS[d as usize]),
        Key::Function(n) if (1..=12).contains(&n) => Some(FUNCTION[(n - 1) as usize]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The table is the whole risk here: a wrong entry registers some other
    /// key and there is nothing on screen to notice it by. These are the
    /// values in `HIToolbox/Events.h`.
    #[test]
    fn the_key_codes_are_the_ones_carbon_documents() {
        assert_eq!(key_code(Key::Letter(b'A')), Some(0));
        assert_eq!(key_code(Key::Letter(b'K')), Some(40));
        assert_eq!(key_code(Key::Letter(b'Z')), Some(6));
        assert_eq!(key_code(Key::Digit(0)), Some(29));
        assert_eq!(key_code(Key::Digit(9)), Some(25));
        assert_eq!(key_code(Key::Function(1)), Some(122));
        assert_eq!(key_code(Key::Function(12)), Some(111));
        // Nothing out of range may produce a code by accident.
        assert_eq!(key_code(Key::Function(13)), None);
        assert_eq!(key_code(Key::Letter(b'a')), None);
    }

    /// Every combination the shared fallback list offers has to be
    /// registrable, or the list is quietly one shorter here than elsewhere.
    #[test]
    fn every_fallback_maps_to_a_key_code() {
        for spec in tpmplaner_core::hotkey::FALLBACKS {
            let combo = tpmplaner_core::hotkey::parse(spec).expect(spec);
            assert!(key_code(combo.key).is_some(), "{spec}");
            assert_ne!(modifiers(combo), 0, "{spec}");
        }
    }
}
