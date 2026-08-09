// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! The widget, drawn — once, for both macOS and Linux.
//!
//! Everything here is arithmetic over [`tpmplaner_core::layout`] and calls into
//! [`Canvas`]. It never touches an operating system API, which is why the same
//! file serves Core Graphics and Cairo: a rounded rectangle, a line, a circle
//! and a line of text are the four things a panel is made of, and both engines
//! draw all four.
//!
//! The order is the order things are stacked in:
//!
//! ```text
//! shadow ─► glass ─► header + day rail ─► highlighted event
//!        ─► [clipped: events, tomorrow, tasks] ─► scrollbar ─► footer ─► tooltip
//! ```
//!
//! Two things are folded in on the way past [`Painter`] rather than by the back
//! ends. **Mirroring**: a right-to-left layout folds every x across the panel's
//! centre, so the layout module goes on computing left to right and neither
//! renderer knows the difference. **The reveal fade**: one factor multiplied
//! into every alpha while the panel fades in, which is cheaper and far harder
//! to forget than fading each call site.
//!
//! The Windows front end draws the same picture from its own Direct2D code.
//! That is one description of the interface too many and is known to be so —
//! see `docs/development/porting.md`. What keeps the two honest in the
//! meantime is that everything *decided* rather than drawn already lives in
//! `tpmplaner_core::layout`: which rows are visible, where the now line goes,
//! how wide a column has to be, which rectangle a click landed in.

use crate::unix::canvas::{Align, Canvas, Font, Stop};
use std::f32::consts::PI;
use tpmplaner_core::layout::{
    self, EventList, Frame, FrameResult, Hit, HitRegion, Panel, Rect, TaskList,
};
use tpmplaner_core::model::{Event, Task};
use tpmplaner_core::sync::Status;
use tpmplaner_core::theme::{Appearance, Metrics, Palette, mix};

/// Where a line of text sits inside its box, in reading order rather than on
/// screen.
///
/// [`Painter::text`] turns this into a physical [`Align`], which is the one
/// place the reading direction has to be considered: a time column that starts
/// at the reading edge is `Start` in both directions and on opposite sides of
/// the panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Start,
    Center,
    End,
}

/// Draws one frame and refills `hits`.
///
/// `size` is the whole window in device-independent pixels, shadow margin
/// included; the glass body is inset from it by [`Metrics::shadow`].
#[allow(clippy::too_many_arguments)]
pub fn draw<C: Canvas>(
    canvas: &mut C,
    size: (f32, f32),
    metrics: Metrics,
    palette: Palette,
    custom: &Appearance,
    rtl: bool,
    frame: &Frame,
    hits: &mut Vec<HitRegion>,
) -> FrameResult {
    let (w, h) = size;
    let panel = Panel::new(w, h, &metrics);
    hits.clear();

    let mut p = Painter {
        c: canvas,
        pal: palette,
        custom,
        m: metrics,
        rtl,
        mirror: panel.mirror_axis,
        fade: 1.0,
        tooltip: None,
    };

    p.c.clear();
    p.draw_shadow(panel.rect);
    p.draw_panel(panel.rect, p.pal.opacity(frame.opacity));

    let mut y = panel.rect.top;
    y = p.draw_header(&panel, y, frame, hits);
    y = p.draw_hero(&panel, y, frame, hits);

    let content_top = y;
    let content_bottom = panel.content_bottom(&metrics);

    // Everything between is clipped so scrolled rows cannot run into the
    // header, the footer or the frame.
    p.push_clip(panel.content_clip(content_top, content_bottom));

    // Reveal: slide up slightly from below while fading in.
    let reveal = frame.anim.reveal.value;
    p.fade = reveal.clamp(0.0, 1.0);
    let slide = layout::reveal_slide(reveal);

    let mut cy = content_top - frame.anim.scroll.value + slide;
    cy = p.draw_events_section(&panel, cy, frame, hits);
    cy = p.draw_tasks_section(&panel, cy, frame, hits);

    p.fade = 1.0;
    p.c.pop_clip();

    let content_height =
        layout::content_height(cy, frame.anim.scroll.value, slide, content_top, &metrics);
    let viewport_height = content_bottom - content_top;

    p.draw_scrollbar(
        &panel,
        content_top,
        content_bottom,
        content_height,
        viewport_height,
        frame,
    );
    p.draw_footer(&panel, frame, hits);
    // Last of all, so the overlay sits above everything and is not cut off by
    // the content clip.
    p.draw_tooltip(&panel, content_top, content_bottom);

    p.c.present();

    FrameResult {
        content_height,
        viewport_height,
    }
}

struct Painter<'a, C: Canvas> {
    c: &'a mut C,
    pal: Palette,
    /// Kept for the per-calendar colour overrides. Everything else it carries
    /// is already baked into the metrics, the palette and the back end's fonts.
    custom: &'a Appearance,
    m: Metrics,
    rtl: bool,
    /// Mirror axis, `panel.left + panel.right`.
    mirror: f32,
    /// Global opacity factor for the reveal animation.
    fade: f32,
    /// Full text of the hovered row, if it was drawn truncated, with the row's
    /// vertical extent. Set while drawing and emitted as an overlay at the end.
    tooltip: Option<(String, f32, f32)>,
}

