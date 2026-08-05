// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! Internationalisierung.
//!
//! Zwei getrennte Zustaendigkeiten, und die Trennung ist der Kern des Ganzen:
//!
//! * **Oberflaechentexte** kommen aus einem einkompilierten Katalog. Jede
//!   Sprache ist ein [`Catalog`]-Literal; eine weitere hinzuzufuegen heisst,
//!   eine Konstante zu schreiben und sie in [`catalog_for`] einzutragen.
//! * **Datum, Uhrzeit und Leserichtung** kommen vom Betriebssystem
//!   (`GetDateFormatEx`, `GetTimeFormatEx`, `GetLocaleInfoEx`). Das ist der
//!   entscheidende Unterschied zu einer blossen Uebersetzung: Windows kennt
//!   fuer *jedes* Gebietsschema die richtige Datumsreihenfolge, die lokalen
//!   Monatsnamen und vor allem, ob 12- oder 24-Stunden-Zaehlung gilt. Eine
//!   fest verdrahtete Formatierung `{:02}:{:02}` zeigt einem Benutzer in den
//!   USA "20:09" statt "8:09 PM" — formal richtig, aber falsch.
//!
//! Dadurch funktionieren Datum und Uhrzeit auch in Sprachen korrekt, fuer die
//! gar kein Katalog existiert; dort werden lediglich die Beschriftungen
//! englisch.

use chrono::{DateTime, Datelike, Local, NaiveDate, Timelike};
use windows::Win32::Foundation::SYSTEMTIME;
use windows::Win32::Globalization::{
    DATE_LONGDATE, ENUM_DATE_FORMATS_FLAGS, GetDateFormatEx, GetLocaleInfoEx, GetTimeFormatEx,
    GetUserDefaultLocaleName, LOCALE_IREADINGLAYOUT, LOCALE_RETURN_NUMBER, LOCALE_SLONGDATE,
    TIME_NOSECONDS,
};
use windows::core::PCWSTR;

/// Alle Zeichenketten der Oberflaeche.
///
/// Bewusst ein Struct mit benannten Feldern statt einer Nachschlagetabelle:
/// eine vergessene Uebersetzung ist damit ein Kompilierfehler, kein zur
/// Laufzeit fehlender Schluessel.
pub struct Catalog {
    pub code: &'static str,
    /// Ist die Sprache selbst rechts-nach-links geschrieben?
    ///
    /// Nicht dasselbe wie die Leserichtung des Gebietsschemas: laeuft das
    /// Widget unter `ar-SA`, ist das *Layout* gespiegelt, die Texte kommen
    /// aber mangels arabischem Katalog auf Englisch — also lateinisch und
    /// links-nach-rechts. Beides muss getrennt behandelt werden.
    pub rtl: bool,

    // Abschnitte und Listen
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

    // Hervorgehobener Termin
    pub now_label: &'static str,
    pub next_label: &'static str,
    pub running: &'static str,

    // Relative Zeiten. `{}` steht fuer die Menge samt Einheit.
    pub in_pattern: &'static str,
    pub ago_pattern: &'static str,
    pub left_pattern: &'static str,
    pub just_now: &'static str,
    pub unit_min: &'static str,
    pub unit_hour: &'static str,
    pub unit_day: &'static str,

    // Aufgaben
    pub undo: &'static str,
    pub overdue_one: &'static str,
    pub overdue_many: &'static str,

    // Statuszeile
    pub syncing: &'static str,
    pub updated_next: &'static str,
    pub not_synced: &'static str,
    pub setup_needed: &'static str,
    pub connect_google: &'static str,
    pub config_broken: &'static str,

    // Kontextmenue
    pub menu_sync: &'static str,
    pub menu_autostart: &'static str,
    pub menu_config: &'static str,
    pub menu_reset_pos: &'static str,
    pub menu_log: &'static str,
    pub menu_folder: &'static str,
    pub menu_calendars: &'static str,
    pub menu_tasklists: &'static str,
    pub menu_copy: &'static str,
    pub menu_relogin: &'static str,
    pub menu_quit: &'static str,

