// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! Internationalisation.
//!
//! Two responsibilities, and keeping them apart is the whole point:
//!
//! * **Interface text** comes from compiled-in catalogues. Each language is a
//!   [`Catalog`] literal; adding one means writing a constant and listing it
//!   in [`catalog_for`].
//! * **Dates, times and reading direction** come from the operating system,
//!   through [`crate::host::LocaleBackend`]. That is the difference between a
//!   translation and an internationalised program: the system knows, for
//!   *every* locale, the field order of a date, the local month names and —
//!   above all — whether the clock counts to twelve or to twenty-four. A hard
//!   coded `{:02}:{:02}` shows a user in the United States "20:09" instead of
//!   "8:09 PM": formally right, actually wrong.
//!
//! Dates and times therefore work correctly even in languages that have no
//! catalogue at all; only the labels fall back to English.

use crate::host::locale_backend;
use chrono::{DateTime, Datelike, Local, NaiveDate, Timelike};

/// Every string in the interface.
///
/// Deliberately a struct with named fields rather than a lookup table: a
/// forgotten translation is then a compile error, not a key missing at
/// runtime.
pub struct Catalog {
    pub code: &'static str,
    /// Is the language itself written right to left?
    ///
    /// Not the same as the locale's reading direction: running under `ar-SA`
    /// mirrors the *layout*, but for want of an Arabic catalogue the text
    /// arrives in English — Latin script, left to right. The two have to be
    /// handled separately.
    pub rtl: bool,

    // Sections and lists
    pub section_events: &'static str,
    pub section_tasks: &'static str,
    pub no_events: &'static str,
    pub no_tasks: &'static str,
    pub all_day: &'static str,
    pub today: &'static str,
    pub yesterday: &'static str,
    pub tomorrow: &'static str,
    /// "{} Ueberschneidung" bzw. Mehrzahl.
    pub conflict_one: &'static str,
    pub conflict_many: &'static str,

    // Highlighted event
    pub now_label: &'static str,
    pub next_label: &'static str,
    pub running: &'static str,

    // Relative times. `{}` stands for the amount including its unit.
    pub in_pattern: &'static str,
    pub ago_pattern: &'static str,
    pub left_pattern: &'static str,
    pub just_now: &'static str,
    pub unit_min: &'static str,
    pub unit_hour: &'static str,
    pub unit_day: &'static str,

    // Tasks
    pub undo: &'static str,
    pub overdue_one: &'static str,
    pub overdue_many: &'static str,

    // Status line
    pub syncing: &'static str,
    pub updated_next: &'static str,
    pub not_synced: &'static str,
    pub setup_needed: &'static str,
    pub connect_google: &'static str,
    pub config_broken: &'static str,

    // Context menu
    pub menu_sync: &'static str,
    pub menu_autostart: &'static str,
    pub menu_config: &'static str,
    pub menu_reset_pos: &'static str,
    pub menu_log: &'static str,
    pub menu_folder: &'static str,
    pub menu_calendars: &'static str,
    pub menu_tasklists: &'static str,
    pub menu_copy: &'static str,
    pub menu_update: &'static str,
    /// "{} available - click to update"
    pub update_available: &'static str,
    pub menu_relogin: &'static str,
    pub menu_quit: &'static str,

    // Sign-in and errors
    pub auth_connected_title: &'static str,
    pub auth_connected_body: &'static str,
    pub auth_cancelled_title: &'static str,
    pub auth_waiting: &'static str,
    pub err_missing_client: &'static str,
    pub err_not_connected: &'static str,
    pub err_grant_expired: &'static str,
    pub err_no_refresh_token: &'static str,
    pub err_timeout: &'static str,
    pub fatal_start: &'static str,
}

