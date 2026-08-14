// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! The widget itself: everything that happens between an input event and a
//! frame, for both macOS and Linux.
//!
//! `window.rs` on Windows mixes three things — a Win32 message pump, the
//! widget's behaviour, and the Win32 calls that behaviour needs. Only the
//! first of those is really platform-specific. A run loop source and an X11
//! event queue have nothing in common and each front end owns its own loop;
//! *what a click on a tick circle does* is the same on both, and lives here.
//!
//! The split is [`Shell`]: the loop owns the window and implements the trait,
//! [`App`] owns the state and calls back through it. Everything the widget
//! needs from a window system is in that trait and nothing else is, which is
//! also what makes it obvious how much of a front end is left to write.
//!
//! Coordinates: pointer positions arrive in device-independent pixels relative
//! to the window's corner, which is what [`tpmplaner_core::layout`] computes
//! in. Window geometry goes the other way and is in physical pixels, because
//! that is what every window system places windows with.

use crate::unix::autostart;
use chrono::{DateTime, Duration as ChronoDuration, Local, Timelike};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};
use tpmplaner_core::anim::Animations;
use tpmplaner_core::config::{self, Config};
use tpmplaner_core::i18n::Locale;
use tpmplaner_core::layout::{Frame, Hit, HitRegion, Panel, UndoView, hit_point};
use tpmplaner_core::menu;
use tpmplaner_core::model::TaskKey;
use tpmplaner_core::sync::{self, Command, Shared, Status, SyncHandle};
use tpmplaner_core::theme::{Appearance, Metrics, Palette, SystemVisuals, ThemePref};
use tpmplaner_core::{demo, log};

/// How far from the glass edge the grip still responds, in DIPs.
const RESIZE_GRIP: f32 = 6.0;
/// The smallest sensible size of the glass body, in DIPs.
const MIN_PANEL: (f32, f32) = (240.0, 180.0);
/// ~60 Hz. Runs only while something is actually moving.
pub const ANIM_INTERVAL: Duration = Duration::from_millis(16);
/// Ten steps a second is plenty for a countdown bar.
pub const UNDO_INTERVAL: Duration = Duration::from_millis(100);

/// A window rectangle in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// The pointer shape the widget is asking for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cursor {
    Arrow,
    /// Both diagonals and both axes, named after the corner or edge they grab.
    SizeNwSe,
    SizeNeSw,
    SizeWe,
    SizeNs,
}

/// The edge or edges of the glass body under the pointer.
///
/// The window has no frame on any of the three platforms — that is what keeps
/// it out of the taskbar and off the window switcher — so the system provides
/// no resizing either, and this stands in for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Edges {
    left: bool,
    right: bool,
    top: bool,
    bottom: bool,
}

impl Edges {
    fn any(&self) -> bool {
        self.left || self.right || self.top || self.bottom
    }

    fn cursor(&self) -> Cursor {
        match (self.left, self.right, self.top, self.bottom) {
            (true, _, true, _) | (_, true, _, true) => Cursor::SizeNwSe,
            (_, true, true, _) | (true, _, _, true) => Cursor::SizeNeSw,
            (true, _, _, _) | (_, true, _, _) => Cursor::SizeWe,
            (_, _, true, _) | (_, _, _, true) => Cursor::SizeNs,
            _ => Cursor::Arrow,
        }
    }
}

/// Everything the widget needs from a window system.
///
/// Deliberately small and deliberately complete: an entry that is missing here
/// is an entry the widget cannot use, and one that is here is one every front
/// end has to provide. The two implementations are `unix::mac::shell` and
/// `unix::linux::shell`.
pub trait Shell {
    /// Where the window is, in physical pixels.
    fn window_rect(&self) -> WindowRect;
    /// Move and resize it. Never activates it: the widget must not take focus.
    fn set_window_rect(&mut self, rect: WindowRect);
    /// Physical pixels per device-independent pixel.
    fn scale(&self) -> f32;
    /// The desktop area a window should be placed in, in physical pixels —
    /// menu bars, docks and panels excluded. `None` if the platform cannot
    /// say, which puts the widget in the corner instead.
    fn work_area(&self) -> Option<WindowRect>;
    /// Is any part of this rectangle on a monitor?
    fn is_on_screen(&self, rect: WindowRect) -> bool;

    /// Draw at the next opportunity.
    fn request_redraw(&mut self);
    fn set_cursor(&mut self, cursor: Cursor);
    /// Run the animation timer, or stop it once nothing is moving.
    fn set_anim_timer(&mut self, running: bool);
    /// Run the 100 ms timer that drives the undo countdown.
    fn set_undo_timer(&mut self, running: bool);
    /// Bring the window to the front for `for_at_most`, or let it drop back.
    ///
    /// The duration is passed rather than left to the widget because not every
    /// loop can notice a deadline going by. A loop that polls — the X11 and
    /// Wayland ones — ignore it and watch [`App::peek_deadline`]; one that only
    /// wakes for events — the AppKit one — has to set a timer, and without the
    /// duration it would have nothing to set it to.
    fn set_peek(&mut self, for_at_most: Option<Duration>);

    /// Hand a drag from here to the window system, if it insists on owning
    /// window placement.
    ///
    /// `true` means it took over and the widget must not move the window
    /// itself. Only a Wayland `xdg_toplevel` says so: Wayland gives no client
    /// the power to place its own window, and `xdg_toplevel.move` is the only
    /// way one is dragged. Everywhere else the widget moves its own window and
    /// the default answer stands.
    fn begin_system_drag(&mut self) -> bool {
        false
    }

