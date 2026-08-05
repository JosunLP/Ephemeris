//! Direct2D-Renderer auf einer DirectComposition-Oberflaeche.
//!
//! Aufbau der Kette:
//!
//! ```text
//! D3D11 Device ─► DXGI Device ─┬─► D2D Device ─► D2D DeviceContext  (zeichnen)
//!                              └─► DComp Device ─► Visual ─► Target ─► HWND
//!                                        ▲
//!               Composition-Swapchain ───┘   (DXGI_ALPHA_MODE_PREMULTIPLIED)
//! ```
//!
//! Warum dieser Weg statt eines klassischen Layered Window:
//! `UpdateLayeredWindow` laeuft ueber eine CPU-Bitmap und kostet bei jedem
//! Frame eine volle Kopie. Die Composition-Swapchain bleibt komplett auf der
//! GPU und liefert trotzdem echtes Per-Pixel-Alpha — Voraussetzung dafuer ist
//! `WS_EX_NOREDIRECTIONBITMAP` am Fenster.
//!
//! Gezeichnet wird bei Bedarf: neue Daten, Minutenwechsel, Hover — und
//! waehrend einer laufenden Animation mit ~60 Hz. Sobald alles zur Ruhe
//! gekommen ist, hoert das Zeichnen vollstaendig auf.

use crate::anim::Animations;
use crate::i18n::Locale;
use crate::model::{Agenda, Event, Task};
use crate::sync::Status;
use crate::theme::{Metrics, Palette, mix, rgba};
use chrono::{DateTime, Local, NaiveDate, Timelike};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::mem::ManuallyDrop;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_GRADIENT_STOP, D2D1_PIXEL_FORMAT,
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
use windows::core::{HRESULT, Interface, Result, w};
use windows_numerics::{Matrix3x2, Vector2};

/// Anklickbare Flaeche, in DIPs relativ zur Fensterecke.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Hit {
    Refresh,
    /// Kreis vor einer Aufgabe -> abhaken.
    TaskCheck(usize),
    /// Aufgabenzeile -> in Google Tasks oeffnen.
    Task(usize),
    /// Zeile einer Aufgabe, deren Abhaken noch zurueckgenommen werden kann.
    Undo(usize),
    /// Terminzeile -> im Kalender oeffnen.
    Event(usize),
    /// Hervorgehobener Termin im Kopfbereich.
    Hero(usize),
    /// Termin aus der Morgen-Vorschau.
    Tomorrow(usize),
    /// Statuszeile mit Handlungsbedarf (Einrichtung/Anmeldung/Konfiguration).
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

/// Aufgabe, deren Abhaken noch aussteht und zurueckgenommen werden kann.
#[derive(Debug, Clone, Copy)]
pub struct UndoView<'a> {
    pub task_id: &'a str,
    /// 1.0 direkt nach dem Klick, 0.0 wenn gesendet wird.
    pub remaining: f32,
}

/// Zeichenzustand, den das Fenster hereinreicht.
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
    /// Syntaxfehler in `config.json`; hat Vorrang vor der Sync-Statuszeile.
    pub config_error: Option<&'a str>,
}

pub struct FrameResult {
    /// Gesamthoehe des Inhalts — Basis fuer die Scroll-Begrenzung.
    pub content_height: f32,
    pub viewport_height: f32,
}

