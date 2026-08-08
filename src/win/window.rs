// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! The widget window.
//!
//! It behaves like a Vista gadget:
//!
//! * `WS_EX_NOREDIRECTIONBITMAP` — the prerequisite for the DirectComposition
//!   surface with true per-pixel alpha.
//! * `WS_EX_TOOLWINDOW` — no entry in the taskbar or in Alt-Tab.
//! * `WS_EX_NOACTIVATE` — clicking the widget never takes focus away from the
//!   application you are working in. That is exactly what separates a "gadget"
//!   from "a window in the way".
//! * `HWND_BOTTOM` forced in `WM_WINDOWPOSCHANGING` — the widget stays below
//!   every normal window, but above the desktop and therefore clickable.
//!
//! The window is larger than the visible glass body by [`Metrics::shadow`] on
//! every side; the renderer draws the drop shadow in that margin.
//!
//! Three timers, each alive only as long as it is needed: the minute tick
//! (clock, sync due, configuration check), ~60 Hz during an animation, and
//! 100 ms while an undo grace period is running.

use crate::win::platform;
use crate::win::render::{self, Frame, Hit, HitRegion, Renderer, UndoView};
use chrono::{DateTime, Duration as ChronoDuration, Local, Timelike};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};
use tpmplaner_core::anim::Animations;
use tpmplaner_core::config::{self, Config};
use tpmplaner_core::i18n::Locale;
use tpmplaner_core::log;
use tpmplaner_core::sync::{self, Command, Shared, Status, SyncHandle};
use tpmplaner_core::theme::{Appearance, Metrics, Palette, SystemVisuals, ThemePref};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWMSBT_NONE, DWMSBT_TRANSIENTWINDOW, DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_IMMERSIVE_DARK_MODE,
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DWMWCP_ROUND, DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{HBRUSH, ValidateRect};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
    UnregisterHotKey,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, Result, w};

const TIMER_TICK: usize = 1;
const TIMER_ANIM: usize = 2;
const TIMER_UNDO: usize = 3;
const TIMER_PEEK: usize = 4;
/// Identifier of the global hotkey.
const HOTKEY_PEEK: i32 = 1;

/// Posted by the sync thread when a command finished. The core knows nothing
/// about window messages; `WindowWaker` turns its wake call into this.
const WM_APP_SYNC_DONE: u32 = WM_APP + 1;
/// ~60 Hz. Runs only while something is actually moving.
const ANIM_INTERVAL_MS: u32 = 16;
/// Ten steps a second is plenty for a countdown bar.
const UNDO_INTERVAL_MS: u32 = 100;

/// DWM reporting a changed accent colour. Not defined in
/// `WindowsAndMessaging`, but documented.
const WM_DWMCOLORIZATIONCOLORCHANGED: u32 = 0x0320;

const CMD_SYNC: usize = 1001;
const CMD_AUTOSTART: usize = 1002;
const CMD_CONFIG: usize = 1003;
const CMD_FOLDER: usize = 1004;
const CMD_LOG: usize = 1005;
const CMD_RELOGIN: usize = 1006;
const CMD_RESET_POS: usize = 1007;
const CMD_COPY: usize = 1009;
const CMD_UPDATE: usize = 1011;
const CMD_QUIT: usize = 1010;
/// Identifier ranges for the dynamically created source entries.
const CMD_CALENDAR_BASE: usize = 2000;
const CMD_TASKLIST_BASE: usize = 3000;

/// The edge or edges of the glass body under the mouse pointer.
///
/// The window has no frame (`WS_POPUP` without `WS_THICKFRAME`), so the system
/// provides no resizing either. The few lines here stand in for it — without
/// them every change of width would mean editing the JSON file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edges {
    left: bool,
    right: bool,
    top: bool,
    bottom: bool,
}

impl Edges {
    const NONE: Edges = Edges {
        left: false,
        right: false,
        top: false,
        bottom: false,
    };

    fn any(&self) -> bool {
        self.left || self.right || self.top || self.bottom
    }

    /// The matching cursor: diagonal at the corners, otherwise horizontal or vertical.
    fn cursor(&self) -> PCWSTR {
        match (self.left, self.right, self.top, self.bottom) {
            (true, _, true, _) | (_, true, _, true) => IDC_SIZENWSE,
            (_, true, true, _) | (true, _, _, true) => IDC_SIZENESW,
            (true, _, _, _) | (_, true, _, _) => IDC_SIZEWE,
            (_, _, true, _) | (_, _, _, true) => IDC_SIZENS,
            _ => IDC_ARROW,
        }
    }
}

/// How far from the glass edge the grip still responds (in DIPs).
const RESIZE_GRIP: f32 = 6.0;
/// The smallest sensible size of the glass body, in DIPs.
const MIN_PANEL: (f32, f32) = (240.0, 180.0);

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

struct State {
    hwnd: HWND,
    shared: Arc<Mutex<Shared>>,
    sync: Option<SyncHandle>,
    renderer: Option<Renderer>,

    /// Refilled once per frame rather than allocated again.
    hits: Vec<HitRegion>,
    hover: Option<Hit>,
    tracking_mouse: bool,
    /// Dragging with the left mouse button held down.
    drag: Option<(POINT, POINT)>,
    /// Resizing: the starting state of the cursor and the window rectangle.
    resize: Option<(Edges, POINT, RECT)>,
    /// The edge under the pointer, for switching the cursor.
    hover_edge: Edges,

    anim: Animations,
    animating: bool,
    scroll_target: f32,
    content_height: f32,
    viewport_height: f32,

    pending: Option<Pending>,
    /// Is a peek running? While it is, the window stays on top.
    peeking: bool,

    next_sync_at: DateTime<Local>,
    consecutive_failures: u32,
    last_minute: u32,
    last_stamp: Option<DateTime<Local>>,

    dpi: f32,
    scale: f32,
    metrics: Metrics,
    /// The customisation last read from the settings, resolved from an inline
    /// block or a named theme file. Kept to compare against: typography and
    /// density live in the DirectWrite formats and the metrics, so a change to
    /// any of it has to rebuild the renderer rather than repaint.
    appearance: Appearance,
    /// The named theme file, if `appearance` came from one. Watched alongside
    /// the settings: editing that file is the whole point of having one, and it
    /// leaves `config.json` untouched.
    theme_file: Option<PathBuf>,
    config_mtime: Stamps,
    loc: Locale,
    /// The appearance settings last read from Windows. Re-read on
    /// `WM_SETTINGCHANGE`, and also kept as the value to compare against, so
    /// an unrelated system notification does not trigger a redraw.
    visuals: SystemVisuals,
}

