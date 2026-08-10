// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! The pointer, which on Wayland the client has to draw.
//!
//! There is no `XDefineCursor` here. A Wayland client is handed a pointer and
//! given nothing to show for it: over a surface that has never called
//! `wl_pointer.set_cursor` the cursor is undefined, and several compositors
//! simply hide it. So even the plain arrow has to be loaded from the user's
//! cursor theme, drawn onto a surface of the widget's own, and handed back.
//!
//! `libwayland-cursor` does the loading. It ships beside `libwayland-client`
//! and reads the XCursor themes every desktop already has, which is why the
//! shape names below are the X11 ones — that is what the themes are keyed by,
//! whatever protocol is asking.

use super::ffi::{Wl, wl_pointer, wl_proxy};
use crate::unix::app::Cursor as AppCursor;
use crate::unix::linux::dl::Library;
use std::ffi::{CStr, CString, c_char, c_int, c_void};

/// `struct wl_cursor` — the animation frames of one shape.
#[repr(C)]
struct WlCursor {
    image_count: u32,
    images: *mut *mut WlCursorImage,
    name: *mut c_char,
}

/// `struct wl_cursor_image` — one frame, and where its point actually is.
#[repr(C)]
struct WlCursorImage {
    width: u32,
    height: u32,
    hotspot_x: u32,
    hotspot_y: u32,
    delay: u32,
}

#[allow(non_snake_case)]
pub struct CursorLib {
    wl_cursor_theme_load: unsafe extern "C" fn(*const c_char, c_int, *mut wl_proxy) -> *mut c_void,
    wl_cursor_theme_destroy: unsafe extern "C" fn(*mut c_void),
    wl_cursor_theme_get_cursor: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut WlCursor,
    wl_cursor_image_get_buffer: unsafe extern "C" fn(*mut WlCursorImage) -> *mut wl_proxy,
    _library: Library,
}

impl CursorLib {
    /// `None` when the library is missing, which is survivable: the widget
    /// then shows whatever the compositor last set, which is usually an arrow.
    pub fn load() -> Option<Self> {
        let lib = Library::open(&["libwayland-cursor.so.0", "libwayland-cursor.so"])?;
        unsafe {
            Some(Self {
                wl_cursor_theme_load: lib.symbol(c"wl_cursor_theme_load")?,
                wl_cursor_theme_destroy: lib.symbol(c"wl_cursor_theme_destroy")?,
                wl_cursor_theme_get_cursor: lib.symbol(c"wl_cursor_theme_get_cursor")?,
                wl_cursor_image_get_buffer: lib.symbol(c"wl_cursor_image_get_buffer")?,
                _library: lib,
            })
        }
    }
}

/// The five shapes the widget asks for, loaded once.
pub struct Cursors {
    theme: *mut c_void,
    /// The surface the cursor image is attached to. One is enough: only one
    /// shape is shown at a time.
    surface: *mut wl_proxy,
    /// Indexed the way [`AppCursor`] is ordered by [`index`].
    images: [*mut WlCursorImage; 5],
    buffers: [*mut wl_proxy; 5],
    shown: usize,
}

impl Default for Cursors {
    fn default() -> Self {
        Self {
            theme: std::ptr::null_mut(),
            surface: std::ptr::null_mut(),
            images: [std::ptr::null_mut(); 5],
            buffers: [std::ptr::null_mut(); 5],
            shown: usize::MAX,
        }
    }
}

/// The slot in [`Cursors::images`] for a shape.
fn index(cursor: AppCursor) -> usize {
    match cursor {
        AppCursor::Arrow => 0,
        AppCursor::SizeNwSe => 1,
        AppCursor::SizeNeSw => 2,
        AppCursor::SizeWe => 3,
        AppCursor::SizeNs => 4,
    }
}

/// The XCursor theme names for those shapes, in the same order.
///
/// `left_ptr` rather than `default`: the older name is the one every theme
/// still carries, and `libwayland-cursor` does no aliasing of its own.
const NAMES: [&CStr; 5] = [
    c"left_ptr",
    c"top_left_corner",
    c"top_right_corner",
    c"sb_h_double_arrow",
    c"sb_v_double_arrow",
];

