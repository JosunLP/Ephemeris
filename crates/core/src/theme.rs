// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! Colour palette and layout metrics.
//!
//! Visually modelled on the Aero gadgets of Windows Vista. Three ingredients
//! create the impression of real glass there, and all three are rebuilt here:
//!
//! 1. **A soft outer shadow** — the panel floats above the background.
//! 2. **A double edge** — dark outside, one bright pixel inside. Without that
//!    contrast any glass surface looks like a flat sticker.
//! 3. **A sheen across the upper area** — a light gradient fading downwards,
//!    hinting at curvature.
//!
//! The palette follows the system appearance, light or dark, and adopts the
//! system accent colour, so the widget feels like part of the system rather
//! than a foreign application with a blue of its own.
//!
//! That is the default, not the whole story. A widget that sits permanently on
//! the desktop meets wallpapers it disappears into and themes its glass clashes
//! with, so [`Appearance`] layers optional overrides on top: colours beyond the
//! accent, typography, the surface style, layout density and per-calendar
//! colours. Everything in it defaults to `"system"` and the derivation rules
//! below stay in force for whatever is left there.
//!
//! Two things always win over it. A contrast theme keeps its own colours and
//! its flatness, because that is what the mode is for; and a custom colour is
//! corrected until it is readable against the surface it sits on, because a
//! settings file must not be able to produce invisible text.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A colour with straight alpha, in the 0..1 range every graphics API wants.
///
/// Deliberately not a toolkit type: this crate must not depend on Direct2D,
/// Core Graphics or anything else. The renderer converts on use.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

/// `0xRRGGBB` plus alpha.
pub fn rgba(hex: u32, a: f32) -> Rgba {
    Rgba {
        r: ((hex >> 16) & 0xFF) as f32 / 255.0,
        g: ((hex >> 8) & 0xFF) as f32 / 255.0,
        b: (hex & 0xFF) as f32 / 255.0,
        a,
    }
}

/// Appearance settings read from the operating system.
///
/// All five are accessibility or personalisation switches an application has
/// to respect to feel like part of the system instead of imposing its own
/// look. The host fills this in; this crate only reacts to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemVisuals {
    /// A contrast theme is active. Gradients, gloss and shadows then work
    /// against the very contrast the mode exists to provide.
    pub high_contrast: bool,
    /// "Transparency effects" in the system's appearance settings.
    pub transparency: bool,
    /// "Show animations". Off means no motion at all.
    pub animations: bool,
    pub light: bool,
    /// System accent colour as `0xRRGGBB`.
    pub accent: Option<u32>,
    /// Colours of the active contrast scheme. There are several such schemes
    /// with entirely different palettes, so nothing is guessed — the host
    /// reports what the system says.
    pub contrast: Option<ContrastColors>,
}

impl Default for SystemVisuals {
    fn default() -> Self {
        Self {
            high_contrast: false,
            transparency: true,
            animations: true,
            light: false,
            accent: None,
            contrast: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContrastColors {
    pub window: u32,
    pub text: u32,
    pub gray: u32,
    pub highlight: u32,
    pub hot: u32,
}

/// Linear blend of two colours, for hover and state transitions.
pub fn mix(a: u32, b: u32, t: f32) -> u32 {
    let t = t.clamp(0.0, 1.0);
    let ch = |shift: u32| {
        let ca = ((a >> shift) & 0xFF) as f32;
        let cb = ((b >> shift) & 0xFF) as f32;
        ((ca + (cb - ca) * t).round() as u32) << shift
    };
    ch(16) | ch(8) | ch(0)
}

/// The appearance requested by the configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemePref {
    System,
    Dark,
    Light,
    /// Forces the contrast rendering — flat, opaque, system colours only —
    /// even without an active system contrast theme. Useful for anyone who
    /// wants maximum contrast without switching their whole desktop, and for
    /// previewing how it looks.
    Contrast,
}

impl ThemePref {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "dark" | "dunkel" => ThemePref::Dark,
            "light" | "hell" => ThemePref::Light,
            "contrast" | "kontrast" | "high-contrast" => ThemePref::Contrast,
            _ => ThemePref::System,
        }
    }

    /// Will the palette be a high-contrast one?
    ///
    /// Two independent things force it: the system contrast setting, and
    /// asking for [`ThemePref::Contrast`] outright. Anything that has to agree
    /// with the palette about contrast must ask *this*, not `vis.high_contrast`
    /// alone — the metrics above all, because contrast overrides the surface
    /// style and the surface style decides whether the geometry reserves a
    /// shadow margin. See [`Appearance::effective_surface`].
    pub fn high_contrast(self, vis: SystemVisuals) -> bool {
        vis.high_contrast || self == ThemePref::Contrast
    }
}

/// The chrome drawn around the panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    /// Whatever the backdrop setting implies — the Aero look for the widget's
    /// own drawing, nothing for the system backdrop. The default.
    System,
    /// The Aero gadget look this palette is modelled on: soft shadow, double
    /// edge, sheen.
    Aero,
    /// One colour, a hairline border, no shadow and no sheen. For a desktop
    /// whose own theme is flat, where glass reads as a foreign object.
    Flat,
    /// Flat without the border either — the panel and nothing else.
    Borderless,
}

impl Surface {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "aero" | "glass" | "gadget" => Surface::Aero,
            "flat" => Surface::Flat,
            "borderless" | "none" => Surface::Borderless,
            _ => Surface::System,
        }
    }

    /// Is there a drop shadow to leave room for? The window is larger than the
    /// visible panel by [`Metrics::shadow`] precisely to hold it, so a surface
    /// without one must not reserve the margin.
    pub fn has_shadow(self) -> bool {
        matches!(self, Surface::System | Surface::Aero)
    }
}

/// How much room the layout gives itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Density {
    /// The DPI-derived spacing, unchanged.
    System,
    Compact,
    Normal,
    Roomy,
}

impl Density {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "compact" | "dense" | "tight" => Density::Compact,
            "normal" | "default" => Density::Normal,
            "roomy" | "comfortable" | "relaxed" => Density::Roomy,
            _ => Density::System,
        }
    }

    /// Multiplier for the spacing metrics. Deliberately mild: the panel is
    /// small, and a factor that reads as gentle here would be drastic on a
    /// full window.
    pub fn factor(self) -> f32 {
        match self {
            Density::System | Density::Normal => 1.0,
            Density::Compact => 0.86,
            Density::Roomy => 1.18,
        }
    }
}

/// Weight of the section headers, as an OpenType weight class.
///
/// A number rather than a toolkit enum: this crate does not know what a
/// `DWRITE_FONT_WEIGHT` is, and 400/500/600/700 mean the same thing to every
/// text stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FontWeight(pub u16);

impl FontWeight {
    /// Semi-bold, which is what the section labels have always been drawn in.
    pub const DEFAULT: FontWeight = FontWeight(600);

    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "regular" | "normal" | "400" => FontWeight(400),
            "medium" | "500" => FontWeight(500),
            "semibold" | "semi-bold" | "600" => FontWeight(600),
            "bold" | "700" => FontWeight(700),
            _ => Self::DEFAULT,
        }
    }
}

/// Colour overrides, each `"system"` unless set.
///
/// Only the colours a person actually reasons about are exposed. Everything
/// else the palette holds — the sheen, the hover highlight, the secondary and
/// faint text — is derived from these, so the set stays coherent instead of
/// becoming twenty knobs that can contradict each other.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Colors {
    /// Base colour of the glass body. The gradient is rebuilt around it with
    /// the same lightness spread the built-in palette uses.
    pub panel: String,
    /// Primary text.
    pub text: String,
    /// The dimmer text: times, meta lines, the footer.
    pub text_muted: String,
    /// Separators between sections.
    pub separator: String,
    /// The "now" colour — the accent, used for the running event, the now
    /// line and the day progress.
    pub now: String,
    /// Overdue tasks.
    pub overdue: String,
    /// The badge on overlapping events.
    pub conflict: String,
}

impl Default for Colors {
    fn default() -> Self {
        Self {
            panel: SYSTEM.into(),
            text: SYSTEM.into(),
            text_muted: SYSTEM.into(),
            separator: SYSTEM.into(),
            now: SYSTEM.into(),
            overdue: SYSTEM.into(),
            conflict: SYSTEM.into(),
        }
    }
}