pub const EN: Catalog = Catalog {
    code: "en",
    rtl: false,
    section_events: "SCHEDULE",
    section_tasks: "TASKS",
    no_events: "No events today",
    no_tasks: "Nothing due today",
    all_day: "all day",
    today: "today",
    yesterday: "yesterday",
    tomorrow: "Tomorrow",
    conflict_one: "{} conflict",
    conflict_many: "{} conflicts",
    now_label: "NOW",
    next_label: "UP NEXT",
    running: "now",
    in_pattern: "in {}",
    ago_pattern: "{} ago",
    left_pattern: "{} left",
    just_now: "now",
    unit_min: "min",
    unit_hour: "hr",
    unit_day: "d",
    undo: "Undo",
    overdue_one: "{} overdue",
    overdue_many: "{} overdue",
    syncing: "Syncing …",
    updated_next: "Updated {}   ·   next sync {}",
    not_synced: "Not synced yet",
    setup_needed: "Setup required — click for details",
    connect_google: "Connect to Google — click here",
    config_broken: "Invalid configuration — click to open",
    menu_sync: "Sync now",
    menu_autostart: "Start with Windows",
    menu_config: "Edit configuration",
    menu_reset_pos: "Reset position",
    menu_log: "Open log",
    menu_folder: "Open data folder",
    menu_calendars: "Calendars",
    menu_tasklists: "Task lists",
    menu_copy: "Copy agenda",
    menu_update: "Install update",
    update_available: "Version {} available — click to update",
    menu_relogin: "Sign in to Google again",
    menu_quit: "Exit",
    auth_connected_title: "TPMPlaner is connected.",
    auth_connected_body: "You can close this window now.",
    auth_cancelled_title: "Sign-in cancelled.",
    auth_waiting: "Waiting for the Google sign-in …",
    err_missing_client: "client_secret.json is missing. Create an OAuth client (desktop app) in the Google Cloud Console and place the file here:",
    err_not_connected: "Not connected to Google yet.",
    err_grant_expired: "Google revoked or expired the access — please sign in again (right-click the widget). Most common cause: the OAuth client is still in \"Testing\" status, where refresh tokens expire after 7 days.",
    err_no_refresh_token: "Google did not return a refresh token. Remove the access at myaccount.google.com/permissions and sign in again.",
    err_timeout: "Sign-in timed out.",
    fatal_start: "TPMPlaner could not start.",
};

pub const DE: Catalog = Catalog {
    code: "de",
    rtl: false,
    section_events: "TERMINE",
    section_tasks: "AUFGABEN",
    no_events: "Keine Termine heute",
    no_tasks: "Nichts offen für heute",
    all_day: "ganztg.",
    today: "heute",
    yesterday: "gestern",
    tomorrow: "Morgen",
    conflict_one: "{} Überschneidung",
    conflict_many: "{} Überschneidungen",
    now_label: "JETZT",
    next_label: "ALS NÄCHSTES",
    running: "läuft",
    in_pattern: "in {}",
    ago_pattern: "vor {}",
    left_pattern: "noch {}",
    just_now: "jetzt",
    unit_min: "Min",
    unit_hour: "Std",
    unit_day: "Tg",
    undo: "Rückgängig",
    overdue_one: "{} überfällig",
    overdue_many: "{} überfällig",
    syncing: "Synchronisiere …",
    updated_next: "Aktualisiert {}   ·   nächster Abgleich {}",
    not_synced: "Noch nicht abgeglichen",
    setup_needed: "Einrichtung nötig — klicken für Details",
    connect_google: "Mit Google verbinden — hier klicken",
    config_broken: "Konfiguration fehlerhaft — klicken zum Öffnen",
    menu_sync: "Jetzt synchronisieren",
    menu_autostart: "Mit Windows starten",
    menu_config: "Konfiguration bearbeiten",
    menu_reset_pos: "Position zurücksetzen",
    menu_log: "Protokoll öffnen",
    menu_folder: "Datenordner öffnen",
    menu_calendars: "Kalender",
    menu_tasklists: "Aufgabenlisten",
    menu_copy: "Agenda kopieren",
    menu_update: "Update installieren",
    update_available: "Version {} verfügbar — klicken zum Aktualisieren",
    menu_relogin: "Neu bei Google anmelden",
    menu_quit: "Beenden",
    auth_connected_title: "TPMPlaner ist verbunden.",
    auth_connected_body: "Du kannst dieses Fenster jetzt schließen.",
    auth_cancelled_title: "Anmeldung abgebrochen.",
    auth_waiting: "Warte auf die Google-Anmeldung …",
    err_missing_client: "client_secret.json fehlt. Bitte einen OAuth-Client (Desktop-App) in der Google Cloud Console anlegen und die Datei hier ablegen:",
    err_not_connected: "Noch nicht mit Google verbunden.",
    err_grant_expired: "Zugriff von Google widerrufen oder abgelaufen — bitte neu anmelden (Rechtsklick auf das Widget). Häufigste Ursache: Der OAuth-Client steht noch auf Veröffentlichungsstatus \"Testing\", dort verfallen Refresh-Tokens nach 7 Tagen.",
    err_no_refresh_token: "Google hat keinen Refresh-Token geliefert. Bitte den Zugriff unter myaccount.google.com/permissions entfernen und erneut anmelden.",
    err_timeout: "Zeitüberschreitung bei der Anmeldung.",
    fatal_start: "TPMPlaner konnte nicht gestartet werden.",
};

