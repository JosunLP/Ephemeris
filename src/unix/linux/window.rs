// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Ephemeris contributors
//! The widget's window on X11.
//!
//! Every property of the Windows window has a standard here, which is why X11
//! came before Wayland:
//!
//! | | |
//! |---|---|
//! | below every window, above the desktop | `_NET_WM_STATE_BELOW` |
//! | no taskbar or window-switcher entry | `_NET_WM_STATE_SKIP_TASKBAR`, `_SKIP_PAGER` |
//! | on every virtual desktop | `_NET_WM_STATE_STICKY` |
//! | never takes focus | `WM_HINTS.input = False` and no `WM_TAKE_FOCUS` |
//! | no frame | `_MOTIF_WM_HINTS` with the decorations bit cleared |
//! | per-pixel alpha | a 32-bit `TrueColor` visual and its own colourmap |
//!
//! `_NET_WM_WINDOW_TYPE_DESKTOP` was the other candidate for the first row and
//! is wrong: it means *is* the desktop, and window managers that take it
//! seriously stack the widget beneath the icons and stop sending it clicks.
//! `BELOW` on an ordinary window says what is actually wanted — behind
//! everything else, still a window.
//!
//! **Under Wayland** this runs through XWayland. Whether `BELOW` is honoured
//! is then the compositor's business and several ignore it, so the widget says
//! what it cannot do rather than pretending — see [`super::report_session`].
//!
//! **The loop is a `poll`**, because X11 has no timers of its own. It waits on
//! the display's socket and on the pipe the sync thread writes to, with a
//! timeout taken from whichever of the widget's three timers is due first.
//! That is what keeps a widget at rest costing nothing: with no animation
//! running and no undo pending, the next wake-up is the top of the minute.

// The X11 event and mask names are C constants and keep their spelling: a
// reader with `X.h` open has to be able to find them.
#![allow(non_upper_case_globals)]

use crate::paint::widget as paint;
use crate::unix::app::{self, App, Cursor as AppCursor, Shell, WindowRect};
use crate::unix::autostart;
use crate::unix::linux::canvas::{Cairo, Look};
use crate::unix::linux::ffi::*;
use crate::unix::linux::{menu, visuals};
use ephemeris_core::host::Waker;
use ephemeris_core::log;
use ephemeris_core::theme::SystemVisuals;
use std::ffi::{CString, c_int, c_long, c_uint, c_ulong, c_void};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The atoms the widget needs, interned once.
///
/// Gathered in a struct rather than looked up where they are used: each
/// lookup is a round trip to the server, and several of them happen inside the
/// event loop.
#[allow(non_snake_case)]
pub struct Atoms {
    pub wm_protocols: Atom,
    pub wm_delete_window: Atom,
    pub wm_hints: Atom,
    pub net_wm_state: Atom,
    pub net_wm_state_below: Atom,
    pub net_wm_state_above: Atom,
    pub net_wm_state_skip_taskbar: Atom,
    pub net_wm_state_skip_pager: Atom,
    pub net_wm_state_sticky: Atom,
    pub net_wm_window_type: Atom,
    pub net_wm_window_type_normal: Atom,
    pub net_wm_desktop: Atom,
    pub net_wm_name: Atom,
    pub motif_wm_hints: Atom,
    pub clipboard: Atom,
    pub utf8_string: Atom,
    pub targets: Atom,
}

impl Atoms {
    fn new(libs: &Libs, display: *mut Display) -> Self {
        let intern = |name: &str| -> Atom {
            let Ok(c) = CString::new(name) else { return 0 };
            unsafe { (libs.XInternAtom)(display, c.as_ptr(), 0) }
        };
        Self {
            wm_protocols: intern("WM_PROTOCOLS"),
            wm_delete_window: intern("WM_DELETE_WINDOW"),
            wm_hints: intern("WM_HINTS"),
            net_wm_state: intern("_NET_WM_STATE"),
            net_wm_state_below: intern("_NET_WM_STATE_BELOW"),
            net_wm_state_above: intern("_NET_WM_STATE_ABOVE"),
            net_wm_state_skip_taskbar: intern("_NET_WM_STATE_SKIP_TASKBAR"),
            net_wm_state_skip_pager: intern("_NET_WM_STATE_SKIP_PAGER"),
            net_wm_state_sticky: intern("_NET_WM_STATE_STICKY"),
            net_wm_window_type: intern("_NET_WM_WINDOW_TYPE"),
            net_wm_window_type_normal: intern("_NET_WM_WINDOW_TYPE_NORMAL"),
            net_wm_desktop: intern("_NET_WM_DESKTOP"),
            net_wm_name: intern("_NET_WM_NAME"),
            motif_wm_hints: intern("_MOTIF_WM_HINTS"),
            clipboard: intern("CLIPBOARD"),
            utf8_string: intern("UTF8_STRING"),
            targets: intern("TARGETS"),
        }
    }
}

