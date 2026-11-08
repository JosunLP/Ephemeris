// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Ephemeris contributors
//! Settings, stored as `config.json` in the data directory.
//!
//! Read at start-up and written back whenever the window moves or is resized.
//! Unknown or missing fields fall back to their defaults, so a hand edited
//! file cannot bring the widget down.

use crate::theme::Appearance;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Reads a settings file, minus the byte order mark several Windows editors
/// add.
///
/// Windows PowerShell with `-Encoding UTF8` is one of them. JSON has no concept
/// of a BOM, and without trimming it the parse fails on an invisible first
/// character, with an error message nobody can act on. The settings and the
/// theme files both come through here, so that tolerance cannot drift between
/// them.
///
/// `None` means there is no file — the ordinary case at first start, and for a
/// theme that was never written.
fn read_json_text(path: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(path).ok()?;
    Some(raw.trim_start_matches('\u{feff}').to_string())
}

/// How `appearance` is written in `config.json`.
///
/// Two shapes, because a growing pile of top-level keys is not a theme. A
/// whole set of choices belongs in one file that can be shared, and a name is
/// what points at it:
///
/// ```jsonc
/// "appearance": "midnight"                       // <data dir>/midnight.theme.json
/// "appearance": { "surface": "flat", ... }       // written out in place
/// "appearance": "system"                         // nothing customised
/// ```
///
/// Both forms round-trip: the settings file is written back whenever the
/// window moves, and a named theme must not turn into its contents behind the
/// user's back.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum AppearanceRef {
    /// `"system"`, or the name of a theme file next to `config.json`.
    Named(String),
    /// Written out in `config.json` itself.
    Inline(Box<Appearance>),
}

/// Deserialised by hand rather than with `#[serde(untagged)]`.
///
/// Untagged buffers the input into serde's own value type and replays it into
/// each variant in turn. Two things follow from that, and both were visible in
/// the log. The replayed input carries no position, so *every* complaint about
/// `appearance` came out as "line 0" — which contradicts the one thing the
/// configuration errors promise, that they name the line to go and fix. And a
/// failure is reported as "data did not match any variant", hiding whichever
/// error the map arm actually produced.
///
/// Dispatching on the input kind keeps both: the string arm and the map arm
/// each hand the real deserialiser through, so a bad value inside an inline
/// block is reported where it is written.
impl<'de> Deserialize<'de> for AppearanceRef {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        use serde::de::{MapAccess, Visitor};

        struct Pick;

        impl<'de> Visitor<'de> for Pick {
            type Value = AppearanceRef;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a theme name such as \"midnight\", or a block of appearance settings")
            }

            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<AppearanceRef, E> {
                Ok(AppearanceRef::Named(v.to_owned()))
            }

            fn visit_map<M: MapAccess<'de>>(self, map: M) -> Result<AppearanceRef, M::Error> {
                let custom =
                    Appearance::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;
                Ok(AppearanceRef::Inline(Box::new(custom)))
            }
        }

        de.deserialize_any(Pick)
    }
}

impl Default for AppearanceRef {
    fn default() -> Self {
        AppearanceRef::Named("system".into())
    }
}

impl AppearanceRef {
    /// The customisation to apply, reading the theme file if one was named.
    ///
    /// Returns the notes alongside it, in the same spirit as
    /// [`crate::theme::Palette::customize`]: a theme file that is missing or
    /// malformed falls back to the system look, and says so rather than
    /// looking like a setting that had no effect.
    pub fn resolve(&self) -> (Appearance, Vec<String>) {
        if let AppearanceRef::Inline(custom) = self {
            return ((**custom).clone(), Vec::new());
        }
        // No name, or a name meaning "nothing customised".
        let Some(name) = self.named() else {
            return (Appearance::default(), Vec::new());
        };
        let Some(path) = theme_path(name) else {
            return (
                Appearance::default(),
                vec![format!(
                    "\"{name}\" is not a usable theme name: a name labels a file in the data \
                     folder, so only letters, digits, spaces, hyphens and underscores are \
                     allowed. The system look is in use."
                )],
            );
        };
        let Some(raw) = read_json_text(&path) else {
            return (
                Appearance::default(),
                vec![format!(
                    "No theme file at {} — the system look is in use.",
                    path.display()
                )],
            );
        };
        match serde_json::from_str::<Appearance>(&raw) {
            Ok(custom) => (custom, Vec::new()),
            Err(e) => (
                Appearance::default(),
                vec![format!(
                    "{} line {}: {e} — the system look is in use.",
                    path.display(),
                    e.line()
                )],
            ),
        }
    }

