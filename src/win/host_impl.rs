// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! The Windows answers to the portable core's questions.
//!
//! `tpmplaner-core` deliberately cannot call an operating system API. The
//! handful of things that genuinely differ per platform — where settings
//! live, how a secret is stored, how a browser opens, how a locale formats a
//! date, how a background thread wakes the window — are declared there as
//! traits and implemented here.
//!
//! A macOS or Linux front end supplies its own versions of exactly these; the
//! core never learns which one it received.

use chrono::{DateTime, Datelike, Local, NaiveDate, Timelike};
use std::path::PathBuf;
use tpmplaner_core::host::{Host, LocaleBackend, Waker};
use windows::Win32::Foundation::SYSTEMTIME;
use windows::Win32::Globalization::{
    DATE_LONGDATE, ENUM_DATE_FORMATS_FLAGS, GetDateFormatEx, GetLocaleInfoEx, GetTimeFormatEx,
    GetUserDefaultLocaleName, LOCALE_IREADINGLAYOUT, LOCALE_RETURN_NUMBER, LOCALE_SLONGDATE,
    TIME_NOSECONDS,
};
use windows::core::PCWSTR;

use crate::win::platform::wide;

pub struct WindowsHost;

impl Host for WindowsHost {
    fn data_dir(&self) -> PathBuf {
        match std::env::var_os("APPDATA") {
            Some(v) => PathBuf::from(v).join("TPMPlaner"),
            None => PathBuf::from("."),
        }
    }

    fn open_url(&self, url: &str) {
        crate::win::platform::open_in_browser(url);
    }

    fn protect(&self, plain: &[u8], tag: &[u8]) -> Option<Vec<u8>> {
        crate::win::secure::protect(plain, tag)
    }

    fn unprotect(&self, cipher: &[u8], tag: &[u8]) -> Option<Vec<u8>> {
        crate::win::secure::unprotect(cipher, tag)
    }

    fn random_bytes(&self, len: usize) -> Option<Vec<u8>> {
        crate::win::secure::random_bytes(len)
    }
}

/// Date and time formatting through the Windows NLS database.
///
/// This is what keeps the widget correct in locales that have no text
/// catalogue: the system knows, for every locale it ships, the field order of
/// a date, the local month names and whether the clock counts to twelve or to
/// twenty-four.
pub struct WindowsLocale;

impl LocaleBackend for WindowsLocale {
    fn user_default_tag(&self) -> String {
        // LOCALE_NAME_MAX_LENGTH is 85.
        let mut buf = [0u16; 85];
        let len = unsafe { GetUserDefaultLocaleName(&mut buf) };
        from_wide(&buf, len).unwrap_or_else(|| "en-US".to_string())
    }

    fn is_rtl(&self, tag: &str) -> bool {
        let name = wide(tag);
        let mut value: u32 = 0;
        let ok = unsafe {
            GetLocaleInfoEx(
                PCWSTR(name.as_ptr()),
                LOCALE_IREADINGLAYOUT | LOCALE_RETURN_NUMBER,
                // With LOCALE_RETURN_NUMBER the buffer is read as a DWORD.
                Some(std::slice::from_raw_parts_mut(
                    &mut value as *mut u32 as *mut u16,
                    2,
                )),
            )
        };
        ok > 0 && value == 1
    }

    fn format_time(&self, tag: &str, dt: DateTime<Local>) -> Option<String> {
        let st = systemtime_of(dt);
        let name = wide(tag);
        let mut buf = [0u16; 96];
        let len = unsafe {
            GetTimeFormatEx(
                PCWSTR(name.as_ptr()),
                TIME_NOSECONDS,
                Some(&st),
                PCWSTR::null(),
                Some(buf.as_mut_slice()),
            )
        };
        from_wide(&buf, len)
    }

    fn format_weekday(&self, tag: &str, date: NaiveDate) -> Option<String> {
        format_with(tag, date, Some(&wide("dddd")))
    }

    fn format_date(&self, tag: &str, date: NaiveDate) -> Option<String> {
        // The locale's long date pattern with the weekday removed — that one
        // already sits on the line above. Editing the pattern beats inventing
        // one per language, because the order of day, month and year stays the
        // locale's own.
        let pattern = long_date_without_weekday(tag);
        format_with(
            tag,
            date,
            (!pattern.is_empty()).then_some(pattern.as_slice()),
        )
    }

    fn format_day_month(&self, tag: &str, date: NaiveDate) -> Option<String> {
        format_with(tag, date, Some(&wide("d MMM")))
    }
}

fn long_date_without_weekday(tag: &str) -> Vec<u16> {
    let name = wide(tag);
    let mut buf = [0u16; 128];
    let len = unsafe {
        GetLocaleInfoEx(
            PCWSTR(name.as_ptr()),
            LOCALE_SLONGDATE,
            Some(buf.as_mut_slice()),
        )
    };
    let Some(pattern) = from_wide(&buf, len) else {
        return Vec::new();
    };

    let mut cleaned = pattern.replace("dddd", "");
    // Separators left behind at the edges, including the Arabic comma and the
    // ideographic comma.
    let trim: &[char] = &[' ', ',', '\u{060C}', '\u{3001}', '.', '-', '/'];
    cleaned = cleaned.trim_matches(trim).to_string();
    while cleaned.contains("  ") {
        cleaned = cleaned.replace("  ", " ");
    }
    if cleaned.is_empty() {
        Vec::new()
    } else {
        wide(&cleaned)
    }
}

