// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
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

use crate::platform;
use tpmplaner_core::anim::Animations;
use tpmplaner_core::i18n::Locale;
use tpmplaner_core::model::{Agenda, Event, Task};
use tpmplaner_core::sync::Status;
use tpmplaner_core::theme::{self, Appearance, Metrics, Palette, mix};

/// Converts the core's toolkit-neutral colour into the Direct2D one.
///
/// The portable crate must not know about Direct2D, so the conversion happens
/// here, at the one boundary where it belongs.
fn rgba(hex: u32, a: f32) -> D2D1_COLOR_F {
    let c = theme::rgba(hex, a);
    D2D1_COLOR_F {
        r: c.r,
        g: c.g,
        b: c.b,
        a: c.a,
    }
}
use chrono::{DateTime, Local, NaiveDate, Timelike};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::mem::ManuallyDrop;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_GRADIENT_STOP, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_TARGET, D2D1_BITMAP_PROPERTIES1,
    D2D1_BRUSH_PROPERTIES, D2D1_BUFFER_PRECISION_8BPC_UNORM, D2D1_CAP_STYLE_ROUND,
    D2D1_COLOR_INTERPOLATION_MODE_PREMULTIPLIED, D2D1_COLOR_SPACE_SRGB,
    D2D1_DEVICE_CONTEXT_OPTIONS_NONE, D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_ELLIPSE,
    D2D1_EXTEND_MODE_CLAMP, D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_LINE_JOIN_ROUND,
    D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES, D2D1_ROUNDED_RECT, D2D1_STROKE_STYLE_PROPERTIES1,
    D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE, D2D1CreateFactory, ID2D1Bitmap1, ID2D1Brush,
    ID2D1DeviceContext, ID2D1Factory1, ID2D1GradientStopCollection1, ID2D1SolidColorBrush,
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
    DWRITE_READING_DIRECTION_RIGHT_TO_LEFT, DWRITE_TEXT_ALIGNMENT, DWRITE_TEXT_ALIGNMENT_CENTER,
    DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_TEXT_METRICS,
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
use windows::core::{HRESULT, Interface, PCWSTR, Result, w};
use windows_numerics::{Matrix3x2, Vector2};

/// A clickable area, in DIPs relative to the window corner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Hit {
    Refresh,
    /// The circle before a task -> tick it off.
    TaskCheck(usize),
    /// A task row -> open it in the web interface.
    Task(usize),
    /// The row of a task whose completion can still be undone.
    Undo(usize),
    /// An event row -> open it in the calendar.
    Event(usize),
    /// The highlighted event in the header area.
    Hero(usize),
    /// An event from tomorrow's preview.
    Tomorrow(usize),
    /// A status line that needs action (setup, sign-in or configuration).
    StatusAction,
}

#[derive(Debug, Clone, Copy)]
pub struct HitRegion {
    pub rect: D2D_RECT_F,
    pub hit: Hit,
}

impl HitRegion {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.rect.left && x < self.rect.right && y >= self.rect.top && y < self.rect.bottom
    }
}

/// A task that has been ticked off but not yet sent, and can still be undone.
#[derive(Debug, Clone, Copy)]
pub struct UndoView<'a> {
    pub task_id: &'a str,
    /// 1.0 right after the click, 0.0 when it is sent.
    pub remaining: f32,
}

/// The drawing state the window passes in.
pub struct Frame<'a> {
    pub agenda: &'a Agenda,
    pub loc: &'a Locale,
    pub status: &'a Status,
    pub anim: &'a Animations,
    pub now: DateTime<Local>,
    pub hover: Option<Hit>,
    pub sync_minutes: u32,
    pub opacity: f32,
    pub show_past_events: bool,
    pub undo: Option<UndoView<'a>>,
    /// A syntax error in `config.json`; outranks the sync status line.
    pub config_error: Option<&'a str>,
    /// Version of a newer release, if the daily check found one.
    pub update: Option<&'a str>,
}

pub struct FrameResult {
    /// Total height of the content — the basis for limiting the scroll.
    pub content_height: f32,
    pub viewport_height: f32,
}

/// Type roles; the index addresses `Renderer::formats`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(usize)]
enum Font {
    Title = 0,
    Clock,
    Subtitle,
    Section,
    Row,
    RowStrong,
    Meta,
    MetaRight,
    Footer,
    Icon,
    /// Multi-line with wrapping and no truncation — for the tooltip only.
    Tooltip,
}
const FONT_COUNT: usize = 11;

pub struct Renderer {
    dcomp: IDCompositionDevice,
    _target: IDCompositionTarget,
    _visual: IDCompositionVisual,
    swapchain: IDXGISwapChain1,
    dc: ID2D1DeviceContext,
    dwrite: IDWriteFactory,
    _d2d_factory: ID2D1Factory1,

    formats: [IDWriteTextFormat; FONT_COUNT],
    round_stroke: ID2D1StrokeStyle,

    /// One brush per colour; opacity is set per use. At 60 Hz this saves a
    /// few hundred COM object creations per second.
    brushes: RefCell<HashMap<u32, ID2D1SolidColorBrush>>,
    /// Finished text layouts. DirectWrite layout is the most expensive single
    /// step in a frame, and the text does not change during an animation.
    layouts: RefCell<HashMap<(u64, u32, u32, u32), IDWriteTextLayout>>,

    /// Global opacity factor for the reveal animation.
    fade: Cell<f32>,
    /// Full text of the hovered row, if it was drawn truncated. Set while
    /// drawing and emitted as an overlay right at the end.
    tooltip: RefCell<Option<(String, f32, f32)>>,

    pal: Palette,
    /// The user's customisation, kept for the per-calendar colour overrides.
    /// Everything else it carries — typography, density, the surface style —
    /// is already baked into `formats`, `metrics` and `pal` by the time the
    /// renderer exists, which is why a change to any of it rebuilds it.
    custom: Appearance,
    /// Right-to-left layout. Mirroring happens exclusively in the drawing
    /// primitives; all the layout code goes on computing left to right,
    /// unchanged.
    rtl: bool,
    /// Mirror axis `panel.left + panel.right`, set once per frame.
    mirror: Cell<f32>,
    metrics: Metrics,
    dpi: f32,
    size_px: (u32, u32),
}

impl Renderer {
    /// `metrics` is passed in rather than derived from a scale factor: the
    /// window sizes itself from the same values — the shadow margin decides
    /// how much larger the window is than the visible panel — and the two
    /// drifting apart is how the panel ends up inset inside its own window.
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
            // Both kept alive for the whole block: every format borrows them.
            let family = font_family(custom);
            let section_weight = DWRITE_FONT_WEIGHT(custom.header_weight().0 as i32);

            let formats = [
                text_format(
                    &dwrite,
                    m.fs_title,
                    DWRITE_FONT_WEIGHT_SEMI_BOLD,
                    DWRITE_TEXT_ALIGNMENT_LEADING,
                    rtl,
                    &family,
                )?,
                text_format(
                    &dwrite,
                    m.fs_clock,
                    DWRITE_FONT_WEIGHT_NORMAL,
                    DWRITE_TEXT_ALIGNMENT_TRAILING,
                    rtl,
                    &family,
                )?,
                text_format(
                    &dwrite,
                    m.fs_subtitle,
                    DWRITE_FONT_WEIGHT_NORMAL,
                    DWRITE_TEXT_ALIGNMENT_LEADING,
                    rtl,
                    &family,
                )?,
                text_format(
                    &dwrite,
                    m.fs_section,
                    section_weight,
                    DWRITE_TEXT_ALIGNMENT_LEADING,
                    rtl,
                    &family,
                )?,
                text_format(
                    &dwrite,
                    m.fs_row,
                    DWRITE_FONT_WEIGHT_NORMAL,
                    DWRITE_TEXT_ALIGNMENT_LEADING,
                    rtl,
                    &family,
                )?,
                text_format(
                    &dwrite,
                    m.fs_row,
                    DWRITE_FONT_WEIGHT_SEMI_BOLD,
                    DWRITE_TEXT_ALIGNMENT_LEADING,
                    rtl,
                    &family,
                )?,
                text_format(
                    &dwrite,
                    m.fs_meta,
                    DWRITE_FONT_WEIGHT_NORMAL,
                    DWRITE_TEXT_ALIGNMENT_LEADING,
                    rtl,
                    &family,
                )?,
                text_format(
                    &dwrite,
                    m.fs_meta,
                    DWRITE_FONT_WEIGHT_NORMAL,
                    DWRITE_TEXT_ALIGNMENT_TRAILING,
                    rtl,
                    &family,
                )?,
                text_format(
                    &dwrite,
                    m.fs_footer,
                    DWRITE_FONT_WEIGHT_NORMAL,
                    DWRITE_TEXT_ALIGNMENT_LEADING,
                    rtl,
                    &family,
                )?,
                icon_format(&dwrite, m.fs_row + 1.5)?,
                tooltip_format(&dwrite, m.fs_row, rtl, &family)?,
            ];

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
                fade: Cell::new(1.0),
                tooltip: RefCell::new(None),
                pal,
                custom: custom.clone(),
                rtl,
                mirror: Cell::new(0.0),
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