/// The value that means "leave this to the system".
const SYSTEM: &str = "system";

/// Customisation layered on top of the system-derived look.
///
/// Not to be confused with [`SystemVisuals`], which is what the operating
/// system reports. This is what the user asked for on top of it, and every
/// field defaults to `"system"`, which is to say "nothing".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    /// `"system"`, `"aero"`, `"flat"` or `"borderless"`.
    pub surface: String,
    /// `"system"`, `"compact"`, `"normal"` or `"roomy"`.
    pub density: String,
    /// A font family name, or `"system"`.
    pub font_family: String,
    /// Added to every font size, in device independent pixels at `scale` 1.
    ///
    /// Independent of `scale` on purpose: `scale` moves the whole layout, and
    /// sometimes only the text needs to grow. The boxes that hold text grow
    /// with it, or the larger glyphs would simply be clipped.
    pub font_size_offset: f32,
    /// `"system"`, `"regular"`, `"medium"`, `"semibold"` or `"bold"`.
    pub header_weight: String,
    pub colors: Colors,
    /// Per-calendar colour overrides, keyed by calendar id or by the calendar
    /// name shown in the widget. Provider colours are not always
    /// distinguishable at 0.82 opacity.
    pub calendar_colors: BTreeMap<String, String>,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            surface: SYSTEM.into(),
            density: SYSTEM.into(),
            font_family: SYSTEM.into(),
            font_size_offset: 0.0,
            header_weight: SYSTEM.into(),
            colors: Colors::default(),
            calendar_colors: BTreeMap::new(),
        }
    }
}

impl Appearance {
    /// Do these two differ in anything the renderer fixes at construction?
    ///
    /// The split matters because the two answers cost very different things.
    /// The font family, the header weight, the size offset, the density and the
    /// surface style are baked into the DirectWrite text formats and into
    /// [`Metrics`], neither of which can be changed afterwards, so a change
    /// there means building the renderer again — tearing down the Direct3D and
    /// Direct2D pipeline and resizing the window. Colours are uploaded per
    /// frame and only need `set_palette`.
    ///
    /// Comparing whole `Appearance` values instead would rebuild everything for
    /// a one-character edit to a colour, which is the common case: someone
    /// nudging `colors.now` in a theme file sees the widget flicker on every
    /// save.
    pub fn layout_differs(&self, other: &Self) -> bool {
        self.surface != other.surface
            || self.density != other.density
            || self.font_family != other.font_family
            || self.header_weight != other.header_weight
            || (self.font_size_offset() - other.font_size_offset()).abs() > f32::EPSILON
    }

    pub fn surface(&self) -> Surface {
        Surface::parse(&self.surface)
    }

    /// The surface style actually in force.
    ///
    /// A contrast theme keeps its own flatness — that is the point of it — so
    /// [`Palette::customize`] returns before it ever reaches `apply_surface`,
    /// and the palette stays on [`Surface::System`]. `Metrics` has to make the
    /// same call or the window is sized for one surface while the palette draws
    /// another: a contrast theme with `"surface": "borderless"` would drop the
    /// shadow margin from the geometry while the palette still drew the shadow
    /// and the border into it.
    pub fn effective_surface(&self, high_contrast: bool) -> Surface {
        if high_contrast {
            Surface::System
        } else {
            self.surface()
        }
    }

    pub fn density(&self) -> Density {
        Density::parse(&self.density)
    }

    pub fn header_weight(&self) -> FontWeight {
        FontWeight::parse(&self.header_weight)
    }

    /// The font family to ask for, or `None` for the system font.
    pub fn font_family(&self) -> Option<&str> {
        let name = self.font_family.trim();
        (!name.is_empty() && !name.eq_ignore_ascii_case(SYSTEM)).then_some(name)
    }

    /// Clamped rather than rejected, like every other numeric setting. Below
    /// about minus three the smallest labels vanish; far above plus eight
    /// nothing fits in a 380 pixel panel whatever the layout does.
    pub fn font_size_offset(&self) -> f32 {
        if self.font_size_offset.is_finite() {
            self.font_size_offset.clamp(-3.0, 8.0)
        } else {
            0.0
        }
    }

    /// Colour override for one calendar.
    ///
    /// Keyed by id first, then by the name shown in the widget. The id is what
    /// `calendar_ids` and the context menu use; the name is what somebody
    /// editing the file by hand will reach for, and both are unambiguous
    /// enough to accept.
    pub fn calendar_color(&self, id: &str, name: &str) -> Option<u32> {
        self.calendar_colors
            .get(id)
            .or_else(|| self.calendar_colors.get(name))
            .and_then(|v| parse_hex(v))
    }

    /// Does anything here touch colour or chrome? Used to say so when a
    /// contrast theme is going to ignore all of it.
    fn changes_colours(&self) -> bool {
        self.surface() != Surface::System
            || self.colors != Colors::default()
            || !self.calendar_colors.is_empty()
    }
}

/// Every colour of one appearance.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    /// Contrast theme: gradients, sheen and shadow are dropped, the panel is
    /// opaque and every colour comes from the system.
    pub high_contrast: bool,
    /// Draw opaque — in the contrast theme, and when the user has turned
    /// transparency effects off.
    pub force_opaque: bool,
    /// Is motion wanted? Follows the system's "show animations" setting.
    pub animations: bool,

    pub panel_top: u32,
    pub panel_mid: u32,
    pub panel_bottom: u32,
    /// Colour of the sheen and the inner edge; white in both appearances.
    pub sheen: u32,
    pub sheen_gloss: f32,
    pub sheen_border: f32,
    /// Separators. **Dark** rather than white in the light appearance: a
    /// white line on light glass is invisible.
    pub rule: u32,
    pub rule_alpha: f32,
    /// Highlight behind the row under the pointer. Appearance dependent as
    /// well: on a light background it has to darken, not lighten.
    pub hover: u32,
    pub hover_alpha: f32,
    pub border_outer: u32,
    pub border_outer_alpha: f32,
    pub shadow_alpha: f32,

    pub text_primary: u32,
    pub text_secondary: u32,
    pub text_dim: u32,
    pub text_faint: u32,

    pub accent: u32,
    pub accent_soft: u32,
    pub overdue: u32,
    pub ok_green: u32,
    /// Problems the user has to act on: a broken configuration, a sign-in that
    /// expired, setup that was never finished. Footer text, in other words.
    pub warn: u32,
    /// Overlapping appointments: the badge in the header and the time of a
    /// clashing row.
    ///
    /// The same colour as [`Self::warn`] in every built-in palette, and a
    /// separate field only so the customisation can move one without the
    /// other. A conflict is a fact about the day; a broken configuration is
    /// something gone wrong with the program, and somebody who tints their
    /// conflict marks to taste has not asked for their error messages to
    /// follow along.
    pub conflict: u32,
}

impl Palette {
    /// Builds the palette from the system settings and the configuration.
    pub fn resolve(pref: ThemePref, accent_cfg: &str, vis: SystemVisuals) -> Self {
        // A system contrast theme outranks any configuration: whoever turns
        // it on needs it, and an application that overrides it becomes
        // unusable.
        if pref.high_contrast(vis) {
            return Self::high_contrast(vis);
        }

        let light = match pref {
            ThemePref::Dark => false,
            ThemePref::Light => true,
            ThemePref::Contrast => unreachable!("caught by the high contrast branch above"),
            ThemePref::System => vis.light,
        };

        let mut p = if light { Self::light() } else { Self::dark() };
        p.animations = vis.animations;
        p.force_opaque = !vis.transparency;

        // An explicit colour in the configuration beats the system one.
        let raw = parse_hex(accent_cfg).or(vis.accent);
        if let Some(raw) = raw {
            p.accent = readable_accent(raw, light);
            // Tinted background of the hero card: the accent pulled far
            // towards the panel colour so text on it stays readable.
            p.accent_soft = mix(p.accent, p.panel_mid, if light { 0.80 } else { 0.72 });
            p.overdue = distinct_from(p.overdue, p.accent);
        }
        p
    }

