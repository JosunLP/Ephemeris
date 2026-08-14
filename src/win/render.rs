// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Ephemeris contributors
//! Direct2D renderer on a DirectComposition surface.
//!
//! How the chain is put together:
//!
//! ```text
//! D3D11 Device ─► DXGI Device ─┬─► D2D Device ─► D2D DeviceContext  (drawing)
//!                              └─► DComp Device ─► Visual ─► Target ─► HWND
//!                                        ▲
//!               Composition-Swapchain ───┘   (DXGI_ALPHA_MODE_PREMULTIPLIED)
//! ```
//!
//! Why this route rather than a classic layered window:
//! `UpdateLayeredWindow` goes through a CPU bitmap and costs a full copy every
//! frame. The composition swapchain stays entirely on the GPU and still gives
//! true per-pixel alpha — which requires `WS_EX_NOREDIRECTIONBITMAP` on the
//! window.
//!
//! Drawing happens on demand: new data, a change of minute, hover — and at
//! roughly 60 Hz while an animation runs. Once everything has settled, drawing
//! stops completely.
//!
//! **What this file does not decide is what the widget looks like.** That is
//! [`crate::paint::widget`], written once for every front end against a small
//! drawing trait, and [`super::canvas`] is Direct2D's implementation of it.
//! What is left here is the device chain, the fonts, and the three lines that
//! begin a frame, hand it over and present it.

use crate::paint::canvas::{FONTS, Font};
use crate::paint::widget as paint;
use crate::win::canvas::{self, D2dCanvas, LayoutKey};
use crate::win::platform;
use ephemeris_core::log;
use ephemeris_core::theme::{Appearance, Metrics, Palette};

/// What a click means, the rectangle it landed in, and what one frame is drawn
/// from. All of it lives in the core now — the window translates a click into a
/// [`Hit`] without drawing anything, and every front end needs the identical
/// set. Re-exported so the window still reaches them through the renderer it
/// already imports.
pub use ephemeris_core::layout::{Frame, FrameResult, Hit, HitRegion, UndoView};

use std::cell::RefCell;
use std::collections::HashMap;
use std::mem::ManuallyDrop;
use windows::Win32::Foundation::HWND;
use windows::Win32::Globalization::IsValidLocaleName;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_TARGET, D2D1_BITMAP_PROPERTIES1,
    D2D1_CAP_STYLE_ROUND, D2D1_DEVICE_CONTEXT_OPTIONS_NONE, D2D1_FACTORY_TYPE_SINGLE_THREADED,
    D2D1_LINE_JOIN_ROUND, D2D1_STROKE_STYLE_PROPERTIES1, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE,
    D2D1CreateFactory, ID2D1Bitmap1, ID2D1DeviceContext, ID2D1Factory1, ID2D1SolidColorBrush,
    ID2D1StrokeStyle,
};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11CreateDevice, ID3D11Device,
};
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice, IDCompositionDevice, IDCompositionTarget, IDCompositionVisual,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD,
    DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_PARAGRAPH_ALIGNMENT_NEAR,
    DWRITE_READING_DIRECTION_RIGHT_TO_LEFT, DWRITE_TEXT_ALIGNMENT, DWRITE_TEXT_ALIGNMENT_LEADING,
    DWRITE_TRIMMING, DWRITE_TRIMMING_GRANULARITY_CHARACTER, DWRITE_WORD_WRAPPING_NO_WRAP,
    DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat, IDWriteTextLayout,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    DXGI_PRESENT, DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
    DXGI_USAGE_RENDER_TARGET_OUTPUT, IDXGIDevice, IDXGIFactory2, IDXGISurface, IDXGISwapChain1,
};
use windows::core::{HRESULT, Interface, PCWSTR, Result};

pub struct Renderer {
    dcomp: IDCompositionDevice,
    _target: IDCompositionTarget,
    _visual: IDCompositionVisual,
    swapchain: IDXGISwapChain1,
    dc: ID2D1DeviceContext,
    dwrite: IDWriteFactory,
    _d2d_factory: ID2D1Factory1,

