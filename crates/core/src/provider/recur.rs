// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Ephemeris contributors
//! Recurrence rules (RFC 5545 section 3.3.10), for the one source that needs
//! them expanded on this side: a subscribed `.ics` feed.
//!
//! CalDAV does not come through here. Its server expands recurrences on
//! request, and the module comment in [`super::ical`] explains why that is the
//! better place for it. A feed is a plain file over HTTP, though — nobody on
//! the other end will expand anything — and a feed without its weekly
//! meetings and yearly holidays would be a feed with almost nothing in it.
//!
//! What is implemented is what calendars actually write: `FREQ` from daily to
//! yearly, `INTERVAL`, `COUNT`, `UNTIL`, `BYDAY` with and without ordinals,
//! `BYMONTHDAY` (negative too), `BYMONTH`, `BYSETPOS` and `WKST`. That covers
//! "every weekday", "the last Friday of the month", "the second Tuesday in
//! November" and every daylight saving rule in a `VTIMEZONE`.
//!
//! What is not — `BYYEARDAY`, `BYWEEKNO`, and the sub-daily frequencies with
//! their `BYHOUR`/`BYMINUTE` — is *reported*, not approximated: [`Rule::parse`]
//! rejects a rule it would expand wrongly, and the caller shows the first
//! occurrence and says so in the log. A wrong recurrence puts a meeting on a
//! day it is not on, which is worse than a missing one.
//!
//! The engine works on calendar dates only. Time of day and time zone are the
//! caller's business: an occurrence keeps the wall-clock time of `DTSTART` in
//! the zone of `DTSTART`, which is what makes a 09:00 meeting stay at 09:00
//! across a daylight saving change.

use chrono::{Datelike, Duration, NaiveDate, Weekday};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freq {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

/// A weekday with an optional ordinal: `MO`, `2TU`, `-1FR`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByDay {
    pub ordinal: Option<i32>,
    pub weekday: Weekday,
}

/// A parsed `RRULE`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub freq: Freq,
    pub interval: u32,
    pub count: Option<u32>,
    /// Inclusive last date. The time part of a timed `UNTIL` is dropped by the
    /// caller after converting it to the series' own zone; see [`Until`].
    pub until: Option<Until>,
    pub by_day: Vec<ByDay>,
    pub by_month_day: Vec<i32>,
    pub by_month: Vec<u32>,
    pub by_set_pos: Vec<i32>,
    pub week_start: Weekday,
}

/// `UNTIL` as written: a date, or a timestamp that still has to be put into
/// the series' zone before it can be compared with an occurrence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Until {
    Date(NaiveDate),
    /// `YYYYMMDDTHHMMSS`, with `utc` set when it ended in `Z`.
    DateTime {
        at: chrono::NaiveDateTime,
        utc: bool,
    },
}

/// Why a rule cannot be expanded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unsupported(pub String);

impl std::fmt::Display for Unsupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Hard ceiling on the periods walked for one series.
///
/// A daily rule that started in 1990 is about 13 000 periods to today, which
/// is cheap; a malformed feed must not be able to make it a billion.
const MAX_PERIODS: u32 = 100_000;

