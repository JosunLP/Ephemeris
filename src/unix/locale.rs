// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! [`LocaleBackend`] for macOS and Linux.
//!
//! The design note in `i18n.rs` asks for the platform's own locale database
//! rather than a pattern written by hand, and it is right to: twelve- against
//! twenty-four-hour counting is the setting people notice immediately, month
//! names decline in half the languages the widget ships, and field order is
//! not guessable from a language tag. What replaced [`PortableLocale`] here is
//! the platform's answer on both systems.
//!
//! **macOS** uses Core Foundation's `CFDateFormatter`, driven by
//! `CFDateFormatterCreateDateFormatFromTemplate` — the C form of the
//! `dateFormatFromTemplate:` the porting notes named, and the same CLDR data
//! `NSDateFormatter` sits on top of. Core Foundation rather than the
//! Objective-C class for one reason: it is a plain C API, so it needs no
//! message-send machinery and no additional crate.
//!
//! **Linux** goes through the C library: `newlocale`/`uselocale` to select a
//! locale for this thread alone, `nl_langinfo` for the locale's own date and
//! time patterns, and `strftime` to render them. Not `setlocale`, which would
//! change the process — the sync threads share it, and a formatter that
//! reaches sideways into other threads is a bug waiting for a slow calendar
//! server.
//!
//! **What Linux gets right and what it does not.** The hour convention, the
//! weekday and month names, the field order and the locale's own separators
//! are all the C library's, taken from its patterns rather than reconstructed
//! — so `%Y年%m月%d日` stays that shape rather than being flattened into
//! something European. What is genuinely approximate is the *long* date: POSIX
//! has no long-date pattern, so this widens the short one, and a locale whose
//! long form differs by more than the month's width will be close rather than
//! exact. `icu4x` is the fix and the porting notes carry the open measurement;
//! this is a large step from "English month names and a hard 24-hour clock in
//! every locale", which is what the portable fallback does.
//!
//! Everything that can be decided without the platform — which POSIX locale
//! names to try for a BCP-47 tag, reading an hour convention out of a pattern,
//! widening a short date, dropping the year from one — is ordinary code below
//! with ordinary tests, which run on both systems continuous integration
//! builds this front end on. What is left unsafe is a thin shell around it.
//!
//! [`PortableLocale`]: tpmplaner_core::host::PortableLocale

use chrono::{DateTime, Local, NaiveDate};
use tpmplaner_core::host::{LocaleBackend, PortableLocale};

pub struct UnixLocale;

impl LocaleBackend for UnixLocale {
    fn user_default_tag(&self) -> String {
        platform::user_default_tag().unwrap_or_else(|| PortableLocale.user_default_tag())
    }

    /// Delegated on purpose. Which languages are written right to left is a
    /// property of the languages, not of the operating system, and the list is
    /// already in the core with its own test.
    fn is_rtl(&self, tag: &str) -> bool {
        PortableLocale.is_rtl(tag)
    }

    fn format_time(&self, tag: &str, dt: DateTime<Local>) -> Option<String> {
        platform::format_time(tag, dt)
    }

    fn format_weekday(&self, tag: &str, date: NaiveDate) -> Option<String> {
        platform::format_weekday(tag, date)
    }

    fn format_date(&self, tag: &str, date: NaiveDate) -> Option<String> {
        platform::format_date(tag, date)
    }

    fn format_day_month(&self, tag: &str, date: NaiveDate) -> Option<String> {
        platform::format_day_month(tag, date)
    }
}

/// Midday on a date, as Unix seconds.
///
/// Midday rather than midnight: the formatter renders in the system's time
/// zone, and a date pinned to its own midnight lands on the previous day in any
/// zone behind the one the timestamp was built in — and on the day a clock goes
/// forward, midnight is a time that did not happen.
#[cfg(target_os = "macos")]
fn noon(date: NaiveDate) -> Option<i64> {
    use chrono::TimeZone;
    let naive = date.and_hms_opt(12, 0, 0)?;
    Local
        .from_local_datetime(&naive)
        .earliest()
        .map(|dt| dt.timestamp())
}