    /// Show the context menu at the pointer and block until it closes.
    fn show_menu(&mut self, entries: &[menu::Entry]) -> Option<menu::Command>;
    /// Open a file or folder in whatever the desktop has registered.
    fn open_path(&self, path: &Path);
    fn set_clipboard(&mut self, text: &str);
    /// Does the widget start with the session?
    fn autostart_enabled(&self) -> bool;
    fn set_autostart(&mut self, enabled: bool);
    /// Leave the event loop.
    fn quit(&mut self);

    /// The system's current appearance and accessibility settings.
    fn system_visuals(&self) -> SystemVisuals;
}

/// A tick that has not been sent yet.
///
/// A click on a circle 14 pixels across happens by accident on a desktop.
/// Without a grace period the task would be done immediately and with no way
/// back — so it only travels to the API once the period has elapsed.
struct Pending {
    account_id: String,
    task_id: String,
    tasklist_id: String,
    started: Instant,
    window: Duration,
}

/// Modification times of the files the widget watches.
type Stamps = (Option<SystemTime>, Option<SystemTime>);

pub struct App {
    pub shared: Arc<Mutex<Shared>>,
    sync: Option<SyncHandle>,

    /// Refilled once per frame rather than allocated again.
    pub hits: Vec<HitRegion>,
    hover: Option<Hit>,
    /// Dragging: pointer position and window origin when the button went down,
    /// both in physical pixels.
    drag: Option<((i32, i32), (i32, i32))>,
    /// Resizing: which edges were grabbed, and the same two starting values.
    resize: Option<(Edges, (i32, i32), WindowRect)>,
    hover_edge: Edges,

    pub anim: Animations,
    animating: bool,
    scroll_target: f32,
    content_height: f32,
    viewport_height: f32,

    pending: Option<Pending>,
    pub peeking: bool,
    peek_until: Option<Instant>,

    next_sync_at: DateTime<Local>,
    consecutive_failures: u32,
    last_minute: u32,

    pub metrics: Metrics,
    pub palette: Palette,
    pub appearance: Appearance,
    theme_file: Option<PathBuf>,
    config_mtime: Stamps,
    pub loc: Locale,
    pub visuals: SystemVisuals,
    /// Set when the palette, the metrics or the typography changed and the
    /// renderer's cached fonts are stale. The loop clears it once it has
    /// rebuilt them.
    pub needs_rebuild: bool,
}

impl App {
    /// Loads the settings, resolves the look and starts the sync thread.
    ///
    /// `waker` is what the sync thread uses to nudge the event loop; every
    /// platform has a different one, which is the whole reason
    /// [`tpmplaner_core::host::Waker`] is a trait.
    pub fn new(
        visuals: SystemVisuals,
        waker: Arc<dyn tpmplaner_core::host::Waker>,
    ) -> (Self, Config) {
        let (cfg, config_error) = Config::load();
        if let Some(e) = &config_error {
            log::warn(e);
        }
        let appearance = appearance_for(&cfg);
        let metrics = metrics_for(&cfg, &appearance, visuals);
        let theme_file = cfg.appearance.file();
        let loc = Locale::resolve(&cfg.language);
        // The sync thread and the emergency exit have no access to this
        // instance, so they reach for the global catalogue instead.
        tpmplaner_core::i18n::set_global(loc.cat);
        let palette = palette_for(&cfg, &appearance, visuals, false);
        log::info(&format!(
            "Start — locale {} ({}{}), theme {}{}, accent #{:06X}, sync every {} min",
            loc.tag,
            loc.cat.code,
            if loc.rtl { ", RTL" } else { "" },
            if palette.dark { "dark" } else { "light" },
            if palette.high_contrast {
                ", high contrast"
            } else {
                ""
            },
            palette.accent,
            cfg.sync_minutes
        ));

        let demo = demo::enabled();
        let shared = Arc::new(Mutex::new(Shared {
            agenda: if demo {
                demo::agenda()
            } else {
                // The last known state, so start-up does not begin with a
                // blank surface.
                sync::read_cache().unwrap_or_default()
            },
            status: if demo { Status::Idle } else { Status::Syncing },
            config: cfg.clone(),
            config_error,
            calendars: Vec::new(),
            tasklists: Vec::new(),
            update: None,
        }));

        // Demo mode deliberately runs no sync thread — otherwise the missing
        // credentials would immediately bury the sample data under an error.
        let sync = (!demo).then(|| sync::spawn(shared.clone(), waker));
        if let Some(handle) = &sync {
            handle.send(Command::Sync);
        }

        let mut anim = Animations::default();
        anim.enabled = palette.animations;
        anim.spinning = sync.is_some();
        // Cached data is there immediately: show it straight away.
        if sync::lock(&shared).agenda.fetched_at.is_some() {
            anim.restart_reveal();
        } else {
            anim.reveal.jump(1.0);
        }

        let app = Self {
            shared,
            sync,
            hits: Vec::with_capacity(64),
            hover: None,
            drag: None,
            resize: None,
            hover_edge: Edges::default(),
            anim,
            animating: false,
            scroll_target: 0.0,
            content_height: 0.0,
            viewport_height: 0.0,
            pending: None,
            peeking: false,
            peek_until: None,
            next_sync_at: Local::now() + ChronoDuration::minutes(cfg.sync_minutes as i64),
            consecutive_failures: 0,
            last_minute: u32::MAX,
            metrics,
            palette,
            appearance,
            config_mtime: config_mtime(theme_file.as_deref()),
            theme_file,
            loc,
            visuals,
            needs_rebuild: false,
        };
        (app, cfg)
    }

