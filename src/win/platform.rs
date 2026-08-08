// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! Small Windows helpers: opening a browser, autostart, trimming memory.

use tpmplaner_core::log;
use tpmplaner_core::theme::{ContrastColors, SystemVisuals};
use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    HMONITOR, MONITOR_DEFAULTTONULL, MONITORINFO, MonitorFromRect,
};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, REG_DWORD, REG_OPTION_NON_VOLATILE, REG_SZ,
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};
use windows::Win32::System::Threading::{
    CreateMutexW, GetCurrentProcess, SetProcessWorkingSetSize,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::PCWSTR;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "TPMPlaner";
const PERSONALIZE_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";

/// UTF-16 with a trailing null, as the Win32 `*W` functions expect.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn open_in_browser(url: &str) {
    let verb = wide("open");
    let target = wide(url);
    unsafe {
        ShellExecuteW(
            Some(HWND::default()),
            PCWSTR(verb.as_ptr()),
            PCWSTR(target.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }
}

/// Opens a path in its associated program: the settings file, the data folder.
pub fn open_path(path: &std::path::Path) {
    open_in_browser(&path.to_string_lossy());
}

pub fn autostart_enabled() -> bool {
    autostart_enabled_in(RUN_KEY)
}

/// Split out from [`autostart_enabled`] so a test can drive it against a
/// scratch key. Pointing the test at the real Run key would mean deleting the
/// user's own autostart entries to reach the case worth testing: the key not
/// being there at all.
fn autostart_enabled_in(subkey: &str) -> bool {
    unsafe {
        let mut key = HKEY::default();
        let sub = wide(subkey);
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(sub.as_ptr()),
            None,
            KEY_READ,
            &mut key,
        )
        .is_err()
        {
            return false;
        }
        let name = wide(RUN_VALUE);
        let mut size = 0u32;
        let present = RegQueryValueExW(
            key,
            PCWSTR(name.as_ptr()),
            None,
            None,
            None,
            Some(&mut size),
        )
        .is_ok();
        let _ = RegCloseKey(key);
        present
    }
}

pub fn set_autostart(enabled: bool) {
    set_autostart_in(RUN_KEY, enabled);
}

/// Split out from [`set_autostart`] for the same reason as
/// [`autostart_enabled_in`].
fn set_autostart_in(subkey: &str, enabled: bool) {
    unsafe {
        let mut key = HKEY::default();
        let sub = wide(subkey);
        // The Run key is not guaranteed to exist. A profile on which nothing
        // has ever registered itself for autostart simply does not have it,
        // and opening it then fails — which would leave switching autostart on
        // silently doing nothing at all. So create it when switching on;
        // creating is a no-op when it is already there. Switching off needs no
        // such care: no key means no value to remove, which is the wanted
        // state already.
        let opened = if enabled {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(sub.as_ptr()),
                None,
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE,
                None,
                &mut key,
                None,
            )
        } else {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(sub.as_ptr()),
                None,
                KEY_SET_VALUE,
                &mut key,
            )
        };
        if opened.is_err() {
            return;
        }
        let name = wide(RUN_VALUE);

        if enabled {
            // Quote the path, or Windows splits it at spaces such as in
            // "C:\Program Files\...".
            let exe = std::env::current_exe().unwrap_or_default();
            let quoted = format!("\"{}\"", exe.to_string_lossy());
            let value = wide(&quoted);
            let bytes = std::slice::from_raw_parts(
                value.as_ptr() as *const u8,
                value.len() * std::mem::size_of::<u16>(),
            );
            let _ = RegSetValueExW(key, PCWSTR(name.as_ptr()), None, REG_SZ, Some(bytes));
        } else {
            let _ = RegDeleteValueW(key, PCWSTR(name.as_ptr()));
        }

        let _ = RegCloseKey(key);
    }
}