// --- Reading and reshaping what a C library reports --------------------------
//
// Only the C library back end needs any of this — macOS asks Core Foundation
// for a pattern and never sees a `strftime` one — so it lives with that back
// end rather than above both. It is ordinary code with ordinary tests all the
// same: everything below this line is decided by string handling, and the
// unsafe part of the module is the thin shell that calls it.

#[cfg(not(target_os = "macos"))]
mod patterns {
    /// POSIX locale names to try for a BCP-47 tag, most specific first.
    ///
    /// `de-DE` becomes `de_DE.UTF-8`, then `de_DE.utf8` — Debian and Fedora spell
    /// the codeset differently in `locale -a` — then `de_DE`, then `de`. A script
    /// subtag is dropped: `zh-Hant-TW` is `zh_TW` to the C library, which has no
    /// notion of a script.
    ///
    /// Returning several is the point. Which locales exist is a property of the
    /// machine, not of the tag, and a container with only `C.UTF-8` generated must
    /// fail over to the portable default rather than to a wrong language.
    pub(super) fn posix_candidates(tag: &str) -> Vec<String> {
        let mut parts = tag.split(['-', '_']).filter(|p| !p.is_empty());
        let Some(language) = parts.next() else {
            return Vec::new();
        };
        let language = language.to_ascii_lowercase();
        if language == "c" || language == "posix" {
            return Vec::new();
        }
        // The first two-or-three-letter subtag after the language that is not a
        // four-letter script: that is the territory.
        let territory = parts
            .find(|p| p.len() == 2 || p.len() == 3)
            .map(|p| p.to_ascii_uppercase());

        let base = match &territory {
            Some(t) => format!("{language}_{t}"),
            None => language.clone(),
        };
        let mut out = vec![
            format!("{base}.UTF-8"),
            format!("{base}.utf8"),
            base.clone(),
        ];
        if base != language {
            out.push(format!("{language}.UTF-8"));
            out.push(language);
        }
        out
    }

    /// Does this locale count in twelve hours?
    ///
    /// Read out of the locale's own `T_FMT`, which is the only reliable source:
    /// `%I` and `%l` are the twelve-hour hour, `%H` and `%k` the twenty-four-hour
    /// one. Guessing from the language is exactly the mistake `i18n.rs` warns
    /// about — `en-GB` is a twenty-four-hour locale and `en-US` is not.
    pub(super) fn is_twelve_hour(t_fmt: &str) -> bool {
        specifiers(t_fmt).any(|c| c == 'I' || c == 'l')
    }

