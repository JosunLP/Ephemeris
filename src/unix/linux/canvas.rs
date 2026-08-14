// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Ephemeris contributors
//! [`Canvas`] on Cairo, with Pango for the type.
//!
//! The surface is an Xlib one wrapped straight around the window, so Cairo
//! draws into it and the X server composites — no intermediate image and no
//! copy per frame. Alpha works when the window has a 32-bit visual and a
//! compositing manager is running, which is the ordinary case on every current
//! desktop; without one the panel simply comes out opaque, which is what the
//! `"backdrop"` and transparency settings already have to cope with.
//!
//! Cairo's coordinates already have y growing downwards, so they match
//! [`ephemeris_core::layout`] with no conversion.
//!
//! Text goes through Pango rather than `cairo_show_text` — see the note in
//! [`super::ffi`]. Every line is one `PangoLayout`, built and thrown away; the
//! measurement is cached, because that is what the column widths ask for many
//! times per frame and what costs.

use crate::paint::canvas::{Align, Canvas, FONTS, Font, Stop};
use crate::unix::linux::ffi::*;
use ephemeris_core::layout::Rect;
use ephemeris_core::theme::{self, Appearance, Metrics, Palette};
use std::collections::HashMap;
use std::f64::consts::PI;
use std::ffi::{CString, c_int};
use std::sync::Arc;

/// What the surface draws into, which decides what `resize` and `present`
/// can do.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Backing {
    /// An X11 window: the server composites, and the surface follows the
    /// window's size.
    Xlib,
    /// Memory this program owns: a fixed size, and the pixels have to be
    /// marked dirty so Cairo's cache does not hold a stale copy of them.
    Image,
}

pub struct Cairo {
    libs: Arc<Libs>,
    surface: *mut cairo_surface_t,
    backing: Backing,
    /// Rebuilt for every frame: a `cairo_t` carries the whole graphics state,
    /// and a fresh one is the cheapest way to be sure nothing has leaked from
    /// the last frame.
    cr: *mut cairo_t,
    /// One font description per role, as Pango's own string form.
    fonts: Vec<*mut PangoFontDescription>,
    widths: HashMap<(u64, u32), f32>,
    rtl: bool,
    clips: u32,
    size: (i32, i32),
    /// Device pixels per logical pixel — one everywhere except a Wayland
    /// buffer on a high-density display.
    scale: f64,
}

/// Everything a canvas needs to know about how the widget should look.
///
/// Passed as one value because three of the four always travel together and
/// the fourth — the reading direction — decides how the others are used. It is
/// also what the drawn menu needs, and what a shell keeps in step with the
/// widget at every frame.
#[derive(Clone)]
pub struct Look {
    pub metrics: Metrics,
    pub palette: Palette,
    pub appearance: Appearance,
    pub rtl: bool,
}

impl Cairo {
    /// Wraps an X11 window: the server composites, so there is no intermediate
    /// image and no copy per frame.
    pub fn for_window(
        libs: Arc<Libs>,
        display: *mut Display,
        window: Window,
        visual: VisualPtr,
        size: (i32, i32),
        look: &Look,
    ) -> Option<Self> {
        let surface = unsafe {
            (libs.cairo_xlib_surface_create)(display, window, visual, size.0.max(1), size.1.max(1))
        };
        Self::wrap(libs, surface, size, look, Backing::Xlib)
    }

    /// Wraps memory this program owns — a Wayland shared-memory buffer.
    ///
    /// # Safety
    ///
    /// `pixels` must point at `stride * height` writable bytes that outlive
    /// this canvas, and nothing else may read or write them while it does. On
    /// Wayland that means the compositor has released the buffer.
    pub unsafe fn for_pixels(
        libs: Arc<Libs>,
        pixels: *mut u8,
        size: (i32, i32),
        stride: i32,
        look: &Look,
    ) -> Option<Self> {
        let surface = unsafe {
            (libs.cairo_image_surface_create_for_data)(
                pixels,
                CAIRO_FORMAT_ARGB32,
                size.0.max(1),
                size.1.max(1),
                stride,
            )
        };
        Self::wrap(libs, surface, size, look, Backing::Image)
    }

