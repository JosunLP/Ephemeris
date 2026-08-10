// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! The drawing surface the widget is painted onto.
//!
//! Direct2D, Core Graphics and Cairo are the same *kind* of interface — an
//! immediate-mode 2D vector API with paths, fills, strokes and a clip stack —
//! and this is the shape they have in common. Everything above the trait
//! ([`crate::paint::widget`]) is written once; everything below it is a few
//! hundred lines per platform.
//!
//! Three things are deliberately *not* in the trait:
//!
//! * **Mirroring.** Right-to-left folds x across the panel's centre, which is
//!   arithmetic. [`crate::paint::widget::Painter`] does it before the call, so
//!   neither back end has to know the layout can be mirrored at all.
//! * **The reveal fade.** A single factor multiplied into every alpha, applied
//!   in the same place and for the same reason.
//! * **Colour.** Colours arrive as `0xRRGGBB` plus an alpha, exactly as
//!   [`tpmplaner_core::theme`] produces them. Converting to whatever the
//!   platform wants is one function per back end.
//!
//! Text is the part that genuinely differs, and the trait keeps it narrow on
//! purpose: draw one line into a box, draw a wrapped block into a box, measure
//! a line, measure a block. Everything else about the widget's typography —
//! which size, which weight, what gets ellipsised — is decided in
//! [`Font`] and in [`tpmplaner_core::layout`].

use tpmplaner_core::layout::Rect;

/// A type role. The size and weight behind each one come from
/// [`tpmplaner_core::theme::Metrics`] and the user's customisation, and are
/// resolved once when the back end is built.
///
/// There is no role for the refresh symbol. It used to be a glyph from Segoe
/// Fluent Icons on Windows and is a drawn path everywhere now: no font on macOS
/// or Linux can be relied on to carry it, and one widget with two different
/// refresh symbols would be two widgets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(usize)]
pub enum Font {
    Title = 0,
    Clock,
    Subtitle,
    Section,
    Row,
    RowStrong,
    Meta,
    Footer,
    /// Wrapping, no truncation — for the tooltip only.
    Tooltip,
}

/// Every role, so a back end can build its font table by iterating rather than
/// by listing them again and risking a gap.
pub const FONTS: [Font; 9] = [
    Font::Title,
    Font::Clock,
    Font::Subtitle,
    Font::Section,
    Font::Row,
    Font::RowStrong,
    Font::Meta,
    Font::Footer,
    Font::Tooltip,
];

/// Where a line sits inside the box it is drawn into.
///
/// In *physical* terms: [`crate::paint::widget::Painter`] has already swapped
/// leading for trailing where the layout is mirrored, so a back end never has
/// to reason about reading direction for placement. It still has to for
/// *shaping* — an Arabic run inside the line is the text engine's business,
/// and both back ends use one that handles it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// One stop of the panel's vertical gradient.
#[derive(Debug, Clone, Copy)]
pub struct Stop {
    /// 0.0 at the top of the rectangle, 1.0 at the bottom.
    pub at: f32,
    pub color: u32,
    pub alpha: f32,
}

/// What a renderer has to be able to draw.
///
/// Coordinates are device-independent pixels with the origin at the window's
/// top-left corner, matching [`tpmplaner_core::layout`]. Scaling for the
/// display is the back end's job and happens once, not per call.
pub trait Canvas {
    /// Everything transparent again, ready for a new frame.
    fn clear(&mut self);

    fn fill_round(&mut self, r: Rect, radius: f32, color: u32, alpha: f32);
    fn stroke_round(&mut self, r: Rect, radius: f32, color: u32, alpha: f32, width: f32);

    /// A rounded rectangle filled with a top-to-bottom gradient.
    ///
    /// Interpolated in premultiplied alpha where the platform offers the
    /// choice: the stops are partly transparent, and straight interpolation
    /// puts a visible grey haze between two of them.
    fn fill_gradient(&mut self, r: Rect, radius: f32, stops: &[Stop]);

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
    );

    /// Filled when `stroke` is `None`, otherwise stroked at that width.
    fn circle(&mut self, cx: f32, cy: f32, r: f32, color: u32, alpha: f32, stroke: Option<f32>);

    /// A stroked arc, angles in radians, clockwise from three o'clock.
    #[allow(clippy::too_many_arguments)]
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
    );

    /// A filled polygon. Only the refresh icon's arrowhead needs one.
    fn polygon(&mut self, points: &[(f32, f32)], color: u32, alpha: f32);

    fn push_clip(&mut self, r: Rect);
    fn pop_clip(&mut self);

    /// One line, vertically centred in `r`, ellipsised where it does not fit.
    #[allow(clippy::too_many_arguments)]
    fn text(&mut self, s: &str, font: Font, align: Align, r: Rect, color: u32, alpha: f32);

    /// A wrapped block, from the top edge of `r` downwards. Tooltips only.
    fn text_block(&mut self, s: &str, r: Rect, color: u32, alpha: f32);

    /// How wide one line is, unconstrained.
    ///
    /// The measurement [`tpmplaner_core::layout::column_width`] is waiting for:
    /// what to do with the number is the same everywhere, and only the front
    /// end knows how wide a word actually is.
    fn text_width(&mut self, s: &str, font: Font) -> f32;

    /// How tall the tooltip's text is once wrapped to `width`.
    fn text_block_height(&mut self, s: &str, width: f32) -> f32;

    /// Hand the finished frame to the compositor.
    fn present(&mut self);
}
