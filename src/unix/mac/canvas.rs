// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! [`Canvas`] on Core Graphics, with Core Text for the type.
//!
//! The context comes from AppKit: the view is asked to draw and hands the
//! current `CGContext` in. There is no swap chain and no surface to manage —
//! the window server owns the backing store, and a Retina display is handled
//! by a scale on the context that nothing here has to know about. That is why
//! this file is a third the length of the Direct2D one.
//!
//! **The view is flipped** (`isFlipped` returns YES), so y grows downwards and
//! the coordinates match [`tpmplaner_core::layout`] exactly. Two things follow.
//! Glyphs would come out upside down, so the text matrix carries a `(1, -1)`
//! scale — the standard recipe for Core Text in a flipped context. And a
//! wrapped block, which Core Text lays out into a frame from the bottom up,
//! has to be drawn inside a locally unflipped transform; see [`text_block`].
//!
//! [`text_block`]: Cg::text_block
//!
//! Units are points, not pixels, and deliberately: a point is 1/72 inch where
//! a Windows device-independent pixel is 1/96, so the metrics tuned against
//! Windows come out about a third larger here — which is exactly the
//! difference between the two systems' interface conventions. `fs_row` at 12.5
//! lands beside the 13-point system font, and `fs_title` at 17 beside the
//! 17-point title. Anyone who disagrees has `"scale"` in the settings.

use crate::paint::canvas::{Align, Canvas, FONTS, Font, Stop};
use crate::unix::cf;
use crate::unix::mac::objc::*;
use std::collections::HashMap;
use std::f32::consts::PI;
use tpmplaner_core::layout::Rect;
use tpmplaner_core::theme::{self, Appearance, Metrics};

/// A Core Text font this code owns.
struct OwnedFont(CTFontRef);

impl Drop for OwnedFont {
    fn drop(&mut self) {
        unsafe { cf::CFRelease(self.0 as cf::CFTypeRef) };
    }
}

pub struct Cg {
    /// Valid only for the duration of one `drawRect:`; set before drawing and
    /// cleared afterwards, so a stale context can never be drawn into.
    ctx: CGContextRef,
    fonts: Vec<OwnedFont>,
    /// The ellipsis appended to a truncated line, one per font role.
    ellipses: Vec<CTLineRef>,
    /// Measured line widths, keyed by text and role. Laying a line out is the
    /// most expensive step in a frame and the text does not change during an
    /// animation.
    widths: HashMap<(u64, u32), f32>,
    rtl: bool,
    /// Depth of the clip stack, so `pop_clip` cannot restore a state that was
    /// never saved.
    clips: u32,
}

impl Cg {
    /// Builds the font for every role from the metrics and the user's
    /// customisation.
    ///
    /// The system interface font unless a family was named: it is what the
    /// rest of the desktop uses, it already follows the accessibility settings
    /// for weight and size, and it carries the scripts the twenty catalogues
    /// need. A named family that cannot be found falls back to it rather than
    /// to whatever Core Text picks, and says so.
    pub fn new(metrics: Metrics, custom: &Appearance, rtl: bool) -> Self {
        let m = metrics;
        let bold = custom.header_weight().0 >= 600;
        let mut fonts = Vec::with_capacity(FONTS.len());
        let mut ellipses = Vec::with_capacity(FONTS.len());

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
            let font = make_font(custom.font_family(), size as f64, heavy);
            ellipses.push(make_line("…", font.0, rtl));
            fonts.push(font);
        }