/// Schriftrollen; der Index adressiert `Renderer::formats`.
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
    /// Mehrzeilig mit Umbruch, ohne Kuerzung — nur fuer den Tooltip.
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

    /// Ein Pinsel je Farbe; die Deckkraft wird pro Verwendung gesetzt. Spart
    /// bei 60 Hz einige hundert COM-Objekterzeugungen pro Sekunde.
    brushes: RefCell<HashMap<u32, ID2D1SolidColorBrush>>,
    /// Fertige Textlayouts. DirectWrite-Layout ist der teuerste Einzelschritt
    /// im Frame; waehrend einer Animation aendert sich der Text aber nicht.
    layouts: RefCell<HashMap<(u64, u32, u32, u32), IDWriteTextLayout>>,

    /// Globaler Deckkraftfaktor fuer die Einblend-Animation.
    fade: Cell<f32>,
    /// Voller Text der ueberfahrenen Zeile, sofern er gekuerzt dargestellt
    /// wurde. Wird waehrend des Zeichnens gesetzt und ganz am Ende als
    /// Ueberlagerung ausgegeben.
    tooltip: RefCell<Option<(String, f32, f32)>>,

    pal: Palette,
    /// Rechts-nach-links-Layout. Gespiegelt wird ausschliesslich in den
    /// Zeichenprimitiven; der gesamte Layoutcode rechnet unveraendert von
    /// links nach rechts.
    rtl: bool,
    /// Spiegelachse `panel.left + panel.right`, je Frame gesetzt.
    mirror: Cell<f32>,
    metrics: Metrics,
    dpi: f32,
    size_px: (u32, u32),
}

impl Renderer {
    pub fn new(
        hwnd: HWND,
        width_px: u32,
        height_px: u32,
        dpi: f32,
        scale: f32,
        pal: Palette,
        rtl: bool,
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
                // Ohne premultiplied Alpha bleibt das Fenster undurchsichtig.
                AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
                Flags: 0,
            };
            let swapchain = factory.CreateSwapChainForComposition(&device, &desc, None)?;

            let dcomp: IDCompositionDevice = DCompositionCreateDevice(&dxgi_device)?;

            // `topmost = false`: die Z-Ordnung regelt das Fenster selbst
            // (bottom-most), nicht die Komposition.
            let target = dcomp.CreateTargetForHwnd(hwnd, false)?;
            let visual = dcomp.CreateVisual()?;
            visual.SetContent(&swapchain)?;
            target.SetRoot(&visual)?;
            dcomp.Commit()?;

            let d2d_factory: ID2D1Factory1 =
                D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let d2d_device = d2d_factory.CreateDevice(&dxgi_device)?;
            let dc = d2d_device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;

