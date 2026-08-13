// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! [`Canvas`] on Direct2D, with DirectWrite for the type.
//!
//! The device chain this draws through is built in [`super::render`]; what is
//! here is only the translation from the trait's primitives to Direct2D's.
//! Almost all of it is one call: a rounded rectangle, a line, an ellipse and a
//! text layout are things Direct2D has. The two that are not — an arc and a
//! filled polygon — are path geometries, and both exist for the refresh icon.
//!
//! Two caches earn their keep at sixty frames a second. Brushes are kept per
//! colour, because a `CreateSolidColorBrush` per fill is a few hundred COM
//! objects a second for colours that never change. Text layouts are kept per
//! (text, role, alignment, box), because laying text out is the single most
//! expensive step in a frame and the words do not change while an animation
//! runs.

use crate::paint::canvas::{Align, Canvas, FONTS, Font, Stop};
use std::cell::RefCell;
use std::collections::HashMap;
use tpmplaner_core::layout::Rect;
use tpmplaner_core::theme::{self, Appearance, Metrics};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D_SIZE_F, D2D1_COLOR_F, D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_BEGIN_HOLLOW,
    D2D1_FIGURE_END_CLOSED, D2D1_FIGURE_END_OPEN, D2D1_GRADIENT_STOP,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_ARC_SEGMENT, D2D1_ARC_SIZE_LARGE, D2D1_ARC_SIZE_SMALL, D2D1_BRUSH_PROPERTIES,
    D2D1_BUFFER_PRECISION_8BPC_UNORM, D2D1_COLOR_INTERPOLATION_MODE_PREMULTIPLIED,
    D2D1_COLOR_SPACE_SRGB, D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_ELLIPSE, D2D1_EXTEND_MODE_CLAMP,
    D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES, D2D1_ROUNDED_RECT, D2D1_SWEEP_DIRECTION_CLOCKWISE,
    ID2D1Brush, ID2D1DeviceContext, ID2D1GradientStopCollection1, ID2D1SolidColorBrush,
    ID2D1StrokeStyle,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_TEXT_ALIGNMENT, DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_LEADING,
    DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_TEXT_METRICS, IDWriteFactory, IDWriteTextFormat,
    IDWriteTextLayout,
};
use windows::core::Interface;
use windows_numerics::Vector2;

/// Everything a frame is drawn with.
///
/// Borrowed from the renderer for the duration of one frame rather than owned,
/// because the device chain outlives any number of frames and the alternative
/// is threading four handles through every call.
pub struct D2dCanvas<'a> {
    pub dc: &'a ID2D1DeviceContext,
    pub dwrite: &'a IDWriteFactory,
    pub formats: &'a [IDWriteTextFormat],
    pub round_stroke: &'a ID2D1StrokeStyle,
    pub brushes: &'a RefCell<HashMap<u32, ID2D1SolidColorBrush>>,
    pub layouts: &'a RefCell<HashMap<LayoutKey, IDWriteTextLayout>>,
}

/// What makes one text layout different from another.
///
/// Width and height belong in it because they decide where the text is cut
/// with an ellipsis and how it is centred; both are quantised to quarter
/// device-independent pixels so rounding noise produces no new entries while a
/// real difference in width still does.
pub type LayoutKey = (u64, u32, u32, u32, u32);