    /// The conversion specifiers in a `strftime` pattern, in order.
    ///
    /// `%%` is a literal per cent and carries no specifier, so it is stepped over
    /// rather than read as one. `E` and `O` are modifiers — `%Ex`, `%Od` — and the
    /// letter after them is the specifier.
    fn specifiers(fmt: &str) -> impl Iterator<Item = char> + '_ {
        let mut chars = fmt.chars();
        std::iter::from_fn(move || {
            loop {
                if chars.next()? != '%' {
                    continue;
                }
                let mut c = chars.next()?;
                if c == '%' {
                    continue;
                }
                if c == 'E' || c == 'O' {
                    c = chars.next()?;
                }
                return Some(c);
            }
        })
    }

    /// A `strftime` pattern split into literal runs and conversions, so a field can
    /// be replaced or removed without disturbing the locale's own punctuation.
    #[derive(Debug, PartialEq)]
    enum Token {
        /// The whole conversion including the leading `%` and any modifier, and
        /// the specifier letter on its own.
        Conversion(String, char),
        Literal(String),
    }

    fn tokenize(fmt: &str) -> Vec<Token> {
        let mut out = Vec::new();
        let mut literal = String::new();
        let mut chars = fmt.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '%' {
                literal.push(c);
                continue;
            }
            let Some(&next) = chars.peek() else {
                literal.push('%');
                break;
            };
            if next == '%' {
                chars.next();
                literal.push('%');
                continue;
            }
            let mut conversion = String::from('%');
            let mut spec = chars.next().unwrap_or('%');
            conversion.push(spec);
            if (spec == 'E' || spec == 'O')
                && let Some(&after) = chars.peek()
            {
                chars.next();
                conversion.push(after);
                spec = after;
            }
            if !literal.is_empty() {
                out.push(Token::Literal(std::mem::take(&mut literal)));
            }
            out.push(Token::Conversion(conversion, spec));
        }
        if !literal.is_empty() {
            out.push(Token::Literal(literal));
        }
        out
    }

    fn is_year(spec: char) -> bool {
        matches!(spec, 'Y' | 'y' | 'C' | 'G' | 'g')
    }

    fn is_month(spec: char) -> bool {
        matches!(spec, 'm' | 'b' | 'B' | 'h')
    }

    fn is_weekday(spec: char) -> bool {
        matches!(spec, 'a' | 'A' | 'u' | 'w')
    }

    /// Widens a locale's short date into a long one.
    ///
    /// The numeric month becomes the full name and a two-digit year becomes four,
    /// while the field order and every separator the locale chose stay exactly as
    /// they were — which is what keeps `%Y年%m月%d日` the right shape instead of a
    /// European reconstruction of it.
    ///
    /// A weekday in the pattern is dropped: the widget draws the weekday on the
    /// line above, and this is the line under it.
    pub(super) fn long_date_pattern(d_fmt: &str) -> String {
        let tokens = tokenize(d_fmt);
        let mut out = String::new();
        let mut pending_literal: Option<String> = None;
        let mut written_any = false;

        for token in tokens {
            match token {
                Token::Literal(text) => pending_literal = Some(text),
                Token::Conversion(raw, spec) => {
                    if is_weekday(spec) {
                        // Drop the separator that came with it as well, or the
                        // line starts with a stray comma.
                        pending_literal = None;
                        continue;
                    }
                    if let Some(text) = pending_literal.take()
                        && written_any
                    {
                        out.push_str(&text);
                    }
                    match spec {
                        'm' | 'b' | 'h' => out.push_str("%B"),
                        'y' | 'C' | 'g' => out.push_str("%Y"),
                        _ => out.push_str(&raw),
                    }
                    written_any = true;
                }
            }
        }
        // A trailing literal belongs to the last field — `%Y年%m月%d日` ends in one.
        if let Some(text) = pending_literal
            && written_any
        {
            out.push_str(&text);
        }
        if out.is_empty() {
            "%e %B %Y".into()
        } else {
            out
        }
    }

    /// The same short date with the year taken out, for the task due column.
    ///
    /// The separator that went with the year goes too, so `%d.%m.%Y` becomes
    /// `%d.%b` rather than `%d.%b.`, and `%Y年%m月%d日` becomes `%m月%d日`. The
    /// month is abbreviated: the column is narrow and the year is the field that
    /// carries no information in a list of things due this week.
    pub(super) fn day_month_pattern(d_fmt: &str) -> String {
        let mut kept: Vec<Token> = Vec::new();
        for token in tokenize(d_fmt) {
            match token {
                Token::Conversion(_, spec) if is_year(spec) || is_weekday(spec) => {
                    // The separator that introduced the field goes with it. When
                    // the field came first there is none, and the separator that
                    // follows is dropped by the leading-punctuation trim below.
                    if matches!(kept.last(), Some(Token::Literal(_))) {
                        kept.pop();
                    }
                }
                other => kept.push(other),
            }
        }

        let mut out = String::new();
        let mut started = false;
        for token in &kept {
            match token {
                // Punctuation before any field has been written is what the
                // dropped leading field left behind.
                Token::Literal(_) if !started => {}
                Token::Literal(text) => out.push_str(text),
                Token::Conversion(raw, spec) => {
                    out.push_str(if is_month(*spec) { "%b" } else { raw });
                    started = true;
                }
            }
        }
        if out.is_empty() { "%e %b".into() } else { out }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn a_language_tag_becomes_the_locale_names_a_c_library_would_know() {
            assert_eq!(
                posix_candidates("de-DE"),
                vec!["de_DE.UTF-8", "de_DE.utf8", "de_DE", "de.UTF-8", "de"]
            );
            // Underscores are accepted as readily as hyphens: the tag may have
            // come back out of the environment.
            assert_eq!(posix_candidates("pt_BR")[0], "pt_BR.UTF-8");
            // A script subtag means nothing to a C library and is dropped, but the
            // territory after it is not.
            assert_eq!(posix_candidates("zh-Hant-TW")[0], "zh_TW.UTF-8");
            // A bare language has no territory to guess.
            assert_eq!(posix_candidates("fr"), vec!["fr.UTF-8", "fr.utf8", "fr"]);
            // Case is normalised, since a tag can arrive either way.
            assert_eq!(posix_candidates("EN-gb")[0], "en_GB.UTF-8");
        }

        #[test]
        fn the_c_locale_is_not_a_language_and_produces_no_candidates() {
            // Falling through to the portable default is right here. Formatting a
            // date "in C" would be an English date presented as the user's choice.
            assert!(posix_candidates("C").is_empty());
            assert!(posix_candidates("POSIX").is_empty());
            assert!(posix_candidates("").is_empty());
        }

        #[test]
        fn the_hour_convention_comes_from_the_pattern_and_not_from_the_language() {
            // en_US and en_GB share a language and disagree about this, which is
            // why it may not be guessed.
            assert!(is_twelve_hour("%I:%M:%S %p"));
            assert!(is_twelve_hour("%l:%M:%S %P"));
            assert!(!is_twelve_hour("%H:%M:%S"));
            assert!(!is_twelve_hour("%k:%M"));
            // A literal per cent is not a conversion.
            assert!(!is_twelve_hour("%H%%I"));
        }

        #[test]
        fn a_pattern_splits_into_fields_and_the_punctuation_between_them() {
            assert_eq!(
                tokenize("%d.%m.%Y"),
                vec![
                    Token::Conversion("%d".into(), 'd'),
                    Token::Literal(".".into()),
                    Token::Conversion("%m".into(), 'm'),
                    Token::Literal(".".into()),
                    Token::Conversion("%Y".into(), 'Y'),
                ]
            );
            // A modifier belongs to the conversion it precedes.
            assert_eq!(
                tokenize("%Oe %EY"),
                vec![
                    Token::Conversion("%Oe".into(), 'e'),
                    Token::Literal(" ".into()),
                    Token::Conversion("%EY".into(), 'Y'),
                ]
            );
        }

        #[test]
        fn widening_a_short_date_keeps_the_locales_order_and_separators() {
            assert_eq!(long_date_pattern("%d.%m.%Y"), "%d.%B.%Y");
            assert_eq!(long_date_pattern("%m/%d/%y"), "%B/%d/%Y");
            // The shape a CJK locale writes is preserved rather than rebuilt as a
            // European one — this is what reconstructing from a field order would
            // have lost.
            assert_eq!(long_date_pattern("%Y年%m月%d日"), "%Y年%B月%d日");
        }

        #[test]
        fn a_weekday_in_the_short_date_is_dropped_with_its_separator() {
            // The widget draws the weekday on the line above this one, and a
            // pattern that kept it would start with a stray comma once it went.
            assert_eq!(long_date_pattern("%a, %d %b %Y"), "%d %B %Y");
            assert_eq!(day_month_pattern("%a %d/%m/%Y"), "%d/%b");
        }

        #[test]
        fn the_due_column_loses_the_year_and_the_separator_that_came_with_it() {
            assert_eq!(day_month_pattern("%d.%m.%Y"), "%d.%b");
            assert_eq!(day_month_pattern("%d/%m/%Y"), "%d/%b");
            // Year first: the separator that follows it goes instead of one that
            // precedes it, so nothing starts with a stray mark.
            assert_eq!(day_month_pattern("%Y-%m-%d"), "%b-%d");
            assert_eq!(day_month_pattern("%Y年%m月%d日"), "%b月%d日");
            assert_eq!(day_month_pattern("%m/%d/%y"), "%b/%d");
        }

        #[test]
        fn an_unusable_pattern_still_yields_something_drawable() {
            // A locale with no date pattern at all, or one made of nothing this
            // understands, must not produce an empty line where the date goes.
            assert_eq!(long_date_pattern(""), "%e %B %Y");
            assert_eq!(day_month_pattern(""), "%e %b");
            assert_eq!(day_month_pattern("%Y"), "%e %b");
        }
    }
}