        Self {
            ctx: std::ptr::null_mut(),
            fonts,
            ellipses,
            widths: HashMap::new(),
            rtl,
            clips: 0,
        }
    }

    /// Points this context at the `CGContext` AppKit handed in, for the
    /// duration of one `drawRect:`.
    ///
    /// # Safety
    ///
    /// `ctx` must stay valid until [`Self::end`] is called.
    pub unsafe fn begin(&mut self, ctx: CGContextRef) {
        self.ctx = ctx;
        self.clips = 0;
        unsafe {
            // Upright glyphs in a flipped context. Set once per frame rather
            // than per line: nothing else touches the text matrix.
            CGContextSetTextMatrix(
                ctx,
                CGAffineTransform {
                    a: 1.0,
                    b: 0.0,
                    c: 0.0,
                    d: -1.0,
                    tx: 0.0,
                    ty: 0.0,
                },
            );
        }
    }

    pub fn end(&mut self) {
        // Any clip left open would leak into the next frame, which shares
        // nothing else with this one.
        while self.clips > 0 {
            self.pop_clip();
        }
        self.ctx = std::ptr::null_mut();
    }

    /// Keeps the width cache from growing without bound: the relative times
    /// ("in 25 min") produce a new key every minute.
    pub fn trim_caches(&mut self) {
        if self.widths.len() > 512 {
            self.widths.clear();
        }
    }

    fn font(&self, role: Font) -> CTFontRef {
        self.fonts[role as usize].0
    }

    fn fill(&self, color: u32, alpha: f32) {
        let c = theme::rgba(color, alpha);
        unsafe {
            CGContextSetRGBFillColor(self.ctx, c.r as f64, c.g as f64, c.b as f64, c.a as f64);
        }
    }

    fn stroke_color(&self, color: u32, alpha: f32) {
        let c = theme::rgba(color, alpha);
        unsafe {
            CGContextSetRGBStrokeColor(self.ctx, c.r as f64, c.g as f64, c.b as f64, c.a as f64);
        }
    }

    /// A rounded rectangle as the current path.
    ///
    /// `AddArcToPoint` rather than four explicit arcs: it takes the corner and
    /// the two directions and works out the tangents, which is exactly what a
    /// rounded corner is and leaves nothing to get wrong at the joins.
    fn round_rect_path(&self, r: Rect, radius: f32) {
        let c = self.ctx;
        // A radius larger than half the shorter side would produce a
        // self-intersecting path. Clamping is what every other rounded
        // rectangle does and is what the shadow's growing corners need.
        let radius = radius.min(r.width() * 0.5).min(r.height() * 0.5).max(0.0) as f64;
        let (l, t, rt, b) = (r.left as f64, r.top as f64, r.right as f64, r.bottom as f64);
        unsafe {
            CGContextBeginPath(c);
            CGContextMoveToPoint(c, l + radius, t);
            CGContextAddArcToPoint(c, rt, t, rt, b, radius);
            CGContextAddArcToPoint(c, rt, b, l, b, radius);
            CGContextAddArcToPoint(c, l, b, l, t, radius);
            CGContextAddArcToPoint(c, l, t, rt, t, radius);
            CGContextClosePath(c);
        }
    }

    /// One laid-out line, ready to draw or measure.
    ///
    /// Returned rather than cached: a `CTLine` holds its own glyph run and
    /// keeping every one alive for a widget that redraws at 60 Hz would grow
    /// without bound. What *is* cached is the measurement, which is what the
    /// column widths ask for many times per frame.
    ///
    /// Not called `line`: [`Canvas`] has a method of that name and the trait's
    /// would win here, because its `&mut self` matches the receiver a step
    /// earlier than this one's `&self` does.
    fn laid_out(&self, s: &str, role: Font) -> Option<Line> {
        Line::new(s, self.font(role), self.rtl)
    }
}

impl Canvas for Cg {
    fn clear(&mut self) {
        // The window is transparent and the view fills it, so the whole
        // rectangle goes back to nothing before the panel is drawn over it.
        // Without this, the previous frame shows through wherever the new one
        // is translucent.
        // Larger than any window, and centred on the origin so it covers
        // the view whatever the clip happens to be. `CGContextClearRect`
        // takes a rectangle rather than "everything", and asking the view for
        // its bounds here would mean a message send per frame for a number
        // that only has to be big enough.
        unsafe {
            CGContextClearRect(self.ctx, NSRect::new(-5.0e5, -5.0e5, 1.0e6, 1.0e6));
        }
    }

    fn fill_round(&mut self, r: Rect, radius: f32, color: u32, alpha: f32) {
        self.round_rect_path(r, radius);
        self.fill(color, alpha);
        unsafe { CGContextFillPath(self.ctx) };
    }

    fn stroke_round(&mut self, r: Rect, radius: f32, color: u32, alpha: f32, width: f32) {
        self.round_rect_path(r, radius);
        self.stroke_color(color, alpha);
        unsafe {
            CGContextSetLineWidth(self.ctx, width as f64);
            CGContextSetLineJoin(self.ctx, kCGLineJoinRound);
            CGContextStrokePath(self.ctx);
        }
    }

