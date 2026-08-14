// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Ephemeris contributors
//! The widget's window on macOS.
//!
//! It behaves the way the Windows one does, through the equivalent AppKit
//! settings:
//!
//! * **Level** `kCGDesktopIconWindowLevel + 1` — below every ordinary window,
//!   above the desktop and its icons, and still clickable. This is the one
//!   thing the widget is defined by.
//! * **`canBecomeKeyWindow` returns NO** — clicking it never takes focus away
//!   from what you are working in. That is what separates a gadget from a
//!   window in the way.
//! * **Collection behaviour** `.canJoinAllSpaces | .stationary | .ignoresCycle`
//!   — it follows you between Spaces, does not slide away under Mission
//!   Control, and never appears in the window switcher.
//! * **Activation policy** `.accessory` — no Dock tile and no menu bar, which
//!   is what `LSUIElement` gives a bundle. Set in code so the bare binary from
//!   the tarball behaves the same as the `.app`.
//! * **Borderless and not opaque**, with `hasShadow` off: the renderer draws
//!   the glass, the rounded corners and the shadow itself, exactly as on
//!   Windows.
//!
//! **Coordinates.** AppKit measures screens from the bottom left and the rest
//! of this program from the top left. The conversion happens here, at the two
//! functions that touch a screen rectangle, so `config.json` stores the same
//! kind of coordinates on every platform and [`crate::unix::app`] needs no
//! special case. The view is flipped, so inside the window the two agree
//! already.
//!
//! **State.** AppKit calls back into C functions with no context of their own,
//! so the widget lives in a thread-local that each callback *takes* for the
//! duration of its work. Taking rather than borrowing is what makes
//! re-entrancy safe: showing the context menu runs a nested event loop, and
//! anything that arrives during it finds the slot empty and does nothing,
//! rather than panicking on a double borrow.

use crate::paint::widget as paint;
use crate::unix::app::{App, Cursor, Shell, WindowRect};
use crate::unix::autostart;
use crate::unix::mac::canvas::Cg;
use crate::unix::mac::objc::*;
use crate::unix::mac::{hotkey, menu, visuals};
use ephemeris_core::host::Waker;
use ephemeris_core::log;
use ephemeris_core::theme::SystemVisuals;
use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

thread_local! {
    /// The running widget. See the module note on why this is taken rather
    /// than borrowed.
    static STATE: RefCell<Option<Box<MacState>>> = const { RefCell::new(None) };

    /// A sync finished while the state was out.
    ///
    /// [`with_state`] drops a callback that arrives re-entrantly, which is the
    /// right answer for a redraw or a pointer move — the next event makes those
    /// good. It is the wrong answer for `onWake:`: `App::on_sync_done` is the
    /// only edge that stops the spinner, clears the failure count and schedules
    /// the next sync, and nothing else ever calls it. Dropped during the nested
    /// loop of a context menu, the spinner would keep turning — and with it the
    /// sixty-a-second animation timer — until the sync after next happened to
    /// land at a better moment. So it is latched here instead and delivered by
    /// the next callback that gets through.
    static PENDING_WAKE: Cell<bool> = const { Cell::new(false) };
}

/// Runs `f` with the widget, if it is not already in use further up the stack.
///
/// A callback arriving while the state is out — during the nested loop the
/// context menu runs, or while a previous callback is still working — is
/// dropped. Each of those is a redraw or a pointer move that the next event
/// makes good anyway.
fn with_state<R>(f: impl FnOnce(&mut MacState) -> R) -> Option<R> {
    let taken = STATE.with(|s| s.borrow_mut().take());
    let mut state = taken?;
    let out = f(&mut state);
    STATE.with(|s| *s.borrow_mut() = Some(state));
    Some(out)
}

struct MacState {
    app: App,
    shell: MacShell,
    canvas: Cg,
}