/// Reads a DWORD value under HKEY_CURRENT_USER.
fn read_dword(subkey: &str, value: &str) -> Option<u32> {
    unsafe {
        let mut key = HKEY::default();
        let sub = wide(subkey);
        // `RegOpenKeyExW` returns a `WIN32_ERROR`, not a `Result`, so `?`
        // does not apply here.
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(sub.as_ptr()),
            None,
            KEY_READ,
            &mut key,
        )
        .is_err()
        {
            return None;
        }

        let name = wide(value);
        let mut data: u32 = 0;
        let mut size = std::mem::size_of::<u32>() as u32;
        let mut kind = REG_DWORD;
        let ok = RegQueryValueExW(
            key,
            PCWSTR(name.as_ptr()),
            None,
            Some(&mut kind),
            Some(&mut data as *mut u32 as *mut u8),
            Some(&mut size),
        )
        .is_ok();
        let _ = RegCloseKey(key);
        (ok && kind == REG_DWORD).then_some(data)
    }
}

/// Is Windows currently using the light app appearance?
///
/// `AppsUseLightTheme` is the value the system's own applications read;
/// `SystemUsesLightTheme` only covers the taskbar and start menu and would be
/// the wrong signal here.
pub fn system_uses_light_theme() -> bool {
    read_dword(PERSONALIZE_KEY, "AppsUseLightTheme") == Some(1)
}

/// The system accent colour as `0xRRGGBB`.
///
/// `DwmGetColorizationColor` returns `0xAARRGGBB`; the alpha part describes
/// the glass blend of the window frame and is meaningless here.
pub fn system_accent() -> Option<u32> {
    unsafe {
        use windows::Win32::Graphics::Dwm::DwmGetColorizationColor;
        let mut color: u32 = 0;
        let mut opaque = windows::core::BOOL::default();
        DwmGetColorizationColor(&mut color, &mut opaque).ok()?;
        Some(color & 0x00FF_FFFF)
    }
}

/// Reads the appearance settings the portable core reacts to.
pub fn system_visuals() -> SystemVisuals {
    use windows::Win32::Graphics::Gdi::{
        COLOR_GRAYTEXT, COLOR_HIGHLIGHT, COLOR_HOTLIGHT, COLOR_WINDOW, COLOR_WINDOWTEXT,
    };
    let high_contrast = high_contrast_active();
    SystemVisuals {
        high_contrast,
        // A missing value means "on" - that is how Windows ships.
        transparency: read_dword(PERSONALIZE_KEY, "EnableTransparency") != Some(0),
        animations: client_area_animation(),
        light: system_uses_light_theme(),
        accent: system_accent(),
        // There are several contrast themes with entirely different palettes,
        // so nothing is guessed: the system is asked.
        contrast: high_contrast.then(|| ContrastColors {
            window: sys_color(COLOR_WINDOW),
            text: sys_color(COLOR_WINDOWTEXT),
            gray: sys_color(COLOR_GRAYTEXT),
            highlight: sys_color(COLOR_HIGHLIGHT),
            hot: sys_color(COLOR_HOTLIGHT),
        }),
    }
}