fn format_with(tag: &str, date: NaiveDate, pattern: Option<&[u16]>) -> Option<String> {
    let st = systemtime_of_date(date);
    let name = wide(tag);
    let mut buf = [0u16; 160];
    let fmt = match pattern {
        Some(p) => PCWSTR(p.as_ptr()),
        None => PCWSTR::null(),
    };
    // A custom pattern and DATE_LONGDATE are mutually exclusive.
    let flags = if pattern.is_some() {
        ENUM_DATE_FORMATS_FLAGS(0)
    } else {
        DATE_LONGDATE
    };
    let len = unsafe {
        GetDateFormatEx(
            PCWSTR(name.as_ptr()),
            flags,
            Some(&st),
            fmt,
            Some(buf.as_mut_slice()),
            PCWSTR::null(),
        )
    };
    from_wide(&buf, len)
}

/// The `Get*FormatEx` functions count the terminating null.
fn from_wide(buf: &[u16], len: i32) -> Option<String> {
    if len <= 0 {
        return None;
    }
    let n = (len as usize).saturating_sub(1).min(buf.len());
    let s = String::from_utf16_lossy(&buf[..n]);
    (!s.trim().is_empty()).then_some(s)
}

fn systemtime_of(dt: DateTime<Local>) -> SYSTEMTIME {
    SYSTEMTIME {
        wYear: dt.year() as u16,
        wMonth: dt.month() as u16,
        wDayOfWeek: dt.weekday().num_days_from_sunday() as u16,
        wDay: dt.day() as u16,
        wHour: dt.hour() as u16,
        wMinute: dt.minute() as u16,
        wSecond: dt.second() as u16,
        wMilliseconds: 0,
    }
}

fn systemtime_of_date(d: NaiveDate) -> SYSTEMTIME {
    SYSTEMTIME {
        wYear: d.year() as u16,
        wMonth: d.month() as u16,
        wDayOfWeek: d.weekday().num_days_from_sunday() as u16,
        wDay: d.day() as u16,
        ..Default::default()
    }
}

/// Wakes the window from the sync thread.
///
/// The handle travels as an `isize` because `HWND` is not `Send`; it is only
/// ever used for `PostMessage`, which is thread safe by contract.
pub struct WindowWaker {
    pub hwnd: isize,
    pub message: u32,
}

impl Waker for WindowWaker {
    fn wake(&self) {
        use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
        use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
        unsafe {
            let _ = PostMessageW(
                Some(HWND(self.hwnd as *mut _)),
                self.message,
                WPARAM(0),
                LPARAM(0),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    /// These assertions are the reason the NLS backend exists at all; they
    /// only hold where the Windows locale database is present, which is why
    /// they live here and not in the portable core.
    #[test]
    fn twelve_and_twentyfour_hour_clocks_both_come_from_the_system() {
        let dt = Local.with_ymd_and_hms(2026, 8, 4, 20, 9, 0).unwrap();
        let de = WindowsLocale.format_time("de-DE", dt).expect("de-DE");
        let us = WindowsLocale.format_time("en-US", dt).expect("en-US");
        assert!(de.contains("20"), "de-DE: {de}");
        assert!(
            us.contains('8') && us.to_ascii_uppercase().contains("PM"),
            "en-US: {us}"
        );
    }

    #[test]
    fn date_field_order_follows_the_locale() {
        let d = NaiveDate::from_ymd_opt(2026, 8, 4).unwrap();
        let de = WindowsLocale.format_date("de-DE", d).expect("de-DE");
        let us = WindowsLocale.format_date("en-US", d).expect("en-US");
        assert!(de.starts_with('4'), "de-DE: {de}");
        assert!(us.starts_with("August"), "en-US: {us}");
        // The weekday belongs on the line above and must be absent here.
        assert!(!de.contains("Dienstag"), "de-DE: {de}");
        assert!(!us.contains("Tuesday"), "en-US: {us}");
    }

    #[test]
    fn weekday_names_are_localised() {
        let d = NaiveDate::from_ymd_opt(2026, 8, 4).unwrap();
        assert_eq!(
            WindowsLocale.format_weekday("de-DE", d).as_deref(),
            Some("Dienstag")
        );
        assert_eq!(
            WindowsLocale.format_weekday("en-US", d).as_deref(),
            Some("Tuesday")
        );
    }

    #[test]
    fn reading_direction_comes_from_the_locale_database() {
        assert!(!WindowsLocale.is_rtl("de-DE"));
        assert!(!WindowsLocale.is_rtl("en-US"));
        assert!(
            WindowsLocale.is_rtl("ar-SA"),
            "Arabic must be right to left"
        );
        assert!(
            WindowsLocale.is_rtl("he-IL"),
            "Hebrew must be right to left"
        );
    }
}
