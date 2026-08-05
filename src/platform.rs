// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! Kleine Windows-Helfer: Browser oeffnen, Autostart, Speicher trimmen.

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

/// UTF-16 mit abschliessender Null, wie es die Win32-`*W`-Funktionen wollen.
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

/// Oeffnet einen Pfad im zugeordneten Programm (Konfig-Datei, Datenordner).
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
            // Pfad in Anfuehrungszeichen, sonst zerlegt Windows ihn an
            // Leerzeichen ("C:\Program Files\...").
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

/// Liest einen DWORD-Wert unter HKEY_CURRENT_USER.
fn read_dword(subkey: &str, value: &str) -> Option<u32> {
    unsafe {
        let mut key = HKEY::default();
        let sub = wide(subkey);
        // `RegOpenKeyExW` liefert `WIN32_ERROR`, kein `Result` — `?` ist hier
        // nicht anwendbar.
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

/// Nutzt Windows gerade das helle App-Design?
///
/// `AppsUseLightTheme` ist der Wert, den auch die Systemanwendungen auswerten;
/// `SystemUsesLightTheme` betrifft nur Taskleiste und Startmenue und waere
/// hier das falsche Signal.
pub fn system_uses_light_theme() -> bool {
    read_dword(PERSONALIZE_KEY, "AppsUseLightTheme") == Some(1)
}

/// Systemakzentfarbe als `0xRRGGBB`.
///
/// `DwmGetColorizationColor` liefert `0xAARRGGBB`; der Alphaanteil beschreibt
/// die Glasmischung des Fensterrahmens und ist fuer uns bedeutungslos.
pub fn system_accent() -> Option<u32> {
    unsafe {
        use windows::Win32::Graphics::Dwm::DwmGetColorizationColor;
        let mut color: u32 = 0;
        let mut opaque = windows::core::BOOL::default();
        DwmGetColorizationColor(&mut color, &mut opaque).ok()?;
        Some(color & 0x00FF_FFFF)
    }
}

/// Darstellungsbezogene Windows-Einstellungen.
///
/// Alle vier sind Barrierefreiheits- bzw. Personalisierungsschalter, die eine
/// Anwendung respektieren muss, damit sie sich in das System einfuegt statt
/// ihre eigene Optik durchzusetzen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemVisuals {
    /// Kontrastdesign aktiv. Dann gelten ausschliesslich die Systemfarben,
    /// und Transparenz, Verlaeufe und Schatten sind unerwuenscht — sie
    /// senken genau den Kontrast, den der Modus herstellen soll.
    pub high_contrast: bool,
    /// "Transparenzeffekte" in Einstellungen → Personalisierung → Farben.
    pub transparency: bool,
    /// "Animationseffekte in Windows anzeigen". Aus = keine Bewegung.
    pub animations: bool,
    pub light: bool,
    /// Systemakzentfarbe als `0xRRGGBB`.
    pub accent: Option<u32>,
}

pub fn system_visuals() -> SystemVisuals {
    SystemVisuals {
        high_contrast: high_contrast_active(),
        // Fehlender Wert bedeutet "eingeschaltet" — so ist es ausgeliefert.
        transparency: read_dword(PERSONALIZE_KEY, "EnableTransparency") != Some(0),
        animations: client_area_animation(),
        light: system_uses_light_theme(),
        accent: system_accent(),
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
        // Im Zweifel animieren — der Schalter fehlt auf aelteren Systemen.
        !ok || enabled.as_bool()
    }
}

/// Eine Systemfarbe als `0xRRGGBB`.
///
/// `GetSysColor` liefert ein COLORREF, also `0x00BBGGRR` — Rot und Blau sind
/// gegenueber der hier ueblichen Schreibweise vertauscht.
pub fn sys_color(index: windows::Win32::Graphics::Gdi::SYS_COLOR_INDEX) -> u32 {
    let bgr = unsafe { windows::Win32::Graphics::Gdi::GetSysColor(index) };
    let (r, g, b) = (bgr & 0xFF, (bgr >> 8) & 0xFF, (bgr >> 16) & 0xFF);
    (r << 16) | (g << 8) | b
}

/// Legt Text als Unicode in die Zwischenablage.
///
/// Damit laesst sich der Tagesplan in eine Mail, ein Ticket oder einen
/// Vorleser uebernehmen — das Widget selbst ist als nicht aktivierbares
/// Werkzeugfenster fuer Bildschirmleser praktisch unerreichbar.
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
            // Die Zwischenablage uebernimmt den Speicher; er darf deshalb
            // nicht wieder freigegeben werden.
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

/// Zerlegt `"Win+Alt+K"` in Modifizierer und virtuellen Tastencode.
///
/// Bewusst genuegsam: Buchstaben, Ziffern und F1-F12 decken ab, was jemand
/// realistisch als Kurzbefehl waehlt.
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
                    // Virtuelle Tastencodes fuer A-Z und 0-9 entsprechen den
                    // ASCII-Werten der Grossbuchstaben bzw. Ziffern.
                    Some(bytes[0].to_ascii_uppercase() as u32)
                } else if let Some(number) = other.strip_prefix('f') {
                    // F1 bis F12 liegen ab VK_F1 (0x70) fortlaufend.
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
    // Ohne Modifizierer waere es eine globale Einzeltaste — die wuerde sie
    // jeder anderen Anwendung wegnehmen.
    match (modifiers, key) {
        (0, _) => None,
        (_, Some(k)) => Some((modifiers, k)),
        _ => None,
    }
}