    /// The colour to draw one event's calendar in.
    ///
    /// The provider's own colour by default, so the widget matches the web
    /// calendar. A configured override wins, because the provider's palette
    /// was not chosen to be told apart at 0.82 opacity on a wallpaper.
    fn calendar_color(&self, ev: &Event) -> u32 {
        self.custom
            .calendar_color(&ev.calendar_id, &ev.calendar_name)
            .unwrap_or(ev.color)
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
    pub fn draw(&mut self, frame: &Frame, hits: &mut Vec<HitRegion>) -> Result<FrameResult> {
        let (w, h) = self.size_dip();
        let m = self.metrics;
        hits.clear();

        // The glass body is inset by the shadow margin.
        let panel = rect(m.shadow, m.shadow, w - m.shadow, h - m.shadow);
        let cx0 = panel.left + m.pad;
        let cx1 = panel.right - m.pad;
        // Axis for the RTL mirroring: everything inside the glass body is
        // folded across it, and the body maps onto itself.
        self.mirror.set(panel.left + panel.right);

        unsafe {
            self.dc.BeginDraw();
            self.dc.Clear(Some(&rgba(0, 0.0)));
            self.tooltip.replace(None);

            self.draw_shadow(panel)?;
            self.draw_panel(panel, self.pal.opacity(frame.opacity))?;

            let mut y = panel.top;
            y = self.draw_header(cx0, cx1, y, frame, hits)?;
            y = self.draw_hero(cx0, cx1, y, frame, hits)?;

            let content_top = y;
            let content_bottom = panel.bottom - m.footer_h;

            // Everything between is clipped so scrolled rows cannot run into
            // the header, the footer or the frame.
            self.dc.PushAxisAlignedClip(
                &rect(
                    panel.left + 1.0,
                    content_top,
                    panel.right - 1.0,
                    content_bottom,
                ),
                Default::default(),
            );

            // Reveal: slide up slightly from below while fading in.
            let reveal = frame.anim.reveal.value;
            self.fade.set(reveal.clamp(0.0, 1.0));
            let slide = (1.0 - reveal) * 10.0;

            let mut cy = content_top - frame.anim.scroll.value + slide;
            cy = self.draw_events_section(cx0, cx1, cy, frame, hits)?;
            cy = self.draw_tasks_section(cx0, cx1, cy, frame, hits)?;

            self.fade.set(1.0);
            self.dc.PopAxisAlignedClip();

            let content_height = cy + frame.anim.scroll.value - slide - content_top + m.pad;
            let viewport_height = content_bottom - content_top;

            self.draw_scrollbar(
                panel,
                content_top,
                content_bottom,
                content_height,
                viewport_height,
                frame,
            )?;
            self.draw_footer(panel, cx0, cx1, frame, hits)?;
            // Last of all, so the overlay sits above everything and is not
            // cut off by the content clip.
            self.draw_tooltip(panel, content_top, content_bottom)?;

            self.dc.EndDraw(None, None)?;
            self.swapchain.Present(1, DXGI_PRESENT(0)).ok()?;
            self.dcomp.Commit()?;

            // Keep this from growing without bound: the relative times
            // ("in 25 min") produce new keys every minute.
            let mut layouts = self.layouts.borrow_mut();
            if layouts.len() > 512 {
                layouts.clear();
            }

            Ok(FrameResult {
                content_height,
                viewport_height,
            })
        }
    }

    // --- Glass body ---------------------------------------------------------

    /// A soft drop shadow made of stacked rounded rectangles growing
    /// outwards.
    ///
    /// A real Gaussian blur through `ID2D1Effect` would be cleaner, but needs
    /// an intermediate bitmap and a full effect graph every frame. With twelve
    /// low-opacity fills the result is indistinguishable at this size, and far
    /// cheaper.
    fn draw_shadow(&self, panel: D2D_RECT_F) -> Result<()> {
        let m = self.metrics;
        // A contrast theme has no shadow: it softens exactly the edge that
        // this mode wants hard. Nor is there one when no margin was reserved
        // for it — the system backdrop fills the whole window rectangle, and
        // twelve rectangles that cannot grow outwards are not a soft edge but
        // twelve coats of black over the panel.
        if self.pal.shadow_alpha <= 0.0 || m.shadow <= 0.0 {
            return Ok(());
        }
        let steps = 12;
        let brush = self.brush(0x00_0000, self.pal.shadow_alpha)?;
        unsafe {
            for i in 0..steps {
                let t = i as f32 / (steps - 1) as f32;
                let grow = m.shadow * (1.0 - t);
                self.dc.FillRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        // Offset slightly downwards: the light comes from above.
                        rect: rect(
                            panel.left - grow,
                            panel.top - grow * 0.6,
                            panel.right + grow,
                            panel.bottom + grow * 1.2,
                        ),
                        radiusX: m.corner + grow,
                        radiusY: m.corner + grow,
                    },
                    &brush,
                );
            }
        }
        Ok(())
    }

    /// Gradient, gloss arc and the double edge.
    fn draw_panel(&self, panel: D2D_RECT_F, opacity: f32) -> Result<()> {
        unsafe {
            let m = self.metrics;
            let p = &self.pal;
            let body = D2D1_ROUNDED_RECT {
                rect: panel,
                radiusX: m.corner,
                radiusY: m.corner,
            };

            let stops = [
                D2D1_GRADIENT_STOP {
                    position: 0.0,
                    color: rgba(p.panel_top, opacity),
                },
                D2D1_GRADIENT_STOP {
                    position: 0.35,
                    color: rgba(p.panel_mid, opacity),
                },
                D2D1_GRADIENT_STOP {
                    position: 1.0,
                    color: rgba(p.panel_bottom, (opacity * 1.02).min(1.0)),
                },
            ];
            let gradient = self.dc.CreateLinearGradientBrush(
                &D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                    startPoint: point(0.0, panel.top),
                    endPoint: point(0.0, panel.bottom),
                },
                None::<*const D2D1_BRUSH_PROPERTIES>,
                &self.gradient_stops(&stops)?,
            )?;
            self.dc.FillRoundedRectangle(&body, &gradient);

            // Gloss arc: a light gradient across the top that fades out
            // completely towards the bottom. Dropped in a contrast theme.
            if p.sheen_gloss > 0.0 {
                let gloss_h = (panel.bottom - panel.top) * 0.30;
                let gloss_stops = [
                    D2D1_GRADIENT_STOP {
                        position: 0.0,
                        color: rgba(p.sheen, p.sheen_gloss),
                    },
                    D2D1_GRADIENT_STOP {
                        position: 0.55,
                        color: rgba(p.sheen, p.sheen_gloss * 0.32),
                    },
                    D2D1_GRADIENT_STOP {
                        position: 1.0,
                        color: rgba(p.sheen, 0.0),
                    },
                ];
                let gloss = self.dc.CreateLinearGradientBrush(
                    &D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                        startPoint: point(0.0, panel.top),
                        endPoint: point(0.0, panel.top + gloss_h),
                    },
                    None::<*const D2D1_BRUSH_PROPERTIES>,
                    &self.gradient_stops(&gloss_stops)?,
                )?;
                self.dc.FillRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: rect(panel.left, panel.top, panel.right, panel.top + gloss_h),
                        radiusX: m.corner,
                        radiusY: m.corner,
                    },
                    &gloss,
                );
            }

            // Light inside, dark outside — that contrast is what makes the
            // edge look raised. Both edges are skipped when their alpha is
            // zero: a flat or borderless surface removes them by setting it,
            // and a fully transparent stroke is work with nothing to show for
            // it.
            if p.sheen_border > 0.0 {
                self.dc.DrawRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: inset(panel, 1.0),
                        radiusX: m.corner - 1.0,
                        radiusY: m.corner - 1.0,
                    },
                    &self.brush(p.sheen, p.sheen_border)?,
                    1.0,
                    None,
                );

                // A narrow strip of light right at the top, like the
                // reflection on the edge of a sheet of glass.
                self.dc.DrawLine(
                    point(panel.left + m.corner, panel.top + 1.0),
                    point(panel.right - m.corner, panel.top + 1.0),
                    &self.brush(p.sheen, p.sheen_border * 1.4)?,
                    1.0,
                    None,
                );
            }
            if p.border_outer_alpha > 0.0 {
                self.dc.DrawRoundedRectangle(
                    &body,
                    &self.brush(p.border_outer, p.border_outer_alpha)?,
                    1.0,
                    None,
                );
            }
            Ok(())
        }
    }

    // --- Header -------------------------------------------------------------

    fn draw_header(
        &self,
        cx0: f32,
        cx1: f32,
        top: f32,
        frame: &Frame,
        hits: &mut Vec<HitRegion>,
    ) -> Result<f32> {
        let m = self.metrics;
        let p = &self.pal;
        let loc = frame.loc;
        let today = frame.agenda.day.unwrap_or_else(|| frame.now.date_naive());
        let bottom = top + m.header_h;

        // Refresh button
        let btn = rect(
            cx1 - m.icon_btn,
            top + m.pad * 0.65,
            cx1,
            top + m.pad * 0.65 + m.icon_btn,
        );
        let hovered = frame.hover == Some(Hit::Refresh);
        let spinning = matches!(frame.status, Status::Syncing);

        if hovered {
            let a = p.hover_alpha * (1.0 + frame.anim.hover.value);
            self.fill_round(btn, m.icon_btn * 0.5, p.hover, a)?;
        }

        // \u{E72C} = "Refresh" in Segoe Fluent Icons / MDL2 Assets.
        let angle = frame.anim.spinner;
        let bx = self.mx((btn.left + btn.right) * 0.5);
        let by = (btn.top + btn.bottom) * 0.5;
        unsafe {
            if angle != 0.0 {
                self.dc.SetTransform(&rotation(angle, bx, by));
            }
        }
        self.text(
            "\u{E72C}",
            Font::Icon,
            btn,
            if spinning { p.accent } else { p.text_secondary },
            if hovered || spinning { 1.0 } else { 0.7 },
        )?;
        unsafe {
            if angle != 0.0 {
                self.dc.SetTransform(&Matrix3x2::identity());
            }
        }
        hits.push(HitRegion {
            rect: btn,
            hit: Hit::Refresh,
        });

        // Weekday large, date small underneath.
        let title_y = top + m.pad * 0.55;
        self.text(
            &loc.weekday(today),
            Font::Title,
            rect(
                cx0,
                title_y,
                cx1 - m.icon_btn - 60.0,
                title_y + m.fs_title * 1.45,
            ),
            p.text_primary,
            1.0,
        )?;
        self.text(
            &loc.date_line(today),
            Font::Subtitle,
            rect(cx0, title_y + m.fs_title * 1.35, cx1, bottom),
            p.text_dim,
            1.0,
        )?;

        // The clock on the right, level with the date line. It is the reason
        // the widget redraws every minute.
        self.text(
            &loc.time(frame.now),
            Font::Clock,
            rect(
                cx0,
                title_y + m.fs_title * 1.3,
                cx1,
                title_y + m.fs_title * 1.3 + m.fs_clock * 1.5,
            ),
            p.text_secondary,
            0.9,
        )?;

        self.draw_day_rail(cx0, cx1, bottom - m.rail_h - 5.0, frame)?;

        let line_y = bottom - 1.0;
        self.line(cx0, line_y, cx1, line_y, p.rule, p.rule_alpha, 1.0, false)?;
        Ok(bottom)
    }

    /// The day rail: the whole day from 06:00 to 22:00 on a single strip.
    ///
    /// It shows the *shape* of the day, which a list cannot — clustered
    /// mornings, free afternoons, how long something runs. Every event is a
    /// segment in its calendar colour, at the right place and the right width;
    /// the elapsed time is tinted, and a marker shows where you are now.
    fn draw_day_rail(&self, x0: f32, x1: f32, y: f32, frame: &Frame) -> Result<()> {
        let m = self.metrics;
        let p = &self.pal;
        let now_min = frame.now.hour() as f32 * 60.0 + frame.now.minute() as f32;

        // The default window is the working day, but it stretches to cover
        // whatever is actually on. A fixed 06:00-22:00 would squash the 05:00
        // flight and the 23:00 event against the edges — precisely the
        // outliers worth seeing.
        let (mut lo, mut hi) = (6.0 * 60.0_f32, 22.0 * 60.0_f32);
        for ev in &frame.agenda.events {
            if ev.all_day {
                continue;
            }
            if let Some(start) = ev.start {
                let s_min = start.hour() as f32 * 60.0 + start.minute() as f32;
                lo = lo.min(s_min);
                hi = hi.max(end_minutes(ev, s_min));
            }
        }
        lo = lo.min(now_min).max(0.0);
        hi = hi.max(now_min).min(24.0 * 60.0);
        // On a very empty day the span would otherwise degenerate.
        if hi - lo < 240.0 {
            hi = (lo + 240.0).min(24.0 * 60.0);
            lo = (hi - 240.0).max(0.0);
        }
        let span = hi - lo;

        let width = x1 - x0;
        let pos = |minutes: f32| x0 + width * ((minutes - lo) / span).clamp(0.0, 1.0);

        let track = rect(x0, y, x1, y + m.rail_h);
        let radius = m.rail_h * 0.5;
        self.fill_round(track, radius, p.rule, p.rule_alpha * 0.9)?;

        let now_x = pos(now_min);

        // The elapsed part of the day. Deliberately very restrained: the rail
        // should be readable in passing, not compete for attention like a
        // progress bar — with a strong accent colour a wide coloured stripe
        // would be all you see.
        if now_x > x0 + 0.5 {
            self.fill_round(rect(x0, y, now_x, y + m.rail_h), radius, p.accent, 0.09)?;
        }

        for ev in &frame.agenda.events {
            if ev.all_day {
                continue;
            }
            let Some(start) = ev.start else { continue };
            let start_min = start.hour() as f32 * 60.0 + start.minute() as f32;
            let (sx, ex) = (pos(start_min), pos(end_minutes(ev, start_min)));
            // Short events would otherwise be invisible.
            let ex = ex.max(sx + 2.5);
            let running = ev.is_now(frame.now);
            let color = if self.pal.high_contrast {
                p.text_primary
            } else if running {
                p.accent
            } else {
                self.calendar_color(ev)
            };
            self.fill_round(
                rect(sx, y + 1.0, ex.min(x1), y + m.rail_h - 1.0),
                (m.rail_h - 2.0) * 0.5,
                color,
                if running { 1.0 } else { 0.85 },
            )?;
        }

        // The now marker goes on top so no segment can ever hide it.
        if (lo..=hi).contains(&now_min) {
            self.line(
                now_x,
                y - 2.0,
                now_x,
                y + m.rail_h + 2.0,
                if p.dark { 0xFF_FFFF } else { 0x00_0000 },
                0.85,
                2.0,
                true,
            )?;
        }
        Ok(())
    }

    /// The highlighted event: the one running now, otherwise the next one.
    ///
    /// This is the one piece of information you would otherwise have to go
    /// looking for — it belongs at the very top, not in a list.
    fn draw_hero(
        &self,
        cx0: f32,
        cx1: f32,
        top: f32,
        frame: &Frame,
        hits: &mut Vec<HitRegion>,
    ) -> Result<f32> {
        let m = self.metrics;
        let p = &self.pal;
        let loc = frame.loc;
        let Some((idx, ev, running)) = pick_hero(&frame.agenda.events, frame.now) else {
            return Ok(top);
        };

        let card = rect(cx0, top + m.pad * 0.55, cx1, top + m.pad * 0.55 + m.hero_h);
        let hovered = frame.hover == Some(Hit::Hero(idx));

        // A running event strongly, an upcoming one quietly.
        let tint = if running { p.accent_soft } else { p.panel_top };
        let base = if running { 0.70 } else { 0.42 };
        let a = base
            + if hovered {
                0.12 * frame.anim.hover.value
            } else {
                0.0
            };
        self.fill_round(card, 6.0, tint, a)?;

        self.stroke_round(
            card,
            6.0,
            if running { p.accent } else { p.rule },
            if running { 0.40 } else { p.rule_alpha },
            1.0,
        )?;
        // Accent bar on the side the reading starts from.
        self.fill_round(
            rect(
                card.left + 4.0,
                card.top + 7.0,
                card.left + 7.0,
                card.bottom - 7.0,
            ),
            1.5,
            if running { p.accent } else { p.text_dim },
            0.95,
        )?;

        let tx = card.left + 15.0;
        self.text(
            &loc.label(if running {
                loc.cat.now_label
            } else {
                loc.cat.next_label
            }),
            Font::Section,
            rect(
                tx,
                card.top + 5.0,
                cx1 - 8.0,
                card.top + 5.0 + m.fs_section * 1.6,
            ),
            if running { p.accent } else { p.text_dim },
            1.0,
        )?;

        // Countdown on the right: to the end while an event runs, otherwise
        // to the start.
        let rel = if running {
            ev.end
                .map(|e| loc.time_left((e - frame.now).num_minutes()))
                .unwrap_or_else(|| loc.label(loc.cat.running))
        } else {
            ev.start
                .map(|s| loc.relative((s - frame.now).num_minutes()))
                .unwrap_or_default()
        };
        self.text(
            &rel,
            Font::MetaRight,
            rect(
                tx,
                card.top + 5.0,
                cx1 - 9.0,
                card.top + 5.0 + m.fs_section * 1.6,
            ),
            if running { p.accent } else { p.text_secondary },
            1.0,
        )?;

        let time_prefix = if ev.all_day {
            String::new()
        } else {
            match (ev.start, ev.end) {
                (Some(s), Some(e)) => format!("{}–{}  ", loc.time(s), loc.time(e)),
                (Some(s), None) => format!("{}  ", loc.time(s)),
                _ => String::new(),
            }
        };
        self.text(
            &format!("{time_prefix}{}", ev.title),
            Font::RowStrong,
            rect(
                tx,
                card.bottom - m.fs_row * 2.0,
                cx1 - 9.0,
                card.bottom - 4.0,
            ),
            p.text_primary,
            1.0,
        )?;

        // Progress of the running event as a fine line at the foot of the card.
        if running && let (Some(s), Some(e)) = (ev.start, ev.end) {
            let total = (e - s).num_seconds().max(1) as f32;
            let done = ((frame.now - s).num_seconds().max(0) as f32 / total).clamp(0.0, 1.0);
            let y = card.bottom - 2.5;
            self.line(
                card.left + 8.0,
                y,
                card.right - 8.0,
                y,
                p.accent,
                0.16,
                2.0,
                true,
            )?;
            if done > 0.005 {
                self.line(
                    card.left + 8.0,
                    y,
                    card.left + 8.0 + (card.right - card.left - 16.0) * done,
                    y,
                    p.accent,
                    0.85,
                    2.0,
                    true,
                )?;
            }
        }

        hits.push(HitRegion {
            rect: card,
            hit: Hit::Hero(idx),
        });
        Ok(card.bottom)
    }

    // --- Lists --------------------------------------------------------------

    fn draw_events_section(
        &self,
        x0: f32,
        x1: f32,
        mut y: f32,
        frame: &Frame,
        hits: &mut Vec<HitRegion>,
    ) -> Result<f32> {
        let m = self.metrics;
        let p = &self.pal;
        let loc = frame.loc;
        let now = frame.now;

        // The running event always stays visible, even when past ones are
        // hidden — otherwise the most important one is the one that vanishes.
        let visible: Vec<(usize, &Event)> = frame
            .agenda
            .events
            .iter()
            .enumerate()
            .filter(|(_, e)| frame.show_past_events || !e.is_past(now) || e.is_now(now))
            .collect();

        // Double bookings are hard to spot when skimming a list — you would
        // have to compare the end of one row with the start of the next in
        // your head.
        let overlapping = tpmplaner_core::model::mark_overlaps(&frame.agenda.events);
        let conflicts = overlapping.iter().filter(|&&f| f).count() / 2;

        let mut badges: Vec<(String, u32)> = Vec::new();
        if conflicts > 0 {
            badges.push((loc.conflicts(conflicts), p.conflict));
        }

        y += m.section_gap;
        y = self.section_label(
            x0,
            x1,
            y,
            &loc.label(loc.cat.section_events),
            visible.len(),
            &badges,
        )?;

        if visible.is_empty() {
            self.text(
                &loc.label(loc.cat.no_events),
                Font::Row,
                rect(x0 + 2.0, y, x1, y + m.event_row_h),
                p.text_faint,
                1.0,
            )?;
            return Ok(y + m.event_row_h);
        }

        // Only show the calendar name when more than one is in play —
        // otherwise it is the same redundant label on every row.
        let multi_cal = visible
            .iter()
            .filter(|(_, e)| !e.all_day)
            .map(|(_, e)| e.calendar_name.as_str())
            .collect::<std::collections::HashSet<_>>()
            .len()
            > 1;

        // Column widths from the actual content rather than fixed values —
        // otherwise the layout only fits one language.
        let time_labels: Vec<String> = visible
            .iter()
            .map(|(_, e)| {
                if e.all_day {
                    loc.label(loc.cat.all_day)
                } else {
                    e.start.map(|s| loc.time(s)).unwrap_or_else(|| "—".into())
                }
            })
            .collect();
        let time_w = self.column_width(
            time_labels.iter().map(String::as_str),
            Font::Meta,
            m.time_col_w,
            m.time_col_w * 2.0,
        );
        let rel_labels: Vec<String> = visible
            .iter()
            .map(|(_, e)| {
                if e.is_now(now) {
                    loc.label(loc.cat.running)
                } else if e.is_past(now) {
                    e.calendar_name.clone()
                } else {
                    e.start
                        .map(|s| loc.relative((s - now).num_minutes()))
                        .unwrap_or_default()
                }
            })
            .collect();
        let rel_w = self.column_width(
            rel_labels.iter().map(String::as_str),
            Font::MetaRight,
            m.rel_col_w * 0.6,
            m.rel_col_w * 1.6,
        );

        // Where the now line goes: before the first event still to come.
        let now_line_before = visible
            .iter()
            .position(|(_, e)| !e.all_day && e.start.map(|s| s > now).unwrap_or(false));

        for (slot, (idx, ev)) in visible.iter().enumerate() {
            if now_line_before == Some(slot) {
                self.draw_now_line(x0, x1, y, frame)?;
                y += m.nowline_h;
            }

            let row = rect(x0 - 5.0, y, x1 + 5.0, y + m.event_row_h);
            let is_now = ev.is_now(now);
            let past = ev.is_past(now) && !is_now;
            // Held back, but still comfortable to read.
            let dim: f32 = if past { 0.55 } else { 1.0 };

            if frame.hover == Some(Hit::Event(*idx)) {
                self.fill_round(row, 5.0, p.hover, p.hover_alpha * frame.anim.hover.value)?;
            }
            if is_now {
                self.fill_round(row, 5.0, p.accent, 0.10)?;
            }

            // Colour marker for the source calendar.
            // A contrast theme replaces it with the system colour — calendar
            // colours carry no guaranteed contrast there.
            let bar_color = if self.pal.high_contrast {
                if is_now { p.accent } else { p.text_primary }
            } else if is_now {
                p.accent
            } else {
                self.calendar_color(ev)
            };
            self.fill_round(
                rect(x0, y + 6.0, x0 + 3.0, y + m.event_row_h - 6.0),
                1.5,
                bar_color,
                dim,
            )?;

            let tx = x0 + 11.0;
            // Overlapping events get a warning colour on the time — that is
            // where you look when hunting for conflicts.
            let conflicted = overlapping.get(*idx).copied().unwrap_or(false);
            self.text(
                &time_labels[slot],
                Font::Meta,
                rect(tx, y, tx + time_w, y + m.event_row_h),
                if is_now {
                    p.accent
                } else if conflicted {
                    p.conflict
                } else {
                    p.text_dim
                },
                dim,
            )?;

            // Right column: time remaining, or for past events the calendar
            // name (time remaining would be noise there).
            let rel_x = x1 - rel_w;
            if !ev.all_day && !past {
                let rel = if is_now {
                    loc.cat.running.to_string()
                } else {
                    ev.start
                        .map(|s| loc.relative((s - now).num_minutes()))
                        .unwrap_or_default()
                };
                self.text(
                    &rel,
                    Font::MetaRight,
                    rect(rel_x, y, x1, y + m.event_row_h),
                    if is_now { p.accent } else { p.text_faint },
                    dim,
                )?;
            } else if multi_cal && !ev.calendar_name.is_empty() {
                self.text(
                    &ev.calendar_name,
                    Font::MetaRight,
                    rect(rel_x, y, x1, y + m.event_row_h),
                    p.text_faint,
                    dim,
                )?;
            }

            let title_x = tx + time_w;
            let title = match &ev.location {
                Some(loc) => format!("{}  ·  {loc}", ev.title),
                None => ev.title.clone(),
            };
            let title_font = if is_now { Font::RowStrong } else { Font::Row };
            self.note_truncation(
                frame.hover == Some(Hit::Event(*idx)),
                &title,
                title_font,
                rel_x - 6.0 - title_x,
                row,
            );
            self.text(
                &title,
                title_font,
                rect(title_x, y, rel_x - 6.0, y + m.event_row_h),
                p.text_primary,
                dim,
            )?;

            hits.push(HitRegion {
                rect: row,
                hit: Hit::Event(*idx),
            });
            y += m.event_row_h + m.row_gap;
        }

        // Every event is over: the line goes to the end.
        if now_line_before.is_none() && visible.iter().any(|(_, e)| !e.all_day) {
            self.draw_now_line(x0, x1, y, frame)?;
            y += m.nowline_h;
        }

        y = self.draw_tomorrow(x0, x1, y, frame, hits)?;
        Ok(y)
    }

    /// A look ahead to tomorrow.
    ///
    /// From late afternoon today's list is empty and the widget would be a
    /// blank surface — while that is exactly when "what is first tomorrow?"
    /// becomes the interesting question. At most two events are shown, so the
    /// preview never crowds out today.
    fn draw_tomorrow(
        &self,
        x0: f32,
        x1: f32,
        mut y: f32,
        frame: &Frame,
        hits: &mut Vec<HitRegion>,
    ) -> Result<f32> {
        let m = self.metrics;
        let p = &self.pal;
        let loc = frame.loc;
        let upcoming: Vec<(usize, &Event)> = frame
            .agenda
            .tomorrow
            .iter()
            .enumerate()
            .filter(|(_, e)| !e.all_day)
            .take(2)
            .collect();
        if upcoming.is_empty() {
            return Ok(y);
        }

        y += m.section_gap * 0.7;
        self.line(x0, y, x1, y, p.rule, p.rule_alpha * 0.7, 1.0, false)?;
        y += 4.0;

        let label = loc.label(loc.cat.tomorrow);
        let label_w = self.text_width(&label, Font::Section) + 10.0;
        self.text(
            &label,
            Font::Section,
            rect(x0, y, x0 + label_w, y + m.event_row_h),
            p.text_faint,
            1.0,
        )?;

        for (i, (idx, ev)) in upcoming.iter().enumerate() {
            let row = rect(x0 - 5.0, y, x1 + 5.0, y + m.event_row_h);
            if frame.hover == Some(Hit::Tomorrow(*idx)) {
                self.fill_round(row, 5.0, p.hover, p.hover_alpha * frame.anim.hover.value)?;
            }
            // The label sits on the first row only; the second is indented
            // underneath it.
            let tx = x0 + label_w + 4.0;
            let time = ev.start.map(|s| loc.time(s)).unwrap_or_default();
            let time_w = self.text_width(&time, Font::Meta) + 8.0;
            self.text(
                &time,
                Font::Meta,
                rect(tx, y, tx + time_w, y + m.event_row_h),
                p.text_faint,
                1.0,
            )?;
            self.text(
                &ev.title,
                Font::Row,
                rect(tx + time_w, y, x1, y + m.event_row_h),
                p.text_dim,
                1.0,
            )?;
            hits.push(HitRegion {
                rect: row,
                hit: Hit::Tomorrow(*idx),
            });
            y += m.event_row_h * 0.85;
            let _ = i;
        }
        Ok(y)
    }

    /// A thin line with a time marker showing where in the day you are.
    fn draw_now_line(&self, x0: f32, x1: f32, y: f32, frame: &Frame) -> Result<()> {
        let m = self.metrics;
        let cy = y + m.nowline_h * 0.5;
        // Depending on the locale the time can be noticeably wider
        // ("8:09 PM" rather than "20:09"), so this is measured generously.
        let label_w = m.fs_meta * 4.2;

        self.ellipse(x0 + 2.0, cy, 2.5, self.pal.accent, 0.9, true, 0.0)?;
        self.line(
            x0 + 6.0,
            cy,
            x1 - label_w - 5.0,
            cy,
            self.pal.accent,
            0.30,
            1.0,
            false,
        )?;
        self.text(
            &frame.loc.time(frame.now),
            Font::MetaRight,
            rect(x1 - label_w, y, x1, y + m.nowline_h),
            self.pal.accent,
            0.85,
        )
    }

    fn draw_tasks_section(
        &self,
        x0: f32,
        x1: f32,
        mut y: f32,
        frame: &Frame,
        hits: &mut Vec<HitRegion>,
    ) -> Result<f32> {
        let m = self.metrics;
        let p = &self.pal;
        let loc = frame.loc;
        let today = frame.agenda.day.unwrap_or_else(|| frame.now.date_naive());
        let tasks = &frame.agenda.tasks;
        let overdue = tasks.iter().filter(|t| t.is_overdue(today)).count();

        y += m.section_gap;
        let badges: Vec<(String, u32)> = if overdue > 0 {
            vec![(loc.overdue(overdue), p.overdue)]
        } else {
            Vec::new()
        };
        y = self.section_label(
            x0,
            x1,
            y,
            &loc.label(loc.cat.section_tasks),
            tasks.len(),
            &badges,
        )?;

        if tasks.is_empty() {
            self.text(
                &loc.label(loc.cat.no_tasks),
                Font::Row,
                rect(x0 + 2.0, y, x1, y + m.task_row_h),
                p.text_faint,
                1.0,
            )?;
            return Ok(y + m.task_row_h);
        }

        // Only show the list name when more than one task list is involved.
        let multi_list = tasks
            .iter()
            .map(|t| t.tasklist_name.as_str())
            .collect::<std::collections::HashSet<_>>()
            .len()
            > 1;

        let due_labels: Vec<String> = tasks.iter().map(|t| due_label(t, today, loc)).collect();
        let due_w = self.column_width(
            due_labels.iter().map(String::as_str),
            Font::Meta,
            m.time_col_w,
            m.time_col_w * 2.2,
        );
        // The undo hint and the list name share the right column.
        let list_names: Vec<&str> = if multi_list {
            tasks.iter().map(|t| t.tasklist_name.as_str()).collect()
        } else {
            Vec::new()
        };
        let right_w = self.column_width(
            std::iter::once(loc.cat.undo).chain(list_names.iter().copied()),
            Font::MetaRight,
            m.rel_col_w * 0.6,
            m.rel_col_w * 1.8,
        );

        for (idx, task) in tasks.iter().enumerate() {
            let row = rect(x0 - 5.0, y, x1 + 5.0, y + m.task_row_h);
            let is_overdue = task.is_overdue(today);
            let undo = frame.undo.filter(|u| u.task_id == task.id);
            // While the tick is pending: fade out at once, so the click feels
            // immediate instead of waiting on the API.
            let dim: f32 = if task.completing { 0.42 } else { 1.0 };
            let check_hovered = frame.hover == Some(Hit::TaskCheck(idx));
            let row_hovered = check_hovered
                || frame.hover == Some(Hit::Task(idx))
                || frame.hover == Some(Hit::Undo(idx));

            if row_hovered {
                self.fill_round(row, 5.0, p.hover, p.hover_alpha * frame.anim.hover.value)?;
            }

            let indent = task.depth as f32 * m.indent;
            let ccx = x0 + indent + m.check_size * 0.5 + 1.0;
            let ccy = y + m.task_row_h * 0.5;
            self.draw_check(ccx, ccy, task, is_overdue, check_hovered, frame)?;

            let due_x = ccx + m.check_size * 0.5 + 9.0;
            self.text(
                &due_labels[idx],
                Font::Meta,
                rect(due_x, y, due_x + due_w, y + m.task_row_h),
                if is_overdue { p.overdue } else { p.text_dim },
                dim,
            )?;

            // Right column: the undo hint beats the list name.
            let mut title_right = x1;
            if let Some(u) = undo {
                title_right = x1 - right_w - 6.0;
                self.text(
                    &loc.label(loc.cat.undo),
                    Font::MetaRight,
                    rect(x1 - right_w, y, x1, y + m.task_row_h),
                    p.accent,
                    1.0,
                )?;
                // Countdown bar under the row.
                let by = y + m.task_row_h - 2.0;
                self.line(
                    x1 - right_w,
                    by,
                    x1 - right_w + right_w * u.remaining,
                    by,
                    p.accent,
                    0.55,
                    1.5,
                    true,
                )?;
            } else if multi_list && !task.tasklist_name.is_empty() {
                title_right = x1 - right_w - 6.0;
                self.text(
                    &task.tasklist_name,
                    Font::MetaRight,
                    rect(x1 - right_w, y, x1, y + m.task_row_h),
                    p.text_faint,
                    dim,
                )?;
            }

            // The note as a muted continuation after the title — the same
            // treatment the location gets on events. First line only; the rest
            // would be cut off anyway.
            let title_x = due_x + due_w;
            let title = match task.notes.as_deref().and_then(|n| n.lines().next()) {
                Some(note) if !note.trim().is_empty() => {
                    format!("{}  ·  {}", task.title, note.trim())
                }
                _ => task.title.clone(),
            };
            self.note_truncation(
                row_hovered && undo.is_none(),
                &title,
                Font::Row,
                title_right - title_x,
                row,
            );
            self.text(
                &title,
                Font::Row,
                rect(title_x, y, title_right, y + m.task_row_h),
                p.text_primary,
                dim,
            )?;

            // Order matters: regions pushed later win. While a tick is
            // pending the whole row catches the click, so a mistake can be
            // taken back from anywhere on it.
            hits.push(HitRegion {
                rect: row,
                hit: Hit::Task(idx),
            });
            hits.push(HitRegion {
                rect: rect(x0 - 5.0, y, ccx + m.check_size, y + m.task_row_h),
                hit: Hit::TaskCheck(idx),
            });
            if undo.is_some() {
                hits.push(HitRegion {
                    rect: row,
                    hit: Hit::Undo(idx),
                });
            }

            y += m.task_row_h + m.row_gap;
        }

        Ok(y)
    }

    /// The tick circle. On sending it fills and gains a check mark.
    fn draw_check(
        &self,
        cx: f32,
        cy: f32,
        task: &Task,
        overdue: bool,
        hovered: bool,
        frame: &Frame,
    ) -> Result<()> {
        let m = self.metrics;
        let p = &self.pal;
        let r = m.check_size * 0.5;
        let resting = if overdue { p.overdue } else { p.text_dim };
        let color = if task.completing {
            p.ok_green
        } else if hovered {
            // Fade in through the hover animation rather than switching hard.
            mix(resting, p.accent, frame.anim.hover.value)
        } else {
            resting
        };

        self.ellipse(
            cx,
            cy,
            r,
            color,
            if task.completing { 1.0 } else { 0.78 },
            false,
            1.5,
        )?;

        if task.completing {
            self.ellipse(cx, cy, r - 1.0, p.ok_green, 0.92, true, 0.0)?;
            // A check mark from two strokes with round caps. On the filled
            // circle it needs the opposing colour.
            let tick = if p.high_contrast {
                p.panel_top
            } else if p.dark {
                p.panel_bottom
            } else {
                0xFF_FFFF
            };
            self.line(
                cx - r * 0.45,
                cy,
                cx - r * 0.1,
                cy + r * 0.38,
                tick,
                1.0,
                1.8,
                true,
            )?;
            self.line(
                cx - r * 0.1,
                cy + r * 0.38,
                cx + r * 0.5,
                cy - r * 0.4,
                tick,
                1.0,
                1.8,
                true,
            )?;
        } else if hovered {
            // Preview: a filled dot hinting at what a click would do.
            let a = 0.20 + 0.25 * frame.anim.hover.value;
            self.ellipse(cx, cy, r - 3.0, p.accent, a, true, 0.0)?;
        }
        Ok(())
    }

    // --- Trimmings ----------------------------------------------------------

    /// A section heading with a count and an optional overdue badge.
    #[allow(clippy::too_many_arguments)]
    fn section_label(
        &self,
        x0: f32,
        x1: f32,
        y: f32,
        label: &str,
        count: usize,
        badges: &[(String, u32)],
    ) -> Result<f32> {
        let m = self.metrics;
        let h = m.section_label_h;

        // Place badges right to left so several of them (overdue, conflicts)
        // cannot overlap.
        let mut right = x1;
        for (text, color) in badges.iter().rev() {
            let w = self.text_width(text, Font::Section) + 13.0;
            let pill = rect(right - w, y + 1.0, right, y + h - 3.0);
            self.fill_round(
                pill,
                (pill.bottom - pill.top) * 0.5,
                *color,
                if self.pal.dark { 0.16 } else { 0.13 },
            )?;
            self.text(
                text,
                Font::Section,
                rect(pill.left + 6.0, pill.top, pill.right - 6.0, pill.bottom),
                *color,
                1.0,
            )?;
            right = pill.left - 6.0;
        }

        self.text(
            &format!("{label}   {count}"),
            Font::Section,
            rect(x0, y, (right - 6.0).max(x0 + 20.0), y + h),
            self.pal.text_dim,
            0.9,
        )?;
        Ok(y + h)
    }

    fn draw_scrollbar(
        &self,
        panel: D2D_RECT_F,
        top: f32,
        bottom: f32,
        content: f32,
        viewport: f32,
        frame: &Frame,
    ) -> Result<()> {
        let m = self.metrics;
        let alpha = frame.anim.scrollbar.value;
        if content <= viewport || alpha < 0.01 {
            return Ok(());
        }

        let track_h = bottom - top - 8.0;
        let thumb_h = (track_h * (viewport / content)).max(24.0);
        let max_scroll = content - viewport;
        let t = (frame.anim.scroll.value / max_scroll).clamp(0.0, 1.0);
        let thumb_y = top + 4.0 + (track_h - thumb_h) * t;
        let x = panel.right - m.pad * 0.45 - m.scrollbar_w;

        self.fill_round(
            rect(x, thumb_y, x + m.scrollbar_w, thumb_y + thumb_h),
            m.scrollbar_w * 0.5,
            if self.pal.dark { 0xFF_FFFF } else { 0x2A_3140 },
            0.30 * alpha,
        )
    }

    fn draw_footer(
        &self,
        panel: D2D_RECT_F,
        cx0: f32,
        cx1: f32,
        frame: &Frame,
        hits: &mut Vec<HitRegion>,
    ) -> Result<()> {
        let m = self.metrics;
        let p = &self.pal;
        let c = frame.loc.cat;
        let y = panel.bottom - m.footer_h;
        self.line(cx0, y, cx1, y, p.rule, p.rule_alpha * 0.8, 1.0, false)?;

        // A broken configuration is more urgent than any sync state: without
        // a hint you are left wondering why your changes do nothing.
        let (text, color, actionable) = match (frame.config_error, frame.status) {
            (Some(_), _) => (frame.loc.label(c.config_broken), p.warn, true),
            (None, Status::NeedsSetup(_)) => (frame.loc.label(c.setup_needed), p.warn, true),
            (None, Status::NeedsLogin(_)) => (frame.loc.label(c.connect_google), p.warn, true),
            (None, Status::Error(e)) => (short(e, 56), p.overdue, true),
            (None, Status::Syncing) => (frame.loc.label(c.syncing), p.text_dim, false),
            // An available update outranks the routine "last synced" line —
            // that one carries no news once it has been read.
            (None, Status::Idle) if frame.update.is_some() => (
                frame.loc.label(c.update_available).replacen(
                    "{}",
                    frame.update.unwrap_or_default(),
                    1,
                ),
                p.accent,
                true,
            ),
            (None, Status::Idle) => match frame.agenda.fetched_at {
                Some(t) => {
                    let next = t + chrono::Duration::minutes(frame.sync_minutes as i64);
                    (
                        frame
                            .loc
                            .updated_next(&frame.loc.time(t), &frame.loc.time(next)),
                        p.text_faint,
                        false,
                    )
                }
                None => (frame.loc.label(c.not_synced), p.text_faint, false),
            },
        };

        let footer = rect(cx0, y + 1.0, cx1, panel.bottom - 2.0);
        let hovered = actionable && frame.hover == Some(Hit::StatusAction);
        self.text(
            &text,
            Font::Footer,
            footer,
            color,
            if hovered { 1.0 } else { 0.88 },
        )?;
        if actionable {
            hits.push(HitRegion {
                rect: footer,
                hit: Hit::StatusAction,
            });
        }
        Ok(())
    }

    // --- Drawing primitives -------------------------------------------------

    /// Mirrors an x coordinate when the locale reads right to left. The axis
    /// is the centre of the glass body.
    fn mx(&self, x: f32) -> f32 {
        if self.rtl { self.mirror.get() - x } else { x }
    }

    /// A mirrored rectangle; left and right swap roles in the process.
    fn mrect(&self, r: D2D_RECT_F) -> D2D_RECT_F {
        if !self.rtl {
            return r;
        }
        let a = self.mirror.get();
        rect(a - r.right, r.top, a - r.left, r.bottom)
    }

    fn fill_round(&self, r: D2D_RECT_F, radius: f32, color: u32, alpha: f32) -> Result<()> {
        if alpha < 0.004 {
            return Ok(());
        }
        unsafe {
            self.dc.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: self.mrect(r),
                    radiusX: radius,
                    radiusY: radius,
                },
                &self.brush(color, alpha)?,
            );
        }
        Ok(())
    }

    fn stroke_round(
        &self,
        r: D2D_RECT_F,
        radius: f32,
        color: u32,
        alpha: f32,
        width: f32,
    ) -> Result<()> {
        unsafe {
            self.dc.DrawRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: self.mrect(r),
                    radiusX: radius,
                    radiusY: radius,
                },
                &self.brush(color, alpha)?,
                width,
                None,
            );
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn line(
        &self,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        color: u32,
        alpha: f32,
        width: f32,
        round_caps: bool,
    ) -> Result<()> {
        if alpha < 0.004 {
            return Ok(());
        }
        unsafe {
            self.dc.DrawLine(
                point(self.mx(x0), y0),
                point(self.mx(x1), y1),
                &self.brush(color, alpha)?,
                width,
                round_caps.then_some(&self.round_stroke),
            );
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn ellipse(
        &self,
        cx: f32,
        cy: f32,
        r: f32,
        color: u32,
        alpha: f32,
        fill: bool,
        stroke: f32,
    ) -> Result<()> {
        let e = D2D1_ELLIPSE {
            point: point(self.mx(cx), cy),
            radiusX: r,
            radiusY: r,
        };
        let brush = self.brush(color, alpha)?;
        unsafe {
            if fill {
                self.dc.FillEllipse(&e, &brush);
            } else {
                self.dc.DrawEllipse(&e, &brush, stroke, None);
            }
        }
        Ok(())
    }

    /// Gradient stops for the glass body.
    ///
    /// Interpolation is deliberately *premultiplied*: the stops are partly
    /// transparent, and without premultiplied blending the path between two
    /// stops picks up a visible grey haze.
    fn gradient_stops(&self, stops: &[D2D1_GRADIENT_STOP]) -> Result<ID2D1GradientStopCollection1> {
        unsafe {
            self.dc.CreateGradientStopCollection(
                stops,
                D2D1_COLOR_SPACE_SRGB,
                D2D1_COLOR_SPACE_SRGB,
                D2D1_BUFFER_PRECISION_8BPC_UNORM,
                D2D1_EXTEND_MODE_CLAMP,
                D2D1_COLOR_INTERPOLATION_MODE_PREMULTIPLIED,
            )
        }
    }

    /// A cached brush; opacity is set per call and carries the global reveal
    /// factor.
    fn brush(&self, color: u32, alpha: f32) -> Result<ID2D1SolidColorBrush> {
        let mut cache = self.brushes.borrow_mut();
        let brush = match cache.get(&color) {
            Some(b) => b.clone(),
            None => {
                let b = unsafe {
                    self.dc.CreateSolidColorBrush(
                        &rgba(color, 1.0),
                        None::<*const D2D1_BRUSH_PROPERTIES>,
                    )?
                };
                cache.insert(color, b.clone());
                b
            }
        };
        unsafe { brush.SetOpacity((alpha * self.fade.get()).clamp(0.0, 1.0)) };
        Ok(brush)
    }

    /// Single-line text with an ellipsis on overflow, vertically centred.
    fn text(&self, s: &str, font: Font, r: D2D_RECT_F, color: u32, alpha: f32) -> Result<()> {
        if s.is_empty() || r.right <= r.left {
            return Ok(());
        }
        let w = (r.right - r.left).max(1.0);
        let h = (r.bottom - r.top).max(1.0);
        let layout = self.layout(s, font, w, h)?;
        let brush: ID2D1Brush = self.brush(color, alpha)?.cast()?;
        let r = self.mrect(r);
        unsafe {
            self.dc.DrawTextLayout(
                point(r.left, r.top),
                &layout,
                &brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }
        Ok(())
    }

    /// Note that the hovered row is being drawn truncated.
    ///
    /// An event title cut off with "…" is simply not readable — and having
    /// to go and look it up in the calendar defeats the point of a
    /// Widgets.
    fn note_truncation(&self, hovered: bool, s: &str, font: Font, avail: f32, row: D2D_RECT_F) {
        if !hovered || avail <= 0.0 || self.text_width(s, font) <= avail {
            return;
        }
        self.tooltip
            .replace(Some((s.to_string(), row.top, row.bottom)));
    }

    /// An overlay carrying the full text of the hovered row.
    fn draw_tooltip(&self, panel: D2D_RECT_F, top_limit: f32, bottom_limit: f32) -> Result<()> {
        let Some((text, row_top, row_bottom)) = self.tooltip.borrow().clone() else {
            return Ok(());
        };
        let m = self.metrics;
        let p = &self.pal;

        let pad = 8.0;
        let width = (panel.right - panel.left) - m.pad * 2.0;
        let inner_w = width - pad * 2.0;
        let Ok(layout) = self.layout(&text, Font::Tooltip, inner_w, 400.0) else {
            return Ok(());
        };
        let mut metrics = DWRITE_TEXT_METRICS::default();
        if unsafe { layout.GetMetrics(&mut metrics) }.is_err() {
            return Ok(());
        }
        let height = metrics.height + pad * 2.0;

        // Below the row by preference, above it otherwise — and inside the
        // content area either way.
        let mut top = row_bottom + 3.0;
        if top + height > bottom_limit {
            top = row_top - 3.0 - height;
        }
        let top = top.clamp(top_limit, (bottom_limit - height).max(top_limit));
        let box_rect = rect(
            panel.left + m.pad,
            top,
            panel.left + m.pad + width,
            top + height,
        );

        // Drawn opaque: the overlay has to cover the text underneath
        // completely, or it becomes unreadable itself.
        self.fill_round(box_rect, 5.0, p.panel_top, 1.0)?;
        self.stroke_round(box_rect, 5.0, p.accent, 0.55, 1.0)?;
        self.text_wrapped(
            &text,
            rect(
                box_rect.left + pad,
                box_rect.top + pad,
                box_rect.right - pad,
                box_rect.bottom - pad,
            ),
            p.text_primary,
        )
    }

    /// Multi-line text at the top edge of the rectangle.
    fn text_wrapped(&self, s: &str, r: D2D_RECT_F, color: u32) -> Result<()> {
        let w = (r.right - r.left).max(1.0);
        let h = (r.bottom - r.top).max(1.0);
        let layout = self.layout(s, Font::Tooltip, w, h)?;
        let brush: ID2D1Brush = self.brush(color, 1.0)?.cast()?;
        let r = self.mrect(r);
        unsafe {
            self.dc.DrawTextLayout(
                point(r.left, r.top),
                &layout,
                &brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }
        Ok(())
    }

    /// The actual width of a text in DIPs.
    ///
    /// Fixed column widths are the classic internationalisation trap: what is
    /// enough for "gestern" truncates "yesterday", and "المتأخرة" all the more.
    /// So the narrow columns are derived from the measured content.
    fn text_width(&self, s: &str, font: Font) -> f32 {
        // A generous default width, so nothing wraps or gets truncated.
        const UNBOUNDED: f32 = 4096.0;
        let Ok(layout) = self.layout(s, font, UNBOUNDED, 64.0) else {
            return 0.0;
        };
        let mut metrics = DWRITE_TEXT_METRICS::default();
        if unsafe { layout.GetMetrics(&mut metrics) }.is_err() {
            return 0.0;
        }
        metrics.width
    }

    /// A column's width from its widest entry, capped so a single outlier
    /// cannot eat half the row.
    fn column_width<'s>(
        &self,
        items: impl Iterator<Item = &'s str>,
        font: Font,
        min: f32,
        max: f32,
    ) -> f32 {
        let widest = items.fold(0.0_f32, |acc, s| acc.max(self.text_width(s, font)));
        // A little air, so the text and the neighbouring column do not touch.
        (widest + 6.0).clamp(min, max)
    }

    fn layout(&self, s: &str, font: Font, w: f32, h: f32) -> Result<IDWriteTextLayout> {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        s.hash(&mut hasher);

        // Width and height belong in the key: they decide where the text is
        // cut with "…" and how it is centred vertically. Quantised to quarter
        // DIPs so rounding noise produces no new entries while real
        // differences in width stay visible.
        let key = (
            hasher.finish(),
            font as u32,
            (w * 4.0).round() as u32,
            (h * 4.0).round() as u32,
        );

        if let Some(l) = self.layouts.borrow().get(&key) {
            return Ok(l.clone());
        }
        let utf16: Vec<u16> = s.encode_utf16().collect();
        let layout = unsafe {
            self.dwrite
                .CreateTextLayout(&utf16, &self.formats[font as usize], w, h)?
        };
        self.layouts.borrow_mut().insert(key, layout.clone());
        Ok(layout)
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

/// End time of an event in minutes since midnight.
///
/// With no end time an hour is assumed; if the event runs past midnight the
/// raw time would point backwards, so it is clamped to the end of the day.
fn end_minutes(ev: &Event, start_min: f32) -> f32 {
    match ev.end {
        Some(e) => {
            let v = e.hour() as f32 * 60.0 + e.minute() as f32;
            if v <= start_min { 24.0 * 60.0 } else { v }
        }
        None => start_min + 60.0,
    }
}

/// The event running now, otherwise the next one still to come.
fn pick_hero(events: &[Event], now: DateTime<Local>) -> Option<(usize, &Event, bool)> {
    if let Some((i, e)) = events.iter().enumerate().find(|(_, e)| e.is_now(now)) {
        return Some((i, e, true));
    }
    events
        .iter()
        .enumerate()
        .filter(|(_, e)| !e.all_day && e.start.map(|s| s > now).unwrap_or(false))
        .min_by_key(|(_, e)| e.start.map(|s| s.timestamp()).unwrap_or(i64::MAX))
        .map(|(i, e)| (i, e, false))
}

fn due_label(task: &Task, today: NaiveDate, loc: &Locale) -> String {
    match task.due {
        None => "—".into(),
        Some(d) if d == today => loc.label(loc.cat.today),
        Some(d) if (today - d).num_days() == 1 => loc.label(loc.cat.yesterday),
        // Day and month in the order the locale uses.
        Some(d) => loc.day_month(d),
    }
}

fn short(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
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

/// The font family to draw the interface in, as a nul-terminated wide string.
///
/// "Segoe UI Variable Text" is the Windows 11 system font; on older systems
/// DirectWrite falls back to Segoe UI by itself. A name it does not know falls
/// back the same way rather than failing, which is why a typo here costs a
/// different font and not a widget that will not start.
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
) -> Result<IDWriteTextFormat> {
    unsafe {
        let format = dwrite.CreateTextFormat(
            PCWSTR(family.as_ptr()),
            None,
            weight,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            size,
            w!("de-DE"),
        )?;
        // With a right-to-left reading direction DirectWrite swaps the meaning
        // of LEADING and TRAILING itself — "leading" is then the right edge.
        // So the layout code stays as it is and never swaps alignments.
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
) -> Result<IDWriteTextFormat> {
    unsafe {
        let format = dwrite.CreateTextFormat(
            PCWSTR(family.as_ptr()),
            None,
            DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            size,
            w!("de-DE"),
        )?;
        if rtl {
            format.SetReadingDirection(DWRITE_READING_DIRECTION_RIGHT_TO_LEFT)?;
        }
        format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_LEADING)?;
        // Aligned to the top: the height is derived from the content, not
        // umgekehrt.
        format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR)?;
        Ok(format)
    }
}

fn icon_format(dwrite: &IDWriteFactory, size: f32) -> Result<IDWriteTextFormat> {
    unsafe {
        let format = dwrite.CreateTextFormat(
            w!("Segoe Fluent Icons"),
            None,
            DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            size,
            w!("en-us"),
        )?;
        format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
        format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        Ok(format)
    }
}

/// Rotation about a point, in radians.
///
/// Computed by hand rather than through `Matrix3x2::rotation_around`, because
/// there the angle unit (degrees or radians) is not evident from the
/// signature.
/// D2D verwendet Zeilenvektoren: `[x y 1] · M`.
fn rotation(rad: f32, cx: f32, cy: f32) -> Matrix3x2 {
    let (s, c) = rad.sin_cos();
    Matrix3x2 {
        M11: c,
        M12: s,
        M21: -s,
        M22: c,
        M31: cx - c * cx + s * cy,
        M32: cy - s * cx - c * cy,
    }
}

pub fn rect(left: f32, top: f32, right: f32, bottom: f32) -> D2D_RECT_F {
    D2D_RECT_F {
        left,
        top,
        right,
        bottom,
    }
}

fn inset(r: D2D_RECT_F, by: f32) -> D2D_RECT_F {
    rect(r.left + by, r.top + by, r.right - by, r.bottom - by)
}

fn point(x: f32, y: f32) -> Vector2 {
    Vector2 { X: x, Y: y }
}