/// Wakes the loop by writing a byte to a pipe the `poll` is watching.
///
/// Not `XSendEvent`: an Xlib display connection is not safe to use from two
/// threads unless `XInitThreads` was called before anything else, and a pipe
/// needs no such promise from a library that is loaded at run time.
struct PipeWaker(c_int);

impl Waker for PipeWaker {
    fn wake(&self) {
        // One byte, and a full pipe means a wake is already pending — which is
        // exactly as good as another one.
        let byte = 1u8;
        unsafe {
            libc::write(self.0, &byte as *const u8 as *const c_void, 1);
        }
    }
}

pub struct X11Shell {
    pub libs: Arc<Libs>,
    pub display: *mut Display,
    pub window: Window,
    pub visual: VisualPtr,
    /// The visual's depth, so the menu's own windows match it. Mixing depths
    /// between a window and its colourmap is a `BadMatch`.
    pub depth: c_int,
    pub colormap: Colormap,
    pub atoms: Atoms,
    pub look: Look,
    root: Window,
    screen: c_int,
    cursors: [Cursor; 5],
    current_cursor: usize,
    animating: bool,
    undo_running: bool,
    next_minute: Instant,
    peeking: bool,
    quit: bool,
    /// What the clipboard would hand out if somebody asked for it.
    clipboard: String,
    hotkey: Option<(c_uint, c_uint)>,
    pub rebuild: bool,
}

impl X11Shell {
    pub fn root_window(&self) -> Window {
        self.root
    }

    /// The whole screen, in pixels. Used to keep a menu on it.
    pub fn screen_size(&self) -> (c_int, c_int) {
        unsafe {
            (
                (self.libs.XDisplayWidth)(self.display, self.screen),
                (self.libs.XDisplayHeight)(self.display, self.screen),
            )
        }
    }

    /// The next moment the loop has to be awake, or `None` to wait for an
    /// event.
    fn next_deadline(&self, peek: Option<Instant>) -> Option<Instant> {
        [
            self.animating.then(|| Instant::now() + app::ANIM_INTERVAL),
            self.undo_running
                .then(|| Instant::now() + app::UNDO_INTERVAL),
            Some(self.next_minute),
            peek,
        ]
        .into_iter()
        .flatten()
        .min()
    }
}

/// Builds the window and runs the event loop.
pub fn run() -> Result<(), String> {
    let libs = Arc::new(Libs::load().ok_or("the desktop's libraries could not be loaded")?);

    // Xlib's default error handler calls `exit`. A widget that vanishes
    // because a window manager sent something unexpected is exactly the
    // failure mode the panic hook exists to prevent, so the handler is
    // replaced by one that ignores.
    unsafe { (libs.XSetErrorHandler)(ignore_x_error as *const c_void) };

    let display = unsafe { (libs.XOpenDisplay)(std::ptr::null()) };
    if display.is_null() {
        return Err("could not open the X display".into());
    }
    let screen = unsafe { (libs.XDefaultScreen)(display) };
    let root = unsafe { (libs.XRootWindow)(display, screen) };
    let atoms = Atoms::new(&libs, display);

    // A 32-bit TrueColor visual is what gives the panel its per-pixel alpha.
    // Not every display has one — a bare X server with no compositing, a
    // virtual framebuffer — and the widget works there too: the panel comes
    // out opaque, which is the same result the "reduce transparency"
    // accessibility setting already produces and which the palette has a
    // `force_opaque` path for. Refusing to start would be far worse than
    // being opaque.
    let mut vi = XVisualInfo::default();
    let (visual, depth) = if unsafe {
        (libs.XMatchVisualInfo)(display, screen, 32, TrueColor, &mut vi)
    } != 0
    {
        (vi.visual, 32)
    } else {
        log::warn(
            "No 32-bit visual on this display — the panel will be opaque rather than translucent.",
        );
        unsafe {
            (
                (libs.XDefaultVisual)(display, screen),
                (libs.XDefaultDepth)(display, screen),
            )
        }
    };

    let colormap = unsafe { (libs.XCreateColormap)(display, root, visual, AllocNone) };
    let mut attributes = XSetWindowAttributes {
        background_pixel: 0,
        border_pixel: 0,
        colormap,
        event_mask: ExposureMask
            | ButtonPressMask
            | ButtonReleaseMask
            | PointerMotionMask
            | LeaveWindowMask
            | StructureNotifyMask,
        ..XSetWindowAttributes::default()
    };

    // Before the widget: the sync thread it starts is handed this waker, so
    // the pipe has to exist first.
    let (wake_read, wake_write) = pipe().ok_or("could not create the wake pipe")?;
    let (mut widget, cfg) = App::new(visuals::read(), Arc::new(PipeWaker(wake_write)));

    let window = unsafe {
        (libs.XCreateWindow)(
            display,
            root,
            0,
            0,
            400,
            640,
            0,
            depth,
            InputOutput,
            visual,
            CWBackPixel | CWBorderPixel | CWColormap | CWEventMask,
            &mut attributes,
        )
    };
    if window == 0 {
        return Err("the window could not be created".into());
    }

    set_window_properties(&libs, display, window, &atoms);

    let cursors = [
        XC_left_ptr,
        XC_top_left_corner,
        XC_top_right_corner,
        XC_sb_h_double_arrow,
        XC_sb_v_double_arrow,
    ]
    .map(|shape| unsafe { (libs.XCreateFontCursor)(display, shape) });

    let mut shell = X11Shell {
        libs: libs.clone(),
        display,
        window,
        visual,
        depth,
        colormap,
        atoms,
        look: Look {
            palette: widget.palette,
            metrics: widget.metrics,
            appearance: widget.appearance.clone(),
            rtl: widget.loc.rtl,
        },
        root,
        screen,
        cursors,
        current_cursor: 0,
        animating: false,
        undo_running: false,
        next_minute: next_minute_boundary(),
        peeking: false,
        quit: false,
        clipboard: String::new(),
        hotkey: None,
        rebuild: false,
    };

    let rect = widget.target_geometry(&cfg, &shell);
    shell.set_window_rect(rect);
    shell.grab_hotkey(&cfg.peek_hotkey);
    unsafe {
        (libs.XMapWindow)(display, window);
        (libs.XLowerWindow)(display, window);
        (libs.XFlush)(display);
    }
    widget.rescue_offscreen(&mut shell);
    super::report_session();
    visuals::watch(wake_write);

    let mut canvas = Cairo::for_window(
        libs.clone(),
        display,
        window,
        visual,
        (rect.width, rect.height),
        &shell.look,
    )
    .ok_or("the Cairo surface could not be created")?;

    event_loop(&mut widget, &mut shell, &mut canvas, wake_read);

    widget.commit_pending_on_exit();
    widget.shut_down();
    unsafe {
        (libs.XDestroyWindow)(display, window);
        (libs.XCloseDisplay)(display);
    }
    Ok(())
}