/// The window and everything the widget asks the window system for.
pub struct MacShell {
    window: Id,
    view: Id,
    /// Receives the timer, menu and wake callbacks.
    delegate: Id,
    anim_timer: Option<Obj>,
    undo_timer: Option<Obj>,
    minute_timer: Option<Obj>,
    /// One-shot, armed for the length of a peek. See [`MacShell::set_peek`].
    peek_timer: Option<Obj>,
    /// The level the window rests at, and the one it rises to for a peek.
    resting_level: isize,
    quit: bool,
    /// Set to true when the renderer's fonts have to be rebuilt.
    rebuild: bool,
}

/// Wakes the run loop from the sync thread.
///
/// `performSelectorOnMainThread:` rather than posting an event: it is the
/// documented way in, it needs no `NSEvent` built off the main thread, and it
/// coalesces nothing — every finished command produces exactly one callback.
struct MacWaker(usize);

// The delegate is created before the sync thread starts and lives until the
// process exits, so the pointer stays valid. Nothing is done with it off the
// main thread except hand it to `performSelectorOnMainThread:`, which is
// documented as thread-safe.
unsafe impl Send for MacWaker {}
unsafe impl Sync for MacWaker {}

impl Waker for MacWaker {
    fn wake(&self) {
        let _pool = Pool::new();
        unsafe {
            send3::<Sel, Id, i8, ()>(
                self.0 as Id,
                c"performSelectorOnMainThread:withObject:waitUntilDone:",
                sel(c"onWake:"),
                nil,
                0,
            );
            // The loop may be blocked in `nextEventMatchingMask:` with nothing
            // else due; without this the perform sits in the queue until the
            // user moves the mouse.
            CFRunLoopWakeUp(CFRunLoopGetMain());
        }
    }
}

/// Builds the window and runs the event loop. Returns when the widget quits.
pub fn run() -> Result<(), String> {
    let _pool = Pool::new();
    unsafe {
        let app: Id = send(class(c"NSApplication") as Id, c"sharedApplication");
        if app.is_null() {
            return Err("no window server session — cannot open a window".into());
        }
        // No Dock tile, no menu bar. Has to happen before the first window.
        send1::<isize, ()>(
            app,
            c"setActivationPolicy:",
            NSApplicationActivationPolicyAccessory,
        );

        let classes = classes();
        let delegate: Id = {
            let raw: Id = send(classes.delegate, c"alloc");
            send(raw, c"init")
        };

        let visuals = visuals::read();
        let (mut widget, cfg) = App::new(visuals, Arc::new(MacWaker(delegate as usize)));

        // Provisional; the real rectangle needs a screen to measure against,
        // which the shell provides once the window exists.
        let window: Id = {
            let raw: Id = send(classes.window, c"alloc");
            send4::<NSRect, usize, usize, i8, Id>(
                raw,
                c"initWithContentRect:styleMask:backing:defer:",
                NSRect::new(0.0, 0.0, 400.0, 640.0),
                NSWindowStyleMaskBorderless,
                NSBackingStoreBuffered,
                0,
            )
        };
        if window.is_null() {
            return Err("NSWindow could not be created".into());
        }

        let resting_level = CGWindowLevelForKey(kCGDesktopIconWindowLevelKey) as isize + 1;
        send1::<isize, ()>(window, c"setLevel:", resting_level);
        send1::<usize, ()>(
            window,
            c"setCollectionBehavior:",
            NSWindowCollectionBehaviorCanJoinAllSpaces
                | NSWindowCollectionBehaviorStationary
                | NSWindowCollectionBehaviorIgnoresCycle,
        );
        send1::<i8, ()>(window, c"setOpaque:", 0);
        // The renderer draws the glass, so the window itself is transparent
        // and casts no shadow of its own — a second, square one behind the
        // rounded panel is exactly what this avoids.
        send1::<i8, ()>(window, c"setHasShadow:", 0);
        send1::<i8, ()>(window, c"setAcceptsMouseMovedEvents:", 1);
        send1::<i8, ()>(window, c"setReleasedWhenClosed:", 0);
        send1::<Id, ()>(
            window,
            c"setBackgroundColor:",
            send(class(c"NSColor") as Id, c"clearColor"),
        );
        // Excluded from the window list, so screen sharing and Mission Control
        // treat it as furniture rather than as a document.
        send1::<i8, ()>(window, c"setExcludedFromWindowsMenu:", 1);

        let view: Id = {
            let raw: Id = send(classes.view, c"alloc");
            send1::<NSRect, Id>(raw, c"initWithFrame:", NSRect::new(0.0, 0.0, 400.0, 640.0))
        };
        send1::<Id, ()>(window, c"setContentView:", view);
        add_tracking_area(view);

        let canvas = Cg::new(widget.metrics, &widget.appearance, widget.loc.rtl);
        let mut shell = MacShell {
            window,
            view,
            delegate,
            anim_timer: None,
            undo_timer: None,
            minute_timer: None,
            peek_timer: None,
            resting_level,
            quit: false,
            rebuild: false,
        };

        let rect = widget.target_geometry(&cfg, &shell);
        shell.set_window_rect(rect);
        widget.rescue_offscreen(&mut shell);
        // The reveal starts at zero when there is cached data to fade in, and
        // only moves while the animation timer runs. This is the first moment
        // there is a shell to start it on.
        widget.start(&mut shell);
        hotkey::install(&cfg.peek_hotkey);
        visuals::watch(delegate);

        // `orderFront:` would raise it above other windows for an instant.
        // `orderBack:` puts it straight in at its level without ever being on
        // top and without activating this application.
        send1::<Id, ()>(window, c"orderBack:", nil);

        shell.arm_minute_timer();
        STATE.with(|s| {
            *s.borrow_mut() = Some(Box::new(MacState {
                app: widget,
                shell,
                canvas,
            }))
        });
        with_state(|st| {
            let (app, shell) = (&mut st.app, &mut st.shell);
            app.kick(shell);
        });

        event_loop(app);
    }

    // The window is gone; what is left is to stop the sync thread and let a
    // completion that has not gone out yet do so.
    STATE.with(|s| {
        if let Some(st) = s.borrow_mut().as_mut() {
            st.app.commit_pending_on_exit();
            st.app.shut_down();
        }
    });
    Ok(())
}