/// Belegt die Einzelinstanz-Sperre. `false` = es laeuft bereits ein Widget.
///
/// Ohne diese Sperre legt ein zweiter Start ein deckungsgleiches Fenster auf
/// das erste — beide zeichnen, beide synchronisieren, und der Benutzer sieht
/// nur, dass Klicks scheinbar ins Leere gehen.
pub fn acquire_single_instance() -> bool {
    unsafe {
        // "Local\" = pro Anmeldesitzung. Auf einem Terminalserver darf jeder
        // Benutzer sein eigenes Widget haben.
        let name = wide(r"Local\TPMPlaner.SingleInstance");
        match CreateMutexW(None, true, PCWSTR(name.as_ptr())) {
            Ok(handle) => {
                if GetLastError() == ERROR_ALREADY_EXISTS {
                    return false;
                }
                // Absichtlich nicht geschlossen: die Sperre soll exakt so lange
                // gelten wie der Prozess lebt. `HANDLE` ist ein reiner
                // Zahlenwert ohne Drop-Verhalten, das Fallenlassen der
                // Variable schliesst also nichts.
                let _ = handle;
                true
            }
            // Im Zweifel starten — lieber zwei Fenster als gar keines.
            Err(_) => true,
        }
    }
}

/// Liegt das Fenster noch auf einem angeschlossenen Monitor?
///
/// Wird der Monitor abgezogen, auf dem das Widget stand, bleibt die
/// gespeicherte Position in einem Bereich, den es nicht mehr gibt — das Widget
/// waere unsichtbar und ohne Registry-Eingriff nicht zurueckzuholen.
pub fn is_on_screen(r: &RECT) -> bool {
    unsafe { MonitorFromRect(r, MONITOR_DEFAULTTONULL) != HMONITOR::default() }
}

/// Arbeitsbereich des Primaermonitors (ohne Taskleiste).
pub fn primary_work_area() -> Option<RECT> {
    unsafe {
        use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTOPRIMARY};
        // Ein entartetes Rechteck am Ursprung landet zuverlaessig auf dem
        // Primaermonitor.
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

/// Gibt nach dem Start und nach jedem Sync die inzwischen unbenutzten Seiten
/// ans Betriebssystem zurueck.
///
/// Ein Sync alloziert kurzzeitig ein paar hundert Kilobyte fuer JSON und
/// TLS-Puffer; ohne diesen Hinweis bleibt der Peak fuer den Rest des Tages im
/// Working Set stehen. `(usize::MAX, usize::MAX)` ist der dokumentierte
/// Sonderwert dafuer.
pub fn trim_working_set() {
    unsafe {
        let _ = SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
    }
}

#[cfg(test)]
mod tests {
    use super::parse_hotkey;

    /// Virtuelle Tastencodes: 'K' = 0x4B, F5 = 0x74.
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
        // Ohne Modifizierer wuerde die Taste global belegt und stuende keiner
        // anderen Anwendung mehr zur Verfuegung.
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