/// The loop. Everything else in this file is what it calls.
fn event_loop(app: &mut App, shell: &mut X11Shell, canvas: &mut Cairo, wake: c_int) {
    let x_fd = unsafe { (shell.libs.XConnectionNumber)(shell.display) };
    // The socket another copy writes `--peek` to. It is the single-instance
    // lock as well, so there is always one unless the name was unusable.
    let control_fd = super::control_fd().unwrap_or(-1);
    let mut dirty = true;

    loop {
        if shell.quit {
            return;
        }
        // Draw before waiting, so a redraw asked for by an event that has
        // already been handled does not sit until the next one arrives.
        if dirty {
            dirty = false;
            draw(app, shell, canvas);
        }

        let deadline = shell.next_deadline(app.peek_deadline());
        let timeout = deadline
            .map(|d| d.saturating_duration_since(Instant::now()))
            .unwrap_or(Duration::from_secs(60));
        // Only sleep when the server has nothing queued: Xlib buffers events
        // internally, and `poll` on the socket cannot see those.
        if unsafe { (shell.libs.XPending)(shell.display) } == 0 {
            wait(&[x_fd, wake, control_fd], timeout);
        }

        if super::take_control_requests() {
            app.begin_peek(shell);
            dirty = true;
        }

        // Anything written to the pipe, drained in one go: several finished
        // commands mean one refresh, not one each. The appearance watcher
        // writes to the same pipe and raises its own flag first, so the two
        // are told apart without a second descriptor.
        if drain(wake) {
            if visuals::appearance_changed() {
                app.refresh_palette(shell);
                shell.rebuild = true;
            }
            app.on_sync_done(shell);
            dirty = true;
        }

        while unsafe { (shell.libs.XPending)(shell.display) } > 0 {
            let mut event = XEvent::default();
            unsafe { (shell.libs.XNextEvent)(shell.display, &mut event) };
            if handle(app, shell, canvas, &event) {
                dirty = true;
            }
            if shell.quit {
                return;
            }
        }

        let now = Instant::now();
        if shell.animating {
            app.pump(shell);
            dirty = true;
        }
        if shell.undo_running {
            app.on_undo_tick(shell);
            dirty = true;
        }
        if now >= shell.next_minute {
            shell.next_minute = next_minute_boundary();
            app.on_minute(shell);
            dirty = true;
        }
        if app.peek_deadline().is_some_and(|d| now >= d) {
            app.end_peek(shell);
            dirty = true;
        }
    }
}