    /// The theme file this points at, if it points at one.
    ///
    /// `None` for an inline block and for the names that mean "nothing
    /// customised": there is no separate file to read — nor, which is the other
    /// caller, to watch for changes. A named theme is a file of its own, and
    /// editing it is the whole point of having one, so the widget has to notice
    /// when it moves.
    pub fn file(&self) -> Option<PathBuf> {
        self.named().and_then(theme_path)
    }

    /// The trimmed name, if this is a name that asks for a theme file at all.
    ///
    /// `None` for an inline block and for the names that mean "nothing
    /// customised". Says nothing about whether the name is *usable* — that is
    /// [`theme_path`]'s question, and the two are kept apart so `resolve` can
    /// tell a rejected name from a name that never wanted a file.
    fn named(&self) -> Option<&str> {
        let AppearanceRef::Named(name) = self else {
            return None;
        };
        let name = name.trim();
        (!name.is_empty()
            && !name.eq_ignore_ascii_case("system")
            && !name.eq_ignore_ascii_case("none"))
        .then_some(name)
    }
}

/// Which layout the panel shows.
///
/// Written in `config.json` as `"view": "day"`, `"week"` or `"workweek"`. An
/// unknown value falls back to the day view rather than taking the whole file
/// down with it: the view is a matter of taste, and a typo in it is no reason
/// to lose every other setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum View {
    /// Today's agenda, the highlighted event and the tasks — the widget as it
    /// has always been.
    #[default]
    Day,
    /// Monday to Sunday side by side, on a time grid.
    Week,
    /// Monday to Friday side by side. Shows the coming week from Saturday
    /// on, because the weekend's empty grid of a week that is over is the one
    /// thing nobody wants to look at on a Sunday evening.
    WorkWeek,
}

impl View {
    pub fn parse(raw: &str) -> View {
        match raw.trim().to_ascii_lowercase().as_str() {
            "week" => View::Week,
            "workweek" | "work-week" | "work_week" | "work week" => View::WorkWeek,
            _ => View::Day,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            View::Day => "day",
            View::Week => "week",
            View::WorkWeek => "workweek",
        }
    }

    /// How many days the week grid shows, or `None` for the day view.
    pub fn week_days(self) -> Option<u32> {
        match self {
            View::Day => None,
            View::Week => Some(7),
            View::WorkWeek => Some(5),
        }
    }

