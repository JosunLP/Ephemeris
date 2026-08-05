// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! Kleine Windows-Helfer: Browser oeffnen, Autostart, Speicher trimmen.

use tpmplaner_core::theme::{ContrastColors, SystemVisuals};
use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    HMONITOR, MONITOR_DEFAULTTONULL, MONITORINFO, MonitorFromRect,
};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, REG_DWORD, REG_SZ, RegCloseKey,
    RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
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
    unsafe {
        let mut key = HKEY::default();
        let sub = wide(RUN_KEY);
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
    unsafe {
        let mut key = HKEY::default();
        let sub = wide(RUN_KEY);
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

/// Puts text on the clipboard as Unicode.
///
/// That is how the day's plan reaches an email, a ticket or a screen reader —
/// the widget itself, being a non-activatable tool window, is practically
/// unreachable for assistive technology.
pub fn set_clipboard_text(text: &str) -> bool {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
    };
    use windows::Win32::System::Memory::{GHND, GlobalAlloc, GlobalLock, GlobalUnlock};
    use windows::Win32::System::Ole::CF_UNICODETEXT;

    let wide_text = wide(text);
    let bytes = wide_text.len() * std::mem::size_of::<u16>();

    unsafe {
        if OpenClipboard(None).is_err() {
            return false;
        }
        let result = (|| {
            EmptyClipboard().ok()?;
            // The clipboard takes ownership of the memory, so it must not be
            // freed here.
            let handle = GlobalAlloc(GHND, bytes).ok()?;
            let target = GlobalLock(handle);
            if target.is_null() {
                return None;
            }
            std::ptr::copy_nonoverlapping(wide_text.as_ptr(), target as *mut u16, wide_text.len());
            let _ = GlobalUnlock(handle);
            SetClipboardData(CF_UNICODETEXT.0 as u32, Some(HANDLE(handle.0))).ok()?;
            Some(())
        })();
        let _ = CloseClipboard();
        result.is_some()
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
    use super::parse_hotkey;

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
