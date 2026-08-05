//! Schlankes Dateiprotokoll.
//!
//! Die Statuszeile im Widget hat Platz fuer rund 56 Zeichen — fuer eine
//! Google-Fehlermeldung reicht das nicht annaehernd. Ohne Protokoll bleibt bei
//! einem Problem im Betrieb nur Raten. Deshalb landet der vollstaendige Text
//! zusaetzlich in `%APPDATA%\TPMPlaner\tpmplaner.log`, erreichbar ueber das
//! Kontextmenue.
//!
//! Bewusst ohne Log-Crate: ein `Mutex<File>` und eine Groessenrotation sind
//! alles, was hier gebraucht wird.

use crate::config;
use chrono::Local;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

/// Ab dieser Groesse wird nach `.1` rotiert. Zwei Dateien à 256 KB reichen fuer
/// mehrere Tage Betrieb und fallen auf keiner Platte auf.
const MAX_BYTES: u64 = 256 * 1024;

fn path() -> PathBuf {
    config::data_dir().join("tpmplaner.log")
}

fn lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Schreibt eine Zeile. Fehler werden geschluckt — ein nicht schreibbares
/// Protokoll darf das Widget nicht stoppen.
pub fn write(level: &str, message: &str) {
    let _guard = lock().lock();
    let path = path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }

    if std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) >= MAX_BYTES {
        let _ = std::fs::rename(&path, path.with_extension("log.1"));
    }

    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        // Einzeilig und mit fester Spaltenbreite, damit sich das Protokoll mit
        // Bordmitteln filtern laesst.
        let _ = writeln!(
            file,
            "{} {:<5} {}",
            Local::now().format("%Y-%m-%d %H:%M:%S"),
            level,
            message.replace('\n', " | ")
        );
    }
}

pub fn info(message: &str) {
    write("INFO", message);
}

pub fn warn(message: &str) {
    write("WARN", message);
}

pub fn error(message: &str) {
    write("ERROR", message);
}

pub fn file_path() -> PathBuf {
    path()
}

/// Schreibt jede Panik ins Protokoll, bevor der Prozess endet.
///
/// Das Widget laeuft ohne Konsole und mit `panic = "abort"`. Ohne diesen Haken
/// verschwindet es bei einem Fehler einfach vom Desktop — ohne Meldung, ohne
/// Spur, und niemand kann sagen warum. Der Haken laeuft noch vor dem Abbruch
/// und kostet im Normalbetrieb nichts.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "(unbekannte Ursache)".to_string());
        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "(unbekannte Stelle)".to_string());
        let thread = std::thread::current()
            .name()
            .unwrap_or("unbenannt")
            .to_string();
        error(&format!("PANIK in Thread '{thread}' bei {location}: {payload}"));
        previous(info);
    }));
}