    /// One text format per role in [`FONTS`], in that order.
    formats: Vec<IDWriteTextFormat>,
    round_stroke: ID2D1StrokeStyle,

    /// One brush per colour; opacity is set per use. At 60 Hz this saves a
    /// few hundred COM object creations per second.
    brushes: RefCell<HashMap<u32, ID2D1SolidColorBrush>>,
    /// Finished text layouts. DirectWrite layout is the most expensive single
    /// step in a frame, and the text does not change during an animation.
    layouts: RefCell<HashMap<LayoutKey, IDWriteTextLayout>>,

    pal: Palette,
    /// The user's customisation, kept for the per-calendar colour overrides.
    /// Everything else it carries — typography, density, the surface style —
    /// is already baked into `formats`, `metrics` and `pal` by the time the
    /// renderer exists, which is why a change to any of it rebuilds it.
    custom: Appearance,
    /// Right-to-left layout. The mirroring itself happens in
    /// [`crate::paint::widget`], before anything reaches Direct2D; what the
    /// format carries is the reading direction, which is what DirectWrite
    /// reorders a mixed run by.
    rtl: bool,
    metrics: Metrics,
    dpi: f32,
    size_px: (u32, u32),
}

impl Renderer {
    /// `metrics` is passed in rather than derived from a scale factor: the
    /// window sizes itself from the same values — the shadow margin decides
    /// how much larger the window is than the visible panel — and the two
    /// drifting apart is how the panel ends up inset inside its own window.
    ///
    /// `lang` is the BCP-47 tag the interface text is written in. It is not
    /// cosmetic — see [`locale_name`].
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        hwnd: HWND,
        width_px: u32,
        height_px: u32,
        dpi: f32,
        metrics: Metrics,
        pal: Palette,
        rtl: bool,
        custom: &Appearance,
        lang: &str,
    ) -> Result<Self> {
        unsafe {
            let device = create_d3d_device()?;
            let dxgi_device: IDXGIDevice = device.cast()?;

            let adapter = dxgi_device.GetAdapter()?;
            let factory: IDXGIFactory2 = adapter.GetParent()?;

            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: width_px.max(1),
                Height: height_px.max(1),
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                Stereo: false.into(),
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                Scaling: DXGI_SCALING_STRETCH,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                // Without premultiplied alpha the window stays opaque.
                AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
                Flags: 0,
            };
            let swapchain = factory.CreateSwapChainForComposition(&device, &desc, None)?;

            let dcomp: IDCompositionDevice = DCompositionCreateDevice(&dxgi_device)?;

            // `topmost = false`: the window manages its own z-order
            // (bottom-most); composition does not.
            let target = dcomp.CreateTargetForHwnd(hwnd, false)?;
            let visual = dcomp.CreateVisual()?;
            visual.SetContent(&swapchain)?;
            target.SetRoot(&visual)?;
            dcomp.Commit()?;

            let d2d_factory: ID2D1Factory1 =
                D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let d2d_device = d2d_factory.CreateDevice(&dxgi_device)?;
            let dc = d2d_device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;

            // ClearType needs an opaque background. On a transparent glass
            // surface it produces colour fringes, hence greyscale
            // antialiasing — which is what WPF and WinUI do on acrylic too.
            dc.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);

            // Round caps for ticks and progress bars.
            let round_stroke = d2d_factory.CreateStrokeStyle(
                &D2D1_STROKE_STYLE_PROPERTIES1 {
                    startCap: D2D1_CAP_STYLE_ROUND,
                    endCap: D2D1_CAP_STYLE_ROUND,
                    dashCap: D2D1_CAP_STYLE_ROUND,
                    lineJoin: D2D1_LINE_JOIN_ROUND,
                    miterLimit: 10.0,
                    ..Default::default()
                },
                None,
            )?;
            // `ID2D1Factory1` hands back the "1" variant, but `DrawLine`
            // wants exactly the base interface.
            let round_stroke: ID2D1StrokeStyle = round_stroke.into();

            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let m = metrics;
            // All kept alive for the whole block: every format below borrows
            // them.
            let family = font_family(custom);
            let locale = locale_name(lang);
            let section_weight = DWRITE_FONT_WEIGHT(custom.header_weight().0 as i32);

            // One format per role in `FONTS`, in that order. Alignment is
            // deliberately *not* baked in: the same role is used at both ends
            // of a row, and a format per combination would be twice as many
            // for no gain. `SetTextAlignment` does it per layout instead.
            let mut formats = Vec::with_capacity(FONTS.len());
            for (role, (size, heavy)) in FONTS.into_iter().zip(canvas::font_sizes(m, custom)) {
                formats.push(match role {
                    // The tooltip is the one role that wraps rather than being
                    // cut with an ellipsis, which is a property of the format.
                    Font::Tooltip => tooltip_format(&dwrite, size, rtl, &family, &locale)?,
                    Font::Section => text_format(
                        &dwrite,
                        size,
                        section_weight,
                        DWRITE_TEXT_ALIGNMENT_LEADING,
                        rtl,
                        &family,
                        &locale,
                    )?,
                    _ => text_format(
                        &dwrite,
                        size,
                        if heavy {
                            DWRITE_FONT_WEIGHT_SEMI_BOLD
                        } else {
                            DWRITE_FONT_WEIGHT_NORMAL
                        },
                        DWRITE_TEXT_ALIGNMENT_LEADING,
                        rtl,
                        &family,
                        &locale,
                    )?,
                });
            }

            let mut me = Self {
                dcomp,
                _target: target,
                _visual: visual,
                swapchain,
                dc,
                dwrite,
                _d2d_factory: d2d_factory,
                formats,
                round_stroke,
                brushes: RefCell::new(HashMap::new()),
                layouts: RefCell::new(HashMap::new()),
                pal,
                custom: custom.clone(),
                rtl,
                metrics: m,
                dpi,
                size_px: (width_px.max(1), height_px.max(1)),
            };
            me.bind_target()?;
            Ok(me)
        }
    }

    /// The theme changed (Windows light/dark, a different accent colour, or an
    /// edited colour in the configuration).
    ///
    /// Takes the appearance as well as the palette, and not only because the
    /// two are read from the same configuration: the per-calendar overrides in
    /// [`Self::calendar_color`] are looked up on `custom` at draw time rather
    /// than baked into `pal`. `layout_differs` deliberately excludes
    /// `calendar_colors`, so editing only those comes down this path instead of
    /// through a rebuild — and leaving `custom` behind meant the new colour
    /// never reached the screen until the next restart.
    ///
    /// Brushes are cached by colour; without clearing them the old entries
    /// would stay around for good.
    pub fn set_palette(&mut self, pal: Palette, custom: &Appearance) {
        self.pal = pal;
        self.custom = custom.clone();
        self.brushes.borrow_mut().clear();
    }

    pub fn palette(&self) -> Palette {
        self.pal
    }

    /// Point D2D at the back buffer. Has to happen again after every resize.
    fn bind_target(&mut self) -> Result<()> {
        unsafe {
            let surface: IDXGISurface = self.swapchain.GetBuffer(0)?;
            let props = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: self.dpi,
                dpiY: self.dpi,
                bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                colorContext: ManuallyDrop::new(None),
            };
            let bitmap: ID2D1Bitmap1 = self
                .dc
                .CreateBitmapFromDxgiSurface(&surface, Some(&props))?;
            self.dc.SetTarget(&bitmap);
            // From here everything is in DIPs; D2D handles the DPI scaling.
            self.dc.SetDpi(self.dpi, self.dpi);
            Ok(())
        }
    }

    pub fn resize(&mut self, width_px: u32, height_px: u32, dpi: f32) -> Result<()> {
        let (w, h) = (width_px.max(1), height_px.max(1));
        if (w, h) == self.size_px && (dpi - self.dpi).abs() < f32::EPSILON {
            return Ok(());
        }
        unsafe {
            // The old target has to let go of the back buffer, or
            // ResizeBuffers fails with DXGI_ERROR_INVALID_CALL.
            self.dc.SetTarget(None);
            self.swapchain.ResizeBuffers(
                0,
                w,
                h,
                DXGI_FORMAT_B8G8R8A8_UNORM,
                Default::default(),
            )?;
        }
        self.size_px = (w, h);
        self.dpi = dpi;
        self.layouts.borrow_mut().clear();
        self.bind_target()
    }

    /// Size of the visible area in DIPs.
    pub fn size_dip(&self) -> (f32, f32) {
        let k = 96.0 / self.dpi;
        (self.size_px.0 as f32 * k, self.size_px.1 as f32 * k)
    }

    /// `hits` is supplied by the caller and refilled here, which avoids a Vec
    /// allocation per frame.
    /// Draws one frame and refills `hits`.
    ///
    /// The picture is [`crate::paint::widget::draw`]'s, the same one macOS and
    /// Linux paint. What happens here is opening the frame, lending it a
    /// Direct2D canvas, and presenting.
    pub fn draw(&mut self, frame: &Frame, hits: &mut Vec<HitRegion>) -> Result<FrameResult> {
        let size = self.size_dip();
        let (metrics, pal, rtl) = (self.metrics, self.pal, self.rtl);

        unsafe { self.dc.BeginDraw() };
        let result = {
            let mut canvas = D2dCanvas {
                dc: &self.dc,
                dwrite: &self.dwrite,
                formats: &self.formats,
                round_stroke: &self.round_stroke,
                brushes: &self.brushes,
                layouts: &self.layouts,
                rtl,
            };
            paint::draw(
                &mut canvas,
                size,
                metrics,
                pal,
                &self.custom,
                rtl,
                frame,
                hits,
            )
        };

        unsafe {
            // `EndDraw` is where a lost device surfaces: Direct2D records the
            // failure on the context rather than returning it from every call.
            self.dc.EndDraw(None, None)?;
            self.swapchain.Present(1, DXGI_PRESENT(0)).ok()?;
            self.dcomp.Commit()?;
        }
        Ok(result)
    }
}