/// Handles one X event; true if the widget has to be drawn again.
fn handle(app: &mut App, shell: &mut X11Shell, canvas: &mut Cairo, event: &XEvent) -> bool {
    match event.kind() {
        Expose => true,

        ConfigureNotify => {
            let e: &XConfigureEvent = unsafe { event.as_ref() };
            canvas.resize(e.width, e.height);
            true
        }

        ButtonPress => {
            let e: &XButtonEvent = unsafe { event.as_ref() };
            let point = (e.x as f32, e.y as f32);
            let root = (e.x_root, e.y_root);
            match e.button {
                1 => app.on_press(shell, point.0, point.1, root),
                // Buttons four and five are the wheel; X11 reports them as
                // clicks, one per notch.
                4 => app.on_scroll(shell, 1.0),
                5 => app.on_scroll(shell, -1.0),
                3 => app.show_menu(shell),
                _ => {}
            }
            true
        }

        ButtonRelease => {
            let e: &XButtonEvent = unsafe { event.as_ref() };
            if e.button == 1 {
                app.on_release(shell);
            }
            true
        }

        MotionNotify => {
            let e: &XMotionEvent = unsafe { event.as_ref() };
            app.on_pointer_move(shell, e.x as f32, e.y as f32, (e.x_root, e.y_root));
            true
        }

        LeaveNotify => {
            app.on_pointer_leave(shell);
            true
        }

        KeyPress => {
            // The only key that reaches here is the grabbed one: nothing else
            // is selected for, and the widget never has the input focus.
            app.begin_peek(shell);
            true
        }

        SelectionRequest => {
            let e: &XSelectionRequestEvent = unsafe { event.as_ref() };
            shell.answer_selection(e);
            false
        }

        // `WM_DELETE_WINDOW` is advertised in `WM_PROTOCOLS`, so it has to be
        // honoured. The widget has no close button — it has no frame at all —
        // but `wmctrl -c`, a session ending and a compositor tidying up all
        // send this, and ignoring a protocol the window claims to speak is how
        // a widget survives a logout it should not have.
        ClientMessage => {
            let e: &XClientMessageEvent = unsafe { event.as_ref() };
            if e.message_type == shell.atoms.wm_protocols
                && e.data[0] as Atom == shell.atoms.wm_delete_window
            {
                shell.quit();
            }
            false
        }

        _ => false,
    }
}

fn draw(app: &mut App, shell: &mut X11Shell, canvas: &mut Cairo) {
    shell.look = Look {
        palette: app.palette,
        metrics: app.metrics,
        appearance: app.appearance.clone(),
        rtl: app.loc.rtl,
    };
    if std::mem::take(&mut shell.rebuild) || std::mem::take(&mut app.needs_rebuild) {
        let rect = shell.window_rect();
        if let Some(fresh) = Cairo::for_window(
            shell.libs.clone(),
            shell.display,
            shell.window,
            shell.visual,
            (rect.width, rect.height),
            &shell.look,
        ) {
            *canvas = fresh;
        }
    }
    // Before the context, not after: `cairo_xlib_surface_set_size` is
    // documented as something to do between drawing operations, and a `cairo_t`
    // created against the old size has already sampled it.
    let rect = shell.window_rect();
    canvas.resize(rect.width, rect.height);
    if !canvas.begin() {
        return;
    }
    let size = (rect.width as f32, rect.height as f32);
    let (metrics, palette, rtl) = (app.metrics, app.palette, app.loc.rtl);
    let appearance = app.appearance.clone();
    let result = app.with_frame(|frame, hits| {
        paint::draw(
            canvas,
            size,
            metrics,
            palette,
            &appearance,
            rtl,
            frame,
            hits,
        )
    });
    app.frame_drawn(result.content_height, result.viewport_height);
    unsafe { (shell.libs.XFlush)(shell.display) };
}

// --- Window properties ------------------------------------------------------