    /// Builds the font for each role and takes ownership of the surface.
    ///
    /// The families come from Pango's own description syntax — `"Sans 12"` —
    /// which is what makes `"Sans"` mean whatever the user's fontconfig
    /// settings say it means. Naming a specific face would work on the machine
    /// it was written on and nowhere else.
    fn wrap(
        libs: Arc<Libs>,
        surface: *mut cairo_surface_t,
        size: (i32, i32),
        look: &Look,
        backing: Backing,
    ) -> Option<Self> {
        if surface.is_null() {
            return None;
        }
        let family = look.appearance.font_family().unwrap_or("Sans");
        let bold = look.appearance.header_weight().0 >= 600;
        let m = look.metrics;

        let mut fonts = Vec::with_capacity(FONTS.len());
        for role in FONTS {
            let (size, heavy) = match role {
                Font::Title => (m.fs_title, true),
                Font::Clock => (m.fs_clock, false),
                Font::Subtitle => (m.fs_subtitle, false),
                Font::Section => (m.fs_section, bold),
                Font::Row => (m.fs_row, false),
                Font::RowStrong => (m.fs_row, true),
                Font::Meta => (m.fs_meta, false),
                Font::Footer => (m.fs_footer, false),
                Font::Tooltip => (m.fs_row, false),
            };
            // Pango takes the size in points, and the metrics are in the same
            // 1/96-inch units the Windows front end uses, so the conversion is
            // the ratio between them. Getting this wrong is a widget a third
            // too large, which is exactly what it looked like without it.
            let points = size as f64 * 72.0 / 96.0;
            let weight = if heavy { " Semi-Bold" } else { "" };
            let description = CString::new(format!("{family}{weight} {points:.1}")).ok()?;
            fonts.push(unsafe { (libs.pango_font_description_from_string)(description.as_ptr()) });
        }

        Some(Self {
            libs,
            surface,
            backing,
            cr: std::ptr::null_mut(),
            fonts,
            widths: HashMap::new(),
            rtl: look.rtl,
            clips: 0,
            size,
            scale: 1.0,
        })
    }

    /// Follows the window's new size.
    ///
    /// Only an Xlib surface can: an image surface is bound to the memory it
    /// was handed, so the Wayland side builds a new canvas around a new buffer
    /// instead.
    pub fn resize(&mut self, width: i32, height: i32) {
        if (width, height) == self.size || self.backing != Backing::Xlib {
            return;
        }
        self.size = (width, height);
        unsafe {
            (self.libs.cairo_xlib_surface_set_size)(self.surface, width.max(1), height.max(1));
        }
    }

    /// Draws everything this much larger than it is asked for.
    ///
    /// The widget's coordinates are logical pixels; a Wayland buffer on a
    /// high-density display holds `scale` times as many. Applying it here
    /// rather than at every call site is what keeps the drawing code unaware
    /// that such displays exist.
    pub fn set_scale(&mut self, scale: f64) {
        self.scale = scale.max(0.1);
    }

    /// Opens a drawing context for one frame.
    pub fn begin(&mut self) -> bool {
        if !self.cr.is_null() {
            return true;
        }
        self.cr = unsafe { (self.libs.cairo_create)(self.surface) };
        self.clips = 0;
        if self.cr.is_null() {
            return false;
        }
        if self.scale != 1.0 {
            unsafe { (self.libs.cairo_scale)(self.cr, self.scale, self.scale) };
        }
        true
    }

    /// A layout for one line, ready to draw or measure.
    fn layout(&self, s: &str, font: Font, single_line: bool) -> Option<Layout> {
        if self.cr.is_null() {
            return None;
        }
        let text = CString::new(s.replace('\0', " ")).ok()?;
        unsafe {
            let raw = (self.libs.pango_cairo_create_layout)(self.cr);
            if raw.is_null() {
                return None;
            }
            (self.libs.pango_layout_set_font_description)(raw, self.fonts[font as usize]);
            (self.libs.pango_layout_set_text)(raw, text.as_ptr(), -1);
            (self.libs.pango_layout_set_single_paragraph_mode)(raw, single_line as c_int);
            // The base direction decides where a line starts and how a mixed
            // run is reordered. It matters even for Latin text: running under
            // `ar-SA` with no Arabic catalogue gives English words in a
            // mirrored layout, and without this its trailing punctuation lands
            // on the wrong side.
            let context = (self.libs.pango_layout_get_context)(raw);
            if !context.is_null() {
                (self.libs.pango_context_set_base_dir)(
                    context,
                    if self.rtl {
                        PANGO_DIRECTION_RTL
                    } else {
                        PANGO_DIRECTION_LTR
                    },
                );
            }
            Some(Layout {
                libs: self.libs.clone(),
                raw,
            })
        }
    }