pub fn run() -> Result<()> {
    unsafe {
        // Has to happen before the first window is created, or Windows scales
        // the window up blurrily on monitors with a different DPI.
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

        let (cfg, config_error) = Config::load();
        if let Some(e) = &config_error {
            log::warn(e);
        }
        let appearance = appearance_for(&cfg);
        // Read before the metrics: a contrast theme overrides the surface
        // style, and the surface style decides whether the geometry reserves a
        // margin for the shadow.
        let visuals = platform::system_visuals();
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

        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hbrBackground: HBRUSH::default(),
            lpszClassName: w!("TPMPlanerWidget"),
            ..Default::default()
        };
        if RegisterClassExW(&class) == 0 {
            return Err(windows::core::Error::from_thread());
        }

        let demo = tpmplaner_core::demo::enabled();
        let mut state = Box::new(State {
            hwnd: HWND::default(),
            shared: Arc::new(Mutex::new(Shared {
                agenda: if demo {
                    tpmplaner_core::demo::agenda()
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
            })),
            sync: None,
            renderer: None,
            hits: Vec::with_capacity(64),
            hover: None,
            tracking_mouse: false,
            drag: None,
            resize: None,
            hover_edge: Edges::NONE,
            anim: Animations::default(),
            animating: false,
            scroll_target: 0.0,
            content_height: 0.0,
            viewport_height: 0.0,
            pending: None,
            peeking: false,
            next_sync_at: Local::now(),
            consecutive_failures: 0,
            last_minute: u32::MAX,
            last_stamp: None,
            dpi: 96.0,
            scale: cfg.scale,
            metrics,
            appearance,
            config_mtime: config_mtime(theme_file.as_deref()),
            theme_file,
            loc,
            visuals,
        });
        let shared = state.shared.clone();

        // A provisional size; the real DPI is only known once the window sits
        // on an actual monitor.
        let hwnd = CreateWindowExW(
            WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            w!("TPMPlanerWidget"),
            w!("TPMPlaner"),
            WS_POPUP,
            cfg.x.unwrap_or(0),
            cfg.y.unwrap_or(0),
            cfg.width as i32,
            cfg.height as i32,
            None,
            None,
            Some(instance.into()),
            Some(&mut *state as *mut State as *mut _),
        )?;

        apply_backdrop(hwnd, &cfg, palette.dark);

        let st = &mut *(GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State);
        st.hwnd = hwnd;
        st.dpi = GetDpiForWindow(hwnd) as f32;

        let (px, py, pw, ph) = target_geometry(&cfg, st.metrics, st.dpi);
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_BOTTOM),
            px,
            py,
            pw,
            ph,
            SWP_NOACTIVATE | SWP_NOOWNERZORDER,
        );
        st.renderer = Some(Renderer::new(
            hwnd,
            pw.max(1) as u32,
            ph.max(1) as u32,
            st.dpi,
            st.metrics,
            palette,
            st.loc.rtl,
            &st.appearance,
            &st.loc.tag,
        )?);

        // Demo mode deliberately runs no sync thread — otherwise the missing
        // Google credentials would immediately bury the sample data under an
        // error message.
        if !demo {
            st.sync = Some(sync::spawn(
                shared,
                std::sync::Arc::new(crate::win::host_impl::WindowWaker {
                    hwnd: hwnd.0 as isize,
                    message: WM_APP_SYNC_DONE,
                }),
            ));
            st.sync.as_ref().unwrap().send(Command::Sync);
            st.anim.spinning = true;
        }
        st.anim.enabled = palette.animations;
        st.next_sync_at = Local::now() + ChronoDuration::minutes(cfg.sync_minutes as i64);
        // Cached data is there immediately: show it straight away.
        if sync::lock(&st.shared).agenda.fetched_at.is_some() {
            st.anim.restart_reveal();
        } else {
            st.anim.reveal.jump(1.0);
        }

        register_peek_hotkey(hwnd, &cfg);
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        arm_tick(hwnd);
        kick(st);
        platform::trim_working_set();

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).into() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        Ok(())
    }
}

/// Metrics for a configuration: the DPI scale, the density and font offset
/// from the customisation, and no shadow margin where nothing draws one — the
/// acrylic backdrop fills the whole window rectangle, and a flat surface has
/// no shadow to leave room for.
fn metrics_for(cfg: &Config, custom: &Appearance, visuals: SystemVisuals) -> Metrics {
    Metrics::resolve(
        cfg.scale,
        cfg.backdrop != "acrylic",
        custom,
        // The same question `Palette::resolve` asks. Reading `visuals` alone
        // would leave `"theme": "contrast"` with a high-contrast palette and
        // metrics sized for the configured surface — the exact disagreement
        // `effective_surface` exists to prevent.
        ThemePref::parse(&cfg.theme).high_contrast(visuals),
    )
}

/// Resolves the customisation and puts anything it complained about in the
/// log.
///
/// A theme file that is missing, a colour that is not a colour, text that had
/// to be lightened to stay readable: each of those otherwise looks exactly
/// like a setting that had no effect, which is the hardest kind of thing to
/// work out from the outside.
fn appearance_for(cfg: &Config) -> Appearance {
    let (custom, notes) = cfg.appearance.resolve();
    for note in notes {
        log::warn(&note);
    }
    custom
}

/// The palette for a configuration, with the customisation applied.
fn palette_for(cfg: &Config, custom: &Appearance, visuals: SystemVisuals, quiet: bool) -> Palette {
    let mut palette = Palette::resolve(ThemePref::parse(&cfg.theme), &cfg.accent, visuals);
    let notes = palette.customize(custom);
    // `quiet` is for the paths that re-resolve the same palette after a system
    // appearance change or a lost graphics device. The notes would be the same
    // ones already in the log, and repeating them on every theme switch turns
    // the log into noise.
    if !quiet {
        for note in notes {
            log::warn(&note);
        }
    }
    palette
}

/// Window position and size in physical pixels.
///
/// `cfg.width`/`cfg.height` describe the visible glass body; the window is
/// larger than that by the shadow margin on every side.
fn target_geometry(cfg: &Config, m: Metrics, dpi: f32) -> (i32, i32, i32, i32) {
    let scale = dpi / 96.0;
    let shadow_px = (m.shadow * scale).round() as i32;
    let pw = ((cfg.width + m.shadow * 2.0) * scale).round() as i32;
    let ph = ((cfg.height + m.shadow * 2.0) * scale).round() as i32;

    let (px, py) = match (cfg.x, cfg.y) {
        (Some(x), Some(y)) => (x, y),
        _ => {
            // Top right, like the Vista sidebar. The gap applies to the
            // visible glass edge, not to the invisible shadow margin.
            let margin = (24.0 * scale).round() as i32;
            match platform::primary_work_area() {
                Some(w) => (
                    w.right - margin - pw + shadow_px,
                    w.top + margin - shadow_px,
                ),
                None => (margin, margin),
            }
        }
    };
    (px, py, pw, ph)
}

/// Desktop Window Manager attributes for the window.
///
/// **Important:** `DWMWA_SYSTEMBACKDROP_TYPE` colours the *entire* window
/// rectangle. This window is larger than the visible glass body by the shadow
/// margin on every side, and it draws its own corners — so a system backdrop
/// lays an opaque, *square* box around the rounded panel. That is exactly what
/// was visible for as long as acrylic was set here.
///
/// The normal case is therefore `DWMSBT_NONE`, set explicitly rather than
/// merely omitted: the default `DWMSBT_AUTO` leaves the decision to the system
/// and can produce the same result. The renderer draws the glass itself in any
/// case.
fn apply_backdrop(hwnd: HWND, cfg: &Config, dark: bool) {
    unsafe {
        let dark_flag: i32 = dark as i32;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark_flag as *const i32 as *const _,
            4,
        );

        // With no system backdrop the renderer draws the corners itself using
        // per-pixel alpha, and DWM must not round them as well or the radius
        // is applied twice. With acrylic it is the other way round: there DWM
        // has to round, because it fills the surface.
        let corner = if cfg.backdrop == "acrylic" {
            DWMWCP_ROUND.0
        } else {
            DWMWCP_DONOTROUND.0
        };
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &corner as *const i32 as *const _,
            4,
        );

        // Acrylic only when explicitly asked for — and then without the
        // shadow margin, or the box comes back. See `Metrics::new`, where the
        // margin drops to zero in that case.
        let backdrop = if cfg.backdrop == "acrylic" {
            DWMSBT_TRANSIENTWINDOW.0
        } else {
            DWMSBT_NONE.0
        };
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_SYSTEMBACKDROP_TYPE,
            &backdrop as *const i32 as *const _,
            4,
        );
    }
}

/// Sets the minute timer to the next full minute.
fn arm_tick(hwnd: HWND) {
    let now = Local::now();
    let ms_to_next_minute = 60_000 - (now.second() * 1000 + now.timestamp_subsec_millis()) as i32;
    let elapse = ms_to_next_minute.clamp(1_000, 60_000) as u32;
    unsafe {
        SetTimer(Some(hwnd), TIMER_TICK, elapse, None);
    }
}