fn high_contrast_active() -> bool {
    unsafe {
        use windows::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
        use windows::Win32::UI::WindowsAndMessaging::{
            SPI_GETHIGHCONTRAST, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
        };
        let mut hc = HIGHCONTRASTW {
            cbSize: std::mem::size_of::<HIGHCONTRASTW>() as u32,
            ..Default::default()
        };
        let ok = SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            hc.cbSize,
            Some(&mut hc as *mut HIGHCONTRASTW as *mut _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
        .is_ok();
        ok && (hc.dwFlags & HCF_HIGHCONTRASTON) == HCF_HIGHCONTRASTON
    }
}

fn client_area_animation() -> bool {
    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::{
            SPI_GETCLIENTAREAANIMATION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
        };
        let mut enabled = windows::core::BOOL::default();
        let ok = SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some(&mut enabled as *mut _ as *mut _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
        .is_ok();
        // Animate when in doubt: the switch is absent on older systems.
        !ok || enabled.as_bool()
    }
}

/// A system colour as `0xRRGGBB`.
///
/// `GetSysColor` returns a COLORREF, that is `0x00BBGGRR` — red and blue are
/// swapped relative to the notation used everywhere else here.
pub fn sys_color(index: windows::Win32::Graphics::Gdi::SYS_COLOR_INDEX) -> u32 {
    let bgr = unsafe { windows::Win32::Graphics::Gdi::GetSysColor(index) };
    let (r, g, b) = (bgr & 0xFF, (bgr >> 8) & 0xFF, (bgr >> 16) & 0xFF);
    (r << 16) | (g << 8) | b
}

/// How often [`open_clipboard`] tries, and how long it waits between attempts.
///
/// Two hundred milliseconds in total: longer than another application keeps
/// the clipboard for a copy of its own, and short enough that the menu click
/// this runs on cannot feel stuck.
const CLIPBOARD_ATTEMPTS: u32 = 10;
const CLIPBOARD_RETRY: std::time::Duration = std::time::Duration::from_millis(20);

/// Opens the clipboard for `owner`, retrying for a moment before giving up.
///
/// Only one task may have the clipboard open at a time and `OpenClipboard`
/// does not wait its turn — it fails at once. Clipboard managers, remote
/// desktop bridges and other editors all take it for a few milliseconds when
/// something is copied, which is ordinary rather than exceptional, so giving
/// up on the first attempt turns an everyday overlap into a copy that silently
/// did nothing. Retrying is Microsoft's own advice for this.
fn open_clipboard(owner: HWND) -> bool {
    use windows::Win32::System::DataExchange::OpenClipboard;
    for attempt in 0..CLIPBOARD_ATTEMPTS {
        if unsafe { OpenClipboard(Some(owner)) }.is_ok() {
            return true;
        }
        // Not after the last one: that would only delay the failure.
        if attempt + 1 < CLIPBOARD_ATTEMPTS {
            std::thread::sleep(CLIPBOARD_RETRY);
        }
    }
    false
}

/// Puts text on the clipboard as Unicode, owned by `owner`.
///
/// That is how the day's plan reaches an email, a ticket or a screen reader —
/// the widget itself, being a non-activatable tool window, is practically
/// unreachable for assistive technology.
///
/// **`owner` must be a real window.** Opening the clipboard with a null handle
/// is what this used to do, and it is documented to leave the clipboard owner
/// null once `EmptyClipboard` has run — which makes `SetClipboardData` fail.
/// The copy then did nothing, intermittently and with nothing written down
/// about why, since every step folded into one `false`. Each step now says
/// which one it was and what Windows called it.
pub fn set_clipboard_text(owner: HWND, text: &str) -> bool {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, SetClipboardData};
    use windows::Win32::System::Memory::{GHND, GlobalAlloc, GlobalLock, GlobalUnlock};
    use windows::Win32::System::Ole::CF_UNICODETEXT;

    let wide_text = wide(text);
    let bytes = wide_text.len() * std::mem::size_of::<u16>();

    if !open_clipboard(owner) {
        log::warn(&format!(
            "Clipboard stayed busy for {} ms — nothing was copied",
            CLIPBOARD_ATTEMPTS as u128 * CLIPBOARD_RETRY.as_millis()
        ));
        return false;
    }
    unsafe {
        let result = (|| {
            EmptyClipboard().map_err(|e| format!("EmptyClipboard: {e}"))?;
            // The clipboard takes ownership of the memory, so it must not be
            // freed here.
            let handle = GlobalAlloc(GHND, bytes).map_err(|e| format!("GlobalAlloc: {e}"))?;
            let target = GlobalLock(handle);
            if target.is_null() {
                return Err("GlobalLock returned nothing".to_owned());
            }
            std::ptr::copy_nonoverlapping(wide_text.as_ptr(), target as *mut u16, wide_text.len());
            let _ = GlobalUnlock(handle);
            SetClipboardData(CF_UNICODETEXT.0 as u32, Some(HANDLE(handle.0)))
                .map_err(|e| format!("SetClipboardData: {e}"))?;
            Ok(())
        })();
        let _ = CloseClipboard();
        match result {
            Ok(()) => true,
            Err(step) => {
                log::warn(&format!("Copying to the clipboard failed at {step}"));
                false
            }
        }
    }
}

