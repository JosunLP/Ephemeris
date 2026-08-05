// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! Farbpalette, Masse und deutsche Datumsformate.
//!
//! Optisch an die Aero-Gadgets von Vista angelehnt. Drei Zutaten machen dort
//! den Eindruck von echtem Glas aus, und alle drei sind hier nachgebaut:
//!
//! 1. **Weicher Aussenschatten** — das Panel schwebt ueber dem Hintergrund.
//! 2. **Doppelte Kante** — aussen dunkel, innen ein helles Pixel. Ohne diesen
//!    Kontrast wirkt jede Glasflaeche wie ein flacher Aufkleber.
//! 3. **Glanzbogen im oberen Bereich** — heller Verlauf, der nach unten
//!    ausblendet und die Woelbung andeutet.
//!
//! Die Palette folgt dem Windows-App-Design (hell/dunkel) und uebernimmt die
//! Systemakzentfarbe, damit sich das Widget wie ein Teil des Systems anfuehlt
//! statt wie eine Fremdanwendung mit eigenem Blau.

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

/// Lineare Mischung zweier Farbwerte, fuer Hover- und Zustandsuebergaenge.
pub fn mix(a: u32, b: u32, t: f32) -> u32 {
    let t = t.clamp(0.0, 1.0);
    let ch = |shift: u32| {
        let ca = ((a >> shift) & 0xFF) as f32;
        let cb = ((b >> shift) & 0xFF) as f32;
        ((ca + (cb - ca) * t).round() as u32) << shift
    };
    ch(16) | ch(8) | ch(0)
}

/// Gewuenschtes Erscheinungsbild aus der Konfiguration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemePref {
    System,
    Dark,
    Light,
    /// Erzwingt die Kontrastdarstellung — flach, deckend, ausschliesslich
    /// Systemfarben — auch ohne aktives Windows-Kontrastdesign. Nuetzlich
    /// fuer alle, die maximalen Kontrast wollen, ohne das gesamte System
    /// umzustellen, und um die Darstellung vorab zu pruefen.
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
}

/// Alle Farben eines Erscheinungsbilds.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    /// Kontrastdesign: Verlaeufe, Glanz und Schatten entfallen, das Panel ist
    /// deckend, alle Farben kommen vom System.
    pub high_contrast: bool,
    /// Deckend zeichnen — im Kontrastdesign und wenn der Benutzer die
    /// Transparenzeffekte abgeschaltet hat.
    pub force_opaque: bool,
    /// Bewegung erwuenscht? Folgt "Animationseffekte in Windows anzeigen".
    pub animations: bool,

    pub panel_top: u32,
    pub panel_mid: u32,
    pub panel_bottom: u32,
    /// Farbe von Glanzbogen und Innenkante — in beiden Designs Weiss.
    pub sheen: u32,
    pub sheen_gloss: f32,
    pub sheen_border: f32,
    /// Trennlinien. Im hellen Design **dunkel** statt weiss: eine weisse Linie
    /// auf hellem Glas ist unsichtbar.
    pub rule: u32,
    pub rule_alpha: f32,
    /// Hinterlegung der Zeile unter dem Mauszeiger. Ebenfalls
    /// designabhaengig — auf hellem Grund muss sie abdunkeln, nicht aufhellen.
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
    pub warn: u32,
}

impl Palette {
    /// Baut die Palette aus Systemeinstellungen und Konfiguration.
    pub fn resolve(pref: ThemePref, accent_cfg: &str, vis: SystemVisuals) -> Self {
        // Das Windows-Kontrastdesign hat Vorrang vor jeder Konfiguration: wer
        // es einschaltet, braucht es, und eine App, die sich darueber
        // hinwegsetzt, wird unbenutzbar.
        if vis.high_contrast || pref == ThemePref::Contrast {
            return Self::high_contrast(vis);
        }

        let light = match pref {
            ThemePref::Dark => false,
            ThemePref::Light => true,
            ThemePref::Contrast => unreachable!("oben abgefangen"),
            ThemePref::System => vis.light,
        };

        let mut p = if light { Self::light() } else { Self::dark() };
        p.animations = vis.animations;
        p.force_opaque = !vis.transparency;

        // Feste Farbe aus der Konfiguration schlaegt die Systemfarbe.
        let raw = parse_hex(accent_cfg).or(vis.accent);
        if let Some(raw) = raw {
            p.accent = readable_accent(raw, light);
            // Getoenter Hintergrund der Hero-Karte: die Akzentfarbe stark in
            // Richtung Panelfarbe gezogen, damit Text darauf lesbar bleibt.
            p.accent_soft = mix(p.accent, p.panel_mid, if light { 0.80 } else { 0.72 });
            p.overdue = distinct_from(p.overdue, p.accent);
        }
        p
    }

    /// Tatsaechlich zu verwendende Deckkraft.
    pub fn opacity(&self, configured: f32) -> f32 {
        if self.force_opaque {
            1.0
        } else {
            configured.clamp(0.15, 1.0)
        }
    }

    /// Palette aus den Systemfarben des Kontrastdesigns.
    ///
    /// Es gibt vier Kontrastdesigns mit voellig verschiedenen Farbwerten;
    /// deshalb wird hier nichts geraten, sondern alles ueber `GetSysColor`
    /// abgefragt. Verlaeufe, Glanz und Schatten sind abgeschaltet — sie
    /// wuerden genau den Kontrast zerstoeren, um den es geht.
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
        // Helligkeit des Fensterhintergrunds entscheidet, ob es ein helles
        // oder dunkles Kontrastdesign ist.
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
            // `COLOR_GRAYTEXT` ist im Kontrastdesign bewusst noch lesbar.
            text_dim: gray,
            text_faint: gray,