    /// Applies the optional customisation, returning anything it had to
    /// correct or refuse.
    ///
    /// The notes are meant for the log. They are returned rather than written
    /// because this module calls no operating system and opens no file; the
    /// front end decides where they go.
    ///
    /// Order matters. The panel colour comes first, because everything drawn
    /// on it is measured against it; then the text and semantic colours, which
    /// a person may want to set explicitly even after the panel moved them;
    /// then the surface, which only removes chrome; and last the readability
    /// correction, which sees the finished result.
    #[must_use = "the notes belong in the log, or a silently corrected colour looks like a bug"]
    pub fn customize(&mut self, custom: &Appearance) -> Vec<String> {
        let mut notes = Vec::new();

        // A contrast theme outranks the configuration for everything that
        // carries meaning. Its colours come from the system because there are
        // several such schemes with entirely different palettes, and its
        // flatness is the point rather than a style choice. Typography and
        // density are not part of that bargain — a larger font or more room
        // helps — and those live in `Metrics`, which applies them regardless.
        if self.high_contrast {
            if custom.changes_colours() {
                notes.push(
                    "A contrast theme is active, so the custom colours and surface style are \
                     ignored — the contrast the mode exists for comes first. Typography and \
                     density still apply."
                        .into(),
                );
            }
            return notes;
        }

        let colors = &custom.colors;
        self.apply_panel(colors, &mut notes);
        self.apply_text(colors, &mut notes);
        self.apply_semantic(colors, &mut notes);
        self.apply_surface(custom.surface());
        self.enforce_readability(colors, &mut notes);
        notes
    }

    /// The panel colour, and everything whose readability depends on it.
    fn apply_panel(&mut self, colors: &Colors, notes: &mut Vec<String>) {
        let Some(base) = checked_hex("panel", &colors.panel, notes) else {
            return;
        };

        // Keep the gradient's shape. The built-in palette lifts the top and
        // drops the bottom by a fixed amount of lightness, and reproducing
        // those two deltas around the chosen colour is what keeps the surface
        // looking like glass instead of a flat rectangle in a rounded box.
        let reference = if self.dark {
            Self::dark()
        } else {
            Self::light()
        };
        let (h, s, l) = rgb_to_hsl(base);
        let (_, _, l_top) = rgb_to_hsl(reference.panel_top);
        let (_, _, l_mid) = rgb_to_hsl(reference.panel_mid);
        let (_, _, l_bottom) = rgb_to_hsl(reference.panel_bottom);

        self.panel_mid = base;
        self.panel_top = hsl_to_rgb(h, s, (l + (l_top - l_mid)).clamp(0.0, 1.0));
        self.panel_bottom = hsl_to_rgb(h, s, (l + (l_bottom - l_mid)).clamp(0.0, 1.0));

        // Whether the panel is light decides how the edges have to be drawn,
        // and that is not the same question as which theme was requested. A
        // white separator on light glass is invisible, and so is white text.
        // If the chosen colour crosses that line, everything that depends on
        // it follows — a dark panel in the light theme is a legitimate thing
        // to ask for, and it has to remain usable.
        let now_dark = l < 0.5;
        if now_dark != self.dark {
            // Read before `follow_panel_lightness`, which is what changes it.
            // Naming the theme afterwards names the one being switched *to*,
            // and the message then contradicts itself.
            let was_dark = self.dark;
            self.follow_panel_lightness(now_dark);
            notes.push(format!(
                "The panel colour #{base:06X} is {} than the {} theme expects, so the text, \
                 separators and edges were taken from the other one.",
                if now_dark { "darker" } else { "lighter" },
                if was_dark { "dark" } else { "light" },
            ));
        }
    }

    /// Takes everything whose value depends on a light or dark background from
    /// the other built-in palette. The panel colours and anything derived from
    /// the accent are left alone: those have already been decided.
    fn follow_panel_lightness(&mut self, now_dark: bool) {
        let other = if now_dark {
            Self::dark()
        } else {
            Self::light()
        };
        self.dark = now_dark;
        self.sheen = other.sheen;
        self.sheen_gloss = other.sheen_gloss;
        self.sheen_border = other.sheen_border;
        self.rule = other.rule;
        self.rule_alpha = other.rule_alpha;
        self.hover = other.hover;
        self.hover_alpha = other.hover_alpha;
        self.border_outer = other.border_outer;
        self.border_outer_alpha = other.border_outer_alpha;
        self.text_primary = other.text_primary;
        self.text_secondary = other.text_secondary;
        self.text_dim = other.text_dim;
        self.text_faint = other.text_faint;
        // The accent keeps its hue but has to be readable against the new
        // background, which is the one thing `readable_accent` exists for.
        self.accent = readable_accent(self.accent, !now_dark);
        self.accent_soft = mix(
            self.accent,
            self.panel_mid,
            if now_dark { 0.72 } else { 0.80 },
        );
    }

    /// Primary and muted text. The two shades below each are derived rather
    /// than exposed: four independent greys are four chances to produce a set
    /// that no longer reads as one family.
    fn apply_text(&mut self, colors: &Colors, notes: &mut Vec<String>) {
        if let Some(text) = checked_hex("text", &colors.text, notes) {
            self.text_primary = text;
            self.text_secondary = mix(text, self.panel_mid, 0.25);
        }
        if let Some(muted) = checked_hex("text_muted", &colors.text_muted, notes) {
            self.text_dim = muted;
            self.text_faint = mix(muted, self.panel_mid, 0.35);
        }
        if let Some(rule) = checked_hex("separator", &colors.separator, notes) {
            self.rule = rule;
            // The built-in rule is a white or black hairline composited at 9%,
            // which is how it reads as a hairline at all. Keeping that alpha
            // for a named colour made the setting look like it did nothing:
            // 9% of anything against the panel is the panel. Somebody who
            // names a separator colour means that colour, so the base becomes
            // solid — the call sites that want a quieter line still scale it
            // down from here, which keeps the hierarchy between them.
            self.rule_alpha = 1.0;
        }
    }

    /// The three colours that mean something rather than decorate.
    fn apply_semantic(&mut self, colors: &Colors, notes: &mut Vec<String>) {
        if let Some(now) = checked_hex("now", &colors.now, notes) {
            self.accent = now;
            self.accent_soft = mix(now, self.panel_mid, if self.dark { 0.72 } else { 0.80 });
            // An explicit "now" can collide with the overdue colour just as a
            // system accent can, and the rule that keeps them apart is the
            // same one.
            self.overdue = distinct_from(self.overdue, self.accent);
        }
        // Set after the derivation above, so an explicit overdue colour is not
        // then moved away from the accent behind the user's back.
        if let Some(overdue) = checked_hex("overdue", &colors.overdue, notes) {
            self.overdue = overdue;
        }
        // Only the conflict marks. `warn` is the footer's "something is wrong
        // with the program" colour and stays where the built-in palette put
        // it: tinting the conflict badge is a taste decision, and it should
        // not quietly restyle the error messages as well.
        if let Some(conflict) = checked_hex("conflict", &colors.conflict, notes) {
            self.conflict = conflict;
        }
    }

    /// Removes chrome. Nothing here adds any, so a surface style can never
    /// bring gloss back into a mode that switched it off.
    fn apply_surface(&mut self, surface: Surface) {
        match surface {
            Surface::System | Surface::Aero => {}
            Surface::Flat | Surface::Borderless => {
                self.panel_top = self.panel_mid;
                self.panel_bottom = self.panel_mid;
                self.sheen_gloss = 0.0;
                self.sheen_border = 0.0;
                self.shadow_alpha = 0.0;
                if surface == Surface::Borderless {
                    self.border_outer_alpha = 0.0;
                }
            }
        }
    }