    /// Is the widget pinned where it is?
    pub fn locked(&self) -> bool {
        sync::lock(&self.shared).config.locked
    }

    pub fn config(&self) -> Config {
        sync::lock(&self.shared).config.clone()
    }

    /// Window position and size in physical pixels for a configuration.
    ///
    /// `cfg.width`/`cfg.height` describe the visible glass body; the window is
    /// larger than that by the shadow margin on every side.
    pub fn target_geometry(&self, cfg: &Config, shell: &dyn Shell) -> WindowRect {
        let scale = shell.scale();
        let shadow_px = (self.metrics.shadow * scale).round() as i32;
        let width = ((cfg.width + self.metrics.shadow * 2.0) * scale).round() as i32;
        let height = ((cfg.height + self.metrics.shadow * 2.0) * scale).round() as i32;

        let (x, y) = match (cfg.x, cfg.y) {
            (Some(x), Some(y)) => (x, y),
            // Top right, like the Vista sidebar. The gap applies to the
            // visible glass edge, not to the invisible shadow margin.
            _ => {
                let margin = (24.0 * scale).round() as i32;
                match shell.work_area() {
                    Some(w) => (
                        w.x + w.width - margin - width + shadow_px,
                        w.y + margin - shadow_px,
                    ),
                    None => (margin, margin),
                }
            }
        };
        WindowRect {
            x,
            y,
            width,
            height,
        }
    }

    // --- Drawing ------------------------------------------------------------

    /// Builds the frame for the renderer and hands it to `paint`.
    ///
    /// Takes a closure rather than the canvas because the borrow of the shared
    /// agenda has to outlive the drawing and end before anything else touches
    /// the mutex — a `Frame` is a set of references into it.
    pub fn with_frame<R>(&mut self, draw: impl FnOnce(&Frame, &mut Vec<HitRegion>) -> R) -> R {
        // Clone the Arc, not the contents: during an animation this runs 60
        // times a second, and a full clone of the agenda would mean thousands
        // of allocations per second for data that is not changing.
        let shared = self.shared.clone();
        let guard = sync::lock(&shared);

        let undo = self.pending.as_ref().map(|p| UndoView {
            key: TaskKey::new(&p.account_id, &p.tasklist_id, &p.task_id),
            remaining: 1.0
                - (p.started.elapsed().as_secs_f32() / p.window.as_secs_f32()).clamp(0.0, 1.0),
        });
        let frame = Frame {
            agenda: &guard.agenda,
            loc: &self.loc,
            status: &guard.status,
            anim: &self.anim,
            now: Local::now(),
            hover: self.hover,
            sync_minutes: guard.config.sync_minutes,
            opacity: guard.config.opacity,
            show_past_events: guard.config.show_past_events,
            undo,
            config_error: guard.config_error.as_deref(),
            update: guard.update.as_ref().map(|u| u.version.as_str()),
        };
        draw(&frame, &mut self.hits)
    }

    /// Takes the measurements a finished frame reports.
    pub fn frame_drawn(&mut self, content_height: f32, viewport_height: f32) {
        self.content_height = content_height;
        self.viewport_height = viewport_height;
        // After rows are removed the scroll offset can point at nothing — pull
        // it back in that case.
        let overflow = (self.content_height - self.viewport_height).max(0.0);
        if self.scroll_target > overflow {
            self.scroll_target = overflow;
            self.anim.scroll.set(overflow);
        }
    }

    // --- Animation ----------------------------------------------------------

    /// Starts the animation timer if needed and takes a step immediately, so
    /// the response does not feel delayed by up to 16 ms.
    pub fn kick(&mut self, shell: &mut dyn Shell) {
        if !self.animating {
            self.animating = true;
            shell.set_anim_timer(true);
        }
        self.pump(shell);
    }

    /// One animation step. As soon as nothing is moving the timer is switched
    /// off — from then on the widget costs nothing again.
    pub fn pump(&mut self, shell: &mut dyn Shell) {
        let active = self.anim.tick();
        shell.request_redraw();
        if !active && self.animating {
            self.animating = false;
            shell.set_anim_timer(false);
        }
    }

    // --- Input --------------------------------------------------------------

    /// The pointer moved. `x`/`y` are in DIPs relative to the window corner;
    /// `pointer` is the same position on the desktop, in physical pixels.
    pub fn on_pointer_move(&mut self, shell: &mut dyn Shell, x: f32, y: f32, pointer: (i32, i32)) {
        if let Some((edges, start_pointer, start_rect)) = self.resize {
            self.apply_resize(shell, edges, start_pointer, start_rect, pointer);
            return;
        }
        if let Some((start_pointer, start_origin)) = self.drag {
            shell.set_window_rect(WindowRect {
                x: start_origin.0 + (pointer.0 - start_pointer.0),
                y: start_origin.1 + (pointer.1 - start_pointer.1),
                ..shell.window_rect()
            });
            return;
        }

        // The edge outranks row highlighting: otherwise the grip competes with
        // the hover state of the row underneath.
        let edge = self.edge_at(shell, x, y);
        if edge != self.hover_edge {
            self.hover_edge = edge;
            shell.set_cursor(edge.cursor());
        }
        if edge.any() {
            if self.hover.take().is_some() {
                self.anim.hover.set(0.0);
                self.kick(shell);
            }
            return;
        }

        let hover = self.hit_at(shell, x, y);
        if hover != self.hover {
            self.hover = hover;
            // Fade in again on a change rather than jumping.
            self.anim.hover.jump(0.0);
            self.anim.hover.set(if hover.is_some() { 1.0 } else { 0.0 });
            self.kick(shell);
        }
    }

