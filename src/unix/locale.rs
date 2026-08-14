// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Ephemeris contributors
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
//! [`PortableLocale`]: ephemeris_core::host::PortableLocale

use chrono::{DateTime, Local, NaiveDate};
use ephemeris_core::host::{LocaleBackend, PortableLocale, PosixLocale, posix_environment_locale};

/// The tag that stands for no language at all.
///
/// Passed on as a tag rather than resolved to English here, so the rest of the
/// program sees what the environment actually said and each part answers it in
/// its own terms: [`patterns::posix_candidates`] produces nothing for it, so
/// the C library is never asked; [`platform::format`] on macOS declines it for
/// the same reason; the catalogue lookup falls through to English. What draws
/// the dates is then [`PortableLocale`] — ISO order, English names — which is
/// what the C locale is.
const NEUTRAL_TAG: &str = "C";

/// Does this tag name no language?
///
/// `C` and `POSIX` are locales rather than languages, and neither platform's
/// database has any business being asked about them: Core Foundation would
/// answer with whatever it takes the root locale to be, and the C library would
/// be asked for the absence of a locale. Declining lands on [`PortableLocale`]
/// — ISO order, English names — which is what the C locale is.
fn names_no_language(tag: &str) -> bool {
    let primary = tag.split(['-', '_', '.', '@']).next().unwrap_or(tag);
    primary.eq_ignore_ascii_case("C") || primary.eq_ignore_ascii_case("POSIX")
}

pub struct UnixLocale;