impl Rule {
    pub fn parse(value: &str) -> Result<Rule, Unsupported> {
        let mut freq = None;
        let mut rule = Rule {
            freq: Freq::Daily,
            interval: 1,
            count: None,
            until: None,
            by_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month: Vec::new(),
            by_set_pos: Vec::new(),
            week_start: Weekday::Mon,
        };

        for part in value.split(';').filter(|p| !p.trim().is_empty()) {
            let (key, val) = part
                .split_once('=')
                .ok_or_else(|| Unsupported(format!("malformed rule part '{part}'")))?;
            let val = val.trim();
            match key.trim().to_ascii_uppercase().as_str() {
                "FREQ" => {
                    freq = Some(match val.to_ascii_uppercase().as_str() {
                        "DAILY" => Freq::Daily,
                        "WEEKLY" => Freq::Weekly,
                        "MONTHLY" => Freq::Monthly,
                        "YEARLY" => Freq::Yearly,
                        other => return Err(Unsupported(format!("FREQ={other}"))),
                    })
                }
                "INTERVAL" => {
                    rule.interval = val
                        .parse()
                        .ok()
                        .filter(|n| *n > 0)
                        .ok_or_else(|| Unsupported(format!("INTERVAL={val}")))?
                }
                "COUNT" => {
                    rule.count = Some(
                        val.parse()
                            .map_err(|_| Unsupported(format!("COUNT={val}")))?,
                    )
                }
                "UNTIL" => rule.until = Some(parse_until(val)?),
                "BYDAY" => {
                    rule.by_day = list(val, parse_by_day)?;
                }
                "BYMONTHDAY" => {
                    rule.by_month_day = list(val, |v| {
                        v.parse::<i32>()
                            .ok()
                            .filter(|d| (1..=31).contains(&d.abs()))
                    })?
                }
                "BYMONTH" => {
                    rule.by_month = list(val, |v| {
                        v.parse::<u32>().ok().filter(|m| (1..=12).contains(m))
                    })?
                }
                "BYSETPOS" => {
                    rule.by_set_pos = list(val, |v| {
                        v.parse::<i32>().ok().filter(|p| *p != 0 && p.abs() <= 366)
                    })?
                }
                "WKST" => {
                    rule.week_start =
                        weekday(val).ok_or_else(|| Unsupported(format!("WKST={val}")))?
                }
                // Selecting hours, minutes or seconds within a day only means
                // something to a sub-daily rule, and those are rejected above.
                // A value that merely repeats DTSTART's own time changes
                // nothing; anything else would, so it is refused.
                "BYHOUR" | "BYMINUTE" | "BYSECOND" if val.split(',').count() <= 1 => {}
                other @ ("BYYEARDAY" | "BYWEEKNO" | "BYHOUR" | "BYMINUTE" | "BYSECOND") => {
                    return Err(Unsupported(format!("{other} is not supported")));
                }
                // RSCALE and SKIP (RFC 7529) and anything newer.
                other => return Err(Unsupported(format!("{other} is not supported"))),
            }
        }

        rule.freq = freq.ok_or_else(|| Unsupported("rule without FREQ".into()))?;
        // An ordinal on BYDAY selects within a month or a year. Inside a week
        // or a day it has no meaning, and RFC 5545 forbids it there.
        if matches!(rule.freq, Freq::Daily | Freq::Weekly)
            && rule.by_day.iter().any(|d| d.ordinal.is_some())
        {
            return Err(Unsupported(
                "BYDAY ordinal in a daily or weekly rule".into(),
            ));
        }
        Ok(rule)
    }

    /// Every date the rule produces from `start` up to and including `last`,
    /// in order, with `start` itself always first.
    ///
    /// `until` is the already-converted inclusive end, if any; see
    /// [`Rule::until`]. `COUNT` counts from `start`, so dates before the
    /// window the caller cares about still have to be walked.
    pub fn dates(
        &self,
        start: NaiveDate,
        until: Option<NaiveDate>,
        last: NaiveDate,
    ) -> Vec<NaiveDate> {
        let end = match until {
            Some(u) => u.min(last),
            None => last,
        };
        let mut out = Vec::new();
        if start > end {
            return out;
        }
        let mut produced: u32 = 0;
        let limit = self.count.unwrap_or(u32::MAX);

        // DTSTART is always the first instance, whether or not the rule
        // would have produced it (RFC 5545 section 3.8.5.3).
        out.push(start);
        produced += 1;
        if produced >= limit {
            return out;
        }

        for step in 0..MAX_PERIODS {
            let Some(period) = self.period(start, step) else {
                break;
            };
            if period_start(self.freq, period, self.week_start) > end {
                break;
            }
            let mut candidates = self.candidates(start, period);
            candidates.sort_unstable();
            candidates.dedup();
            let candidates = apply_set_pos(candidates, &self.by_set_pos);

            for date in candidates {
                if date <= start {
                    continue;
                }
                if date > end {
                    return out;
                }
                out.push(date);
                produced += 1;
                if produced >= limit {
                    return out;
                }
            }
        }
        out
    }

    /// Any date in the `step`-th period after the one containing `start`.
    fn period(&self, start: NaiveDate, step: u32) -> Option<NaiveDate> {
        let n = i64::from(step) * i64::from(self.interval);
        match self.freq {
            Freq::Daily => start.checked_add_signed(Duration::days(n)),
            Freq::Weekly => start.checked_add_signed(Duration::weeks(n)),
            Freq::Monthly => {
                let months = i64::from(start.month0()) + n;
                let year = i64::from(start.year()) + months.div_euclid(12);
                NaiveDate::from_ymd_opt(
                    i32::try_from(year).ok()?,
                    months.rem_euclid(12) as u32 + 1,
                    1,
                )
            }
            Freq::Yearly => {
                NaiveDate::from_ymd_opt(start.year().checked_add(i32::try_from(n).ok()?)?, 1, 1)
            }
        }
    }

