// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! The slice of Xlib, Cairo and Pango the Linux front end needs, resolved at
//! run time.
//!
//! Every entry is a function pointer looked up once when the library opens.
//! Missing any one of them means the desktop's libraries are older than this
//! program expects, and [`Libs::load`] then reports which and returns `None`,
//! which sends the front end to the printed agenda instead of failing to
//! start.
//!
//! **Pango rather than Cairo's own text API.** `cairo_show_text` is documented
//! as a "toy" interface: one font, no shaping, no bidirectional reordering, no
//! fallback for a script the font does not cover. The widget ships twenty
//! catalogues, Arabic and Hebrew among them, and picks its date formats out of
//! the C library's locale database — so text that cannot shape or reorder
//! would undo most of that. Pango is on every desktop that has Cairo, because
//! GTK needs both.
//!
//! The signatures below are copied from the C headers and there is nothing
//! that checks them, which is the price of `dlopen`. They are gathered here
//! rather than spread across the callers precisely so that risk sits in one
//! reviewable place.

#![allow(non_camel_case_types, non_upper_case_globals)]

use super::dl::Library;
use std::ffi::{c_char, c_int, c_long, c_uint, c_ulong, c_void};

// --- Xlib types -------------------------------------------------------------

pub type Display = c_void;
pub type Window = c_ulong;
pub type Atom = c_ulong;
pub type Colormap = c_ulong;
pub type Cursor = c_ulong;
pub type KeySym = c_ulong;
pub type Time = c_ulong;
pub type VisualPtr = *mut c_void;

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct XVisualInfo {
    pub visual: VisualPtr,
    pub visualid: c_ulong,
    pub screen: c_int,
    pub depth: c_int,
    pub class: c_int,
    pub red_mask: c_ulong,
    pub green_mask: c_ulong,
    pub blue_mask: c_ulong,
    pub colormap_size: c_int,
    pub bits_per_rgb: c_int,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct XSetWindowAttributes {
    pub background_pixmap: c_ulong,
    pub background_pixel: c_ulong,
    pub border_pixmap: c_ulong,
    pub border_pixel: c_ulong,
    pub bit_gravity: c_int,
    pub win_gravity: c_int,
    pub backing_store: c_int,
    pub backing_planes: c_ulong,
    pub backing_pixel: c_ulong,
    pub save_under: c_int,
    pub event_mask: c_long,
    pub do_not_propagate_mask: c_long,
    pub override_redirect: c_int,
    pub colormap: Colormap,
    pub cursor: Cursor,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct XWindowAttributes {
    pub x: c_int,
    pub y: c_int,
    pub width: c_int,
    pub height: c_int,
    pub border_width: c_int,
    pub depth: c_int,
    pub visual: VisualPtr,
    pub root: Window,
    pub class: c_int,
    pub bit_gravity: c_int,
    pub win_gravity: c_int,
    pub backing_store: c_int,
    pub backing_planes: c_ulong,
    pub backing_pixel: c_ulong,
    pub save_under: c_int,
    pub colormap: Colormap,
    pub map_installed: c_int,
    pub map_state: c_int,
    pub all_event_masks: c_long,
    pub your_event_mask: c_long,
    pub do_not_propagate_mask: c_long,
    pub override_redirect: c_int,
    pub screen: *mut c_void,
}

/// `XEvent` is a union of every event structure. Only the head is read
/// directly; the rest is addressed through the typed views below, which is
/// what the C header does too.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct XEvent {
    /// Large enough for the biggest member, which is what the union's
    /// declaration comes to; Xlib writes through this and reads its own size.
    pub pad: [c_long; 24],
}

impl XEvent {
    pub fn kind(&self) -> c_int {
        self.pad[0] as c_int
    }

    /// # Safety
    ///
    /// The event's `kind` must match the structure being read.
    pub unsafe fn as_ref<T>(&self) -> &T {
        unsafe { &*(self as *const XEvent as *const T) }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct XButtonEvent {
    pub type_: c_int,
    pub serial: c_ulong,
    pub send_event: c_int,
    pub display: *mut Display,
    pub window: Window,
    pub root: Window,
    pub subwindow: Window,
    pub time: Time,
    pub x: c_int,
    pub y: c_int,
    pub x_root: c_int,
    pub y_root: c_int,
    pub state: c_uint,
    pub button: c_uint,
    pub same_screen: c_int,
}

/// `XMotionEvent` has the same prefix as `XButtonEvent` up to `state`, with
/// `is_hint` where the button number is. Only the shared prefix is read.
pub type XMotionEvent = XButtonEvent;
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct XConfigureEvent {
    pub type_: c_int,
    pub serial: c_ulong,
    pub send_event: c_int,
    pub display: *mut Display,
    pub event: Window,
    pub window: Window,
    pub x: c_int,
    pub y: c_int,
    pub width: c_int,
    pub height: c_int,
    pub border_width: c_int,
    pub above: Window,
    pub override_redirect: c_int,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct XSelectionRequestEvent {
    pub type_: c_int,
    pub serial: c_ulong,
    pub send_event: c_int,
    pub display: *mut Display,
    pub owner: Window,
    pub requestor: Window,
    pub selection: Atom,
    pub target: Atom,
    pub property: Atom,
    pub time: Time,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct XSelectionEvent {
    pub type_: c_int,
    pub serial: c_ulong,
    pub send_event: c_int,
    pub display: *mut Display,
    pub requestor: Window,
    pub selection: Atom,
    pub target: Atom,
    pub property: Atom,
    pub time: Time,
}

/// What the error handler is handed. The field order is `Xlib.h`'s, and the
/// resource id comes *before* the serial — the opposite way round from the
/// event structures above.
///
/// Needed because the requests that matter here have no return value to check:
/// `XGrabKey` reports a combination another client already holds as a
/// `BadAccess` error, delivered to the handler after the fact.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct XErrorEvent {
    pub type_: c_int,
    pub display: *mut Display,
    pub resourceid: c_ulong,
    pub serial: c_ulong,
    pub error_code: u8,
    pub request_code: u8,
    pub minor_code: u8,
}

// --- Xlib constants ---------------------------------------------------------

pub const KeyPress: c_int = 2;
pub const ButtonPress: c_int = 4;
pub const ButtonRelease: c_int = 5;
pub const MotionNotify: c_int = 6;
pub const LeaveNotify: c_int = 8;
pub const Expose: c_int = 12;
pub const ConfigureNotify: c_int = 22;
pub const SelectionRequest: c_int = 30;

pub const ExposureMask: c_long = 1 << 15;
pub const ButtonPressMask: c_long = 1 << 2;
pub const ButtonReleaseMask: c_long = 1 << 3;
pub const PointerMotionMask: c_long = 1 << 6;
pub const LeaveWindowMask: c_long = 1 << 5;
pub const StructureNotifyMask: c_long = 1 << 17;

pub const CWBackPixel: c_ulong = 1 << 1;
pub const CWOverrideRedirect: c_ulong = 1 << 9;
pub const CWBorderPixel: c_ulong = 1 << 3;
pub const CWEventMask: c_ulong = 1 << 11;
pub const CWColormap: c_ulong = 1 << 13;

pub const InputOutput: c_uint = 1;
pub const AllocNone: c_int = 0;
pub const TrueColor: c_int = 4;

pub const PropModeReplace: c_int = 0;
pub const XA_ATOM: Atom = 4;
pub const XA_CARDINAL: Atom = 6;
pub const XA_STRING: Atom = 31;

pub const GrabModeAsync: c_int = 1;
/// What `XGrabPointer` returns when the grab was actually granted. Every other
/// value means another client holds one.
pub const GrabSuccess: c_int = 0;
/// The error code `XGrabKey` produces for a combination another client has
/// already grabbed.
pub const BadAccess: u8 = 10;
pub const CurrentTime: Time = 0;
pub const NoSymbol: KeySym = 0;

/// X11 modifier masks. `Mod1` is Alt and `Mod4` is Super on every desktop
/// this century; the mapping is configurable in principle and nobody changes
/// it.
pub const ShiftMask: c_uint = 1 << 0;
pub const LockMask: c_uint = 1 << 1;
pub const ControlMask: c_uint = 1 << 2;
pub const Mod1Mask: c_uint = 1 << 3;
pub const Mod2Mask: c_uint = 1 << 4;
pub const Mod4Mask: c_uint = 1 << 6;

/// Cursor shapes from `X11/cursorfont.h`.
pub const XC_left_ptr: c_uint = 68;
pub const XC_sb_h_double_arrow: c_uint = 108;
pub const XC_sb_v_double_arrow: c_uint = 116;
pub const XC_top_left_corner: c_uint = 134;
pub const XC_top_right_corner: c_uint = 136;

// --- Cairo and Pango types --------------------------------------------------

pub type cairo_t = c_void;
pub type cairo_surface_t = c_void;
pub type cairo_pattern_t = c_void;
pub type PangoLayout = c_void;
pub type PangoContext = c_void;
pub type PangoFontDescription = c_void;

/// Pango measures in 1/1024 of a device unit.
pub const PANGO_SCALE: f64 = 1024.0;
/// `PANGO_ELLIPSIZE_END`.
pub const PANGO_ELLIPSIZE_END: c_int = 3;
/// `PANGO_ALIGN_LEFT` / `CENTER` / `RIGHT`.
pub const PANGO_ALIGN_LEFT: c_int = 0;
pub const PANGO_ALIGN_CENTER: c_int = 1;
pub const PANGO_ALIGN_RIGHT: c_int = 2;
/// `PANGO_WRAP_WORD_CHAR`: break at a word, and inside one if a single word is
/// wider than the box.
pub const PANGO_WRAP_WORD_CHAR: c_int = 2;
/// `PANGO_DIRECTION_LTR` / `RTL`.
pub const PANGO_DIRECTION_LTR: c_int = 0;
pub const PANGO_DIRECTION_RTL: c_int = 1;
/// `CAIRO_OPERATOR_SOURCE`, which replaces rather than blends — how the
/// surface is cleared to nothing at the start of a frame.
pub const CAIRO_OPERATOR_SOURCE: c_int = 1;
/// `CAIRO_FORMAT_ARGB32`, which is byte for byte `WL_SHM_FORMAT_ARGB8888` on
/// a little-endian machine: premultiplied alpha, thirty-two bits, blue in the
/// low byte. That equivalence is what lets Cairo draw straight into a Wayland
/// buffer with no conversion step in between.
pub const CAIRO_FORMAT_ARGB32: c_int = 0;

pub const CAIRO_LINE_CAP_BUTT: c_int = 0;
pub const CAIRO_LINE_CAP_ROUND: c_int = 1;
pub const CAIRO_LINE_JOIN_ROUND: c_int = 1;

/// Everything resolved out of the three libraries.
///
/// One struct rather than a global per symbol: it is built once, it either
/// succeeds completely or not at all, and the front end holds it for as long
/// as the window lives.
#[allow(non_snake_case)]
pub struct Libs {
    // Xlib
    pub XOpenDisplay: unsafe extern "C" fn(*const c_char) -> *mut Display,
    pub XCloseDisplay: unsafe extern "C" fn(*mut Display) -> c_int,
    pub XDefaultScreen: unsafe extern "C" fn(*mut Display) -> c_int,
    pub XRootWindow: unsafe extern "C" fn(*mut Display, c_int) -> Window,
    pub XDisplayWidth: unsafe extern "C" fn(*mut Display, c_int) -> c_int,
    pub XDisplayHeight: unsafe extern "C" fn(*mut Display, c_int) -> c_int,
    pub XMatchVisualInfo:
        unsafe extern "C" fn(*mut Display, c_int, c_int, c_int, *mut XVisualInfo) -> c_int,
    pub XDefaultVisual: unsafe extern "C" fn(*mut Display, c_int) -> VisualPtr,
    pub XDefaultDepth: unsafe extern "C" fn(*mut Display, c_int) -> c_int,
    pub XCreateColormap: unsafe extern "C" fn(*mut Display, Window, VisualPtr, c_int) -> Colormap,
    #[allow(clippy::type_complexity)]
    pub XCreateWindow: unsafe extern "C" fn(
        *mut Display,
        Window,
        c_int,
        c_int,
        c_uint,
        c_uint,
        c_uint,
        c_int,
        c_uint,
        VisualPtr,
        c_ulong,
        *mut XSetWindowAttributes,
    ) -> Window,
    pub XDestroyWindow: unsafe extern "C" fn(*mut Display, Window) -> c_int,
    pub XMapWindow: unsafe extern "C" fn(*mut Display, Window) -> c_int,
    pub XLowerWindow: unsafe extern "C" fn(*mut Display, Window) -> c_int,
    pub XRaiseWindow: unsafe extern "C" fn(*mut Display, Window) -> c_int,
    pub XMoveResizeWindow:
        unsafe extern "C" fn(*mut Display, Window, c_int, c_int, c_uint, c_uint) -> c_int,
    pub XGetWindowAttributes:
        unsafe extern "C" fn(*mut Display, Window, *mut XWindowAttributes) -> c_int,
    pub XTranslateCoordinates: unsafe extern "C" fn(
        *mut Display,
        Window,
        Window,
        c_int,
        c_int,
        *mut c_int,
        *mut c_int,
        *mut Window,
    ) -> c_int,
    pub XInternAtom: unsafe extern "C" fn(*mut Display, *const c_char, c_int) -> Atom,
    #[allow(clippy::type_complexity)]
    pub XChangeProperty: unsafe extern "C" fn(
        *mut Display,
        Window,
        Atom,
        Atom,
        c_int,
        c_int,
        *const u8,
        c_int,
    ) -> c_int,
    pub XNextEvent: unsafe extern "C" fn(*mut Display, *mut XEvent) -> c_int,
    pub XPending: unsafe extern "C" fn(*mut Display) -> c_int,
    pub XFlush: unsafe extern "C" fn(*mut Display) -> c_int,
    pub XSync: unsafe extern "C" fn(*mut Display, c_int) -> c_int,
    pub XConnectionNumber: unsafe extern "C" fn(*mut Display) -> c_int,
    pub XCreateFontCursor: unsafe extern "C" fn(*mut Display, c_uint) -> Cursor,
    pub XDefineCursor: unsafe extern "C" fn(*mut Display, Window, Cursor) -> c_int,
    pub XGrabKey:
        unsafe extern "C" fn(*mut Display, c_int, c_uint, Window, c_int, c_int, c_int) -> c_int,
    pub XUngrabKey: unsafe extern "C" fn(*mut Display, c_int, c_uint, Window) -> c_int,
    pub XKeysymToKeycode: unsafe extern "C" fn(*mut Display, KeySym) -> c_uint,
    pub XStringToKeysym: unsafe extern "C" fn(*const c_char) -> KeySym,
    pub XSetSelectionOwner: unsafe extern "C" fn(*mut Display, Atom, Window, Time) -> c_int,
    pub XSendEvent: unsafe extern "C" fn(*mut Display, Window, c_int, c_long, *mut XEvent) -> c_int,
    pub XSetErrorHandler: unsafe extern "C" fn(*const c_void) -> *const c_void,
    pub XMapRaised: unsafe extern "C" fn(*mut Display, Window) -> c_int,
    #[allow(clippy::type_complexity)]
    pub XGrabPointer: unsafe extern "C" fn(
        *mut Display,
        Window,
        c_int,
        c_uint,
        c_int,
        c_int,
        Window,
        Cursor,
        Time,
    ) -> c_int,
    pub XUngrabPointer: unsafe extern "C" fn(*mut Display, Time) -> c_int,
    #[allow(clippy::type_complexity)]
    pub XQueryPointer: unsafe extern "C" fn(
        *mut Display,
        Window,
        *mut Window,
        *mut Window,
        *mut c_int,
        *mut c_int,
        *mut c_int,
        *mut c_int,
        *mut c_uint,
    ) -> c_int,

    // Cairo
    pub cairo_xlib_surface_create:
        unsafe extern "C" fn(*mut Display, Window, VisualPtr, c_int, c_int) -> *mut cairo_surface_t,
    pub cairo_xlib_surface_set_size: unsafe extern "C" fn(*mut cairo_surface_t, c_int, c_int),
    /// Wraps memory this program owns — a Wayland shared-memory buffer — so
    /// Cairo draws straight into what the compositor will read.
    pub cairo_image_surface_create_for_data:
        unsafe extern "C" fn(*mut u8, c_int, c_int, c_int, c_int) -> *mut cairo_surface_t,
    pub cairo_surface_mark_dirty: unsafe extern "C" fn(*mut cairo_surface_t),
    pub cairo_surface_destroy: unsafe extern "C" fn(*mut cairo_surface_t),
    pub cairo_surface_flush: unsafe extern "C" fn(*mut cairo_surface_t),
    pub cairo_create: unsafe extern "C" fn(*mut cairo_surface_t) -> *mut cairo_t,
    pub cairo_destroy: unsafe extern "C" fn(*mut cairo_t),
    pub cairo_save: unsafe extern "C" fn(*mut cairo_t),
    pub cairo_restore: unsafe extern "C" fn(*mut cairo_t),
    pub cairo_set_source_rgba: unsafe extern "C" fn(*mut cairo_t, f64, f64, f64, f64),
    pub cairo_set_source: unsafe extern "C" fn(*mut cairo_t, *mut cairo_pattern_t),
    pub cairo_set_operator: unsafe extern "C" fn(*mut cairo_t, c_int),
    pub cairo_paint: unsafe extern "C" fn(*mut cairo_t),
    pub cairo_fill: unsafe extern "C" fn(*mut cairo_t),
    pub cairo_stroke: unsafe extern "C" fn(*mut cairo_t),
    pub cairo_clip: unsafe extern "C" fn(*mut cairo_t),
    pub cairo_new_path: unsafe extern "C" fn(*mut cairo_t),
    pub cairo_close_path: unsafe extern "C" fn(*mut cairo_t),
    pub cairo_move_to: unsafe extern "C" fn(*mut cairo_t, f64, f64),
    pub cairo_line_to: unsafe extern "C" fn(*mut cairo_t, f64, f64),
    pub cairo_rectangle: unsafe extern "C" fn(*mut cairo_t, f64, f64, f64, f64),
    pub cairo_arc: unsafe extern "C" fn(*mut cairo_t, f64, f64, f64, f64, f64),
    pub cairo_set_line_width: unsafe extern "C" fn(*mut cairo_t, f64),
    pub cairo_set_line_cap: unsafe extern "C" fn(*mut cairo_t, c_int),
    pub cairo_set_line_join: unsafe extern "C" fn(*mut cairo_t, c_int),
    pub cairo_scale: unsafe extern "C" fn(*mut cairo_t, f64, f64),
    pub cairo_pattern_create_linear:
        unsafe extern "C" fn(f64, f64, f64, f64) -> *mut cairo_pattern_t,
    pub cairo_pattern_add_color_stop_rgba:
        unsafe extern "C" fn(*mut cairo_pattern_t, f64, f64, f64, f64, f64),
    pub cairo_pattern_destroy: unsafe extern "C" fn(*mut cairo_pattern_t),

    // Pango
    pub pango_cairo_create_layout: unsafe extern "C" fn(*mut cairo_t) -> *mut PangoLayout,
    pub pango_cairo_show_layout: unsafe extern "C" fn(*mut cairo_t, *mut PangoLayout),
    pub pango_layout_set_text: unsafe extern "C" fn(*mut PangoLayout, *const c_char, c_int),
    pub pango_layout_set_font_description:
        unsafe extern "C" fn(*mut PangoLayout, *mut PangoFontDescription),
    pub pango_layout_set_width: unsafe extern "C" fn(*mut PangoLayout, c_int),
    pub pango_layout_set_ellipsize: unsafe extern "C" fn(*mut PangoLayout, c_int),
    pub pango_layout_set_alignment: unsafe extern "C" fn(*mut PangoLayout, c_int),
    pub pango_layout_set_wrap: unsafe extern "C" fn(*mut PangoLayout, c_int),
    pub pango_layout_set_single_paragraph_mode: unsafe extern "C" fn(*mut PangoLayout, c_int),
    pub pango_layout_get_pixel_size: unsafe extern "C" fn(*mut PangoLayout, *mut c_int, *mut c_int),
    pub pango_layout_get_context: unsafe extern "C" fn(*mut PangoLayout) -> *mut PangoContext,
    pub pango_context_set_base_dir: unsafe extern "C" fn(*mut PangoContext, c_int),
    pub pango_font_description_from_string:
        unsafe extern "C" fn(*const c_char) -> *mut PangoFontDescription,
    pub pango_font_description_free: unsafe extern "C" fn(*mut PangoFontDescription),
    pub g_object_unref: unsafe extern "C" fn(*mut c_void),

    /// Kept alive: unloading any of them would invalidate every pointer above.
    _libraries: Vec<Library>,
}

impl Libs {
    /// Opens the libraries and resolves every symbol, or says what is missing.
    pub fn load() -> Option<Self> {
        let x11 = Library::open(&["libX11.so.6", "libX11.so"])?;
        let cairo = Library::open(&["libcairo.so.2", "libcairo.so"])?;
        let pango = Library::open(&["libpango-1.0.so.0"])?;
        let pangocairo = Library::open(&["libpangocairo-1.0.so.0"])?;
        let gobject = Library::open(&["libgobject-2.0.so.0"])?;

        // Every one of these is an `unsafe` promise that the signature above
        // matches the C header, checked by reading rather than by the
        // compiler. `?` means one missing symbol takes the whole front end
        // down to the printed agenda, which is the honest outcome: a widget
        // with no way to draw text is not a widget.
        unsafe {
            Some(Self {
                XOpenDisplay: x11.symbol(c"XOpenDisplay")?,
                XCloseDisplay: x11.symbol(c"XCloseDisplay")?,
                XDefaultScreen: x11.symbol(c"XDefaultScreen")?,
                XRootWindow: x11.symbol(c"XRootWindow")?,
                XDisplayWidth: x11.symbol(c"XDisplayWidth")?,
                XDisplayHeight: x11.symbol(c"XDisplayHeight")?,
                XMatchVisualInfo: x11.symbol(c"XMatchVisualInfo")?,
                XDefaultVisual: x11.symbol(c"XDefaultVisual")?,
                XDefaultDepth: x11.symbol(c"XDefaultDepth")?,
                XCreateColormap: x11.symbol(c"XCreateColormap")?,
                XCreateWindow: x11.symbol(c"XCreateWindow")?,
                XDestroyWindow: x11.symbol(c"XDestroyWindow")?,
                XMapWindow: x11.symbol(c"XMapWindow")?,
                XLowerWindow: x11.symbol(c"XLowerWindow")?,
                XRaiseWindow: x11.symbol(c"XRaiseWindow")?,
                XMoveResizeWindow: x11.symbol(c"XMoveResizeWindow")?,
                XGetWindowAttributes: x11.symbol(c"XGetWindowAttributes")?,
                XTranslateCoordinates: x11.symbol(c"XTranslateCoordinates")?,
                XInternAtom: x11.symbol(c"XInternAtom")?,
                XChangeProperty: x11.symbol(c"XChangeProperty")?,
                XNextEvent: x11.symbol(c"XNextEvent")?,
                XPending: x11.symbol(c"XPending")?,
                XFlush: x11.symbol(c"XFlush")?,
                XSync: x11.symbol(c"XSync")?,
                XConnectionNumber: x11.symbol(c"XConnectionNumber")?,
                XCreateFontCursor: x11.symbol(c"XCreateFontCursor")?,
                XDefineCursor: x11.symbol(c"XDefineCursor")?,
                XGrabKey: x11.symbol(c"XGrabKey")?,
                XUngrabKey: x11.symbol(c"XUngrabKey")?,
                XKeysymToKeycode: x11.symbol(c"XKeysymToKeycode")?,
                XStringToKeysym: x11.symbol(c"XStringToKeysym")?,
                XSetSelectionOwner: x11.symbol(c"XSetSelectionOwner")?,
                XSendEvent: x11.symbol(c"XSendEvent")?,
                XSetErrorHandler: x11.symbol(c"XSetErrorHandler")?,
                XMapRaised: x11.symbol(c"XMapRaised")?,
                XGrabPointer: x11.symbol(c"XGrabPointer")?,
                XUngrabPointer: x11.symbol(c"XUngrabPointer")?,
                XQueryPointer: x11.symbol(c"XQueryPointer")?,

                cairo_xlib_surface_create: cairo.symbol(c"cairo_xlib_surface_create")?,
                cairo_xlib_surface_set_size: cairo.symbol(c"cairo_xlib_surface_set_size")?,
                cairo_image_surface_create_for_data: cairo
                    .symbol(c"cairo_image_surface_create_for_data")?,
                cairo_surface_mark_dirty: cairo.symbol(c"cairo_surface_mark_dirty")?,
                cairo_surface_destroy: cairo.symbol(c"cairo_surface_destroy")?,
                cairo_surface_flush: cairo.symbol(c"cairo_surface_flush")?,
                cairo_create: cairo.symbol(c"cairo_create")?,
                cairo_destroy: cairo.symbol(c"cairo_destroy")?,
                cairo_save: cairo.symbol(c"cairo_save")?,
                cairo_restore: cairo.symbol(c"cairo_restore")?,
                cairo_set_source_rgba: cairo.symbol(c"cairo_set_source_rgba")?,
                cairo_set_source: cairo.symbol(c"cairo_set_source")?,
                cairo_set_operator: cairo.symbol(c"cairo_set_operator")?,
                cairo_paint: cairo.symbol(c"cairo_paint")?,
                cairo_fill: cairo.symbol(c"cairo_fill")?,
                cairo_stroke: cairo.symbol(c"cairo_stroke")?,
                cairo_clip: cairo.symbol(c"cairo_clip")?,
                cairo_new_path: cairo.symbol(c"cairo_new_path")?,
                cairo_close_path: cairo.symbol(c"cairo_close_path")?,
                cairo_move_to: cairo.symbol(c"cairo_move_to")?,
                cairo_line_to: cairo.symbol(c"cairo_line_to")?,
                cairo_rectangle: cairo.symbol(c"cairo_rectangle")?,
                cairo_arc: cairo.symbol(c"cairo_arc")?,
                cairo_set_line_width: cairo.symbol(c"cairo_set_line_width")?,
                cairo_set_line_cap: cairo.symbol(c"cairo_set_line_cap")?,
                cairo_set_line_join: cairo.symbol(c"cairo_set_line_join")?,
                cairo_scale: cairo.symbol(c"cairo_scale")?,
                cairo_pattern_create_linear: cairo.symbol(c"cairo_pattern_create_linear")?,
                cairo_pattern_add_color_stop_rgba: cairo
                    .symbol(c"cairo_pattern_add_color_stop_rgba")?,
                cairo_pattern_destroy: cairo.symbol(c"cairo_pattern_destroy")?,

                pango_cairo_create_layout: pangocairo.symbol(c"pango_cairo_create_layout")?,
                pango_cairo_show_layout: pangocairo.symbol(c"pango_cairo_show_layout")?,
                pango_layout_set_text: pango.symbol(c"pango_layout_set_text")?,
                pango_layout_set_font_description: pango
                    .symbol(c"pango_layout_set_font_description")?,
                pango_layout_set_width: pango.symbol(c"pango_layout_set_width")?,
                pango_layout_set_ellipsize: pango.symbol(c"pango_layout_set_ellipsize")?,
                pango_layout_set_alignment: pango.symbol(c"pango_layout_set_alignment")?,
                pango_layout_set_wrap: pango.symbol(c"pango_layout_set_wrap")?,
                pango_layout_set_single_paragraph_mode: pango
                    .symbol(c"pango_layout_set_single_paragraph_mode")?,
                pango_layout_get_pixel_size: pango.symbol(c"pango_layout_get_pixel_size")?,
                pango_layout_get_context: pango.symbol(c"pango_layout_get_context")?,
                pango_context_set_base_dir: pango.symbol(c"pango_context_set_base_dir")?,
                pango_font_description_from_string: pango
                    .symbol(c"pango_font_description_from_string")?,
                pango_font_description_free: pango.symbol(c"pango_font_description_free")?,
                g_object_unref: gobject.symbol(c"g_object_unref")?,

                _libraries: vec![x11, cairo, pango, pangocairo, gobject],
            })
        }
    }
}