    /// The pointer left the window.
    pub fn on_pointer_leave(&mut self, shell: &mut dyn Shell) {
        if self.hover_edge.any() {
            self.hover_edge = Edges::default();
            shell.set_cursor(Cursor::Arrow);
        }
        if self.hover.take().is_some() {
            self.anim.hover.set(0.0);
            self.kick(shell);
        }
    }

    pub fn on_press(&mut self, shell: &mut dyn Shell, x: f32, y: f32, pointer: (i32, i32)) {
        let edge = self.edge_at(shell, x, y);
        if edge.any() {
            self.resize = Some((edge, pointer, shell.window_rect()));
            return;
        }

        match self.hit_at(shell, x, y) {
            Some(Hit::Refresh) => self.request_sync(shell),

            // Sits on top of the row for as long as the grace period runs.
            Some(Hit::Undo(_)) => self.cancel_pending(shell),

            Some(Hit::TaskCheck(key)) => self.begin_pending(shell, key),

            Some(Hit::Event(idx)) | Some(Hit::Hero(idx)) => self.open_event(idx, false),
            Some(Hit::Tomorrow(idx)) => self.open_event(idx, true),

            Some(Hit::Task(_)) => {
                tpmplaner_core::host::host().open_url("https://tasks.google.com/")
            }

            Some(Hit::StatusAction) => self.status_action(shell),

            // Empty space: drag the window — unless it is pinned, in which
            // case the click does nothing at all. That is the point of the
            // lock: the widget sits below everything and is grabbed by its
            // empty space, so reaching past it for something on the desktop
            // moves it by accident.
            None if self.locked() => {}
            None => {
                if !shell.begin_system_drag() {
                    let r = shell.window_rect();
                    self.drag = Some((pointer, (r.x, r.y)));
                }
            }
        }
    }

    pub fn on_release(&mut self, shell: &mut dyn Shell) {
        if self.resize.take().is_some() {
            self.save_geometry(shell);
        } else if self.drag.take().is_some() {
            self.save_position(shell);
        }
    }

    pub fn on_scroll(&mut self, shell: &mut dyn Shell, delta_lines: f32) {
        let overflow = (self.content_height - self.viewport_height).max(0.0);
        let next = (self.scroll_target - delta_lines * 52.0).clamp(0.0, overflow);
        if (next - self.scroll_target).abs() > 0.01 {
            self.scroll_target = next;
            self.anim.scroll.set(next);
            // Show the scrollbar; it fades out again by itself.
            self.anim.scrollbar.jump(1.0);
            self.anim.scrollbar.set(0.0);
            self.kick(shell);
        }
    }

    /// Which edge is under the pointer? Nothing at all while the widget is
    /// locked, which is what removes the resize cursor along with the resize:
    /// an edge that shows a double arrow but refuses to move reads as a bug.
    fn edge_at(&self, shell: &dyn Shell, x: f32, y: f32) -> Edges {
        if self.locked() {
            return Edges::default();
        }
        let (w, h) = self.size_dip(shell);
        // The glass body is inset by the shadow margin; the grip sits on *its*
        // edge, not on the invisible window edge.
        let s = self.metrics.shadow;
        let (l, t, r, b) = (s, s, w - s, h - s);
        if x < l - RESIZE_GRIP || x > r + RESIZE_GRIP || y < t - RESIZE_GRIP || y > b + RESIZE_GRIP
        {
            return Edges::default();
        }
        Edges {
            left: (x - l).abs() <= RESIZE_GRIP,
            right: (x - r).abs() <= RESIZE_GRIP,
            top: (y - t).abs() <= RESIZE_GRIP,
            bottom: (y - b).abs() <= RESIZE_GRIP,
        }
    }

    fn apply_resize(
        &mut self,
        shell: &mut dyn Shell,
        edges: Edges,
        start_pointer: (i32, i32),
        start: WindowRect,
        pointer: (i32, i32),
    ) {
        let (dx, dy) = (pointer.0 - start_pointer.0, pointer.1 - start_pointer.1);
        let scale = shell.scale();
        let min_w = ((MIN_PANEL.0 + self.metrics.shadow * 2.0) * scale) as i32;
        let min_h = ((MIN_PANEL.1 + self.metrics.shadow * 2.0) * scale) as i32;

        let (mut left, mut top) = (start.x, start.y);
        let (mut right, mut bottom) = (start.x + start.width, start.y + start.height);
        if edges.left {
            // Dragging the left edge moves the origin with it, so the minimum
            // width has to constrain the *left* side.
            left = (left + dx).min(right - min_w);
        }
        if edges.right {
            right = (right + dx).max(left + min_w);
        }
        if edges.top {
            top = (top + dy).min(bottom - min_h);
        }
        if edges.bottom {
            bottom = (bottom + dy).max(top + min_h);
        }
        shell.set_window_rect(WindowRect {
            x: left,
            y: top,
            width: right - left,
            height: bottom - top,
        });
    }