    pub fn is_week(self) -> bool {
        self.week_days().is_some()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Window position in physical pixels. `None` means "place it top right
    /// on the primary monitor at first start".
    pub x: Option<i32>,
    pub y: Option<i32>,
    /// Size in device independent pixels, at 96 dpi.
    pub width: f32,
    pub height: f32,
    /// Pin the widget where it is: no dragging, no resizing, no reset from the
    /// menu.
    ///
    /// It stays fully usable — scrolling, ticking tasks off and opening events
    /// all still work. The widget sits below every other window and is dragged
    /// by its empty space rather than by a title bar, so reaching past it for
    /// something on the desktop is easy to do and moves it by accident. This
    /// is the setting that stops that, and the right-click menu toggles it.
    pub locked: bool,

    /// `"day"` (the default), `"week"` or `"workweek"`. See [`View`].
    pub view: String,
    /// Size of the week view, in the same units as `width` and `height`.
    ///
    /// A size of its own because the two views want very different windows:
    /// a day is a narrow column, a week is a wide grid. One shared size would
    /// mean resizing by hand at every switch, or a week squeezed into 380
    /// pixels.
    pub week_width: f32,
    pub week_height: f32,
    /// The calendars the week view shows. Empty means the same calendars as
    /// the day view.
    ///
    /// A calendar listed here is fetched even when `calendar_ids` leaves it
    /// out, and then appears in the week view only — so a timetable can live
    /// in the week grid without crowding today's agenda.
    pub week_calendar_ids: Vec<String>,

    /// Sync interval in minutes.
    pub sync_minutes: u32,

    /// Configured accounts. An empty list is treated as a single Google
    /// account, which is what every installation before multi-account support
    /// had — nobody has to edit a file to keep working.
    #[serde(default)]
    pub accounts: Vec<crate::provider::AccountConfig>,

    /// An empty list means every calendar or task list the account offers.
    pub calendar_ids: Vec<String>,
    pub tasklist_ids: Vec<String>,

    /// Show tasks that have no due date as well.
    pub show_undated_tasks: bool,
    /// Keep events that already ended visible, dimmed.
    pub show_past_events: bool,
    /// Hide events you declined yourself.
    pub hide_declined: bool,

    /// Panel opacity, 0.0 to 1.0.
    pub opacity: f32,
    /// `"none"` (the default) draws the widget's own gradient, shadow and
    /// rounded corners. `"acrylic"` uses the Windows system backdrop; the
    /// shadow margin then has to go and the compositor rounds the corners,
    /// because a system backdrop fills the whole window rectangle and would
    /// otherwise put a square box around the panel.
    pub backdrop: String,
    /// Extra layout and font scaling on top of the monitor's own DPI.
    pub scale: f32,

    /// `"system"` follows the display language, otherwise a BCP-47 tag such
    /// as `"en-US"`, `"fr-FR"` or `"ar-SA"`. Dates and times always follow the
    /// chosen locale, even where no text catalogue exists for it.
    pub language: String,
    /// `"system"` follows the system appearance, otherwise `"dark"`,
    /// `"light"` or `"contrast"`.
    pub theme: String,
    /// `"system"` adopts the system accent colour, otherwise `"#RRGGBB"`.
    pub accent: String,
    /// Customisation on top of the system-derived look: colours beyond the
    /// accent, typography, surface style, density and per-calendar colours.
    ///
    /// Either written out here, or the name of a theme file in the data
    /// directory — see [`AppearanceRef`].
    #[serde(default)]
    pub appearance: AppearanceRef,
    /// Shortcut that brings the widget to the front for a moment.
    /// Modifiers plus a key, for example `"Ctrl+Alt+K"` or `"Ctrl+Shift+P"`.
    /// Avoid `Win+...` where possible — Windows 11 has reserved a great deal
    /// of it (Win+Alt+K is the microphone mute, for instance). An empty value
    /// turns the feature off.
    pub peek_hotkey: String,
    /// How long the widget stays in front after the shortcut.
    pub peek_seconds: u32,
    /// Grace period in seconds before a completed task is sent to the
    /// service. `0` turns the undo window off.
    pub undo_seconds: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            x: None,
            y: None,
            width: 380.0,
            height: 620.0,
            locked: false,
            view: "day".into(),
            week_width: 760.0,
            week_height: 560.0,
            week_calendar_ids: Vec::new(),
            sync_minutes: 30,
            accounts: Vec::new(),
            calendar_ids: Vec::new(),
            tasklist_ids: Vec::new(),
            show_undated_tasks: false,
            show_past_events: true,
            hide_declined: true,
            opacity: 0.82,
            backdrop: "none".into(),
            scale: 1.0,
            language: "system".into(),
            theme: "system".into(),
            accent: "system".into(),
            appearance: AppearanceRef::default(),
            peek_hotkey: "Ctrl+Alt+Shift+K".into(),
            peek_seconds: 5,
            undo_seconds: 4,
        }
    }
}

