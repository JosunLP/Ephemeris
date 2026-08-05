//! Konfiguration in `%APPDATA%\TPMPlaner\config.json`.
//!
//! Wird beim Start gelesen und bei Positionsaenderungen zurueckgeschrieben.
//! Unbekannte/fehlende Felder fallen auf die Defaults zurueck, damit ein
//! haendisch editiertes File das Widget nicht lahmlegt.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Fensterposition in physischen Pixeln. `None` = beim ersten Start
    /// oben rechts auf dem Primaermonitor platzieren.
    pub x: Option<i32>,
    pub y: Option<i32>,
    /// Groesse in DIPs (geraeteunabhaengige Pixel bei 96 dpi).
    pub width: f32,
    pub height: f32,

    /// Sync-Intervall in Minuten.
    pub sync_minutes: u32,

    /// Leere Liste = alle Kalender bzw. alle Aufgabenlisten des Kontos.
    pub calendar_ids: Vec<String>,
    pub tasklist_ids: Vec<String>,

    /// Aufgaben ohne Faelligkeitsdatum ebenfalls anzeigen.
    pub show_undated_tasks: bool,
    /// Bereits vergangene Termine des Tages weiterhin (gedimmt) anzeigen.
    pub show_past_events: bool,
    /// Termine, die man selbst abgelehnt hat, ausblenden.
    pub hide_declined: bool,

    /// Deckkraft des Panels (0.0 - 1.0).
    pub opacity: f32,
    /// `"acrylic"` = dokumentierte DWM-Systembackdrop (Win11 22621+),
    /// alles andere = nur der eigene Verlauf.
    pub backdrop: String,
    /// Schriftgroessen-/Layoutskalierung zusaetzlich zur Monitor-DPI.
    pub scale: f32,

    /// `"system"` folgt der Windows-Anzeigesprache, sonst ein BCP-47-Tag wie
    /// `"en-US"`, `"fr-FR"` oder `"ar-SA"`. Datum und Uhrzeit richten sich
    /// immer nach dem gewaehlten Gebietsschema, auch wenn dafuer kein
    /// Textkatalog existiert.
    pub language: String,
    /// `"system"` folgt dem Windows-App-Design, sonst `"dark"` / `"light"`.
    pub theme: String,
    /// `"system"` uebernimmt die Windows-Akzentfarbe, sonst `"#RRGGBB"`.
    pub accent: String,
    /// Bedenkzeit in Sekunden, bevor ein Abhaken an Google gesendet wird.
    /// `0` schaltet die Rueckgaengig-Moeglichkeit ab.
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
            calendar_ids: Vec::new(),
            tasklist_ids: Vec::new(),
            show_undated_tasks: false,
            show_past_events: true,
            hide_declined: true,
            opacity: 0.82,
            backdrop: "acrylic".into(),
            scale: 1.0,
            language: "system".into(),
            theme: "system".into(),
            accent: "system".into(),
            undo_seconds: 4,
        }
    }
}

impl Config {
    /// Laedt die Konfiguration und meldet einen Syntaxfehler mit zurueck.
    ///
    /// Frueher fiel eine kaputte Datei stillschweigend auf die Vorgaben
    /// zurueck: der Benutzer speicherte, nichts passierte, und es gab keinen
    /// Hinweis warum. Der Fehlertext landet jetzt in der Statuszeile.
    pub fn load() -> (Self, Option<String>) {
        let path = config_path();
        let Ok(raw) = std::fs::read_to_string(&path) else {
            // Noch keine Datei — das ist der Normalfall beim ersten Start.
            return (Self::default(), None);
        };
        // Viele Windows-Editoren (u. a. Windows PowerShell mit `-Encoding
        // UTF8`) stellen der Datei eine Byte-Order-Mark voran. JSON kennt kein
        // BOM, und ohne dieses Abschneiden scheitert das Parsen an einem
        // unsichtbaren ersten Zeichen — mit einer Fehlermeldung, die niemand
        // deuten kann.
        let raw = raw.trim_start_matches('\u{feff}');

        match serde_json::from_str::<Config>(raw) {
            Ok(cfg) => (cfg.sanitized(), None),
            Err(e) => (
                Self::default(),
                Some(format!(
                    "config.json Zeile {}: {} — es gelten die Vorgabewerte.",
                    e.line(),
                    e
                )),
            ),
        }
    }

    /// Grenzen erzwingen, damit ein Tippfehler im JSON nicht zu einem
    /// 0x0-Fenster oder einem Sync-Sturm fuehrt.
    fn sanitized(mut self) -> Self {
        self.width = self.width.clamp(240.0, 1200.0);
        self.height = self.height.clamp(200.0, 2000.0);
        self.opacity = self.opacity.clamp(0.15, 1.0);
        self.scale = self.scale.clamp(0.6, 3.0);
        self.sync_minutes = self.sync_minutes.clamp(1, 24 * 60);
        self.undo_seconds = self.undo_seconds.min(30);
        self
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

/// `%APPDATA%\TPMPlaner`, mit Fallback ins Arbeitsverzeichnis.
pub fn data_dir() -> PathBuf {
    match std::env::var_os("APPDATA") {
        Some(v) => PathBuf::from(v).join("TPMPlaner"),
        None => PathBuf::from("."),
    }
}

pub fn config_path() -> PathBuf {
    data_dir().join("config.json")
}

/// Von der Google Cloud Console heruntergeladene OAuth-Client-Datei
/// (Anwendungstyp "Desktop-App").
pub fn client_secret_path() -> PathBuf {
    data_dir().join("client_secret.json")
}

/// DPAPI-verschluesselter Refresh-Token.
pub fn token_path() -> PathBuf {
    data_dir().join("token.bin")
}

/// Letzter erfolgreicher Sync-Stand, damit beim Start sofort etwas dasteht.
pub fn cache_path() -> PathBuf {
    data_dir().join("cache.json")
}