    /// Hit testing back to front: the regions drawn last (those on top) win —
    /// which is how the tick circle beats the task row, and the undo area beats
    /// them both.
    ///
    /// The point is folded first: the rectangles come out of the core
    /// unmirrored while a click arrives in the coordinates the user is looking
    /// at. See [`hit_point`].
    fn hit_at(&self, shell: &dyn Shell, x: f32, y: f32) -> Option<Hit> {
        let (x, y) = if self.loc.rtl {
            let (w, h) = self.size_dip(shell);
            hit_point(&Panel::new(w, h, &self.metrics), true, x, y)
        } else {
            (x, y)
        };
        self.hits
            .iter()
            .rev()
            .find(|r| r.contains(x, y))
            .map(|r| r.hit)
    }

    fn size_dip(&self, shell: &dyn Shell) -> (f32, f32) {
        let r = shell.window_rect();
        let k = 1.0 / shell.scale().max(0.1);
        (r.width as f32 * k, r.height as f32 * k)
    }

    // --- Timers -------------------------------------------------------------

    /// The minute tick: the clock, the sync schedule, the settings check and
    /// the widget's own position.
    ///
    /// The position is checked here rather than from a notification because
    /// the two window systems report a changed screen layout in entirely
    /// different ways, and because a minute is a perfectly good response time
    /// for a monitor being unplugged. It costs two calls to the window system
    /// and only writes anything when the widget really is off every screen.
    pub fn on_minute(&mut self, shell: &mut dyn Shell) {
        let now = Local::now();
        if now.minute() != self.last_minute {
            self.last_minute = now.minute();
            shell.request_redraw();
        }
        self.reload_config_if_changed(shell);
        self.rescue_offscreen(shell);
        if now >= self.next_sync_at {
            self.request_sync(shell);
        }
        // A backstop for the AppKit loop's one-shot timer and the polling
        // loops' deadline alike: whatever else went wrong, a peek does not
        // outlive the minute it started in.
        if let Some(until) = self.peek_deadline()
            && Instant::now() >= until
        {
            self.end_peek(shell);
        }
    }

    /// One step of the undo countdown.
    pub fn on_undo_tick(&mut self, shell: &mut dyn Shell) {
        let expired = self
            .pending
            .as_ref()
            .is_some_and(|p| p.started.elapsed() >= p.window);
        if expired {
            self.commit_pending(shell);
        }
        shell.request_redraw();
    }

    /// The sync thread finished something.
    pub fn on_sync_done(&mut self, shell: &mut dyn Shell) {
        let status = sync::lock(&self.shared).status.clone();
        match status {
            Status::Syncing => {
                self.anim.spinning = true;
            }
            Status::Idle => {
                self.anim.spinning = false;
                self.consecutive_failures = 0;
                self.next_sync_at =
                    Local::now() + ChronoDuration::minutes(self.config().sync_minutes as i64);
                self.anim.restart_reveal();
            }
            // A failure backs off rather than hammering a server that is
            // already unhappy, up to a ceiling so it still recovers by itself.
            Status::Error(_) | Status::NeedsLogin(_) | Status::NeedsSetup(_) => {
                self.anim.spinning = false;
                self.consecutive_failures = self.consecutive_failures.saturating_add(1);
                let minutes = self.config().sync_minutes as i64;
                let backoff = minutes * (1 << self.consecutive_failures.min(3)) as i64;
                self.next_sync_at = Local::now() + ChronoDuration::minutes(backoff.min(120));
            }
        }
        self.kick(shell);
    }

    pub fn request_sync(&mut self, shell: &mut dyn Shell) {
        if let Some(s) = self.sync.as_ref() {
            s.send(Command::Sync);
            self.anim.spinning = true;
        }
        self.next_sync_at =
            Local::now() + ChronoDuration::minutes(self.config().sync_minutes as i64);
        self.kick(shell);
    }

    // --- Peek ---------------------------------------------------------------

    /// The global shortcut: bring the widget forward for a few seconds.
    ///
    /// This is what makes the bottom-most position workable. The widget is
    /// never in the way, which also means it is never visible while you work;
    /// one key press is enough, and it sinks back on its own — no click, and no
    /// change of focus.
    pub fn begin_peek(&mut self, shell: &mut dyn Shell) {
        let window = Duration::from_secs(self.config().peek_seconds.max(1) as u64);
        self.peeking = true;
        self.peek_until = Some(Instant::now() + window);
        shell.set_peek(Some(window));
        // Fade in as for new data: the eye should be led to it.
        self.anim.restart_reveal();
        self.kick(shell);
    }

    pub fn end_peek(&mut self, shell: &mut dyn Shell) {
        if !self.peeking {
            return;
        }
        self.peeking = false;
        self.peek_until = None;
        shell.set_peek(None);
        shell.request_redraw();
    }

    /// When the peek is due to end.
    ///
    /// A loop that polls — the X11 and Wayland ones — waits until this rather
    /// than waking on a timer; the AppKit loop, which only wakes for events,
    /// sets a one-shot timer in [`Shell::set_peek`] instead. Both are backed
    /// up by [`Self::on_minute`], which is what reads this on every platform.
    pub fn peek_deadline(&self) -> Option<Instant> {
        self.peek_until
    }

    // --- Undo ---------------------------------------------------------------