    fn fill_gradient(&mut self, r: Rect, radius: f32, stops: &[Stop]) {
        // Four components per stop, in the order Core Graphics wants them.
        let mut components: Vec<CGFloat> = Vec::with_capacity(stops.len() * 4);
        let mut locations: Vec<CGFloat> = Vec::with_capacity(stops.len());
        for s in stops {
            let c = theme::rgba(s.color, s.alpha);
            components.extend_from_slice(&[c.r as f64, c.g as f64, c.b as f64, c.a as f64]);
            locations.push(s.at as f64);
        }

        unsafe {
            let space = CGColorSpaceCreateDeviceRGB();
            let gradient = CGGradientCreateWithColorComponents(
                space,
                components.as_ptr(),
                locations.as_ptr(),
                stops.len(),
            );
            CGColorSpaceRelease(space);
            if gradient.is_null() {
                return;
            }
            // Clipped to the rounded shape rather than drawn as one: a
            // gradient has no path of its own.
            CGContextSaveGState(self.ctx);
            self.round_rect_path(r, radius);
            CGContextClip(self.ctx);
            CGContextDrawLinearGradient(
                self.ctx,
                gradient,
                NSPoint {
                    x: r.left as f64,
                    y: r.top as f64,
                },
                NSPoint {
                    x: r.left as f64,
                    y: r.bottom as f64,
                },
                kCGGradientClamp,
            );
            CGContextRestoreGState(self.ctx);
            CGGradientRelease(gradient);
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
        self.stroke_color(color, alpha);
        unsafe {
            CGContextSetLineWidth(self.ctx, width as f64);
            CGContextSetLineCap(
                self.ctx,
                if round_caps {
                    kCGLineCapRound
                } else {
                    kCGLineCapButt
                },
            );
            CGContextBeginPath(self.ctx);
            CGContextMoveToPoint(self.ctx, x0 as f64, y0 as f64);
            CGContextAddLineToPoint(self.ctx, x1 as f64, y1 as f64);
            CGContextStrokePath(self.ctx);
        }
    }

    fn circle(&mut self, cx: f32, cy: f32, r: f32, color: u32, alpha: f32, stroke: Option<f32>) {
        unsafe {
            CGContextBeginPath(self.ctx);
            CGContextAddArc(
                self.ctx,
                cx as f64,
                cy as f64,
                r.max(0.0) as f64,
                0.0,
                (2.0 * PI) as f64,
                0,
            );
            match stroke {
                Some(width) => {
                    self.stroke_color(color, alpha);
                    CGContextSetLineWidth(self.ctx, width as f64);
                    CGContextStrokePath(self.ctx);
                }
                None => {
                    self.fill(color, alpha);
                    CGContextFillPath(self.ctx);
                }
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
        self.stroke_color(color, alpha);
        unsafe {
            CGContextSetLineWidth(self.ctx, width as f64);
            CGContextSetLineCap(self.ctx, kCGLineCapRound);
            CGContextBeginPath(self.ctx);
            // Zero means "increasing angle", which is what the caller's
            // arithmetic assumes. It *looks* clockwise because the context is
            // flipped, and the arrowhead is computed in the same flipped
            // convention, so the two agree.
            CGContextAddArc(
                self.ctx,
                cx as f64,
                cy as f64,
                r.max(0.0) as f64,
                from as f64,
                to as f64,
                0,
            );
            CGContextStrokePath(self.ctx);
        }
    }

    fn polygon(&mut self, points: &[(f32, f32)], color: u32, alpha: f32) {
        let Some((first, rest)) = points.split_first() else {
            return;
        };
        self.fill(color, alpha);
        unsafe {
            CGContextBeginPath(self.ctx);
            CGContextMoveToPoint(self.ctx, first.0 as f64, first.1 as f64);
            for (x, y) in rest {
                CGContextAddLineToPoint(self.ctx, *x as f64, *y as f64);
            }
            CGContextClosePath(self.ctx);
            CGContextFillPath(self.ctx);
        }
    }

    fn push_clip(&mut self, r: Rect) {
        unsafe {
            CGContextSaveGState(self.ctx);
            CGContextClipToRect(
                self.ctx,
                NSRect::new(
                    r.left as f64,
                    r.top as f64,
                    r.width() as f64,
                    r.height() as f64,
                ),
            );
        }
        self.clips += 1;
    }

    fn pop_clip(&mut self) {
        if self.clips == 0 {
            return;
        }
        self.clips -= 1;
        unsafe { CGContextRestoreGState(self.ctx) };
    }

    fn text(&mut self, s: &str, font: Font, align: Align, r: Rect, color: u32, alpha: f32) {
        let Some(line) = self.laid_out(s, font) else {
            return;
        };
        let available = r.width().max(0.0);
        // Truncated with an ellipsis rather than clipped: a title that runs out
        // of room has to *look* cut off, or the tooltip that offers the rest
        // has nothing to hint at.
        let drawn = line.truncated(available, self.ellipses[font as usize]);
        let width = drawn.width();

        // Placed from the measured width rather than through the paragraph
        // style's alignment: that one aligns within a *frame*, and a `CTLine`
        // has none — it is drawn at wherever the text position says.
        let x = match align {
            Align::Left => r.left,
            Align::Center => r.center_x() - width * 0.5,
            Align::Right => r.right - width,
        };
        // The baseline, so the line sits optically centred in its box.
        let (ascent, descent) = drawn.vertical_metrics();
        let y = r.center_y() + (ascent - descent) * 0.5;

        self.fill(color, alpha);
        unsafe {
            CGContextSetTextPosition(self.ctx, x as f64, y as f64);
            CTLineDraw(drawn.raw(), self.ctx);
        }
    }

    fn text_block(&mut self, s: &str, r: Rect, color: u32, alpha: f32) {
        let Some(attributed) = attributed(s, self.font(Font::Tooltip), self.rtl) else {
            return;
        };
        self.fill(color, alpha);
        unsafe {
            let framesetter = CTFramesetterCreateWithAttributedString(attributed.as_raw());
            let Some(framesetter) = cf::Owned::new(framesetter) else {
                return;
            };
            // Core Text lays a frame out from its bottom-left corner upwards.
            // Rather than reasoning about that inside a flipped context, the
            // transform is undone for the duration: the origin moves to the
            // box's bottom edge and y grows up again, so the frame occupies
            // (0,0) to (width, height) in the ordinary way.
            CGContextSaveGState(self.ctx);
            CGContextTranslateCTM(self.ctx, r.left as f64, r.bottom as f64);
            CGContextScaleCTM(self.ctx, 1.0, -1.0);
            CGContextSetTextMatrix(
                self.ctx,
                CGAffineTransform {
                    a: 1.0,
                    b: 0.0,
                    c: 0.0,
                    d: 1.0,
                    tx: 0.0,
                    ty: 0.0,
                },
            );

            let path = CGPathCreateWithRect(
                NSRect::new(0.0, 0.0, r.width() as f64, r.height() as f64),
                std::ptr::null(),
            );
            let frame = CTFramesetterCreateFrame(
                framesetter.as_raw() as CTFramesetterRef,
                CFRange::default(),
                path,
                std::ptr::null(),
            );
            if !frame.is_null() {
                CTFrameDraw(frame, self.ctx);
                cf::CFRelease(frame as cf::CFTypeRef);
            }
            CGPathRelease(path);

            CGContextRestoreGState(self.ctx);
            // The frame-local transform above replaced the frame-wide one.
            CGContextSetTextMatrix(
                self.ctx,
                CGAffineTransform {
                    a: 1.0,
                    b: 0.0,
                    c: 0.0,
                    d: -1.0,
                    tx: 0.0,
                    ty: 0.0,
                },
            );
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
        let width = self.laid_out(s, font).map(|l| l.width()).unwrap_or(0.0);
        self.widths.insert(key, width);
        width
    }

    fn text_block_height(&mut self, s: &str, width: f32) -> f32 {
        let Some(attributed) = attributed(s, self.font(Font::Tooltip), self.rtl) else {
            return 0.0;
        };
        unsafe {
            let framesetter = CTFramesetterCreateWithAttributedString(attributed.as_raw());
            let Some(framesetter) = cf::Owned::new(framesetter) else {
                return 0.0;
            };
            let size = CTFramesetterSuggestFrameSizeWithConstraints(
                framesetter.as_raw() as CTFramesetterRef,
                CFRange::default(),
                std::ptr::null(),
                NSSize {
                    width: width.max(1.0) as f64,
                    // Tall enough that the answer is the text's own height
                    // rather than this bound.
                    height: 4096.0,
                },
                std::ptr::null_mut(),
            );
            size.height as f32
        }
    }

    /// Nothing to do: AppKit presents the view's backing store when
    /// `drawRect:` returns.
    fn present(&mut self) {
        self.trim_caches();
    }
}

impl Drop for Cg {
    fn drop(&mut self) {
        for line in self.ellipses.drain(..) {
            if !line.is_null() {
                unsafe { cf::CFRelease(line as cf::CFTypeRef) };
            }
        }
    }
}

/// A `CTLine` this code owns.
struct Line(CTLineRef);

impl Line {
    fn new(s: &str, font: CTFontRef, rtl: bool) -> Option<Self> {
        let attributed = attributed(s, font, rtl)?;
        let raw = unsafe { CTLineCreateWithAttributedString(attributed.as_raw()) };
        (!raw.is_null()).then_some(Self(raw))
    }

    fn raw(&self) -> CTLineRef {
        self.0
    }

    fn width(&self) -> f32 {
        unsafe {
            CTLineGetTypographicBounds(
                self.0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            ) as f32
        }
    }

    fn vertical_metrics(&self) -> (f32, f32) {
        let (mut ascent, mut descent) = (0.0f64, 0.0f64);
        unsafe {
            CTLineGetTypographicBounds(self.0, &mut ascent, &mut descent, std::ptr::null_mut());
        }
        (ascent as f32, descent as f32)
    }

    /// The same line cut to `width` with an ellipsis, or this one if it fits.
    ///
    /// `CTLineCreateTruncatedLine` returns null when it cannot do the job — a
    /// width narrower than the ellipsis itself, most often — and the original
    /// is the right answer there: the clip rectangle bounds it anyway.
    fn truncated(self, width: f32, token: CTLineRef) -> Self {
        if width <= 0.0 || self.width() <= width {
            return self;
        }
        let raw =
            unsafe { CTLineCreateTruncatedLine(self.0, width as f64, kCTLineTruncationEnd, token) };
        match (!raw.is_null()).then_some(raw) {
            Some(raw) => Self(raw),
            None => self,
        }
    }
}

impl Drop for Line {
    fn drop(&mut self) {
        unsafe { cf::CFRelease(self.0 as cf::CFTypeRef) };
    }
}

/// A `CFAttributedString` carrying the font, the base writing direction and a
/// note to take the colour from the context.
fn attributed(s: &str, font: CTFontRef, rtl: bool) -> Option<cf::Owned> {
    let text = cf::string(s)?;
    let style = paragraph_style(rtl)?;
    let keys = [
        unsafe { kCTFontAttributeName },
        unsafe { kCTForegroundColorFromContextAttributeName },
        unsafe { kCTParagraphStyleAttributeName },
    ];
    let values = [
        font as cf::CFTypeRef,
        unsafe { cf::kCFBooleanTrue },
        style.as_raw(),
    ];
    let attributes = cf::dictionary(&keys, &values)?;
    cf::Owned::new(unsafe {
        CFAttributedStringCreate(std::ptr::null(), text.as_raw(), attributes.as_raw())
    })
}

/// The paragraph style, which is only ever asked one question: which way does
/// this line read?
///
/// It matters even when the text is Latin. Running under `ar-SA` with no
/// Arabic catalogue gives an English string in a mirrored layout, and without
/// a right-to-left base direction its trailing punctuation lands on the wrong
/// side.
fn paragraph_style(rtl: bool) -> Option<cf::Owned> {
    let direction = if rtl {
        kCTWritingDirectionRightToLeft
    } else {
        kCTWritingDirectionLeftToRight
    };
    let settings = [CTParagraphStyleSetting {
        spec: kCTParagraphStyleSpecifierBaseWritingDirection,
        valueSize: std::mem::size_of::<i8>(),
        value: &direction as *const i8 as *const std::ffi::c_void,
    }];
    cf::Owned::new(unsafe { CTParagraphStyleCreate(settings.as_ptr(), settings.len()) })
}

/// The font for one role.
fn make_font(family: Option<&str>, size: f64, heavy: bool) -> OwnedFont {
    let base = family
        .and_then(|f| {
            let name = cf::string(f)?;
            let raw = unsafe { CTFontCreateWithName(name.as_raw(), size, std::ptr::null()) };
            (!raw.is_null()).then_some(raw)
        })
        .unwrap_or_else(|| unsafe {
            CTFontCreateUIFontForLanguage(kCTFontUIFontSystem, size, std::ptr::null())
        });

    if !heavy {
        return OwnedFont(base);
    }
    // A bold copy, or the regular one where the family has no bold face —
    // which is not worth refusing to draw over.
    let bold = unsafe {
        CTFontCreateCopyWithSymbolicTraits(
            base,
            size,
            std::ptr::null(),
            kCTFontTraitBold,
            kCTFontTraitBold,
        )
    };
    if bold.is_null() {
        return OwnedFont(base);
    }
    unsafe { cf::CFRelease(base as cf::CFTypeRef) };
    OwnedFont(bold)
}

fn make_line(s: &str, font: CTFontRef, rtl: bool) -> CTLineRef {
    match Line::new(s, font, rtl) {
        Some(line) => {
            // Handed to `CTLineCreateTruncatedLine` for the lifetime of the
            // canvas, so the reference is kept rather than dropped.
            let raw = line.0;
            std::mem::forget(line);
            raw
        }
        None => std::ptr::null(),
    }
}

fn hash(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish()
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPathCreateWithRect(rect: NSRect, transform: *const CGAffineTransform) -> CGPathRef;
    fn CGPathRelease(path: CGPathRef);
}