pub const FR: Catalog = Catalog {
    code: "fr",
    rtl: false,
    section_events: "AGENDA",
    section_tasks: "TÂCHES",
    no_events: "Aucun événement aujourd'hui",
    no_tasks: "Rien à faire aujourd'hui",
    all_day: "journée",
    today: "auj.",
    yesterday: "hier",
    tomorrow: "Demain",
    conflict_one: "{} conflit",
    conflict_many: "{} conflits",
    now_label: "MAINTENANT",
    next_label: "À SUIVRE",
    running: "en cours",
    in_pattern: "dans {}",
    ago_pattern: "il y a {}",
    left_pattern: "encore {}",
    just_now: "maintenant",
    unit_min: "min",
    unit_hour: "h",
    unit_day: "j",
    undo: "Annuler",
    overdue_one: "{} en retard",
    overdue_many: "{} en retard",
    syncing: "Synchronisation …",
    updated_next: "Mis à jour {}   ·   prochaine synchro {}",
    not_synced: "Pas encore synchronisé",
    setup_needed: "Configuration requise — cliquez pour les détails",
    connect_google: "Se connecter à Google — cliquez ici",
    config_broken: "Configuration invalide — cliquez pour ouvrir",
    menu_sync: "Synchroniser maintenant",
    menu_autostart: "Démarrer avec Windows",
    menu_config: "Modifier la configuration",
    menu_reset_pos: "Réinitialiser la position",
    menu_log: "Ouvrir le journal",
    menu_folder: "Ouvrir le dossier de données",
    menu_calendars: "Agendas",
    menu_tasklists: "Listes de tâches",
    menu_copy: "Copier l'agenda",
    menu_update: "Installer la mise à jour",
    update_available: "Version {} disponible — cliquez pour mettre à jour",
    menu_relogin: "Se reconnecter à Google",
    menu_quit: "Quitter",
    auth_connected_title: "TPMPlaner est connecté.",
    auth_connected_body: "Vous pouvez fermer cette fenêtre.",
    auth_cancelled_title: "Connexion annulée.",
    auth_waiting: "En attente de la connexion Google …",
    err_missing_client: "client_secret.json est introuvable. Créez un client OAuth (application de bureau) dans la Google Cloud Console et placez le fichier ici :",
    err_not_connected: "Pas encore connecté à Google.",
    err_grant_expired: "Accès révoqué ou expiré — veuillez vous reconnecter (clic droit sur le widget). Cause la plus fréquente : le client OAuth est encore en statut « Testing », où les jetons d'actualisation expirent au bout de 7 jours.",
    err_no_refresh_token: "Google n'a pas fourni de jeton d'actualisation. Supprimez l'accès sur myaccount.google.com/permissions puis reconnectez-vous.",
    err_timeout: "Délai de connexion dépassé.",
    fatal_start: "TPMPlaner n'a pas pu démarrer.",
};