impl Config {
    /// Loads the settings and reports a syntax error alongside them.
    ///
    /// A broken file used to fall back to the defaults in silence: you saved,
    /// nothing happened, and there was no hint why. The error text now reaches
    /// the status line.
    pub fn load() -> (Self, Option<String>) {
        let path = config_path();
        let Some(raw) = read_json_text(&path) else {
            // No file yet, which is the normal case at first start.
            return (Self::default(), None);
        };

        match serde_json::from_str::<Config>(&raw) {
            Ok(cfg) => (cfg.sanitized(), None),
            Err(e) => (
                Self::default(),
                Some(format!(
                    "config.json line {}: {} — the defaults are in use.",
                    e.line(),
                    e
                )),
            ),
        }
    }

    /// Clamps the values, so a typo in the file cannot produce a zero sized
    /// window or a storm of requests.
    fn sanitized(mut self) -> Self {
        self.width = self.width.clamp(240.0, 1200.0);
        self.height = self.height.clamp(200.0, 2000.0);
        self.week_width = self.week_width.clamp(420.0, 2400.0);
        self.week_height = self.week_height.clamp(280.0, 2000.0);
        self.opacity = self.opacity.clamp(0.15, 1.0);
        self.scale = self.scale.clamp(0.6, 3.0);
        self.sync_minutes = self.sync_minutes.clamp(1, 24 * 60);
        self.undo_seconds = self.undo_seconds.min(30);
        self.peek_seconds = self.peek_seconds.clamp(1, 60);
        self
    }

    /// The view the settings ask for.
    pub fn view(&self) -> View {
        View::parse(&self.view)
    }

    /// Switches the view. The window size goes with it — see
    /// [`Config::size`].
    pub fn set_view(&mut self, view: View) {
        self.view = view.as_str().to_owned();
    }

    /// The size of the glass body for the current view.
    pub fn size(&self) -> (f32, f32) {
        if self.view().is_week() {
            (self.week_width, self.week_height)
        } else {
            (self.width, self.height)
        }
    }

    /// Stores a size for the current view, leaving the other one alone.
    pub fn set_size(&mut self, width: f32, height: f32) {
        if self.view().is_week() {
            self.week_width = width;
            self.week_height = height;
        } else {
            self.width = width;
            self.height = height;
        }
    }

    /// The calendars to ask the providers for.
    ///
    /// The day selection, plus the week-only calendars while a week view is
    /// up. Empty still means "every calendar": a selection that is empty on
    /// the day side already includes whatever the week side wants.
    pub fn fetch_calendar_ids(&self) -> Vec<String> {
        let mut ids = self.calendar_ids.clone();
        if ids.is_empty() || !self.view().is_week() {
            return ids;
        }
        for id in &self.week_calendar_ids {
            if !ids.contains(id) {
                ids.push(id.clone());
            }
        }
        ids
    }

    /// Does a calendar belong in the day view?
    ///
    /// Only the calendars fetched for the week view alone are turned away.
    /// Everything else was asked for by the day selection — or by a provider
    /// that hands out calendar ids of its own, which is no reason to hide what
    /// it returned.
    pub fn shows_in_day(&self, calendar_id: &str) -> bool {
        let week_only = !self.calendar_ids.is_empty()
            && self.week_calendar_ids.iter().any(|id| id == calendar_id)
            && !self.calendar_ids.iter().any(|id| id == calendar_id);
        !week_only
    }

    /// Does a calendar belong in the week view?
    pub fn shows_in_week(&self, calendar_id: &str) -> bool {
        self.week_calendar_ids.is_empty()
            || self.week_calendar_ids.iter().any(|id| id == calendar_id)
    }