/// Splits `"Win+Alt+K"` into modifiers and a virtual key code.
///
/// Deliberately frugal: letters, digits and F1 to F12 cover what anyone
/// realistically picks as a shortcut.
pub fn parse_hotkey(spec: &str) -> Option<(u32, u32)> {
    use windows::Win32::UI::Input::KeyboardAndMouse::{MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN};
    let mut modifiers = 0u32;
    let mut key = None;

    for part in spec.split('+').map(str::trim).filter(|p| !p.is_empty()) {
        match part.to_ascii_lowercase().as_str() {
            "win" | "windows" => modifiers |= MOD_WIN.0,
            "alt" => modifiers |= MOD_ALT.0,
            "ctrl" | "control" | "strg" => modifiers |= MOD_CONTROL.0,
            "shift" | "umschalt" => modifiers |= MOD_SHIFT.0,
            other => {
                let bytes = other.as_bytes();
                key = if bytes.len() == 1 && bytes[0].is_ascii_alphanumeric() {
                    // The virtual key codes for A-Z and 0-9 are the ASCII
                    // values of the upper case letters and digits.
                    Some(bytes[0].to_ascii_uppercase() as u32)
                } else if let Some(number) = other.strip_prefix('f') {
                    // F1 to F12 run consecutively from VK_F1 (0x70).
                    number
                        .parse::<u32>()
                        .ok()
                        .filter(|n| (1..=12).contains(n))
                        .map(|n| 0x6F + n)
                } else {
                    None
                };
            }
        }
    }
    // Without a modifier this would be a global single key, taken away from
    // every other application.
    match (modifiers, key) {
        (0, _) => None,
        (_, Some(k)) => Some((modifiers, k)),
        _ => None,
    }
}

/// Takes the single instance lock. `false` means a widget is already running.
///
/// Without it a second start puts an identical window on top of the first —
/// both draw, both synchronise, and all the user sees is clicks apparently
/// going nowhere.
pub fn acquire_single_instance() -> bool {
    unsafe {
        // "Local\" scopes the lock to the logon session, so every user of a
        // terminal server may have their own widget.
        let name = wide(r"Local\TPMPlaner.SingleInstance");
        match CreateMutexW(None, true, PCWSTR(name.as_ptr())) {
            Ok(handle) => {
                if GetLastError() == ERROR_ALREADY_EXISTS {
                    return false;
                }
                // Deliberately not closed: the lock should last exactly as
                // long as the process. `HANDLE` is a plain numeric value with
                // no drop behaviour, so letting the variable go closes
                // nothing.
                let _ = handle;
                true
            }
            // Start when in doubt: two windows beat none at all.
            Err(_) => true,
        }
    }
}

/// Is the window still on a connected monitor?
///
/// Unplug the monitor the widget sat on and the stored position points into an
/// area that no longer exists — the widget would be invisible and
/// unrecoverable without editing the settings by hand.
pub fn is_on_screen(r: &RECT) -> bool {
    unsafe { MonitorFromRect(r, MONITOR_DEFAULTTONULL) != HMONITOR::default() }
}

/// Work area of the primary monitor, excluding the taskbar.
pub fn primary_work_area() -> Option<RECT> {
    unsafe {
        use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTOPRIMARY};
        // A degenerate rectangle at the origin reliably lands on the primary
        // monitor.
        let origin = RECT {
            left: 0,
            top: 0,
            right: 1,
            bottom: 1,
        };
        let mon = MonitorFromRect(&origin, MONITOR_DEFAULTTOPRIMARY);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        GetMonitorInfoW(mon, &mut info)
            .as_bool()
            .then_some(info.rcWork)
    }
}