pub const ES: Catalog = Catalog {
    code: "es",
    rtl: false,
    section_events: "AGENDA",
    section_tasks: "TAREAS",
    no_events: "Sin eventos hoy",
    no_tasks: "Nada pendiente para hoy",
    all_day: "todo día",
    today: "hoy",
    yesterday: "ayer",
    tomorrow: "Mañana",
    conflict_one: "{} conflicto",
    conflict_many: "{} conflictos",
    now_label: "AHORA",
    next_label: "A CONTINUACIÓN",
    running: "en curso",
    in_pattern: "en {}",
    ago_pattern: "hace {}",
    left_pattern: "quedan {}",
    just_now: "ahora",
    unit_min: "min",
    unit_hour: "h",
    unit_day: "d",
    undo: "Deshacer",
    overdue_one: "{} atrasada",
    overdue_many: "{} atrasadas",
    syncing: "Sincronizando …",
    updated_next: "Actualizado {}   ·   próxima sinc. {}",
    not_synced: "Aún sin sincronizar",
    setup_needed: "Configuración necesaria — haz clic para ver detalles",
    connect_google: "Conectar con Google — haz clic aquí",
    config_broken: "Configuración no válida — haz clic para abrir",
    menu_sync: "Sincronizar ahora",
    menu_autostart: "Iniciar con Windows",
    menu_config: "Editar configuración",
    menu_reset_pos: "Restablecer posición",
    menu_log: "Abrir registro",
    menu_folder: "Abrir carpeta de datos",
    menu_calendars: "Calendarios",
    menu_tasklists: "Listas de tareas",
    menu_copy: "Copiar agenda",
    menu_update: "Instalar actualización",
    update_available: "Versión {} disponible — haz clic para actualizar",
    menu_relogin: "Volver a iniciar sesión en Google",
    menu_quit: "Salir",
    auth_connected_title: "TPMPlaner está conectado.",
    auth_connected_body: "Ya puedes cerrar esta ventana.",
    auth_cancelled_title: "Inicio de sesión cancelado.",
    auth_waiting: "Esperando el inicio de sesión de Google …",
    err_missing_client: "Falta client_secret.json. Crea un cliente OAuth (aplicación de escritorio) en Google Cloud Console y coloca el archivo aquí:",
    err_not_connected: "Aún no conectado con Google.",
    err_grant_expired: "Acceso revocado o caducado — vuelve a iniciar sesión (clic derecho en el widget). Causa más frecuente: el cliente OAuth sigue en estado «Testing», donde los tokens de actualización caducan a los 7 días.",
    err_no_refresh_token: "Google no devolvió un token de actualización. Elimina el acceso en myaccount.google.com/permissions y vuelve a iniciar sesión.",
    err_timeout: "Tiempo de espera agotado al iniciar sesión.",
    fatal_start: "No se pudo iniciar TPMPlaner.",
};

pub const IT: Catalog = Catalog {
    code: "it",
    rtl: false,
    section_events: "AGENDA",
    section_tasks: "ATTIVITÀ",
    no_events: "Nessun evento oggi",
    no_tasks: "Niente in scadenza oggi",
    all_day: "giornata",
    today: "oggi",
    yesterday: "ieri",
    tomorrow: "Domani",
    conflict_one: "{} conflitto",
    conflict_many: "{} conflitti",
    now_label: "ORA",
    next_label: "PROSSIMO",
    running: "in corso",
    in_pattern: "tra {}",
    ago_pattern: "{} fa",
    left_pattern: "ancora {}",
    just_now: "ora",
    unit_min: "min",
    unit_hour: "h",
    unit_day: "g",
    undo: "Annulla",
    overdue_one: "{} in ritardo",
    overdue_many: "{} in ritardo",
    syncing: "Sincronizzazione …",
    updated_next: "Aggiornato {}   ·   prossima sincr. {}",
    not_synced: "Non ancora sincronizzato",
    setup_needed: "Configurazione necessaria — clicca per i dettagli",
    connect_google: "Connetti a Google — clicca qui",
    config_broken: "Configurazione non valida — clicca per aprire",
    menu_sync: "Sincronizza ora",
    menu_autostart: "Avvia con Windows",
    menu_config: "Modifica configurazione",
    menu_reset_pos: "Reimposta posizione",
    menu_log: "Apri registro",
    menu_folder: "Apri cartella dati",
    menu_calendars: "Calendari",
    menu_tasklists: "Elenchi attività",
    menu_copy: "Copia agenda",
    menu_update: "Installa aggiornamento",
    update_available: "Versione {} disponibile — clicca per aggiornare",
    menu_relogin: "Accedi di nuovo a Google",
    menu_quit: "Esci",
    auth_connected_title: "TPMPlaner è connesso.",
    auth_connected_body: "Puoi chiudere questa finestra.",
    auth_cancelled_title: "Accesso annullato.",
    auth_waiting: "In attesa dell'accesso Google …",
    err_missing_client: "client_secret.json non trovato. Crea un client OAuth (app desktop) nella Google Cloud Console e inserisci il file qui:",
    err_not_connected: "Non ancora connesso a Google.",
    err_grant_expired: "Accesso revocato o scaduto — accedi di nuovo (clic destro sul widget). Causa più frequente: il client OAuth è ancora in stato «Testing», dove i token di aggiornamento scadono dopo 7 giorni.",
    err_no_refresh_token: "Google non ha restituito un token di aggiornamento. Rimuovi l'accesso su myaccount.google.com/permissions e accedi di nuovo.",
    err_timeout: "Timeout durante l'accesso.",
    fatal_start: "Impossibile avviare TPMPlaner.",
};