    /// Is this calendar ticked in the week view's menu?
    ///
    /// An empty week selection follows the day selection, so the ticks have
    /// to say that rather than claiming every calendar.
    pub fn week_calendar_checked(&self, calendar_id: &str) -> bool {
        if self.week_calendar_ids.is_empty() {
            self.calendar_ids.is_empty() || self.calendar_ids.iter().any(|id| id == calendar_id)
        } else {
            self.week_calendar_ids.iter().any(|id| id == calendar_id)
        }
    }

    /// Ticks a calendar on or off for the week view.
    ///
    /// `all` is every calendar the accounts offer, in menu order. The same
    /// rules as the day selection: starting from "follow the day view" the
    /// selection is written out first, the last calendar cannot be taken
    /// away — an empty list would mean all of them again — and a selection
    /// that ends up naming every calendar goes back to empty, so a calendar
    /// added later comes along.
    pub fn toggle_week_calendar(&mut self, all: &[String], id: &str) {
        if !all.iter().any(|a| a == id) {
            return;
        }
        if self.week_calendar_ids.is_empty() {
            self.week_calendar_ids = all
                .iter()
                .filter(|a| self.week_calendar_checked(a))
                .cloned()
                .collect();
        }
        // Naming every calendar is only the same as naming none while the day
        // view shows every calendar too; otherwise empty would mean "follow
        // the day" and drop the calendars the week added.
        let day_shows_all = self.calendar_ids.is_empty();
        let selected = &mut self.week_calendar_ids;
        if selected.iter().any(|s| s == id) {
            if selected.len() > 1 {
                selected.retain(|s| s != id);
            }
        } else {
            selected.push(id.to_owned());
        }
        if day_shows_all && all.iter().all(|a| selected.contains(a)) {
            selected.clear();
        }
    }

    /// The accounts to actually query, with the legacy single-Google setup
    /// filled in.
    pub fn effective_accounts(&self) -> Vec<crate::provider::AccountConfig> {
        use crate::provider::{AccountConfig, Kind};
        if self.accounts.is_empty() {
            return vec![AccountConfig {
                kind: Kind::Google,
                id: "google".into(),
                label: "Google".into(),
                enabled: true,
            }];
        }
        self.accounts
            .iter()
            .filter(|a| a.enabled)
            .cloned()
            .collect()
    }

    pub fn save(&self) {
        let path = config_path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, json);
        }
    }
}

/// Where settings, credentials, cache and log live.
///
/// The location differs per platform, so the host decides; see
/// [`crate::host::Host::data_dir`].
pub fn data_dir() -> PathBuf {
    crate::host::host().data_dir()
}

pub fn config_path() -> PathBuf {
    data_dir().join("config.json")
}

/// A named theme, next to the settings file it belongs to.
///
/// The suffix is fixed rather than part of the name, and the name itself has to
/// be letters, digits, spaces, hyphens and underscores throughout. A theme name
/// is a label for a file in the data directory, not a way to reach one
/// somewhere else — and a whitelist is far easier to be sure about than a list
/// of the separators and traversal sequences to strip.
///
/// `None` when the name does not survive that check, rather than the sanitised
/// remainder. Stripping quietly is what makes a whitelist go wrong: `"../"`
/// lost every character and landed on a hidden `.theme.json`, and `"mid/night"`
/// and `"midnight"` became the same file — two names for one theme, with no
/// hint that the first was not the one on disk. A name that has to be repaired
/// to be safe is a name the user should hear about.
pub fn theme_path(name: &str) -> Option<PathBuf> {
    let name = name.trim();
    let usable = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | ' '));
    usable.then(|| data_dir().join(format!("{name}.theme.json")))
}

/// The OAuth client file downloaded from the Google Cloud Console
/// (application type "desktop app").
pub fn client_secret_path() -> PathBuf {
    data_dir().join("client_secret.json")
}