    /// Records the tick provisionally and starts the grace period.
    fn begin_pending(&mut self, shell: &mut dyn Shell, key: TaskKey) {
        // Only one task waits at a time: a second completion confirms the
        // first immediately.
        self.commit_pending(shell);

        let seconds = self.config().undo_seconds;
        let ids = {
            let mut guard = sync::lock(&self.shared);
            // By task, not by row: a sync between the frame that drew the row
            // and this click may have moved it, and nothing must be ticked off
            // except what was clicked. Gone from the list is a click on a row
            // that is no longer there, and does nothing.
            guard.agenda.task_mut(key).map(|t| {
                // Acknowledge visually at once, so the click feels immediate.
                t.completing = true;
                (t.account_id.clone(), t.tasklist_id.clone(), t.id.clone())
            })
        };
        let Some((account_id, tasklist_id, task_id)) = ids else {
            return;
        };

        if seconds == 0 {
            self.send_completion(&account_id, &tasklist_id, &task_id);
            shell.request_redraw();
            return;
        }

        self.pending = Some(Pending {
            account_id,
            task_id,
            tasklist_id,
            started: Instant::now(),
            window: Duration::from_secs(seconds as u64),
        });
        shell.set_undo_timer(true);
        shell.request_redraw();
    }

    /// Sends a waiting completion and ends the grace period.
    pub fn commit_pending(&mut self, shell: &mut dyn Shell) {
        let Some(p) = self.pending.take() else { return };
        shell.set_undo_timer(false);
        self.send_completion(&p.account_id, &p.tasklist_id, &p.task_id);
    }

    /// Sends a waiting completion without a shell to hand.
    ///
    /// The one caller is shutdown, where the timer is about to stop with the
    /// process and the window may already be gone.
    pub fn commit_pending_on_exit(&mut self) {
        let Some(p) = self.pending.take() else { return };
        self.send_completion(&p.account_id, &p.tasklist_id, &p.task_id);
    }

    /// Takes the completion back — nothing was ever sent.
    fn cancel_pending(&mut self, shell: &mut dyn Shell) {
        let Some(p) = self.pending.take() else { return };
        shell.set_undo_timer(false);
        let mut guard = sync::lock(&self.shared);
        let key = TaskKey::new(&p.account_id, &p.tasklist_id, &p.task_id);
        if let Some(t) = guard.agenda.task_mut(key) {
            t.completing = false;
        }
        drop(guard);
        log::info("Task completion undone");
        shell.request_redraw();
    }

    fn send_completion(&self, account_id: &str, tasklist_id: &str, task_id: &str) {
        if let Some(s) = self.sync.as_ref() {
            s.send(Command::CompleteTask {
                account_id: account_id.to_owned(),
                tasklist_id: tasklist_id.to_owned(),
                task_id: task_id.to_owned(),
            });
        }
    }

    // --- Actions ------------------------------------------------------------

    fn open_event(&self, idx: usize, tomorrow: bool) {
        let guard = sync::lock(&self.shared);
        let list = if tomorrow {
            &guard.agenda.tomorrow
        } else {
            &guard.agenda.events
        };
        let link = list.get(idx).and_then(|e| e.html_link.clone());
        drop(guard);
        if let Some(link) = link {
            tpmplaner_core::host::host().open_url(&link);
        }
    }

    fn status_action(&mut self, shell: &mut dyn Shell) {
        let (status, has_config_error, has_update) = {
            let guard = sync::lock(&self.shared);
            (
                guard.status.clone(),
                guard.config_error.is_some(),
                guard.update.is_some(),
            )
        };
        if has_config_error {
            shell.open_path(&config::config_path());
            return;
        }
        if has_update && matches!(status, Status::Idle) {
            self.start_update(shell);
            return;
        }
        match status {
            Status::NeedsSetup(_) => shell.open_path(&config::data_dir()),
            Status::NeedsLogin(_) => self.relogin(shell),
            // On a sync failure the full text is in the log.
            Status::Error(_) => shell.open_path(&log::file_path()),
            _ => self.request_sync(shell),
        }
    }

    fn relogin(&mut self, shell: &mut dyn Shell) {
        if let Some(s) = self.sync.as_ref() {
            s.send(Command::Relogin);
        }
        self.anim.spinning = true;
        self.kick(shell);
    }

    /// There is no installer script for macOS or Linux, so this points at the
    /// release page rather than replacing the binary behind the user's back.
    ///
    /// The Windows one-liner writes inside the user's profile and nothing
    /// else knows about it. A tarball unpacked by hand, a Flatpak and a
    /// distribution package are all somebody else's to update, and a widget
    /// that overwrote one of them would be a bad citizen — see
    /// `docs/development/porting.md`.
    fn start_update(&mut self, _shell: &mut dyn Shell) {
        let Some(update) = sync::lock(&self.shared).update.clone() else {
            return;
        };
        log::info(&format!(
            "Update {} available — opening the release page",
            update.version
        ));
        tpmplaner_core::host::host().open_url(&update.url);
    }

    /// Today's agenda as plain text, for pasting into a message.
    fn copy_agenda(&mut self, shell: &mut dyn Shell) {
        let guard = sync::lock(&self.shared);
        let text = tpmplaner_core::model::agenda_as_text(&guard.agenda, &self.loc);
        drop(guard);
        shell.set_clipboard(&text);
        log::info("Agenda copied to the clipboard");
    }