// --- macOS -------------------------------------------------------------------

#[cfg(target_os = "macos")]
mod platform {
    //! Core Foundation's `CFDateFormatter`, driven by skeleton patterns.
    //!
    //! A *skeleton* — `jm`, `yMMMMd` — names the fields wanted and lets
    //! `CFDateFormatterCreateDateFormatFromTemplate` produce the pattern this
    //! locale actually writes them in, separators and all. `j` in particular is
    //! "the hour field this locale prefers", which is the twelve-against-
    //! twenty-four-hour question answered by the system rather than by us.

    use super::*;
    use crate::unix::cf::{self, CFAbsoluteTime, CFIndex, CFOptionFlags, CFStringRef, CFTypeRef};

    type CFLocaleRef = CFTypeRef;
    type CFDateFormatterRef = CFTypeRef;

    /// `kCFDateFormatterNoStyle`. Which style is irrelevant — every formatter
    /// here has its pattern set explicitly straight afterwards — but one has to
    /// be passed.
    const NO_STYLE: CFIndex = 0;

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFLocaleCopyCurrent() -> CFLocaleRef;
        fn CFLocaleCreate(alloc: CFTypeRef, identifier: CFStringRef) -> CFLocaleRef;
        fn CFLocaleGetIdentifier(locale: CFLocaleRef) -> CFStringRef;
        fn CFDateFormatterCreate(
            alloc: CFTypeRef,
            locale: CFLocaleRef,
            dateStyle: CFIndex,
            timeStyle: CFIndex,
        ) -> CFDateFormatterRef;
        fn CFDateFormatterCreateDateFormatFromTemplate(
            alloc: CFTypeRef,
            template: CFStringRef,
            options: CFOptionFlags,
            locale: CFLocaleRef,
        ) -> CFStringRef;
        fn CFDateFormatterSetFormat(formatter: CFDateFormatterRef, format: CFStringRef);
        fn CFDateFormatterCreateStringWithAbsoluteTime(
            alloc: CFTypeRef,
            formatter: CFDateFormatterRef,
            at: CFAbsoluteTime,
        ) -> CFStringRef;
    }

    fn locale_for(tag: &str) -> Option<cf::Owned> {
        let identifier = cf::string(tag)?;
        cf::Owned::new(unsafe { CFLocaleCreate(std::ptr::null(), identifier.as_raw()) })
    }

    /// Formats one instant with the pattern this locale writes `skeleton` in.
    fn format(tag: &str, skeleton: &str, unix_seconds: i64) -> Option<String> {
        let locale = locale_for(tag)?;
        let template = cf::string(skeleton)?;
        let pattern = cf::Owned::new(unsafe {
            CFDateFormatterCreateDateFormatFromTemplate(
                std::ptr::null(),
                template.as_raw(),
                0,
                locale.as_raw(),
            )
        })?;
        let formatter = cf::Owned::new(unsafe {
            CFDateFormatterCreate(std::ptr::null(), locale.as_raw(), NO_STYLE, NO_STYLE)
        })?;
        unsafe { CFDateFormatterSetFormat(formatter.as_raw(), pattern.as_raw()) };

        let at = unix_seconds as CFAbsoluteTime - cf::EPOCH_OFFSET;
        let out = cf::Owned::new(unsafe {
            CFDateFormatterCreateStringWithAbsoluteTime(std::ptr::null(), formatter.as_raw(), at)
        })?;
        unsafe { cf::to_string(out.as_raw()) }
    }

    pub fn user_default_tag() -> Option<String> {
        let locale = cf::Owned::new(unsafe { CFLocaleCopyCurrent() })?;
        // `Get`, not `Copy`: the identifier belongs to the locale and must not
        // be released here. It stays valid while `locale` does.
        let identifier = unsafe { cf::to_string(CFLocaleGetIdentifier(locale.as_raw()))? };
        // Core Foundation writes `de_DE`; the rest of the widget speaks BCP-47.
        let tag = identifier.replace('_', "-");
        (!tag.is_empty()).then_some(tag)
    }

    pub fn format_time(tag: &str, dt: DateTime<Local>) -> Option<String> {
        // `j` is the hour field this locale prefers, which is the whole point
        // of going through a template rather than naming `h` or `H` ourselves.
        format(tag, "jm", dt.timestamp())
    }

    pub fn format_weekday(tag: &str, date: NaiveDate) -> Option<String> {
        format(tag, "EEEE", super::noon(date)?)
    }

    pub fn format_date(tag: &str, date: NaiveDate) -> Option<String> {
        format(tag, "yMMMMd", super::noon(date)?)
    }

    pub fn format_day_month(tag: &str, date: NaiveDate) -> Option<String> {
        format(tag, "MMMd", super::noon(date)?)
    }
}

