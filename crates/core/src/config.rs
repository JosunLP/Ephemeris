// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! Settings, stored as `config.json` in the data directory.
//!
//! Read at start-up and written back whenever the window moves or is resized.
//! Unknown or missing fields fall back to their defaults, so a hand edited
//! file cannot bring the widget down.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
        let Ok(raw) = std::fs::read_to_string(&path) else {
            // No file yet, which is the normal case at first start.
            return (Self::default(), None);
        };
        // Many Windows editors — Windows PowerShell with `-Encoding UTF8`
        // among them — put a byte order mark in front of the file. JSON has no
        // concept of a BOM, and without trimming it the parse fails on an
        // invisible first character, with an error message nobody can act on.
        let raw = raw.trim_start_matches('\u{feff}');

        match serde_json::from_str::<Config>(raw) {
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
        self.opacity = self.opacity.clamp(0.15, 1.0);
        self.scale = self.scale.clamp(0.6, 3.0);
        self.sync_minutes = self.sync_minutes.clamp(1, 24 * 60);
        self.undo_seconds = self.undo_seconds.min(30);
        self.peek_seconds = self.peek_seconds.clamp(1, 60);
        self
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