    // Anmeldung und Fehler
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

/// Katalog anhand des primaeren Sprach-Subtags.
///
/// Regionale Varianten teilen sich einen Katalog: `de-AT` und `de-CH`
/// bekommen `DE`. Datum und Uhrzeit unterscheiden sich trotzdem korrekt, weil
/// die vom Betriebssystem kommen und nicht von hier.
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

/// Katalog fuer Code, der keinen Zugriff auf die [`Locale`] des Fensters hat.
///
/// Betrifft den Sync-Thread (OAuth-Meldungen, Anmeldeseite im Browser) und den
/// Notausgang in `main`. Ein `RwLock` statt `OnceLock`, weil die Sprache ueber
/// die Konfiguration im Betrieb wechseln kann.
static GLOBAL: std::sync::RwLock<&'static Catalog> = std::sync::RwLock::new(&EN);

pub fn set_global(cat: &'static Catalog) {
    if let Ok(mut g) = GLOBAL.write() {
        *g = cat;
    }
}

/// Aktueller Katalog. Faellt bei vergifteter Sperre auf Englisch zurueck —
/// eine fehlende Uebersetzung darf keine Anmeldung verhindern.
pub fn global() -> &'static Catalog {
    GLOBAL.read().map(|g| *g).unwrap_or(&EN)
}

pub struct Locale {
    /// BCP-47-Kennung, so wie Windows sie liefert (z. B. `de-DE`).
    pub tag: String,
    pub cat: &'static Catalog,
    /// Rechts-nach-links-Leserichtung (Arabisch, Hebraeisch, Persisch …).
    pub rtl: bool,
    /// Zwischengespeichertes Datumsmuster ohne Wochentag.
    date_pattern: Vec<u16>,
}

impl Locale {
    /// `pref` ist `"system"` oder ein BCP-47-Tag aus der Konfiguration.
    pub fn resolve(pref: &str) -> Self {
        let tag = match pref.trim() {
            "" | "system" | "auto" => user_default_locale(),
            explicit => explicit.to_string(),
        };
        let cat = catalog_for(&tag);
        let rtl = reading_layout_is_rtl(&tag);
        let date_pattern = long_date_without_weekday(&tag);
        Self {
            tag,
            cat,
            rtl,
            date_pattern,
        }
    }

    /// Uhrzeit im kurzen Format des Gebietsschemas.
    ///
    /// Hier entscheidet sich 12- gegen 24-Stunden-Zaehlung, und zwar so, wie
    /// der Benutzer es in Windows eingestellt hat.
    pub fn time(&self, dt: DateTime<Local>) -> String {
        let st = to_systemtime_time(dt);
        format_time(&self.tag, &st)
            .unwrap_or_else(|| format!("{:02}:{:02}", dt.hour(), dt.minute()))
    }

    /// Ausgeschriebener Wochentag ("Dienstag", "Tuesday", "الثلاثاء").
    pub fn weekday(&self, d: NaiveDate) -> String {
        let st = to_systemtime_date(d);
        format_date(&self.tag, &st, Some(&wide("dddd"))).unwrap_or_else(|| d.weekday().to_string())
    }

    /// Langes Datum **ohne** Wochentag — der steht bereits eine Zeile darueber.
    pub fn date_line(&self, d: NaiveDate) -> String {
        let st = to_systemtime_date(d);
        let pattern = (!self.date_pattern.is_empty()).then_some(&self.date_pattern);
        format_date(&self.tag, &st, pattern.map(|v| v.as_slice()))
            .unwrap_or_else(|| format!("{}-{:02}-{:02}", d.year(), d.month(), d.day()))
    }

    /// Kompaktes Tag/Monat fuer die Faelligkeitsspalte ("4 Aug", "4 août").
    pub fn day_month(&self, d: NaiveDate) -> String {
        let st = to_systemtime_date(d);
        format_date(&self.tag, &st, Some(&wide("d MMM")))
            .unwrap_or_else(|| format!("{:02}.{:02}.", d.day(), d.month()))
    }