    /// Switches a calendar or task list on or off.
    ///
    /// An empty list in the configuration means "all of them". Deselecting a
    /// source from that state means the selection has to be written out in
    /// full first — otherwise the result would be "all" again. Conversely a
    /// complete selection is normalised back to "empty", so a calendar added
    /// later comes along automatically.
    fn toggle_source(&mut self, shell: &mut dyn Shell, is_calendar: bool, index: usize) {
        {
            let mut guard = sync::lock(&self.shared);
            let all: Vec<String> = if is_calendar {
                guard.calendars.iter().map(|(id, _)| id.clone()).collect()
            } else {
                guard.tasklists.iter().map(|(id, _)| id.clone()).collect()
            };
            let Some(id) = all.get(index).cloned() else {
                return;
            };

            let selected = if is_calendar {
                &mut guard.config.calendar_ids
            } else {
                &mut guard.config.tasklist_ids
            };

            if selected.is_empty() {
                *selected = all.iter().filter(|i| **i != id).cloned().collect();
            } else if selected.contains(&id) {
                // The last source must not disappear: an empty selection would
                // mean "all" again, which is exactly the opposite.
                if selected.len() > 1 {
                    selected.retain(|i| *i != id);
                }
            } else {
                selected.push(id);
            }

            if selected.len() == all.len() {
                selected.clear();
            }
            guard.config.save();
        }
        self.config_mtime = config_mtime(self.theme_file.as_deref());
        self.request_sync(shell);
    }

    // --- The context menu ---------------------------------------------------

    pub fn show_menu(&mut self, shell: &mut dyn Shell) {
        let autostart = shell.autostart_enabled();
        let (calendars, tasklists, selected_cal, selected_list, update_available, locked) = {
            let g = sync::lock(&self.shared);
            (
                g.calendars.clone(),
                g.tasklists.clone(),
                g.config.calendar_ids.clone(),
                g.config.tasklist_ids.clone(),
                g.update.is_some(),
                g.config.locked,
            )
        };
        let entries = menu::context_menu(&menu::Inputs {
            cat: self.loc.cat,
            autostart,
            can_autostart: autostart::supported(),
            locked,
            calendars: &calendars,
            tasklists: &tasklists,
            selected_calendars: &selected_cal,
            selected_tasklists: &selected_list,
            update_available,
            // No sync thread in demo mode, so nothing would receive it.
            can_relogin: self.sync.is_some(),
        });

        let Some(command) = shell.show_menu(&entries) else {
            return;
        };
        self.run_command(shell, command, autostart);
    }

    /// Carries out a menu choice.
    ///
    /// The `match` is exhaustive over [`menu::Command`] on purpose: a command
    /// added to the core stops compiling here until this front end says what
    /// it does.
    fn run_command(&mut self, shell: &mut dyn Shell, command: menu::Command, autostart: bool) {
        match command {
            menu::Command::Sync => self.request_sync(shell),
            menu::Command::Autostart => {
                shell.set_autostart(!autostart);
                log::info(if autostart {
                    "Autostart disabled"
                } else {
                    "Autostart enabled"
                });
            }
            menu::Command::Lock => self.toggle_lock(shell),
            menu::Command::OpenConfig => {
                // Make sure the file exists before opening it — the editor
                // should not report "not found".
                sync::lock(&self.shared).config.save();
                self.config_mtime = config_mtime(self.theme_file.as_deref());
                shell.open_path(&config::config_path());
            }
            menu::Command::ResetPosition => self.reset_position(shell),
            menu::Command::OpenLog => shell.open_path(&log::file_path()),
            menu::Command::OpenDataFolder => shell.open_path(&config::data_dir()),
            menu::Command::InstallUpdate => self.start_update(shell),
            menu::Command::CopyAgenda => self.copy_agenda(shell),
            menu::Command::Relogin => self.relogin(shell),
            menu::Command::Quit => shell.quit(),
            menu::Command::Calendar(i) => self.toggle_source(shell, true, i),
            menu::Command::Tasklist(i) => self.toggle_source(shell, false, i),
        }
    }

    /// Pins the widget where it is, or lets it go again.
    ///
    /// Written to `config.json` rather than kept in memory: the point of the
    /// lock is that the widget stays put, and a setting that forgets itself at
    /// the next restart would not do that.
    fn toggle_lock(&mut self, shell: &mut dyn Shell) {
        let locked = {
            let mut guard = sync::lock(&self.shared);
            guard.config.locked = !guard.config.locked;
            guard.config.save();
            guard.config.locked
        };
        self.config_mtime = config_mtime(self.theme_file.as_deref());
        log::info(if locked {
            "Position and size locked"
        } else {
            "Position and size unlocked"
        });

        // The pointer may be sitting on a grip that has just stopped being
        // one. Without this the double arrow stays until the mouse next moves.
        self.hover_edge = Edges::default();
        shell.set_cursor(Cursor::Arrow);
    }

    fn reset_position(&mut self, shell: &mut dyn Shell) {
        {
            let mut guard = sync::lock(&self.shared);
            guard.config.x = None;
            guard.config.y = None;
            guard.config.save();
        }
        self.config_mtime = config_mtime(self.theme_file.as_deref());
        let cfg = self.config();
        let rect = self.target_geometry(&cfg, shell);
        shell.set_window_rect(rect);
    }

    // --- Persisting geometry ------------------------------------------------

    /// Stores the new size — in DIPs, and without the shadow margin.
    fn save_geometry(&mut self, shell: &mut dyn Shell) {
        let r = shell.window_rect();
        let scale = shell.scale().max(0.1);
        let mut guard = sync::lock(&self.shared);
        guard.config.x = Some(r.x);
        guard.config.y = Some(r.y);
        guard.config.width = r.width as f32 / scale - self.metrics.shadow * 2.0;
        guard.config.height = r.height as f32 / scale - self.metrics.shadow * 2.0;
        guard.config.save();
        drop(guard);
        // Do not mistake our own save for someone else's edit.
        self.config_mtime = config_mtime(self.theme_file.as_deref());
    }

    fn save_position(&mut self, shell: &mut dyn Shell) {
        let r = shell.window_rect();
        let mut guard = sync::lock(&self.shared);
        guard.config.x = Some(r.x);
        guard.config.y = Some(r.y);
        guard.config.save();
        drop(guard);
        self.config_mtime = config_mtime(self.theme_file.as_deref());
    }

