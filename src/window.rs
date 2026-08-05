// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! Das Widget-Fenster.
//!
//! Verhalten wie ein Vista-Gadget:
//!
//! * `WS_EX_NOREDIRECTIONBITMAP` — Voraussetzung fuer die DirectComposition-
//!   Oberflaeche mit echtem Per-Pixel-Alpha.
//! * `WS_EX_TOOLWINDOW` — kein Eintrag in Taskleiste und Alt-Tab.
//! * `WS_EX_NOACTIVATE` — ein Klick auf das Widget nimmt der laufenden
//!   Anwendung nicht den Fokus. Genau das macht den Unterschied zwischen
//!   "Gadget" und "stoerendes Fenster".
//! * Erzwungenes `HWND_BOTTOM` in `WM_WINDOWPOSCHANGING` — das Widget bleibt
//!   unter allen normalen Fenstern, aber ueber dem Desktop und damit klickbar.
//!
//! Das Fenster ist rundum um [`Metrics::shadow`] groesser als der sichtbare
//! Glaskoerper; in diesem Rand zeichnet der Renderer den Schlagschatten.
//!
//! Drei Timer, jeder nur so lange aktiv wie noetig:
//! Minutentakt (Uhr, Sync-Faelligkeit, Konfigurationspruefung), ~60 Hz
//! waehrend einer Animation, 100 ms waehrend einer laufenden Bedenkzeit
//! zum Rueckgaengigmachen.

use crate::anim::Animations;
use crate::config::{self, Config};
use crate::i18n::Locale;
use crate::log;
use crate::platform;
use crate::render::{self, Frame, Hit, HitRegion, Renderer, UndoView};
use crate::sync::{self, Command, Shared, Status, SyncHandle, WM_APP_STATUS, WM_APP_SYNC_DONE};
use crate::theme::{Metrics, Palette, ThemePref};
use chrono::{DateTime, Duration as ChronoDuration, Local, Timelike};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};
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
/// Kennung der globalen Tastenkombination.
const HOTKEY_PEEK: i32 = 1;
/// ~60 Hz. Laeuft ausschliesslich, solange etwas in Bewegung ist.
const ANIM_INTERVAL_MS: u32 = 16;
/// Zehn Schritte pro Sekunde reichen fuer einen Ablaufbalken voellig.
const UNDO_INTERVAL_MS: u32 = 100;

/// DWM meldet eine geaenderte Akzentfarbe. In `WindowsAndMessaging` nicht
/// definiert, aber dokumentiert.
const WM_DWMCOLORIZATIONCOLORCHANGED: u32 = 0x0320;

const CMD_SYNC: usize = 1001;
const CMD_AUTOSTART: usize = 1002;
const CMD_CONFIG: usize = 1003;
const CMD_FOLDER: usize = 1004;
const CMD_LOG: usize = 1005;
const CMD_RELOGIN: usize = 1006;
const CMD_RESET_POS: usize = 1007;
const CMD_COPY: usize = 1009;
const CMD_QUIT: usize = 1010;
/// Kennungsbereiche fuer die dynamisch erzeugten Quellen-Eintraege.
const CMD_CALENDAR_BASE: usize = 2000;
const CMD_TASKLIST_BASE: usize = 3000;

/// Kante(n) des Glaskoerpers unter dem Mauszeiger.
///
/// Das Fenster hat keinen Rahmen (`WS_POPUP` ohne `WS_THICKFRAME`), also gibt
/// es auch keine Groessenaenderung vom System. Die paar Zeilen hier ersetzen
/// sie — sonst muesste man fuer jede Breitenaenderung die JSON-Datei
/// bearbeiten.
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

    /// Passender Mauszeiger: diagonal an den Ecken, sonst waagerecht/senkrecht.
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

/// Wie weit von der Glaskante entfernt der Griff noch anspricht (in DIPs).
const RESIZE_GRIP: f32 = 6.0;
/// Kleinste sinnvolle Groesse des Glaskoerpers in DIPs.
const MIN_PANEL: (f32, f32) = (240.0, 180.0);