    /// Isoliert lateinischen Text in einem gespiegelten Layout.
    ///
    /// Ohne diese Klammer wendet der Unicode-Bidi-Algorithmus die Absatz-
    /// richtung auf die schwachen Zeichen am Rand an: aus "32 min left" wird
    /// sichtbar "min left 32", weil die fuehrende Ziffer als schwach
    /// links-nach-rechts gilt und ans andere Ende rutscht.
    ///
    /// Verwendet werden `U+202A LEFT-TO-RIGHT EMBEDDING` und
    /// `U+202C POP DIRECTIONAL FORMATTING`. Die neueren Isolate (`U+2066` /
    /// `U+2069`, Unicode 6.3) werden von DirectWrite nicht ausgewertet — mit
    /// ihnen blieb die Umsortierung bestehen.
    ///
    /// Nutzerdaten (Termintitel, Aufgabennamen) bleiben bewusst unangetastet —
    /// die koennen sehr wohl arabisch sein und muessen frei laufen duerfen.
    fn iso(&self, s: String) -> String {
        if self.rtl && !self.cat.rtl {
            format!("\u{202A}{s}\u{202C}")
        } else {
            s
        }
    }

    /// Feste Katalogbeschriftung, fuer die Anzeige aufbereitet.
    pub fn label(&self, s: &str) -> String {
        self.iso(s.to_string())
    }

    /// Menschenlesbarer Abstand: "in 25 min", "2 hr 10 ago", "encore 5 min".
    ///
    /// Bewusst grob gerundet und mit abgekuerzten Einheiten — das umgeht
    /// zugleich die Pluralregeln, die sonst je Sprache eigene Formen
    /// braeuchten.
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

