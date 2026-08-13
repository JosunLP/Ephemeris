// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! Where everything goes, in device-independent pixels.
//!
//! The renderer draws; this decides what to draw and where. That split matters
//! now that there is more than one renderer coming: where a row starts, how
//! wide the time column has to be, which rows are visible at all, and which
//! rectangle a click landed in are the same answers on Direct2D, Core Graphics
//! and Cairo. Written once, they stay the same answers; written per front end,
//! they drift, and the second widget is subtly not the first.
//!
//! Everything here is arithmetic over [`Metrics`] and the model. No colours —
//! those are [`crate::theme`] — and nothing that needs a device: a headless
//! test can compute a whole frame's geometry and check it.
//!
//! **The one thing this cannot do is measure text.** How wide "yesterday" is
//! depends on the font, the size and the shaping engine, so the front end
//! measures and passes the number in. [`column_width`] is the shape that takes:
//! the platform supplies the widest measured entry, this decides what to do
//! with it. Fixed column widths are the classic internationalisation trap —
//! what fits `gestern` truncates `yesterday`, and `المتأخرة` all the more.
//!
//! **Coordinates are unmirrored.** Right-to-left layout is applied by the
//! renderer at the drawing primitives, by folding x across
//! [`Panel::mirror_axis`]. Everything in this module goes on computing left to
//! right, which is what keeps one set of arithmetic serving both directions.

use crate::i18n::Locale;
use crate::model::{Agenda, Event, Task};
use crate::theme::Metrics;
use chrono::{DateTime, Local, NaiveDate, Timelike};

/// A rectangle in device-independent pixels, relative to the window's corner.
///
/// Field order matches Direct2D's `D2D_RECT_F` so the Windows front end can
/// convert with a struct literal rather than a function call, but nothing here
/// depends on that.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl Rect {
    pub const fn new(left: f32, top: f32, right: f32, bottom: f32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    pub fn width(&self) -> f32 {
        self.right - self.left
    }

    pub fn height(&self) -> f32 {
        self.bottom - self.top
    }

    pub fn center_x(&self) -> f32 {
        (self.left + self.right) * 0.5
    }

    pub fn center_y(&self) -> f32 {
        (self.top + self.bottom) * 0.5
    }

    /// Half-open on the right and bottom edges, so two rows that share a
    /// boundary cannot both claim the same pixel.
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }

    pub fn inset(&self, by: f32) -> Rect {
        Rect::new(
            self.left + by,
            self.top + by,
            self.right - by,
            self.bottom - by,
        )
    }

    /// Folded across a vertical axis, for right-to-left layout. Left and right
    /// swap roles in the process, which is why this returns a rectangle rather
    /// than mirroring in place.
    pub fn mirrored(&self, axis: f32) -> Rect {
        Rect::new(axis - self.right, self.top, axis - self.left, self.bottom)
    }
}

/// What a click at a point means.
///
/// Lives here rather than with a renderer because the window translates a click
/// into one of these and never draws anything, and because every front end
/// needs the identical set.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Hit {
    Refresh,
    /// The circle before a task -> tick it off.
    TaskCheck(usize),
    /// A task row -> open it in the web interface.
    Task(usize),
    /// The row of a task whose completion can still be undone.
    Undo(usize),
    /// An event row -> open it in the calendar.
    Event(usize),
    /// The highlighted event in the header area.
    Hero(usize),
    /// An event from tomorrow's preview.
    Tomorrow(usize),
    /// A status line that needs action (setup, sign-in or configuration).
    StatusAction,
}

#[derive(Debug, Clone, Copy)]
pub struct HitRegion {
    pub rect: Rect,
    pub hit: Hit,
}

impl HitRegion {
    /// `x` and `y` are unmirrored, as everything else here is. A front end
    /// holding a raw client position has to put it through [`hit_point`] first.
    pub fn contains(&self, x: f32, y: f32) -> bool {
        self.rect.contains(x, y)
    }
}

/// A raw client position in the coordinates the hit rectangles are written in.
///
/// The module note above says coordinates are unmirrored and that right-to-left
/// is folded at the drawing primitives. That holds for the *drawing*; a click
/// arrives from the window system in the mirrored coordinates the user is
/// actually looking at, so it has to be folded back before it is compared
/// against anything in here.
///
/// Skipping this is invisible for the full-width rows, which is how it
/// survived, and quite visible for the regions that are not: in an Arabic
/// layout the refresh button is drawn at the left while its rectangle stays on
/// the right, so the button does nothing and the empty opposite corner
/// refreshes — and clicking a task's tick circle opens the task in the browser
/// while clicking the far end of the row ticks it off.
pub fn hit_point(panel: &Panel, rtl: bool, x: f32, y: f32) -> (f32, f32) {
    (if rtl { panel.mirror_axis - x } else { x }, y)
}

/// A task that has been ticked off but not yet sent, and can still be undone.
#[derive(Debug, Clone, Copy)]
pub struct UndoView<'a> {
    pub task_id: &'a str,
    /// 1.0 right after the click, 0.0 when it is sent.
    pub remaining: f32,
}

/// Everything a renderer needs for one frame.
///
/// Here rather than beside a renderer because every front end draws from
/// exactly this and nothing else. A field added for one of them would
/// otherwise be a field the others silently do not draw.
pub struct Frame<'a> {
    pub agenda: &'a Agenda,
    pub loc: &'a Locale,
    pub status: &'a crate::sync::Status,
    pub anim: &'a crate::anim::Animations,
    pub now: DateTime<Local>,
    pub hover: Option<Hit>,
    pub sync_minutes: u32,
    pub opacity: f32,
    pub show_past_events: bool,
    pub undo: Option<UndoView<'a>>,
    /// A syntax error in `config.json`; outranks the sync status line.
    pub config_error: Option<&'a str>,
    /// Version of a newer release, if the daily check found one.
    pub update: Option<&'a str>,
}