// --- Animationsantrieb ------------------------------------------------------

/// Starts the animation timer if needed and takes a step immediately, so the
/// response does not feel delayed by up to 16 ms.
fn kick(st: &mut State) {
    if !st.animating {
        st.animating = true;
        unsafe {
            SetTimer(Some(st.hwnd), TIMER_ANIM, ANIM_INTERVAL_MS, None);
        }
    }
    pump(st);
}

/// One animation step. As soon as nothing is moving any more the timer is
/// switched off — from then on the widget costs nothing again.
fn pump(st: &mut State) {
    let active = st.anim.tick();
    redraw(st);
    if !active && st.animating {
        st.animating = false;
        unsafe {
            KillTimer(Some(st.hwnd), TIMER_ANIM).ok();
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_NCCREATE {
            let cs = lparam.0 as *const CREATESTRUCTW;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, (*cs).lpCreateParams as isize);
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }

        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
        if ptr.is_null() {
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }
        let st = &mut *ptr;

        match msg {
            WM_WINDOWPOSCHANGING => {
                let wp = &mut *(lparam.0 as *mut WINDOWPOS);
                // Normally always right to the back. `SWP_NOZORDER` has to go
                // for that, or Windows ignores `hwndInsertAfter`. During a
                // peek the opposite applies, or the window drops straight back
                // behind everything.
                wp.hwndInsertAfter = if st.peeking {
                    HWND_TOPMOST
                } else {
                    HWND_BOTTOM
                };
                wp.flags &= !SWP_NOZORDER;
                // "Show desktop" (Win+D) tries to hide the window — we insist
                // on staying visible.
                wp.flags &= !SWP_HIDEWINDOW;
                LRESULT(0)
            }

            // A gadget is never minimised.
            WM_SYSCOMMAND if (wparam.0 & 0xFFF0) == SC_MINIMIZE as usize => LRESULT(0),

            // Refuse activation, even if something tries.
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),

            WM_PAINT => {
                redraw(st);
                let _ = ValidateRect(Some(hwnd), None);
                LRESULT(0)
            }

            WM_SIZE => {
                let w = (lparam.0 & 0xFFFF) as u32;
                let h = ((lparam.0 >> 16) & 0xFFFF) as u32;
                if let Some(r) = st.renderer.as_mut() {
                    let _ = r.resize(w.max(1), h.max(1), st.dpi);
                }
                redraw(st);
                LRESULT(0)
            }

            WM_DPICHANGED => {
                st.dpi = (wparam.0 & 0xFFFF) as f32;
                let r = &*(lparam.0 as *const RECT);
                let _ = SetWindowPos(
                    hwnd,
                    Some(HWND_BOTTOM),
                    r.left,
                    r.top,
                    r.right - r.left,
                    r.bottom - r.top,
                    SWP_NOACTIVATE | SWP_NOOWNERZORDER,
                );
                if let Some(rend) = st.renderer.as_mut() {
                    let _ = rend.resize(
                        (r.right - r.left).max(1) as u32,
                        (r.bottom - r.top).max(1) as u32,
                        st.dpi,
                    );
                }
                redraw(st);
                LRESULT(0)
            }

            // The system time or time zone changed (travel, daylight saving):
            // the whole agenda hangs off the local day boundary and off
            // relative times, so fetch it again.
            WM_TIMECHANGE => {
                log::info("System time changed — resyncing");
                st.last_minute = u32::MAX;
                request_sync(st);
                LRESULT(0)
            }

            // Windows is shutting down or logging off: `WM_DESTROY` no longer
            // arrives reliably, and a waiting completion would be lost.
            WM_ENDSESSION => {
                commit_pending(st);
                LRESULT(0)
            }

            // A monitor was unplugged or the resolution changed: check the position.
            WM_DISPLAYCHANGE => {
                rescue_offscreen(st);
                LRESULT(0)
            }

            // Windows switched between light and dark, or changed the accent
            // colour. `WM_SETTINGCHANGE` does carry the reason in lParam, but
            // resolving the palette again is so cheap (one registry value plus
            // one DWM call) that parsing the string is not worth it.
            WM_SETTINGCHANGE | WM_THEMECHANGED | WM_DWMCOLORIZATIONCOLORCHANGED => {
                refresh_palette(st);
                LRESULT(0)
            }

            WM_TIMER if wparam.0 == TIMER_ANIM => {
                pump(st);
                LRESULT(0)
            }

            WM_HOTKEY if wparam.0 as i32 == HOTKEY_PEEK => {
                begin_peek(st);
                LRESULT(0)
            }

            WM_TIMER if wparam.0 == TIMER_PEEK => {
                end_peek(st);
                LRESULT(0)
            }

            WM_TIMER if wparam.0 == TIMER_UNDO => {
                on_undo_tick(st);
                LRESULT(0)
            }

            WM_TIMER if wparam.0 == TIMER_TICK => {
                on_tick(st);
                arm_tick(hwnd);
                LRESULT(0)
            }

            WM_APP_SYNC_DONE => {
                on_sync_done(st);
                platform::trim_working_set();
                LRESULT(0)
            }

            // Back from standby: sync at once rather than showing stale data
            // until the next interval.
            WM_POWERBROADCAST => {
                if wparam.0 as u32 == PBT_APMRESUMEAUTOMATIC
                    || wparam.0 as u32 == PBT_APMRESUMESUSPEND
                {
                    log::info("Back from standby — syncing now");
                    // The display may have changed while asleep (a dock
                    // plugged in or unplugged).
                    rescue_offscreen(st);
                    request_sync(st);
                }
                LRESULT(1)
            }

            WM_MOUSEMOVE => {
                on_mouse_move(st, lparam);
                LRESULT(0)
            }

            WM_MOUSELEAVE => {
                st.tracking_mouse = false;
                if st.hover.take().is_some() {
                    st.anim.hover.set(0.0);
                    kick(st);
                }
                LRESULT(0)
            }

            WM_LBUTTONDOWN => {
                on_left_down(st, lparam);
                LRESULT(0)
            }

            WM_LBUTTONUP => {
                if st.resize.take().is_some() {
                    let _ = ReleaseCapture();
                    save_geometry(st);
                } else if st.drag.take().is_some() {
                    let _ = ReleaseCapture();
                    save_position(st);
                }
                LRESULT(0)
            }

            // Without this Windows resets the class cursor as soon as the
            // mouse moves, and the resize cursor flickers.
            WM_SETCURSOR if st.hover_edge.any() => {
                if let Ok(cursor) = LoadCursorW(None, st.hover_edge.cursor()) {
                    SetCursor(Some(cursor));
                }
                LRESULT(1)
            }

            WM_MOUSEWHEEL => {
                let delta = ((wparam.0 >> 16) & 0xFFFF) as i16 as f32;
                let overflow = (st.content_height - st.viewport_height).max(0.0);
                let next = (st.scroll_target - delta / 120.0 * 52.0).clamp(0.0, overflow);
                if (next - st.scroll_target).abs() > 0.01 {
                    st.scroll_target = next;
                    st.anim.scroll.set(next);
                    // Show the scrollbar; it fades out again by itself.
                    st.anim.scrollbar.jump(1.0);
                    st.anim.scrollbar.set(0.0);
                    kick(st);
                }
                LRESULT(0)
            }

            WM_RBUTTONUP => {
                show_menu(st);
                LRESULT(0)
            }

            WM_DESTROY => {
                // A completion that has not been sent yet must not be lost
                // merely because the widget is closing.
                commit_pending(st);
                if let Some(s) = st.sync.as_ref() {
                    s.send(Command::Quit);
                }
                KillTimer(Some(hwnd), TIMER_TICK).ok();
                KillTimer(Some(hwnd), TIMER_ANIM).ok();
                KillTimer(Some(hwnd), TIMER_UNDO).ok();
                KillTimer(Some(hwnd), TIMER_PEEK).ok();
                let _ = UnregisterHotKey(Some(hwnd), HOTKEY_PEEK);
                log::info("Shut down");
                PostQuitMessage(0);
                LRESULT(0)
            }

            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

// --- Rueckgaengig -----------------------------------------------------------

/// Records the tick provisionally and starts the grace period.
fn begin_pending(st: &mut State, idx: usize) {
    // Only one task waits at a time: a second completion confirms the first
    // immediately.
    commit_pending(st);

    let seconds = sync::lock(&st.shared).config.undo_seconds;
    let ids = {
        let mut guard = sync::lock(&st.shared);
        guard.agenda.tasks.get_mut(idx).map(|t| {
            // Acknowledge visually at once, so the click feels immediate.
            t.completing = true;
            (t.account_id.clone(), t.tasklist_id.clone(), t.id.clone())
        })
    };
    let Some((account_id, tasklist_id, task_id)) = ids else {
        return;
    };

    if seconds == 0 {
        send_completion(st, &account_id, &tasklist_id, &task_id);
        redraw(st);
        return;
    }

    st.pending = Some(Pending {
        account_id,
        task_id,
        tasklist_id,
        started: Instant::now(),
        window: Duration::from_secs(seconds as u64),
    });
    unsafe {
        SetTimer(Some(st.hwnd), TIMER_UNDO, UNDO_INTERVAL_MS, None);
    }
    redraw(st);
}

/// Has the grace period run out? Then send it.
fn on_undo_tick(st: &mut State) {
    let expired = st
        .pending
        .as_ref()
        .is_some_and(|p| p.started.elapsed() >= p.window);
    if expired {
        commit_pending(st);
    }
    redraw(st);
}

/// Sends a waiting completion and ends the grace period.
fn commit_pending(st: &mut State) {
    let Some(p) = st.pending.take() else { return };
    stop_undo_timer(st);
    send_completion(st, &p.account_id, &p.tasklist_id, &p.task_id);
}

/// Takes the completion back — nothing was ever sent to Google.
fn cancel_pending(st: &mut State) {
    let Some(p) = st.pending.take() else { return };
    stop_undo_timer(st);
    let mut guard = sync::lock(&st.shared);
    if let Some(t) = guard.agenda.tasks.iter_mut().find(|t| t.id == p.task_id) {
        t.completing = false;
    }
    drop(guard);
    log::info("Task completion undone");
    redraw(st);
}

fn stop_undo_timer(st: &State) {
    unsafe {
        KillTimer(Some(st.hwnd), TIMER_UNDO).ok();
    }
}

fn send_completion(st: &State, account_id: &str, tasklist_id: &str, task_id: &str) {
    if let Some(s) = st.sync.as_ref() {
        s.send(Command::CompleteTask {
            account_id: account_id.to_owned(),
            tasklist_id: tasklist_id.to_owned(),
            task_id: task_id.to_owned(),
        });
    }
}

/// Switches a calendar or task list on or off.
///
/// An empty list in the configuration means "all of them". Deselecting a
/// source from that state means the selection has to be written out in full
/// first — otherwise the result would be "all" again. Conversely a complete
/// selection is normalised back to "empty", so a calendar added later comes
/// along automatically.
fn toggle_source(st: &mut State, is_calendar: bool, index: usize) {
    {
        let mut guard = sync::lock(&st.shared);
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
    st.config_mtime = config_mtime(st.theme_file.as_deref());
    request_sync(st);
}

/// Runs the published installer and steps aside so it can replace the binary.
///
/// Deliberately a visible console window: this downloads and executes a script
/// from the internet, and the user should be able to watch it rather than
/// having their widget swapped out invisibly.
fn start_update(st: &mut State) {
    let Some(update) = sync::lock(&st.shared).update.clone() else {
        return;
    };
    log::info(&format!("Installing update {}", update.version));

    const INSTALLER: &str =
        "irm https://github.com/JosunLP/TPMPlaner/releases/latest/download/install.ps1 | iex";
    let args = platform::wide(&format!(
        "-NoProfile -ExecutionPolicy Bypass -Command \"{INSTALLER}\""
    ));
    let exe = platform::wide("powershell.exe");
    unsafe {
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        use windows::core::PCWSTR;
        ShellExecuteW(
            None,
            PCWSTR(platform::wide("open").as_ptr()),
            PCWSTR(exe.as_ptr()),
            PCWSTR(args.as_ptr()),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }
    // The installer stops a running instance anyway; leaving first avoids the
    // file lock and makes the restart its job, not ours.
    unsafe {
        let _ = DestroyWindow(st.hwnd);
    }
}

/// Puts the day's plan on the clipboard as text.
fn copy_agenda(st: &mut State) {
    let guard = sync::lock(&st.shared);
    let loc = &st.loc;
    let today = guard
        .agenda
        .day
        .unwrap_or_else(|| Local::now().date_naive());
    let mut out = format!(
        "{} — {}
",
        loc.weekday(today),
        loc.date_line(today)
    );

    out.push_str(&format!(
        "
{}
",
        loc.cat.section_events
    ));
    if guard.agenda.events.is_empty() {
        out.push_str(&format!(
            "  {}
",
            loc.cat.no_events
        ));
    }
    for ev in &guard.agenda.events {
        let when = if ev.all_day {
            loc.cat.all_day.to_string()
        } else {
            match (ev.start, ev.end) {
                (Some(s), Some(e)) => format!("{}-{}", loc.time(s), loc.time(e)),
                (Some(s), None) => loc.time(s),
                _ => String::new(),
            }
        };
        match &ev.location {
            Some(place) => out.push_str(&format!(
                "  {when}  {}  ({place})
",
                ev.title
            )),
            None => out.push_str(&format!(
                "  {when}  {}
",
                ev.title
            )),
        }
    }

    out.push_str(&format!(
        "
{}
",
        loc.cat.section_tasks
    ));
    if guard.agenda.tasks.is_empty() {
        out.push_str(&format!(
            "  {}
",
            loc.cat.no_tasks
        ));
    }
    for task in &guard.agenda.tasks {
        let due = match task.due {
            Some(d) if d == today => loc.cat.today.to_string(),
            Some(d) => loc.day_month(d),
            None => "-".into(),
        };
        // Subtasks indented, as they are on screen.
        let indent = "  ".repeat(task.depth as usize + 1);
        out.push_str(&format!(
            "{indent}[ ] {due}  {}
",
            task.title
        ));
    }
    drop(guard);

    if platform::set_clipboard_text(&out) {
        log::info("Agenda in die Zwischenablage kopiert");
    } else {
        log::warn("Clipboard is not available");
    }
}

/// Combinations tried when the configured one is already taken.
///
/// Measured on a normal Windows 11 desktop: `Ctrl+Alt+K` and `Win+Alt+K` are
/// both refused with `ERROR_HOTKEY_ALREADY_REGISTERED`. Silently doing
/// nothing would leave a documented feature dead, so the widget falls back and
/// records which combination it ended up with.
const PEEK_FALLBACKS: &[&str] = &["Ctrl+Alt+Shift+K", "Ctrl+Shift+F12", "Ctrl+Alt+Y"];

/// Registers the global "peek" hotkey.
fn register_peek_hotkey(hwnd: HWND, cfg: &Config) {
    unsafe {
        let _ = UnregisterHotKey(Some(hwnd), HOTKEY_PEEK);
    }
    if cfg.peek_hotkey.trim().is_empty() {
        return;
    }

    let mut candidates: Vec<&str> = vec![cfg.peek_hotkey.as_str()];
    candidates.extend(
        PEEK_FALLBACKS
            .iter()
            .copied()
            .filter(|f| !f.eq_ignore_ascii_case(cfg.peek_hotkey.trim())),
    );

    for spec in &candidates {
        let Some((modifiers, key)) = platform::parse_hotkey(spec) else {
            log::warn(&format!("peek_hotkey '{spec}' is not a usable combination"));
            continue;
        };
        if try_register(hwnd, modifiers, key) {
            if *spec == cfg.peek_hotkey {
                log::info(&format!("Peek hotkey: {spec}"));
            } else {
                log::warn(&format!(
                    "peek_hotkey '{}' is taken by another application; using {spec} instead",
                    cfg.peek_hotkey
                ));
            }
            return;
        }
    }

    log::warn("No peek hotkey could be registered; set peek_hotkey in config.json");
}

fn try_register(hwnd: HWND, modifiers: u32, key: u32) -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::HOT_KEY_MODIFIERS;
    // Held keys must not repeat-fire.
    const MOD_NOREPEAT: u32 = 0x4000;
    unsafe {
        RegisterHotKey(
            Some(hwnd),
            HOTKEY_PEEK,
            HOT_KEY_MODIFIERS(modifiers | MOD_NOREPEAT),
            key,
        )
        .is_ok()
    }
}

/// Brings the widget forward for a few seconds.
///
/// This is what makes the bottom-most position workable: the widget is never
/// in the way, which also means it is never visible while you work. One key
/// press is enough, and it sinks back on its own afterwards — no click, no
/// change of focus.
fn begin_peek(st: &mut State) {
    let seconds = sync::lock(&st.shared).config.peek_seconds.max(1);
    st.peeking = true;
    unsafe {
        let _ = SetWindowPos(
            st.hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
        );
        SetTimer(Some(st.hwnd), TIMER_PEEK, seconds * 1000, None);
    }
    // Fade in as for new data: the eye should be led to it.
    st.anim.restart_reveal();
    kick(st);
}

fn end_peek(st: &mut State) {
    st.peeking = false;
    unsafe {
        KillTimer(Some(st.hwnd), TIMER_PEEK).ok();
        let _ = SetWindowPos(
            st.hwnd,
            Some(HWND_BOTTOM),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
        );
    }
    redraw(st);
}

// --- Zeitplanung ------------------------------------------------------------

fn on_tick(st: &mut State) {
    let now = Local::now();

    // The day rolled over: the whole agenda has become invalid.
    let stale_day = {
        let guard = sync::lock(&st.shared);
        guard
            .agenda
            .day
            .map(|d| d != now.date_naive())
            .unwrap_or(true)
    };

    if now >= st.next_sync_at || stale_day {
        request_sync(st);
    }

    reload_config_if_changed(st);

    // The clock and the relative times hang off the minute.
    if now.minute() != st.last_minute {
        st.last_minute = now.minute();
        redraw(st);
    }
}

fn request_sync(st: &mut State) {
    if let Some(s) = st.sync.as_ref() {
        s.send(Command::Sync);
    }
    st.anim.spinning = true;
    kick(st);
    // Set provisionally; `on_sync_done` corrects it once the result is in.
    let interval = sync::lock(&st.shared).config.sync_minutes as i64;
    st.next_sync_at = Local::now() + ChronoDuration::minutes(interval);
}

/// Decides when the next sync happens, after every result.
///
/// On failure it backs off exponentially (1, 2, 4, 8 … minutes), capped at
/// the regular interval. That way a longer network outage does not have the
/// widget hammering away at the API quota.
fn on_sync_done(st: &mut State) {
    let (status, interval, stamp) = {
        let guard = sync::lock(&st.shared);
        (
            guard.status.clone(),
            guard.config.sync_minutes as i64,
            guard.agenda.fetched_at,
        )
    };

    match status {
        Status::Idle => {
            st.consecutive_failures = 0;
            st.next_sync_at = Local::now() + ChronoDuration::minutes(interval);
        }
        Status::NeedsSetup(_) | Status::NeedsLogin(_) => {
            // Retrying achieves nothing without the user acting first.
            st.next_sync_at = Local::now() + ChronoDuration::minutes(interval.max(15));
        }
        _ => {
            st.consecutive_failures = st.consecutive_failures.saturating_add(1);
            let backoff = 1i64 << st.consecutive_failures.min(5);
            st.next_sync_at = Local::now() + ChronoDuration::minutes(backoff.min(interval));
        }
    }

    // Only fade in for genuinely new data — a failed attempt should not make
    // the list flash for no reason.
    if stamp != st.last_stamp {
        st.last_stamp = stamp;
        st.anim.restart_reveal();
        // A new list, so the old scroll position may point at nothing.
        st.scroll_target = 0.0;
        st.anim.scroll.set(0.0);
    }
    st.anim.spinning = matches!(status, Status::Syncing);
    kick(st);
}

/// Uebernimmt Aenderungen an `config.json` ohne Neustart.
fn reload_config_if_changed(st: &mut State) {
    if config_mtime(st.theme_file.as_deref()) == st.config_mtime {
        return;
    }

    let (cfg, error) = Config::load();
    if let Some(e) = &error {
        log::warn(e);
    } else {
        log::info("Configuration reloaded");
    }

    let scale_changed = (cfg.scale - st.scale).abs() > f32::EPSILON;
    // Typography, density and the surface style are baked into the DirectWrite
    // formats and the metrics at construction, so a change to any of them
    // needs the same rebuild a change of scale does. A colour is not: it is
    // uploaded per frame, and rebuilding for one would flicker the whole panel
    // every time somebody nudges a value in a theme file.
    let appearance = appearance_for(&cfg);
    let layout_changed = appearance.layout_differs(&st.appearance);
    let appearance_changed = appearance != st.appearance;
    st.appearance = appearance;
    // Stamped after the theme name is known, not before: switching from one
    // named theme to another changes which file is watched, and stamping the
    // outgoing one would leave a mismatch that reloads again on the next tick.
    st.theme_file = cfg.appearance.file();
    st.config_mtime = config_mtime(st.theme_file.as_deref());

    // Compared rather than derived from the settings that feed it. The
    // renderer caches these metrics at construction, so any move at all has to
    // rebuild — and the list of settings that move them has grown twice now
    // (`backdrop` decides the shadow margin, `theme: contrast` overrides the
    // surface style). Asking `Metrics` directly cannot fall behind that list.
    let previous_metrics = st.metrics;
    st.metrics = metrics_for(&cfg, &st.appearance, st.visuals);
    let metrics_changed = st.metrics != previous_metrics;
    let (px, py, pw, ph) = target_geometry(&cfg, st.metrics, st.dpi);
    let palette = palette_for(&cfg, &st.appearance, st.visuals, !appearance_changed);
    st.anim.enabled = palette.animations;

    // Reading direction and the locale name both live in the DirectWrite
    // formats, so a change to either forces the same rebuild as a change of
    // scale. The locale name is not cosmetic: it selects the Han glyph shapes
    // and the line breaking rules — see `render::locale_name`.
    let new_loc = Locale::resolve(&cfg.language);
    let text_layout_changed = new_loc.rtl != st.loc.rtl || new_loc.tag != st.loc.tag;
    tpmplaner_core::i18n::set_global(new_loc.cat);
    st.loc = new_loc;

    st.scale = cfg.scale;
    {
        let mut guard = sync::lock(&st.shared);
        guard.config = cfg.clone();
        guard.config_error = error;
    }
    apply_backdrop(st.hwnd, &cfg, palette.dark);
    register_peek_hotkey(st.hwnd, &cfg);

    unsafe {
        let _ = SetWindowPos(
            st.hwnd,
            Some(HWND_BOTTOM),
            px,
            py,
            pw,
            ph,
            SWP_NOACTIVATE | SWP_NOOWNERZORDER,
        );
    }

    // Font sizes, the family, the header weight and the locale name live in
    // the DirectWrite formats and cannot be changed after the fact, and the
    // metrics are fixed at construction — so a change of scale, of the
    // metrics, of the interface language or of the layout half of the
    // customisation means rebuilding the renderer completely. Colours alone do
    // not, which is what `layout_differs` separates out: they are uploaded per
    // frame, and rebuilding for one would flicker the whole panel every time
    // somebody nudges a value in a theme file.
    if scale_changed || text_layout_changed || layout_changed || metrics_changed {
        recreate_renderer(st, pw.max(1) as u32, ph.max(1) as u32, palette);
    } else if let Some(r) = st.renderer.as_mut() {
        r.set_palette(palette, &st.appearance);
    }
    redraw(st);
}

/// The system theme, accent colour, contrast mode or motion setting changed.
///
/// `WM_SETTINGCHANGE` arrives for all sorts of reasons; without comparing
/// against the last known state, every unrelated system event would trigger a
/// redraw.
fn refresh_palette(st: &mut State) {
    let visuals = platform::system_visuals();
    if visuals == st.visuals {
        return;
    }
    log::info(&format!(
        "System appearance changed: theme {}, high contrast {}, transparency {}, animations {}",
        if visuals.light { "light" } else { "dark" },
        visuals.high_contrast,
        visuals.transparency,
        visuals.animations
    ));
    // Which corrections `customize` reports depends on both of these: contrast
    // decides whether the custom colours are ignored at all, and a light or
    // dark background decides what has to be corrected to stay readable. When
    // either moves, the notes are genuinely new rather than the repetition
    // `quiet` exists to suppress, and staying silent would hide the one that
    // says the custom colours are being ignored.
    let contrast_changed = visuals.high_contrast != st.visuals.high_contrast;
    let notes_would_differ = contrast_changed || visuals.light != st.visuals.light;
    st.visuals = visuals;

    let cfg = sync::lock(&st.shared).config.clone();
    let palette = palette_for(&cfg, &st.appearance, st.visuals, !notes_would_differ);
    st.anim.enabled = palette.animations;
    apply_backdrop(st.hwnd, &cfg, palette.dark);

    // Contrast overrides the surface style, and the surface style decides
    // whether the geometry reserves a shadow margin — so the window has to be
    // resized and the renderer rebuilt around the new metrics, exactly as a
    // change to the setting itself would.
    if contrast_changed {
        let metrics = metrics_for(&cfg, &st.appearance, st.visuals);
        let moved = metrics != st.metrics;
        st.metrics = metrics;
        if moved {
            let (px, py, pw, ph) = target_geometry(&cfg, st.metrics, st.dpi);
            unsafe {
                let _ = SetWindowPos(
                    st.hwnd,
                    Some(HWND_BOTTOM),
                    px,
                    py,
                    pw,
                    ph,
                    SWP_NOACTIVATE | SWP_NOOWNERZORDER,
                );
            }
            recreate_renderer(st, pw.max(1) as u32, ph.max(1) as u32, palette);
            redraw(st);
            return;
        }
    }

    if let Some(r) = st.renderer.as_mut() {
        r.set_palette(palette, &st.appearance);
    }
    redraw(st);
}

/// When the settings and the named theme file were last written.
///
/// Both, because a named theme lives in its own file: watching only
/// `config.json` meant `"appearance": "midnight"` never picked up an edit to
/// `midnight.theme.json` until the settings file happened to be rewritten for
/// some unrelated reason — which is the advertised way to use a theme.
///
/// `None` in either slot is "no such file", and that is a state worth noticing
/// rather than ignoring: a theme file appearing or being deleted changes the
/// appearance just as much as an edit to one.
type Stamps = (Option<SystemTime>, Option<SystemTime>);

fn config_mtime(theme_file: Option<&Path>) -> Stamps {
    fn stamp(path: &Path) -> Option<SystemTime> {
        std::fs::metadata(path).ok().and_then(|m| m.modified().ok())
    }
    (stamp(&config::config_path()), theme_file.and_then(stamp))
}

// --- Maus -------------------------------------------------------------------

fn on_mouse_move(st: &mut State, lparam: LPARAM) {
    unsafe {
        if !st.tracking_mouse {
            let mut tme = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: st.hwnd,
                dwHoverTime: 0,
            };
            let _ = TrackMouseEvent(&mut tme);
            st.tracking_mouse = true;
        }

        if st.resize.is_some() {
            apply_resize(st);
            return;
        }

        if let Some((start_cursor, start_window)) = st.drag {
            let mut cursor = POINT::default();
            let _ = GetCursorPos(&mut cursor);
            let _ = SetWindowPos(
                st.hwnd,
                Some(HWND_BOTTOM),
                start_window.x + (cursor.x - start_cursor.x),
                start_window.y + (cursor.y - start_cursor.y),
                0,
                0,
                SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
            );
            return;
        }
    }

    let (x, y) = client_dip(st, lparam);

    // The edge outranks row highlighting: otherwise the grip competes with
    // the hover state of the row underneath.
    let edge = edge_at(st, x, y);
    if edge != st.hover_edge {
        st.hover_edge = edge;
        unsafe {
            if let Ok(cursor) = LoadCursorW(None, edge.cursor()) {
                SetCursor(Some(cursor));
            }
        }
    }
    if edge.any() {
        if st.hover.take().is_some() {
            st.anim.hover.set(0.0);
            kick(st);
        }
        return;
    }

    let hover = hit_at(st, x, y);
    if hover != st.hover {
        st.hover = hover;
        // Fade in again on a change rather than jumping.
        st.anim.hover.jump(0.0);
        st.anim.hover.set(if hover.is_some() { 1.0 } else { 0.0 });
        kick(st);
    }
}

fn on_left_down(st: &mut State, lparam: LPARAM) {
    let (x, y) = client_dip(st, lparam);

    let edge = edge_at(st, x, y);
    if edge.any() {
        unsafe {
            let mut cursor = POINT::default();
            let _ = GetCursorPos(&mut cursor);
            let mut wr = RECT::default();
            let _ = GetWindowRect(st.hwnd, &mut wr);
            st.resize = Some((edge, cursor, wr));
            SetCapture(st.hwnd);
        }
        return;
    }

    match hit_at(st, x, y) {
        Some(Hit::Refresh) => request_sync(st),

        // Sits on top of the row for as long as the grace period runs.
        Some(Hit::Undo(_)) => cancel_pending(st),

        Some(Hit::TaskCheck(idx)) => begin_pending(st, idx),

        Some(Hit::Event(idx)) | Some(Hit::Hero(idx)) => open_event(st, idx, false),
        Some(Hit::Tomorrow(idx)) => open_event(st, idx, true),

        Some(Hit::Task(_)) => platform::open_in_browser("https://tasks.google.com/"),

        Some(Hit::StatusAction) => {
            let (status, has_config_error) = {
                let guard = sync::lock(&st.shared);
                (guard.status.clone(), guard.config_error.is_some())
            };
            let has_update = sync::lock(&st.shared).update.is_some();
            if has_config_error {
                platform::open_path(&config::config_path());
            } else if has_update && matches!(status, Status::Idle) {
                start_update(st);
            } else {
                match status {
                    Status::NeedsSetup(_) => platform::open_path(&config::data_dir()),
                    Status::NeedsLogin(_) => {
                        if let Some(s) = st.sync.as_ref() {
                            s.send(Command::Relogin);
                        }
                        st.anim.spinning = true;
                        kick(st);
                    }
                    // On a sync failure the full text is in the log.
                    Status::Error(_) => platform::open_path(&log::file_path()),
                    _ => request_sync(st),
                }
            }
        }

        // Empty space: drag the window.
        None => unsafe {
            let mut cursor = POINT::default();
            let _ = GetCursorPos(&mut cursor);
            let mut wr = RECT::default();
            let _ = GetWindowRect(st.hwnd, &mut wr);
            st.drag = Some((
                cursor,
                POINT {
                    x: wr.left,
                    y: wr.top,
                },
            ));
            SetCapture(st.hwnd);
        },
    }
}

/// Opens an event in its calendar.
fn open_event(st: &State, idx: usize, tomorrow: bool) {
    let guard = sync::lock(&st.shared);
    let list = if tomorrow {
        &guard.agenda.tomorrow
    } else {
        &guard.agenda.events
    };
    let link = list.get(idx).and_then(|e| e.html_link.clone());
    drop(guard);
    if let Some(link) = link {
        platform::open_in_browser(&link);
    }
}

/// Which edge is under the pointer? Coordinates in DIPs.
fn edge_at(st: &State, x: f32, y: f32) -> Edges {
    let Some(r) = st.renderer.as_ref() else {
        return Edges::NONE;
    };
    let (w, h) = r.size_dip();
    // The glass body is inset by the shadow margin; the grip sits on *its*
    // edge, not on the invisible window edge.
    let s = st.metrics.shadow;
    let (l, t, rgt, b) = (s, s, w - s, h - s);
    if x < l - RESIZE_GRIP || x > rgt + RESIZE_GRIP || y < t - RESIZE_GRIP || y > b + RESIZE_GRIP {
        return Edges::NONE;
    }
    Edges {
        left: (x - l).abs() <= RESIZE_GRIP,
        right: (x - rgt).abs() <= RESIZE_GRIP,
        top: (y - t).abs() <= RESIZE_GRIP,
        bottom: (y - b).abs() <= RESIZE_GRIP,
    }
}

/// Drags the window to its new size by the edge that was grabbed.
fn apply_resize(st: &mut State) {
    let Some((edges, start_cursor, start_rect)) = st.resize else {
        return;
    };
    unsafe {
        let mut cursor = POINT::default();
        let _ = GetCursorPos(&mut cursor);
        let (dx, dy) = (cursor.x - start_cursor.x, cursor.y - start_cursor.y);

        let scale = st.dpi / 96.0;
        let min_w = ((MIN_PANEL.0 + st.metrics.shadow * 2.0) * scale) as i32;
        let min_h = ((MIN_PANEL.1 + st.metrics.shadow * 2.0) * scale) as i32;

        let mut r = start_rect;
        if edges.left {
            // Dragging the left edge moves the origin with it, so the minimum
            // width has to constrain the *left* side.
            r.left = (r.left + dx).min(r.right - min_w);
        }
        if edges.right {
            r.right = (r.right + dx).max(r.left + min_w);
        }
        if edges.top {
            r.top = (r.top + dy).min(r.bottom - min_h);
        }
        if edges.bottom {
            r.bottom = (r.bottom + dy).max(r.top + min_h);
        }

        let _ = SetWindowPos(
            st.hwnd,
            Some(HWND_BOTTOM),
            r.left,
            r.top,
            r.right - r.left,
            r.bottom - r.top,
            SWP_NOACTIVATE | SWP_NOOWNERZORDER,
        );
    }
}

/// Stores the new size — in DIPs, and without the shadow margin.
fn save_geometry(st: &mut State) {
    unsafe {
        let mut wr = RECT::default();
        if GetWindowRect(st.hwnd, &mut wr).is_err() {
            return;
        }
        let scale = st.dpi / 96.0;
        let mut guard = sync::lock(&st.shared);
        guard.config.x = Some(wr.left);
        guard.config.y = Some(wr.top);
        guard.config.width = (wr.right - wr.left) as f32 / scale - st.metrics.shadow * 2.0;
        guard.config.height = (wr.bottom - wr.top) as f32 / scale - st.metrics.shadow * 2.0;
        guard.config.save();
    }
    st.config_mtime = config_mtime(st.theme_file.as_deref());
}

/// Hit testing back to front: the regions drawn last (those on top) win —
/// which is how the tick circle beats the task row, and the undo area beats
/// them both.
fn hit_at(st: &State, x: f32, y: f32) -> Option<Hit> {
    st.hits
        .iter()
        .rev()
        .find(|r| r.contains(x, y))
        .map(|r| r.hit)
}

/// Converts a mouse position from `LPARAM` (physical pixels) into DIPs.
fn client_dip(st: &State, lparam: LPARAM) -> (f32, f32) {
    let x = (lparam.0 & 0xFFFF) as i16 as f32;
    let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as f32;
    let k = 96.0 / st.dpi;
    (x * k, y * k)
}

fn save_position(st: &mut State) {
    unsafe {
        let mut wr = RECT::default();
        if GetWindowRect(st.hwnd, &mut wr).is_ok() {
            let mut guard = sync::lock(&st.shared);
            guard.config.x = Some(wr.left);
            guard.config.y = Some(wr.top);
            guard.config.save();
        }
    }
    // Do not mistake our own save for someone else's edit.
    st.config_mtime = config_mtime(st.theme_file.as_deref());
}

/// Brings the window back when its position is no longer on any monitor.
fn rescue_offscreen(st: &mut State) {
    unsafe {
        let mut wr = RECT::default();
        if GetWindowRect(st.hwnd, &mut wr).is_err() || platform::is_on_screen(&wr) {
            return;
        }
        log::warn("Window position is off every monitor — reset");
        let cfg = {
            let mut guard = sync::lock(&st.shared);
            guard.config.x = None;
            guard.config.y = None;
            guard.config.save();
            guard.config.clone()
        };
        st.config_mtime = config_mtime(st.theme_file.as_deref());

        let (px, py, pw, ph) = target_geometry(&cfg, st.metrics, st.dpi);
        let _ = SetWindowPos(
            st.hwnd,
            Some(HWND_BOTTOM),
            px,
            py,
            pw,
            ph,
            SWP_NOACTIVATE | SWP_NOOWNERZORDER,
        );
    }
}

// --- Zeichnen ---------------------------------------------------------------

fn redraw(st: &mut State) {
    // Copy the Arc, not the contents: during an animation this runs 60 times
    // a second, and a full clone of the agenda would mean thousands of
    // allocations per second for data that is not changing.
    let shared = st.shared.clone();
    let guard = sync::lock(&shared);

    let undo = st.pending.as_ref().map(|p| UndoView {
        task_id: p.task_id.as_str(),
        remaining: 1.0
            - (p.started.elapsed().as_secs_f32() / p.window.as_secs_f32()).clamp(0.0, 1.0),
    });

    let draw_result = {
        let anim = &st.anim;
        let hover = st.hover;
        let loc = &st.loc;
        let hits = &mut st.hits;
        let Some(renderer) = st.renderer.as_mut() else {
            return;
        };
        let frame = Frame {
            agenda: &guard.agenda,
            loc,
            status: &guard.status,
            anim,
            now: Local::now(),
            hover,
            sync_minutes: guard.config.sync_minutes,
            opacity: guard.config.opacity,
            show_past_events: guard.config.show_past_events,
            undo,
            config_error: guard.config_error.as_deref(),
            update: guard.update.as_ref().map(|u| u.version.as_str()),
        };
        renderer.draw(&frame, hits)
    };
    drop(guard);

    match draw_result {
        Ok(result) => {
            st.content_height = result.content_height;
            st.viewport_height = result.viewport_height;
            // After rows are removed the scroll offset can point at nothing —
            // pull it back in that case.
            let overflow = (st.content_height - st.viewport_height).max(0.0);
            if st.scroll_target > overflow {
                st.scroll_target = overflow;
                st.anim.scroll.set(overflow);
            }
        }
        Err(e) if render::is_device_lost(e.code()) => {
            // A driver change, a GPU reset or a move into an RDP session: the
            // entire device chain is invalid. Rebuild it and draw again at the
            // next opportunity.
            log::warn(&format!("Graphics device lost ({e}) — rebuilding"));
            let (w, h) = current_size_px(st.hwnd);
            let pal = st.renderer.as_ref().map(|r| r.palette());
            let cfg = sync::lock(&st.shared).config.clone();
            let pal = pal.unwrap_or_else(|| palette_for(&cfg, &st.appearance, st.visuals, true));
            recreate_renderer(st, w, h, pal);
        }
        Err(_) => {}
    }
}

fn recreate_renderer(st: &mut State, width_px: u32, height_px: u32, pal: Palette) {
    // Release first: otherwise the swapchain and the composition target still
    // hold references to the lost device.
    st.renderer = None;
    st.renderer = Renderer::new(
        st.hwnd,
        width_px.max(1),
        height_px.max(1),
        st.dpi,
        st.metrics,
        pal,
        st.loc.rtl,
        &st.appearance,
        &st.loc.tag,
    )
    .ok();
    if st.renderer.is_none() {
        log::error("The renderer could not be rebuilt");
    }
}

fn current_size_px(hwnd: HWND) -> (u32, u32) {
    unsafe {
        let mut r = RECT::default();
        if GetClientRect(hwnd, &mut r).is_ok() {
            (
                (r.right - r.left).max(1) as u32,
                (r.bottom - r.top).max(1) as u32,
            )
        } else {
            (1, 1)
        }
    }
}

// --- Kontextmenue -----------------------------------------------------------

fn show_menu(st: &mut State) {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let autostart = platform::autostart_enabled();
        let c = st.loc.cat;

        // The labels come from the catalogue and therefore have to be
        // converted to UTF-16 at run time; `w!()` only handles literals.
        let item = |flags: MENU_ITEM_FLAGS, id: usize, label: &str| {
            let text = platform::wide(label);
            let _ = AppendMenuW(menu, flags, id, PCWSTR(text.as_ptr()));
        };

        item(MF_STRING, CMD_SYNC, c.menu_sync);
        item(
            if autostart {
                MF_STRING | MF_CHECKED
            } else {
                MF_STRING
            },
            CMD_AUTOSTART,
            c.menu_autostart,
        );
        // Select and deselect sources straight from the menu. The ids are
        // long, email-like strings, and copying them into the JSON by hand was
        // the most unpleasant part of the setup.
        let (calendars, tasklists, selected_cal, selected_list) = {
            let g = sync::lock(&st.shared);
            (
                g.calendars.clone(),
                g.tasklists.clone(),
                g.config.calendar_ids.clone(),
                g.config.tasklist_ids.clone(),
            )
        };
        let mut sources = Vec::new();
        if !calendars.is_empty() {
            sources.push((
                c.menu_calendars,
                &calendars,
                &selected_cal,
                CMD_CALENDAR_BASE,
            ));
        }
        if !tasklists.is_empty() {
            sources.push((
                c.menu_tasklists,
                &tasklists,
                &selected_list,
                CMD_TASKLIST_BASE,
            ));
        }
        if !sources.is_empty() {
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        }
        // Untermenues muessen leben, bis `TrackPopupMenu` zurueckkehrt.
        let mut submenus = Vec::new();
        for (label, entries, selected, base) in sources {
            let Ok(sub) = CreatePopupMenu() else { continue };
            for (i, (id, name)) in entries.iter().enumerate() {
                // An empty selection means "all" — so everything is ticked.
                let checked = selected.is_empty() || selected.contains(id);
                let text = platform::wide(name);
                let _ = AppendMenuW(
                    sub,
                    if checked {
                        MF_STRING | MF_CHECKED
                    } else {
                        MF_STRING
                    },
                    base + i,
                    PCWSTR(text.as_ptr()),
                );
            }
            let text = platform::wide(label);
            let _ = AppendMenuW(menu, MF_POPUP, sub.0 as usize, PCWSTR(text.as_ptr()));
            submenus.push(sub);
        }

        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        // Only offered when there is something to install.
        if sync::lock(&st.shared).update.is_some() {
            item(MF_STRING, CMD_UPDATE, c.menu_update);
        }
        item(MF_STRING, CMD_COPY, c.menu_copy);
        item(MF_STRING, CMD_CONFIG, c.menu_config);
        item(MF_STRING, CMD_RESET_POS, c.menu_reset_pos);
        item(MF_STRING, CMD_LOG, c.menu_log);
        item(MF_STRING, CMD_FOLDER, c.menu_folder);
        item(MF_STRING, CMD_RELOGIN, c.menu_relogin);
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        item(MF_STRING, CMD_QUIT, c.menu_quit);

        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);

        // Without a foreground window the menu would stay open after a click
        // beside it; the WM_NULL message afterwards is the documented
        // companion workaround.
        let _ = SetForegroundWindow(st.hwnd);
        // With right-to-left reading the menu opens at the right edge of the
        // pointer, the way the system does it too.
        let align = if st.loc.rtl {
            TPM_RIGHTALIGN | TPM_LAYOUTRTL
        } else {
            TPM_LEFTALIGN
        };
        let choice = TrackPopupMenu(
            menu,
            TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NONOTIFY | align,
            pt.x,
            pt.y,
            None,
            st.hwnd,
            None,
        );
        let _ = PostMessageW(Some(st.hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        for sub in submenus {
            let _ = DestroyMenu(sub);
        }
        let _ = DestroyMenu(menu);

        match choice.0 as usize {
            CMD_SYNC => request_sync(st),
            CMD_AUTOSTART => {
                platform::set_autostart(!autostart);
                log::info(if autostart {
                    "Autostart deaktiviert"
                } else {
                    "Autostart aktiviert"
                });
            }
            CMD_CONFIG => {
                // Make sure the file exists before opening it — the editor
                // should not report "not found".
                sync::lock(&st.shared).config.save();
                st.config_mtime = config_mtime(st.theme_file.as_deref());
                platform::open_path(&config::config_path());
            }
            CMD_RESET_POS => {
                {
                    let mut guard = sync::lock(&st.shared);
                    guard.config.x = None;
                    guard.config.y = None;
                    guard.config.save();
                }
                st.config_mtime = config_mtime(st.theme_file.as_deref());
                let cfg = sync::lock(&st.shared).config.clone();
                let (px, py, pw, ph) = target_geometry(&cfg, st.metrics, st.dpi);
                let _ = SetWindowPos(
                    st.hwnd,
                    Some(HWND_BOTTOM),
                    px,
                    py,
                    pw,
                    ph,
                    SWP_NOACTIVATE | SWP_NOOWNERZORDER,
                );
            }
            CMD_UPDATE => start_update(st),
            CMD_COPY => copy_agenda(st),
            CMD_LOG => platform::open_path(&log::file_path()),
            id if (CMD_CALENDAR_BASE..CMD_CALENDAR_BASE + calendars.len()).contains(&id) => {
                toggle_source(st, true, id - CMD_CALENDAR_BASE);
            }
            id if (CMD_TASKLIST_BASE..CMD_TASKLIST_BASE + tasklists.len()).contains(&id) => {
                toggle_source(st, false, id - CMD_TASKLIST_BASE);
            }
            CMD_FOLDER => platform::open_path(&config::data_dir()),
            CMD_RELOGIN => {
                if let Some(s) = st.sync.as_ref() {
                    s.send(Command::Relogin);
                }
                st.anim.spinning = true;
                kick(st);
            }
            CMD_QUIT => {
                let _ = DestroyWindow(st.hwnd);
            }
            _ => {}
        }
    }
}