/// The event loop, written out rather than `[NSApp run]`.
///
/// `-run` never returns until `-stop:` is sent, and `-stop:` only takes effect
/// once the loop finishes the event it happens to be in — so quitting from the
/// menu would leave the widget closed but the process alive until the next
/// mouse movement arrived. Pumping the loop here makes the quit flag the only
/// thing that decides, and costs four lines.
///
/// `nextEventMatchingMask:` runs the run loop while it waits, so timers and
/// the sync thread's `performSelectorOnMainThread:` are serviced exactly as
/// they would be under `-run`.
///
/// # Safety
///
/// `app` must be the shared `NSApplication`.
unsafe fn event_loop(app: Id) {
    unsafe {
        // `-run` would do this; a hand-written loop has to.
        send::<()>(app, c"finishLaunching");
        let forever: Id = send(class(c"NSDate") as Id, c"distantFuture");
        // `NSDefaultRunLoopMode` is this string; there is no exported symbol
        // to link against from outside Objective-C.
        let mode = nsstring("kCFRunLoopDefaultMode");

        loop {
            // One pool per event, so the autoreleased strings a frame creates
            // are freed at the same rate they are made.
            let _pool = Pool::new();
            if with_state(|st| st.shell.quit).unwrap_or(false) {
                return;
            }
            let event: Id = send4(
                app,
                c"nextEventMatchingMask:untilDate:inMode:dequeue:",
                NSEventMaskAny,
                forever,
                mode,
                1i8,
            );
            if !event.is_null() {
                send1::<Id, ()>(app, c"sendEvent:", event);
            }
        }
    }
}

// --- The classes ------------------------------------------------------------

struct Classes {
    window: Class,
    view: Class,
    delegate: Class,
}

// Registered once and never freed, which is what `objc_registerClassPair`
// expects; the pointers are valid for the life of the process.
unsafe impl Send for Classes {}
unsafe impl Sync for Classes {}