/// Everything that has to be set before the window is mapped.
///
/// Order matters: a window manager reads these once when the window appears,
/// and several of them — the states in particular — are only honoured as
/// initial properties. Changing them afterwards needs a client message to the
/// root instead, which is what [`X11Shell::set_peek`] does.
fn set_window_properties(libs: &Libs, display: *mut Display, window: Window, atoms: &Atoms) {
    let change = |property: Atom, kind: Atom, format: c_int, data: &[u8], count: usize| unsafe {
        (libs.XChangeProperty)(
            display,
            window,
            property,
            kind,
            format,
            PropModeReplace,
            data.as_ptr(),
            count as c_int,
        );
    };

    // `XA_WM_NAME` is 39; 31 is `XA_STRING`, which is the *type* here. Both
    // are written: the old property for anything that still reads it, and
    // `_NET_WM_NAME` as UTF-8, which is what every current window manager,
    // taskbar and screen reader looks at first.
    let name = b"Ephemeris";
    change(XA_WM_NAME, XA_STRING, 8, name, name.len());
    change(atoms.net_wm_name, atoms.utf8_string, 8, name, name.len());

    let kind = [atoms.net_wm_window_type_normal];
    change(
        atoms.net_wm_window_type,
        XA_ATOM,
        32,
        as_bytes(&kind),
        kind.len(),
    );

    let states = [
        atoms.net_wm_state_below,
        atoms.net_wm_state_skip_taskbar,
        atoms.net_wm_state_skip_pager,
        atoms.net_wm_state_sticky,
    ];
    change(
        atoms.net_wm_state,
        XA_ATOM,
        32,
        as_bytes(&states),
        states.len(),
    );

    // `0xFFFFFFFF` means "on every desktop", which `STICKY` asks for and this
    // states outright for the window managers that read only one of the two.
    let all_desktops: [c_ulong; 1] = [0xFFFF_FFFF];
    change(
        atoms.net_wm_desktop,
        XA_CARDINAL,
        32,
        as_bytes(&all_desktops),
        1,
    );

    // Motif hints: flags = MWM_HINTS_DECORATIONS, decorations = none. Older
    // than the EWMH and still the only way to say "no frame" that every window
    // manager understands.
    let motif: [c_ulong; 5] = [2, 0, 0, 0, 0];
    change(
        atoms.motif_wm_hints,
        atoms.motif_wm_hints,
        32,
        as_bytes(&motif),
        5,
    );

    // `WM_HINTS` with `input = False` and no `WM_TAKE_FOCUS` protocol: the
    // widget is telling the window manager it does not want the keyboard, ever.
    // That is what stops a click on it pulling focus out of whatever you are
    // typing in — the X11 equivalent of `WS_EX_NOACTIVATE`.
    //
    // The layout is `flags, input, initial_state, icon_pixmap, icon_window,
    // icon_x, icon_y, icon_mask, window_group`; `flags = InputHint` alone.
    let hints: [c_long; 9] = [1, 0, 0, 0, 0, 0, 0, 0, 0];
    change(atoms.wm_hints, atoms.wm_hints, 32, as_bytes(&hints), 9);

    let protocols = [atoms.wm_delete_window];
    change(
        atoms.wm_protocols,
        XA_ATOM,
        32,
        as_bytes(&protocols),
        protocols.len(),
    );
}

/// A slice of 32- or 64-bit values as the bytes `XChangeProperty` wants.
///
/// Xlib's contract for `format = 32` is a `long` array, whatever a `long` is
/// on the machine — 64 bits on every platform this builds for. That is why the
/// arrays above are `c_ulong` and `c_long` rather than `u32`.
fn as_bytes<T>(values: &[T]) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts(values.as_ptr() as *const u8, std::mem::size_of_val(values))
    }
}

/// The lock states a key grab has to be repeated for.
///
/// X11 counts Caps Lock and Num Lock as part of the modifier state, so a grab
/// for the plain combination stops matching the moment either is on. The same
/// four go into the ungrab, which is why they are named here rather than
/// written out twice.
const LOCK_VARIANTS: [c_uint; 4] = [0, LockMask, Mod2Mask, LockMask | Mod2Mask];

/// The code of the last error Xlib reported, for the one caller that needs it.
///
/// A global because Xlib's error handler is global: it is a plain C function
/// pointer with nowhere to hang a context. [`X11Shell::grab_hotkey`] clears
/// this, makes its request, waits for the server and reads it back.
static LAST_X_ERROR: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// Xlib's default handler exits the process. Not taking the widget down is
/// right here: every request this program makes is either advisory (a hint the
/// window manager may not know) or already checked, and an error on one of them
/// is not a reason to vanish.
///
/// The code is kept rather than dropped, because some requests report failure
/// only this way — `XGrabKey` has no return value, and a combination another
/// program holds arrives here as `BadAccess` some time after the call.
extern "C" fn ignore_x_error(_display: *mut Display, event: *mut c_void) -> c_int {
    if !event.is_null() {
        let code = unsafe { (*(event as *const XErrorEvent)).error_code };
        LAST_X_ERROR.store(code, std::sync::atomic::Ordering::Relaxed);
    }
    0
}

// --- The shell --------------------------------------------------------------

impl X11Shell {
    /// Asks the window manager to add or remove a state.
    ///
    /// A client message to the root, which is how the EWMH says a mapped
    /// window changes its own state — writing the property directly is only
    /// honoured before the window is mapped.
    fn set_state(&self, add: bool, first: Atom, second: Atom) {
        // `_NET_WM_STATE` client message: action, two atoms, source (1 = an
        // ordinary application).
        let mut event = XEvent::default();
        {
            let e = unsafe { &mut *(&mut event as *mut XEvent as *mut XClientMessageEvent) };
            e.type_ = ClientMessage;
            e.serial = 0;
            e.send_event = 1;
            e.display = self.display;
            e.window = self.window;
            e.message_type = self.atoms.net_wm_state;
            e.format = 32;
            e.data = [
                if add { 1 } else { 0 },
                first as c_long,
                second as c_long,
                1,
                0,
            ];
        }
        unsafe {
            (self.libs.XSendEvent)(
                self.display,
                self.root,
                0,
                SubstructureNotifyMask | SubstructureRedirectMask,
                &mut event,
            );
        }
    }