/// What a frame turned out to need, which is what bounds the scrolling.
#[derive(Debug, Clone, Copy)]
pub struct FrameResult {
    /// Total height of the content — the basis for limiting the scroll.
    pub content_height: f32,
    pub viewport_height: f32,
}

/// A list row bleeds this far past the content columns on each side, so the
/// hover highlight reaches under the text rather than stopping flush with it.
pub const ROW_BLEED: f32 = 5.0;

/// Air between a measured column and its neighbour, so the two never touch.
const COLUMN_AIR: f32 = 6.0;

/// How far the content slides up during the reveal animation.
const REVEAL_SLIDE: f32 = 10.0;

/// The glass body and the content column inside it.
///
/// Derived once per frame; every other function here takes what it needs from
/// it rather than recomputing the inset.
#[derive(Debug, Clone, Copy)]
pub struct Panel {
    /// The glass body, inset from the window edge by the shadow margin.
    pub rect: Rect,
    /// Left edge of the content column.
    pub content_left: f32,
    /// Right edge of the content column.
    pub content_right: f32,
    /// Axis for right-to-left mirroring. The body maps onto itself across it,
    /// so the panel edges stay put while everything inside is folded.
    pub mirror_axis: f32,
}

impl Panel {
    /// `width` and `height` are the window's, in device-independent pixels.
    pub fn new(width: f32, height: f32, m: &Metrics) -> Self {
        let rect = Rect::new(m.shadow, m.shadow, width - m.shadow, height - m.shadow);
        Self {
            content_left: rect.left + m.pad,
            content_right: rect.right - m.pad,
            mirror_axis: rect.left + rect.right,
            rect,
        }
    }

    /// Top of the footer band, which is also the bottom of the scrolling area.
    pub fn content_bottom(&self, m: &Metrics) -> f32 {
        self.rect.bottom - m.footer_h
    }

    /// The clip the scrolling rows are drawn inside, so a scrolled row cannot
    /// run into the header, the footer or the rounded frame.
    ///
    /// One pixel in from each side: the body has a corner radius, and content
    /// clipped flush to the edge shows through it.
    pub fn content_clip(&self, content_top: f32, content_bottom: f32) -> Rect {
        Rect::new(
            self.rect.left + 1.0,
            content_top,
            self.rect.right - 1.0,
            content_bottom,
        )
    }

    /// The status line at the foot of the panel.
    pub fn footer_rect(&self, m: &Metrics) -> Rect {
        Rect::new(
            self.content_left,
            self.content_bottom(m) + 1.0,
            self.content_right,
            self.rect.bottom - 2.0,
        )
    }
}

/// The reveal animation's vertical offset, from its progress.
///
/// Both the drawing and the content height need it, and the height would be
/// wrong by the offset if the two were computed apart — which is a scroll limit
/// that lets the last row disappear.
pub fn reveal_slide(reveal: f32) -> f32 {
    (1.0 - reveal) * REVEAL_SLIDE
}

/// Total height of the scrolling content, from where the last row ended.
///
/// `cursor` is the y the sections finished at, which carries the scroll offset
/// and the reveal slide that were applied before drawing; both are taken back
/// out so the result is a property of the content rather than of where it
/// happens to be sitting this frame. The trailing [`Metrics::pad`] is breathing
/// room at the bottom, so the last row can be scrolled clear of the footer.
pub fn content_height(cursor: f32, scroll: f32, slide: f32, content_top: f32, m: &Metrics) -> f32 {
    cursor + scroll - slide - content_top + m.pad
}

// --- Header ------------------------------------------------------------------

/// The refresh button, in the top corner the reading ends at.
pub fn refresh_button(panel: &Panel, top: f32, m: &Metrics) -> Rect {
    let y = top + m.pad * 0.65;
    Rect::new(
        panel.content_right - m.icon_btn,
        y,
        panel.content_right,
        y + m.icon_btn,
    )
}

/// Where the day rail sits inside the header band.
pub fn day_rail_top(header_bottom: f32, m: &Metrics) -> f32 {
    header_bottom - m.rail_h - 5.0
}

// --- The day rail ------------------------------------------------------------

/// Minutes since midnight, as the rail measures them.
pub fn minutes_of_day(dt: DateTime<Local>) -> f32 {
    dt.hour() as f32 * 60.0 + dt.minute() as f32
}

/// End of an event in minutes since midnight.
///
/// With no end time an hour is assumed; if the event runs past midnight the raw
/// time would point backwards, so it is clamped to the end of the day.
pub fn end_minutes(ev: &Event, start_min: f32) -> f32 {
    match ev.end {
        Some(e) => {
            let v = e.hour() as f32 * 60.0 + e.minute() as f32;
            if v <= start_min { DAY_MINUTES } else { v }
        }
        None => start_min + 60.0,
    }
}

const DAY_MINUTES: f32 = 24.0 * 60.0;
/// Never show less of the day than this, or an empty day degenerates to a
/// span of nothing and every position lands on the same pixel.
const MIN_RAIL_SPAN: f32 = 240.0;

/// The slice of the day the rail shows.
///
/// The default window is the working day, but it stretches to cover whatever is
/// actually on. A fixed 06:00–22:00 would squash the 05:00 flight and the 23:00
/// event against the edges — precisely the outliers worth seeing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RailSpan {
    pub lo: f32,
    pub hi: f32,
}