            // ClearType braucht einen deckenden Hintergrund. Auf einer
            // transparenten Glasflaeche erzeugt es Farbsaeume, deshalb
            // Graustufen-Antialiasing — dasselbe tun WPF und WinUI auf Acryl.
            dc.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);

            // Runde Enden fuer Haken und Fortschrittsbalken.
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
            // `ID2D1Factory1` liefert die "1"-Variante; `DrawLine` erwartet
            // aber exakt die Basisschnittstelle.
            let round_stroke: ID2D1StrokeStyle = round_stroke.into();

            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let m = Metrics::new(scale);

            let formats = [
                text_format(
                    &dwrite,
                    m.fs_title,
                    DWRITE_FONT_WEIGHT_SEMI_BOLD,
                    DWRITE_TEXT_ALIGNMENT_LEADING,
                    rtl,
                )?,
                text_format(
                    &dwrite,
                    m.fs_clock,
                    DWRITE_FONT_WEIGHT_NORMAL,
                    DWRITE_TEXT_ALIGNMENT_TRAILING,
                    rtl,
                )?,
                text_format(
                    &dwrite,
                    m.fs_subtitle,
                    DWRITE_FONT_WEIGHT_NORMAL,
                    DWRITE_TEXT_ALIGNMENT_LEADING,
                    rtl,
                )?,
                text_format(
                    &dwrite,
                    m.fs_section,
                    DWRITE_FONT_WEIGHT_SEMI_BOLD,
                    DWRITE_TEXT_ALIGNMENT_LEADING,
                    rtl,
                )?,
                text_format(
                    &dwrite,
                    m.fs_row,
                    DWRITE_FONT_WEIGHT_NORMAL,
                    DWRITE_TEXT_ALIGNMENT_LEADING,
                    rtl,
                )?,
                text_format(
                    &dwrite,
                    m.fs_row,
                    DWRITE_FONT_WEIGHT_SEMI_BOLD,
                    DWRITE_TEXT_ALIGNMENT_LEADING,
                    rtl,
                )?,
                text_format(
                    &dwrite,
                    m.fs_meta,
                    DWRITE_FONT_WEIGHT_NORMAL,
                    DWRITE_TEXT_ALIGNMENT_LEADING,
                    rtl,
                )?,
                text_format(
                    &dwrite,
                    m.fs_meta,
                    DWRITE_FONT_WEIGHT_NORMAL,
                    DWRITE_TEXT_ALIGNMENT_TRAILING,
                    rtl,
                )?,
                text_format(
                    &dwrite,
                    m.fs_footer,
                    DWRITE_FONT_WEIGHT_NORMAL,
                    DWRITE_TEXT_ALIGNMENT_LEADING,
                    rtl,
                )?,
                icon_format(&dwrite, m.fs_row + 1.5)?,
                tooltip_format(&dwrite, m.fs_row, rtl)?,
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

    /// Design gewechselt (Windows hell/dunkel, geaenderte Akzentfarbe).
    ///
    /// Die Pinsel sind nach Farbe zwischengespeichert; ohne Leeren blieben die
    /// alten Eintraege dauerhaft liegen.
    pub fn set_palette(&mut self, pal: Palette) {
        self.pal = pal;
        self.brushes.borrow_mut().clear();
    }

    pub fn palette(&self) -> Palette {
        self.pal
    }

    /// Backbuffer als D2D-Ziel setzen. Muss nach jedem Resize erneut passieren.
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
            // Ab hier rechnet alles in DIPs; die DPI-Skalierung uebernimmt D2D.
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
            // Das alte Ziel muss die Referenz auf den Backbuffer loslassen,
            // sonst schlaegt ResizeBuffers mit DXGI_ERROR_INVALID_CALL fehl.
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

    /// Groesse des sichtbaren Bereichs in DIPs.
    pub fn size_dip(&self) -> (f32, f32) {
        let k = 96.0 / self.dpi;
        (self.size_px.0 as f32 * k, self.size_px.1 as f32 * k)
    }

    /// `hits` wird vom Aufrufer bereitgestellt und hier neu befuellt — so
    /// entfaellt eine Vec-Allokation pro Frame.
    pub fn draw(&mut self, frame: &Frame, hits: &mut Vec<HitRegion>) -> Result<FrameResult> {
        let (w, h) = self.size_dip();
        let m = self.metrics;
        hits.clear();

        // Der Glaskoerper ist um den Schattenrand eingerueckt.
        let panel = rect(m.shadow, m.shadow, w - m.shadow, h - m.shadow);
        let cx0 = panel.left + m.pad;
        let cx1 = panel.right - m.pad;
        // Achse fuer die RTL-Spiegelung: alles innerhalb des Glaskoerpers wird
        // daran geklappt, der Koerper selbst bildet sich dabei auf sich ab.
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

            // Alles dazwischen wird geklippt, damit gescrollte Zeilen nicht in
            // Kopf-, Fuss- oder Rahmenbereich hineinlaufen.
            self.dc.PushAxisAlignedClip(
                &rect(
                    panel.left + 1.0,
                    content_top,
                    panel.right - 1.0,
                    content_bottom,
                ),
                Default::default(),
            );

            // Einblenden: leicht von unten hereinschieben und aufblenden.
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
            // Ganz zuletzt, damit die Ueberlagerung ueber allem liegt und
            // nicht vom Inhaltsklipp beschnitten wird.
            self.draw_tooltip(panel, content_top, content_bottom)?;

            self.dc.EndDraw(None, None)?;
            self.swapchain.Present(1, DXGI_PRESENT(0)).ok()?;
            self.dcomp.Commit()?;

            // Unbegrenztes Wachstum verhindern: die Relativzeiten ("in 25 Min")
            // erzeugen jede Minute neue Schluessel.
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

    // --- Glaskoerper --------------------------------------------------------

    /// Weicher Schlagschatten aus uebereinandergelegten, nach aussen
    /// wachsenden Rundrechtecken.
    ///
    /// Ein echter Gauss-Blur ueber `ID2D1Effect` waere sauberer, braucht aber
    /// eine Zwischenbitmap und einen kompletten Effektgraphen pro Frame. Bei
    /// zwoelf Fuellungen mit geringer Deckkraft ist das Ergebnis auf dieser
    /// Groesse nicht unterscheidbar und deutlich billiger.
    fn draw_shadow(&self, panel: D2D_RECT_F) -> Result<()> {
        let m = self.metrics;
        // Im Kontrastdesign gibt es keinen Schatten: er weicht den Rand auf,
        // den dieser Modus gerade hart haben will.
        if self.pal.shadow_alpha <= 0.0 {
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
                        // Leicht nach unten versetzt: Licht kommt von oben.
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

    /// Verlauf, Glanzbogen und die doppelte Kante.
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

            // Glanzbogen: heller Verlauf im oberen Bereich, der nach unten
            // vollstaendig ausblendet. Im Kontrastdesign entfaellt er.
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

            // Innen hell, aussen dunkel — dieser Kontrast erzeugt die
            // plastische Kante.
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
            self.dc.DrawRoundedRectangle(
                &body,
                &self.brush(p.border_outer, p.border_outer_alpha)?,
                1.0,
                None,
            );

            // Schmaler Lichtstreifen ganz oben, wie die Reflexion auf einer
            // Glaskante.
            self.dc.DrawLine(
                point(panel.left + m.corner, panel.top + 1.0),
                point(panel.right - m.corner, panel.top + 1.0),
                &self.brush(p.sheen, p.sheen_border * 1.4)?,
                1.0,
                None,
            );
            Ok(())
        }
    }

    // --- Kopfbereich --------------------------------------------------------

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

        // Aktualisieren-Knopf
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

        // Wochentag gross, Datum klein darunter.
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

        // Uhr rechts, auf Hoehe der Datumszeile. Sie ist der Grund, warum das
        // Widget minuetlich neu zeichnet.
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

    /// Tagesschiene: der ganze Tag von 06:00 bis 22:00 auf einem Streifen.
    ///
    /// Zeigt die *Form* des Tages, die aus einer Liste nicht hervorgeht —
    /// geballte Vormittage, freie Nachmittage, wie lange etwas dauert. Jeder
    /// Termin ist ein Segment in seiner Kalenderfarbe, an der richtigen
    /// Stelle und in der richtigen Breite; die verstrichene Zeit ist getoent,
    /// und eine Marke zeigt, wo man gerade steht.
    fn draw_day_rail(&self, x0: f32, x1: f32, y: f32, frame: &Frame) -> Result<()> {
        let m = self.metrics;
        let p = &self.pal;
        // Fenster des Arbeitstags. Alles davor oder danach wird an den Rand
        // geklemmt, statt die Schiene auf 24 Stunden zu strecken — dann waere
        // der interessante Bereich nur noch halb so breit.
        const DAY_FROM: f32 = 6.0 * 60.0;
        const DAY_SPAN: f32 = 16.0 * 60.0;
        let width = x1 - x0;
        let pos = |minutes: f32| x0 + width * ((minutes - DAY_FROM) / DAY_SPAN).clamp(0.0, 1.0);

        let track = rect(x0, y, x1, y + m.rail_h);
        let radius = m.rail_h * 0.5;
        self.fill_round(track, radius, p.rule, p.rule_alpha * 0.9)?;

        let now_min = frame.now.hour() as f32 * 60.0 + frame.now.minute() as f32;
        let now_x = pos(now_min);

        // Verstrichener Teil des Tages.
        if now_x > x0 + 0.5 {
            self.fill_round(rect(x0, y, now_x, y + m.rail_h), radius, p.accent, 0.14)?;
        }

        for ev in &frame.agenda.events {
            if ev.all_day {
                continue;
            }
            let Some(start) = ev.start else { continue };
            let start_min = start.hour() as f32 * 60.0 + start.minute() as f32;
            let end_min = ev
                .end
                .map(|e| {
                    // Ein Termin ueber Mitternacht hinaus wuerde sonst
                    // rueckwaerts laufen.
                    let v = e.hour() as f32 * 60.0 + e.minute() as f32;
                    if v <= start_min { DAY_FROM + DAY_SPAN } else { v }
                })
                .unwrap_or(start_min + 60.0);

            let (sx, ex) = (pos(start_min), pos(end_min));
            // Kurze Termine bleiben sonst unsichtbar.
            let ex = ex.max(sx + 2.5);
            let running = ev.is_now(frame.now);
            let color = if self.pal.high_contrast {
                p.text_primary
            } else if running {
                p.accent
            } else {
                ev.color
            };
            self.fill_round(
                rect(sx, y + 1.0, ex.min(x1), y + m.rail_h - 1.0),
                (m.rail_h - 2.0) * 0.5,
                color,
                if running { 1.0 } else { 0.85 },
            )?;
        }

        // Jetzt-Marke ueber allem, damit sie nie von einem Segment verdeckt wird.
        if (0.0..=1.0).contains(&((now_min - DAY_FROM) / DAY_SPAN)) {
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

    /// Hervorgehobener Termin: der laufende, sonst der naechste.
    ///
    /// Das ist die eine Information, fuer die man sonst hinsehen muesste — sie
    /// gehoert nach ganz oben und nicht in eine Liste.
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

        // Laufender Termin kraeftig, kommender zurueckhaltend.
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
        // Akzentbalken an der Leseanfangs-Seite.
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

        // Countdown rechts: bei laufendem Termin bis zum Ende, sonst bis zum
        // Start.
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

        // Fortschritt des laufenden Termins als feine Linie am Kartenfuss.
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

    // --- Listen -------------------------------------------------------------

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

        // Der laufende Termin bleibt immer sichtbar, auch wenn vergangene
        // ausgeblendet sind — sonst verschwindet ausgerechnet der wichtigste.
        let visible: Vec<(usize, &Event)> = frame
            .agenda
            .events
            .iter()
            .enumerate()
            .filter(|(_, e)| frame.show_past_events || !e.is_past(now) || e.is_now(now))
            .collect();

        // Doppelbuchungen sind beim Ueberfliegen einer Liste kaum zu sehen —
        // man muesste Ende und Anfang zweier Zeilen im Kopf vergleichen.
        let overlapping = crate::model::mark_overlaps(&frame.agenda.events);
        let conflicts = overlapping.iter().filter(|&&f| f).count() / 2;

        let mut badges: Vec<(String, u32)> = Vec::new();
        if conflicts > 0 {
            badges.push((loc.conflicts(conflicts), p.warn));
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

        // Kalendername nur zeigen, wenn ueberhaupt mehrere im Spiel sind —
        // sonst waere es in jeder Zeile dieselbe redundante Angabe.
        let multi_cal = visible
            .iter()
            .filter(|(_, e)| !e.all_day)
            .map(|(_, e)| e.calendar_name.as_str())
            .collect::<std::collections::HashSet<_>>()
            .len()
            > 1;

        // Spaltenbreiten aus dem tatsaechlichen Inhalt statt aus festen
        // Werten — sonst passt das Layout nur zu einer Sprache.
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

        // Position der Jetzt-Linie: vor dem ersten Termin, der noch kommt.
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
            // Zurueckgenommen, aber noch bequem lesbar.
            let dim: f32 = if past { 0.55 } else { 1.0 };

            if frame.hover == Some(Hit::Event(*idx)) {
                self.fill_round(row, 5.0, p.hover, p.hover_alpha * frame.anim.hover.value)?;
            }
            if is_now {
                self.fill_round(row, 5.0, p.accent, 0.10)?;
            }

            // Farbmarke des Quellkalenders.
            // Im Kontrastdesign wird sie durch die Systemfarbe ersetzt — die
            // Google-Kalenderfarben haetten dort keinen garantierten Kontrast.
            let bar_color = if self.pal.high_contrast {
                if is_now { p.accent } else { p.text_primary }
            } else if is_now {
                p.accent
            } else {
                ev.color
            };
            self.fill_round(
                rect(x0, y + 6.0, x0 + 3.0, y + m.event_row_h - 6.0),
                1.5,
                bar_color,
                dim,
            )?;

            let tx = x0 + 11.0;
            // Ueberschneidende Termine bekommen eine Warnfarbe auf der
            // Uhrzeit — dort schaut man hin, wenn man Konflikte sucht.
            let conflicted = overlapping.get(*idx).copied().unwrap_or(false);
            self.text(
                &time_labels[slot],
                Font::Meta,
                rect(tx, y, tx + time_w, y + m.event_row_h),
                if is_now {
                    p.accent
                } else if conflicted {
                    p.warn
                } else {
                    p.text_dim
                },
                dim,
            )?;

            // Rechte Spalte: Restzeit, bei vergangenen Terminen der
            // Kalendername (die Restzeit waere dort nur Rauschen).
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

        // Alle Termine schon vorbei: Linie ans Ende.
        if now_line_before.is_none() && visible.iter().any(|(_, e)| !e.all_day) {
            self.draw_now_line(x0, x1, y, frame)?;
            y += m.nowline_h;
        }

        y = self.draw_tomorrow(x0, x1, y, frame, hits)?;
        Ok(y)
    }

    /// Ausblick auf morgen.
    ///
    /// Ab dem spaeten Nachmittag ist die heutige Liste leer und das Widget
    /// waere eine leere Flaeche — dabei ist genau dann die Frage "was kommt
    /// morgen zuerst?" die interessante. Angezeigt werden hoechstens zwei
    /// Termine, damit der Ausblick den heutigen Tag nicht verdraengt.
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
            // Die Beschriftung steht nur an der ersten Zeile; die zweite
            // rueckt darunter ein.
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

    /// Duenne Linie mit Uhrzeit-Marke, die zeigt, wo im Tag man gerade steht.
    fn draw_now_line(&self, x0: f32, x1: f32, y: f32, frame: &Frame) -> Result<()> {
        let m = self.metrics;
        let cy = y + m.nowline_h * 0.5;
        // Die Uhrzeit kann je nach Gebietsschema deutlich breiter sein
        // ("8:09 PM" statt "20:09"), deshalb grosszuegig bemessen.
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

        // Listenname nur zeigen, wenn mehrere Aufgabenlisten beteiligt sind.
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
        // Rechte Spalte teilen sich Rueckgaengig-Hinweis und Listenname.
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
            // Waehrend das Abhaken laeuft: sofort ausblassen, damit der Klick
            // sich unmittelbar anfuehlt, statt auf die API zu warten.
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

            // Rechte Spalte: Rueckgaengig-Hinweis schlaegt den Listennamen.
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
                // Ablaufbalken unter der Zeile.
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

            // Notiz als gedaempfte Fortsetzung hinter dem Titel — analog zum
            // Ort bei Terminen. Nur die erste Zeile, der Rest ist ohnehin
            // abgeschnitten.
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

            // Reihenfolge zaehlt: spaeter gepushte Regionen gewinnen. Bei
            // ausstehendem Abhaken faengt die ganze Zeile den Klick ab, damit
            // ein Versehen ueberall zurueckgenommen werden kann.
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

    /// Abhak-Kreis. Beim Absenden fuellt er sich und bekommt einen Haken.
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
            // Ueber die Hover-Animation einblenden statt hart umzuschalten.
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
            // Haken aus zwei Strichen mit runden Enden. Auf dem gefuellten
            // Kreis braucht er die Gegenfarbe.
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
            // Vorschau: gefuellter Punkt, der andeutet, was ein Klick tut.
            let a = 0.20 + 0.25 * frame.anim.hover.value;
            self.ellipse(cx, cy, r - 3.0, p.accent, a, true, 0.0)?;
        }
        Ok(())
    }

    // --- Beiwerk ------------------------------------------------------------

    /// Abschnittsueberschrift mit Anzahl und optionaler Ueberfaellig-Plakette.
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

        // Plaketten von rechts nach links setzen, damit sich mehrere
        // (ueberfaellig, Ueberschneidungen) nicht ueberlappen.
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

        // Eine kaputte Konfiguration ist dringender als jeder Sync-Zustand:
        // ohne Hinweis wundert man sich, warum Aenderungen nichts bewirken.
        let (text, color, actionable) = match (frame.config_error, frame.status) {
            (Some(_), _) => (frame.loc.label(c.config_broken), p.warn, true),
            (None, Status::NeedsSetup(_)) => (frame.loc.label(c.setup_needed), p.warn, true),
            (None, Status::NeedsLogin(_)) => (frame.loc.label(c.connect_google), p.warn, true),
            (None, Status::Error(e)) => (short(e, 56), p.overdue, true),
            (None, Status::Syncing) => (frame.loc.label(c.syncing), p.text_dim, false),
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

    // --- Zeichenprimitive ---------------------------------------------------

    /// Spiegelt eine X-Koordinate, wenn das Gebietsschema von rechts nach
    /// links liest. Die Achse ist die Mitte des Glaskoerpers.
    fn mx(&self, x: f32) -> f32 {
        if self.rtl { self.mirror.get() - x } else { x }
    }

    /// Gespiegeltes Rechteck; links und rechts tauschen dabei die Rollen.
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

    /// Farbverlaufsstufen fuer den Glaskoerper.
    ///
    /// Interpolation bewusst *premultiplied*: die Stufen sind teiltransparent,
    /// und ohne premultiplied Mischung erhaelt man auf dem Weg zwischen zwei
    /// Stufen einen sichtbaren Grauschleier.
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

    /// Zwischengespeicherter Pinsel; Deckkraft wird pro Aufruf gesetzt und
    /// enthaelt den globalen Einblendfaktor.
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

    /// Einzeiliger Text mit Ellipse bei Ueberlauf, vertikal zentriert.
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

    /// Vormerken, dass die ueberfahrene Zeile gekuerzt dargestellt wird.
    ///
    /// Ein mit "…" abgeschnittener Termintitel ist sonst schlicht nicht
    /// lesbar — im Kalender nachsehen zu muessen widerspricht dem Sinn eines
    /// Widgets.
    fn note_truncation(&self, hovered: bool, s: &str, font: Font, avail: f32, row: D2D_RECT_F) {
        if !hovered || avail <= 0.0 || self.text_width(s, font) <= avail {
            return;
        }
        self.tooltip
            .replace(Some((s.to_string(), row.top, row.bottom)));
    }

    /// Ueberlagerung mit dem vollstaendigen Text der ueberfahrenen Zeile.
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

        // Bevorzugt unter die Zeile, sonst darueber — und in jedem Fall
        // innerhalb des Inhaltsbereichs.
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

        // Deckend zeichnen: die Ueberlagerung muss den Text darunter
        // vollstaendig verdecken, sonst wird sie selbst unlesbar.
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

    /// Mehrzeiliger Text am oberen Rand des Rechtecks.
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

    /// Tatsaechliche Breite eines Textes in DIPs.
    ///
    /// Feste Spaltenbreiten sind die klassische Falle bei der
    /// Internationalisierung: was fuer "gestern" reicht, schneidet
    /// "yesterday" ab und erst recht "المتأخرة". Deshalb werden die schmalen
    /// Spalten aus dem gemessenen Inhalt bestimmt.
    fn text_width(&self, s: &str, font: Font) -> f32 {
        // Grosszuegige Vorgabebreite, damit nichts umbricht oder gekuerzt wird.
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

    /// Breite einer Spalte aus ihrem breitesten Eintrag, begrenzt damit ein
    /// einzelner Ausreisser nicht die halbe Zeile frisst.
    fn column_width<'s>(
        &self,
        items: impl Iterator<Item = &'s str>,
        font: Font,
        min: f32,
        max: f32,
    ) -> f32 {
        let widest = items.fold(0.0_f32, |acc, s| acc.max(self.text_width(s, font)));
        // Etwas Luft, damit Text und Nachbarspalte sich nicht beruehren.
        (widest + 6.0).clamp(min, max)
    }

    fn layout(&self, s: &str, font: Font, w: f32, h: f32) -> Result<IDWriteTextLayout> {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        s.hash(&mut hasher);

        // Breite und Hoehe gehoeren in den Schluessel: sie bestimmen, wo der
        // Text mit "…" gekuerzt und wie er vertikal zentriert wird. Auf
        // Viertel-DIP quantisiert, damit Rundungsrauschen keine neuen
        // Eintraege erzeugt, aber echte Breitenunterschiede sichtbar bleiben.
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

/// Verlorenes Grafikgeraet — Treiberwechsel, GPU-Reset, Wechsel in eine
/// RDP-Sitzung. Danach ist die gesamte Kette ungueltig und muss neu aufgebaut
/// werden; ohne diese Behandlung bliebe das Widget dauerhaft schwarz.
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

/// Laufender Termin, sonst der naechste noch kommende.
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
        // Tag und Monat in der Reihenfolge des Gebietsschemas.
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

/// Hardware bevorzugt, WARP als Rueckfallebene (RDP-Sitzungen, VMs ohne GPU).
fn create_d3d_device() -> Result<ID3D11Device> {
    for driver in [D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP] {
        let mut device: Option<ID3D11Device> = None;
        let hr = unsafe {
            D3D11CreateDevice(
                None,
                driver,
                Default::default(),
                // BGRA_SUPPORT ist Pflicht fuer Direct2D-Interop.
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

fn text_format(
    dwrite: &IDWriteFactory,
    size: f32,
    weight: DWRITE_FONT_WEIGHT,
    align: DWRITE_TEXT_ALIGNMENT,
    rtl: bool,
) -> Result<IDWriteTextFormat> {
    unsafe {
        // "Segoe UI Variable Text" ist die Windows-11-Systemschrift; auf
        // aelteren Systemen faellt DirectWrite selbsttaetig auf Segoe UI zurueck.
        let format = dwrite.CreateTextFormat(
            w!("Segoe UI Variable Text"),
            None,
            weight,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            size,
            w!("de-DE"),
        )?;
        // Bei RTL-Leserichtung dreht DirectWrite die Bedeutung von LEADING
        // und TRAILING selbst um — "vorn" ist dann rechts. Deshalb bleibt der
        // Layoutcode unveraendert und muss keine Ausrichtungen tauschen.
        if rtl {
            format.SetReadingDirection(DWRITE_READING_DIRECTION_RIGHT_TO_LEFT)?;
        }
        format.SetTextAlignment(align)?;
        format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;

        // Ueberlaufender Text endet in "…" statt hart abgeschnitten zu werden.
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

/// Mehrzeilig mit Wortumbruch und ohne Kuerzung.
fn tooltip_format(dwrite: &IDWriteFactory, size: f32, rtl: bool) -> Result<IDWriteTextFormat> {
    unsafe {
        let format = dwrite.CreateTextFormat(
            w!("Segoe UI Variable Text"),
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
        // Oben ausgerichtet: die Hoehe wird aus dem Inhalt bestimmt, nicht
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

/// Drehung um einen Punkt, in Radiant.
///
/// Selbst gerechnet statt ueber `Matrix3x2::rotation_around`, weil dort die
/// Winkeleinheit (Grad vs. Radiant) nicht aus der Signatur hervorgeht.
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