    fn set_color(&self, color: u32, alpha: f32) {
        let c = theme::rgba(color, alpha);
        unsafe {
            (self.libs.cairo_set_source_rgba)(
                self.cr, c.r as f64, c.g as f64, c.b as f64, c.a as f64,
            );
        }
    }

    /// A rounded rectangle as the current path.
    fn round_rect_path(&self, r: Rect, radius: f32) {
        let l = &self.libs;
        let radius = radius.min(r.width() * 0.5).min(r.height() * 0.5).max(0.0) as f64;
        let (x0, y0, x1, y1) = (r.left as f64, r.top as f64, r.right as f64, r.bottom as f64);
        unsafe {
            (l.cairo_new_path)(self.cr);
            // Four quarter arcs joined by the straight sides. Cairo's `arc`
            // sweeps in increasing angle, which with y downwards is clockwise
            // on screen, so the corners run top-right, bottom-right,
            // bottom-left, top-left.
            (l.cairo_arc)(self.cr, x1 - radius, y0 + radius, radius, -PI / 2.0, 0.0);
            (l.cairo_arc)(self.cr, x1 - radius, y1 - radius, radius, 0.0, PI / 2.0);
            (l.cairo_arc)(self.cr, x0 + radius, y1 - radius, radius, PI / 2.0, PI);
            (l.cairo_arc)(
                self.cr,
                x0 + radius,
                y0 + radius,
                radius,
                PI,
                3.0 * PI / 2.0,
            );
            (l.cairo_close_path)(self.cr);
        }
    }
}

impl Canvas for Cairo {
    fn clear(&mut self) {
        let l = &self.libs;
        unsafe {
            (l.cairo_save)(self.cr);
            // `SOURCE` replaces rather than blends, which is what makes the
            // window transparent again instead of laying nothing over the
            // previous frame.
            (l.cairo_set_operator)(self.cr, CAIRO_OPERATOR_SOURCE);
            (l.cairo_set_source_rgba)(self.cr, 0.0, 0.0, 0.0, 0.0);
            (l.cairo_paint)(self.cr);
            (l.cairo_restore)(self.cr);
        }
    }

    fn fill_round(&mut self, r: Rect, radius: f32, color: u32, alpha: f32) {
        self.round_rect_path(r, radius);
        self.set_color(color, alpha);
        unsafe { (self.libs.cairo_fill)(self.cr) };
    }

    fn stroke_round(&mut self, r: Rect, radius: f32, color: u32, alpha: f32, width: f32) {
        self.round_rect_path(r, radius);
        self.set_color(color, alpha);
        unsafe {
            (self.libs.cairo_set_line_width)(self.cr, width as f64);
            (self.libs.cairo_set_line_join)(self.cr, CAIRO_LINE_JOIN_ROUND);
            (self.libs.cairo_stroke)(self.cr);
        }
    }