impl RailSpan {
    pub fn of(events: &[Event], now: DateTime<Local>) -> Self {
        let now_min = minutes_of_day(now);
        let (mut lo, mut hi) = (6.0 * 60.0_f32, 22.0 * 60.0_f32);
        for ev in events {
            if ev.all_day {
                continue;
            }
            if let Some(start) = ev.start {
                let s_min = minutes_of_day(start);
                lo = lo.min(s_min);
                hi = hi.max(end_minutes(ev, s_min));
            }
        }
        lo = lo.min(now_min).max(0.0);
        hi = hi.max(now_min).min(DAY_MINUTES);
        if hi - lo < MIN_RAIL_SPAN {
            hi = (lo + MIN_RAIL_SPAN).min(DAY_MINUTES);
            lo = (hi - MIN_RAIL_SPAN).max(0.0);
        }
        Self { lo, hi }
    }

    /// x for a time, across a rail running from `x0` to `x1`.
    pub fn position(&self, minutes: f32, x0: f32, x1: f32) -> f32 {
        let span = (self.hi - self.lo).max(1.0);
        x0 + (x1 - x0) * ((minutes - self.lo) / span).clamp(0.0, 1.0)
    }

    /// Is this time on the rail at all? The now marker is suppressed when it is
    /// not, rather than being pinned to an edge where it would lie.
    pub fn contains(&self, minutes: f32) -> bool {
        (self.lo..=self.hi).contains(&minutes)
    }
}

/// A segment on the rail, already clamped to the track.
///
/// Short events would be invisible at their true width, so every segment gets a
/// minimum — the rail is there to show that something is on, and a meeting that
/// draws as nothing defeats it.
pub fn rail_segment(span: &RailSpan, ev: &Event, x0: f32, x1: f32) -> Option<(f32, f32)> {
    let start = ev.start.filter(|_| !ev.all_day)?;
    let start_min = minutes_of_day(start);
    let sx = span.position(start_min, x0, x1);
    let ex = span.position(end_minutes(ev, start_min), x0, x1);
    Some((sx, ex.max(sx + 2.5).min(x1)))
}

// --- The highlighted event ---------------------------------------------------

/// The event running now, otherwise the next one still to come.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hero {
    /// Index into [`Agenda::events`].
    pub index: usize,
    pub running: bool,
}

/// A time block whose task has just been ticked off is skipped. The card is the
/// loudest thing on the panel, and announcing something the user has just
/// declared finished is the one place where the fade of the list row would not
/// be enough.
pub fn pick_hero(agenda: &Agenda, now: DateTime<Local>) -> Option<Hero> {
    let events = &agenda.events;
    let live = |e: &&Event| !agenda.is_completing(e);
    if let Some((index, _)) = events
        .iter()
        .enumerate()
        .find(|(_, e)| e.is_now(now) && live(e))
    {
        return Some(Hero {
            index,
            running: true,
        });
    }
    events
        .iter()
        .enumerate()
        .filter(|(_, e)| !e.all_day && e.start.map(|s| s > now).unwrap_or(false) && live(e))
        .min_by_key(|(_, e)| e.start.map(|s| s.timestamp()).unwrap_or(i64::MAX))
        .map(|(index, _)| Hero {
            index,
            running: false,
        })
}

/// The card the highlighted event is drawn on.
pub fn hero_card(panel: &Panel, top: f32, m: &Metrics) -> Rect {
    let y = top + m.pad * 0.55;
    Rect::new(panel.content_left, y, panel.content_right, y + m.hero_h)
}

/// How far through a running event we are, for the progress line at the foot of
/// the card. `None` when it has no span to be a fraction of.
pub fn hero_progress(ev: &Event, now: DateTime<Local>) -> Option<f32> {
    let (s, e) = (ev.start?, ev.end?);
    let total = (e - s).num_seconds().max(1) as f32;
    Some(((now - s).num_seconds().max(0) as f32 / total).clamp(0.0, 1.0))
}

// --- Rows --------------------------------------------------------------------

/// The clickable rectangle of a list row.
pub fn row_rect(panel: &Panel, y: f32, height: f32) -> Rect {
    Rect::new(
        panel.content_left - ROW_BLEED,
        y,
        panel.content_right + ROW_BLEED,
        y + height,
    )
}

/// How far a row is held back: fully lit, faded because it is over, or faded
/// because it has just been ticked off.
///
/// One function because the two lists must agree. A task and the calendar entry
/// that is its time block fade together during the undo window, and they would
/// not if each list decided for itself.
fn dim_for(completing: bool, past: bool) -> f32 {
    if completing {
        0.42
    } else if past {
        0.55
    } else {
        1.0
    }
}

/// One visible calendar row and everything about it the drawing depends on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EventRow {
    /// Index into [`Agenda::events`] — *not* the position in the visible list.
    /// Hit regions and click handling both address the agenda.
    pub index: usize,
    /// Running now, and not already ticked off.
    pub running: bool,
    /// Over, and not running.
    pub past: bool,
    /// The task this is the time block of has just been ticked off.
    pub completing: bool,
    /// Double-booked against another entry.
    pub conflicted: bool,
}

impl EventRow {
    pub fn dim(&self) -> f32 {
        dim_for(self.completing, self.past)
    }

    /// Does the right-hand column carry a countdown for this row?
    ///
    /// Not for an all-day entry, which has no start to count to; not for one
    /// that is over, where the calendar name is the more useful thing; and not
    /// for one just ticked off, where "in 2 h" next to a finished item reads as
    /// a contradiction.
    pub fn shows_countdown(&self, ev: &Event) -> bool {
        !self.completing && !ev.all_day && !self.past
    }
}