    /// Registers the global peek shortcut on the root window.
    ///
    /// The grab is repeated for the lock and numeric-lock combinations,
    /// because X11 treats those as part of the modifier state: without it the
    /// shortcut stops working the moment Caps Lock is on.
    fn grab_hotkey(&mut self, configured: &str) {
        if configured.trim().is_empty() {
            return;
        }
        for spec in ephemeris_core::hotkey::candidates(configured) {
            let Some(combo) = ephemeris_core::hotkey::parse(spec) else {
                log::warn(&format!("peek_hotkey '{spec}' is not a usable combination"));
                continue;
            };
            let Some(keycode) = self.keycode(combo.key) else {
                log::warn(&format!("peek_hotkey '{spec}' has no key on this keyboard"));
                continue;
            };
            let mut mask = 0;
            if combo.ctrl {
                mask |= ControlMask;
            }
            if combo.alt {
                mask |= Mod1Mask;
            }
            if combo.shift {
                mask |= ShiftMask;
            }
            if combo.meta {
                mask |= Mod4Mask;
            }

            // `XGrabKey` returns nothing worth reading: the request is
            // asynchronous, and a combination another program already holds
            // comes back afterwards as a `BadAccess` error to the handler. So
            // each variant is waited for on its own and the handler's record
            // read back — otherwise the widget would announce a shortcut it
            // does not have, and the fallback candidates would never be tried.
            let mut granted: Vec<c_uint> = Vec::new();
            for extra in LOCK_VARIANTS {
                LAST_X_ERROR.store(0, std::sync::atomic::Ordering::Relaxed);
                unsafe {
                    (self.libs.XGrabKey)(
                        self.display,
                        keycode as c_int,
                        mask | extra,
                        self.root,
                        0,
                        GrabModeAsync,
                        GrabModeAsync,
                    );
                    (self.libs.XSync)(self.display, 0);
                }
                if LAST_X_ERROR.swap(0, std::sync::atomic::Ordering::Relaxed) == BadAccess {
                    break;
                }
                granted.push(mask | extra);
            }
            // All of them or none: a shortcut registered for the plain
            // modifiers but not for Caps Lock is one that stops working halfway
            // through a session, and the variants that did succeed would keep
            // the next candidate from being granted.
            if granted.len() < LOCK_VARIANTS.len() {
                for m in granted {
                    unsafe { (self.libs.XUngrabKey)(self.display, keycode as c_int, m, self.root) };
                }
                unsafe { (self.libs.XSync)(self.display, 0) };
                log::warn(&format!(
                    "peek_hotkey '{spec}' is held by another program on this display"
                ));
                continue;
            }
            self.hotkey = Some((keycode, mask));
            if spec == configured.trim() {
                log::info(&format!("Peek hotkey: {spec}"));
            } else {
                log::warn(&format!(
                    "peek_hotkey '{configured}' could not be grabbed; using {spec} instead"
                ));
            }
            return;
        }
        log::warn("No peek hotkey could be registered; set peek_hotkey in config.json");
    }

    /// The keycode for a key, through its X keysym name.
    ///
    /// `XStringToKeysym` takes the names from `keysymdef.h`: `"k"`, `"7"`,
    /// `"F12"`. Going through the name rather than a table means the widget
    /// follows the user's keyboard layout, which is what a shortcut written as
    /// a letter should do.
    fn keycode(&self, key: ephemeris_core::hotkey::Key) -> Option<c_uint> {
        use ephemeris_core::hotkey::Key;
        let name = match key {
            Key::Letter(c) => (c as char).to_ascii_lowercase().to_string(),
            Key::Digit(d) => d.to_string(),
            Key::Function(n) => format!("F{n}"),
        };
        let name = CString::new(name).ok()?;
        let keysym = unsafe { (self.libs.XStringToKeysym)(name.as_ptr()) };
        if keysym == NoSymbol {
            return None;
        }
        let code = unsafe { (self.libs.XKeysymToKeycode)(self.display, keysym) };
        (code != 0).then_some(code)
    }