impl D2dCanvas<'_> {
    /// A cached brush. Opacity is set per call, so one brush serves every use
    /// of a colour.
    fn brush(&self, color: u32, alpha: f32) -> Option<ID2D1SolidColorBrush> {
        let mut cache = self.brushes.borrow_mut();
        let brush = match cache.get(&color) {
            Some(b) => b.clone(),
            None => {
                let created = unsafe {
                    self.dc.CreateSolidColorBrush(
                        &rgba(color, 1.0),
                        None::<*const D2D1_BRUSH_PROPERTIES>,
                    )
                };
                let b = created.ok()?;
                cache.insert(color, b.clone());
                b
            }
        };
        unsafe { brush.SetOpacity(alpha.clamp(0.0, 1.0)) };
        Some(brush)
    }

    fn layout(
        &mut self,
        s: &str,
        font: Font,
        align: Align,
        w: f32,
        h: f32,
    ) -> Option<IDWriteTextLayout> {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        s.hash(&mut hasher);
        let key = (
            hasher.finish(),
            font as u32,
            align as u32,
            (w * 4.0).round() as u32,
            (h * 4.0).round() as u32,
        );
        if let Some(l) = self.layouts.borrow().get(&key) {
            return Some(l.clone());
        }

        let utf16: Vec<u16> = s.encode_utf16().collect();
        let layout = unsafe {
            self.dwrite
                .CreateTextLayout(&utf16, &self.formats[font as usize], w, h)
                .ok()?
        };
        // Alignment is per layout rather than per format: the same role is
        // used at both ends of a row, and a format per combination would be
        // twice as many for no gain.
        unsafe { layout.SetTextAlignment(alignment(align)).ok()? };
        self.layouts.borrow_mut().insert(key, layout.clone());
        // The relative times ("in 25 min") produce new keys every minute, so
        // this cannot be allowed to grow without bound.
        if self.layouts.borrow().len() > 512 {
            self.layouts.borrow_mut().clear();
        }
        Some(layout)
    }

    fn stops(&self, stops: &[D2D1_GRADIENT_STOP]) -> Option<ID2D1GradientStopCollection1> {
        // Premultiplied on purpose: the stops are partly transparent, and
        // straight interpolation puts a visible grey haze between two of them.
        unsafe {
            self.dc
                .CreateGradientStopCollection(
                    stops,
                    D2D1_COLOR_SPACE_SRGB,
                    D2D1_COLOR_SPACE_SRGB,
                    D2D1_BUFFER_PRECISION_8BPC_UNORM,
                    D2D1_EXTEND_MODE_CLAMP,
                    D2D1_COLOR_INTERPOLATION_MODE_PREMULTIPLIED,
                )
                .ok()
        }
    }

    /// A path built by the caller, filled or stroked.
    ///
    /// Direct2D has no arc or polygon primitive: both are figures in a
    /// geometry, and a geometry has to be opened, written and closed before it
    /// can be drawn. The two callers below are the only ones that need it.
    fn geometry(
        &self,
        build: impl FnOnce(&windows::Win32::Graphics::Direct2D::ID2D1GeometrySink),
    ) -> Option<windows::Win32::Graphics::Direct2D::ID2D1PathGeometry> {
        unsafe {
            let factory = self.dc.GetFactory().ok()?;
            let path = factory.CreatePathGeometry().ok()?;
            let sink = path.Open().ok()?;
            build(&sink);
            sink.Close().ok()?;
            Some(path)
        }
    }
}