/// Returns pages that have fallen out of use to the operating system, after
/// start-up and after every sync.
///
/// A sync briefly allocates a few hundred kilobytes for JSON and TLS buffers;
/// without this hint the peak stays in the working set for the rest of the
/// day. `(usize::MAX, usize::MAX)` is the documented special value for it.
pub fn trim_working_set() {
    unsafe {
        let _ = SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        autostart_enabled, autostart_enabled_in, open_clipboard, parse_hotkey, set_autostart,
        set_autostart_in, set_clipboard_text, wide,
    };
    use windows::Win32::Foundation::HWND;
    use windows::core::PCWSTR;

    /// Autostart is a registry write, so reading the code proves nothing about
    /// whether Windows accepts it. This drives the real key.
    ///
    /// The user's own setting is captured and put back verbatim, including the
    /// case where it was absent — a test must not leave an entry behind that
    /// launches the *test binary* at every logon.
    #[test]
    fn autostart_writes_and_removes_the_real_registry_value() {
        let was_enabled = autostart_enabled();
        let previous = read_run_value();

        set_autostart(true);
        assert!(autostart_enabled(), "the value was not written");
        let written = read_run_value().expect("the value is missing after writing");
        assert!(
            written.starts_with('"') && written.ends_with('"'),
            "the path must be quoted or Windows splits it at spaces: {written}"
        );
        assert!(
            written.trim_matches('"').ends_with(".exe"),
            "an executable path was expected: {written}"
        );

        set_autostart(false);
        assert!(!autostart_enabled(), "the value was not removed");

        // Put the machine back exactly as it was found.
        match previous {
            Some(value) if was_enabled => write_run_value(&value),
            _ => {}
        }
        assert_eq!(
            autostart_enabled(),
            was_enabled,
            "the prior state was not restored"
        );
    }

    /// Reads the autostart value as a string, or `None` when it is absent.
    fn read_run_value() -> Option<String> {
        use windows::Win32::System::Registry::{
            HKEY, HKEY_CURRENT_USER, KEY_READ, RegCloseKey, RegOpenKeyExW, RegQueryValueExW,
        };
        unsafe {
            let mut key = HKEY::default();
            let sub = super::wide(super::RUN_KEY);
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(sub.as_ptr()),
                None,
                KEY_READ,
                &mut key,
            )
            .ok()
            .ok()?;
            let name = super::wide(super::RUN_VALUE);
            let mut size = 0u32;
            let probe = RegQueryValueExW(
                key,
                PCWSTR(name.as_ptr()),
                None,
                None,
                None,
                Some(&mut size),
            );
            let result = if probe.is_ok() {
                let mut buf = vec![0u8; size as usize];
                let ok = RegQueryValueExW(
                    key,
                    PCWSTR(name.as_ptr()),
                    None,
                    None,
                    Some(buf.as_mut_ptr()),
                    Some(&mut size),
                )
                .is_ok();
                ok.then(|| {
                    let units: Vec<u16> = buf
                        .chunks_exact(2)
                        .map(|c| u16::from_le_bytes([c[0], c[1]]))
                        .take_while(|&u| u != 0)
                        .collect();
                    String::from_utf16_lossy(&units)
                })
            } else {
                None
            };
            let _ = RegCloseKey(key);
            result
        }
    }

    /// Restores a captured autostart value verbatim.
    fn write_run_value(value: &str) {
        use windows::Win32::System::Registry::{
            HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_SZ, RegCloseKey, RegOpenKeyExW,
            RegSetValueExW,
        };
        unsafe {
            let mut key = HKEY::default();
            let sub = super::wide(super::RUN_KEY);
            if RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(sub.as_ptr()),
                None,
                KEY_SET_VALUE,
                &mut key,
            )
            .is_err()
            {
                return;
            }
            let name = super::wide(super::RUN_VALUE);
            let wide_value = super::wide(value);
            let bytes = std::slice::from_raw_parts(
                wide_value.as_ptr() as *const u8,
                wide_value.len() * std::mem::size_of::<u16>(),
            );
            let _ = RegSetValueExW(key, PCWSTR(name.as_ptr()), None, REG_SZ, Some(bytes));
            let _ = RegCloseKey(key);
        }
    }

    /// A profile on which nothing has ever registered for autostart does not
    /// have the Run key at all, and writing into a key that is not there
    /// fails — so switching autostart on used to do nothing whatsoever, in
    /// silence. The test above cannot reach that case: emptying the real Run
    /// key would mean deleting whatever the machine already starts at logon.
    /// Hence a scratch key, removed first so it is reliably absent.
    #[test]
    fn autostart_creates_the_key_when_it_is_missing() {
        const SCRATCH: &str = r"Software\TPMPlaner\autostart-create-test";

        delete_key(SCRATCH);
        assert!(
            !autostart_enabled_in(SCRATCH),
            "the scratch key was still there after deleting it"
        );

        set_autostart_in(SCRATCH, true);
        assert!(
            autostart_enabled_in(SCRATCH),
            "the missing key was not created"
        );

        set_autostart_in(SCRATCH, false);
        assert!(
            !autostart_enabled_in(SCRATCH),
            "the value was not removed again"
        );

        delete_key(SCRATCH);
        // Succeeds only while it is empty, which is the wanted behaviour: the
        // parent is not ours to remove once something else lives under it.
        delete_key(r"Software\TPMPlaner");
    }

    /// Removes a key under `HKEY_CURRENT_USER`. A key that is not there is not
    /// an error worth reporting — that is the state being asked for.
    fn delete_key(subkey: &str) {
        use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RegDeleteKeyW};
        unsafe {
            let sub = super::wide(subkey);
            let _ = RegDeleteKeyW(HKEY_CURRENT_USER, PCWSTR(sub.as_ptr()));
        }
    }

    /// A message-only window, to own the clipboard the way the widget does.
    ///
    /// `"STATIC"` is a class the system has already registered, so this needs
    /// no class of its own; `HWND_MESSAGE` keeps it off the screen entirely.
    /// Passing a real window rather than a null handle is the whole point —
    /// with a null one `EmptyClipboard` leaves no owner and `SetClipboardData`
    /// is documented to fail, so a test that passed null would be exercising
    /// the bug rather than the fix.
    fn message_only_window() -> HWND {
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, HWND_MESSAGE, WINDOW_EX_STYLE, WINDOW_STYLE,
        };
        let class = wide("STATIC");
        unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                PCWSTR(class.as_ptr()),
                PCWSTR::null(),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                None,
                None,
            )
        }
        .expect("could not create a message-only window")
    }

    /// Round trip through the real clipboard.
    ///
    /// "Copy agenda" is only reachable from the context menu, which a test
    /// cannot open, so the clipboard call itself is verified here instead —
    /// including that non-ASCII survives, since the day's agenda is full of it.
    ///
    /// Reading back goes through [`open_clipboard`] for the same reason the
    /// writing does: another application holding the clipboard for a moment is
    /// not this test failing, and without the retry it reported one.
    #[test]
    fn text_survives_a_round_trip_through_the_clipboard() {
        use windows::Win32::System::DataExchange::{CloseClipboard, GetClipboardData};
        use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};
        use windows::Win32::System::Ole::CF_UNICODETEXT;
        use windows::Win32::UI::WindowsAndMessaging::DestroyWindow;

        let owner = message_only_window();
        let sample = "Tuesday - 09:00 Sprint Review - überfällig - 予定 - ✓";
        assert!(
            set_clipboard_text(owner, sample),
            "clipboard was not writable"
        );

        assert!(open_clipboard(owner), "could not open the clipboard");
        let read_back = unsafe {
            let handle = GetClipboardData(CF_UNICODETEXT.0 as u32).expect("no text on clipboard");
            let ptr = GlobalLock(windows::Win32::Foundation::HGLOBAL(handle.0)) as *const u16;
            assert!(!ptr.is_null(), "clipboard memory could not be locked");
            let mut len = 0usize;
            while *ptr.add(len) != 0 {
                len += 1;
            }
            let text = String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len));
            let _ = GlobalUnlock(windows::Win32::Foundation::HGLOBAL(handle.0));
            let _ = CloseClipboard();
            text
        };

        unsafe {
            let _ = DestroyWindow(owner);
        }
        assert_eq!(read_back, sample);
    }

    /// Virtual key codes: 'K' is 0x4B, F5 is 0x74.
    #[test]
    fn common_combinations_parse() {
        let (m, k) = parse_hotkey("Win+Alt+K").unwrap();
        assert_eq!(k, 0x4B);
        assert_ne!(m, 0);

        assert_eq!(parse_hotkey("Ctrl+Shift+F5").unwrap().1, 0x74);
        assert_eq!(parse_hotkey("strg+umschalt+7").unwrap().1, 0x37);
    }

    #[test]
    fn spelling_and_spacing_are_forgiving() {
        assert_eq!(parse_hotkey("win + alt + k"), parse_hotkey("WIN+ALT+K"));
        assert_eq!(
            parse_hotkey("Control+Shift+P"),
            parse_hotkey("ctrl+shift+p")
        );
    }

    #[test]
    fn a_bare_key_is_rejected() {
        // Without a modifier the key would be claimed globally and no longer
        // available to any other application.
        assert_eq!(parse_hotkey("K"), None);
        assert_eq!(parse_hotkey("F5"), None);
    }

    #[test]
    fn nonsense_is_rejected_instead_of_guessed() {
        assert_eq!(parse_hotkey(""), None);
        assert_eq!(parse_hotkey("Win+Alt"), None);
        assert_eq!(parse_hotkey("Win+Alt+F13"), None);
        assert_eq!(parse_hotkey("Win+Alt+Ente"), None);
    }
}