            accent: hot,
            accent_soft: window,
            overdue: distinct_from(hot, highlight),
            ok_green: hot,
            warn: hot,
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
        }
    }

    /// Helles Glas: der Verlauf laeuft nach unten **heller**, nicht dunkler —
    /// sonst sieht die Flaeche schmutzig statt wie Milchglas aus. Kanten und
    /// Glanz muessen deutlich zurueckgenommen werden, weil Weiss auf Weiss
    /// nichts hergibt; die Tiefe kommt hier fast nur aus dem Aussenschatten.
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
        }
    }
}

/// Hebt oder senkt die Helligkeit einer Farbe in ein lesbares Band.
///
/// Die Windows-Akzentfarbe darf beliebig dunkel oder grell sein — Schwarz und
/// Neongelb sind beides gueltige Einstellungen. Als Textfarbe auf dem Glas
/// waere das unbrauchbar, deshalb wird nur die Helligkeit korrigiert und der
/// Farbton unangetastet gelassen: der Benutzer erkennt seine Farbe wieder.
fn readable_accent(rgb: u32, light_theme: bool) -> u32 {
    let (h, s, l) = rgb_to_hsl(rgb);
    // Sehr blasse Akzente (fast Grau) etwas anheben, sonst geht der Zustand
    // "jetzt/aktiv" optisch unter.
    let s = s.max(0.35);
    let l = if light_theme {
        l.clamp(0.30, 0.46)
    } else {
        l.clamp(0.58, 0.76)
    };
    hsl_to_rgb(h, s, l)
}

/// Abstand zweier Farbtoene auf dem Farbkreis, 0.0 bis 0.5.
fn hue_distance(a: f32, b: f32) -> f32 {
    let d = (a - b).abs().fract();
    d.min(1.0 - d)
}

/// Haelt die Ueberfaellig-Farbe vom Akzent unterscheidbar.
///
/// Die Windows-Akzentfarbe darf rot sein — dann faerben sich "jetzt",
/// Tagesfortschritt und Jetzt-Linie im selben Rot wie die ueberfaelligen
/// Aufgaben, und die beiden Bedeutungen sind optisch nicht mehr zu trennen.
/// In dem Fall weicht die Warnfarbe auf Bernstein bzw. Magenta aus — je
/// nachdem, was weiter vom Akzent entfernt liegt.
fn distinct_from(overdue: u32, accent: u32) -> u32 {
    /// Rund 25 Grad auf dem Farbkreis.
    const MIN_GAP: f32 = 0.07;

    let (h_o, s_o, l_o) = rgb_to_hsl(overdue);
    let (h_a, _, _) = rgb_to_hsl(accent);
    if hue_distance(h_o, h_a) >= MIN_GAP {
        return overdue;
    }
    // Bernstein (~32°) und Magenta (~317°) sind beide als Warnfarbe lesbar.
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

/// `"#4FA3FF"` oder `"4FA3FF"`; alles andere (inkl. `"system"`) ergibt `None`.
fn parse_hex(s: &str) -> Option<u32> {
    let t = s.trim().trim_start_matches('#');
    (t.len() == 6)
        .then(|| u32::from_str_radix(t, 16).ok())
        .flatten()
}

/// Alle Masse in DIPs bei `scale = 1.0`.
#[derive(Debug, Clone, Copy)]
pub struct Metrics {
    /// Freier Rand im Fenster fuer den Schlagschatten. Der Glaskoerper ist um
    /// diesen Betrag von der Fensterkante eingerueckt.
    pub shadow: f32,
    pub pad: f32,
    pub corner: f32,

    pub header_h: f32,
    /// Hoehe der Tagesschiene unter dem Kopfbereich.
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

    /// `shadow = false` fuer den Acryl-Modus: dort fuellt der Desktopfenster-
    /// Manager das ganze Fensterrechteck, ein freier Rand wuerde als eckiger
    /// Kasten um das Panel sichtbar.
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
}

// Wochentags-, Monatsnamen und Zeitformate kommen aus [`crate::i18n`] —
// dort vom Betriebssystem, damit sie in jedem Gebietsschema stimmen.

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
            // Rundungsfehler von hoechstens einem Schritt je Kanal.
            for shift in [16, 8, 0] {
                let a = ((color >> shift) & 0xFF) as i32;
                let b = ((back >> shift) & 0xFF) as i32;
                assert!((a - b).abs() <= 1, "{color:06X} -> {back:06X}");
            }
        }
    }

    #[test]
    fn accent_is_forced_into_a_readable_band() {
        // Schwarz und Weiss sind gueltige Windows-Akzentfarben und beide als
        // Textfarbe unbrauchbar.
        for input in [0x000000, 0xFFFFFF, 0x0A0A64] {
            let (_, _, l_dark) = rgb_to_hsl(readable_accent(input, false));
            assert!((0.57..=0.77).contains(&l_dark), "dunkel: {input:06X}");
            let (_, _, l_light) = rgb_to_hsl(readable_accent(input, true));
            assert!((0.29..=0.47).contains(&l_light), "hell: {input:06X}");
        }
    }

    #[test]
    fn accent_keeps_the_users_hue() {
        // Ein sehr dunkles Rot bleibt rot, wird aber aufgehellt.
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
            "Warnfarbe {adjusted:06X} liegt zu nah am Akzent {red_accent:06X}"
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
}