/// Abhaken, das noch nicht abgeschickt wurde.
///
/// Ein Klick auf einen 14 Pixel grossen Kreis passiert auf dem Desktop auch
/// mal versehentlich. Ohne Bedenkzeit waere die Aufgabe sofort und ohne
/// Rueckweg erledigt — deshalb wandert sie erst nach Ablauf zur API.
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

    /// Wird pro Frame neu befuellt statt neu alloziert.
    hits: Vec<HitRegion>,
    hover: Option<Hit>,
    tracking_mouse: bool,
    /// Ziehen mit gedrueckter linker Maustaste.
    drag: Option<(POINT, POINT)>,
    /// Groessenaenderung: Startzustand von Cursor und Fensterrechteck.
    resize: Option<(Edges, POINT, RECT)>,
    /// Kante unter dem Mauszeiger, fuer den Mauszeigerwechsel.
    hover_edge: Edges,

    anim: Animations,
    animating: bool,
    scroll_target: f32,
    content_height: f32,
    viewport_height: f32,

    pending: Option<Pending>,
    /// Laeuft gerade ein "Kurz zeigen"? Solange bleibt das Fenster oben.
    peeking: bool,

    next_sync_at: DateTime<Local>,
    consecutive_failures: u32,
    last_minute: u32,
    last_stamp: Option<DateTime<Local>>,

    dpi: f32,
    scale: f32,
    metrics: Metrics,
    config_mtime: Option<SystemTime>,
    loc: Locale,
    /// Zuletzt gelesene Darstellungseinstellungen von Windows. Wird bei
    /// `WM_SETTINGCHANGE` neu bestimmt und dient zugleich als Vergleichswert,
    /// damit ein unbeteiligter Systemhinweis kein Neuzeichnen ausloest.
    visuals: platform::SystemVisuals,
}