    /// Reine Mengenangabe ohne Richtungswort, noch ohne Bidi-Klammer.
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

// --- Windows-NLS ------------------------------------------------------------

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn from_wide(buf: &[u16], len: i32) -> Option<String> {
    if len <= 0 {
        return None;
    }
    // Die Get*FormatEx-Funktionen zaehlen die abschliessende Null mit.
    let n = (len as usize).saturating_sub(1).min(buf.len());
    let s = String::from_utf16_lossy(&buf[..n]);
    (!s.trim().is_empty()).then_some(s)
}

fn user_default_locale() -> String {
    // LOCALE_NAME_MAX_LENGTH ist 85.
    let mut buf = [0u16; 85];
    let len = unsafe { GetUserDefaultLocaleName(&mut buf) };
    from_wide(&buf, len).unwrap_or_else(|| "en-US".to_string())
}

/// `LOCALE_IREADINGLAYOUT` == 1 bedeutet rechts-nach-links.
fn reading_layout_is_rtl(tag: &str) -> bool {
    let name = wide(tag);
    let mut value: u32 = 0;
    let ok = unsafe {
        GetLocaleInfoEx(
            PCWSTR(name.as_ptr()),
            LOCALE_IREADINGLAYOUT | LOCALE_RETURN_NUMBER,
            // Bei LOCALE_RETURN_NUMBER erwartet die API einen Puffer, der als
            // DWORD interpretiert wird.
            Some(std::slice::from_raw_parts_mut(
                &mut value as *mut u32 as *mut u16,
                2,
            )),
        )
    };
    ok > 0 && value == 1
}

/// Das lange Datumsmuster des Gebietsschemas ohne den Wochentag.
///
/// Der Wochentag steht im Widget bereits eine Zeile darueber. Ihn aus dem
/// Muster zu entfernen ist zuverlaessiger, als ein eigenes Muster je Sprache
/// zu erfinden — die Reihenfolge von Tag, Monat und Jahr bleibt so die des
/// Gebietsschemas.
fn long_date_without_weekday(tag: &str) -> Vec<u16> {
    let name = wide(tag);
    let mut buf = [0u16; 128];
    let len = unsafe {
        GetLocaleInfoEx(
            PCWSTR(name.as_ptr()),
            LOCALE_SLONGDATE,
            Some(buf.as_mut_slice()),
        )
    };
    let Some(pattern) = from_wide(&buf, len) else {
        return Vec::new();
    };

    // "dddd, d. MMMM yyyy" -> "d. MMMM yyyy"
    let mut cleaned = pattern.replace("dddd", "");
    // Zurueckbleibende Trennzeichen an den Raendern abraeumen.
    let trim: &[char] = &[' ', ',', '،', '、', '.', '-', '/'];
    cleaned = cleaned.trim_matches(trim).to_string();
    // Doppelte Leerzeichen aus der Mitte entfernen.
    while cleaned.contains("  ") {
        cleaned = cleaned.replace("  ", " ");
    }

    if cleaned.is_empty() {
        Vec::new()
    } else {
        wide(&cleaned)
    }
}

fn format_time(tag: &str, st: &SYSTEMTIME) -> Option<String> {
    let name = wide(tag);
    let mut buf = [0u16; 96];
    let len = unsafe {
        GetTimeFormatEx(
            PCWSTR(name.as_ptr()),
            TIME_NOSECONDS,
            Some(st),
            PCWSTR::null(),
            Some(buf.as_mut_slice()),
        )
    };
    from_wide(&buf, len)
}

fn format_date(tag: &str, st: &SYSTEMTIME, pattern: Option<&[u16]>) -> Option<String> {
    let name = wide(tag);
    let mut buf = [0u16; 160];
    let fmt = match pattern {
        Some(p) => PCWSTR(p.as_ptr()),
        None => PCWSTR::null(),
    };
    // Eigenes Muster und DATE_LONGDATE schliessen sich gegenseitig aus.
    let flags = if pattern.is_some() {
        ENUM_DATE_FORMATS_FLAGS(0)
    } else {
        DATE_LONGDATE
    };
    let len = unsafe {
        GetDateFormatEx(
            PCWSTR(name.as_ptr()),
            flags,
            Some(st),
            fmt,
            Some(buf.as_mut_slice()),
            PCWSTR::null(),
        )
    };
    from_wide(&buf, len)
}

fn to_systemtime_date(d: NaiveDate) -> SYSTEMTIME {
    SYSTEMTIME {
        wYear: d.year() as u16,
        wMonth: d.month() as u16,
        wDayOfWeek: d.weekday().num_days_from_sunday() as u16,
        wDay: d.day() as u16,
        ..Default::default()
    }
}

fn to_systemtime_time(dt: DateTime<Local>) -> SYSTEMTIME {
    SYSTEMTIME {
        wYear: dt.year() as u16,
        wMonth: dt.month() as u16,
        wDayOfWeek: dt.weekday().num_days_from_sunday() as u16,
        wDay: dt.day() as u16,
        wHour: dt.hour() as u16,
        wMinute: dt.minute() as u16,
        wSecond: dt.second() as u16,
        wMilliseconds: 0,
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

    #[test]
    fn twelve_and_twentyfour_hour_clocks_both_come_from_windows() {
        use chrono::TimeZone;
        let dt = Local.with_ymd_and_hms(2026, 8, 4, 20, 9, 0).unwrap();

        // Deutsch zaehlt 24-stuendig, US-Englisch 12-stuendig. Genau diese
        // Unterscheidung ginge bei fest verdrahtetem "{:02}:{:02}" verloren.
        let de = Locale::resolve("de-DE").time(dt);
        let us = Locale::resolve("en-US").time(dt);
        assert!(de.contains("20"), "de-DE: {de}");
        assert!(
            us.contains('8') && us.to_ascii_uppercase().contains("PM"),
            "en-US: {us}"
        );
    }

    #[test]
    fn date_order_follows_the_locale() {
        let d = NaiveDate::from_ymd_opt(2026, 8, 4).unwrap();
        // Im Deutschen steht der Tag vorn, im US-Englischen der Monat.
        let de = Locale::resolve("de-DE").date_line(d);
        let us = Locale::resolve("en-US").date_line(d);
        assert!(de.starts_with('4'), "de-DE: {de}");
        assert!(us.starts_with("August"), "en-US: {us}");
        // Der Wochentag gehoert in die Zeile darueber und muss hier fehlen.
        assert!(!de.contains("Dienstag"), "de-DE: {de}");
        assert!(!us.contains("Tuesday"), "en-US: {us}");
    }

    #[test]
    fn weekday_is_localised() {
        let d = NaiveDate::from_ymd_opt(2026, 8, 4).unwrap();
        assert_eq!(Locale::resolve("de-DE").weekday(d), "Dienstag");
        assert_eq!(Locale::resolve("en-US").weekday(d), "Tuesday");
    }

    #[test]
    fn reading_direction_is_detected() {
        assert!(!Locale::resolve("de-DE").rtl);
        assert!(!Locale::resolve("en-US").rtl);
        assert!(Locale::resolve("ar-SA").rtl, "Arabisch muss RTL sein");
        assert!(Locale::resolve("he-IL").rtl, "Hebräisch muss RTL sein");
    }
}