/// Defines the three classes AppKit needs from us, once.
///
/// Building them at runtime rather than declaring them in Objective-C is what
/// keeps this a single Rust binary with no build step. The three overrides are
/// the minimum: a window that refuses focus, a view with the origin at the top
/// that draws and takes events, and an object for timers and callbacks to
/// target.
fn classes() -> &'static Classes {
    static CLASSES: OnceLock<Classes> = OnceLock::new();
    CLASSES.get_or_init(|| unsafe {
        let window = objc_allocateClassPair(class(c"NSWindow"), c"EphemerisWindow".as_ptr(), 0);
        add_method(
            window,
            c"canBecomeKeyWindow",
            can_become_key as *const c_void,
            c"c@:",
        );
        add_method(
            window,
            c"canBecomeMainWindow",
            can_become_key as *const c_void,
            c"c@:",
        );
        objc_registerClassPair(window);

        let view = objc_allocateClassPair(class(c"NSView"), c"EphemerisView".as_ptr(), 0);
        add_method(view, c"isFlipped", is_flipped as *const c_void, c"c@:");
        add_method(
            view,
            c"drawRect:",
            draw_rect as *const c_void,
            c"v@:{CGRect={CGPoint=dd}{CGSize=dd}}",
        );
        for name in [
            c"mouseDown:",
            c"mouseUp:",
            c"mouseDragged:",
            c"mouseMoved:",
            c"mouseExited:",
            c"rightMouseUp:",
            c"scrollWheel:",
        ] {
            add_method(view, name, mouse_event as *const c_void, c"v@:@");
        }
        objc_registerClassPair(view);

        let delegate = objc_allocateClassPair(class(c"NSObject"), c"EphemerisAgent".as_ptr(), 0);
        add_method(delegate, c"onAnim:", on_anim as *const c_void, c"v@:@");
        add_method(delegate, c"onUndo:", on_undo as *const c_void, c"v@:@");
        add_method(delegate, c"onMinute:", on_minute as *const c_void, c"v@:@");
        add_method(delegate, c"onPeek:", on_peek as *const c_void, c"v@:@");
        add_method(delegate, c"onWake:", on_wake as *const c_void, c"v@:@");
        add_method(
            delegate,
            c"onAppearanceChanged:",
            on_appearance as *const c_void,
            c"v@:@",
        );
        add_method(
            delegate,
            c"menuAction:",
            menu::action as *const c_void,
            c"v@:@",
        );
        objc_registerClassPair(delegate);

        Classes {
            window,
            view,
            delegate,
        }
    })
}

extern "C" fn can_become_key(_this: Id, _sel: Sel) -> i8 {
    0
}

/// The origin at the top left, so view coordinates and
/// [`ephemeris_core::layout`] agree without a conversion at every call.
extern "C" fn is_flipped(_this: Id, _sel: Sel) -> i8 {
    1
}