/// Picks a catalogue from the primary language subtag.
///
/// Regional variants share one catalogue: `de-AT` and `de-CH` both get `DE`.
/// Dates and times still differ correctly, because those come from the
/// operating system and not from here.
pub fn catalog_for(tag: &str) -> &'static Catalog {
    match tag
        .split(['-', '_'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "de" => &DE,
        "fr" => &FR,
        "es" => &ES,
        "it" => &IT,
        _ => &EN,
    }
}

/// Catalogue for code that has no access to the window's [`Locale`].
///
/// That means the sync thread — OAuth messages, the sign-in page shown in the
/// browser — and the emergency exit in `main`. An `RwLock` rather than a
/// `OnceLock`, because the language can change through the configuration while
/// running.
static GLOBAL: std::sync::RwLock<&'static Catalog> = std::sync::RwLock::new(&EN);

pub fn set_global(cat: &'static Catalog) {
    if let Ok(mut g) = GLOBAL.write() {
        *g = cat;
    }
}

/// The current catalogue. Falls back to English on a poisoned lock: a missing
/// translation must never prevent a sign-in.
pub fn global() -> &'static Catalog {
    GLOBAL.read().map(|g| *g).unwrap_or(&EN)
}

pub struct Locale {
    /// BCP-47 tag as the system reports it, for example `de-DE`.
    pub tag: String,
    pub cat: &'static Catalog,
    /// Right-to-left reading (Arabic, Hebrew, Persian and others).
    pub rtl: bool,
}

impl Locale {
    /// `pref` is `"system"` or a BCP-47 tag from the configuration.
    pub fn resolve(pref: &str) -> Self {
        let backend = locale_backend();
        let tag = match pref.trim() {
            "" | "system" | "auto" => backend.user_default_tag(),
            explicit => explicit.to_string(),
        };
        let rtl = backend.is_rtl(&tag);
        Self {
            cat: catalog_for(&tag),
            tag,
            rtl,
        }
    }

    /// Time of day in the locale's short format.
    ///
    /// This is where twelve- against twenty-four-hour counting is decided, the
    /// way the user set it up.
    pub fn time(&self, dt: DateTime<Local>) -> String {
        locale_backend()
            .format_time(&self.tag, dt)
            .unwrap_or_else(|| format!("{:02}:{:02}", dt.hour(), dt.minute()))
    }

    /// Full weekday name ("Dienstag", "Tuesday", "الثلاثاء").
    pub fn weekday(&self, d: NaiveDate) -> String {
        locale_backend()
            .format_weekday(&self.tag, d)
            .unwrap_or_else(|| d.weekday().to_string())
    }

    /// Long date **without** the weekday — that already sits one line above.
    pub fn date_line(&self, d: NaiveDate) -> String {
        locale_backend()
            .format_date(&self.tag, d)
            .unwrap_or_else(|| format!("{}-{:02}-{:02}", d.year(), d.month(), d.day()))
    }

    /// Compact day and month for the due column ("4 Aug", "4 août").
    pub fn day_month(&self, d: NaiveDate) -> String {
        locale_backend()
            .format_day_month(&self.tag, d)
            .unwrap_or_else(|| format!("{:02}.{:02}.", d.day(), d.month()))
    }

    /// Isolates Latin text inside a mirrored layout.
    ///
    /// Without this bracket the Unicode bidi algorithm applies the paragraph
    /// direction to the weak characters at the edges: "32 min left" visibly
    /// becomes "min left 32", because the leading digit counts as weakly
    /// left-to-right and slides to the other end.
    ///
    /// Uses `U+202A LEFT-TO-RIGHT EMBEDDING` and `U+202C POP DIRECTIONAL
    /// FORMATTING`. The newer isolates (`U+2066` / `U+2069`, Unicode 6.3) are
    /// not honoured by DirectWrite — with them the reordering persisted.
    ///
    /// User data such as event titles and task names is deliberately left
    /// alone: it may well be Arabic and has to run freely.
    fn iso(&self, s: String) -> String {
        if self.rtl && !self.cat.rtl {
            format!("\u{202A}{s}\u{202C}")
        } else {
            s
        }
    }

    /// A fixed catalogue label, prepared for display.
    pub fn label(&self, s: &str) -> String {
        self.iso(s.to_string())
    }

    /// Menschenlesbarer Abstand: "in 25 min", "2 hr 10 ago", "encore 5 min".
    ///
    /// Deliberately coarse, with abbreviated units — which also sidesteps the
    /// plural rules that would otherwise need their own forms per language.
    pub fn relative(&self, minutes: i64) -> String {
        let c = self.cat;
        if minutes.abs() < 1 {
            return self.label(c.just_now);
        }
        let body = self.duration_raw(minutes.abs());
        self.iso(if minutes < 0 {
            c.ago_pattern.replacen("{}", &body, 1)
        } else {
            c.in_pattern.replacen("{}", &body, 1)
        })
    }