    /// Pulls a custom colour into a readable band against the panel it sits
    /// on.
    ///
    /// Only colours that were actually overridden are touched — the built-in
    /// palettes were designed as a set and are left exactly as they are. The
    /// panel is usually translucent, so the true contrast depends on the
    /// wallpaper behind it and cannot be known here; measuring against the
    /// panel colour is the best available proxy and is far better than letting
    /// a settings file produce grey on grey.
    fn enforce_readability(&mut self, colors: &Colors, notes: &mut Vec<String>) {
        let bg = self.panel_mid;
        // WCAG AA for body text is 4.5; the thresholds below it are for text
        // and marks that are deliberately quieter and only have to remain
        // findable.
        let mut fix = |label: &str, value: &mut u32, min: f32| {
            let corrected = ensure_contrast(*value, bg, min);
            if corrected != *value {
                notes.push(format!(
                    "{label} #{:06X} was too close to the panel to read (contrast {:.1}:1); \
                     using #{corrected:06X} instead.",
                    *value,
                    contrast_ratio(*value, bg),
                ));
                *value = corrected;
            }
        };

        if parse_hex(&colors.text).is_some() {
            fix("text", &mut self.text_primary, 4.5);
            fix("the derived secondary text", &mut self.text_secondary, 3.5);
        }
        if parse_hex(&colors.text_muted).is_some() {
            fix("text_muted", &mut self.text_dim, 3.0);
            fix("the derived faint text", &mut self.text_faint, 2.2);
        }
        if parse_hex(&colors.now).is_some() {
            fix("now", &mut self.accent, 3.0);
        }
        if parse_hex(&colors.overdue).is_some() {
            fix("overdue", &mut self.overdue, 3.0);
        }
        if parse_hex(&colors.conflict).is_some() {
            fix("conflict", &mut self.conflict, 3.0);
        }
        // A custom panel moves the background under text that was never
        // overridden, so the built-in text has to be checked against it too —
        // all four shades, not just the two loudest. The quiet ones are where
        // a mid grey panel does its damage: `"panel": "#808080"` left the
        // faint text at roughly 1.3:1, which is not quiet, it is invisible.
        if parse_hex(&colors.panel).is_some() {
            fix("the text on the custom panel", &mut self.text_primary, 4.5);
            fix(
                "the secondary text on the custom panel",
                &mut self.text_secondary,
                3.5,
            );
            fix(
                "the muted text on the custom panel",
                &mut self.text_dim,
                3.0,
            );
            fix(
                "the faint text on the custom panel",
                &mut self.text_faint,
                2.2,
            );
            // The marks that mean something have to survive the move as well.
            // `follow_panel_lightness` re-derives the accent but leaves these
            // at the values of the palette that was switched away from, and
            // `warn` is the footer's "your configuration is broken" colour —
            // the one message that must not be the one that disappears. A
            // light panel under the dark palette left it at roughly 1.5:1.
            let was = self.accent;
            fix("the now marker on the custom panel", &mut self.accent, 3.0);
            fix("overdue on the custom panel", &mut self.overdue, 3.0);
            fix(
                "the conflict mark on the custom panel",
                &mut self.conflict,
                3.0,
            );
            fix(
                "the warning colour on the custom panel",
                &mut self.warn,
                3.0,
            );
            fix(
                "the success colour on the custom panel",
                &mut self.ok_green,
                3.0,
            );
            if self.accent != was {
                // The soft accent is a blend of the accent with the panel, so
                // it goes stale the moment the accent moves.
                self.accent_soft = mix(
                    self.accent,
                    self.panel_mid,
                    if self.dark { 0.72 } else { 0.80 },
                );
            }
        }
    }

    /// The opacity actually to be used.
    pub fn opacity(&self, configured: f32) -> f32 {
        if self.force_opaque {
            1.0
        } else {
            configured.clamp(0.15, 1.0)
        }
    }

    /// Palette built from the contrast theme's system colours.
    ///
    /// There are several contrast themes with entirely different values, so
    /// nothing is guessed here — the host reports what the system says.
    /// Gradients, sheen and shadow are switched off: they would destroy the
    /// very contrast the mode exists for.
    fn high_contrast(vis: SystemVisuals) -> Self {
        // Without reported colours there is nothing to build on; a readable
        // black on white beats inventing a scheme.
        let c = vis.contrast.unwrap_or(ContrastColors {
            window: 0xFF_FFFF,
            text: 0x00_0000,
            gray: 0x60_6060,
            highlight: 0x00_78D4,
            hot: 0x00_5A9E,
        });
        let (window, text, gray, highlight, hot) = (c.window, c.text, c.gray, c.highlight, c.hot);
        // The lightness of the window background decides whether this is a
        // light or a dark contrast theme.
        let (_, _, l) = rgb_to_hsl(window);
        let dark = l < 0.5;

        Self {
            dark,
            high_contrast: true,
            force_opaque: true,
            animations: vis.animations,

            panel_top: window,
            panel_mid: window,
            panel_bottom: window,
            sheen: text,
            sheen_gloss: 0.0,
            sheen_border: 1.0,
            rule: text,
            rule_alpha: 0.55,
            hover: highlight,
            hover_alpha: 0.35,
            border_outer: text,
            border_outer_alpha: 1.0,
            shadow_alpha: 0.0,

            text_primary: text,
            text_secondary: text,
            // The system's grey text stays deliberately readable here.
            text_dim: gray,
            text_faint: gray,

            accent: hot,
            accent_soft: window,
            overdue: distinct_from(hot, highlight),
            ok_green: hot,
            warn: hot,
            conflict: hot,
        }
    }

    fn dark() -> Self {
        Self {
            dark: true,
            high_contrast: false,
            force_opaque: false,
            animations: true,
            panel_top: 0x2E_3646,
            panel_mid: 0x23_2A38,
            panel_bottom: 0x14_1922,
            sheen: 0xFF_FFFF,
            sheen_gloss: 0.11,
            sheen_border: 0.14,
            rule: 0xFF_FFFF,
            rule_alpha: 0.09,
            hover: 0xFF_FFFF,
            hover_alpha: 0.075,
            border_outer: 0x00_0000,
            border_outer_alpha: 0.50,
            shadow_alpha: 0.038,

            text_primary: 0xEE_F2F8,
            text_secondary: 0xBC_C6D4,
            text_dim: 0x81_8D9E,
            text_faint: 0x5E_6879,

            accent: 0x5A_AEFF,
            accent_soft: 0x2C_4A6E,
            overdue: 0xFF_7A7A,
            ok_green: 0x5C_D6A0,
            warn: 0xFF_C05C,
            conflict: 0xFF_C05C,
        }
    }

    /// Light glass: the gradient runs **lighter** towards the bottom, not
    /// darker — otherwise the surface looks dirty instead of frosted. Edges
    /// and sheen have to be pulled back sharply because white on white gives
    /// nothing; here the depth comes almost entirely from the outer shadow.
    fn light() -> Self {
        Self {
            dark: false,
            high_contrast: false,
            force_opaque: false,
            animations: true,
            panel_top: 0xFA_FBFD,
            panel_mid: 0xF2_F4F8,
            panel_bottom: 0xE7_EBF1,
            sheen: 0xFF_FFFF,
            sheen_gloss: 0.60,
            sheen_border: 0.85,
            rule: 0x2A_3140,
            rule_alpha: 0.13,
            hover: 0x2A_3140,
            hover_alpha: 0.060,
            border_outer: 0x5A_6376,
            border_outer_alpha: 0.30,
            shadow_alpha: 0.030,

            text_primary: 0x18_1D26,
            text_secondary: 0x3D_4653,
            text_dim: 0x69_7383,
            text_faint: 0x8D_96A4,

            accent: 0x1B_6FD6,
            accent_soft: 0xD4_E4F8,
            overdue: 0xC0_3535,
            ok_green: 0x1D_8A5C,
            warn: 0xA8_6A08,
            conflict: 0xA8_6A08,
        }
    }
}

/// Lifts or lowers a colour's lightness into a readable band.
///
/// A system accent colour may be arbitrarily dark or garish — black and neon
/// yellow are both valid settings. As text on glass that would be unusable, so
/// only the lightness is corrected while the hue is left alone: the user still
/// recognises their colour.
fn readable_accent(rgb: u32, light_theme: bool) -> u32 {
    let (h, s, l) = rgb_to_hsl(rgb);
    // Lift very pale, near grey accents a little, or the "now" and "active"
    // states disappear visually — but only where there is a hue to lift.
    // `rgb_to_hsl` reports hue 0 for anything neutral, so saturating a black,
    // white or grey accent unconditionally would invent a colour the user
    // never chose: `"accent": "#FFFFFF"` came out dusty pink, and
    // `distinct_from` then pushed `overdue` off to amber because it believed
    // the accent was red.
    let s = if s > 0.02 { s.max(0.35) } else { s };
    let l = if light_theme {
        l.clamp(0.30, 0.46)
    } else {
        l.clamp(0.58, 0.76)
    };
    hsl_to_rgb(h, s, l)
}