/// A lost graphics device — a driver change, a GPU reset, a move into an RDP
/// session. Afterwards the entire chain is invalid and has to be rebuilt;
/// without handling this the widget would stay black for good.
pub fn is_device_lost(code: HRESULT) -> bool {
    const D2DERR_RECREATE_TARGET: u32 = 0x8899_000C;
    const DXGI_ERROR_DEVICE_REMOVED: u32 = 0x887A_0005;
    const DXGI_ERROR_DEVICE_HUNG: u32 = 0x887A_0006;
    const DXGI_ERROR_DEVICE_RESET: u32 = 0x887A_0007;
    const DXGI_ERROR_DRIVER_INTERNAL_ERROR: u32 = 0x887A_0020;

    matches!(
        code.0 as u32,
        D2DERR_RECREATE_TARGET
            | DXGI_ERROR_DEVICE_REMOVED
            | DXGI_ERROR_DEVICE_HUNG
            | DXGI_ERROR_DEVICE_RESET
            | DXGI_ERROR_DRIVER_INTERNAL_ERROR
    )
}

/// Hardware by preference, WARP as the fallback (RDP sessions, VMs with no GPU).
fn create_d3d_device() -> Result<ID3D11Device> {
    for driver in [D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP] {
        let mut device: Option<ID3D11Device> = None;
        let hr = unsafe {
            D3D11CreateDevice(
                None,
                driver,
                Default::default(),
                // BGRA_SUPPORT is mandatory for Direct2D interop.
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                None,
            )
        };
        if hr.is_ok()
            && let Some(d) = device
        {
            return Ok(d);
        }
    }
    Err(windows::core::Error::from_thread())
}