    /// Hands the clipboard's contents to whoever asked for them.
    ///
    /// X11 has no clipboard: it has an owner who answers questions. This is
    /// the whole of the answer — the list of formats offered, and the text
    /// itself as UTF-8.
    pub fn answer_selection(&self, request: &XSelectionRequestEvent) {
        let mut property = request.property;
        if property == 0 {
            // An obsolete client asking without naming a property; the
            // convention is to use the target atom.
            property = request.target;
        }

        if request.target == self.atoms.targets {
            let offered = [self.atoms.targets, self.atoms.utf8_string, XA_STRING];
            unsafe {
                (self.libs.XChangeProperty)(
                    self.display,
                    request.requestor,
                    property,
                    XA_ATOM,
                    32,
                    PropModeReplace,
                    as_bytes(&offered).as_ptr(),
                    offered.len() as c_int,
                );
            }
        } else if request.target == self.atoms.utf8_string || request.target == XA_STRING {
            unsafe {
                (self.libs.XChangeProperty)(
                    self.display,
                    request.requestor,
                    property,
                    request.target,
                    8,
                    PropModeReplace,
                    self.clipboard.as_ptr(),
                    self.clipboard.len() as c_int,
                );
            }
        } else {
            // Nothing that can be offered: the convention is a reply with no
            // property, which tells the asker to give up rather than wait.
            property = 0;
        }

        let mut event = XEvent::default();
        {
            let e = unsafe { &mut *(&mut event as *mut XEvent as *mut XSelectionEvent) };
            e.type_ = SelectionNotify;
            e.serial = 0;
            e.send_event = 1;
            e.display = self.display;
            e.requestor = request.requestor;
            e.selection = request.selection;
            e.target = request.target;
            e.property = property;
            e.time = request.time;
        }
        unsafe {
            (self.libs.XSendEvent)(self.display, request.requestor, 0, 0, &mut event);
            (self.libs.XFlush)(self.display);
        }
    }
}

impl Shell for X11Shell {
    fn window_rect(&self) -> WindowRect {
        let mut attributes = XWindowAttributes::default();
        unsafe { (self.libs.XGetWindowAttributes)(self.display, self.window, &mut attributes) };
        // A reparenting window manager puts the window inside a frame, so its
        // own `x`/`y` are relative to that. Translating to the root is what
        // gives the position actually on screen — and what makes the saved
        // one still right after a restart.
        let (mut x, mut y) = (0, 0);
        let mut child: Window = 0;
        unsafe {
            (self.libs.XTranslateCoordinates)(
                self.display,
                self.window,
                self.root,
                0,
                0,
                &mut x,
                &mut y,
                &mut child,
            );
        }
        WindowRect {
            x,
            y,
            width: attributes.width,
            height: attributes.height,
        }
    }

    fn set_window_rect(&mut self, rect: WindowRect) {
        unsafe {
            (self.libs.XMoveResizeWindow)(
                self.display,
                self.window,
                rect.x,
                rect.y,
                rect.width.max(1) as c_uint,
                rect.height.max(1) as c_uint,
            );
            (self.libs.XFlush)(self.display);
        }
    }

    /// One pixel per unit.
    ///
    /// X11 has no per-monitor scale of its own: a desktop on a high-density
    /// display sets `Xft.dpi` or scales the whole screen, and the widget's own
    /// `"scale"` setting is what adjusts it beyond that.
    fn scale(&self) -> f32 {
        1.0
    }

    fn work_area(&self) -> Option<WindowRect> {
        // `_NET_WORKAREA` would exclude panels and docks, and reading it means
        // a round trip with a reply to unpack for a value used once at first
        // start. The screen itself is close enough: the default position is a
        // corner with a margin, and the margin is what keeps it clear of a
        // panel either way.
        let width = unsafe { (self.libs.XDisplayWidth)(self.display, self.screen) };
        let height = unsafe { (self.libs.XDisplayHeight)(self.display, self.screen) };
        (width > 0 && height > 0).then_some(WindowRect {
            x: 0,
            y: 0,
            width,
            height,
        })
    }

    fn is_on_screen(&self, rect: WindowRect) -> bool {
        let Some(screen) = self.work_area() else {
            return true;
        };
        rect.x < screen.width
            && rect.y < screen.height
            && rect.x + rect.width > 0
            && rect.y + rect.height > 0
    }

    fn request_redraw(&mut self) {
        // The loop draws whenever it has been round once with something to do,
        // so there is nothing to post: the event that caused this is already
        // being handled inside it.
    }

    fn set_cursor(&mut self, cursor: AppCursor) {
        let index = match cursor {
            AppCursor::Arrow => 0,
            AppCursor::SizeNwSe => 1,
            AppCursor::SizeNeSw => 2,
            AppCursor::SizeWe => 3,
            AppCursor::SizeNs => 4,
        };
        if index == self.current_cursor {
            return;
        }
        self.current_cursor = index;
        unsafe {
            (self.libs.XDefineCursor)(self.display, self.window, self.cursors[index]);
            (self.libs.XFlush)(self.display);
        }
    }

    fn set_anim_timer(&mut self, running: bool) {
        self.animating = running;
    }

    fn set_undo_timer(&mut self, running: bool) {
        self.undo_running = running;
    }