pub fn run() -> Result<()> {
    unsafe {
        // Muss vor der ersten Fenstererzeugung passieren, sonst skaliert
        // Windows das Fenster auf Monitoren mit anderer DPI unscharf hoch.
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

        let (cfg, config_error) = Config::load();
        if let Some(e) = &config_error {
            log::warn(e);
        }
        let metrics = metrics_for(&cfg);
        let loc = Locale::resolve(&cfg.language);
        // Sync-Thread und Notausgang haben keinen Zugriff auf diese Instanz
        // und greifen deshalb auf den globalen Katalog zu.
        crate::i18n::set_global(loc.cat);
        let visuals = platform::system_visuals();
        let palette = Palette::resolve(ThemePref::parse(&cfg.theme), &cfg.accent, visuals);
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

        let demo = crate::demo::enabled();
        let mut state = Box::new(State {
            hwnd: HWND::default(),
            shared: Arc::new(Mutex::new(Shared {
                agenda: if demo {
                    crate::demo::agenda()
                } else {
                    // Letzter bekannter Stand, damit beim Start nicht erst
                    // eine leere Flaeche steht.
                    sync::read_cache().unwrap_or_default()
                },
                status: if demo { Status::Idle } else { Status::Syncing },
                config: cfg.clone(),
                config_error,
                calendars: Vec::new(),
                tasklists: Vec::new(),
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
            config_mtime: config_mtime(),
            loc,
            visuals,
        });
        let shared = state.shared.clone();

        // Vorlaeufige Groesse; die echte DPI kennen wir erst, wenn das Fenster
        // auf einem konkreten Monitor liegt.
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
            cfg.scale,
            palette,
            st.loc.rtl,
        )?);

        // Im Vorschaumodus laeuft bewusst kein Sync-Thread — sonst wuerde der
        // fehlende Google-Zugang die Beispieldaten sofort mit einer
        // Fehlermeldung ueberdecken.
        if !demo {
            st.sync = Some(sync::spawn(shared, hwnd.0 as isize));
            st.sync.as_ref().unwrap().send(Command::Sync);
            st.anim.spinning = true;
        }
        st.anim.enabled = palette.animations;
        st.next_sync_at = Local::now() + ChronoDuration::minutes(cfg.sync_minutes as i64);
        // Zwischengespeicherte Daten sind sofort da: gleich sichtbar machen.
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

/// Masse zur Konfiguration: im Acryl-Modus ohne Schattenrand.
fn metrics_for(cfg: &Config) -> Metrics {
    Metrics::with_shadow(cfg.scale, cfg.backdrop != "acrylic")
}

/// Fensterposition und -groesse in physischen Pixeln.
///
/// `cfg.width`/`cfg.height` beschreiben den sichtbaren Glaskoerper; das
/// Fenster ist rundum um den Schattenrand groesser.
fn target_geometry(cfg: &Config, m: Metrics, dpi: f32) -> (i32, i32, i32, i32) {
    let scale = dpi / 96.0;
    let shadow_px = (m.shadow * scale).round() as i32;
    let pw = ((cfg.width + m.shadow * 2.0) * scale).round() as i32;
    let ph = ((cfg.height + m.shadow * 2.0) * scale).round() as i32;

    let (px, py) = match (cfg.x, cfg.y) {
        (Some(x), Some(y)) => (x, y),
        _ => {
            // Oben rechts wie die Vista-Sidebar. Der Abstand gilt fuer die
            // sichtbare Glaskante, nicht fuer den unsichtbaren Schattenrand.
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

/// Fensterattribute des Desktopfenster-Managers.
///
/// **Wichtig:** `DWMWA_SYSTEMBACKDROP_TYPE` faerbt das *gesamte*
/// Fensterrechteck. Dieses Fenster ist rundum um den Schattenrand groesser
/// als der sichtbare Glaskoerper, und die Ecken laesst es sich selbst zeichnen
/// — eine Systembackdrop legt deshalb einen deckenden, *eckigen* Kasten um
/// das runde Panel. Genau das war sichtbar, solange hier Acryl gesetzt wurde.
///
/// Der Normalfall ist daher `DWMSBT_NONE`, explizit gesetzt statt nur
/// weggelassen: die Vorgabe `DWMSBT_AUTO` ueberlaesst die Entscheidung dem
/// System und kann dasselbe Ergebnis liefern. Das Glas zeichnet der Renderer
/// ohnehin selbst.
fn apply_backdrop(hwnd: HWND, cfg: &Config, dark: bool) {
    unsafe {
        let dark_flag: i32 = dark as i32;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark_flag as *const i32 as *const _,
            4,
        );

        // Ohne Systembackdrop zeichnet der Renderer die Ecken selbst mit
        // Per-Pixel-Alpha; DWM darf dann nicht zusaetzlich runden, sonst
        // entsteht ein doppelter Radius. Mit Acryl ist es umgekehrt: dort
        // muss DWM runden, weil es die Flaeche fuellt.
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

        // Acryl nur, wenn ausdruecklich gewuenscht — und dann ohne
        // Schattenrand, sonst entsteht der Kasten erneut. Siehe
        // `Metrics::new`, wo der Rand in diesem Fall auf null geht.
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

/// Minutentimer auf die naechste volle Minute stellen.
fn arm_tick(hwnd: HWND) {
    let now = Local::now();
    let ms_to_next_minute = 60_000 - (now.second() * 1000 + now.timestamp_subsec_millis()) as i32;
    let elapse = ms_to_next_minute.clamp(1_000, 60_000) as u32;
    unsafe {
        SetTimer(Some(hwnd), TIMER_TICK, elapse, None);
    }
}

// --- Animationsantrieb ------------------------------------------------------

/// Startet den Animationstimer (falls noetig) und macht sofort einen Schritt,
/// damit die Reaktion nicht um bis zu 16 ms verzoegert wirkt.
fn kick(st: &mut State) {
    if !st.animating {
        st.animating = true;
        unsafe {
            SetTimer(Some(st.hwnd), TIMER_ANIM, ANIM_INTERVAL_MS, None);
        }
    }
    pump(st);
}

/// Ein Animationsschritt. Sobald nichts mehr in Bewegung ist, wird der Timer
/// abgeschaltet — ab da kostet das Widget wieder nichts.
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
                // Normalerweise immer ganz nach hinten. `SWP_NOZORDER` muss
                // dafuer weg, sonst ignoriert Windows `hwndInsertAfter`.
                // Waehrend eines Peeks gilt das Gegenteil, sonst faellt das
                // Fenster sofort wieder hinter alles zurueck.
                wp.hwndInsertAfter = if st.peeking {
                    HWND_TOPMOST
                } else {
                    HWND_BOTTOM
                };
                wp.flags &= !SWP_NOZORDER;
                // "Desktop anzeigen" (Win+D) versucht das Fenster zu
                // verstecken — wir bestehen darauf, sichtbar zu bleiben.
                wp.flags &= !SWP_HIDEWINDOW;
                LRESULT(0)
            }

            // Ein Gadget wird nie minimiert.
            WM_SYSCOMMAND if (wparam.0 & 0xFFF0) == SC_MINIMIZE as usize => LRESULT(0),

            // Nicht aktivieren lassen, auch wenn jemand es versucht.
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

            // Systemzeit oder Zeitzone geaendert (Reise, Sommerzeit): die
            // gesamte Agenda haengt an der lokalen Tagesgrenze und an
            // Relativzeiten, also neu holen.
            WM_TIMECHANGE => {
                log::info("System time changed — resyncing");
                st.last_minute = u32::MAX;
                request_sync(st);
                LRESULT(0)
            }

            // Windows faehrt herunter oder meldet ab: `WM_DESTROY` kommt dann
            // nicht mehr zuverlaessig, eine wartende Erledigung ginge verloren.
            WM_ENDSESSION => {
                commit_pending(st);
                LRESULT(0)
            }

            // Monitor abgezogen oder Aufloesung geaendert: Position pruefen.
            WM_DISPLAYCHANGE => {
                rescue_offscreen(st);
                LRESULT(0)
            }

            // Windows hat auf hell/dunkel umgeschaltet oder die Akzentfarbe
            // geaendert. `WM_SETTINGCHANGE` traegt zwar den Grund im lParam,
            // aber die Palette neu aufzuloesen ist so billig (ein
            // Registry-Wert plus ein DWM-Aufruf), dass sich das Auswerten des
            // Strings nicht lohnt.
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

            WM_APP_STATUS => {
                st.anim.spinning = matches!(sync::lock(&st.shared).status, Status::Syncing);
                kick(st);
                LRESULT(0)
            }

            // Aus dem Standby zurueck: sofort abgleichen statt bis zum
            // naechsten Intervall veraltete Daten zu zeigen.
            WM_POWERBROADCAST => {
                if wparam.0 as u32 == PBT_APMRESUMEAUTOMATIC
                    || wparam.0 as u32 == PBT_APMRESUMESUSPEND
                {
                    log::info("Aus dem Energiesparmodus zurueck — Sofortabgleich");
                    // Der Bildschirm kann sich waehrend des Schlafs geaendert
                    // haben (Dock ab-/angesteckt).
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

            // Ohne das setzt Windows den Klassenzeiger zurueck, sobald die
            // Maus sich bewegt, und der Groessen-Cursor flackert.
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
                    // Scrollbalken einblenden; er verblasst danach von selbst.
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
                // Eine noch nicht abgeschickte Erledigung darf nicht verloren
                // gehen, nur weil das Widget geschlossen wird.
                commit_pending(st);
                if let Some(s) = st.sync.as_ref() {
                    s.send(Command::Quit);
                }
                KillTimer(Some(hwnd), TIMER_TICK).ok();
                KillTimer(Some(hwnd), TIMER_ANIM).ok();
                KillTimer(Some(hwnd), TIMER_UNDO).ok();
                KillTimer(Some(hwnd), TIMER_PEEK).ok();
                let _ = UnregisterHotKey(Some(hwnd), HOTKEY_PEEK);
                log::info("Beendet");
                PostQuitMessage(0);
                LRESULT(0)
            }

            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

// --- Rueckgaengig -----------------------------------------------------------

/// Merkt das Abhaken vor und startet die Bedenkzeit.
fn begin_pending(st: &mut State, idx: usize) {
    // Nur eine Aufgabe gleichzeitig in der Warteschleife: eine zweite
    // Erledigung bestaetigt die erste sofort.
    commit_pending(st);

    let seconds = sync::lock(&st.shared).config.undo_seconds;
    let ids = {
        let mut guard = sync::lock(&st.shared);
        guard.agenda.tasks.get_mut(idx).map(|t| {
            // Sofort optisch quittieren, damit der Klick sich unmittelbar
            // anfuehlt.
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

/// Bedenkzeit abgelaufen? Dann absenden.
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

/// Schickt eine wartende Erledigung ab und beendet die Bedenkzeit.
fn commit_pending(st: &mut State) {
    let Some(p) = st.pending.take() else { return };
    stop_undo_timer(st);
    send_completion(st, &p.account_id, &p.tasklist_id, &p.task_id);
}

/// Nimmt die Erledigung zurueck — es wurde nie etwas an Google gesendet.
fn cancel_pending(st: &mut State) {
    let Some(p) = st.pending.take() else { return };
    stop_undo_timer(st);
    let mut guard = sync::lock(&st.shared);
    if let Some(t) = guard.agenda.tasks.iter_mut().find(|t| t.id == p.task_id) {
        t.completing = false;
    }
    drop(guard);
    log::info("Abhaken zurueckgenommen");
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

/// Schaltet einen Kalender bzw. eine Aufgabenliste an oder ab.
///
/// Eine leere Liste in der Konfiguration bedeutet "alle". Wird aus diesem
/// Zustand heraus eine Quelle abgewaehlt, muss die Auswahl erst ausgeschrieben
/// werden — sonst waere das Ergebnis wieder "alle". Umgekehrt wird eine
/// vollstaendige Auswahl zurueck auf "leer" normalisiert, damit ein spaeter
/// hinzugefuegter Kalender automatisch mitkommt.
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
            // Die letzte Quelle darf nicht verschwinden: eine leere Auswahl
            // hiesse wieder "alle", also genau das Gegenteil.
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
    st.config_mtime = config_mtime();
    request_sync(st);
}

/// Legt den Tagesplan als Text in die Zwischenablage.
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
        // Unteraufgaben eingerueckt, wie in der Anzeige.
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
        log::warn("Zwischenablage nicht verfuegbar");
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

/// Holt das Widget fuer ein paar Sekunden nach vorn.
///
/// Das ist der Ausgleich fuer die Bottom-Most-Lage: das Widget stoert nie,
/// ist dadurch aber beim Arbeiten auch nie zu sehen. Ein Tastendruck genuegt,
/// danach sinkt es von selbst zurueck — ohne Klick, ohne Fokuswechsel.
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
    // Aufblenden wie bei neuen Daten: der Blick soll gefuehrt werden.
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

    // Tageswechsel: die gesamte Agenda ist ungueltig geworden.
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

    // Uhr und Relativzeiten haengen an der Minute.
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
    // Vorlaeufig weitersetzen; `on_sync_done` korrigiert nach Ergebnis.
    let interval = sync::lock(&st.shared).config.sync_minutes as i64;
    st.next_sync_at = Local::now() + ChronoDuration::minutes(interval);
}

/// Nach jedem Sync-Ergebnis den naechsten Termin festlegen.
///
/// Bei Fehlern exponentiell zurueckrudern (1, 2, 4, 8 … Minuten), gedeckelt
/// auf das regulaere Intervall. So haemmert das Widget bei laengerer
/// Netzstoerung nicht dauernd gegen die API-Quote.
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
            // Ohne Benutzeraktion bringt ein Wiederholen nichts.
            st.next_sync_at = Local::now() + ChronoDuration::minutes(interval.max(15));
        }
        _ => {
            st.consecutive_failures = st.consecutive_failures.saturating_add(1);
            let backoff = 1i64 << st.consecutive_failures.min(5);
            st.next_sync_at = Local::now() + ChronoDuration::minutes(backoff.min(interval));
        }
    }

    // Nur bei tatsaechlich neuen Daten einblenden — ein fehlgeschlagener
    // Versuch soll die Liste nicht grundlos aufblitzen lassen.
    if stamp != st.last_stamp {
        st.last_stamp = stamp;
        st.anim.restart_reveal();
        // Neue Liste, alte Scrollposition kann ins Leere zeigen.
        st.scroll_target = 0.0;
        st.anim.scroll.set(0.0);
    }
    st.anim.spinning = matches!(status, Status::Syncing);
    kick(st);
}

/// Uebernimmt Aenderungen an `config.json` ohne Neustart.
fn reload_config_if_changed(st: &mut State) {
    let current = config_mtime();
    if current == st.config_mtime {
        return;
    }
    st.config_mtime = current;

    let (cfg, error) = Config::load();
    if let Some(e) = &error {
        log::warn(e);
    } else {
        log::info("Konfiguration neu geladen");
    }

    let scale_changed = (cfg.scale - st.scale).abs() > f32::EPSILON;
    let (px, py, pw, ph) = target_geometry(&cfg, metrics_for(&cfg), st.dpi);
    let palette = Palette::resolve(ThemePref::parse(&cfg.theme), &cfg.accent, st.visuals);
    st.anim.enabled = palette.animations;

    // Die Leserichtung steckt in den DirectWrite-Formaten; ein Wechsel
    // zwischen LTR und RTL erzwingt daher denselben Neuaufbau wie eine
    // geaenderte Skalierung.
    let new_loc = Locale::resolve(&cfg.language);
    let direction_changed = new_loc.rtl != st.loc.rtl;
    crate::i18n::set_global(new_loc.cat);
    st.loc = new_loc;

    st.scale = cfg.scale;
    st.metrics = metrics_for(&cfg);
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

    // Die Schriftgroessen stecken in den DirectWrite-Formaten und lassen sich
    // nicht nachtraeglich aendern — bei geaenderter Skalierung muss der
    // Renderer komplett neu aufgebaut werden.
    if scale_changed || direction_changed {
        recreate_renderer(st, pw.max(1) as u32, ph.max(1) as u32, palette);
    } else if let Some(r) = st.renderer.as_mut() {
        r.set_palette(palette);
    }
    redraw(st);
}

/// Systemdesign, Akzentfarbe, Kontrastmodus oder Bewegungseinstellung haben
/// sich geaendert.
///
/// `WM_SETTINGCHANGE` kommt aus vielerlei Anlaessen; ohne den Vergleich mit
/// dem letzten Stand wuerde jedes fremde Systemereignis ein Neuzeichnen
/// ausloesen.
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
    st.visuals = visuals;

    let cfg = sync::lock(&st.shared).config.clone();
    let palette = Palette::resolve(ThemePref::parse(&cfg.theme), &cfg.accent, st.visuals);
    st.anim.enabled = palette.animations;
    apply_backdrop(st.hwnd, &cfg, palette.dark);
    if let Some(r) = st.renderer.as_mut() {
        r.set_palette(palette);
    }
    redraw(st);
}

fn config_mtime() -> Option<SystemTime> {
    std::fs::metadata(config::config_path())
        .ok()
        .and_then(|m| m.modified().ok())
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

    // Kante hat Vorrang vor der Zeilen-Hervorhebung: sonst konkurriert der
    // Griff mit dem Hover der darunterliegenden Zeile.
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
        // Beim Wechsel neu aufblenden statt hart umzuspringen.
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

        // Liegt oben auf der Zeile, solange die Bedenkzeit laeuft.
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
            if has_config_error {
                platform::open_path(&config::config_path());
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
                    // Bei einem Sync-Fehler steht der volle Text im Protokoll.
                    Status::Error(_) => platform::open_path(&log::file_path()),
                    _ => request_sync(st),
                }
            }
        }

        // Leere Flaeche: Fenster verschieben.
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

/// Oeffnet einen Termin im Google-Kalender.
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

/// Welche Kante liegt unter dem Mauszeiger? Koordinaten in DIPs.
fn edge_at(st: &State, x: f32, y: f32) -> Edges {
    let Some(r) = st.renderer.as_ref() else {
        return Edges::NONE;
    };
    let (w, h) = r.size_dip();
    // Der Glaskoerper ist um den Schattenrand eingerueckt; der Griff sitzt an
    // *seiner* Kante, nicht an der unsichtbaren Fensterkante.
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

/// Zieht das Fenster an der gefassten Kante auf die neue Groesse.
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
            // Beim Ziehen an der linken Kante wandert der Ursprung mit; die
            // Mindestbreite muss deshalb den *linken* Rand begrenzen.
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

/// Neue Groesse dauerhaft merken — in DIPs und ohne den Schattenrand.
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
    st.config_mtime = config_mtime();
}

/// Trefferpruefung von hinten nach vorn: zuletzt gezeichnete (obenliegende)
/// Regionen gewinnen — so schlaegt der Abhaken-Kreis die Aufgabenzeile und
/// die Rueckgaengig-Flaeche beide.
fn hit_at(st: &State, x: f32, y: f32) -> Option<Hit> {
    st.hits
        .iter()
        .rev()
        .find(|r| r.contains(x, y))
        .map(|r| r.hit)
}

/// Mausposition aus `LPARAM` (physische Pixel) in DIPs umrechnen.
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
    // Eigenes Speichern nicht als Fremdaenderung missdeuten.
    st.config_mtime = config_mtime();
}

/// Holt das Fenster zurueck, wenn seine Position auf keinem Monitor mehr liegt.
fn rescue_offscreen(st: &mut State) {
    unsafe {
        let mut wr = RECT::default();
        if GetWindowRect(st.hwnd, &mut wr).is_err() || platform::is_on_screen(&wr) {
            return;
        }
        log::warn("Fensterposition liegt auf keinem Monitor — zurueckgesetzt");
        let cfg = {
            let mut guard = sync::lock(&st.shared);
            guard.config.x = None;
            guard.config.y = None;
            guard.config.save();
            guard.config.clone()
        };
        st.config_mtime = config_mtime();

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
    // Den Arc kopieren, nicht den Inhalt: waehrend einer Animation laeuft das
    // hier 60-mal pro Sekunde, und ein voller Klon der Agenda waere dabei
    // tausende Allokationen pro Sekunde fuer Daten, die sich nicht aendern.
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
        };
        renderer.draw(&frame, hits)
    };
    drop(guard);

    match draw_result {
        Ok(result) => {
            st.content_height = result.content_height;
            st.viewport_height = result.viewport_height;
            // Nach dem Loeschen von Zeilen kann der Scroll-Offset ins Leere
            // zeigen — dann zurueckziehen.
            let overflow = (st.content_height - st.viewport_height).max(0.0);
            if st.scroll_target > overflow {
                st.scroll_target = overflow;
                st.anim.scroll.set(overflow);
            }
        }
        Err(e) if render::is_device_lost(e.code()) => {
            // Treiberwechsel, GPU-Reset oder Wechsel in eine RDP-Sitzung: die
            // komplette Geraetekette ist ungueltig. Neu aufbauen und beim
            // naechsten Anlass wieder zeichnen.
            log::warn(&format!("Grafikgeraet verloren ({e}) — Neuaufbau"));
            let (w, h) = current_size_px(st.hwnd);
            let pal = st.renderer.as_ref().map(|r| r.palette());
            let cfg = sync::lock(&st.shared).config.clone();
            let pal = pal.unwrap_or_else(|| {
                Palette::resolve(ThemePref::parse(&cfg.theme), &cfg.accent, st.visuals)
            });
            recreate_renderer(st, w, h, pal);
        }
        Err(_) => {}
    }
}

fn recreate_renderer(st: &mut State, width_px: u32, height_px: u32, pal: Palette) {
    // Erst freigeben: Swapchain und Composition-Target halten sonst noch
    // Referenzen auf das verlorene Geraet.
    st.renderer = None;
    st.renderer = Renderer::new(
        st.hwnd,
        width_px.max(1),
        height_px.max(1),
        st.dpi,
        st.scale,
        pal,
        st.loc.rtl,
    )
    .ok();
    if st.renderer.is_none() {
        log::error("Renderer konnte nicht neu aufgebaut werden");
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

        // Die Beschriftungen kommen aus dem Katalog und muessen deshalb zur
        // Laufzeit nach UTF-16 gewandelt werden; `w!()` kann nur Literale.
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
        // Quellen direkt im Menue an- und abwaehlen. Die IDs sind lange
        // E-Mail-aehnliche Zeichenketten; sie von Hand in die JSON zu
        // uebertragen war die unangenehmste Stelle der Einrichtung.
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
                // Leere Auswahl bedeutet "alle" — dann sind alle angehakt.
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

        // Ohne Vordergrundfenster bliebe das Menue nach einem Klick daneben
        // offen stehen; die WM_NULL-Nachricht danach ist der dokumentierte
        // Begleit-Workaround.
        let _ = SetForegroundWindow(st.hwnd);
        // Bei rechts-nach-links-Leserichtung klappt das Menue an der rechten
        // Kante des Mauszeigers auf, wie es das System auch tut.
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
                // Sicherstellen, dass die Datei existiert, bevor sie geoeffnet
                // wird — der Editor soll nicht "nicht gefunden" melden.
                sync::lock(&st.shared).config.save();
                st.config_mtime = config_mtime();
                platform::open_path(&config::config_path());
            }
            CMD_RESET_POS => {
                {
                    let mut guard = sync::lock(&st.shared);
                    guard.config.x = None;
                    guard.config.y = None;
                    guard.config.save();
                }
                st.config_mtime = config_mtime();
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