impl<C: Canvas> Painter<'_, C> {
    // --- Glass body ---------------------------------------------------------

    /// A soft drop shadow made of stacked rounded rectangles growing outwards.
    ///
    /// A real Gaussian blur would be cleaner and needs an intermediate surface
    /// and a full filter graph every frame. With twelve low-opacity fills the
    /// result is indistinguishable at this size, and far cheaper — the same
    /// trade the Windows renderer makes.
    fn draw_shadow(&mut self, panel: Rect) {
        // A contrast theme has no shadow: it softens exactly the edge that
        // mode wants hard. Nor is there one where no margin was reserved for
        // it, because twelve rectangles that cannot grow outwards are not a
        // soft edge but twelve coats of black over the panel.
        if self.pal.shadow_alpha <= 0.0 || self.m.shadow <= 0.0 {
            return;
        }
        let steps = 12;
        for i in 0..steps {
            let t = i as f32 / (steps - 1) as f32;
            let grow = self.m.shadow * (1.0 - t);
            self.fill_round(
                Rect {
                    // Offset slightly downwards: the light comes from above.
                    left: panel.left - grow,
                    top: panel.top - grow * 0.6,
                    right: panel.right + grow,
                    bottom: panel.bottom + grow * 1.2,
                },
                self.m.corner + grow,
                0x00_0000,
                self.pal.shadow_alpha,
            );
        }
    }

    /// Gradient, gloss arc and the double edge.
    fn draw_panel(&mut self, panel: Rect, opacity: f32) {
        let (m, p) = (self.m, self.pal);

        self.fill_gradient(
            panel,
            m.corner,
            &[
                Stop {
                    at: 0.0,
                    color: p.panel_top,
                    alpha: opacity,
                },
                Stop {
                    at: 0.35,
                    color: p.panel_mid,
                    alpha: opacity,
                },
                Stop {
                    at: 1.0,
                    color: p.panel_bottom,
                    alpha: (opacity * 1.02).min(1.0),
                },
            ],
        );

        // Gloss arc: a light gradient across the top that fades out completely
        // towards the bottom. Dropped in a contrast theme.
        if p.sheen_gloss > 0.0 {
            let gloss_h = panel.height() * 0.30;
            self.fill_gradient(
                Rect {
                    bottom: panel.top + gloss_h,
                    ..panel
                },
                m.corner,
                &[
                    Stop {
                        at: 0.0,
                        color: p.sheen,
                        alpha: p.sheen_gloss,
                    },
                    Stop {
                        at: 0.55,
                        color: p.sheen,
                        alpha: p.sheen_gloss * 0.32,
                    },
                    Stop {
                        at: 1.0,
                        color: p.sheen,
                        alpha: 0.0,
                    },
                ],
            );
        }

        // Light inside, dark outside — that contrast is what makes the edge
        // look raised. Both are skipped when their alpha is zero: a flat or
        // borderless surface removes them that way, and a fully transparent
        // stroke is work with nothing to show for it.
        if p.sheen_border > 0.0 {
            self.stroke_round(
                panel.inset(1.0),
                m.corner - 1.0,
                p.sheen,
                p.sheen_border,
                1.0,
            );
            // A narrow strip of light right at the top, like the reflection on
            // the edge of a sheet of glass.
            self.line(
                panel.left + m.corner,
                panel.top + 1.0,
                panel.right - m.corner,
                panel.top + 1.0,
                p.sheen,
                p.sheen_border * 1.4,
                1.0,
                false,
            );
        }
        if p.border_outer_alpha > 0.0 {
            self.stroke_round(
                panel,
                m.corner,
                p.border_outer,
                p.border_outer_alpha,
                1.0,
            );
        }
    }

    // --- Header -------------------------------------------------------------

    fn draw_header(
        &mut self,
        panel: &Panel,
        top: f32,
        frame: &Frame,
        hits: &mut Vec<HitRegion>,
    ) -> f32 {
        let (m, p) = (self.m, self.pal);
        let loc = frame.loc;
        let (cx0, cx1) = (panel.content_left, panel.content_right);
        let today = frame.agenda.day.unwrap_or_else(|| frame.now.date_naive());
        let bottom = top + m.header_h;

        let btn = layout::refresh_button(panel, top, &m);
        let hovered = frame.hover == Some(Hit::Refresh);
        let spinning = matches!(frame.status, Status::Syncing);

        if hovered {
            let a = p.hover_alpha * (1.0 + frame.anim.hover.value);
            self.fill_round(btn, m.icon_btn * 0.5, p.hover, a);
        }

        self.refresh_icon(
            btn.center_x(),
            btn.center_y(),
            m.fs_row * 0.62,
            frame.anim.spinner,
            if spinning { p.accent } else { p.text_secondary },
            if hovered || spinning { 1.0 } else { 0.7 },
        );
        hits.push(HitRegion {
            rect: btn,
            hit: Hit::Refresh,
        });

        // Weekday large, date small underneath.
        let title_y = top + m.pad * 0.55;
        self.text(
            &loc.weekday(today),
            Font::Title,
            Side::Start,
            rect(
                cx0,
                title_y,
                cx1 - m.icon_btn - 60.0,
                title_y + m.fs_title * 1.45,
            ),
            p.text_primary,
            1.0,
        );
        self.text(
            &loc.date_line(today),
            Font::Subtitle,
            Side::Start,
            rect(cx0, title_y + m.fs_title * 1.35, cx1, bottom),
            p.text_dim,
            1.0,
        );

        // The clock on the right, level with the date line. It is the reason
        // the widget redraws every minute.
        self.text(
            &loc.time(frame.now),
            Font::Clock,
            Side::End,
            rect(
                cx0,
                title_y + m.fs_title * 1.3,
                cx1,
                title_y + m.fs_title * 1.3 + m.fs_clock * 1.5,
            ),
            p.text_secondary,
            0.9,
        );

        self.draw_day_rail(cx0, cx1, layout::day_rail_top(bottom, &m), frame);

        let line_y = bottom - 1.0;
        self.line(cx0, line_y, cx1, line_y, p.rule, p.rule_alpha, 1.0, false);
        bottom
    }

    /// The circular arrow on the refresh button, turning while a sync runs.
    ///
    /// A path rather than a glyph: Windows draws `\u{E72C}` from Segoe Fluent
    /// Icons, and neither macOS nor Linux has a font that can be relied on to
    /// carry it. Seven lines of trigonometry is a better bet than a font
    /// lookup that silently falls back to a box.
    ///
    /// Not mirrored for right-to-left beyond its centre: the icon is a circle,
    /// and Windows shows the same one in both directions.
    fn refresh_icon(&mut self, cx: f32, cy: f32, r: f32, angle: f32, color: u32, alpha: f32) {
        let cx = self.mx(cx);
        // A gap of roughly a fifth of the circle, so the arrowhead has
        // somewhere to point.
        let (from, to) = (angle + PI * 0.22, angle + PI * 1.82);
        self.c
            .arc(cx, cy, r, from, to, color, alpha * self.fade, 1.6);

        // The head sits at the end of the arc, pointing along the tangent.
        let (sin, cos) = to.sin_cos();
        let tip = (cx - sin * r * 0.95, cy + cos * r * 0.95);
        let base = (cx + cos * r, cy + sin * r);
        let wing = r * 0.5;
        self.c.polygon(
            &[
                tip,
                (base.0 + cos * wing, base.1 + sin * wing),
                (base.0 - cos * wing, base.1 - sin * wing),
            ],
            color,
            alpha * self.fade,
        );
    }

    /// The day rail: the whole day from 06:00 to 22:00 on a single strip.
    ///
    /// It shows the *shape* of the day, which a list cannot — clustered
    /// mornings, free afternoons, how long something runs.
    fn draw_day_rail(&mut self, x0: f32, x1: f32, y: f32, frame: &Frame) {
        let (m, p) = (self.m, self.pal);
        let now_min = layout::minutes_of_day(frame.now);
        let span = layout::RailSpan::of(&frame.agenda.events, frame.now);

        let track = rect(x0, y, x1, y + m.rail_h);
        let radius = m.rail_h * 0.5;
        self.fill_round(track, radius, p.rule, p.rule_alpha * 0.9);

        let now_x = span.position(now_min, x0, x1);

        // The elapsed part of the day. Deliberately very restrained: the rail
        // should be readable in passing, not compete for attention like a
        // progress bar.
        if now_x > x0 + 0.5 {
            self.fill_round(rect(x0, y, now_x, y + m.rail_h), radius, p.accent, 0.09);
        }

        for ev in &frame.agenda.events {
            let Some((sx, ex)) = layout::rail_segment(&span, ev, x0, x1) else {
                continue;
            };
            let running = ev.is_now(frame.now);
            let color = if p.high_contrast {
                p.text_primary
            } else if running {
                p.accent
            } else {
                self.calendar_color(ev)
            };
            self.fill_round(
                rect(sx, y + 1.0, ex, y + m.rail_h - 1.0),
                (m.rail_h - 2.0) * 0.5,
                color,
                if running { 1.0 } else { 0.85 },
            );
        }

        // The now marker goes on top so no segment can ever hide it.
        if span.contains(now_min) {
            self.line(
                now_x,
                y - 2.0,
                now_x,
                y + m.rail_h + 2.0,
                if p.dark { 0xFF_FFFF } else { 0x00_0000 },
                0.85,
                2.0,
                true,
            );
        }
    }

    /// The highlighted event: the one running now, otherwise the next one.
    ///
    /// This is the one piece of information you would otherwise have to go
    /// looking for — it belongs at the very top, not in a list.
    fn draw_hero(
        &mut self,
        panel: &Panel,
        top: f32,
        frame: &Frame,
        hits: &mut Vec<HitRegion>,
    ) -> f32 {
        let (m, p) = (self.m, self.pal);
        let loc = frame.loc;
        let cx1 = panel.content_right;
        let Some(hero) = layout::pick_hero(frame.agenda, frame.now) else {
            return top;
        };
        let (idx, running) = (hero.index, hero.running);
        let ev = &frame.agenda.events[idx];

        let card = layout::hero_card(panel, top, &m);
        let hovered = frame.hover == Some(Hit::Hero(idx));

        // A running event strongly, an upcoming one quietly.
        let tint = if running { p.accent_soft } else { p.panel_top };
        let base = if running { 0.70 } else { 0.42 };
        let a = base
            + if hovered {
                0.12 * frame.anim.hover.value
            } else {
                0.0
            };
        self.fill_round(card, 6.0, tint, a);
        self.stroke_round(
            card,
            6.0,
            if running { p.accent } else { p.rule },
            if running { 0.40 } else { p.rule_alpha },
            1.0,
        );
        // Accent bar on the side the reading starts from.
        self.fill_round(
            rect(
                card.left + 4.0,
                card.top + 7.0,
                card.left + 7.0,
                card.bottom - 7.0,
            ),
            1.5,
            if running { p.accent } else { p.text_dim },
            0.95,
        );

        let tx = card.left + 15.0;
        let label_box = rect(
            tx,
            card.top + 5.0,
            cx1 - 8.0,
            card.top + 5.0 + m.fs_section * 1.6,
        );
        self.text(
            &loc.label(if running {
                loc.cat.now_label
            } else {
                loc.cat.next_label
            }),
            Font::Section,
            Side::Start,
            label_box,
            if running { p.accent } else { p.text_dim },
            1.0,
        );

        // Countdown on the right: to the end while an event runs, otherwise to
        // the start.
        let rel = if running {
            ev.end
                .map(|e| loc.time_left((e - frame.now).num_minutes()))
                .unwrap_or_else(|| loc.label(loc.cat.running))
        } else {
            ev.start
                .map(|s| loc.relative((s - frame.now).num_minutes()))
                .unwrap_or_default()
        };
        self.text(
            &rel,
            Font::Meta,
            Side::End,
            Rect {
                right: cx1 - 9.0,
                ..label_box
            },
            if running { p.accent } else { p.text_secondary },
            1.0,
        );

        let time_prefix = if ev.all_day {
            String::new()
        } else {
            match (ev.start, ev.end) {
                (Some(s), Some(e)) => format!("{}–{}  ", loc.time(s), loc.time(e)),
                (Some(s), None) => format!("{}  ", loc.time(s)),
                _ => String::new(),
            }
        };
        self.text(
            &format!("{time_prefix}{}", ev.title),
            Font::RowStrong,
            Side::Start,
            rect(
                tx,
                card.bottom - m.fs_row * 2.0,
                cx1 - 9.0,
                card.bottom - 4.0,
            ),
            p.text_primary,
            1.0,
        );

        // Progress of the running event as a fine line at the foot of the card.
        if running && let Some(done) = layout::hero_progress(ev, frame.now) {
            let y = card.bottom - 2.5;
            self.line(
                card.left + 8.0,
                y,
                card.right - 8.0,
                y,
                p.accent,
                0.16,
                2.0,
                true,
            );
            if done > 0.005 {
                self.line(
                    card.left + 8.0,
                    y,
                    card.left + 8.0 + (card.right - card.left - 16.0) * done,
                    y,
                    p.accent,
                    0.85,
                    2.0,
                    true,
                );
            }
        }

        hits.push(HitRegion {
            rect: card,
            hit: Hit::Hero(idx),
        });
        card.bottom
    }

    // --- Lists --------------------------------------------------------------

    fn draw_events_section(
        &mut self,
        panel: &Panel,
        mut y: f32,
        frame: &Frame,
        hits: &mut Vec<HitRegion>,
    ) -> f32 {
        let (m, p) = (self.m, self.pal);
        let loc = frame.loc;
        let now = frame.now;
        let (x0, x1) = (panel.content_left, panel.content_right);

        // Which rows are visible, which are running, over or just ticked off,
        // which are double-booked and where the now line goes: all decided in
        // the core, so this front end draws the same list the Windows one does
        // rather than reimplementing the rules.
        let list = EventList::build(frame.agenda, now, frame.show_past_events);

        let mut badges: Vec<(String, u32)> = Vec::new();
        if list.conflicts > 0 {
            badges.push((loc.conflicts(list.conflicts), p.conflict));
        }

        y += m.section_gap;
        y = self.section_label(
            panel,
            y,
            &loc.label(loc.cat.section_events),
            list.rows.len(),
            &badges,
        );

        if list.is_empty() {
            self.text(
                &loc.label(loc.cat.no_events),
                Font::Row,
                Side::Start,
                rect(x0 + 2.0, y, x1, y + m.event_row_h),
                p.text_faint,
                1.0,
            );
            return y + m.event_row_h;
        }

        // Column widths from the actual content rather than fixed values —
        // otherwise the layout only fits one language.
        let time_labels: Vec<String> = list
            .rows
            .iter()
            .map(|r| {
                let e = &frame.agenda.events[r.index];
                if e.all_day {
                    loc.label(loc.cat.all_day)
                } else {
                    e.start.map(|s| loc.time(s)).unwrap_or_else(|| "—".into())
                }
            })
            .collect();
        let time_w = self.column_width(&time_labels, Font::Meta, m.time_col_w, m.time_col_w * 2.0);

        let rel_labels: Vec<String> = list
            .rows
            .iter()
            .map(|r| {
                let e = &frame.agenda.events[r.index];
                if r.running {
                    loc.label(loc.cat.running)
                } else if r.past {
                    e.calendar_name.clone()
                } else {
                    e.start
                        .map(|s| loc.relative((s - now).num_minutes()))
                        .unwrap_or_default()
                }
            })
            .collect();
        let rel_w = self.column_width(
            &rel_labels,
            Font::Meta,
            m.rel_col_w * 0.6,
            m.rel_col_w * 1.6,
        );

        for (slot, row_state) in list.rows.iter().enumerate() {
            if list.now_line_before == Some(slot) {
                self.draw_now_line(x0, x1, y, frame);
                y += m.nowline_h;
            }

            let idx = row_state.index;
            let ev = &frame.agenda.events[idx];
            let row = layout::row_rect(panel, y, m.event_row_h);
            let is_now = row_state.running;
            // A row whose event has just been ticked off is faded rather than
            // vanishing at once, so taking the tick back puts everything as it
            // was.
            let dim = row_state.dim();

            if frame.hover == Some(Hit::Event(idx)) {
                self.fill_round(row, 5.0, p.hover, p.hover_alpha * frame.anim.hover.value);
            }
            if is_now {
                self.fill_round(row, 5.0, p.accent, 0.10);
            }

            // Colour marker for the source calendar. A contrast theme replaces
            // it with the system colour — calendar colours carry no guaranteed
            // contrast there.
            let bar_color = if p.high_contrast {
                if is_now { p.accent } else { p.text_primary }
            } else if is_now {
                p.accent
            } else {
                self.calendar_color(ev)
            };
            self.fill_round(
                rect(x0, y + 6.0, x0 + 3.0, y + m.event_row_h - 6.0),
                1.5,
                bar_color,
                dim,
            );

            let tx = x0 + 11.0;
            // Overlapping events get a warning colour on the time — that is
            // where you look when hunting for conflicts.
            self.text(
                &time_labels[slot],
                Font::Meta,
                Side::Start,
                rect(tx, y, tx + time_w, y + m.event_row_h),
                if is_now {
                    p.accent
                } else if row_state.conflicted {
                    p.conflict
                } else {
                    p.text_dim
                },
                dim,
            );

            // Right column: time remaining, or for past events the calendar
            // name. Nothing at all for a row just ticked off — "in 2 h" next to
            // a finished item reads as a contradiction.
            let rel_x = x1 - rel_w;
            if row_state.shows_countdown(ev) {
                let rel = if is_now {
                    loc.cat.running.to_string()
                } else {
                    ev.start
                        .map(|s| loc.relative((s - now).num_minutes()))
                        .unwrap_or_default()
                };
                self.text(
                    &rel,
                    Font::Meta,
                    Side::End,
                    rect(rel_x, y, x1, y + m.event_row_h),
                    if is_now { p.accent } else { p.text_faint },
                    dim,
                );
            } else if !row_state.completing && list.multi_calendar && !ev.calendar_name.is_empty() {
                self.text(
                    &ev.calendar_name,
                    Font::Meta,
                    Side::End,
                    rect(rel_x, y, x1, y + m.event_row_h),
                    p.text_faint,
                    dim,
                );
            }

            let title_x = tx + time_w;
            let title = match &ev.location {
                Some(place) => format!("{}  ·  {place}", ev.title),
                None => ev.title.clone(),
            };
            let title_font = if is_now { Font::RowStrong } else { Font::Row };
            self.note_truncation(
                frame.hover == Some(Hit::Event(idx)),
                &title,
                title_font,
                rel_x - 6.0 - title_x,
                row,
            );
            self.text(
                &title,
                title_font,
                Side::Start,
                rect(title_x, y, rel_x - 6.0, y + m.event_row_h),
                p.text_primary,
                dim,
            );

            hits.push(HitRegion {
                rect: row,
                hit: Hit::Event(idx),
            });
            y += m.event_row_h + m.row_gap;
        }

        // Every event is over: the line goes to the end.
        if list.now_line_at_end {
            self.draw_now_line(x0, x1, y, frame);
            y += m.nowline_h;
        }

        self.draw_tomorrow(panel, y, frame, hits)
    }

    /// A look ahead to tomorrow.
    ///
    /// From late afternoon today's list is empty and the widget would be a
    /// blank surface — while that is exactly when "what is first tomorrow?"
    /// becomes the interesting question. At most two events, so the preview
    /// never crowds out today.
    fn draw_tomorrow(
        &mut self,
        panel: &Panel,
        mut y: f32,
        frame: &Frame,
        hits: &mut Vec<HitRegion>,
    ) -> f32 {
        let (m, p) = (self.m, self.pal);
        let loc = frame.loc;
        let (x0, x1) = (panel.content_left, panel.content_right);
        let upcoming: Vec<(usize, &Event)> = frame
            .agenda
            .tomorrow
            .iter()
            .enumerate()
            .filter(|(_, e)| !e.all_day)
            .take(2)
            .collect();
        if upcoming.is_empty() {
            return y;
        }

        y += m.section_gap * 0.7;
        self.line(x0, y, x1, y, p.rule, p.rule_alpha * 0.7, 1.0, false);
        y += 4.0;

        let label = loc.label(loc.cat.tomorrow);
        let label_w = self.c.text_width(&label, Font::Section) + 10.0;
        self.text(
            &label,
            Font::Section,
            Side::Start,
            rect(x0, y, x0 + label_w, y + m.event_row_h),
            p.text_faint,
            1.0,
        );

        for (idx, ev) in upcoming {
            let row = layout::row_rect(panel, y, m.event_row_h);
            if frame.hover == Some(Hit::Tomorrow(idx)) {
                self.fill_round(row, 5.0, p.hover, p.hover_alpha * frame.anim.hover.value);
            }
            // A block whose task was ticked off a moment ago fades here as it
            // does in today's list; the preview must not still be promising it.
            let dim: f32 = if frame.agenda.is_completing(ev) {
                0.42
            } else {
                1.0
            };
            // The label sits on the first row only; the second is indented
            // underneath it.
            let tx = x0 + label_w + 4.0;
            let time = ev.start.map(|s| loc.time(s)).unwrap_or_default();
            let time_w = self.c.text_width(&time, Font::Meta) + 8.0;
            self.text(
                &time,
                Font::Meta,
                Side::Start,
                rect(tx, y, tx + time_w, y + m.event_row_h),
                p.text_faint,
                dim,
            );
            self.text(
                &ev.title,
                Font::Row,
                Side::Start,
                rect(tx + time_w, y, x1, y + m.event_row_h),
                p.text_dim,
                dim,
            );
            hits.push(HitRegion {
                rect: row,
                hit: Hit::Tomorrow(idx),
            });
            y += m.event_row_h * 0.85;
        }
        y
    }

    /// A thin line with a time marker showing where in the day you are.
    fn draw_now_line(&mut self, x0: f32, x1: f32, y: f32, frame: &Frame) {
        let m = self.m;
        let accent = self.pal.accent;
        let cy = y + m.nowline_h * 0.5;
        // Depending on the locale the time can be noticeably wider ("8:09 PM"
        // rather than "20:09"), so this is measured generously.
        let label_w = m.fs_meta * 4.2;

        self.circle(x0 + 2.0, cy, 2.5, accent, 0.9, None);
        self.line(x0 + 6.0, cy, x1 - label_w - 5.0, cy, accent, 0.30, 1.0, false);
        self.text(
            &frame.loc.time(frame.now),
            Font::Meta,
            Side::End,
            rect(x1 - label_w, y, x1, y + m.nowline_h),
            accent,
            0.85,
        );
    }

    fn draw_tasks_section(
        &mut self,
        panel: &Panel,
        mut y: f32,
        frame: &Frame,
        hits: &mut Vec<HitRegion>,
    ) -> f32 {
        let (m, p) = (self.m, self.pal);
        let loc = frame.loc;
        let (x0, x1) = (panel.content_left, panel.content_right);
        let today = frame.agenda.day.unwrap_or_else(|| frame.now.date_naive());
        let tasks = &frame.agenda.tasks;
        let list = TaskList::build(tasks, today);

        y += m.section_gap;
        let badges: Vec<(String, u32)> = if list.overdue > 0 {
            vec![(loc.overdue(list.overdue), p.overdue)]
        } else {
            Vec::new()
        };
        y = self.section_label(
            panel,
            y,
            &loc.label(loc.cat.section_tasks),
            tasks.len(),
            &badges,
        );

        if list.is_empty() {
            self.text(
                &loc.label(loc.cat.no_tasks),
                Font::Row,
                Side::Start,
                rect(x0 + 2.0, y, x1, y + m.task_row_h),
                p.text_faint,
                1.0,
            );
            return y + m.task_row_h;
        }

        let due_labels: Vec<String> = tasks
            .iter()
            .map(|t| layout::due_label(t, today, loc))
            .collect();
        let due_w = self.column_width(&due_labels, Font::Meta, m.time_col_w, m.time_col_w * 2.2);

        // The undo hint and the list name share the right column.
        let mut right_labels: Vec<String> = vec![loc.cat.undo.to_string()];
        if list.multi_list {
            right_labels.extend(tasks.iter().map(|t| t.tasklist_name.clone()));
        }
        let right_w = self.column_width(
            &right_labels,
            Font::Meta,
            m.rel_col_w * 0.6,
            m.rel_col_w * 1.8,
        );

        for row_state in &list.rows {
            let idx = row_state.index;
            let task = &tasks[idx];
            let row = layout::row_rect(panel, y, m.task_row_h);
            let is_overdue = row_state.overdue;
            let undo = frame.undo.filter(|u| u.task_id == task.id);
            // While the tick is pending: fade out at once, so the click feels
            // immediate instead of waiting on the API.
            let dim = row_state.dim();
            let check_hovered = frame.hover == Some(Hit::TaskCheck(idx));
            let row_hovered = check_hovered
                || frame.hover == Some(Hit::Task(idx))
                || frame.hover == Some(Hit::Undo(idx));

            if row_hovered {
                self.fill_round(row, 5.0, p.hover, p.hover_alpha * frame.anim.hover.value);
            }

            let geo = layout::task_geometry(panel, y, task.depth, &m);
            let (ccx, ccy) = geo.check_center;
            self.draw_check(ccx, ccy, task, is_overdue, check_hovered, frame);

            let due_x = geo.due_left;
            self.text(
                &due_labels[idx],
                Font::Meta,
                Side::Start,
                rect(due_x, y, due_x + due_w, y + m.task_row_h),
                if is_overdue { p.overdue } else { p.text_dim },
                dim,
            );

            // Right column: the undo hint beats the list name.
            let mut title_right = x1;
            if let Some(u) = undo {
                title_right = x1 - right_w - 6.0;
                self.text(
                    &loc.label(loc.cat.undo),
                    Font::Meta,
                    Side::End,
                    rect(x1 - right_w, y, x1, y + m.task_row_h),
                    p.accent,
                    1.0,
                );
                // Countdown bar under the row.
                let by = y + m.task_row_h - 2.0;
                self.line(
                    x1 - right_w,
                    by,
                    x1 - right_w + right_w * u.remaining,
                    by,
                    p.accent,
                    0.55,
                    1.5,
                    true,
                );
            } else if list.multi_list && !task.tasklist_name.is_empty() {
                title_right = x1 - right_w - 6.0;
                self.text(
                    &task.tasklist_name,
                    Font::Meta,
                    Side::End,
                    rect(x1 - right_w, y, x1, y + m.task_row_h),
                    p.text_faint,
                    dim,
                );
            }

            // The note as a muted continuation after the title — the same
            // treatment the location gets on events. First line only; the rest
            // would be cut off anyway.
            let title_x = due_x + due_w;
            let title = match task.notes.as_deref().and_then(|n| n.lines().next()) {
                Some(note) if !note.trim().is_empty() => {
                    format!("{}  ·  {}", task.title, note.trim())
                }
                _ => task.title.clone(),
            };
            self.note_truncation(
                row_hovered && undo.is_none(),
                &title,
                Font::Row,
                title_right - title_x,
                row,
            );
            self.text(
                &title,
                Font::Row,
                Side::Start,
                rect(title_x, y, title_right, y + m.task_row_h),
                p.text_primary,
                dim,
            );

            // Order matters: regions pushed later win. While a tick is pending
            // the whole row catches the click, so a mistake can be taken back
            // from anywhere on it.
            hits.push(HitRegion {
                rect: row,
                hit: Hit::Task(idx),
            });
            hits.push(HitRegion {
                rect: geo.check_hit,
                hit: Hit::TaskCheck(idx),
            });
            if undo.is_some() {
                hits.push(HitRegion {
                    rect: row,
                    hit: Hit::Undo(idx),
                });
            }

            y += m.task_row_h + m.row_gap;
        }

        y
    }

    /// The tick circle. On sending it fills and gains a check mark.
    fn draw_check(
        &mut self,
        cx: f32,
        cy: f32,
        task: &Task,
        overdue: bool,
        hovered: bool,
        frame: &Frame,
    ) {
        let (m, p) = (self.m, self.pal);
        let r = m.check_size * 0.5;
        let resting = if overdue { p.overdue } else { p.text_dim };
        let color = if task.completing {
            p.ok_green
        } else if hovered {
            // Fade in through the hover animation rather than switching hard.
            mix(resting, p.accent, frame.anim.hover.value)
        } else {
            resting
        };

        self.circle(
            cx,
            cy,
            r,
            color,
            if task.completing { 1.0 } else { 0.78 },
            Some(1.5),
        );

        if task.completing {
            self.circle(cx, cy, r - 1.0, p.ok_green, 0.92, None);
            // A check mark from two strokes with round caps. On the filled
            // circle it needs the opposing colour.
            let tick = if p.high_contrast {
                p.panel_top
            } else if p.dark {
                p.panel_bottom
            } else {
                0xFF_FFFF
            };
            self.line(
                cx - r * 0.45,
                cy,
                cx - r * 0.1,
                cy + r * 0.38,
                tick,
                1.0,
                1.8,
                true,
            );
            self.line(
                cx - r * 0.1,
                cy + r * 0.38,
                cx + r * 0.5,
                cy - r * 0.4,
                tick,
                1.0,
                1.8,
                true,
            );
        } else if hovered {
            // Preview: a filled dot hinting at what a click would do.
            let a = 0.20 + 0.25 * frame.anim.hover.value;
            self.circle(cx, cy, r - 3.0, p.accent, a, None);
        }
    }

    // --- Trimmings ----------------------------------------------------------

    /// A section heading with a count and an optional badge.
    fn section_label(
        &mut self,
        panel: &Panel,
        y: f32,
        label: &str,
        count: usize,
        badges: &[(String, u32)],
    ) -> f32 {
        let m = self.m;

        // Placed right to left so several of them (overdue, conflicts) cannot
        // overlap; the heading then takes what is left, down to a floor.
        let mut right = panel.content_right;
        for (text, color) in badges.iter().rev() {
            let width = self.c.text_width(text, Font::Section);
            let (pill, next_right) = layout::badge_pill(right, width, y, &m);
            let a = if self.pal.dark { 0.16 } else { 0.13 };
            self.fill_round(pill, pill.height() * 0.5, *color, a);
            self.text(
                text,
                Font::Section,
                Side::Center,
                rect(pill.left + 6.0, pill.top, pill.right - 6.0, pill.bottom),
                *color,
                1.0,
            );
            right = next_right;
        }

        self.text(
            &format!("{label}   {count}"),
            Font::Section,
            Side::Start,
            layout::section_label_rect(panel, y, right, &m),
            self.pal.text_dim,
            0.9,
        );
        y + m.section_label_h
    }

    fn draw_scrollbar(
        &mut self,
        panel: &Panel,
        top: f32,
        bottom: f32,
        content: f32,
        viewport: f32,
        frame: &Frame,
    ) {
        let m = self.m;
        let alpha = frame.anim.scrollbar.value;
        if alpha < 0.01 {
            return;
        }
        let Some(thumb) = layout::scrollbar_thumb(
            panel,
            top,
            bottom,
            content,
            viewport,
            frame.anim.scroll.value,
            &m,
        ) else {
            return;
        };

        self.fill_round(
            thumb,
            m.scrollbar_w * 0.5,
            if self.pal.dark { 0xFF_FFFF } else { 0x2A_3140 },
            0.30 * alpha,
        );
    }

    fn draw_footer(&mut self, panel: &Panel, frame: &Frame, hits: &mut Vec<HitRegion>) {
        let (m, p) = (self.m, self.pal);
        let c = frame.loc.cat;
        let (cx0, cx1) = (panel.content_left, panel.content_right);
        let y = panel.content_bottom(&m);
        self.line(cx0, y, cx1, y, p.rule, p.rule_alpha * 0.8, 1.0, false);

        // A broken configuration is more urgent than any sync state: without a
        // hint you are left wondering why your changes do nothing.
        let (text, color, actionable) = match (frame.config_error, frame.status) {
            (Some(_), _) => (frame.loc.label(c.config_broken), p.warn, true),
            (None, Status::NeedsSetup(_)) => (frame.loc.label(c.setup_needed), p.warn, true),
            (None, Status::NeedsLogin(_)) => (frame.loc.label(c.connect_google), p.warn, true),
            (None, Status::Error(e)) => (layout::short(e, 56), p.overdue, true),
            (None, Status::Syncing) => (frame.loc.label(c.syncing), p.text_dim, false),
            // An available update outranks the routine "last synced" line —
            // that one carries no news once it has been read.
            (None, Status::Idle) if frame.update.is_some() => (
                frame
                    .loc
                    .label(c.update_available)
                    .replacen("{}", frame.update.unwrap_or_default(), 1),
                p.accent,
                true,
            ),
            (None, Status::Idle) => match frame.agenda.fetched_at {
                Some(t) => {
                    let next = t + chrono::Duration::minutes(frame.sync_minutes as i64);
                    (
                        frame
                            .loc
                            .updated_next(&frame.loc.time(t), &frame.loc.time(next)),
                        p.text_faint,
                        false,
                    )
                }
                None => (frame.loc.label(c.not_synced), p.text_faint, false),
            },
        };

        let footer = panel.footer_rect(&m);
        let hovered = actionable && frame.hover == Some(Hit::StatusAction);
        self.text(
            &text,
            Font::Footer,
            Side::Start,
            footer,
            color,
            if hovered { 1.0 } else { 0.88 },
        );
        if actionable {
            hits.push(HitRegion {
                rect: footer,
                hit: Hit::StatusAction,
            });
        }
    }

    /// Note that the hovered row is being drawn truncated.
    ///
    /// An event title cut off with "…" is simply not readable — and having to
    /// go and look it up in the calendar defeats the point of a widget.
    fn note_truncation(&mut self, hovered: bool, s: &str, font: Font, avail: f32, row: Rect) {
        if !hovered || avail <= 0.0 || self.c.text_width(s, font) <= avail {
            return;
        }
        self.tooltip = Some((s.to_string(), row.top, row.bottom));
    }

    /// An overlay carrying the full text of the hovered row.
    fn draw_tooltip(&mut self, panel: &Panel, top_limit: f32, bottom_limit: f32) {
        let Some((text, row_top, row_bottom)) = self.tooltip.take() else {
            return;
        };
        let (m, p) = (self.m, self.pal);

        // How tall the box has to be is the one part that needs the text
        // engine: only it knows how many lines this wraps to. Where the box
        // then goes is arithmetic, and lives in the core.
        let height = self
            .c
            .text_block_height(&text, layout::tooltip_text_width(panel, &m));
        let box_rect = layout::tooltip_box(
            panel,
            (row_top, row_bottom),
            height,
            (top_limit, bottom_limit),
            &m,
        );

        // Drawn opaque: the overlay has to cover the text underneath
        // completely, or it becomes unreadable itself.
        self.fill_round(box_rect, 5.0, p.panel_top, 1.0);
        self.stroke_round(box_rect, 5.0, p.accent, 0.55, 1.0);
        let inner = self.mrect(box_rect.inset(layout::TOOLTIP_PAD));
        self.c.text_block(&text, inner, p.text_primary, self.fade);
    }

    /// A column's width from its widest entry.
    ///
    /// Measuring is this side's job and the only part of it that needs a text
    /// engine; what to do with the number — the air between columns and the cap
    /// that stops one outlier eating half the row — is the same everywhere and
    /// lives in [`layout::column_width`].
    fn column_width(&mut self, items: &[String], font: Font, min: f32, max: f32) -> f32 {
        let widest = items
            .iter()
            .fold(0.0_f32, |acc, s| acc.max(self.c.text_width(s, font)));
        layout::column_width(widest, min, max)
    }

    /// The colour to draw one event's calendar in.
    ///
    /// The provider's own colour by default, so the widget matches the web
    /// calendar. A configured override wins, because the provider's palette was
    /// not chosen to be told apart at 0.82 opacity on a wallpaper.
    fn calendar_color(&self, ev: &Event) -> u32 {
        self.custom
            .calendar_color(&ev.calendar_id, &ev.calendar_name)
            .unwrap_or(ev.color)
    }

    // --- Primitives ---------------------------------------------------------

    /// Mirrors an x coordinate when the locale reads right to left. The axis is
    /// the centre of the glass body.
    fn mx(&self, x: f32) -> f32 {
        if self.rtl { self.mirror - x } else { x }
    }

    /// The rectangle to draw, mirrored first where the locale reads right to
    /// left. Left and right swap roles in the process, which is why this
    /// returns one rather than mirroring in place.
    fn mrect(&self, r: Rect) -> Rect {
        if self.rtl { r.mirrored(self.mirror) } else { r }
    }

    fn fill_round(&mut self, r: Rect, radius: f32, color: u32, alpha: f32) {
        let a = alpha * self.fade;
        if a < 0.004 {
            return;
        }
        let r = self.mrect(r);
        self.c.fill_round(r, radius, color, a);
    }

    fn fill_gradient(&mut self, r: Rect, radius: f32, stops: &[Stop]) {
        let faded: Vec<Stop> = stops
            .iter()
            .map(|s| Stop {
                alpha: s.alpha * self.fade,
                ..*s
            })
            .collect();
        let r = self.mrect(r);
        self.c.fill_gradient(r, radius, &faded);
    }

    fn stroke_round(&mut self, r: Rect, radius: f32, color: u32, alpha: f32, width: f32) {
        let a = alpha * self.fade;
        if a < 0.004 {
            return;
        }
        let r = self.mrect(r);
        self.c.stroke_round(r, radius, color, a, width);
    }

    #[allow(clippy::too_many_arguments)]
    fn line(
        &mut self,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        color: u32,
        alpha: f32,
        width: f32,
        round_caps: bool,
    ) {
        let a = alpha * self.fade;
        if a < 0.004 {
            return;
        }
        let (x0, x1) = (self.mx(x0), self.mx(x1));
        self.c.line(x0, y0, x1, y1, color, a, width, round_caps);
    }

    fn circle(&mut self, cx: f32, cy: f32, r: f32, color: u32, alpha: f32, stroke: Option<f32>) {
        let a = alpha * self.fade;
        if a < 0.004 {
            return;
        }
        let cx = self.mx(cx);
        self.c.circle(cx, cy, r, color, a, stroke);
    }

    fn push_clip(&mut self, r: Rect) {
        let r = self.mrect(r);
        self.c.push_clip(r);
    }

    /// One line of text, placed by reading order.
    ///
    /// `Side::Start` is the edge the reading begins at, which is the right-hand
    /// one in an Arabic or Hebrew layout. Mirroring the box moves it to the
    /// correct half of the panel; swapping the alignment with it is what keeps
    /// the text against the correct edge *of that box*, and forgetting the
    /// second half is how a mirrored layout ends up with every column adrift.
    fn text(&mut self, s: &str, font: Font, side: Side, r: Rect, color: u32, alpha: f32) {
        let a = alpha * self.fade;
        if s.is_empty() || r.right <= r.left || a < 0.004 {
            return;
        }
        let align = match (side, self.rtl) {
            (Side::Center, _) => Align::Center,
            (Side::Start, false) | (Side::End, true) => Align::Left,
            (Side::End, false) | (Side::Start, true) => Align::Right,
        };
        let r = self.mrect(r);
        self.c.text(s, font, align, r, color, a);
    }
}

fn rect(left: f32, top: f32, right: f32, bottom: f32) -> Rect {
    Rect {
        left,
        top,
        right,
        bottom,
    }
}
