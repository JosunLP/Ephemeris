// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Ephemeris contributors
//! Reading `peek_hotkey` out of the settings.
//!
//! `"Ctrl+Alt+Shift+K"` is a string a user types into a JSON file, and turning
//! it into modifiers and a key is the same job on all three systems. What
//! differs is only the number each platform wants for the key — a Windows
//! virtual key code, a Carbon key code, an X11 keysym — and those are three
//! unrelated tables that belong beside the API that consumes them.
//!
//! So this parses and the front ends translate. Splitting it that way also
//! makes the awkward half testable: which spellings count as a modifier, that
//! a combination with no modifier is refused, and that `F13` is not a function
//! key any of them can register.

/// The key a combination ends in.
///
/// Deliberately narrow. Punctuation keys are where the three platforms stop
/// agreeing — the same physical key is a different code under a different
/// keyboard layout — and a global shortcut that lands somewhere else on a
/// French keyboard is worse than one the user has to choose again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// An upper-case ASCII letter, `A` to `Z`.
    Letter(u8),
    /// A digit, `0` to `9`.
    Digit(u8),
    /// `F1` to `F12`.
    Function(u8),
}

/// A parsed shortcut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Combination {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// The Windows key, the Command key, or the Super key, depending on where
    /// you are sitting.
    pub meta: bool,
    pub key: Key,
}

/// Parses a specification such as `"Ctrl+Alt+Shift+K"`.
///
/// `None` for anything unusable, which the caller reports and then tries the
/// next candidate. The German spellings are accepted because the settings file
/// is edited by hand and the widget ships in German among others.
pub fn parse(spec: &str) -> Option<Combination> {
    let mut combo = Combination {
        ctrl: false,
        alt: false,
        shift: false,
        meta: false,
        key: Key::Letter(b'K'),
    };
    let mut key = None;

    for part in spec.split('+').map(str::trim).filter(|p| !p.is_empty()) {
        match part.to_ascii_lowercase().as_str() {
            "win" | "windows" | "cmd" | "command" | "super" | "meta" => combo.meta = true,
            "alt" | "option" | "opt" => combo.alt = true,
            "ctrl" | "control" | "strg" => combo.ctrl = true,
            "shift" | "umschalt" => combo.shift = true,
            other => {
                // A second key is a mistake, not a second chance: silently
                // taking the last one would register a shortcut nobody asked
                // for.
                if key.is_some() {
                    return None;
                }
                key = parse_key(other);
                key?;
            }
        }
    }

    // Without a modifier this would be a global single key, taken away from
    // every other application on the machine.
    if !(combo.ctrl || combo.alt || combo.shift || combo.meta) {
        return None;
    }
    combo.key = key?;
    Some(combo)
}

fn parse_key(text: &str) -> Option<Key> {
    let bytes = text.as_bytes();
    if bytes.len() == 1 {
        let b = bytes[0];
        if b.is_ascii_alphabetic() {
            return Some(Key::Letter(b.to_ascii_uppercase()));
        }
        if b.is_ascii_digit() {
            return Some(Key::Digit(b - b'0'));
        }
        return None;
    }
    // `F1` to `F12`. Higher ones exist on some keyboards and on none of the
    // laptops this runs on, and not every platform can register them.
    let number = text.strip_prefix('f')?.parse::<u8>().ok()?;
    (1..=12).contains(&number).then_some(Key::Function(number))
}

/// Combinations tried when the configured one is already taken.
///
/// Measured on a normal Windows 11 desktop: `Ctrl+Alt+K` and `Win+Alt+K` are
/// both refused. Silently doing nothing would leave a documented feature dead,
/// so the widget falls back and records which combination it ended up with.
/// The same problem exists on the other two — a desktop environment or another
/// application gets there first — and the same list serves.
pub const FALLBACKS: &[&str] = &["Ctrl+Alt+Shift+K", "Ctrl+Shift+F12", "Ctrl+Alt+Y"];

/// Every combination worth trying for a configured value, in order.
///
/// The configured one first, then the fallbacks that are not it. Returned as
/// an iterator of the original strings so the caller can say which one it
/// ended up registering.
pub fn candidates(configured: &str) -> Vec<&str> {
    let trimmed = configured.trim();
    let mut out = vec![trimmed];
    out.extend(
        FALLBACKS
            .iter()
            .copied()
            .filter(|f| !f.eq_ignore_ascii_case(trimmed)),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_combination_parses_whatever_order_it_is_written_in() {
        let expected = Combination {
            ctrl: true,
            alt: true,
            shift: true,
            meta: false,
            key: Key::Letter(b'K'),
        };
        for spec in [
            "Ctrl+Alt+Shift+K",
            "shift + alt + ctrl + k",
            "STRG+ALT+UMSCHALT+K",
            "control+option+shift+K",
        ] {
            assert_eq!(parse(spec), Some(expected), "{spec}");
        }
    }

    #[test]
    fn digits_and_function_keys_are_their_own_kind() {
        assert_eq!(
            parse("Ctrl+Shift+F12").map(|c| c.key),
            Some(Key::Function(12))
        );
        assert_eq!(parse("Win+7").map(|c| c.key), Some(Key::Digit(7)));
        assert!(parse("Ctrl+F13").is_none(), "F13 is not registrable");
        assert!(parse("Ctrl+F0").is_none());
    }

    /// A shortcut with no modifier would take a plain key away from every
    /// other application on the machine.
    #[test]
    fn a_bare_key_is_refused() {
        for spec in ["K", "F5", "", "   ", "+"] {
            assert!(parse(spec).is_none(), "{spec:?}");
        }
    }

    /// Two keys is a typo, and quietly taking one of them would register a
    /// shortcut nobody asked for.
    #[test]
    fn nonsense_is_refused_rather_than_repaired() {
        for spec in ["Ctrl+K+J", "Ctrl+", "Ctrl+ß", "Ctrl++"] {
            assert!(parse(spec).is_none(), "{spec:?}");
        }
    }

    /// The configured combination is tried first, and never twice.
    #[test]
    fn the_fallback_list_starts_with_what_was_asked_for() {
        let list = candidates("  Ctrl+Alt+J ");
        assert_eq!(list[0], "Ctrl+Alt+J");
        assert_eq!(list.len(), 1 + FALLBACKS.len());

        let already = candidates("ctrl+alt+shift+k");
        assert_eq!(already[0], "ctrl+alt+shift+k");
        assert_eq!(
            already.len(),
            FALLBACKS.len(),
            "the configured combination must not also appear as a fallback"
        );
        for spec in candidates("Ctrl+Alt+J") {
            assert!(parse(spec).is_some(), "{spec} is not usable");
        }
    }
}