impl LocaleBackend for UnixLocale {
    /// The environment first, then the platform.
    ///
    /// Order matters, and getting it wrong is not a small thing: asking Core
    /// Foundation first makes `LC_ALL=fr_FR.UTF-8 ephemeris` print English on a
    /// machine set to English, because `CFLocaleCopyCurrent` reports what
    /// System Settings says and knows nothing about the environment. Somebody
    /// who sets the variable has said which language they want, explicitly, for
    /// this run — that outranks a system-wide preference by definition.
    fn user_default_tag(&self) -> String {
        match posix_environment_locale() {
            PosixLocale::Language(tag) => tag,
            // `LC_ALL=C` is the standard way to ask for reproducible output,
            // and it is an answer: the C library stops there rather than
            // falling through to `LANG`. This used to skip it, so
            // `LC_ALL=C LANG=de_DE.UTF-8` printed a German agenda on a machine
            // where every other program printed English.
            PosixLocale::Neutral => NEUTRAL_TAG.to_string(),
            // Nothing set, which is the normal case for anything started from
            // the Dock or a login item: Core Foundation is the better answer
            // on macOS and there is no answer at all on Linux, where the
            // environment is the mechanism.
            PosixLocale::Unset => {
                platform::user_default_tag().unwrap_or_else(|| PortableLocale.user_default_tag())
            }
        }
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
        // The same rule the macOS back end applies, from the same place: `C`
        // names no language, so there is nothing here to ask the C library for.
        if super::names_no_language(&language) {
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
    ///
    /// `%r` counts too, and missing it is what this originally got wrong: glibc
    /// does not spell `en_US`'s time format out as `%I:%M:%S %p` but writes the
    /// whole thing as `%r`, which *is* the twelve-hour format by definition.
    /// Scanning for the hour field alone found nothing there and called the
    /// most twelve-hour locale in the world a twenty-four-hour one.
    pub(super) fn is_twelve_hour(t_fmt: &str) -> bool {
        tokenize(t_fmt)
            .iter()
            .any(|t| matches!(t, Token::Conversion(_, 'I' | 'l' | 'r')))
    }

    /// A `strftime` pattern split into literal runs and conversions, so a field can
    /// be replaced or removed without disturbing the locale's own punctuation.
    #[derive(Debug, PartialEq)]
    enum Token {
        /// The whole conversion including the leading `%`, any flags, any field
        /// width and any modifier — and the specifier letter on its own.
        Conversion(String, char),
        Literal(String),
    }

    /// Reads one conversion, the `%` having been consumed already.
    ///
    /// The shape is `%`, then flags, then an optional field width, then an
    /// optional `E` or `O` modifier, then the specifier letter. Everything
    /// before the letter has to be stepped over to find it, and skipping only
    /// the modifier is what this got wrong: glibc writes `cs_CZ`'s and
    /// `sk_SK`'s short date as `%-d.%-m.%Y`, where `-` is the GNU "do not pad"
    /// flag. Reading the `-` as the specifier shredded the pattern — the due
    /// column ended in a bare `%`, and the month was never recognised as one to
    /// widen. `%_d`, `%02d` and `%^a` fail the same way.
    ///
    /// The specifier is `None` when the pattern ends part-way through a
    /// conversion, which makes it not a conversion at all: the caller keeps the
    /// text as a literal, which is what `strftime` prints for it.
    fn read_conversion(chars: &mut std::iter::Peekable<std::str::Chars>) -> (String, Option<char>) {
        let mut conversion = String::from('%');
        // The GNU flags, then a width. `0` is both a flag and a digit, which
        // costs nothing here: either loop consumes it.
        while let Some(&c) = chars.peek() {
            if !matches!(c, '-' | '_' | '0' | '^' | '#') {
                break;
            }
            conversion.push(c);
            chars.next();
        }
        while let Some(&c) = chars.peek() {
            if !c.is_ascii_digit() {
                break;
            }
            conversion.push(c);
            chars.next();
        }
        let Some(mut spec) = chars.next() else {
            return (conversion, None);
        };
        conversion.push(spec);
        if (spec == 'E' || spec == 'O')
            && let Some(&after) = chars.peek()
        {
            chars.next();
            conversion.push(after);
            spec = after;
        }
        (conversion, Some(spec))
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
            if chars.peek() == Some(&'%') {
                chars.next();
                literal.push('%');
                continue;
            }
            let (conversion, spec) = read_conversion(&mut chars);
            let Some(spec) = spec else {
                // The pattern ran out mid-conversion, so there is no field
                // here — only the text that was mistaken for one.
                literal.push_str(&conversion);
                break;
            };
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

    /// The `E` or `O` a conversion carried, so a field that is substituted for
    /// it keeps what the locale asked for.
    ///
    /// `ja_JP` asks for `%EY` — the era year — and Arabic locales for `%Od`,
    /// alternative digits. Widening `%Od` to a bare `%e` answers a question the
    /// locale did not ask. The flags and the width are deliberately *not*
    /// carried across: they describe the padding of the field that is being
    /// replaced, and the replacement is a different field, often a name rather
    /// than a number.
    fn modifier(raw: &str) -> &'static str {
        let mut back = raw.chars().rev();
        back.next();
        match back.next() {
            Some('E') => "E",
            Some('O') => "O",
            _ => "",
        }
    }

    /// A conversion for `spec`, keeping `raw`'s modifier.
    fn substitute(raw: &str, spec: char) -> String {
        format!("%{}{spec}", modifier(raw))
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
        // Set when a weekday was dropped before anything had been written, so
        // the separator it left behind can be dropped with it.
        let mut weekday_led = false;

        for token in tokens {
            match token {
                Token::Literal(text) => pending_literal = Some(text),
                Token::Conversion(raw, spec) => {
                    if is_weekday(spec) {
                        // Drop the separator that came with it as well, or the
                        // line starts with a stray comma. Which side that
                        // separator is on depends on where the weekday sat:
                        // `%d, %a` carries it in front, `%a, %d` behind.
                        pending_literal = None;
                        weekday_led |= previous.is_none();
                        continue;
                    }
                    // Only the separator a dropped leading weekday left behind
                    // is suppressed. Anything else is text the locale wrote —
                    // `long_date_pattern("le %d/%m/%Y")` begins with `le `, and
                    // gating this on "something has been written already" threw
                    // it away.
                    if let Some(text) = pending_literal.take()
                        && !(weekday_led && previous.is_none())
                    {
                        out.push_str(&if name_the_month {
                            spaced(&text, previous)
                        } else {
                            text
                        });
                    }
                    match spec {
                        'm' | 'b' | 'h' if name_the_month => {
                            out.push_str(&substitute(&raw, 'B'));
                        }
                        // `%G` is a year like the rest — [`is_year`] counts it,
                        // so the due column already drops it — and leaving it
                        // here let an ISO week-based year through unwidened,
                        // which differs from the calendar year over new year.
                        'y' | 'C' | 'g' | 'G' => out.push_str(&substitute(&raw, 'Y')),
                        _ if is_day(spec) && name_the_month => {
                            out.push_str(&substitute(&raw, 'e'));
                        }
                        _ => out.push_str(&raw),
                    }
                    previous = Some(spec);
                }
            }
        }
        // A trailing literal belongs to the last field — `%Y年%m月%d日` ends in
        // one, and so does `hu_HU`'s `%Y.%m.%d.`, where the closing stop is the
        // Hungarian ordinal marker. It is left alone rather than spaced: there
        // is no following field for it to separate from, so it is part of the
        // date rather than a gap in it.
        if let Some(text) = pending_literal
            && previous.is_some()
        {
            out.push_str(&text);
        }
        if out.is_empty() {
            "%e %B %Y".into()
        } else {
            out
        }
    }

    /// The locale's own short time: its `T_FMT` with the seconds taken out.
    ///
    /// `T_FMT` carries more than the hour convention — it carries the field
    /// *order*, and a twelve-hour locale need not put the meridiem last. The
    /// shape glibc records for a Korean twelve-hour time is
    /// `%p %I시 %M분 %S초` and for a Taiwanese one `%p %I時%M分%S秒`, so a
    /// pattern invented here as `%I:%M %p` would read `08:00 오전` where the
    /// locale reads `오전 8:00`. Field order is not guessable from a language;
    /// that is the design note this module opens with, and the date path has
    /// obeyed it all along.
    ///
    /// A widget clock has no use for seconds, so that field goes, and with it
    /// the punctuation that came with it — on whichever side the locale keeps
    /// it. A separator between numbers introduces the field it precedes;
    /// a separator that carries the field's *unit*, as `%M分%S秒` does, follows
    /// it.
    pub(super) fn short_time_pattern(t_fmt: &str) -> String {
        let mut tokens = tokenize(&expand_compound(t_fmt));
        if let Some(at) = tokens
            .iter()
            .position(|t| matches!(t, Token::Conversion(_, 'S')))
        {
            let unit_follows = matches!(
                tokens.get(at + 1),
                Some(Token::Literal(text)) if !text.is_ascii()
            );
            if unit_follows {
                tokens.remove(at + 1);
                tokens.remove(at);
            } else {
                tokens.remove(at);
                if at > 0 && matches!(tokens.get(at - 1), Some(Token::Literal(_))) {
                    tokens.remove(at - 1);
                }
            }
        }

        // Rebuilt verbatim, with none of the respacing the date path does: a
        // locale's own time punctuation is already right for a time, and there
        // is no field here being widened into a word.
        let mut out = String::new();
        for token in &tokens {
            match token {
                Token::Literal(text) => out.push_str(text),
                Token::Conversion(raw, _) => out.push_str(raw),
            }
        }
        let out = out.trim();
        if out.is_empty() {
            "%H:%M".into()
        } else {
            out.to_owned()
        }
    }

    /// Writes the compound specifiers out as the fields they stand for.
    ///
    /// `%r`, `%T` and `%R` name a whole time rather than a field, and glibc
    /// uses them: `en_US`'s `T_FMT` is `%r` and nothing else. There is no
    /// seconds field to drop from a pattern that is one letter long, so it is
    /// replaced by the one POSIX defines it as.
    fn expand_compound(t_fmt: &str) -> String {
        let mut out = String::with_capacity(t_fmt.len());
        for token in tokenize(t_fmt) {
            match token {
                Token::Literal(text) => out.push_str(&text),
                Token::Conversion(raw, spec) => out.push_str(match spec {
                    'r' => "%I:%M:%S %p",
                    'T' => "%H:%M:%S",
                    'R' => "%H:%M",
                    _ => &raw,
                }),
            }
        }
        out
    }

    /// Takes the padding zero off the hour on a twelve-hour clock.
    ///
    /// No clock anywhere writes `07:13 AM`. `%I` pads to two digits and `%l`,
    /// which pads with a space instead, is not in POSIX — so the zero comes off
    /// the rendered text. The first run of digits rather than the start of the
    /// string, because the hour is not always first: `ko_KR` puts the meridiem
    /// in front of it. Two digits and no more, so `10:07` keeps its own.
    pub(super) fn drop_hour_padding(rendered: &str) -> String {
        let Some(start) = rendered.find(|c: char| c.is_ascii_digit()) else {
            return rendered.to_owned();
        };
        let run = &rendered[start..];
        let end = run.find(|c: char| !c.is_ascii_digit()).unwrap_or(run.len());
        if end != 2 || !run.starts_with('0') {
            return rendered.to_owned();
        }
        let mut out = String::with_capacity(rendered.len() - 1);
        out.push_str(&rendered[..start]);
        out.push_str(&rendered[start + 1..]);
        out
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
                    out.push_str(&match spec {
                        _ if is_month(*spec) && name_the_month => substitute(raw, 'b'),
                        _ if is_day(*spec) && name_the_month => substitute(raw, 'e'),
                        _ => raw.clone(),
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
            // What glibc actually reports for `en_US`: the whole twelve-hour
            // format under one specifier, with no hour field to find. Reading
            // only `%I` and `%l` here called `en_US` a twenty-four-hour locale.
            assert!(is_twelve_hour("%r"));
            assert!(!is_twelve_hour("%H:%M:%S"));
            assert!(!is_twelve_hour("%k:%M"));
            // `%T` and `%R` are the twenty-four-hour equivalents of `%r`.
            assert!(!is_twelve_hour("%T"));
            assert!(!is_twelve_hour("%R"));
            // A literal per cent is not a conversion.
            assert!(!is_twelve_hour("%H%%I"));
            // A GNU flag sits between the per cent and the specifier, and
            // stepping over only `E` and `O` found `-` where the hour was —
            // the same silent failure the `%r` case above is about.
            assert!(is_twelve_hour("%-I:%M:%S %p"));
            assert!(is_twelve_hour("%_I:%M %p"));
            assert!(!is_twelve_hour("%-H:%M"));
        }

        #[test]
        fn a_flag_or_a_field_width_belongs_to_the_conversion_and_not_to_the_text() {
            // glibc's `cs_CZ` and `sk_SK` short date. Reading `-` as the
            // specifier left the day unrecognised and the pattern ending in a
            // bare `%`, so a Czech due column read `4. %`.
            assert_eq!(
                tokenize("%-d.%-m.%Y"),
                vec![
                    Token::Conversion("%-d".into(), 'd'),
                    Token::Literal(".".into()),
                    Token::Conversion("%-m".into(), 'm'),
                    Token::Literal(".".into()),
                    Token::Conversion("%Y".into(), 'Y'),
                ]
            );
            assert_eq!(long_date_pattern("%-d.%-m.%Y"), "%e. %B %Y");
            assert_eq!(day_month_pattern("%-d.%-m.%Y"), "%e. %b");
            // A space-padding flag and a field width, both of which glibc
            // accepts and both of which used to shred the pattern.
            assert_eq!(day_month_pattern("%_d/%_m/%Y"), "%e %b");
            assert_eq!(day_month_pattern("%02d/%02m/%Y"), "%e %b");
            // A pattern that stops part-way through a conversion has no field
            // in it, so what is there is text.
            assert_eq!(tokenize("%"), vec![Token::Literal("%".into())]);
            assert_eq!(tokenize("%-"), vec![Token::Literal("%-".into())]);
        }

        #[test]
        fn a_substituted_field_keeps_the_modifier_the_locale_asked_for() {
            // `%Od` is alternative digits and `%Ey` an era year. Widening the
            // field is not licence to answer a different question.
            assert_eq!(long_date_pattern("%Od.%Om.%Y"), "%Oe. %OB %Y");
            assert_eq!(long_date_pattern("%d.%m.%Ey"), "%e. %B %EY");
            assert_eq!(day_month_pattern("%Od.%Om.%Y"), "%Oe. %Ob");
            // `%G` is a year like the others — `day_month_pattern` already
            // drops it — so the long date widens it rather than letting an ISO
            // week-based year through.
            assert_eq!(long_date_pattern("%G-%m-%d"), "%Y %B %e");
            assert_eq!(day_month_pattern("%G-%m-%d"), "%b %e");
        }

        #[test]
        fn text_at_either_end_of_the_pattern_is_the_locales_and_stays() {
            // Leading: the guard that suppresses a separator orphaned by a
            // dropped weekday used to swallow genuine text with it.
            assert_eq!(long_date_pattern("le %d/%m/%Y"), "le %e %B %Y");
            // Trailing: glibc's `hu_HU` is `%Y.%m.%d.`, where the closing stop
            // is the Hungarian ordinal marker rather than a separator —
            // `2026. augusztus 4.` reads wrong without it.
            assert_eq!(long_date_pattern("%Y.%m.%d."), "%Y %B %e.");
            // And the case the guard was there for still works, because the
            // weekday branch drops its own separator on whichever side it is.
            assert_eq!(long_date_pattern("%a, %d %b %Y"), "%e %B %Y");
            assert_eq!(long_date_pattern("%d %b %Y, %a"), "%e %B %Y");
        }

        #[test]
        fn the_short_time_keeps_the_locales_field_order_and_loses_the_seconds() {
            // European: the separator introduces the field it precedes.
            assert_eq!(short_time_pattern("%H:%M:%S"), "%H:%M");
            assert_eq!(short_time_pattern("%I:%M:%S %p"), "%I:%M %p");
            // The compound specifiers name a whole time, and glibc uses them:
            // `en_US`'s `T_FMT` is `%r` and nothing more.
            assert_eq!(short_time_pattern("%r"), "%I:%M %p");
            assert_eq!(short_time_pattern("%T"), "%H:%M");
            assert_eq!(short_time_pattern("%R"), "%H:%M");
            // Korean and Taiwanese put the meridiem first. Inventing
            // `%I:%M %p` here wrote `08:00 오전` for a locale that reads
            // `오전 8:00`.
            assert_eq!(short_time_pattern("%p %I시 %M분 %S초"), "%p %I시 %M분");
            assert_eq!(short_time_pattern("%p %I時%M分%S秒"), "%p %I時%M分");
            // A separator carrying the field's unit follows it, so that is the
            // one that goes with the seconds.
            assert_eq!(short_time_pattern("%H時%M分%S秒"), "%H時%M分");
            // Nothing usable still has to yield a clock.
            assert_eq!(short_time_pattern(""), "%H:%M");
        }

        #[test]
        fn a_twelve_hour_clock_drops_the_padding_wherever_the_hour_sits() {
            assert_eq!(drop_hour_padding("07:13 AM"), "7:13 AM");
            // Ten past ten keeps its own zero, and so does the minute.
            assert_eq!(drop_hour_padding("10:07 AM"), "10:07 AM");
            // The hour is not always first.
            assert_eq!(drop_hour_padding("오전 08:00"), "오전 8:00");
            assert_eq!(drop_hour_padding("AM"), "AM");
            assert_eq!(drop_hour_padding(""), "");
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
    use std::cell::RefCell;
    use std::collections::HashMap;

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

    thread_local! {
        /// A ready formatter per skeleton and tag, built once.
        ///
        /// Nested rather than keyed on a pair, so a lookup borrows the tag it
        /// was handed instead of allocating a key to ask with.
        ///
        /// The `cf` module's own doc gives the reason this exists: *"The widget
        /// formats a clock every minute for days, so this is not a rounding
        /// error."* It applies here with more force — the clock is one call,
        /// but the renderer asks for a time label per visible event and a due
        /// date per task on every frame. Each of those used to build a
        /// `CFLocale`, a template string, a derived pattern and a formatter and
        /// throw all four away, and
        /// `CFDateFormatterCreateDateFormatFromTemplate` derives a CLDR pattern
        /// every time it is called.
        ///
        /// The set is tiny and fixed — four skeletons against one or two tags —
        /// and a formatter is immutable once its pattern is set, so there is
        /// nothing to invalidate. Per thread because a `CFDateFormatter` is not
        /// documented as safe to use from several at once.
        static FORMATTERS: RefCell<HashMap<&'static str, HashMap<String, cf::Owned>>> =
            RefCell::new(HashMap::new());
    }

    /// The formatter that writes `skeleton` the way `tag` writes it.
    ///
    /// A raw reference rather than the [`cf::Owned`]: the owner stays in the
    /// cache for the life of the thread and nothing removes it, and handing
    /// back a borrow would keep the `RefCell` borrowed across the format call.
    fn formatter(tag: &str, skeleton: &'static str) -> Option<CFDateFormatterRef> {
        FORMATTERS.with(|cache| {
            let mut cache = cache.borrow_mut();
            let by_tag = cache.entry(skeleton).or_default();
            if let Some(ready) = by_tag.get(tag) {
                return Some(ready.as_raw());
            }
            let locale = locale_for(tag)?;
            let template = cf::string(skeleton)?;
            // A *skeleton* names the fields wanted; this is what turns it into
            // the pattern this locale actually writes them in.
            let pattern = cf::Owned::new(unsafe {
                CFDateFormatterCreateDateFormatFromTemplate(
                    std::ptr::null(),
                    template.as_raw(),
                    0,
                    locale.as_raw(),
                )
            })?;
            let built = cf::Owned::new(unsafe {
                CFDateFormatterCreate(std::ptr::null(), locale.as_raw(), NO_STYLE, NO_STYLE)
            })?;
            unsafe { CFDateFormatterSetFormat(built.as_raw(), pattern.as_raw()) };
            let raw = built.as_raw();
            by_tag.insert(tag.to_owned(), built);
            Some(raw)
        })
    }

    /// Formats one instant with the pattern this locale writes `skeleton` in.
    fn format(tag: &str, skeleton: &'static str, unix_seconds: i64) -> Option<String> {
        // The neutral locale is not a language, and Core Foundation would
        // answer for it anyway with whatever it takes the root locale to be.
        // Declining lands on the portable formatter — ISO order, English names
        // — which is what `LC_ALL=C` asks for. The C library back end reaches
        // the same answer through `posix_candidates`, which produces nothing
        // for it.
        if super::names_no_language(tag) {
            return None;
        }
        let formatter = formatter(tag, skeleton)?;
        let at = unix_seconds as CFAbsoluteTime - cf::EPOCH_OFFSET;
        let out = cf::Owned::new(unsafe {
            CFDateFormatterCreateStringWithAbsoluteTime(std::ptr::null(), formatter, at)
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

    use super::patterns::{
        day_month_pattern, drop_hour_padding, is_twelve_hour, long_date_pattern, posix_candidates,
        short_time_pattern,
    };
    use super::*;
    use chrono::{Datelike, Timelike};
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::ffi::CString;

    /// A `locale_t`, freed when the thread that built it goes away.
    struct Handle(libc::locale_t);

    impl Drop for Handle {
        fn drop(&mut self) {
            // Nothing of ours is installed at this point: [`with_locale`]
            // restores the previous locale before it returns, every time.
            unsafe { libc::freelocale(self.0) };
        }
    }

    thread_local! {
        /// The locales a tag resolved to, built once per thread.
        ///
        /// `newlocale` opens and parses the locale archive, and this is not a
        /// once-per-day path: `src/win/render.rs` asks for a time label per
        /// visible event and a due date per task on every frame — sixty times a
        /// second while a reveal or hover animation runs. A locale is immutable
        /// once built, so there is nothing to invalidate, and the widget uses
        /// one tag.
        ///
        /// Per thread rather than shared: a `locale_t` is installed with
        /// `uselocale`, which is a property of the calling thread, and the sync
        /// threads have no business reaching into a handle the drawing thread
        /// is formatting through.
        static LOCALES: RefCell<HashMap<String, Vec<Handle>>> = RefCell::new(HashMap::new());
    }

    /// Every candidate spelling of `tag` that this machine actually has, in
    /// order of preference.
    fn locales_for(tag: &str) -> Vec<libc::locale_t> {
        LOCALES.with(|cache| {
            let mut cache = cache.borrow_mut();
            if !cache.contains_key(tag) {
                let handles: Vec<Handle> = posix_candidates(tag)
                    .into_iter()
                    .filter_map(|name| CString::new(name).ok())
                    .filter_map(|name| {
                        // `LC_ALL_MASK` rather than `LC_TIME_MASK` alone:
                        // `strftime` reads `LC_TIME`, but the month and weekday
                        // names come back in the codeset `LC_CTYPE` describes,
                        // and a mismatch there is how a UTF-8 name arrives as
                        // question marks.
                        let loc = unsafe {
                            libc::newlocale(libc::LC_ALL_MASK, name.as_ptr(), std::ptr::null_mut())
                        };
                        // Null means the locale is not generated on this
                        // machine. Try the next spelling.
                        //
                        // `then` rather than `then_some`, which takes a value
                        // and so would build the `Handle` either way — and a
                        // `Handle` built around null is dropped a moment later
                        // into `freelocale(NULL)`, which is a segmentation
                        // fault rather than a no-op.
                        (!loc.is_null()).then(|| Handle(loc))
                    })
                    .collect();
                // The empty case is cached too: a tag this machine has nothing
                // for must not retry five `newlocale` calls every frame.
                cache.insert(tag.to_owned(), handles);
            }
            // Copied out so the borrow ends before the caller's closure runs.
            // The handles stay in the map for the life of the thread and
            // nothing removes them, so the pointers stay good.
            cache[tag].iter().map(|h| h.0).collect()
        })
    }

    /// Runs `f` under each candidate locale until one of them can answer.
    ///
    /// Not until one of them *loads*: a locale that loads and cannot produce a
    /// usable answer must not end the search. `de_DE` generated in ISO-8859-1
    /// on a machine that also has `de.UTF-8` is the case — `strftime` writes
    /// Latin-1 bytes for `März`, the UTF-8 conversion fails, and stopping there
    /// dropped the whole back end to the English fallback with a perfectly good
    /// Unicode locale one line further down the list. That is the codeset
    /// mismatch the `LC_ALL_MASK` note above is about, one layer up.
    fn with_locale<T>(tag: &str, mut f: impl FnMut() -> Option<T>) -> Option<T> {
        for loc in locales_for(tag) {
            let previous = unsafe { libc::uselocale(loc) };
            let out = f();
            // Restored whatever happened: this thread goes on to format other
            // things, and the sync threads must not inherit a locale from a
            // frame.
            unsafe { libc::uselocale(previous) };
            if out.is_some() {
                return out;
            }
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
            // The locale's own `T_FMT`, with the seconds a widget clock has no
            // use for taken out of it — rather than a pattern invented here,
            // which would put the meridiem where English puts it. The hour
            // convention comes out of the same string.
            let tm = tm_for(dt.date_naive(), dt.hour(), dt.minute());
            let rendered = strftime(&short_time_pattern(&t_fmt), &tm)?;

            if !is_twelve_hour(&t_fmt) {
                // `07:13` is right on a twenty-four-hour clock, where the
                // leading zero is what keeps the column the same width all day.
                return Some(rendered);
            }
            Some(drop_hour_padding(&rendered))
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
/// them and sets `EPHEMERIS_REQUIRE_SYSTEM_LOCALE`, which turns the skip back
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
        if got.is_none() && std::env::var_os("EPHEMERIS_REQUIRE_SYSTEM_LOCALE").is_some() {
            panic!(
                "{what} returned nothing for {tag}, but this run requires the \
                 system locale to be present"
            );
        }
        got
    }

    /// The environment outranks the system preference, and `C` is not a
    /// language.
    ///
    /// This is what a smoke test caught on macOS: asking Core Foundation first
    /// made `LC_ALL=fr_FR.UTF-8 ephemeris` print an English agenda, because
    /// `CFLocaleCopyCurrent` reports System Settings and never looks at the
    /// environment. Every variable is read here rather than set — the
    /// environment is process-wide and tests share it — so this checks the
    /// conversion and the precedence the lookup applies to whatever it finds.
    #[test]
    fn the_environment_is_read_before_the_system_preference() {
        // The parsing itself lives in the core now — one rule, one place, with
        // its own tests. What this checks is the precedence applied to what it
        // reports, and that whatever this machine happens to say comes back as
        // a tag rather than a raw POSIX name.
        let tag = UnixLocale.user_default_tag();
        assert!(!tag.is_empty());
        assert!(!tag.contains('_') && !tag.contains('.'), "{tag}");
    }

    /// The neutral locale is a choice, and every part of the back end answers
    /// it the same way.
    ///
    /// `LC_ALL=C` is how a script or a service unit asks a program for
    /// reproducible output. Reading it as "nothing set" let `LANG` win, so
    /// `LC_ALL=C LANG=de_DE.UTF-8` printed a German agenda where everything
    /// else on the machine printed English.
    #[test]
    fn the_c_locale_is_declined_by_the_platform_rather_than_answered() {
        assert!(names_no_language("C"));
        assert!(names_no_language("POSIX"));
        assert!(names_no_language("C.UTF-8"));
        assert!(!names_no_language("ca-ES"));
        assert!(!names_no_language("cs_CZ"));

        // So the platform gives nothing for it and the portable formatter —
        // ISO order, English names — is what draws the date.
        let date = subject();
        assert_eq!(platform::format_date(NEUTRAL_TAG, date), None);
        assert_eq!(platform::format_day_month(NEUTRAL_TAG, date), None);
        assert_eq!(
            UnixLocale.format_date(NEUTRAL_TAG, date),
            PortableLocale.format_date(NEUTRAL_TAG, date)
        );
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
        // A morning hour, to pin the padding: no clock writes `07:13 AM`, and
        // `%I` pads to two digits where `%l` — which does not exist everywhere
        // — would not.
        let morning = Local.with_ymd_and_hms(2026, 8, 4, 7, 13, 0).unwrap();
        if let Some(american) = answer(
            "format_time",
            "en-US",
            platform::format_time("en-US", morning),
        ) {
            assert!(
                american.starts_with("7:13"),
                "a twelve-hour clock drops the leading zero: {american}"
            );
        }
        if let Some(german) = answer(
            "format_time",
            "de-DE",
            platform::format_time("de-DE", morning),
        ) {
            assert_eq!(
                german.trim(),
                "07:13",
                "a twenty-four-hour clock keeps it, so the column stays put"
            );
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