/// Distance between two hues on the colour wheel, 0.0 to 0.5.
fn hue_distance(a: f32, b: f32) -> f32 {
    let d = (a - b).abs().fract();
    d.min(1.0 - d)
}

/// Keeps the overdue colour distinguishable from the accent.
///
/// A system accent colour may well be red — and then "now", the day progress
/// and the now line all take the same red as the overdue tasks, and the two
/// meanings can no longer be told apart. In that case the warning colour moves
/// to amber or magenta, whichever sits further from the accent.
fn distinct_from(overdue: u32, accent: u32) -> u32 {
    /// Roughly 25 degrees on the colour wheel.
    const MIN_GAP: f32 = 0.07;

    let (h_o, s_o, l_o) = rgb_to_hsl(overdue);
    let (h_a, _, _) = rgb_to_hsl(accent);
    if hue_distance(h_o, h_a) >= MIN_GAP {
        return overdue;
    }
    // Amber (~32 degrees) and magenta (~317) both read as warning colours.
    [0.09_f32, 0.88]
        .into_iter()
        .max_by(|x, y| {
            hue_distance(*x, h_a)
                .partial_cmp(&hue_distance(*y, h_a))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|h| hsl_to_rgb(h, s_o, l_o))
        .unwrap_or(overdue)
}

fn rgb_to_hsl(rgb: u32) -> (f32, f32, f32) {
    let r = ((rgb >> 16) & 0xFF) as f32 / 255.0;
    let g = ((rgb >> 8) & 0xFF) as f32 / 255.0;
    let b = (rgb & 0xFF) as f32 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) * 0.5;
    let d = max - min;

    if d.abs() < f32::EPSILON {
        return (0.0, 0.0, l);
    }
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if max == r {
        ((g - b) / d + if g < b { 6.0 } else { 0.0 }) / 6.0
    } else if max == g {
        ((b - r) / d + 2.0) / 6.0
    } else {
        ((r - g) / d + 4.0) / 6.0
    };
    (h, s, l)
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> u32 {
    if s.abs() < f32::EPSILON {
        let v = (l * 255.0).round() as u32;
        return (v << 16) | (v << 8) | v;
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let hue = |mut t: f32| {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        let v = if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        };
        (v.clamp(0.0, 1.0) * 255.0).round() as u32
    };
    (hue(h + 1.0 / 3.0) << 16) | (hue(h) << 8) | hue(h - 1.0 / 3.0)
}

/// `"#4FA3FF"` or `"4FA3FF"`; anything else, `"system"` included, is `None`.
fn parse_hex(s: &str) -> Option<u32> {
    let t = s.trim().trim_start_matches('#');
    (t.len() == 6)
        .then(|| u32::from_str_radix(t, 16).ok())
        .flatten()
}

/// [`parse_hex`], but a value that was meant as a colour and is not one gets
/// said out loud.
///
/// Out-of-range numbers are clamped rather than rejected everywhere else in
/// the settings, and a malformed colour follows the same rule: it falls back
/// to the system value. Silently, though, it looks exactly like the setting
/// having no effect, which is the hardest kind of thing to debug.
fn checked_hex(field: &str, value: &str, notes: &mut Vec<String>) -> Option<u32> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case(SYSTEM) {
        return None;
    }
    match parse_hex(trimmed) {
        Some(rgb) => Some(rgb),
        None => {
            notes.push(format!(
                "{field}: '{trimmed}' is not a colour — expected \"#RRGGBB\" or \"system\". \
                 Using the system value."
            ));
            None
        }
    }
}