/// The locale name to hand DirectWrite, as a nul-terminated wide string.
///
/// Not a formality. Han unification gives one code point different accepted
/// shapes per language — 直, 骨 and 令 are drawn differently in Japanese,
/// Simplified and Traditional Chinese — and this name is what DirectWrite
/// selects the font and the glyph variant with. It also decides where a line
/// may break, which differs between Chinese and Japanese. Getting it wrong
/// produces text that is perfectly legible and visibly foreign to a native
/// reader, which is precisely the friction a localised interface is for.
///
/// The tag may come straight out of `config.json`, so it is validated first:
/// `CreateTextFormat` rejects an unknown locale name, and without this check a
/// typo in the settings file would turn into "Ephemeris could not start". An
/// empty name means "no particular locale", which is what the renderer used to
/// get in effect.
fn locale_name(tag: &str) -> Vec<u16> {
    let wide = platform::wide(tag);
    let valid = unsafe { IsValidLocaleName(PCWSTR(wide.as_ptr())) }.as_bool();
    if valid {
        wide
    } else {
        log::warn(&format!(
            "'{tag}' is not a locale name Windows knows — text is laid out without one"
        ));
        platform::wide("")
    }
}

/// The font family to draw the interface in, as a nul-terminated wide string.
///
/// "Segoe UI Variable Text" is the Windows 11 system font; on older systems
/// DirectWrite falls back to Segoe UI by itself. A name it does not know falls
/// back the same way rather than failing, which is why a typo here costs a
/// different font and not a widget that will not start. Scripts the family does
/// not cover — CJK, Thai, Devanagari — go through DirectWrite's own font
/// fallback, which is why the locale name matters as well.
fn font_family(custom: &Appearance) -> Vec<u16> {
    match custom.font_family() {
        Some(name) => platform::wide(name),
        None => platform::wide("Segoe UI Variable Text"),
    }
}