// --- Linux and the rest of Unix ----------------------------------------------

#[cfg(not(target_os = "macos"))]
mod platform {
    //! The C library's locale database, through a locale this thread alone
    //! uses.
    //!
    //! `newlocale` builds one, `uselocale` installs it for the calling thread
    //! and hands back the previous one, and both are put back before returning.
    //! `setlocale` would have done the same job for the whole process, which is
    //! not this function's to change: sync threads run alongside the drawing
    //! and would start formatting a server's dates in the user's language
    //! halfway through a request.

    use super::patterns::{day_month_pattern, is_twelve_hour, long_date_pattern, posix_candidates};
    use super::*;
    use chrono::{Datelike, Timelike};
    use std::ffi::CString;

    /// Everything the C library is asked to render, at once, so one locale
    /// lookup serves the whole call.
    fn with_locale<T>(tag: &str, f: impl FnOnce() -> Option<T>) -> Option<T> {
        for name in posix_candidates(tag) {
            let Ok(c_name) = CString::new(name) else {
                continue;
            };
            // `LC_ALL_MASK` rather than `LC_TIME_MASK` alone: `strftime` reads
            // `LC_TIME`, but the month and weekday names come back in the
            // codeset `LC_CTYPE` describes, and a mismatch there is how a
            // UTF-8 name arrives as question marks.
            let loc = unsafe {
                libc::newlocale(libc::LC_ALL_MASK, c_name.as_ptr(), std::ptr::null_mut())
            };
            if loc.is_null() {
                // Not generated on this machine. Try the next spelling.
                continue;
            }
            let previous = unsafe { libc::uselocale(loc) };
            let out = f();
            // Restored before the locale is freed: freeing the locale that is
            // still installed is undefined, and this thread goes on to format
            // other things.
            unsafe {
                libc::uselocale(previous);
                libc::freelocale(loc);
            }
            return out;
        }
        None
    }