impl Canvas for D2dCanvas<'_> {
    fn clear(&mut self) {
        unsafe { self.dc.Clear(Some(&rgba(0, 0.0))) };
    }

    fn fill_round(&mut self, r: Rect, radius: f32, color: u32, alpha: f32) {
        let Some(brush) = self.brush(color, alpha) else {
            return;
        };
        unsafe { self.dc.FillRoundedRectangle(&rounded(r, radius), &brush) };
    }

    fn stroke_round(&mut self, r: Rect, radius: f32, color: u32, alpha: f32, width: f32) {
        let Some(brush) = self.brush(color, alpha) else {
            return;
        };
        unsafe {
            self.dc
                .DrawRoundedRectangle(&rounded(r, radius), &brush, width, None)
        };
    }

    fn fill_gradient(&mut self, r: Rect, radius: f32, stops: &[Stop]) {
        let d2d: Vec<D2D1_GRADIENT_STOP> = stops
            .iter()
            .map(|s| D2D1_GRADIENT_STOP {
                position: s.at,
                color: rgba(s.color, s.alpha),
            })
            .collect();
        let Some(collection) = self.stops(&d2d) else {
            return;
        };
        unsafe {
            let brush = self.dc.CreateLinearGradientBrush(
                &D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                    startPoint: point(0.0, r.top),
                    endPoint: point(0.0, r.bottom),
                },
                None::<*const D2D1_BRUSH_PROPERTIES>,
                &collection,
            );
            if let Ok(brush) = brush {
                self.dc.FillRoundedRectangle(&rounded(r, radius), &brush);
            }
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
        let Some(brush) = self.brush(color, alpha) else {
            return;
        };
        unsafe {
            self.dc.DrawLine(
                point(x0, y0),
                point(x1, y1),
                &brush,
                width,
                round_caps.then_some(self.round_stroke),
            )
        };
    }

    fn circle(&mut self, cx: f32, cy: f32, r: f32, color: u32, alpha: f32, stroke: Option<f32>) {
        let Some(brush) = self.brush(color, alpha) else {
            return;
        };
        let ellipse = D2D1_ELLIPSE {
            point: point(cx, cy),
            radiusX: r.max(0.0),
            radiusY: r.max(0.0),
        };
        unsafe {
            match stroke {
                Some(width) => self.dc.DrawEllipse(&ellipse, &brush, width, None),
                None => self.dc.FillEllipse(&ellipse, &brush),
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
        let Some(brush) = self.brush(color, alpha) else {
            return;
        };
        let r = r.max(0.0);
        let start = point(cx + r * from.cos(), cy + r * from.sin());
        let end = point(cx + r * to.cos(), cy + r * to.sin());
        // Direct2D describes an arc by its endpoints and asks which of the two
        // ways round is meant. More than half a turn is the large one.
        let large = (to - from).abs() > std::f32::consts::PI;
        let Some(path) = self.geometry(|sink| unsafe {
            sink.BeginFigure(start, D2D1_FIGURE_BEGIN_HOLLOW);
            sink.AddArc(&D2D1_ARC_SEGMENT {
                point: end,
                size: D2D_SIZE_F {
                    width: r,
                    height: r,
                },
                rotationAngle: 0.0,
                sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                arcSize: if large {
                    D2D1_ARC_SIZE_LARGE
                } else {
                    D2D1_ARC_SIZE_SMALL
                },
            });
            sink.EndFigure(D2D1_FIGURE_END_OPEN);
        }) else {
            return;
        };
        unsafe {
            self.dc
                .DrawGeometry(&path, &brush, width, Some(self.round_stroke))
        };
    }

    fn polygon(&mut self, points: &[(f32, f32)], color: u32, alpha: f32) {
        let Some((first, rest)) = points.split_first() else {
            return;
        };
        let Some(brush) = self.brush(color, alpha) else {
            return;
        };
        let rest: Vec<Vector2> = rest.iter().map(|(x, y)| point(*x, *y)).collect();
        let Some(path) = self.geometry(|sink| unsafe {
            sink.BeginFigure(point(first.0, first.1), D2D1_FIGURE_BEGIN_FILLED);
            sink.AddLines(&rest);
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
        }) else {
            return;
        };
        unsafe { self.dc.FillGeometry(&path, &brush, None) };
    }

    fn push_clip(&mut self, r: Rect) {
        unsafe { self.dc.PushAxisAlignedClip(&d2d(r), Default::default()) };
    }

    fn pop_clip(&mut self) {
        unsafe { self.dc.PopAxisAlignedClip() };
    }

    fn text(&mut self, s: &str, font: Font, align: Align, r: Rect, color: u32, alpha: f32) {
        let (w, h) = (r.width().max(1.0), r.height().max(1.0));
        let Some(layout) = self.layout(s, font, align, w, h) else {
            return;
        };
        let Some(brush) = self.brush(color, alpha) else {
            return;
        };
        let Ok(brush) = brush.cast::<ID2D1Brush>() else {
            return;
        };
        unsafe {
            self.dc.DrawTextLayout(
                point(r.left, r.top),
                &layout,
                &brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            )
        };
    }

    fn text_block(&mut self, s: &str, r: Rect, color: u32, alpha: f32) {
        let (w, h) = (r.width().max(1.0), r.height().max(1.0));
        let Some(layout) = self.layout(s, Font::Tooltip, Align::Left, w, h) else {
            return;
        };
        let Some(brush) = self.brush(color, alpha) else {
            return;
        };
        let Ok(brush) = brush.cast::<ID2D1Brush>() else {
            return;
        };
        unsafe {
            self.dc.DrawTextLayout(
                point(r.left, r.top),
                &layout,
                &brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            )
        };
    }

    fn text_width(&mut self, s: &str, font: Font) -> f32 {
        if s.is_empty() {
            return 0.0;
        }
        // Generous, so nothing wraps or is truncated before it is measured.
        const UNBOUNDED: f32 = 4096.0;
        let Some(layout) = self.layout(s, font, Align::Left, UNBOUNDED, 64.0) else {
            return 0.0;
        };
        metrics(&layout).map(|m| m.width).unwrap_or(0.0)
    }

    fn text_block_height(&mut self, s: &str, width: f32) -> f32 {
        let Some(layout) = self.layout(s, Font::Tooltip, Align::Left, width.max(1.0), 400.0) else {
            return 0.0;
        };
        metrics(&layout).map(|m| m.height).unwrap_or(0.0)
    }

    /// Nothing: the renderer ends the draw and presents, because only it holds
    /// the swap chain and only it can tell a lost device from a failed frame.
    fn present(&mut self) {}
}

fn metrics(layout: &IDWriteTextLayout) -> Option<DWRITE_TEXT_METRICS> {
    let mut out = DWRITE_TEXT_METRICS::default();
    unsafe { layout.GetMetrics(&mut out) }.ok()?;
    Some(out)
}

fn alignment(align: Align) -> DWRITE_TEXT_ALIGNMENT {
    match align {
        Align::Left => DWRITE_TEXT_ALIGNMENT_LEADING,
        Align::Center => DWRITE_TEXT_ALIGNMENT_CENTER,
        Align::Right => DWRITE_TEXT_ALIGNMENT_TRAILING,
    }
}

/// The portable rectangle as the Direct2D one. Same shape, same meaning; only
/// the type differs, and the portable crate must not know what Direct2D is.
fn d2d(r: Rect) -> D2D_RECT_F {
    D2D_RECT_F {
        left: r.left,
        top: r.top,
        right: r.right,
        bottom: r.bottom,
    }
}

fn rounded(r: Rect, radius: f32) -> D2D1_ROUNDED_RECT {
    D2D1_ROUNDED_RECT {
        rect: d2d(r),
        radiusX: radius,
        radiusY: radius,
    }
}

/// The core's toolkit-neutral colour as the Direct2D one.
pub fn rgba(hex: u32, a: f32) -> D2D1_COLOR_F {
    let c = theme::rgba(hex, a);
    D2D1_COLOR_F {
        r: c.r,
        g: c.g,
        b: c.b,
        a: c.a,
    }
}

fn point(x: f32, y: f32) -> Vector2 {
    Vector2 { X: x, Y: y }
}

/// The type role table, resolved from the metrics and the user's
/// customisation.
///
/// Alignment is deliberately *not* baked in here — it is set on the layout —
/// but the reading direction is, because DirectWrite decides bidi reordering
/// from the format.
pub fn font_sizes(metrics: Metrics, custom: &Appearance) -> [(f32, bool); FONTS.len()] {
    let m = metrics;
    let bold = custom.header_weight().0 >= 600;
    [
        (m.fs_title, true),
        (m.fs_clock, false),
        (m.fs_subtitle, false),
        (m.fs_section, bold),
        (m.fs_row, false),
        (m.fs_row, true),
        (m.fs_meta, false),
        (m.fs_footer, false),
        (m.fs_row, false),
    ]
}