impl Cursors {
    /// Loads the theme and prepares a surface to show it on.
    ///
    /// The theme and size follow `XCURSOR_THEME` and `XCURSOR_SIZE`, which is
    /// what every desktop sets and what every other Wayland client reads.
    pub fn load(
        &mut self,
        wl: &Wl,
        lib: Option<&CursorLib>,
        shm: *mut wl_proxy,
        compositor: *mut wl_proxy,
        scale: i32,
    ) {
        let Some(lib) = lib else { return };
        if shm.is_null() || compositor.is_null() {
            return;
        }

        let name = std::env::var("XCURSOR_THEME")
            .ok()
            .and_then(|t| CString::new(t).ok());
        let size: c_int = std::env::var("XCURSOR_SIZE")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(24);

        let theme = unsafe {
            (lib.wl_cursor_theme_load)(
                name.as_ref().map_or(std::ptr::null(), |n| n.as_ptr()),
                size * scale.max(1),
                shm,
            )
        };
        if theme.is_null() {
            return;
        }
        self.theme = theme;

        for (slot, shape) in NAMES.iter().enumerate() {
            let cursor = unsafe { (lib.wl_cursor_theme_get_cursor)(theme, shape.as_ptr()) };
            if cursor.is_null() {
                continue;
            }
            // The first frame only. The widget's cursors are all static
            // shapes, and animating one would mean a timer for a spinning
            // wait cursor this program never shows.
            let image = unsafe {
                let c = &*cursor;
                if c.image_count == 0 {
                    continue;
                }
                *c.images
            };
            self.images[slot] = image;
            self.buffers[slot] = unsafe { (lib.wl_cursor_image_get_buffer)(image) };
        }

        self.surface = unsafe {
            (wl.wl_proxy_marshal_flags)(
                compositor,
                super::ffi::wl_compositor::CREATE_SURFACE,
                wl.wl_surface_interface,
                (wl.wl_proxy_get_version)(compositor),
                0,
                std::ptr::null_mut::<c_void>(),
            )
        };
        if !self.surface.is_null() && scale > 1 {
            // The cursor was loaded at the display's density, so the surface
            // has to say the buffer is that much denser too.
            unsafe {
                (wl.wl_proxy_marshal_flags)(
                    self.surface,
                    super::ffi::wl_surface::SET_BUFFER_SCALE,
                    std::ptr::null(),
                    (wl.wl_proxy_get_version)(self.surface),
                    0,
                    scale,
                );
            }
        }
    }

    /// Shows a shape, if it is not the one already showing.
    ///
    /// `serial` has to be the one from the most recent `wl_pointer.enter`;
    /// anything else is rejected by the compositor, which is why the shell
    /// keeps that serial separately from every other one.
    pub fn show(&mut self, wl: &Wl, pointer: *mut wl_proxy, cursor: AppCursor, serial: u32) {
        let slot = index(cursor);
        if pointer.is_null() || self.surface.is_null() || slot == self.shown {
            return;
        }
        // A shape the theme does not carry falls back to the arrow rather than
        // to nothing at all.
        let (image, buffer) = if self.images[slot].is_null() {
            (self.images[0], self.buffers[0])
        } else {
            (self.images[slot], self.buffers[slot])
        };
        if image.is_null() || buffer.is_null() {
            return;
        }
        self.shown = slot;

        let (hotspot_x, hotspot_y, width, height) = unsafe {
            let i = &*image;
            (
                i.hotspot_x as i32,
                i.hotspot_y as i32,
                i.width as i32,
                i.height as i32,
            )
        };
        unsafe {
            let version = (wl.wl_proxy_get_version)(self.surface);
            (wl.wl_proxy_marshal_flags)(
                self.surface,
                super::ffi::wl_surface::ATTACH,
                std::ptr::null(),
                version,
                0,
                buffer,
                0,
                0,
            );
            (wl.wl_proxy_marshal_flags)(
                self.surface,
                super::ffi::wl_surface::DAMAGE,
                std::ptr::null(),
                version,
                0,
                0,
                0,
                width,
                height,
            );
            (wl.wl_proxy_marshal_flags)(
                self.surface,
                super::ffi::wl_surface::COMMIT,
                std::ptr::null(),
                version,
                0,
            );
            // The hotspot is in the cursor surface's own coordinates, which
            // are logical pixels — so it is the *unscaled* value even though
            // the image was loaded at the display's density.
            (wl.wl_proxy_marshal_flags)(
                pointer,
                wl_pointer::SET_CURSOR,
                std::ptr::null(),
                (wl.wl_proxy_get_version)(pointer),
                0,
                serial,
                self.surface,
                hotspot_x,
                hotspot_y,
            );
        }
    }

    /// The pointer left, so the next shape has to be sent again even if it is
    /// the same one — a compositor forgets a client's cursor between visits.
    pub fn forget(&mut self) {
        self.shown = usize::MAX;
    }

    pub fn destroy(&mut self, wl: &Wl, lib: Option<&CursorLib>) {
        if !self.surface.is_null() {
            unsafe { (wl.wl_proxy_destroy)(self.surface) };
            self.surface = std::ptr::null_mut();
        }
        if let Some(lib) = lib
            && !self.theme.is_null()
        {
            unsafe { (lib.wl_cursor_theme_destroy)(self.theme) };
            self.theme = std::ptr::null_mut();
        }
    }
}