fn text_format(
    dwrite: &IDWriteFactory,
    size: f32,
    weight: DWRITE_FONT_WEIGHT,
    align: DWRITE_TEXT_ALIGNMENT,
    rtl: bool,
    family: &[u16],
    locale: &[u16],
) -> Result<IDWriteTextFormat> {
    unsafe {
        let format = dwrite.CreateTextFormat(
            PCWSTR(family.as_ptr()),
            None,
            weight,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            size,
            PCWSTR(locale.as_ptr()),
        )?;
        // With a right-to-left reading direction DirectWrite reorders the runs
        // of a bidirectional line, which is what this is for. It also swaps the
        // meaning of LEADING and TRAILING — "leading" is then the right edge —
        // and `super::canvas::alignment` swaps them back, because the widget
        // has already resolved reading order into a physical side.
        if rtl {
            format.SetReadingDirection(DWRITE_READING_DIRECTION_RIGHT_TO_LEFT)?;
        }
        format.SetTextAlignment(align)?;
        format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;

        // Overflowing text ends in "…" rather than being cut off hard.
        let sign = dwrite.CreateEllipsisTrimmingSign(&format)?;
        let trimming = DWRITE_TRIMMING {
            granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
            delimiter: 0,
            delimiterCount: 0,
        };
        format.SetTrimming(&trimming, &sign)?;
        Ok(format)
    }
}

/// Multi-line with word wrapping and no truncation.
fn tooltip_format(
    dwrite: &IDWriteFactory,
    size: f32,
    rtl: bool,
    family: &[u16],
    locale: &[u16],
) -> Result<IDWriteTextFormat> {
    unsafe {
        let format = dwrite.CreateTextFormat(
            PCWSTR(family.as_ptr()),
            None,
            DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            size,
            PCWSTR(locale.as_ptr()),
        )?;
        if rtl {
            format.SetReadingDirection(DWRITE_READING_DIRECTION_RIGHT_TO_LEFT)?;
        }
        format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_LEADING)?;
        // Aligned to the top: the height follows from the content, not the
        // other way round.
        format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR)?;
        Ok(format)
    }
}