    /// Restlaufzeit des gerade laufenden Termins ("noch 32 Min").
    pub fn time_left(&self, minutes: i64) -> String {
        let body = self.duration_raw(minutes.max(0));
        self.iso(self.cat.left_pattern.replacen("{}", &body, 1))
    }

    /// The bare amount without a direction word, and without the bidi bracket.
    fn duration_raw(&self, minutes: i64) -> String {
        let c = self.cat;
        let m = minutes.max(0);
        if m < 60 {
            format!("{m} {}", c.unit_min)
        } else if m < 60 * 24 {
            let (h, rest) = (m / 60, m % 60);
            if rest == 0 {
                format!("{h} {}", c.unit_hour)
            } else {
                format!("{h} {} {rest}", c.unit_hour)
            }
        } else {
            format!("{} {}", m / (60 * 24), c.unit_day)
        }
    }

    pub fn overdue(&self, n: usize) -> String {
        let pattern = if n == 1 {
            self.cat.overdue_one
        } else {
            self.cat.overdue_many
        };
        self.iso(pattern.replacen("{}", &n.to_string(), 1))
    }

    pub fn conflicts(&self, n: usize) -> String {
        let pattern = if n == 1 {
            self.cat.conflict_one
        } else {
            self.cat.conflict_many
        };
        self.iso(pattern.replacen("{}", &n.to_string(), 1))
    }

    pub fn updated_next(&self, updated: &str, next: &str) -> String {
        self.iso(
            self.cat
                .updated_next
                .replacen("{}", updated, 1)
                .replacen("{}", next, 1),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regional_variants_share_a_catalog() {
        assert_eq!(catalog_for("de-AT").code, "de");
        assert_eq!(catalog_for("de_CH").code, "de");
        assert_eq!(catalog_for("de").code, "de");
    }

    #[test]
    fn unknown_languages_fall_back_to_english() {
        assert_eq!(catalog_for("ja-JP").code, "en");
        assert_eq!(catalog_for("").code, "en");
        assert_eq!(catalog_for("xx").code, "en");
    }

    #[test]
    fn relative_time_uses_the_catalog_patterns() {
        let de = Locale::resolve("de-DE");
        assert_eq!(de.relative(0), "jetzt");
        assert_eq!(de.relative(25), "in 25 Min");
        assert_eq!(de.relative(130), "in 2 Std 10");
        assert_eq!(de.relative(-5), "vor 5 Min");
        assert_eq!(de.time_left(32), "noch 32 Min");

        let en = Locale::resolve("en-US");
        assert_eq!(en.relative(25), "in 25 min");
        assert_eq!(en.relative(-5), "5 min ago");
        assert_eq!(en.time_left(32), "32 min left");
    }

    #[test]
    fn spanish_plural_differs_by_count() {
        let es = Locale::resolve("es-ES");
        assert_eq!(es.overdue(1), "1 atrasada");
        assert_eq!(es.overdue(3), "3 atrasadas");
    }

    /// The platform decides how a date looks; this crate only has to route
    /// the question. Verifying the actual formats needs a locale database, so
    /// those assertions live with the host that provides one — see
    /// `host_impl` in the Windows front end.
    #[test]
    fn formatting_is_delegated_and_never_panics() {
        use chrono::TimeZone;
        let loc = Locale::resolve("de-DE");
        let date = NaiveDate::from_ymd_opt(2026, 8, 4).unwrap();
        let dt = Local.with_ymd_and_hms(2026, 8, 4, 20, 9, 0).unwrap();

        assert!(!loc.time(dt).is_empty());
        assert!(!loc.weekday(date).is_empty());
        assert!(!loc.date_line(date).is_empty());
        assert!(!loc.day_month(date).is_empty());
    }

    #[test]
    fn reading_direction_is_detected() {
        // The portable fallback knows the right-to-left languages by their
        // primary subtag, so this holds without a platform locale database.
        assert!(!Locale::resolve("de-DE").rtl);
        assert!(!Locale::resolve("en-US").rtl);
        assert!(Locale::resolve("ar-SA").rtl, "Arabic must be right to left");
        assert!(Locale::resolve("he-IL").rtl, "Hebrew must be right to left");
    }
}