/// The encrypted refresh token.
pub fn token_path() -> PathBuf {
    data_dir().join("token.bin")
}

/// Last successful sync, so something is on screen immediately at start-up.
pub fn cache_path() -> PathBuf {
    data_dir().join("cache.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The settings file is written back whenever the window moves, so
    /// anything that survives a load has to survive a save as well. An
    /// untagged enum is exactly where that goes wrong quietly: a named theme
    /// turning into its own contents would be a one-way door.
    #[test]
    fn an_appearance_survives_the_round_trip_in_both_shapes() {
        for written in [
            r#"{"appearance":"midnight"}"#,
            r#"{"appearance":"system"}"#,
            r#"{"appearance":{"surface":"flat","density":"roomy"}}"#,
        ] {
            let cfg: Config = serde_json::from_str(written).expect(written);
            let again = serde_json::to_string(&cfg).expect("serialise");
            let back: Config = serde_json::from_str(&again).expect("re-read");
            assert_eq!(cfg.appearance, back.appearance, "{written}");
        }

        // A file written before this setting existed keeps working, and one
        // with the key left out is not customised.
        let cfg: Config = serde_json::from_str("{}").expect("empty object");
        assert_eq!(cfg.appearance, AppearanceRef::default());
        let (custom, notes) = cfg.appearance.resolve();
        assert_eq!(custom, crate::theme::Appearance::default());
        assert!(notes.is_empty());
    }

    #[test]
    fn an_inline_appearance_reaches_the_palette() {
        let cfg: Config = serde_json::from_str(
            r##"{"appearance":{"surface":"borderless","font_size_offset":2.0,
                 "colors":{"now":"#FF8800"},
                 "calendar_colors":{"work":"#123456"}}}"##,
        )
        .expect("parse");

        let (custom, notes) = cfg.appearance.resolve();
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(custom.surface(), crate::theme::Surface::Borderless);
        assert_eq!(custom.font_size_offset(), 2.0);
        assert_eq!(custom.colors.now, "#FF8800");
        assert_eq!(custom.calendar_color("work", "Work"), Some(0x12_3456));
        // Anything not mentioned stays with the system.
        assert_eq!(custom.density(), crate::theme::Density::System);
        assert_eq!(custom.colors.panel, "system");
    }

    /// The lock is written back by the menu, so it has to survive the file it
    /// is written to — and a settings file from before it existed must not
    /// come back locked.
    #[test]
    fn the_lock_defaults_to_free_and_survives_a_save() {
        let older: Config = serde_json::from_str(r#"{"width":400.0}"#).expect("parse");
        assert!(!older.locked, "an existing installation must not seize up");

        let cfg = Config {
            locked: true,
            ..Config::default()
        };
        let written = serde_json::to_string(&cfg).expect("serialise");
        let back: Config = serde_json::from_str(&written).expect("re-read");
        assert!(back.locked);
        // And it is not something `sanitized` can quietly undo.
        assert!(back.sanitized().locked);
    }

    /// The view is a matter of taste, so a typo in it must not cost every
    /// other setting — and a file from before it existed opens on the day.
    #[test]
    fn an_unknown_view_falls_back_to_the_day() {
        let older: Config = serde_json::from_str(r#"{"width":400.0}"#).expect("parse");
        assert_eq!(older.view(), View::Day);
        for (raw, view) in [
            ("week", View::Week),
            (" Week ", View::Week),
            ("workweek", View::WorkWeek),
            ("work-week", View::WorkWeek),
            ("weeek", View::Day),
            ("", View::Day),
        ] {
            assert_eq!(View::parse(raw), view, "{raw:?}");
        }
        for view in [View::Day, View::Week, View::WorkWeek] {
            assert_eq!(View::parse(view.as_str()), view);
        }
    }

    /// Each view keeps its own window, so switching back and forth does not
    /// leave the day column as wide as the week grid.
    #[test]
    fn each_view_keeps_its_own_size() {
        let mut cfg = Config::default();
        cfg.set_size(400.0, 600.0);
        cfg.set_view(View::Week);
        assert_eq!(cfg.size(), (760.0, 560.0));
        cfg.set_size(900.0, 500.0);
        cfg.set_view(View::Day);
        assert_eq!(cfg.size(), (400.0, 600.0));
        cfg.set_view(View::WorkWeek);
        assert_eq!(cfg.size(), (900.0, 500.0));

        let written = serde_json::to_string(&cfg).expect("serialise");
        let back: Config = serde_json::from_str(&written).expect("re-read");
        assert_eq!(back.view(), View::WorkWeek);
        assert_eq!(back.size(), (900.0, 500.0));
    }

    /// A timetable calendar can be in the week grid without being in today's
    /// agenda: it is fetched for the week and turned away from the day.
    #[test]
    fn a_week_only_calendar_is_fetched_but_kept_out_of_the_day() {
        let mut cfg = Config {
            calendar_ids: vec!["work".into()],
            week_calendar_ids: vec!["timetable".into()],
            ..Config::default()
        };
        // The day view does not fetch what only the week view wants.
        assert_eq!(cfg.fetch_calendar_ids(), ["work"]);

        cfg.set_view(View::Week);
        assert_eq!(cfg.fetch_calendar_ids(), ["work", "timetable"]);
        assert!(cfg.shows_in_week("timetable"));
        assert!(!cfg.shows_in_week("work"));
        assert!(!cfg.shows_in_day("timetable"));
        assert!(cfg.shows_in_day("work"));
        // An id nobody configured is not hidden on suspicion.
        assert!(cfg.shows_in_day("something-else"));

        // With every calendar selected for the day, nothing is week-only.
        cfg.calendar_ids.clear();
        assert!(cfg.fetch_calendar_ids().is_empty());
        assert!(cfg.shows_in_day("timetable"));
    }

    #[test]
    fn the_week_selection_follows_the_day_until_it_is_changed() {
        let all: Vec<String> = ["a", "b", "c"].map(String::from).to_vec();
        let mut cfg = Config {
            calendar_ids: vec!["a".into(), "b".into()],
            ..Config::default()
        };
        // Empty follows the day selection, and the ticks say so.
        assert!(cfg.week_calendar_checked("a"));
        assert!(!cfg.week_calendar_checked("c"));

        // Taking one away writes out what was showing, minus that one.
        cfg.toggle_week_calendar(&all, "a");
        assert_eq!(cfg.week_calendar_ids, ["b"]);
        // The last one stays: empty would mean "follow the day" again.
        cfg.toggle_week_calendar(&all, "b");
        assert_eq!(cfg.week_calendar_ids, ["b"]);
        // Adding a calendar the day view does not show is the whole point.
        cfg.toggle_week_calendar(&all, "c");
        assert_eq!(cfg.week_calendar_ids, ["b", "c"]);
        // Every calendar ticked while the day view shows only some: kept as
        // written, because empty would fall back to the day's two and lose
        // "c" from the week.
        cfg.toggle_week_calendar(&all, "a");
        assert_eq!(cfg.week_calendar_ids, ["b", "c", "a"]);
        assert!(cfg.week_calendar_checked("c"));
        // An id that is not on offer changes nothing.
        cfg.toggle_week_calendar(&all, "zzz");
        assert_eq!(cfg.week_calendar_ids, ["b", "c", "a"]);

        // With every calendar on the day side, naming them all is the same
        // as naming none, so a calendar added later comes along.
        let mut cfg = Config::default();
        cfg.toggle_week_calendar(&all, "a");
        assert_eq!(cfg.week_calendar_ids, ["b", "c"]);
        cfg.toggle_week_calendar(&all, "a");
        assert!(cfg.week_calendar_ids.is_empty());
    }

    /// A theme file that is not there must not look like a setting that had no
    /// effect.
    #[test]
    fn a_missing_theme_file_falls_back_and_says_so() {
        let named = AppearanceRef::Named("no-such-theme".into());
        let (custom, notes) = named.resolve();
        assert_eq!(custom, crate::theme::Appearance::default());
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(notes[0].contains("no-such-theme.theme.json"), "{notes:?}");
    }

    /// A theme name is a file in the data directory, not a path.
    #[test]
    fn a_theme_name_cannot_escape_the_data_directory() {
        for hostile in [
            "../../etc/passwd",
            "..\\..\\windows\\win.ini",
            "/etc/shadow",
            "..",
            // Sanitising used to turn this into "midnight", silently pointing
            // two different names at one file.
            "mid/night",
        ] {
            assert_eq!(theme_path(hostile), None, "{hostile}");
        }

        // An ordinary name is left exactly as written.
        assert_eq!(
            theme_path("midnight glass_2")
                .expect("a plain name is usable")
                .file_name()
                .unwrap(),
            "midnight glass_2.theme.json"
        );
        assert_eq!(
            theme_path("midnight").expect("usable").parent(),
            Some(data_dir().as_path())
        );
    }

    /// A rejected name must not read as "you asked for the system look".
    #[test]
    fn an_unusable_theme_name_is_reported_rather_than_repaired() {
        let (custom, notes) = AppearanceRef::Named("mid/night".into()).resolve();
        assert_eq!(custom, crate::theme::Appearance::default());
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(notes[0].contains("mid/night"), "{notes:?}");
        // And it names no file, so nothing is watched for changes either.
        assert_eq!(AppearanceRef::Named("mid/night".into()).file(), None);
    }

    /// An error inside `appearance` has to name the line it is on.
    ///
    /// `#[serde(untagged)]` reported every one of them as line 0 and replaced
    /// the real complaint with "data did not match any variant", which is the
    /// one thing the configuration errors promise not to do.
    #[test]
    fn a_broken_appearance_block_reports_its_line() {
        let raw = "{\n  \"scale\": 1.0,\n  \"appearance\": {\n    \"surface\": 7\n  }\n}";
        let err = serde_json::from_str::<Config>(raw).expect_err("7 is not a surface");
        assert_eq!(err.line(), 4, "{err}");
        assert!(
            !err.to_string().contains("any variant"),
            "the real error has to survive: {err}"
        );
    }

    /// Out-of-range values are clamped rather than rejected, and that has to
    /// keep holding for the numbers this feature added.
    #[test]
    fn nonsense_numbers_are_clamped_rather_than_fatal() {
        let cfg: Config = serde_json::from_str(
            r#"{"scale":99.0,"opacity":-5.0,"appearance":{"font_size_offset":250.0}}"#,
        )
        .expect("parse");
        let cfg = cfg.sanitized();
        assert_eq!(cfg.scale, 3.0);
        assert_eq!(cfg.opacity, 0.15);
        assert_eq!(cfg.appearance.resolve().0.font_size_offset(), 8.0);
    }

    /// Only a named theme has a file, and only a file can be watched.
    ///
    /// The widget re-reads its settings when `config.json` changes; a named
    /// theme lives elsewhere, so its path has to be watched as well or editing
    /// it — the advertised way to use one — does nothing until `config.json`
    /// is written for some unrelated reason.
    #[test]
    fn a_named_theme_names_a_file_to_watch() {
        assert_eq!(
            AppearanceRef::Named("midnight".into()).file(),
            theme_path("midnight")
        );
        // Nothing customised: no second file involved.
        for none in ["system", "none", "", "  "] {
            assert_eq!(
                AppearanceRef::Named(none.into()).file(),
                None,
                "{none:?} names no file"
            );
        }
        assert_eq!(
            AppearanceRef::Inline(Box::default()).file(),
            None,
            "an inline block is already in config.json"
        );
    }
}