    /// Brings the window back when its position is no longer on any monitor.
    ///
    /// Runs even while the widget is locked, and deliberately so: the lock
    /// exists to stop the *user* moving it by accident, not to let an
    /// unplugged monitor strand it somewhere with no way back.
    pub fn rescue_offscreen(&mut self, shell: &mut dyn Shell) {
        if shell.is_on_screen(shell.window_rect()) {
            return;
        }
        log::warn("Window position is off every monitor — reset");
        {
            let mut guard = sync::lock(&self.shared);
            guard.config.x = None;
            guard.config.y = None;
            guard.config.save();
        }
        self.config_mtime = config_mtime(self.theme_file.as_deref());
        let cfg = self.config();
        let rect = self.target_geometry(&cfg, shell);
        shell.set_window_rect(rect);
    }

    // --- Reacting to the outside --------------------------------------------

    /// The system appearance changed: light/dark, the accent colour, or one of
    /// the accessibility switches.
    pub fn refresh_palette(&mut self, shell: &mut dyn Shell) {
        let visuals = shell.system_visuals();
        if visuals == self.visuals {
            return;
        }
        self.visuals = visuals;
        let cfg = self.config();
        let palette = palette_for(&cfg, &self.appearance, visuals, true);
        let metrics = metrics_for(&cfg, &self.appearance, visuals);
        // The metrics decide the shadow margin, so a contrast theme switching
        // on changes the window's size as well as its colours.
        let metrics_changed = metrics.shadow != self.metrics.shadow;
        self.palette = palette;
        self.metrics = metrics;
        self.anim.enabled = palette.animations;
        self.needs_rebuild = true;
        if metrics_changed {
            let rect = self.target_geometry(&cfg, shell);
            shell.set_window_rect(rect);
        }
        self.kick(shell);
    }

    /// Re-reads `config.json` and any named theme file when either changed.
    ///
    /// Editing the settings is the documented way to configure the widget, and
    /// a change that only took effect at the next restart would make that a
    /// poor promise.
    fn reload_config_if_changed(&mut self, shell: &mut dyn Shell) {
        let stamps = config_mtime(self.theme_file.as_deref());
        if stamps == self.config_mtime {
            return;
        }
        self.config_mtime = stamps;

        let (cfg, error) = Config::load();
        let appearance = appearance_for(&cfg);
        let loc = Locale::resolve(&cfg.language);
        let palette = palette_for(&cfg, &appearance, self.visuals, true);
        let metrics = metrics_for(&cfg, &appearance, self.visuals);

        // Typography, density and the surface style live in the renderer's
        // fonts and in the metrics, so a change to any of them needs the
        // renderer rebuilt rather than merely repainted.
        // `|=`, not `=`: an appearance change may already have asked for a
        // rebuild that the next frame has not picked up yet, and a settings
        // reload that happens to need none must not cancel it.
        self.needs_rebuild |= self.appearance.layout_differs(&appearance)
            || metrics.shadow != self.metrics.shadow
            || loc.rtl != self.loc.rtl
            || loc.tag != self.loc.tag;

        let geometry_changed = cfg.width != self.config().width
            || cfg.height != self.config().height
            || metrics.shadow != self.metrics.shadow;

        self.appearance = appearance;
        self.theme_file = cfg.appearance.file();
        self.metrics = metrics;
        self.palette = palette;
        self.anim.enabled = palette.animations;
        tpmplaner_core::i18n::set_global(loc.cat);
        self.loc = loc;

        {
            let mut guard = sync::lock(&self.shared);
            guard.config = cfg.clone();
            guard.config_error = error;
        }
        if geometry_changed {
            let rect = self.target_geometry(&cfg, shell);
            shell.set_window_rect(rect);
        }
        log::info("Settings reloaded");
        self.kick(shell);
    }

    /// Tell the sync thread to stop, on the way out.
    pub fn shut_down(&mut self) {
        if let Some(s) = self.sync.as_ref() {
            s.send(Command::Quit);
        }
    }
}

/// Metrics for a configuration: the DPI scale, the density and font offset from
/// the customisation, and no shadow margin where nothing draws one.
fn metrics_for(cfg: &Config, custom: &Appearance, visuals: SystemVisuals) -> Metrics {
    Metrics::resolve(
        cfg.scale,
        cfg.backdrop != "acrylic",
        custom,
        ThemePref::parse(&cfg.theme).high_contrast(visuals),
    )
}

/// Resolves the customisation and puts anything it complained about in the log.
fn appearance_for(cfg: &Config) -> Appearance {
    let (custom, notes) = cfg.appearance.resolve();
    for note in notes {
        log::warn(&note);
    }
    custom
}

/// The palette for a configuration, with the customisation applied.
///
/// `quiet` is for the paths that re-resolve after a system appearance change:
/// the notes would be the same ones already in the log.
fn palette_for(cfg: &Config, custom: &Appearance, visuals: SystemVisuals, quiet: bool) -> Palette {
    let mut palette = Palette::resolve(ThemePref::parse(&cfg.theme), &cfg.accent, visuals);
    let notes = palette.customize(custom);
    if !quiet {
        for note in notes {
            log::warn(&note);
        }
    }
    palette
}

fn config_mtime(theme_file: Option<&Path>) -> Stamps {
    let stamp = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    (stamp(&config::config_path()), theme_file.and_then(stamp))
}