/// The Schedule section, decided before anything is drawn.
#[derive(Debug, Clone, PartialEq)]
pub struct EventList {
    pub rows: Vec<EventRow>,
    /// More than one calendar is in play, so the calendar name earns its space.
    /// Otherwise it is the same redundant label on every row.
    pub multi_calendar: bool,
    /// Double bookings, counted in pairs.
    pub conflicts: usize,
    /// The now line goes before this row. `None` when every event is over, in
    /// which case it goes after the last one — see [`EventList::now_line_at_end`].
    pub now_line_before: Option<usize>,
    /// Every event is over and there was at least one timed event to be over.
    pub now_line_at_end: bool,
}

impl EventList {
    /// `show_past` is the setting; the running event stays visible either way,
    /// because otherwise the most important one is the one that vanishes.
    pub fn build(agenda: &Agenda, now: DateTime<Local>, show_past: bool) -> Self {
        let overlapping = crate::model::mark_overlaps(&agenda.events);
        let conflicts = overlapping.iter().filter(|&&f| f).count() / 2;

        let rows: Vec<EventRow> = agenda
            .events
            .iter()
            .enumerate()
            .filter(|(_, e)| show_past || !e.is_past(now) || e.is_now(now))
            .map(|(index, e)| {
                let completing = agenda.is_completing(e);
                let running = e.is_now(now) && !completing;
                EventRow {
                    index,
                    running,
                    past: e.is_past(now) && !running,
                    completing,
                    conflicted: overlapping.get(index).copied().unwrap_or(false),
                }
            })
            .collect();

        let multi_calendar = rows
            .iter()
            .filter(|r| !agenda.events[r.index].all_day)
            .map(|r| agenda.events[r.index].calendar_name.as_str())
            .collect::<std::collections::HashSet<_>>()
            .len()
            > 1;

        // Before the first event still to come.
        let now_line_before = rows.iter().position(|r| {
            let e = &agenda.events[r.index];
            !e.all_day && e.start.map(|s| s > now).unwrap_or(false)
        });
        let now_line_at_end =
            now_line_before.is_none() && rows.iter().any(|r| !agenda.events[r.index].all_day);

        Self {
            rows,
            multi_calendar,
            conflicts,
            now_line_before,
            now_line_at_end,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

/// One task row.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TaskRow {
    /// Index into [`Agenda::tasks`].
    pub index: usize,
    pub overdue: bool,
    /// The tick has been made but not yet sent.
    pub completing: bool,
}

impl TaskRow {
    /// A ticked-off task fades at once, so the click feels immediate rather
    /// than waiting on the API. Tasks have no "past" state of their own — an
    /// overdue one is emphasised, not held back.
    pub fn dim(&self) -> f32 {
        dim_for(self.completing, false)
    }
}

/// The Tasks section.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskList {
    pub rows: Vec<TaskRow>,
    /// More than one task list is involved, so the list name earns its space.
    pub multi_list: bool,
    pub overdue: usize,
}

impl TaskList {
    pub fn build(tasks: &[Task], today: NaiveDate) -> Self {
        let rows: Vec<TaskRow> = tasks
            .iter()
            .enumerate()
            .map(|(index, t)| TaskRow {
                index,
                overdue: t.is_overdue(today),
                completing: t.completing,
            })
            .collect();
        Self {
            multi_list: tasks
                .iter()
                .map(|t| t.tasklist_name.as_str())
                .collect::<std::collections::HashSet<_>>()
                .len()
                > 1,
            overdue: rows.iter().filter(|r| r.overdue).count(),
            rows,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

/// Geometry of one task row: the tick circle, and where the due column starts.
#[derive(Debug, Clone, Copy)]
pub struct TaskGeometry {
    /// Centre of the tick circle.
    pub check_center: (f32, f32),
    /// Left edge of the due column, clear of the circle.
    pub due_left: f32,
    /// The circle's own click target, which reaches to the row's left bleed so
    /// a click just left of the circle still ticks off rather than opening.
    pub check_hit: Rect,
}

pub fn task_geometry(panel: &Panel, y: f32, depth: u8, m: &Metrics) -> TaskGeometry {
    let indent = depth as f32 * m.indent;
    let cx = panel.content_left + indent + m.check_size * 0.5 + 1.0;
    let cy = y + m.task_row_h * 0.5;
    TaskGeometry {
        check_center: (cx, cy),
        due_left: cx + m.check_size * 0.5 + 9.0,
        check_hit: Rect::new(
            panel.content_left - ROW_BLEED,
            y,
            cx + m.check_size,
            y + m.task_row_h,
        ),
    }
}

/// The label under the tick circle: today, yesterday, or the date itself.
pub fn due_label(task: &Task, today: NaiveDate, loc: &Locale) -> String {
    match task.due {
        None => "—".into(),
        Some(d) if d == today => loc.label(loc.cat.today),
        Some(d) if (today - d).num_days() == 1 => loc.label(loc.cat.yesterday),
        // Day and month in the order the locale uses.
        Some(d) => loc.day_month(d),
    }
}

// --- Columns -----------------------------------------------------------------

/// A column's width from its widest entry, capped so a single outlier cannot
/// eat half the row.
///
/// `widest` comes from the front end, which is the only side that can measure
/// text. `max` is raised to `min` when a caller passes them the wrong way round
/// rather than panicking in `clamp`: metrics are customisable — a font size
/// offset scales some of them and not others — and a widget that dies over a
/// column width would be a poor trade for catching a bad constant.
pub fn column_width(widest: f32, min: f32, max: f32) -> f32 {
    (widest + COLUMN_AIR).clamp(min, max.max(min))
}

// --- Section labels ----------------------------------------------------------

/// A count badge beside a section heading, placed right to left so several of
/// them — overdue, conflicts — cannot overlap.
///
/// `width` is the measured text width; the return is the pill and the right
/// edge the next badge (or the heading) must stay clear of.
pub fn badge_pill(right: f32, width: f32, y: f32, m: &Metrics) -> (Rect, f32) {
    let w = width + 13.0;
    let pill = Rect::new(right - w, y + 1.0, right, y + m.section_label_h - 3.0);
    (pill, pill.left - 6.0)
}

/// The heading itself, never squeezed to nothing by the badges beside it.
pub fn section_label_rect(panel: &Panel, y: f32, right: f32, m: &Metrics) -> Rect {
    Rect::new(
        panel.content_left,
        y,
        (right - 6.0).max(panel.content_left + 20.0),
        y + m.section_label_h,
    )
}

// --- Scrollbar ---------------------------------------------------------------

/// The scroll thumb, or `None` when everything fits and there is nothing to
/// scroll.
pub fn scrollbar_thumb(
    panel: &Panel,
    top: f32,
    bottom: f32,
    content: f32,
    viewport: f32,
    scroll: f32,
    m: &Metrics,
) -> Option<Rect> {
    if content <= viewport {
        return None;
    }
    let track_h = bottom - top - 8.0;
    // A thumb proportional to a very long list would shrink to a sliver that
    // says nothing and cannot be aimed at.
    let thumb_h = (track_h * (viewport / content)).max(24.0);
    let t = (scroll / (content - viewport)).clamp(0.0, 1.0);
    let thumb_y = top + 4.0 + (track_h - thumb_h) * t;
    let x = panel.rect.right - m.pad * 0.45 - m.scrollbar_w;
    Some(Rect::new(x, thumb_y, x + m.scrollbar_w, thumb_y + thumb_h))
}

/// How far the content may be scrolled. Never negative: a list shorter than the
/// viewport has nowhere to go.
pub fn max_scroll(content: f32, viewport: f32) -> f32 {
    (content - viewport).max(0.0)
}

// --- Tooltip -----------------------------------------------------------------

/// Padding inside the tooltip box, between its border and the text.
pub const TOOLTIP_PAD: f32 = 8.0;

/// The width available to tooltip text, before the front end lays it out to
/// find out how tall the box has to be.
pub fn tooltip_text_width(panel: &Panel, m: &Metrics) -> f32 {
    (panel.rect.width() - m.pad * 2.0 - TOOLTIP_PAD * 2.0).max(1.0)
}

/// Where the tooltip box goes, once its height is known.
///
/// Below the hovered row by preference and above it otherwise, and inside the
/// content area either way — an overlay that escapes the clip would sit over
/// the header or the footer, which are not part of the list it belongs to.
pub fn tooltip_box(
    panel: &Panel,
    row: (f32, f32),
    text_height: f32,
    limits: (f32, f32),
    m: &Metrics,
) -> Rect {
    let (row_top, row_bottom) = row;
    let (top_limit, bottom_limit) = limits;
    let height = text_height + TOOLTIP_PAD * 2.0;
    let width = panel.rect.width() - m.pad * 2.0;

    let mut top = row_bottom + 3.0;
    if top + height > bottom_limit {
        top = row_top - 3.0 - height;
    }
    let top = top.clamp(top_limit, (bottom_limit - height).max(top_limit));
    Rect::new(
        panel.rect.left + m.pad,
        top,
        panel.rect.left + m.pad + width,
        top + height,
    )
}

// --- Text --------------------------------------------------------------------

/// Cuts a string to `max` characters, with an ellipsis when it had to.
///
/// Characters rather than bytes: this is used on error text from a server,
/// which is not necessarily ASCII, and slicing that by byte would panic in the
/// middle of a code point.
pub fn short(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(h: u32, min: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 8, 4, h, min, 0).unwrap()
    }

    fn timed(title: &str, from: (u32, u32), to: (u32, u32)) -> Event {
        Event {
            title: title.into(),
            start: Some(at(from.0, from.1)),
            end: Some(at(to.0, to.1)),
            all_day: false,
            location: None,
            html_link: None,
            join_url: None,
            color: 0,
            calendar_name: "Work".into(),
            calendar_id: "w".into(),
            account_id: String::new(),
            task_id: None,
        }
    }

    fn all_day(title: &str) -> Event {
        let mut e = timed(title, (0, 0), (23, 59));
        e.all_day = true;
        e.start = None;
        e.end = None;
        e
    }

    fn task(id: &str, due: Option<NaiveDate>) -> Task {
        Task {
            id: id.into(),
            tasklist_id: "l".into(),
            title: id.into(),
            due,
            notes: None,
            depth: 0,
            tasklist_name: "List".into(),
            account_id: String::new(),
            completing: false,
        }
    }

    fn metrics() -> Metrics {
        Metrics::new(1.0)
    }

    #[test]
    fn a_rectangle_owns_its_top_left_corner_and_not_its_bottom_right() {
        // Two rows sharing an edge must not both claim the boundary pixel, or a
        // click there does two things.
        let upper = Rect::new(0.0, 0.0, 10.0, 10.0);
        let lower = Rect::new(0.0, 10.0, 10.0, 20.0);
        assert!(upper.contains(0.0, 0.0));
        assert!(!upper.contains(10.0, 5.0));
        assert!(!upper.contains(5.0, 10.0));
        assert!(lower.contains(5.0, 10.0));
    }

    #[test]
    fn mirroring_swaps_the_horizontal_edges_and_leaves_the_vertical_ones() {
        let r = Rect::new(10.0, 4.0, 30.0, 8.0);
        let m = r.mirrored(100.0);
        assert_eq!(m, Rect::new(70.0, 4.0, 90.0, 8.0));
        // Twice is identity, which is what makes the renderer's fold safe to
        // apply at a single point.
        assert_eq!(m.mirrored(100.0), r);
    }

    #[test]
    fn the_panel_is_inset_by_the_shadow_and_the_body_maps_onto_itself() {
        let m = metrics();
        let p = Panel::new(360.0, 500.0, &m);
        assert_eq!(p.rect.left, m.shadow);
        assert_eq!(p.rect.right, 360.0 - m.shadow);
        assert_eq!(p.content_left, m.shadow + m.pad);
        assert_eq!(p.content_right, 360.0 - m.shadow - m.pad);
        // The mirror axis maps the panel's own edges onto each other, so the
        // glass body does not move when the locale reads right to left.
        assert_eq!(p.rect.mirrored(p.mirror_axis), p.rect);
    }

    #[test]
    fn without_a_shadow_margin_the_panel_fills_the_window() {
        // The acrylic backdrop case: the compositor fills the whole window and
        // a free margin would show as a square box around the panel.
        let m = Metrics::with_shadow(1.0, false);
        let p = Panel::new(360.0, 500.0, &m);
        assert_eq!(p.rect, Rect::new(0.0, 0.0, 360.0, 500.0));
    }

    #[test]
    fn the_content_height_does_not_depend_on_the_scroll_or_the_reveal() {
        let m = metrics();
        let top = 100.0;
        // The same list, drawn unscrolled and mid-animation, has one height.
        let settled = content_height(400.0, 0.0, 0.0, top, &m);
        let slide = reveal_slide(0.3);
        let scrolled = content_height(400.0 - 60.0 + slide, 60.0, slide, top, &m);
        assert!((settled - scrolled).abs() < 0.001, "{settled} {scrolled}");
    }

    #[test]
    fn the_rail_widens_to_cover_an_early_flight_and_a_late_event() {
        let events = vec![
            timed("flight", (5, 0), (6, 30)),
            timed("late", (22, 30), (23, 30)),
        ];
        let span = RailSpan::of(&events, at(12, 0));
        assert_eq!(span.lo, 5.0 * 60.0);
        assert_eq!(span.hi, 23.5 * 60.0);
    }

    #[test]
    fn an_empty_day_still_spans_four_hours() {
        // Nothing on, and "now" outside the default window: the span must not
        // collapse to a point, or every position lands on the same pixel.
        let span = RailSpan::of(&[], at(2, 0));
        assert!(span.hi - span.lo >= MIN_RAIL_SPAN, "{span:?}");
        assert!(span.contains(120.0));
    }

    #[test]
    fn the_rail_never_leaves_the_day() {
        let events = vec![timed("overnight", (23, 0), (1, 0))];
        let span = RailSpan::of(&events, at(23, 30));
        // An event running past midnight is clamped to the end of the day
        // rather than pointing backwards.
        assert_eq!(span.hi, DAY_MINUTES);
        assert!(span.lo >= 0.0);
    }

    #[test]
    fn a_short_meeting_still_draws_as_something() {
        let span = RailSpan {
            lo: 0.0,
            hi: DAY_MINUTES,
        };
        let ev = timed("standup", (9, 0), (9, 5));
        let (sx, ex) = rail_segment(&span, &ev, 0.0, 300.0).unwrap();
        assert!(ex - sx >= 2.5, "{sx} {ex}");
        assert!(ex <= 300.0);
    }

    #[test]
    fn an_all_day_entry_has_no_rail_segment() {
        let span = RailSpan {
            lo: 0.0,
            hi: DAY_MINUTES,
        };
        assert!(rail_segment(&span, &all_day("holiday"), 0.0, 300.0).is_none());
    }

    #[test]
    fn the_running_event_is_the_hero_and_a_ticked_off_one_is_not() {
        let mut agenda = Agenda {
            events: vec![
                timed("standup", (9, 0), (10, 0)),
                timed("review", (11, 0), (12, 0)),
            ],
            ..Default::default()
        };
        assert_eq!(
            pick_hero(&agenda, at(9, 30)),
            Some(Hero {
                index: 0,
                running: true
            })
        );
        // Nothing running: the next one, not merely the first.
        assert_eq!(
            pick_hero(&agenda, at(10, 30)),
            Some(Hero {
                index: 1,
                running: false
            })
        );

        // Ticked off a moment ago: the card must not still announce it.
        agenda.events[0].task_id = Some("t1".into());
        let mut t = task("t1", None);
        t.completing = true;
        agenda.tasks = vec![t];
        assert_eq!(
            pick_hero(&agenda, at(9, 30)),
            Some(Hero {
                index: 1,
                running: false
            })
        );
    }

    #[test]
    fn nothing_left_today_leaves_no_hero() {
        let agenda = Agenda {
            events: vec![timed("done", (8, 0), (9, 0))],
            ..Default::default()
        };
        assert_eq!(pick_hero(&agenda, at(18, 0)), None);
    }

    #[test]
    fn the_running_event_survives_hiding_past_ones() {
        let agenda = Agenda {
            events: vec![
                timed("over", (8, 0), (9, 0)),
                timed("running", (9, 0), (10, 0)),
                timed("later", (14, 0), (15, 0)),
            ],
            ..Default::default()
        };
        let list = EventList::build(&agenda, at(9, 30), false);
        let kept: Vec<usize> = list.rows.iter().map(|r| r.index).collect();
        assert_eq!(kept, vec![1, 2], "the running event must not be hidden");
        assert!(list.rows[0].running);
        assert!(!list.rows[0].past);

        let all = EventList::build(&agenda, at(9, 30), true);
        assert_eq!(all.rows.len(), 3);
        assert!(all.rows[0].past);
        assert_eq!(all.rows[0].dim(), 0.55);
    }

    #[test]
    fn a_ticked_off_time_block_fades_like_its_task_and_shows_no_countdown() {
        let mut ev = timed("focus", (14, 0), (15, 0));
        ev.task_id = Some("t1".into());
        let mut t = task("t1", None);
        t.completing = true;
        let agenda = Agenda {
            events: vec![ev.clone()],
            tasks: vec![t],
            ..Default::default()
        };
        let list = EventList::build(&agenda, at(13, 0), false);
        let row = list.rows[0];
        assert!(row.completing);
        // The same value the task row uses, so the two rows fade together.
        assert_eq!(
            row.dim(),
            TaskList::build(&agenda.tasks, at(13, 0).date_naive()).rows[0].dim()
        );
        assert!(!row.shows_countdown(&ev));
    }

    #[test]
    fn the_now_line_goes_before_the_next_event_and_at_the_end_when_all_are_over() {
        let agenda = Agenda {
            events: vec![
                timed("morning", (8, 0), (9, 0)),
                timed("afternoon", (14, 0), (15, 0)),
            ],
            ..Default::default()
        };
        let midday = EventList::build(&agenda, at(12, 0), true);
        assert_eq!(midday.now_line_before, Some(1));
        assert!(!midday.now_line_at_end);

        let evening = EventList::build(&agenda, at(20, 0), true);
        assert_eq!(evening.now_line_before, None);
        assert!(
            evening.now_line_at_end,
            "the line goes to the end of the list"
        );
    }

    #[test]
    fn a_day_of_only_all_day_entries_gets_no_now_line() {
        let agenda = Agenda {
            events: vec![all_day("holiday")],
            ..Default::default()
        };
        let list = EventList::build(&agenda, at(12, 0), true);
        assert_eq!(list.now_line_before, None);
        assert!(!list.now_line_at_end, "there is no time of day to mark");
    }

    #[test]
    fn the_calendar_name_appears_only_when_more_than_one_is_in_play() {
        let mut agenda = Agenda {
            events: vec![timed("a", (9, 0), (10, 0)), timed("b", (11, 0), (12, 0))],
            ..Default::default()
        };
        assert!(!EventList::build(&agenda, at(8, 0), true).multi_calendar);
        agenda.events[1].calendar_name = "Private".into();
        assert!(EventList::build(&agenda, at(8, 0), true).multi_calendar);

        // An all-day entry from a second calendar does not on its own earn the
        // column: it has no time row to put a name beside.
        let mut only_all_day = agenda.clone();
        only_all_day.events[1] = all_day("holiday");
        only_all_day.events[1].calendar_name = "Private".into();
        assert!(!EventList::build(&only_all_day, at(8, 0), true).multi_calendar);
    }

    #[test]
    fn double_bookings_are_counted_in_pairs() {
        let agenda = Agenda {
            events: vec![
                timed("a", (9, 0), (10, 0)),
                timed("b", (9, 30), (10, 30)),
                timed("c", (14, 0), (15, 0)),
            ],
            ..Default::default()
        };
        let list = EventList::build(&agenda, at(8, 0), true);
        assert_eq!(list.conflicts, 1);
        assert!(list.rows[0].conflicted && list.rows[1].conflicted);
        assert!(!list.rows[2].conflicted);
    }

    #[test]
    fn task_rows_carry_the_overdue_count_and_the_list_name_rule() {
        let today = NaiveDate::from_ymd_opt(2026, 8, 4).unwrap();
        let yesterday = NaiveDate::from_ymd_opt(2026, 8, 3).unwrap();
        let tasks = vec![task("a", Some(yesterday)), task("b", Some(today))];
        let list = TaskList::build(&tasks, today);
        assert_eq!(list.overdue, 1);
        assert!(list.rows[0].overdue && !list.rows[1].overdue);
        assert!(!list.multi_list);
        // Overdue is emphasis, not restraint: the row stays fully lit.
        assert_eq!(list.rows[0].dim(), 1.0);
    }

    #[test]
    fn a_subtask_is_indented_and_its_tick_target_still_reaches_the_row_edge() {
        let m = metrics();
        let p = Panel::new(360.0, 500.0, &m);
        let top = task_geometry(&p, 200.0, 0, &m);
        let nested = task_geometry(&p, 200.0, 2, &m);
        assert!(nested.check_center.0 - top.check_center.0 - 2.0 * m.indent < 0.001);
        assert!(nested.due_left > top.due_left);
        // However deep, the circle's click target starts at the row's left
        // bleed, so a click just beside it still ticks off.
        assert_eq!(nested.check_hit.left, p.content_left - ROW_BLEED);
        assert!(
            nested
                .check_hit
                .contains(nested.check_center.0, nested.check_center.1)
        );
    }

    /// A click has to be folded before it is compared, or a mirrored layout
    /// hits the wrong target.
    ///
    /// The drawing is folded across the panel's axis at the primitives while
    /// the rectangles here stay unmirrored, so the two only meet if the point
    /// is put back. The rows span the whole width and hide it; the refresh
    /// button and the tick circle do not.
    #[test]
    fn a_click_in_a_mirrored_layout_lands_on_what_was_drawn_under_it() {
        let m = metrics();
        let p = Panel::new(360.0, 500.0, &m);

        // Left to right, a click is itself.
        assert_eq!(hit_point(&p, false, 12.0, 40.0), (12.0, 40.0));
        // Folded twice is where it started, and y never moves.
        let (once, y) = hit_point(&p, true, 12.0, 40.0);
        assert_eq!(hit_point(&p, true, once, y), (12.0, 40.0));

        // The refresh button is drawn in the top corner the reading ends at.
        // In Arabic that is the left of the window, and a click there has to
        // reach the rectangle that still describes the right.
        let button = refresh_button(&p, p.rect.top, &m);
        let drawn = button.mirrored(p.mirror_axis);
        let (x, y) = hit_point(&p, true, drawn.center_x(), drawn.center_y());
        assert!(button.contains(x, y), "{button:?} from {drawn:?}");

        // And the tick circle, which is the other region that is not the full
        // width of the row — ticking a task off rather than opening it.
        let task = task_geometry(&p, 200.0, 0, &m);
        let circle = task.check_hit.mirrored(p.mirror_axis);
        let (x, y) = hit_point(&p, true, circle.center_x(), circle.center_y());
        assert!(task.check_hit.contains(x, y));
    }

    #[test]
    fn a_column_is_at_least_its_minimum_and_never_past_its_cap() {
        assert_eq!(column_width(0.0, 44.0, 88.0), 44.0);
        assert_eq!(column_width(200.0, 44.0, 88.0), 88.0);
        assert_eq!(column_width(50.0, 44.0, 88.0), 56.0);
        // Reversed bounds are survivable rather than fatal.
        assert_eq!(column_width(10.0, 44.0, 20.0), 44.0);
    }

    #[test]
    fn badges_are_laid_out_right_to_left_without_overlapping() {
        let m = metrics();
        let (first, next_right) = badge_pill(300.0, 40.0, 10.0, &m);
        let (second, _) = badge_pill(next_right, 30.0, 10.0, &m);
        assert_eq!(first.right, 300.0);
        assert!(second.right <= first.left, "{second:?} {first:?}");
    }

    #[test]
    fn the_section_heading_keeps_a_minimum_width_however_many_badges_there_are() {
        let m = metrics();
        let p = Panel::new(360.0, 500.0, &m);
        // A right edge pushed past the heading's own left edge.
        let r = section_label_rect(&p, 10.0, p.content_left - 100.0, &m);
        assert!(r.right > r.left);
        assert_eq!(r.right, p.content_left + 20.0);
    }

    #[test]
    fn a_list_that_fits_has_no_scroll_thumb() {
        let m = metrics();
        let p = Panel::new(360.0, 500.0, &m);
        assert!(scrollbar_thumb(&p, 100.0, 400.0, 200.0, 300.0, 0.0, &m).is_none());
        assert_eq!(max_scroll(200.0, 300.0), 0.0);
    }

    #[test]
    fn the_scroll_thumb_stays_inside_its_track_at_both_ends() {
        let m = metrics();
        let p = Panel::new(360.0, 500.0, &m);
        let (top, bottom, content, viewport) = (100.0, 400.0, 1200.0, 300.0);
        let track_top = top + 4.0;
        let track_bottom = bottom - 4.0;

        let at_top = scrollbar_thumb(&p, top, bottom, content, viewport, 0.0, &m).unwrap();
        assert!(at_top.top >= track_top - 0.001, "{at_top:?}");

        let at_end = scrollbar_thumb(
            &p,
            top,
            bottom,
            content,
            viewport,
            max_scroll(content, viewport),
            &m,
        )
        .unwrap();
        assert!(at_end.bottom <= track_bottom + 0.001, "{at_end:?}");

        // Overscrolling past the end cannot push it out either.
        let past = scrollbar_thumb(&p, top, bottom, content, viewport, 99_999.0, &m).unwrap();
        assert_eq!(past, at_end);
    }

    #[test]
    fn the_tooltip_flips_above_a_row_near_the_bottom_and_stays_in_the_content_area() {
        let m = metrics();
        let p = Panel::new(360.0, 500.0, &m);
        let limits = (100.0, 400.0);

        // Room below: it goes below.
        let below = tooltip_box(&p, (150.0, 180.0), 40.0, limits, &m);
        assert!(below.top >= 180.0, "{below:?}");

        // No room below: it flips above the row.
        let above = tooltip_box(&p, (370.0, 395.0), 40.0, limits, &m);
        assert!(above.bottom <= 370.0, "{above:?}");

        // Either way it stays inside the clip.
        for r in [below, above] {
            assert!(
                r.top >= limits.0 - 0.001 && r.bottom <= limits.1 + 0.001,
                "{r:?}"
            );
        }
    }

    #[test]
    fn a_tooltip_taller_than_the_content_area_is_pinned_rather_than_inverted() {
        let m = metrics();
        let p = Panel::new(360.0, 500.0, &m);
        // `bottom_limit - height` is below `top_limit` here; clamping with the
        // bounds the wrong way round would panic.
        let r = tooltip_box(&p, (150.0, 180.0), 900.0, (100.0, 400.0), &m);
        assert_eq!(r.top, 100.0);
    }

    #[test]
    fn shortening_counts_characters_rather_than_bytes() {
        assert_eq!(short("kurz", 10), "kurz");
        assert_eq!(short("abcdefghij", 5), "abcd…");
        // Server error text is not necessarily ASCII, and slicing by byte would
        // panic in the middle of a code point.
        assert_eq!(short("übermäßig länglich", 5), "über…");
        assert_eq!(short("日本語のエラー", 3), "日本…");
    }

    #[test]
    fn hero_progress_runs_from_nothing_to_everything_and_stops_there() {
        let ev = timed("workshop", (9, 0), (11, 0));
        assert_eq!(hero_progress(&ev, at(9, 0)), Some(0.0));
        assert_eq!(hero_progress(&ev, at(10, 0)), Some(0.5));
        // Overrunning does not push the line past the end of the card.
        assert_eq!(hero_progress(&ev, at(13, 0)), Some(1.0));
        assert_eq!(hero_progress(&all_day("holiday"), at(10, 0)), None);
    }
}