extern "C" fn draw_rect(_this: Id, _sel: Sel, _dirty: NSRect) {
    with_state(|st| {
        let ctx: CGContextRef = unsafe {
            let gc: Id = send(class(c"NSGraphicsContext") as Id, c"currentContext");
            if gc.is_null() {
                return;
            }
            send(gc, c"CGContext")
        };
        if ctx.is_null() {
            return;
        }

        if std::mem::take(&mut st.shell.rebuild) || std::mem::take(&mut st.app.needs_rebuild) {
            st.canvas = Cg::new(st.app.metrics, &st.app.appearance, st.app.loc.rtl);
        }

        let size = st.shell.size_points();
        let (metrics, palette, rtl) = (st.app.metrics, st.app.palette, st.app.loc.rtl);
        let appearance = st.app.appearance.clone();
        let canvas = &mut st.canvas;
        unsafe { canvas.begin(ctx) };
        let result = st.app.with_frame(|frame, hits| {
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
        st.canvas.end();
        st.app
            .frame_drawn(result.content_height, result.viewport_height);
    });
}

/// Every pointer event, dispatched on the selector AppKit called.
///
/// One function rather than seven: each of them reads the same two positions
/// out of the event and differs only in which method of the widget it calls,
/// and seven near-identical `extern "C"` shims is where a copied line ends up
/// in the wrong one.
extern "C" fn mouse_event(this: Id, selector: Sel, event: Id) {
    with_state(|st| {
        let (x, y) = view_point(this, event);
        let pointer = st.shell.pointer_on_screen();
        let (app, shell) = (&mut st.app, &mut st.shell);

        if selector == sel(c"mouseDown:") {
            app.on_press(shell, x, y, pointer);
        } else if selector == sel(c"mouseUp:") {
            app.on_release(shell);
        } else if selector == sel(c"mouseDragged:") || selector == sel(c"mouseMoved:") {
            app.on_pointer_move(shell, x, y, pointer);
        } else if selector == sel(c"mouseExited:") {
            app.on_pointer_leave(shell);
        } else if selector == sel(c"scrollWheel:") {
            let precise: i8 = unsafe { send(event, c"hasPreciseScrollingDeltas") };
            let dy: CGFloat = unsafe { send(event, c"scrollingDeltaY") };
            // A trackpad reports points, a wheel reports lines. Both end up as
            // "lines" for the widget, which multiplies by a row height.
            let lines = if precise != 0 { dy / 16.0 } else { dy };
            app.on_scroll(shell, lines as f32);
        } else if selector == sel(c"rightMouseUp:") {
            // The menu runs a nested event loop and blocks until it closes.
            // That is safe here because `with_state` has already taken the
            // widget out of the slot: whatever arrives meanwhile finds it
            // empty and is dropped, rather than re-entering.
            app.show_menu(shell);
        }
    });
    // The menu's nested loop is the likeliest place for a sync to have
    // finished while the widget was out of its slot.
    deliver_wake();
}

extern "C" fn on_anim(_this: Id, _sel: Sel, _timer: Id) {
    deliver_wake();
    with_state(|st| {
        let (app, shell) = (&mut st.app, &mut st.shell);
        app.pump(shell);
    });
}

extern "C" fn on_undo(_this: Id, _sel: Sel, _timer: Id) {
    with_state(|st| {
        let (app, shell) = (&mut st.app, &mut st.shell);
        app.on_undo_tick(shell);
    });
}

extern "C" fn on_minute(_this: Id, _sel: Sel, _timer: Id) {
    deliver_wake();
    with_state(|st| {
        let (app, shell) = (&mut st.app, &mut st.shell);
        app.on_minute(shell);
        shell.arm_minute_timer();
    });
}

/// The peek is over: back down to the desktop level.
extern "C" fn on_peek(_this: Id, _sel: Sel, _timer: Id) {
    with_state(|st| {
        let (app, shell) = (&mut st.app, &mut st.shell);
        app.end_peek(shell);
    });
}

extern "C" fn on_wake(_this: Id, _sel: Sel, _arg: Id) {
    PENDING_WAKE.with(|p| p.set(true));
    deliver_wake();
}

/// Hands a finished sync to the widget, or leaves it latched for the next
/// callback if the state is out. Must not be called from inside [`with_state`].
fn deliver_wake() {
    if !PENDING_WAKE.with(|p| p.get()) {
        return;
    }
    let delivered = with_state(|st| {
        let (app, shell) = (&mut st.app, &mut st.shell);
        app.on_sync_done(shell);
    });
    if delivered.is_some() {
        PENDING_WAKE.with(|p| p.set(false));
    }
}

extern "C" fn on_appearance(_this: Id, _sel: Sel, _note: Id) {
    with_state(|st| {
        let (app, shell) = (&mut st.app, &mut st.shell);
        app.refresh_palette(shell);
        shell.rebuild = true;
        shell.request_redraw();
    });
}

/// Called by [`hotkey`] when the global shortcut fires.
pub fn on_hotkey() {
    with_state(|st| {
        let (app, shell) = (&mut st.app, &mut st.shell);
        app.begin_peek(shell);
    });
}

// --- The shell --------------------------------------------------------------

impl MacShell {
    /// The view's size in points, which is what the renderer draws in.
    fn size_points(&self) -> (f32, f32) {
        let bounds = unsafe { send_rect(self.view, c"bounds") };
        (bounds.size.width as f32, bounds.size.height as f32)
    }

    /// The pointer's position on the desktop, measured from the top left.
    fn pointer_on_screen(&self) -> (i32, i32) {
        let p: NSPoint = unsafe { send(class(c"NSEvent") as Id, c"mouseLocation") };
        (p.x.round() as i32, (primary_height() - p.y).round() as i32)
    }

    /// The minute tick, armed to the next full minute rather than repeating
    /// every second: the widget must cost nothing while it sits there.
    fn arm_minute_timer(&mut self) {
        let now = chrono::Local::now();
        let ms =
            60_000 - (chrono::Timelike::second(&now) * 1000 + now.timestamp_subsec_millis()) as i64;
        let seconds = (ms.clamp(1_000, 60_000) as f64) / 1000.0;
        self.minute_timer = self.schedule(seconds, c"onMinute:", false);
    }

    fn schedule(&self, seconds: f64, selector: &std::ffi::CStr, repeats: bool) -> Option<Obj> {
        unsafe {
            let timer: Id = send5(
                class(c"NSTimer") as Id,
                c"scheduledTimerWithTimeInterval:target:selector:userInfo:repeats:",
                seconds,
                self.delegate,
                sel(selector),
                nil,
                repeats as i8,
            );
            // `scheduledTimer…` returns an autoreleased timer the run loop
            // retains. Retained again here so invalidating it later is safe
            // even once the pool that held it has drained.
            let retained: Id = send(timer, c"retain");
            Obj::new(retained)
        }
    }
}

impl Shell for MacShell {
    fn window_rect(&self) -> WindowRect {
        let frame = unsafe { send_rect(self.window, c"frame") };
        WindowRect {
            x: frame.origin.x.round() as i32,
            y: (primary_height() - frame.origin.y - frame.size.height).round() as i32,
            width: frame.size.width.round() as i32,
            height: frame.size.height.round() as i32,
        }
    }

    fn set_window_rect(&mut self, rect: WindowRect) {
        let frame = NSRect::new(
            rect.x as f64,
            primary_height() - (rect.y + rect.height) as f64,
            rect.width.max(1) as f64,
            rect.height.max(1) as f64,
        );
        unsafe {
            // `display:` is NO on purpose. Every caller reaches this from
            // inside `with_state` — the resize drag, the off-screen rescue, a
            // reloaded configuration — and a synchronous display pass re-enters
            // `drawRect:` while the widget is out of its slot: the draw is
            // dropped, and AppKit has already erased the view against the
            // window's clear colour. Marking it dirty instead paints on the
            // next turn of the run loop, when the state is back.
            send2::<NSRect, i8, ()>(self.window, c"setFrame:display:", frame, 0);
            send1::<i8, ()>(self.view, c"setNeedsDisplay:", 1);
        }
    }

    /// One point per unit. The metrics are in Windows device-independent
    /// pixels, and a point is the unit the interface conventions here are
    /// written in — see the note in [`crate::unix::mac::canvas`]. Retina is
    /// AppKit's business and never reaches this program.
    fn scale(&self) -> f32 {
        1.0
    }

    fn work_area(&self) -> Option<WindowRect> {
        let screen = primary_screen()?;
        let visible = unsafe { send_rect(screen, c"visibleFrame") };
        Some(WindowRect {
            x: visible.origin.x.round() as i32,
            y: (primary_height() - visible.origin.y - visible.size.height).round() as i32,
            width: visible.size.width.round() as i32,
            height: visible.size.height.round() as i32,
        })
    }

    fn is_on_screen(&self, rect: WindowRect) -> bool {
        unsafe {
            let screens: Id = send(class(c"NSScreen") as Id, c"screens");
            let count: usize = send(screens, c"count");
            let height = primary_height();
            (0..count).any(|i| {
                let screen: Id = send1(screens, c"objectAtIndex:", i);
                let f = send_rect(screen, c"frame");
                let top = height - f.origin.y - f.size.height;
                overlaps(
                    rect,
                    WindowRect {
                        x: f.origin.x.round() as i32,
                        y: top.round() as i32,
                        width: f.size.width.round() as i32,
                        height: f.size.height.round() as i32,
                    },
                )
            })
        }
    }

    fn request_redraw(&mut self) {
        unsafe { send1::<i8, ()>(self.view, c"setNeedsDisplay:", 1) };
    }

    fn set_cursor(&mut self, cursor: Cursor) {
        let name: &std::ffi::CStr = match cursor {
            Cursor::Arrow => c"arrowCursor",
            Cursor::SizeWe => c"resizeLeftRightCursor",
            Cursor::SizeNs => c"resizeUpDownCursor",
            // AppKit has no *public* diagonal resize cursor. The private ones
            // are what every application that needs them uses, so they are
            // tried first and the nearest public one stands in if a future
            // release removes them — a slightly wrong arrow is better than a
            // crash, and better than no resize affordance at all.
            Cursor::SizeNwSe => c"_windowResizeNorthWestSouthEastCursor",
            Cursor::SizeNeSw => c"_windowResizeNorthEastSouthWestCursor",
        };
        unsafe {
            let class_id = class(c"NSCursor") as Id;
            let responds: i8 = send1(class_id, c"respondsToSelector:", sel(name));
            let selector = if responds != 0 {
                name
            } else {
                c"resizeLeftRightCursor"
            };
            let cursor: Id = send(class_id, selector);
            if !cursor.is_null() {
                send::<()>(cursor, c"set");
            }
        }
    }

    fn set_anim_timer(&mut self, running: bool) {
        match running {
            true if self.anim_timer.is_none() => {
                self.anim_timer = self.schedule(
                    crate::unix::app::ANIM_INTERVAL.as_secs_f64(),
                    c"onAnim:",
                    true,
                );
            }
            false => invalidate(&mut self.anim_timer),
            _ => {}
        }
    }

    fn set_undo_timer(&mut self, running: bool) {
        match running {
            true if self.undo_timer.is_none() => {
                self.undo_timer = self.schedule(
                    crate::unix::app::UNDO_INTERVAL.as_secs_f64(),
                    c"onUndo:",
                    true,
                );
            }
            false => invalidate(&mut self.undo_timer),
            _ => {}
        }
    }

    /// Raises the window for the length of a peek, and arms the timer that
    /// puts it back.
    ///
    /// The timer is what this loop needs and the X11 one does not: nothing
    /// else wakes AppKit while the widget sits there, so without it the peek
    /// would last until the next minute tick noticed it — up to fifty-five
    /// seconds of a widget that is supposed to sink back after five.
    fn set_peek(&mut self, for_at_most: Option<Duration>) {
        invalidate(&mut self.peek_timer);
        let level = match for_at_most {
            Some(_) => NSFloatingWindowLevel,
            None => self.resting_level,
        };
        unsafe {
            send1::<isize, ()>(self.window, c"setLevel:", level);
            // Without this the window keeps its old place in the order until
            // something else disturbs it.
            let selector = if for_at_most.is_some() {
                c"orderFront:"
            } else {
                c"orderBack:"
            };
            send1::<Id, ()>(self.window, selector, nil);
        }
        if let Some(window) = for_at_most {
            self.peek_timer = self.schedule(window.as_secs_f64(), c"onPeek:", false);
        }
    }

    fn show_menu(
        &mut self,
        entries: &[ephemeris_core::menu::Entry],
    ) -> Option<ephemeris_core::menu::Command> {
        menu::show(entries, self.delegate)
    }

    fn open_path(&self, path: &std::path::Path) {
        // `open` handles a folder, a file and a URL alike, and picks whatever
        // the user has registered for each. Nothing goes through a shell.
        let run = std::process::Command::new("open")
            .arg(path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        match run {
            Ok(mut child) => {
                // Reaped so the widget does not collect zombies over days of
                // uptime, for the reason set out in `unix::host::open_url`.
                let _ = std::thread::Builder::new()
                    .name("ephemeris-open".into())
                    .spawn(move || {
                        let _ = child.wait();
                    });
            }
            Err(e) => log::warn(&format!("Could not run open: {e}")),
        }
    }

    fn set_clipboard(&mut self, text: &str) {
        unsafe {
            let pasteboard: Id = send(class(c"NSPasteboard") as Id, c"generalPasteboard");
            if pasteboard.is_null() {
                return;
            }
            send::<i64>(pasteboard, c"clearContents");
            let types: Id = send1(
                class(c"NSArray") as Id,
                c"arrayWithObject:",
                nsstring("public.utf8-plain-text"),
            );
            send2::<Id, Id, i8>(pasteboard, c"declareTypes:owner:", types, nil);
            let ok: i8 = send2(
                pasteboard,
                c"setString:forType:",
                nsstring(text),
                nsstring("public.utf8-plain-text"),
            );
            if ok == 0 {
                log::warn("The clipboard refused the agenda");
            }
        }
    }

    fn autostart_enabled(&self) -> bool {
        autostart::enabled()
    }

    fn set_autostart(&mut self, enabled: bool) {
        autostart::set(enabled);
    }

    fn quit(&mut self) {
        self.quit = true;
        invalidate(&mut self.anim_timer);
        invalidate(&mut self.undo_timer);
        invalidate(&mut self.minute_timer);
        invalidate(&mut self.peek_timer);
        hotkey::remove();
        unsafe {
            send::<()>(self.window, c"close");
        }
        log::info("Shut down");
    }

    fn system_visuals(&self) -> SystemVisuals {
        visuals::read()
    }
}

fn invalidate(timer: &mut Option<Obj>) {
    if let Some(t) = timer.take() {
        unsafe { send::<()>(t.id(), c"invalidate") };
    }
}

/// Do two rectangles share any area?
fn overlaps(a: WindowRect, b: WindowRect) -> bool {
    a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height
}

/// The screen whose origin is (0, 0) — the one with the menu bar.
///
/// Everything else is measured against it, because that is the frame AppKit's
/// global coordinates are written in.
fn primary_screen() -> Option<Id> {
    unsafe {
        let screens: Id = send(class(c"NSScreen") as Id, c"screens");
        let count: usize = send(screens, c"count");
        (count > 0).then(|| send1(screens, c"objectAtIndex:", 0usize))
    }
}

fn primary_height() -> f64 {
    primary_screen()
        .map(|s| unsafe { send_rect(s, c"frame") }.size.height)
        .unwrap_or(0.0)
}

/// A mouse event's position inside the view, in points from its top left.
fn view_point(view: Id, event: Id) -> (f32, f32) {
    unsafe {
        let in_window: NSPoint = send(event, c"locationInWindow");
        let p: NSPoint = send2(view, c"convertPoint:fromView:", in_window, nil);
        (p.x as f32, p.y as f32)
    }
}

/// The tracking area that makes `mouseMoved:` and `mouseExited:` arrive.
///
/// `inVisibleRect` so it follows the view through every resize; without it the
/// area would keep the size the window had at start-up and hover would stop
/// working over the part that grew.
fn add_tracking_area(view: Id) {
    unsafe {
        let raw: Id = send(class(c"NSTrackingArea") as Id, c"alloc");
        let area: Id = send4(
            raw,
            c"initWithRect:options:owner:userInfo:",
            NSRect::default(),
            NSTrackingMouseEnteredAndExited
                | NSTrackingMouseMoved
                | NSTrackingActiveAlways
                | NSTrackingInVisibleRect,
            view,
            nil,
        );
        if let Some(area) = Obj::new(area) {
            send1::<Id, ()>(view, c"addTrackingArea:", area.id());
        }
    }
}