    /// The poll loop watches [`App::peek_deadline`] itself, so the duration
    /// is nothing this front end has to arrange a timer for.
    fn set_peek(&mut self, for_at_most: Option<Duration>) {
        let peeking = for_at_most.is_some();
        if peeking == self.peeking {
            return;
        }
        self.peeking = peeking;
        if peeking {
            self.set_state(false, self.atoms.net_wm_state_below, 0);
            self.set_state(true, self.atoms.net_wm_state_above, 0);
            unsafe { (self.libs.XRaiseWindow)(self.display, self.window) };
        } else {
            self.set_state(false, self.atoms.net_wm_state_above, 0);
            self.set_state(true, self.atoms.net_wm_state_below, 0);
            unsafe { (self.libs.XLowerWindow)(self.display, self.window) };
        }
        unsafe { (self.libs.XFlush)(self.display) };
    }

    fn show_menu(
        &mut self,
        entries: &[ephemeris_core::menu::Entry],
    ) -> Option<ephemeris_core::menu::Command> {
        menu::show(self, entries)
    }

    fn open_path(&self, path: &std::path::Path) {
        crate::unix::linux::open_path(path);
    }

    fn set_clipboard(&mut self, text: &str) {
        self.clipboard = text.to_owned();
        unsafe {
            (self.libs.XSetSelectionOwner)(
                self.display,
                self.atoms.clipboard,
                self.window,
                CurrentTime,
            );
            (self.libs.XFlush)(self.display);
        }
        // X11 keeps the text in this process rather than in the server, so it
        // is available for as long as the widget runs and no longer. That is
        // the standard behaviour of a program without a clipboard manager, and
        // every desktop environment ships one that takes a copy.
    }

    fn autostart_enabled(&self) -> bool {
        autostart::enabled()
    }

    fn set_autostart(&mut self, enabled: bool) {
        autostart::set(enabled);
    }

    fn quit(&mut self) {
        self.quit = true;
        if let Some((keycode, mask)) = self.hotkey.take() {
            for extra in LOCK_VARIANTS {
                unsafe {
                    (self.libs.XUngrabKey)(self.display, keycode as c_int, mask | extra, self.root)
                };
            }
        }
        log::info("Shut down");
    }

    fn system_visuals(&self) -> SystemVisuals {
        visuals::read()
    }
}

// --- Waiting ----------------------------------------------------------------

/// A non-blocking pipe: `(read, write)`.
fn pipe() -> Option<(c_int, c_int)> {
    let mut fds = [0 as c_int; 2];
    // `O_CLOEXEC` so the browser the sign-in flow opens does not inherit it,
    // and `O_NONBLOCK` on the write end so a full pipe drops the extra wake
    // rather than blocking the sync thread.
    let ok = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) };
    (ok == 0).then_some((fds[0], fds[1]))
}

/// Sleeps until one of the descriptors has something, or the timeout runs out.
///
/// A negative descriptor is ignored by `poll`, which is what makes the control
/// socket optional without a second code path.
fn wait(fds: &[c_int], timeout: Duration) {
    let mut poll: Vec<libc::pollfd> = fds
        .iter()
        .map(|&fd| libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        })
        .collect();
    let ms = timeout.as_millis().min(60_000) as c_int;
    unsafe { libc::poll(poll.as_mut_ptr(), poll.len() as libc::nfds_t, ms) };
}

/// Empties the wake pipe; true if there was anything in it.
fn drain(wake: c_int) -> bool {
    let mut buffer = [0u8; 64];
    let mut woken = false;
    loop {
        let read = unsafe { libc::read(wake, buffer.as_mut_ptr() as *mut c_void, buffer.len()) };
        if read <= 0 {
            return woken;
        }
        woken = true;
    }
}

fn next_minute_boundary() -> Instant {
    use chrono::Timelike;
    let now = chrono::Local::now();
    let ms = 60_000 - (now.second() * 1000 + now.timestamp_subsec_millis()) as i64;
    Instant::now() + Duration::from_millis(ms.clamp(1_000, 60_000) as u64)
}

// --- Two more Xlib structures -----------------------------------------------

/// `XA_WM_NAME` from `Xatom.h`. Not in [`super::ffi`] because nothing else
/// names a window.
const XA_WM_NAME: Atom = 39;

/// Not in [`super::ffi`] because only this file sends or reads one.
#[repr(C)]
struct XClientMessageEvent {
    type_: c_int,
    serial: c_ulong,
    send_event: c_int,
    display: *mut Display,
    window: Window,
    message_type: Atom,
    format: c_int,
    data: [c_long; 5],
}

const ClientMessage: c_int = 33;
const SelectionNotify: c_int = 31;
const SubstructureNotifyMask: c_long = 1 << 19;
const SubstructureRedirectMask: c_long = 1 << 20;