    fn fill_gradient(&mut self, r: Rect, radius: f32, stops: &[Stop]) {
        let l = &self.libs;
        unsafe {
            let pattern = (l.cairo_pattern_create_linear)(
                r.left as f64,
                r.top as f64,
                r.left as f64,
                r.bottom as f64,
            );
            if pattern.is_null() {
                return;
            }
            for s in stops {
                let c = theme::rgba(s.color, s.alpha);
                (l.cairo_pattern_add_color_stop_rgba)(
                    pattern,
                    s.at as f64,
                    c.r as f64,
                    c.g as f64,
                    c.b as f64,
                    c.a as f64,
                );
            }
            self.round_rect_path(r, radius);
            (l.cairo_set_source)(self.cr, pattern);
            (l.cairo_fill)(self.cr);
            (l.cairo_pattern_destroy)(pattern);
        }
    }

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
        let l = &self.libs;
        self.set_color(color, alpha);
        unsafe {
            (l.cairo_set_line_width)(self.cr, width as f64);
            (l.cairo_set_line_cap)(
                self.cr,
                if round_caps {
                    CAIRO_LINE_CAP_ROUND
                } else {
                    CAIRO_LINE_CAP_BUTT
                },
            );
            (l.cairo_new_path)(self.cr);
            (l.cairo_move_to)(self.cr, x0 as f64, y0 as f64);
            (l.cairo_line_to)(self.cr, x1 as f64, y1 as f64);
            (l.cairo_stroke)(self.cr);
        }
    }

    fn circle(&mut self, cx: f32, cy: f32, r: f32, color: u32, alpha: f32, stroke: Option<f32>) {
        let l = &self.libs;
        self.set_color(color, alpha);
        unsafe {
            (l.cairo_new_path)(self.cr);
            (l.cairo_arc)(
                self.cr,
                cx as f64,
                cy as f64,
                r.max(0.0) as f64,
                0.0,
                2.0 * PI,
            );
            match stroke {
                Some(width) => {
                    (l.cairo_set_line_width)(self.cr, width as f64);
                    (l.cairo_stroke)(self.cr);
                }
                None => (l.cairo_fill)(self.cr),
            }
        }
    }

    fn arc(
        &mut self,
        cx: f32,
        cy: f32,
        r: f32,
        from: f32,
        to: f32,
        color: u32,
        alpha: f32,
        width: f32,
    ) {
        let l = &self.libs;
        self.set_color(color, alpha);
        unsafe {
            (l.cairo_set_line_width)(self.cr, width as f64);
            (l.cairo_set_line_cap)(self.cr, CAIRO_LINE_CAP_ROUND);
            (l.cairo_new_path)(self.cr);
            (l.cairo_arc)(
                self.cr,
                cx as f64,
                cy as f64,
                r.max(0.0) as f64,
                from as f64,
                to as f64,
            );
            (l.cairo_stroke)(self.cr);
        }
    }

    fn polygon(&mut self, points: &[(f32, f32)], color: u32, alpha: f32) {
        let Some((first, rest)) = points.split_first() else {
            return;
        };
        let l = &self.libs;
        self.set_color(color, alpha);
        unsafe {
            (l.cairo_new_path)(self.cr);
            (l.cairo_move_to)(self.cr, first.0 as f64, first.1 as f64);
            for (x, y) in rest {
                (l.cairo_line_to)(self.cr, *x as f64, *y as f64);
            }
            (l.cairo_close_path)(self.cr);
            (l.cairo_fill)(self.cr);
        }
    }

    fn push_clip(&mut self, r: Rect) {
        let l = &self.libs;
        unsafe {
            (l.cairo_save)(self.cr);
            (l.cairo_new_path)(self.cr);
            (l.cairo_rectangle)(
                self.cr,
                r.left as f64,
                r.top as f64,
                r.width() as f64,
                r.height() as f64,
            );
            (l.cairo_clip)(self.cr);
        }
        self.clips += 1;
    }

    fn pop_clip(&mut self) {
        if self.clips == 0 {
            return;
        }
        self.clips -= 1;
        unsafe { (self.libs.cairo_restore)(self.cr) };
    }

    fn text(&mut self, s: &str, font: Font, align: Align, r: Rect, color: u32, alpha: f32) {
        let Some(layout) = self.layout(s, font, true) else {
            return;
        };
        let l = &self.libs;
        let width = r.width().max(0.0);
        unsafe {
            // Pango ellipsises within the width it is given, which is what
            // makes a title *look* cut off — the tooltip that offers the rest
            // has nothing to hint at otherwise.
            (l.pango_layout_set_width)(layout.raw, (width as f64 * PANGO_SCALE) as c_int);
            (l.pango_layout_set_ellipsize)(layout.raw, PANGO_ELLIPSIZE_END);
            (l.pango_layout_set_alignment)(
                layout.raw,
                match align {
                    Align::Left => PANGO_ALIGN_LEFT,
                    Align::Center => PANGO_ALIGN_CENTER,
                    Align::Right => PANGO_ALIGN_RIGHT,
                },
            );
            let (_, height) = layout.pixel_size();
            // Vertically centred in the box, which is what every caller wants
            // and what the layout module's row rectangles assume.
            let y = r.center_y() - height * 0.5;
            self.set_color(color, alpha);
            (l.cairo_move_to)(self.cr, r.left as f64, y as f64);
            (l.pango_cairo_show_layout)(self.cr, layout.raw);
        }
    }

    fn text_block(&mut self, s: &str, r: Rect, color: u32, alpha: f32) {
        let Some(layout) = self.layout(s, Font::Tooltip, false) else {
            return;
        };
        let l = &self.libs;
        unsafe {
            (l.pango_layout_set_width)(layout.raw, (r.width() as f64 * PANGO_SCALE) as c_int);
            (l.pango_layout_set_wrap)(layout.raw, PANGO_WRAP_WORD_CHAR);
            (l.pango_layout_set_alignment)(
                layout.raw,
                if self.rtl {
                    PANGO_ALIGN_RIGHT
                } else {
                    PANGO_ALIGN_LEFT
                },
            );
            self.set_color(color, alpha);
            (l.cairo_move_to)(self.cr, r.left as f64, r.top as f64);
            (l.pango_cairo_show_layout)(self.cr, layout.raw);
        }
    }

    fn text_width(&mut self, s: &str, font: Font) -> f32 {
        if s.is_empty() {
            return 0.0;
        }
        let key = (hash(s), font as u32);
        if let Some(w) = self.widths.get(&key) {
            return *w;
        }
        let width = match self.layout(s, font, true) {
            Some(layout) => {
                // Unconstrained, so the answer is the text's own width rather
                // than the box it was last drawn in.
                unsafe { (self.libs.pango_layout_set_width)(layout.raw, -1) };
                layout.pixel_size().0
            }
            None => 0.0,
        };
        self.widths.insert(key, width);
        width
    }

    fn text_block_height(&mut self, s: &str, width: f32) -> f32 {
        let Some(layout) = self.layout(s, Font::Tooltip, false) else {
            return 0.0;
        };
        unsafe {
            (self.libs.pango_layout_set_width)(layout.raw, (width as f64 * PANGO_SCALE) as c_int);
            (self.libs.pango_layout_set_wrap)(layout.raw, PANGO_WRAP_WORD_CHAR);
        }
        layout.pixel_size().1
    }

    /// Hands the finished frame to the X server and closes the context.
    fn present(&mut self) {
        let l = &self.libs;
        unsafe {
            while self.clips > 0 {
                self.clips -= 1;
                (l.cairo_restore)(self.cr);
            }
            (l.cairo_destroy)(self.cr);
            self.cr = std::ptr::null_mut();
            (l.cairo_surface_flush)(self.surface);
            // Cairo keeps its own idea of what an image surface holds. The
            // Wayland side hands the same memory to the compositor, so the
            // cache has to be told the pixels are the authority.
            if self.backing == Backing::Image {
                (l.cairo_surface_mark_dirty)(self.surface);
            }
        }
        if self.widths.len() > 512 {
            // The relative times ("in 25 min") produce a new key every minute.
            self.widths.clear();
        }
    }
}

impl Drop for Cairo {
    fn drop(&mut self) {
        unsafe {
            if !self.cr.is_null() {
                (self.libs.cairo_destroy)(self.cr);
            }
            for font in self.fonts.drain(..) {
                if !font.is_null() {
                    (self.libs.pango_font_description_free)(font);
                }
            }
            (self.libs.cairo_surface_destroy)(self.surface);
        }
    }
}

/// A `PangoLayout` this code owns.
struct Layout {
    libs: Arc<Libs>,
    raw: *mut PangoLayout,
}

impl Layout {
    fn pixel_size(&self) -> (f32, f32) {
        let (mut w, mut h) = (0, 0);
        unsafe { (self.libs.pango_layout_get_pixel_size)(self.raw, &mut w, &mut h) };
        (w as f32, h as f32)
    }
}

impl Drop for Layout {
    fn drop(&mut self) {
        unsafe { (self.libs.g_object_unref)(self.raw) };
    }
}

fn hash(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish()
}