    /// The locale's own pattern for `item`, copied out before the locale is
    /// dropped.
    ///
    /// `nl_langinfo` returns a pointer into storage the locale owns, which
    /// stops being ours the moment `freelocale` runs — so this copies rather
    /// than borrowing.
    fn langinfo(item: libc::nl_item) -> Option<String> {
        let raw = unsafe { libc::nl_langinfo(item) };
        if raw.is_null() {
            return None;
        }
        let text = unsafe { std::ffi::CStr::from_ptr(raw) }
            .to_str()
            .ok()?
            .to_owned();
        (!text.is_empty()).then_some(text)
    }

    /// `strftime` with a pattern, into a buffer large enough for a date.
    fn strftime(pattern: &str, tm: &libc::tm) -> Option<String> {
        let c_pattern = CString::new(pattern).ok()?;
        // Generous: the longest thing here is a full date with a spelled-out
        // month in a script whose characters are three bytes each.
        let mut buf = vec![0u8; 512];
        let written = unsafe {
            libc::strftime(
                buf.as_mut_ptr() as *mut libc::c_char,
                buf.len(),
                c_pattern.as_ptr(),
                tm,
            )
        };
        // Zero means it did not fit — and also means an empty result, which is
        // not a useful thing to draw either way.
        if written == 0 {
            return None;
        }
        buf.truncate(written);
        String::from_utf8(buf).ok()
    }