/// Relative luminance as WCAG 2 defines it.
fn relative_luminance(rgb: u32) -> f32 {
    let channel = |v: f32| {
        if v <= 0.03928 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    let r = channel(((rgb >> 16) & 0xFF) as f32 / 255.0);
    let g = channel(((rgb >> 8) & 0xFF) as f32 / 255.0);
    let b = channel((rgb & 0xFF) as f32 / 255.0);
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

/// Contrast ratio between two colours, 1.0 (identical) to 21.0 (black on
/// white).
fn contrast_ratio(a: u32, b: u32) -> f32 {
    let (la, lb) = (relative_luminance(a), relative_luminance(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// Moves a colour's lightness until it reads against `bg`, keeping its hue.
///
/// The same bargain [`readable_accent`] strikes with the system accent: the
/// user still recognises their colour, it is simply light or dark enough to
/// see. Where no lightness reaches the target — a mid grey background leaves
/// little room in either direction — the best available is returned rather
/// than an arbitrary black or white, and the caller says so.
fn ensure_contrast(fg: u32, bg: u32, min: f32) -> u32 {
    if contrast_ratio(fg, bg) >= min {
        return fg;
    }
    let (h, s, l) = rgb_to_hsl(fg);

    // Sweep the whole lightness range and take the candidate that meets the
    // target while staying closest to what was asked for. Searching outwards
    // from the original rather than jumping to an extreme is what keeps a
    // corrected colour recognisable.
    let mut best = fg;
    let mut best_ratio = contrast_ratio(fg, bg);
    let mut chosen: Option<(u32, f32)> = None;
    for step in 0..=100 {
        let candidate_l = step as f32 / 100.0;
        let candidate = hsl_to_rgb(h, s, candidate_l);
        let ratio = contrast_ratio(candidate, bg);
        if ratio > best_ratio {
            best_ratio = ratio;
            best = candidate;
        }
        if ratio >= min {
            let distance = (candidate_l - l).abs();
            if chosen.is_none_or(|(_, d)| distance < d) {
                chosen = Some((candidate, distance));
            }
        }
    }
    chosen.map(|(c, _)| c).unwrap_or(best)
}

/// All metrics in device independent pixels at `scale = 1.0`.
///
/// `PartialEq` so a caller can ask the one question that matters — "did any
/// of this move?" — instead of enumerating the inputs that feed
/// [`Metrics::resolve`]. Every field is derived from those inputs by
/// multiplication, so equality here means the geometry and the renderer's
/// cached copy still agree.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    /// Free margin inside the window for the drop shadow. The glass body is
    /// inset from the window edge by this amount.
    pub shadow: f32,
    pub pad: f32,
    pub corner: f32,

    pub header_h: f32,
    /// Height of the day rail below the header.
    pub rail_h: f32,
    pub hero_h: f32,
    pub section_gap: f32,
    pub section_label_h: f32,
    pub event_row_h: f32,
    pub task_row_h: f32,
    pub row_gap: f32,
    pub footer_h: f32,
    pub nowline_h: f32,

    pub time_col_w: f32,
    pub rel_col_w: f32,
    pub check_size: f32,
    pub indent: f32,
    pub icon_btn: f32,
    pub scrollbar_w: f32,

    pub fs_title: f32,
    pub fs_clock: f32,
    pub fs_subtitle: f32,
    pub fs_section: f32,
    pub fs_row: f32,
    pub fs_meta: f32,
    pub fs_footer: f32,
}

impl Metrics {
    pub fn new(scale: f32) -> Self {
        Self::with_shadow(scale, true)
    }

    /// `shadow = false` for the acrylic mode: there the compositor fills the
    /// whole window rectangle, and a free margin would show up as a square box
    /// around the panel.
    pub fn with_shadow(scale: f32, shadow: bool) -> Self {
        let s = |v: f32| v * scale;
        Self {
            shadow: if shadow { s(14.0) } else { 0.0 },
            pad: s(15.0),
            corner: s(11.0),

            header_h: s(66.0),
            rail_h: s(7.0),
            hero_h: s(44.0),
            section_gap: s(13.0),
            section_label_h: s(19.0),
            event_row_h: s(30.0),
            task_row_h: s(29.0),
            row_gap: s(2.0),
            footer_h: s(21.0),
            nowline_h: s(15.0),

            time_col_w: s(44.0),
            rel_col_w: s(66.0),
            check_size: s(14.0),
            indent: s(15.0),
            icon_btn: s(27.0),
            scrollbar_w: s(3.0),

            fs_title: s(17.0),
            fs_clock: s(14.0),
            fs_subtitle: s(11.5),
            fs_section: s(9.5),
            fs_row: s(12.5),
            fs_meta: s(10.5),
            fs_footer: s(10.0),
        }
    }

    /// The metrics a running widget actually uses: DPI scale, whether the
    /// backdrop leaves room for a shadow, and the customisation on top.
    ///
    /// Density and the font size offset apply even under a contrast theme.
    /// They are not part of the bargain a contrast theme strikes — nothing
    /// about more room or larger text works against contrast — and somebody
    /// who needs one very often needs the other. The surface style is the part
    /// that does not survive it, which is why `high_contrast` is needed here
    /// and not only in [`Palette::customize`]: see
    /// [`Appearance::effective_surface`].
    pub fn resolve(
        scale: f32,
        backdrop_leaves_room: bool,
        custom: &Appearance,
        high_contrast: bool,
    ) -> Self {
        let mut m = Self::with_shadow(
            scale,
            backdrop_leaves_room && custom.effective_surface(high_contrast).has_shadow(),
        );

        // A font offset is not just a bigger glyph. Every box that holds text
        // has to grow with it or the text is clipped in a container that did
        // not move, so the text-sized metrics are scaled by the same ratio the
        // row font grew by.
        let offset = custom.font_size_offset() * scale;
        if offset.abs() > f32::EPSILON {
            // Nothing may shrink below roughly six device independent pixels;
            // past that the smallest labels are gone rather than small.
            let floor = 6.0 * scale;
            let k = ((m.fs_row + offset) / m.fs_row).max(0.5);
            for size in [
                &mut m.fs_title,
                &mut m.fs_clock,
                &mut m.fs_subtitle,
                &mut m.fs_section,
                &mut m.fs_row,
                &mut m.fs_meta,
                &mut m.fs_footer,
            ] {
                *size = (*size + offset).max(floor);
            }
            for box_metric in [
                &mut m.header_h,
                &mut m.hero_h,
                &mut m.section_label_h,
                &mut m.event_row_h,
                &mut m.task_row_h,
                &mut m.footer_h,
                &mut m.nowline_h,
                &mut m.time_col_w,
                &mut m.rel_col_w,
                &mut m.check_size,
                &mut m.icon_btn,
            ] {
                *box_metric *= k;
            }
        }

        // Density is the spacing between things, not the size of the things
        // themselves — a compact layout with clipped text would be no use.
        let factor = custom.density().factor();
        if (factor - 1.0).abs() > f32::EPSILON {
            for spacing in [
                &mut m.pad,
                &mut m.section_gap,
                &mut m.row_gap,
                &mut m.indent,
                &mut m.header_h,
                &mut m.hero_h,
                &mut m.event_row_h,
                &mut m.task_row_h,
                &mut m.footer_h,
            ] {
                *spacing *= factor;
            }
        }
        m
    }
}

// Weekday names, month names and time formats come from [`crate::i18n`],
// which asks the operating system so they are right in every locale.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mix_hits_both_ends() {
        assert_eq!(mix(0x000000, 0xFFFFFF, 0.0), 0x000000);
        assert_eq!(mix(0x000000, 0xFFFFFF, 1.0), 0xFFFFFF);
        assert_eq!(mix(0x000000, 0xFFFFFF, 0.5), 0x808080);
    }

    #[test]
    fn hsl_roundtrip_keeps_hue() {
        for color in [0x4FA3FF, 0xC03535, 0x1D8A5C, 0x808080] {
            let (h, s, l) = rgb_to_hsl(color);
            let back = hsl_to_rgb(h, s, l);
            // A rounding error of at most one step per channel.
            for shift in [16, 8, 0] {
                let a = ((color >> shift) & 0xFF) as i32;
                let b = ((back >> shift) & 0xFF) as i32;
                assert!((a - b).abs() <= 1, "{color:06X} -> {back:06X}");
            }
        }
    }

    #[test]
    fn accent_is_forced_into_a_readable_band() {
        // Black and white are both valid system accent colours and both
        // unusable as a text colour.
        for input in [0x000000, 0xFFFFFF, 0x0A0A64] {
            let (_, _, l_dark) = rgb_to_hsl(readable_accent(input, false));
            assert!((0.57..=0.77).contains(&l_dark), "dark: {input:06X}");
            let (_, _, l_light) = rgb_to_hsl(readable_accent(input, true));
            assert!((0.29..=0.47).contains(&l_light), "light: {input:06X}");
        }
    }

    #[test]
    fn accent_keeps_the_users_hue() {
        // A very dark red stays red, but gets lightened.
        let (h_in, _, _) = rgb_to_hsl(0x400000);
        let (h_out, _, l_out) = rgb_to_hsl(readable_accent(0x400000, false));
        assert!((h_in - h_out).abs() < 0.02);
        assert!(l_out > 0.5);
    }

    #[test]
    fn overdue_stays_distinguishable_from_a_red_accent() {
        let red_accent = readable_accent(0x8A_1F1F, false);
        let base_overdue = Palette::dark().overdue;
        let adjusted = distinct_from(base_overdue, red_accent);

        let (h_acc, _, _) = rgb_to_hsl(red_accent);
        let (h_adj, _, _) = rgb_to_hsl(adjusted);
        assert!(
            hue_distance(h_acc, h_adj) >= 0.07,
            "warning colour {adjusted:06X} sits too close to the accent {red_accent:06X}"
        );
    }

    #[test]
    fn overdue_is_left_alone_when_the_accent_is_blue() {
        let blue = readable_accent(0x00_78D4, false);
        let base = Palette::dark().overdue;
        assert_eq!(distinct_from(base, blue), base);
    }

    #[test]
    fn explicit_hex_overrides_system_accent() {
        assert_eq!(parse_hex("#4FA3FF"), Some(0x4FA3FF));
        assert_eq!(parse_hex("4fa3ff"), Some(0x4FA3FF));
        assert_eq!(parse_hex("system"), None);
        assert_eq!(parse_hex(""), None);
    }

    // --- Customisation ------------------------------------------------------

    fn dark_palette() -> Palette {
        Palette::resolve(ThemePref::Dark, "system", SystemVisuals::default())
    }

    /// The whole point of the defaults: an untouched `Appearance` has to leave
    /// the palette exactly as the system produced it.
    #[test]
    fn the_default_customisation_changes_nothing() {
        let custom = Appearance::default();
        for pref in [ThemePref::Dark, ThemePref::Light] {
            let base = Palette::resolve(pref, "system", SystemVisuals::default());
            let mut customised = base;
            let notes = customised.customize(&custom);
            assert!(notes.is_empty(), "{notes:?}");

            assert_eq!(customised.panel_top, base.panel_top);
            assert_eq!(customised.panel_mid, base.panel_mid);
            assert_eq!(customised.panel_bottom, base.panel_bottom);
            assert_eq!(customised.text_primary, base.text_primary);
            assert_eq!(customised.text_dim, base.text_dim);
            assert_eq!(customised.accent, base.accent);
            assert_eq!(customised.overdue, base.overdue);
            assert_eq!(customised.warn, base.warn);
            assert_eq!(customised.sheen_gloss, base.sheen_gloss);
            assert_eq!(customised.shadow_alpha, base.shadow_alpha);
            assert_eq!(customised.border_outer_alpha, base.border_outer_alpha);
        }

        let metrics = Metrics::resolve(1.0, true, &custom, false);
        let plain = Metrics::new(1.0);
        assert_eq!(metrics.pad, plain.pad);
        assert_eq!(metrics.event_row_h, plain.event_row_h);
        assert_eq!(metrics.fs_row, plain.fs_row);
        assert_eq!(metrics.shadow, plain.shadow);
    }

    /// The accessibility switch that outranks everything. A contrast theme
    /// keeps its own colours and its flatness; anything else would defeat the
    /// mode.
    #[test]
    fn a_contrast_theme_ignores_the_custom_colours_and_says_so() {
        let mut palette = Palette::resolve(
            ThemePref::Contrast,
            "system",
            SystemVisuals {
                high_contrast: true,
                contrast: Some(ContrastColors {
                    window: 0x000000,
                    text: 0xFFFFFF,
                    gray: 0x808080,
                    highlight: 0x00FF00,
                    hot: 0xFFFF00,
                }),
                ..SystemVisuals::default()
            },
        );
        let before = palette;

        let custom = Appearance {
            surface: "aero".into(),
            colors: Colors {
                panel: "#FF00FF".into(),
                text: "#404040".into(),
                ..Colors::default()
            },
            ..Appearance::default()
        };
        let notes = palette.customize(&custom);

        assert_eq!(palette.panel_mid, before.panel_mid, "system window colour");
        assert_eq!(palette.text_primary, before.text_primary);
        assert_eq!(palette.sheen_gloss, 0.0, "no gloss in a contrast theme");
        assert_eq!(palette.shadow_alpha, 0.0);
        assert_eq!(notes.len(), 1, "the user has to be told: {notes:?}");

        // Typography and density are not part of that bargain — they help.
        let roomy = Metrics::resolve(
            1.0,
            true,
            &Appearance {
                density: "roomy".into(),
                font_size_offset: 2.0,
                ..Appearance::default()
            },
            false,
        );
        assert!(roomy.pad > Metrics::new(1.0).pad);
        assert!(roomy.fs_row > Metrics::new(1.0).fs_row);
    }

    #[test]
    fn a_custom_panel_keeps_the_gradients_shape() {
        let mut palette = dark_palette();
        let base = Palette::dark();
        let (_, _, l_top) = rgb_to_hsl(base.panel_top);
        let (_, _, l_mid) = rgb_to_hsl(base.panel_mid);

        let _ = palette.customize(&Appearance {
            colors: Colors {
                panel: "#2A1F3D".into(),
                ..Colors::default()
            },
            ..Appearance::default()
        });

        assert_eq!(palette.panel_mid, 0x2A_1F3D);
        assert_ne!(palette.panel_top, palette.panel_mid, "still a gradient");
        let (_, _, l_new_top) = rgb_to_hsl(palette.panel_top);
        let (_, _, l_new_mid) = rgb_to_hsl(palette.panel_mid);
        assert!(
            ((l_new_top - l_new_mid) - (l_top - l_mid)).abs() < 0.01,
            "the lightness spread of the built-in gradient was not reproduced"
        );
    }

    /// A light panel in the dark theme is a legitimate thing to ask for, and
    /// white-on-white is not an acceptable answer to it.
    #[test]
    fn a_panel_that_crosses_into_the_other_appearance_takes_its_edges_along() {
        let mut palette = dark_palette();
        assert!(palette.dark);
        let white_text = palette.text_primary;

        let notes = palette.customize(&Appearance {
            colors: Colors {
                panel: "#F5F5F7".into(),
                ..Colors::default()
            },
            ..Appearance::default()
        });

        assert!(!palette.dark, "a near-white panel is not a dark appearance");
        assert_ne!(palette.text_primary, white_text, "text had to follow");
        assert!(
            contrast_ratio(palette.text_primary, palette.panel_mid) >= 4.5,
            "text is unreadable on the chosen panel"
        );
        assert_eq!(
            palette.rule,
            Palette::light().rule,
            "a white separator on light glass is invisible"
        );
        assert!(!notes.is_empty(), "such a swap has to be explained");
        // The note names the theme that was active, not the one being switched
        // to. Reading `self.dark` after the switch made this say "lighter than
        // the light theme expects", which is self-contradictory and tells the
        // reader the opposite of what happened.
        let note = notes.join(" ");
        assert!(
            note.contains("lighter than the dark theme expects"),
            "the note names the wrong theme: {note}"
        );
    }

    /// A mid grey panel is the case that catches the quiet shades: it is far
    /// enough from both built-in palettes that neither's greys survive it.
    #[test]
    fn a_custom_panel_is_checked_against_every_shade_of_text() {
        let mut palette = dark_palette();
        let _ = palette.customize(&Appearance {
            colors: Colors {
                panel: "#808080".into(),
                ..Colors::default()
            },
            ..Appearance::default()
        });

        let bg = palette.panel_mid;
        // The same thresholds `enforce_readability` works to: body text at
        // WCAG AA, the quieter shades only far enough to remain findable.
        for (label, color, min) in [
            ("text_primary", palette.text_primary, 4.5),
            ("text_secondary", palette.text_secondary, 3.5),
            ("text_dim", palette.text_dim, 3.0),
            ("text_faint", palette.text_faint, 2.2),
        ] {
            let ratio = contrast_ratio(color, bg);
            assert!(
                ratio >= min - 0.05,
                "{label} sits at {ratio:.2}:1 on the custom panel, needs {min}"
            );
        }
    }

    /// Tinting the conflict marks is a taste decision. Restyling the error
    /// messages in the footer is not, and the two used to share one field.
    #[test]
    fn a_custom_conflict_colour_leaves_the_error_text_alone() {
        let mut palette = dark_palette();
        let warn = palette.warn;

        let _ = palette.customize(&Appearance {
            colors: Colors {
                conflict: "#7A5CFF".into(),
                ..Colors::default()
            },
            ..Appearance::default()
        });

        assert_ne!(palette.conflict, warn, "the conflict marks had to move");
        assert_eq!(palette.warn, warn, "the footer's warning colour must not");
    }

    /// A named separator colour has to be visible, or the setting reads as one
    /// that did nothing.
    #[test]
    fn a_custom_separator_is_not_swallowed_by_the_hairline_alpha() {
        let mut palette = dark_palette();
        assert!(
            palette.rule_alpha < 0.2,
            "precondition: the built-in rule is a hairline"
        );

        let _ = palette.customize(&Appearance {
            colors: Colors {
                separator: "#FF0000".into(),
                ..Colors::default()
            },
            ..Appearance::default()
        });

        assert_eq!(palette.rule, 0xFF_0000);
        assert_eq!(
            palette.rule_alpha, 1.0,
            "9% of any colour against the panel is the panel"
        );
    }

    /// The settings file must not be able to produce invisible text.
    #[test]
    fn unreadable_custom_colours_are_corrected_and_reported() {
        let mut palette = dark_palette();
        let panel = palette.panel_mid;

        // Almost exactly the panel colour: legal JSON, unusable on screen.
        let notes = palette.customize(&Appearance {
            colors: Colors {
                text: format!("#{panel:06X}"),
                ..Colors::default()
            },
            ..Appearance::default()
        });

        let ratio = contrast_ratio(palette.text_primary, panel);
        assert!(ratio >= 4.5, "text still unreadable at {ratio:.1}:1");
        assert!(
            notes.iter().any(|n| n.contains("text")),
            "the correction was silent: {notes:?}"
        );

        // A colour that is readable already is left exactly as asked for.
        let mut untouched = dark_palette();
        let notes = untouched.customize(&Appearance {
            colors: Colors {
                text: "#FFFFFF".into(),
                ..Colors::default()
            },
            ..Appearance::default()
        });
        assert_eq!(untouched.text_primary, 0xFF_FFFF);
        assert!(notes.is_empty(), "{notes:?}");
    }

    /// A corrected colour has to stay the colour that was asked for. Lightness
    /// moves; the hue does not.
    #[test]
    fn a_corrected_colour_keeps_its_hue() {
        let panel = Palette::dark().panel_mid;
        // A dark red that vanishes into a dark panel.
        let asked = 0x3A_0D0D;
        let fixed = ensure_contrast(asked, panel, 4.5);
        assert!(contrast_ratio(fixed, panel) >= 4.5);

        let (h_asked, _, _) = rgb_to_hsl(asked);
        let (h_fixed, _, l_fixed) = rgb_to_hsl(fixed);
        assert!((h_asked - h_fixed).abs() < 0.02, "the hue moved");
        assert!(l_fixed > 0.5, "it had to get lighter on a dark panel");
    }

    /// Where no lightness reaches the target the best available is used, and
    /// nothing loops forever or panics.
    #[test]
    fn an_impossible_contrast_returns_the_best_available() {
        // Mid grey leaves little room in either direction.
        let bg = 0x80_8080;
        let result = ensure_contrast(0x7F_7F7F, bg, 21.0);
        assert!(contrast_ratio(result, bg) > contrast_ratio(0x7F_7F7F, bg));
    }

    #[test]
    fn a_malformed_colour_falls_back_to_the_system_value_and_is_named() {
        let mut palette = dark_palette();
        let before = palette.text_primary;
        let notes = palette.customize(&Appearance {
            colors: Colors {
                text: "#GGGGGG".into(),
                overdue: "rebeccapurple".into(),
                ..Colors::default()
            },
            ..Appearance::default()
        });

        assert_eq!(palette.text_primary, before, "clamped, not applied");
        assert_eq!(notes.len(), 2, "both bad values named: {notes:?}");
        assert!(notes.iter().any(|n| n.contains("text")));
        assert!(notes.iter().any(|n| n.contains("overdue")));
    }

    #[test]
    fn the_surface_style_only_ever_removes_chrome() {
        for (style, border) in [("flat", true), ("borderless", false)] {
            let mut palette = dark_palette();
            let _ = palette.customize(&Appearance {
                surface: style.into(),
                ..Appearance::default()
            });

            assert_eq!(palette.sheen_gloss, 0.0, "{style}");
            assert_eq!(palette.shadow_alpha, 0.0, "{style}");
            assert_eq!(palette.panel_top, palette.panel_mid, "{style}");
            assert_eq!(palette.panel_bottom, palette.panel_mid, "{style}");
            assert_eq!(palette.border_outer_alpha > 0.0, border, "{style}");
        }

        // A flat surface needs no room for a shadow it does not draw.
        assert_eq!(
            Metrics::resolve(
                1.0,
                true,
                &Appearance {
                    surface: "flat".into(),
                    ..Appearance::default()
                },
                false,
            )
            .shadow,
            0.0
        );
        // Aero keeps it, and the system backdrop still wins over both.
        assert!(
            Metrics::resolve(
                1.0,
                true,
                &Appearance {
                    surface: "aero".into(),
                    ..Appearance::default()
                },
                false,
            )
            .shadow
                > 0.0
        );
        assert_eq!(
            Metrics::resolve(
                1.0,
                false,
                &Appearance {
                    surface: "aero".into(),
                    ..Appearance::default()
                },
                false,
            )
            .shadow,
            0.0,
            "the acrylic backdrop fills the whole window rectangle"
        );
    }

    /// The geometry and the palette have to agree about the surface.
    ///
    /// `customize` returns before `apply_surface` under a contrast theme, so
    /// the palette keeps the shadow. If the metrics still honoured
    /// `borderless`, the window would be sized without the margin the palette
    /// then drew the shadow and the border into.
    #[test]
    fn a_contrast_theme_ignores_the_surface_style_in_the_metrics_too() {
        let borderless = Appearance {
            surface: "borderless".into(),
            ..Appearance::default()
        };
        assert_eq!(
            Metrics::resolve(1.0, true, &borderless, false).shadow,
            0.0,
            "borderless reserves no margin when it is honoured"
        );
        assert_eq!(
            Metrics::resolve(1.0, true, &borderless, true).shadow,
            Metrics::new(1.0).shadow,
            "under contrast the surface style is ignored, as it is in the palette"
        );

        // The same question, asked of the palette: it never reaches
        // `apply_surface`, so the border stays.
        let mut palette = Palette::resolve(
            ThemePref::Contrast,
            "system",
            SystemVisuals {
                high_contrast: true,
                ..SystemVisuals::default()
            },
        );
        let before = palette.border_outer_alpha;
        let _ = palette.customize(&borderless);
        assert_eq!(
            palette.border_outer_alpha, before,
            "a contrast theme keeps its border"
        );
    }

    /// Colours are uploaded per frame; typography and the surface are not.
    #[test]
    fn only_the_baked_in_half_of_a_customisation_forces_a_rebuild() {
        let base = Appearance::default();

        let recoloured = Appearance {
            colors: Colors {
                panel: "#101820".into(),
                ..Colors::default()
            },
            ..Appearance::default()
        };
        assert_ne!(recoloured, base, "the value really did change");
        assert!(
            !base.layout_differs(&recoloured),
            "a colour must not tear down the renderer"
        );

        let mut per_calendar = Appearance::default();
        per_calendar
            .calendar_colors
            .insert("work".into(), "#ff8800".into());
        assert!(!base.layout_differs(&per_calendar));

        for changed in [
            Appearance {
                surface: "flat".into(),
                ..Appearance::default()
            },
            Appearance {
                density: "roomy".into(),
                ..Appearance::default()
            },
            Appearance {
                font_family: "Segoe UI".into(),
                ..Appearance::default()
            },
            Appearance {
                header_weight: "bold".into(),
                ..Appearance::default()
            },
            Appearance {
                font_size_offset: 2.0,
                ..Appearance::default()
            },
        ] {
            assert!(
                base.layout_differs(&changed),
                "{changed:?} is baked in at construction"
            );
        }
    }

    /// A bigger font in a box that did not grow is a clipped label.
    #[test]
    fn a_font_size_offset_grows_the_boxes_that_hold_text() {
        let plain = Metrics::new(1.0);
        let bigger = Metrics::resolve(
            1.0,
            true,
            &Appearance {
                font_size_offset: 3.0,
                ..Appearance::default()
            },
            false,
        );

        assert_eq!(bigger.fs_row, plain.fs_row + 3.0);
        assert_eq!(bigger.fs_footer, plain.fs_footer + 3.0);
        assert!(bigger.event_row_h > plain.event_row_h);
        assert!(bigger.header_h > plain.header_h);
        assert!(bigger.time_col_w > plain.time_col_w);
        // Independent of `scale`, but scaled by it: the offset is in device
        // independent pixels at scale 1.
        let at_two = Metrics::resolve(
            2.0,
            true,
            &Appearance {
                font_size_offset: 3.0,
                ..Appearance::default()
            },
            false,
        );
        assert_eq!(at_two.fs_row, Metrics::new(2.0).fs_row + 6.0);

        // Nothing disappears at the bottom end.
        let tiny = Metrics::resolve(
            1.0,
            true,
            &Appearance {
                font_size_offset: -3.0,
                ..Appearance::default()
            },
            false,
        );
        assert!(tiny.fs_footer >= 6.0, "{}", tiny.fs_footer);
        assert!(tiny.fs_section >= 6.0, "{}", tiny.fs_section);
    }

    #[test]
    fn density_moves_the_spacing_and_leaves_the_type_alone() {
        let plain = Metrics::new(1.0);
        let compact = Metrics::resolve(
            1.0,
            true,
            &Appearance {
                density: "compact".into(),
                ..Appearance::default()
            },
            false,
        );
        let roomy = Metrics::resolve(
            1.0,
            true,
            &Appearance {
                density: "roomy".into(),
                ..Appearance::default()
            },
            false,
        );

        assert!(compact.pad < plain.pad && plain.pad < roomy.pad);
        assert!(compact.event_row_h < plain.event_row_h);
        assert!(roomy.section_gap > plain.section_gap);
        assert_eq!(compact.fs_row, plain.fs_row, "type is not spacing");
        assert_eq!(roomy.fs_row, plain.fs_row);
    }

    #[test]
    fn per_calendar_colours_are_found_by_id_or_by_name() {
        let mut custom = Appearance::default();
        custom
            .calendar_colors
            .insert("work@example.com".into(), "#FF8800".into());
        custom
            .calendar_colors
            .insert("Family".into(), "#00AA55".into());
        custom
            .calendar_colors
            .insert("Broken".into(), "nope".into());

        assert_eq!(
            custom.calendar_color("work@example.com", "Work"),
            Some(0xFF_8800)
        );
        assert_eq!(custom.calendar_color("abc123", "Family"), Some(0x00_AA55));
        assert_eq!(custom.calendar_color("other", "Other"), None);
        assert_eq!(custom.calendar_color("x", "Broken"), None, "malformed");
    }

    #[test]
    fn unknown_words_fall_back_to_the_system_value() {
        assert_eq!(Surface::parse("nonsense"), Surface::System);
        assert_eq!(Surface::parse("  FLAT  "), Surface::Flat);
        assert_eq!(Density::parse("nonsense"), Density::System);
        assert_eq!(Density::parse("Compact"), Density::Compact);
        assert_eq!(FontWeight::parse("nonsense"), FontWeight::DEFAULT);
        assert_eq!(FontWeight::parse("bold"), FontWeight(700));

        let custom = Appearance {
            font_family: "system".into(),
            font_size_offset: f32::NAN,
            ..Appearance::default()
        };
        assert_eq!(custom.font_family(), None);
        assert_eq!(custom.font_size_offset(), 0.0, "NaN cannot reach a layout");
        assert_eq!(
            Appearance {
                font_size_offset: 99.0,
                ..Appearance::default()
            }
            .font_size_offset(),
            8.0,
            "clamped rather than rejected"
        );
    }
}
