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

    // Each falls back to the portable formatter rather than to `None`, because
    // `None` is not "no answer" to the caller — `Locale` turns it into an ISO
    // date and a bare `HH:MM`. That is the right last resort for a crate with
    // no host at all, and the wrong one here: a Linux machine that simply has
    // not generated the user's locale would drop from an English month name to
    // `2026-08-09`, which is worse and looks like a bug rather than a missing
    // locale. macOS never takes this path; Core Foundation carries every
    // locale it knows.

    fn format_time(&self, tag: &str, dt: DateTime<Local>) -> Option<String> {
        platform::format_time(tag, dt).or_else(|| PortableLocale.format_time(tag, dt))
    }

    fn format_weekday(&self, tag: &str, date: NaiveDate) -> Option<String> {
        platform::format_weekday(tag, date).or_else(|| PortableLocale.format_weekday(tag, date))
    }

    fn format_date(&self, tag: &str, date: NaiveDate) -> Option<String> {
        platform::format_date(tag, date).or_else(|| PortableLocale.format_date(tag, date))
    }

    fn format_day_month(&self, tag: &str, date: NaiveDate) -> Option<String> {
        platform::format_day_month(tag, date).or_else(|| PortableLocale.format_day_month(tag, date))
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

    fn is_day(spec: char) -> bool {
        matches!(spec, 'd' | 'e')
    }

    /// May the numeric month in this pattern become a name?
    ///
    /// Only where every separator is ASCII. A locale that writes `%Y年%m月%d日`
    /// puts the field's *unit* in the separator, and the C library's full month
    /// name there is `8月` — already carrying the character that follows it. Widening
    /// would print it twice, so such a pattern keeps its numeric month and gains
    /// only a four-digit year, which is the locale's own long form anyway.
    ///
    /// The test for it is the separators rather than the language, so a locale
    /// nobody thought of is judged by what it actually writes.
    fn month_may_be_named(tokens: &[Token]) -> bool {
        tokens.iter().all(|t| match t {
            Token::Literal(text) => text.is_ascii(),
            Token::Conversion(..) => true,
        })
    }

    /// The separator between two fields, once the month beside it is a word.
    ///
    /// `4.August.2026` is not how any locale writes a date, so punctuation that
    /// separates *numbers* becomes a space. The exception is punctuation that
    /// belongs to the day rather than to the gap: German writes `4. August
    /// 2026`, where the full stop is the ordinal marker, and English writes
    /// `August 4, 2026`. Which is which is decided by the field it follows —
    /// after the day it is kept, anywhere else it is a separator and goes.
    fn spaced(separator: &str, follows: Option<char>) -> String {
        let after_day = follows.is_some_and(is_day);
        match separator.trim() {
            "." if after_day => ". ".into(),
            "," if after_day => ", ".into(),
            "" => " ".into(),
            other if other.chars().all(|c| c.is_ascii_punctuation()) => " ".into(),
            other => format!("{other} "),
        }
    }

    /// Widens a locale's short date into a long one.
    ///
    /// The numeric month becomes the full name where that is safe, a two-digit
    /// year becomes four, and the day loses its leading zero — `%e` pads with a
    /// space, which [`super::platform::tidy`] then takes off. The field order is
    /// the locale's throughout.
    ///
    /// A weekday in the pattern is dropped: the widget draws the weekday on the
    /// line above, and this is the line under it.
    pub(super) fn long_date_pattern(d_fmt: &str) -> String {
        let tokens = tokenize(d_fmt);
        let name_the_month = month_may_be_named(&tokens);
        let mut out = String::new();
        let mut pending_literal: Option<String> = None;
        let mut previous: Option<char> = None;

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
                        && previous.is_some()
                    {
                        out.push_str(&if name_the_month {
                            spaced(&text, previous)
                        } else {
                            text
                        });
                    }
                    match spec {
                        'm' | 'b' | 'h' if name_the_month => out.push_str("%B"),
                        'y' | 'C' | 'g' => out.push_str("%Y"),
                        _ if is_day(spec) && name_the_month => out.push_str("%e"),
                        _ => out.push_str(&raw),
                    }
                    previous = Some(spec);
                }
            }
        }
        // A trailing literal belongs to the last field — `%Y年%m月%d日` ends in
        // one. It is left alone: there is no following field for it to separate
        // from, so it is part of the date rather than a gap in it.
        if let Some(text) = pending_literal
            && previous.is_some()
            && !name_the_month
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
    /// `%e. %b` rather than `%d.%b.`, and `%Y年%m月%d日` becomes `%m月%d日` —
    /// with its month left numeric, for the reason [`month_may_be_named`] gives.
    /// The month is abbreviated where it is named at all: the column is narrow,
    /// and the year is the field carrying no information in a list of things due
    /// this week.
    pub(super) fn day_month_pattern(d_fmt: &str) -> String {
        let tokens = tokenize(d_fmt);
        let name_the_month = month_may_be_named(&tokens);

        let mut kept: Vec<Token> = Vec::new();
        for token in tokens {
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
        let mut previous: Option<char> = None;
        for token in &kept {
            match token {
                // Punctuation before any field has been written is what the
                // dropped leading field left behind.
                Token::Literal(_) if previous.is_none() => {}
                Token::Literal(text) => out.push_str(&if name_the_month {
                    spaced(text, previous)
                } else {
                    text.clone()
                }),
                Token::Conversion(raw, spec) => {
                    out.push_str(match spec {
                        _ if is_month(*spec) && name_the_month => "%b",
                        _ if is_day(*spec) && name_the_month => "%e",
                        _ => raw,
                    });
                    previous = Some(*spec);
                }
            }
        }
        // A named month leaves no trailing punctuation behind: `4. Aug.` gained
        // its stop from the day, not from the end of the pattern.
        let out = if name_the_month {
            out.trim_end()
                .trim_end_matches(['.', ',', '/', '-'])
                .to_string()
        } else {
            out
        };
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
        fn widening_a_short_date_keeps_the_locales_order_and_spaces_the_month() {
            // German: the full stop stays, because it is the day's ordinal
            // marker rather than a separator between numbers — `4. August 2026`.
            assert_eq!(long_date_pattern("%d.%m.%Y"), "%e. %B %Y");
            // French and British: a slash separates numbers and has no business
            // between a number and a word.
            assert_eq!(long_date_pattern("%d/%m/%Y"), "%e %B %Y");
            // American, and the field order is still the locale's.
            assert_eq!(long_date_pattern("%m/%d/%y"), "%B %e %Y");
            assert_eq!(long_date_pattern("%Y-%m-%d"), "%Y %B %e");
        }

        #[test]
        fn a_locale_whose_separators_carry_the_unit_keeps_its_numeric_month() {
            // `%Y年%m月%d日` writes the field's unit as the separator, and the C
            // library's full month name there is `8月` — already carrying the
            // character that follows it. Naming the month would print it twice.
            // So the shape is left alone and only the year is widened.
            assert_eq!(long_date_pattern("%Y年%m月%d日"), "%Y年%m月%d日");
            assert_eq!(long_date_pattern("%Y년 %m월 %d일"), "%Y년 %m월 %d일");
            assert_eq!(day_month_pattern("%Y年%m月%d日"), "%m月%d日");
        }

        #[test]
        fn a_weekday_in_the_short_date_is_dropped_with_its_separator() {
            // The widget draws the weekday on the line above this one, and a
            // pattern that kept it would start with a stray comma once it went.
            assert_eq!(long_date_pattern("%a, %d %b %Y"), "%e %B %Y");
            assert_eq!(day_month_pattern("%a %d/%m/%Y"), "%e %b");
        }

        #[test]
        fn the_due_column_loses_the_year_and_the_separator_that_came_with_it() {
            assert_eq!(day_month_pattern("%d.%m.%Y"), "%e. %b");
            assert_eq!(day_month_pattern("%d/%m/%Y"), "%e %b");
            // Year first: the separator that follows it goes instead of one that
            // precedes it, so nothing starts with a stray mark.
            assert_eq!(day_month_pattern("%Y-%m-%d"), "%b %e");
            assert_eq!(day_month_pattern("%m/%d/%y"), "%b %e");
        }

        #[test]
        fn an_unusable_pattern_still_yields_something_drawable() {
            // A locale with no date pattern at all, or one made of nothing this
            // understands, must not produce an empty line where the date goes.
            assert_eq!(long_date_pattern(""), "%e %B %Y");
            assert_eq!(day_month_pattern(""), "%e %b");
            assert_eq!(day_month_pattern("%Y"), "%e %b");
        }

        #[test]
        fn punctuation_belonging_to_the_day_survives_and_the_rest_becomes_a_space() {
            // `4. August` in German, `August 4, 2026` in English: both marks
            // belong to the day, and both would be wrong anywhere else.
            assert_eq!(spaced(".", Some('d')), ". ");
            assert_eq!(spaced(",", Some('d')), ", ");
            assert_eq!(spaced(".", Some('B')), " ");
            assert_eq!(spaced(",", Some('B')), " ");
            // A separator between numbers has nothing to do beside a word.
            assert_eq!(spaced("/", Some('d')), " ");
            assert_eq!(spaced("-", Some('Y')), " ");
            assert_eq!(spaced(" ", Some('d')), " ");
        }

        #[test]
        fn the_month_is_named_only_where_the_separators_are_plain() {
            assert!(month_may_be_named(&tokenize("%d.%m.%Y")));
            assert!(month_may_be_named(&tokenize("%d %m %Y")));
            assert!(!month_may_be_named(&tokenize("%Y年%m月%d日")));
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
    ///
    /// The result is tidied before it is returned. `%e` pads a single-digit day
    /// with a space so the column lines up in a terminal, which is not what a
    /// widget wants, and a locale with no meridiem marker leaves `%p` empty
    /// with its space still there. Both come out as runs of whitespace, and
    /// neither is anything the locale asked for.
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
        let rendered = String::from_utf8(buf).ok()?;
        let tidied = tidy(&rendered);
        (!tidied.is_empty()).then_some(tidied)
    }

    /// Collapses runs of spaces and trims the ends.
    ///
    /// Only ASCII spaces and tabs: a narrow no-break space between a number and
    /// its unit is a deliberate choice by the locale, and squeezing it would be
    /// this code overruling the database it went to the trouble of asking.
    fn tidy(rendered: &str) -> String {
        let mut out = String::with_capacity(rendered.len());
        let mut last_was_space = false;
        for c in rendered.chars() {
            let space = c == ' ' || c == '\t';
            if space && last_was_space {
                continue;
            }
            out.push(if space { ' ' } else { c });
            last_was_space = space;
        }
        out.trim().to_string()
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

/// What the platform actually returns, asked of the platform.
///
/// The tests above cover the string handling with no system involved. These
/// cover the other half — that the system was asked the right question and its
/// answer arrives intact — which is the part no amount of unit testing of
/// patterns can show.
///
/// They call the back end directly rather than through [`UnixLocale`], whose
/// methods fall back to the portable formatter: a fallback is the right
/// behaviour and the wrong thing to test through, because it would turn "the
/// platform gave nothing" into a passing English answer.
///
/// **On a machine without the locale generated they skip.** `newlocale` fails
/// for `de_DE.UTF-8` on a stock container, and a developer's laptop is not
/// obliged to carry every locale this checks. Continuous integration generates
/// them and sets `TPMPLANER_REQUIRE_SYSTEM_LOCALE`, which turns the skip back
/// into a failure — so a green run there means the back end answered, not that
/// it was excused. macOS needs neither: Core Foundation carries the data
/// itself.
#[cfg(test)]
mod backend {
    use super::*;
    use chrono::Datelike;

    /// The date every case below is asked about. Nothing depends on which
    /// weekday it happens to be — that is looked up — but the month is fixed,
    /// and August is spelled distinctly in the two languages used here.
    fn subject() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 8, 4).unwrap()
    }

    /// `None` when this system has no such locale, unless CI said it must.
    fn answer(what: &str, tag: &str, got: Option<String>) -> Option<String> {
        if got.is_none() && std::env::var_os("TPMPLANER_REQUIRE_SYSTEM_LOCALE").is_some() {
            panic!(
                "{what} returned nothing for {tag}, but this run requires the \
                 system locale to be present"
            );
        }
        got
    }

    #[test]
    fn the_hour_convention_is_the_locales_own() {
        use chrono::TimeZone;
        let afternoon = Local.with_ymd_and_hms(2026, 8, 4, 14, 30, 0).unwrap();

        if let Some(german) = answer(
            "format_time",
            "de-DE",
            platform::format_time("de-DE", afternoon),
        ) {
            assert_eq!(german.trim(), "14:30", "German counts to twenty-four");
        }
        if let Some(american) = answer(
            "format_time",
            "en-US",
            platform::format_time("en-US", afternoon),
        ) {
            let upper = american.to_uppercase();
            assert!(
                upper.contains("PM"),
                "American English counts to twelve: {american}"
            );
            assert!(american.contains("2:30"), "{american}");
        }
        // The pair is the point: the same instant, two conventions, neither
        // guessed from the language — `en-GB` would agree with the German one.
    }

    #[test]
    fn the_weekday_comes_back_in_the_locales_language() {
        // Looked up rather than hard-coded, so the test says nothing about
        // which day the fourth of August falls on.
        const GERMAN: [&str; 7] = [
            "Montag",
            "Dienstag",
            "Mittwoch",
            "Donnerstag",
            "Freitag",
            "Samstag",
            "Sonntag",
        ];
        let date = subject();
        let expected = GERMAN[date.weekday().num_days_from_monday() as usize];

        if let Some(got) = answer(
            "format_weekday",
            "de-DE",
            platform::format_weekday("de-DE", date),
        ) {
            assert_eq!(got, expected);
        }
    }

    #[test]
    fn the_long_date_carries_a_spelled_out_month_in_the_right_language() {
        let date = subject();
        for (tag, month) in [("de-DE", "August"), ("fr-FR", "août")] {
            if let Some(got) = answer("format_date", tag, platform::format_date(tag, date)) {
                assert!(got.contains(month), "{tag}: {got} should contain {month}");
                assert!(got.contains("2026"), "{tag}: {got} should carry the year");
                // The weekday belongs on the line above this one.
                assert!(
                    !got.to_lowercase().contains("dienstag")
                        && !got.to_lowercase().contains("mardi"),
                    "{tag}: {got} should not repeat the weekday"
                );
            }
        }
    }

    #[test]
    fn the_due_column_names_the_month_and_drops_the_year() {
        let date = subject();
        for (tag, month) in [("de-DE", "Aug"), ("fr-FR", "ao")] {
            if let Some(got) = answer(
                "format_day_month",
                tag,
                platform::format_day_month(tag, date),
            ) {
                assert!(got.contains(month), "{tag}: {got} should name the month");
                assert!(got.contains('4'), "{tag}: {got} should carry the day");
                assert!(
                    !got.contains("2026") && !got.contains("26"),
                    "{tag}: {got} should not carry the year"
                );
            }
        }
    }
}