    /// A `struct tm` for a date, with the fields `strftime` reads set and the
    /// rest zeroed.
    ///
    /// `tm_wday` and `tm_yday` matter: `%A` reads the weekday straight out of
    /// the structure rather than deriving it, so leaving it at zero would name
    /// every day Sunday.
    fn tm_for(date: NaiveDate, hour: u32, minute: u32) -> libc::tm {
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        tm.tm_sec = 0;
        tm.tm_min = minute as libc::c_int;
        tm.tm_hour = hour as libc::c_int;
        tm.tm_mday = date.day() as libc::c_int;
        tm.tm_mon = date.month0() as libc::c_int;
        tm.tm_year = date.year() - 1900;
        tm.tm_wday = date.weekday().num_days_from_sunday() as libc::c_int;
        tm.tm_yday = date.ordinal0() as libc::c_int;
        tm.tm_isdst = -1;
        tm
    }

    pub fn user_default_tag() -> Option<String> {
        // The environment is the answer on Linux, and the core already reads
        // the three variables in the right order. Nothing the C library offers
        // improves on it: `setlocale(LC_TIME, "")` returns the same value from
        // the same source.
        None
    }

    pub fn format_time(tag: &str, dt: DateTime<Local>) -> Option<String> {
        with_locale(tag, || {
            let t_fmt = langinfo(libc::T_FMT)?;
            // The locale's own `T_FMT` carries seconds, which a widget clock
            // has no use for. What is taken from it is the hour convention;
            // minutes and the meridiem marker are then placed the way every
            // short time format places them.
            let pattern = if is_twelve_hour(&t_fmt) {
                "%I:%M %p"
            } else {
                "%H:%M"
            };
            let tm = tm_for(dt.date_naive(), dt.hour(), dt.minute());
            strftime(pattern, &tm)
        })
    }

    pub fn format_weekday(tag: &str, date: NaiveDate) -> Option<String> {
        with_locale(tag, || strftime("%A", &tm_for(date, 12, 0)))
    }

    pub fn format_date(tag: &str, date: NaiveDate) -> Option<String> {
        with_locale(tag, || {
            let d_fmt = langinfo(libc::D_FMT)?;
            strftime(&long_date_pattern(&d_fmt), &tm_for(date, 12, 0))
        })
    }

    pub fn format_day_month(tag: &str, date: NaiveDate) -> Option<String> {
        with_locale(tag, || {
            let d_fmt = langinfo(libc::D_FMT)?;
            strftime(&day_month_pattern(&d_fmt), &tm_for(date, 12, 0))
        })
    }
}