    /// The dates a period offers before `BYSETPOS`.
    fn candidates(&self, start: NaiveDate, period: NaiveDate) -> Vec<NaiveDate> {
        let month_ok =
            |d: &NaiveDate| self.by_month.is_empty() || self.by_month.contains(&d.month());
        let weekday_ok = |d: &NaiveDate| {
            self.by_day.is_empty() || self.by_day.iter().any(|b| b.weekday == d.weekday())
        };
        let monthday_ok = |d: &NaiveDate| {
            self.by_month_day.is_empty()
                || self
                    .by_month_day
                    .iter()
                    .any(|&md| resolve_month_day(d.year(), d.month(), md) == Some(d.day()))
        };

        match self.freq {
            Freq::Daily => [period]
                .into_iter()
                .filter(|d| month_ok(d) && monthday_ok(d) && weekday_ok(d))
                .collect(),
            Freq::Weekly => {
                let first = period_start(Freq::Weekly, period, self.week_start);
                let days: Vec<Weekday> = if self.by_day.is_empty() {
                    vec![start.weekday()]
                } else {
                    self.by_day.iter().map(|b| b.weekday).collect()
                };
                (0..7)
                    .map(|i| first + Duration::days(i))
                    .filter(|d| days.contains(&d.weekday()) && month_ok(d))
                    .collect()
            }
            Freq::Monthly => {
                if !month_ok(&period) {
                    return Vec::new();
                }
                self.in_month(start, period.year(), period.month())
            }
            Freq::Yearly => {
                let year = period.year();
                // BYDAY with no BYMONTH and no BYMONTHDAY counts its ordinals
                // across the whole year: `BYDAY=20MO` is the twentieth Monday.
                if !self.by_day.is_empty()
                    && self.by_month.is_empty()
                    && self.by_month_day.is_empty()
                {
                    let (Some(first), Some(last)) = (
                        NaiveDate::from_ymd_opt(year, 1, 1),
                        NaiveDate::from_ymd_opt(year, 12, 31),
                    ) else {
                        return Vec::new();
                    };
                    return by_day_in(first, last, &self.by_day);
                }
                let months: Vec<u32> = if self.by_month.is_empty() {
                    vec![start.month()]
                } else {
                    self.by_month.clone()
                };
                months
                    .into_iter()
                    .flat_map(|m| self.in_month(start, year, m))
                    .collect()
            }
        }
    }

    /// The dates one month offers to a monthly or yearly rule.
    fn in_month(&self, start: NaiveDate, year: i32, month: u32) -> Vec<NaiveDate> {
        let Some(first) = NaiveDate::from_ymd_opt(year, month, 1) else {
            return Vec::new();
        };
        let last = last_of_month(first);

        let from_month_days: Option<Vec<NaiveDate>> = (!self.by_month_day.is_empty()).then(|| {
            self.by_month_day
                .iter()
                .filter_map(|&md| resolve_month_day(year, month, md))
                .filter_map(|d| NaiveDate::from_ymd_opt(year, month, d))
                .collect()
        });
        let from_weekdays: Option<Vec<NaiveDate>> =
            (!self.by_day.is_empty()).then(|| by_day_in(first, last, &self.by_day));

        match (from_month_days, from_weekdays) {
            (Some(days), Some(weekdays)) => {
                days.into_iter().filter(|d| weekdays.contains(d)).collect()
            }
            (Some(days), None) => days,
            (None, Some(weekdays)) => weekdays,
            // Neither: the day of the month DTSTART is on. A month too short
            // for it is skipped, not clamped — RFC 5545 is explicit that the
            // 31st of every month does not happen in April.
            (None, None) => NaiveDate::from_ymd_opt(year, month, start.day())
                .into_iter()
                .collect(),
        }
    }
}

/// The first day of the period a date falls in.
fn period_start(freq: Freq, date: NaiveDate, week_start: Weekday) -> NaiveDate {
    match freq {
        Freq::Daily => date,
        Freq::Weekly => {
            let back =
                (7 + date.weekday().num_days_from_monday() - week_start.num_days_from_monday()) % 7;
            date - Duration::days(i64::from(back))
        }
        Freq::Monthly => date.with_day(1).unwrap_or(date),
        Freq::Yearly => NaiveDate::from_ymd_opt(date.year(), 1, 1).unwrap_or(date),
    }
}

/// Days in `first..=last` matching `BYDAY`, ordinals counted within that span.
fn by_day_in(first: NaiveDate, last: NaiveDate, by_day: &[ByDay]) -> Vec<NaiveDate> {
    let mut out = Vec::new();
    for b in by_day {
        let matching: Vec<NaiveDate> = first
            .iter_days()
            .take_while(|d| *d <= last)
            .filter(|d| d.weekday() == b.weekday)
            .collect();
        match b.ordinal {
            None => out.extend(matching),
            Some(n) => {
                if let Some(d) = pick(&matching, n) {
                    out.push(d);
                }
            }
        }
    }
    out
}

/// The `n`-th entry, counting from 1, or from the end when negative.
fn pick<T: Copy>(items: &[T], n: i32) -> Option<T> {
    let len = i32::try_from(items.len()).ok()?;
    let index = if n > 0 { n - 1 } else { len + n };
    (0..len).contains(&index).then(|| items[index as usize])
}

fn apply_set_pos(candidates: Vec<NaiveDate>, positions: &[i32]) -> Vec<NaiveDate> {
    if positions.is_empty() {
        return candidates;
    }
    let mut out: Vec<NaiveDate> = positions
        .iter()
        .filter_map(|&p| pick(&candidates, p))
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// `BYMONTHDAY=-1` is the last day of the month; `31` does not exist in June.
fn resolve_month_day(year: i32, month: u32, md: i32) -> Option<u32> {
    let len = last_of_month(NaiveDate::from_ymd_opt(year, month, 1)?).day() as i32;
    let day = if md > 0 { md } else { len + md + 1 };
    (1..=len).contains(&day).then_some(day as u32)
}

fn last_of_month(first: NaiveDate) -> NaiveDate {
    let next = if first.month() == 12 {
        NaiveDate::from_ymd_opt(first.year() + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(first.year(), first.month() + 1, 1)
    };
    next.map(|n| n - Duration::days(1)).unwrap_or(first)
}

fn list<T>(val: &str, item: impl Fn(&str) -> Option<T>) -> Result<Vec<T>, Unsupported> {
    val.split(',')
        .map(|v| item(v.trim()).ok_or_else(|| Unsupported(format!("cannot read '{v}' in '{val}'"))))
        .collect()
}

fn weekday(s: &str) -> Option<Weekday> {
    Some(match s.to_ascii_uppercase().as_str() {
        "MO" => Weekday::Mon,
        "TU" => Weekday::Tue,
        "WE" => Weekday::Wed,
        "TH" => Weekday::Thu,
        "FR" => Weekday::Fri,
        "SA" => Weekday::Sat,
        "SU" => Weekday::Sun,
        _ => return None,
    })
}

fn parse_by_day(s: &str) -> Option<ByDay> {
    // The weekday is the last two bytes. Splitting by byte offset with
    // `split_at` panics when that lands inside a multi-byte character, and a
    // feed is untrusted text: `get` turns that into a refused rule instead.
    let at = s.len().checked_sub(2)?;
    let (num, day) = (s.get(..at)?, s.get(at..)?);
    let ordinal = match num.trim_start_matches('+') {
        "" => None,
        n => Some(n.parse::<i32>().ok().filter(|n| *n != 0 && n.abs() <= 53)?),
    };
    Some(ByDay {
        ordinal,
        weekday: weekday(day)?,
    })
}

fn parse_until(val: &str) -> Result<Until, Unsupported> {
    let bad = || Unsupported(format!("UNTIL={val}"));
    if val.len() == 8 {
        return NaiveDate::parse_from_str(val, "%Y%m%d")
            .map(Until::Date)
            .map_err(|_| bad());
    }
    let (body, utc) = match val.strip_suffix(['Z', 'z']) {
        Some(b) => (b, true),
        None => (val, false),
    };
    chrono::NaiveDateTime::parse_from_str(body, "%Y%m%dT%H%M%S")
        .map(|at| Until::DateTime { at, utc })
        .map_err(|_| bad())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn expand(rule: &str, start: NaiveDate, last: NaiveDate) -> Vec<NaiveDate> {
        let rule = Rule::parse(rule).expect("rule parses");
        let until = match rule.until {
            Some(Until::Date(u)) => Some(u),
            Some(Until::DateTime { at, .. }) => Some(at.date()),
            None => None,
        };
        rule.dates(start, until, last)
    }

    fn fmt(dates: &[NaiveDate]) -> Vec<String> {
        dates
            .iter()
            .map(|d| d.format("%Y-%m-%d").to_string())
            .collect()
    }

    // The examples below are the ones RFC 5545 section 3.8.5.3 gives, with
    // the dates it lists.

    #[test]
    fn daily_for_ten_occurrences() {
        let got = expand("FREQ=DAILY;COUNT=10", d(1997, 9, 2), d(1998, 1, 1));
        assert_eq!(got.len(), 10);
        assert_eq!(got[9], d(1997, 9, 11));
    }

    #[test]
    fn every_other_day_until_a_date() {
        let got = expand(
            "FREQ=DAILY;INTERVAL=2;UNTIL=19970910",
            d(1997, 9, 2),
            d(1998, 1, 1),
        );
        assert_eq!(
            fmt(&got),
            [
                "1997-09-02",
                "1997-09-04",
                "1997-09-06",
                "1997-09-08",
                "1997-09-10"
            ]
        );
    }

    #[test]
    fn weekly_on_tuesday_and_thursday_for_five_weeks() {
        let got = expand(
            // The RFC writes UNTIL=19971007T000000Z for a 09:00 series; the
            // time of day is the caller's to compare, so the date before it is
            // what this level sees.
            "FREQ=WEEKLY;UNTIL=19971006;WKST=SU;BYDAY=TU,TH",
            d(1997, 9, 2),
            d(1998, 1, 1),
        );
        assert_eq!(
            fmt(&got),
            [
                "1997-09-02",
                "1997-09-04",
                "1997-09-09",
                "1997-09-11",
                "1997-09-16",
                "1997-09-18",
                "1997-09-23",
                "1997-09-25",
                "1997-09-30",
                "1997-10-02"
            ]
        );
    }

    #[test]
    fn every_other_week_depends_on_the_week_start() {
        // The RFC's own example of why WKST matters.
        let mo = expand(
            "FREQ=WEEKLY;INTERVAL=2;COUNT=4;BYDAY=TU,SU;WKST=MO",
            d(1997, 8, 5),
            d(1998, 1, 1),
        );
        assert_eq!(
            fmt(&mo),
            ["1997-08-05", "1997-08-10", "1997-08-19", "1997-08-24"]
        );
        let su = expand(
            "FREQ=WEEKLY;INTERVAL=2;COUNT=4;BYDAY=TU,SU;WKST=SU",
            d(1997, 8, 5),
            d(1998, 1, 1),
        );
        assert_eq!(
            fmt(&su),
            ["1997-08-05", "1997-08-17", "1997-08-19", "1997-08-31"]
        );
    }

    #[test]
    fn every_weekday() {
        let got = expand(
            "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR",
            d(2026, 11, 2),
            d(2026, 11, 10),
        );
        assert_eq!(
            fmt(&got),
            [
                "2026-11-02",
                "2026-11-03",
                "2026-11-04",
                "2026-11-05",
                "2026-11-06",
                "2026-11-09",
                "2026-11-10"
            ]
        );
    }

    #[test]
    fn first_friday_of_the_month() {
        let got = expand(
            "FREQ=MONTHLY;COUNT=10;BYDAY=1FR",
            d(1997, 9, 5),
            d(1999, 1, 1),
        );
        assert_eq!(
            fmt(&got[..4]),
            ["1997-09-05", "1997-10-03", "1997-11-07", "1997-12-05"]
        );
        assert_eq!(got.len(), 10);
    }

    #[test]
    fn last_day_and_second_to_last_monday() {
        let got = expand("FREQ=MONTHLY;BYMONTHDAY=-1", d(2026, 1, 31), d(2026, 4, 30));
        assert_eq!(
            fmt(&got),
            ["2026-01-31", "2026-02-28", "2026-03-31", "2026-04-30"]
        );

        let got = expand(
            "FREQ=MONTHLY;COUNT=6;BYDAY=-2MO",
            d(1997, 9, 22),
            d(1999, 1, 1),
        );
        assert_eq!(
            fmt(&got),
            [
                "1997-09-22",
                "1997-10-20",
                "1997-11-17",
                "1997-12-22",
                "1998-01-19",
                "1998-02-16"
            ]
        );
    }

    #[test]
    fn the_thirty_first_skips_short_months() {
        let got = expand("FREQ=MONTHLY;COUNT=4", d(2026, 1, 31), d(2027, 1, 1));
        assert_eq!(
            fmt(&got),
            ["2026-01-31", "2026-03-31", "2026-05-31", "2026-07-31"]
        );
    }

    #[test]
    fn last_workday_of_the_month_via_set_pos() {
        let got = expand(
            "FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1",
            d(1997, 9, 30),
            d(1998, 1, 31),
        );
        assert_eq!(
            fmt(&got),
            [
                "1997-09-30",
                "1997-10-31",
                "1997-11-28",
                "1997-12-31",
                "1998-01-30"
            ]
        );
    }

    #[test]
    fn friday_the_thirteenth() {
        let got = expand(
            "FREQ=MONTHLY;BYDAY=FR;BYMONTHDAY=13",
            d(1998, 2, 13),
            d(1999, 1, 1),
        );
        assert_eq!(fmt(&got), ["1998-02-13", "1998-03-13", "1998-11-13"]);
    }

    #[test]
    fn yearly_birthday_including_the_leap_day() {
        let got = expand("FREQ=YEARLY", d(2024, 2, 29), d(2033, 1, 1));
        // Only leap years have a 29 February.
        assert_eq!(fmt(&got), ["2024-02-29", "2028-02-29", "2032-02-29"]);
    }

    #[test]
    fn thanksgiving_and_the_daylight_saving_rules() {
        let got = expand(
            "FREQ=YEARLY;BYMONTH=11;BYDAY=4TH",
            d(2024, 11, 28),
            d(2026, 12, 31),
        );
        assert_eq!(fmt(&got), ["2024-11-28", "2025-11-27", "2026-11-26"]);

        // How every European VTIMEZONE describes the switch to summer time.
        let got = expand(
            "FREQ=YEARLY;BYMONTH=3;BYDAY=-1SU",
            d(1996, 3, 31),
            d(2027, 1, 1),
        );
        assert_eq!(got.last(), Some(&d(2026, 3, 29)));
    }

    #[test]
    fn the_twentieth_monday_of_the_year() {
        let got = expand("FREQ=YEARLY;BYDAY=20MO", d(1997, 5, 19), d(1999, 12, 31));
        assert_eq!(fmt(&got), ["1997-05-19", "1998-05-18", "1999-05-17"]);
    }

    #[test]
    fn dtstart_counts_even_when_the_rule_would_not_produce_it() {
        // Starts on a Wednesday, repeats on Mondays.
        let got = expand(
            "FREQ=WEEKLY;COUNT=3;BYDAY=MO",
            d(2026, 11, 4),
            d(2027, 1, 1),
        );
        assert_eq!(fmt(&got), ["2026-11-04", "2026-11-09", "2026-11-16"]);
    }

    #[test]
    fn expansion_stops_at_the_requested_end() {
        let got = expand("FREQ=DAILY", d(1990, 1, 1), d(2026, 11, 2));
        assert_eq!(got.last(), Some(&d(2026, 11, 2)));
    }

    #[test]
    fn rules_that_would_be_expanded_wrongly_are_refused() {
        for rule in [
            "FREQ=HOURLY",
            "FREQ=YEARLY;BYWEEKNO=20",
            "FREQ=YEARLY;BYYEARDAY=100",
            "FREQ=WEEKLY;BYDAY=2MO",
            "FREQ=DAILY;BYHOUR=9,17",
            "INTERVAL=2",
            "FREQ=DAILY;INTERVAL=0",
            "FREQ=YEARLY;RSCALE=HEBREW",
            // Malformed text must be refused, not panic: the split for the
            // weekday would fall inside the `Ö`.
            "FREQ=WEEKLY;BYDAY=aÖb",
            "FREQ=WEEKLY;BYDAY=Ö",
        ] {
            assert!(Rule::parse(rule).is_err(), "{rule} should be refused");
        }
        // A single BYHOUR merely restates DTSTART's time, as Outlook writes it.
        assert!(Rule::parse("FREQ=DAILY;BYHOUR=9").is_ok());
    }

    #[test]
    fn a_timed_until_is_kept_with_its_zone_marker() {
        let rule = Rule::parse("FREQ=DAILY;UNTIL=20261102T080000Z").unwrap();
        assert!(matches!(
            rule.until,
            Some(Until::DateTime { utc: true, .. })
        ));
        let rule = Rule::parse("FREQ=DAILY;UNTIL=20261102T080000").unwrap();
        assert!(matches!(
            rule.until,
            Some(Until::DateTime { utc: false, .. })
        ));
    }
}
