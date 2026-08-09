// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! Internationalisation.
//!
//! Two responsibilities, and keeping them apart is the whole point:
//!
//! * **Interface text** comes from compiled-in catalogues. Each language is a
//!   [`Catalog`] literal; adding one means writing a constant and listing it
//!   in [`catalog_for`] and [`CATALOGS`].
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

/// The forms of one counted message, together with the rule that picks between
/// them.
///
/// English gets by with two forms and a comparison against one. Russian needs
/// three and has to look at the last two digits; Arabic needs six; Japanese
/// needs one and would look wrong with more. A single `_one` / `_many` pair per
/// message cannot express that, and wording every counted string so the
/// distinction never arises would mean writing around the language rather than
/// in it.
///
/// The variant *is* the language's plural rule and its fields are exactly the
/// forms that rule can select, so a catalogue cannot pair Russian forms with
/// the English rule — choosing the forms and choosing the rule are the same
/// act. The categories and their boundaries follow the CLDR plural rules.
///
/// Note that `one` and `two` deliberately need not contain the `{}`
/// placeholder: "one conflict" reads better than "1 conflict", and in Arabic
/// and Hebrew the numeral is carried by the word itself (تعارض واحد,
/// שתי חפיפות). [`Locale`] substitutes only if there is something to
/// substitute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plural {
    /// One form for every count. Chinese, Japanese, Korean, Vietnamese, Thai
    /// and Turkish: the noun after a numeral simply does not change.
    Invariant(&'static str),
    /// `one` for exactly one, `other` for the rest — English, German, Dutch,
    /// Spanish, Italian, Swedish, European Portuguese.
    OneOther {
        one: &'static str,
        other: &'static str,
    },
    /// Like [`Plural::OneOther`], but zero takes the singular as well. French
    /// and Brazilian Portuguese: "0 conflit", not "0 conflits".
    OneIncludingZero {
        one: &'static str,
        other: &'static str,
    },
    /// Russian and Ukrainian: the last two digits decide. 21 is `one`, 22 is
    /// `few`, 25 is `many`, and 11 to 14 are `many` whatever their last digit.
    EastSlavic {
        one: &'static str,
        few: &'static str,
        many: &'static str,
    },
    /// Polish. Close to [`Plural::EastSlavic`] but not the same: only a bare 1
    /// is `one`, so 21 takes the `many` form where Russian takes `one`.
    Polish {
        one: &'static str,
        few: &'static str,
        many: &'static str,
    },
    /// Czech and Slovak: `one` for 1, `few` for 2 to 4, `other` from 5 upwards
    /// and for zero.
    Czech {
        one: &'static str,
        few: &'static str,
        other: &'static str,
    },
    /// Hebrew, which has a genuine dual: 2 is neither singular nor plural.
    ///
    /// CLDR also lists a `many` for the round tens — 20, 30, 100 — which this
    /// deliberately folds into `other`. The category only earns its keep for
    /// nouns that change shape after a round number, and neither counted
    /// string in [`HE`] does: 20 is "{} חפיפות", the same plural 3 takes. A
    /// fourth field would hold a copy of `other` and nothing else.
    Hebrew {
        one: &'static str,
        two: &'static str,
        other: &'static str,
    },
    /// Arabic, with all six CLDR categories. 3 to 10 is `few`, 11 to 99 is
    /// `many`, and 100, 101, 102 fall back to `other`.
    Arabic {
        zero: &'static str,
        one: &'static str,
        two: &'static str,
        few: &'static str,
        many: &'static str,
        other: &'static str,
    },
}

impl Plural {
    /// The form to use for `n`.
    pub fn select(&self, n: u64) -> &'static str {
        let last = n % 10;
        let last_two = n % 100;
        match *self {
            Plural::Invariant(s) => s,
            Plural::OneOther { one, other } => {
                if n == 1 {
                    one
                } else {
                    other
                }
            }
            Plural::OneIncludingZero { one, other } => {
                if n <= 1 {
                    one
                } else {
                    other
                }
            }
            Plural::EastSlavic { one, few, many } => {
                if last == 1 && last_two != 11 {
                    one
                } else if (2..=4).contains(&last) && !(12..=14).contains(&last_two) {
                    few
                } else {
                    many
                }
            }
            Plural::Polish { one, few, many } => {
                if n == 1 {
                    one
                } else if (2..=4).contains(&last) && !(12..=14).contains(&last_two) {
                    few
                } else {
                    many
                }
            }
            Plural::Czech { one, few, other } => {
                if n == 1 {
                    one
                } else if (2..=4).contains(&n) {
                    few
                } else {
                    other
                }
            }
            Plural::Hebrew { one, two, other } => match n {
                1 => one,
                2 => two,
                _ => other,
            },
            Plural::Arabic {
                zero,
                one,
                two,
                few,
                many,
                other,
            } => match n {
                0 => zero,
                1 => one,
                2 => two,
                _ if (3..=10).contains(&last_two) => few,
                _ if (11..=99).contains(&last_two) => many,
                _ => other,
            },
        }
    }

    /// Every form this value carries, for the consistency checks.
    #[cfg(test)]
    fn forms(&self) -> Vec<&'static str> {
        match *self {
            Plural::Invariant(s) => vec![s],
            Plural::OneOther { one, other } | Plural::OneIncludingZero { one, other } => {
                vec![one, other]
            }
            Plural::EastSlavic { one, few, many } | Plural::Polish { one, few, many } => {
                vec![one, few, many]
            }
            Plural::Czech { one, few, other } => vec![one, few, other],
            Plural::Hebrew { one, two, other } => vec![one, two, other],
            Plural::Arabic {
                zero,
                one,
                two,
                few,
                many,
                other,
            } => vec![zero, one, two, few, many, other],
        }
    }

    /// The forms that must carry the `{}` placeholder.
    ///
    /// `one` and `two` are exempt: a language that spells the numeral out in
    /// the word — Arabic's تعارض واحد, Hebrew's שתי חפיפות — would repeat
    /// itself with a digit in front.
    #[cfg(test)]
    fn counted_forms(&self) -> Vec<&'static str> {
        match *self {
            Plural::Invariant(s) => vec![s],
            Plural::OneOther { other, .. } | Plural::OneIncludingZero { other, .. } => vec![other],
            Plural::EastSlavic { few, many, .. } | Plural::Polish { few, many, .. } => {
                vec![few, many]
            }
            Plural::Czech { few, other, .. } => vec![few, other],
            Plural::Hebrew { other, .. } => vec![other],
            Plural::Arabic {
                zero,
                few,
                many,
                other,
                ..
            } => vec![zero, few, many, other],
        }
    }
}

/// Every string in the interface.
///
/// Deliberately a struct with named fields rather than a lookup table: a
/// forgotten translation is then a compile error, not a key missing at
/// runtime.
pub struct Catalog {
    /// The tag this catalogue is written for. `zh-Hans` and `pt-BR` carry a
    /// subtag because the language alone does not identify the text.
    pub code: &'static str,
    /// Is the language itself written right to left?
    ///
    /// Not the same as the locale's reading direction: running under `ar-SA`
    /// with no Arabic catalogue mirrors the *layout* while the text arrives in
    /// English — Latin script, left to right — and the two then have to be
    /// handled separately. With [`AR`] and [`HE`] both are right to left and
    /// the isolation brackets in [`Locale::iso`] fall away.
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
    /// "{} conflict" and its plural forms.
    pub conflicts: Plural,

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
    /// "{} overdue" and its plural forms.
    pub overdue: Plural,

    // Status line
    pub syncing: &'static str,
    pub updated_next: &'static str,
    pub not_synced: &'static str,
    pub setup_needed: &'static str,
    pub connect_google: &'static str,
    pub config_broken: &'static str,

    // Context menu
    pub menu_sync: &'static str,
    /// "Start at login", not "Start with Windows": the widget starts with the
    /// session on all three systems, through a registry value, a login item
    /// and an XDG autostart file respectively, and naming one of them in the
    /// menu is wrong on the other two.
    pub menu_autostart: &'static str,
    pub menu_config: &'static str,
    pub menu_reset_pos: &'static str,
    /// Pin the widget where it is. Phrased as the action a click performs, not
    /// as the state it is in — see [`crate::menu::context_menu`].
    pub menu_lock: &'static str,
    pub menu_unlock: &'static str,
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

impl Catalog {
    /// `lang` and `dir` attributes for the `<html>` element of the pages the
    /// sign-in flow serves into the user's browser.
    ///
    /// The widget itself gets its reading direction from the platform, but
    /// that never reaches the browser — a page with no `dir` is laid out
    /// left to right whatever the script, so Arabic and Hebrew come out with
    /// the clause order and the trailing punctuation on the wrong side. The
    /// catalogue already knows which of the two it is written in, and it is
    /// the same catalogue these pages take their text from.
    pub fn html_attrs(&self) -> String {
        let dir = if self.rtl { "rtl" } else { "ltr" };
        format!("lang=\"{}\" dir=\"{dir}\"", self.code)
    }

    /// Every plain field, paired with its name.
    ///
    /// Written as a destructuring `let` without `..` on purpose: a field added
    /// to [`Catalog`] fails to compile here until it is listed, so the checks
    /// that walk this list cannot quietly stop covering it.
    #[cfg(test)]
    fn fields(&self) -> Vec<(&'static str, &'static str)> {
        let Catalog {
            code: _,
            rtl: _,
            conflicts: _,
            overdue: _,
            section_events,
            section_tasks,
            no_events,
            no_tasks,
            all_day,
            today,
            yesterday,
            tomorrow,
            now_label,
            next_label,
            running,
            in_pattern,
            ago_pattern,
            left_pattern,
            just_now,
            unit_min,
            unit_hour,
            unit_day,
            undo,
            syncing,
            updated_next,
            not_synced,
            setup_needed,
            connect_google,
            config_broken,
            menu_sync,
            menu_autostart,
            menu_config,
            menu_reset_pos,
            menu_lock,
            menu_unlock,
            menu_log,
            menu_folder,
            menu_calendars,
            menu_tasklists,
            menu_copy,
            menu_update,
            update_available,
            menu_relogin,
            menu_quit,
            auth_connected_title,
            auth_connected_body,
            auth_cancelled_title,
            auth_waiting,
            err_missing_client,
            err_not_connected,
            err_grant_expired,
            err_no_refresh_token,
            err_timeout,
            fatal_start,
        } = self;

        vec![
            ("section_events", section_events),
            ("section_tasks", section_tasks),
            ("no_events", no_events),
            ("no_tasks", no_tasks),
            ("all_day", all_day),
            ("today", today),
            ("yesterday", yesterday),
            ("tomorrow", tomorrow),
            ("now_label", now_label),
            ("next_label", next_label),
            ("running", running),
            ("in_pattern", in_pattern),
            ("ago_pattern", ago_pattern),
            ("left_pattern", left_pattern),
            ("just_now", just_now),
            ("unit_min", unit_min),
            ("unit_hour", unit_hour),
            ("unit_day", unit_day),
            ("undo", undo),
            ("syncing", syncing),
            ("updated_next", updated_next),
            ("not_synced", not_synced),
            ("setup_needed", setup_needed),
            ("connect_google", connect_google),
            ("config_broken", config_broken),
            ("menu_sync", menu_sync),
            ("menu_autostart", menu_autostart),
            ("menu_config", menu_config),
            ("menu_reset_pos", menu_reset_pos),
            ("menu_lock", menu_lock),
            ("menu_unlock", menu_unlock),
            ("menu_log", menu_log),
            ("menu_folder", menu_folder),
            ("menu_calendars", menu_calendars),
            ("menu_tasklists", menu_tasklists),
            ("menu_copy", menu_copy),
            ("menu_update", menu_update),
            ("update_available", update_available),
            ("menu_relogin", menu_relogin),
            ("menu_quit", menu_quit),
            ("auth_connected_title", auth_connected_title),
            ("auth_connected_body", auth_connected_body),
            ("auth_cancelled_title", auth_cancelled_title),
            ("auth_waiting", auth_waiting),
            ("err_missing_client", err_missing_client),
            ("err_not_connected", err_not_connected),
            ("err_grant_expired", err_grant_expired),
            ("err_no_refresh_token", err_no_refresh_token),
            ("err_timeout", err_timeout),
            ("fatal_start", fatal_start),
        ]
    }
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
    conflicts: Plural::OneOther {
        one: "{} conflict",
        other: "{} conflicts",
    },
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
    overdue: Plural::OneOther {
        one: "{} overdue",
        other: "{} overdue",
    },
    syncing: "Syncing …",
    updated_next: "Updated {}   ·   next sync {}",
    not_synced: "Not synced yet",
    setup_needed: "Setup required — click for details",
    connect_google: "Connect to Google — click here",
    config_broken: "Invalid configuration — click to open",
    menu_sync: "Sync now",
    menu_autostart: "Start at login",
    menu_config: "Edit configuration",
    menu_reset_pos: "Reset position",
    menu_lock: "Lock position and size",
    menu_unlock: "Unlock position and size",
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
    conflicts: Plural::OneOther {
        one: "{} Überschneidung",
        other: "{} Überschneidungen",
    },
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
    overdue: Plural::OneOther {
        one: "{} überfällig",
        other: "{} überfällig",
    },
    syncing: "Synchronisiere …",
    updated_next: "Aktualisiert {}   ·   nächster Abgleich {}",
    not_synced: "Noch nicht abgeglichen",
    setup_needed: "Einrichtung nötig — klicken für Details",
    connect_google: "Mit Google verbinden — hier klicken",
    config_broken: "Konfiguration fehlerhaft — klicken zum Öffnen",
    menu_sync: "Jetzt synchronisieren",
    menu_autostart: "Beim Anmelden starten",
    menu_config: "Konfiguration bearbeiten",
    menu_reset_pos: "Position zurücksetzen",
    menu_lock: "Position und Größe sperren",
    menu_unlock: "Position und Größe entsperren",
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
    // Zero takes the singular in French: "0 conflit", not "0 conflits".
    conflicts: Plural::OneIncludingZero {
        one: "{} conflit",
        other: "{} conflits",
    },
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
    overdue: Plural::OneIncludingZero {
        one: "{} en retard",
        other: "{} en retard",
    },
    syncing: "Synchronisation …",
    updated_next: "Mis à jour {}   ·   prochaine synchro {}",
    not_synced: "Pas encore synchronisé",
    setup_needed: "Configuration requise — cliquez pour les détails",
    connect_google: "Se connecter à Google — cliquez ici",
    config_broken: "Configuration invalide — cliquez pour ouvrir",
    menu_sync: "Synchroniser maintenant",
    menu_autostart: "Démarrer à la session",
    menu_config: "Modifier la configuration",
    menu_reset_pos: "Réinitialiser la position",
    menu_lock: "Verrouiller position et taille",
    menu_unlock: "Déverrouiller position et taille",
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
    conflicts: Plural::OneOther {
        one: "{} conflicto",
        other: "{} conflictos",
    },
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
    overdue: Plural::OneOther {
        one: "{} atrasada",
        other: "{} atrasadas",
    },
    syncing: "Sincronizando …",
    updated_next: "Actualizado {}   ·   próxima sinc. {}",
    not_synced: "Aún sin sincronizar",
    setup_needed: "Configuración necesaria — haz clic para ver detalles",
    connect_google: "Conectar con Google — haz clic aquí",
    config_broken: "Configuración no válida — haz clic para abrir",
    menu_sync: "Sincronizar ahora",
    menu_autostart: "Iniciar al iniciar sesión",
    menu_config: "Editar configuración",
    menu_reset_pos: "Restablecer posición",
    menu_lock: "Bloquear posición y tamaño",
    menu_unlock: "Desbloquear posición y tamaño",
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
    conflicts: Plural::OneOther {
        one: "{} conflitto",
        other: "{} conflitti",
    },
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
    overdue: Plural::OneOther {
        one: "{} in ritardo",
        other: "{} in ritardo",
    },
    syncing: "Sincronizzazione …",
    updated_next: "Aggiornato {}   ·   prossima sincr. {}",
    not_synced: "Non ancora sincronizzato",
    setup_needed: "Configurazione necessaria — clicca per i dettagli",
    connect_google: "Connetti a Google — clicca qui",
    config_broken: "Configurazione non valida — clicca per aprire",
    menu_sync: "Sincronizza ora",
    menu_autostart: "Avvia all'accesso",
    menu_config: "Modifica configurazione",
    menu_reset_pos: "Reimposta posizione",
    menu_lock: "Blocca posizione e dimensioni",
    menu_unlock: "Sblocca posizione e dimensioni",
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

/// European Portuguese. Brazilian Portuguese is [`PT_BR`]: the two differ in
/// more than spelling here — the progressive ("a sincronizar" against
/// "sincronizando") and half the menu vocabulary are not shared.
pub const PT: Catalog = Catalog {
    code: "pt",
    rtl: false,
    section_events: "AGENDA",
    section_tasks: "TAREFAS",
    no_events: "Sem eventos hoje",
    no_tasks: "Nada para hoje",
    all_day: "dia todo",
    today: "hoje",
    yesterday: "ontem",
    tomorrow: "Amanhã",
    conflicts: Plural::OneOther {
        one: "{} sobreposição",
        other: "{} sobreposições",
    },
    now_label: "AGORA",
    next_label: "A SEGUIR",
    running: "a decorrer",
    in_pattern: "em {}",
    ago_pattern: "há {}",
    left_pattern: "faltam {}",
    just_now: "agora",
    unit_min: "min",
    unit_hour: "h",
    unit_day: "d",
    undo: "Anular",
    overdue: Plural::OneOther {
        one: "{} em atraso",
        other: "{} em atraso",
    },
    syncing: "A sincronizar …",
    updated_next: "Atualizado {}   ·   próxima sincr. {}",
    not_synced: "Ainda não sincronizado",
    setup_needed: "Configuração necessária — clique para detalhes",
    connect_google: "Ligar ao Google — clique aqui",
    config_broken: "Configuração inválida — clique para abrir",
    menu_sync: "Sincronizar agora",
    menu_autostart: "Iniciar ao iniciar sessão",
    menu_config: "Editar configuração",
    menu_reset_pos: "Repor posição",
    menu_lock: "Bloquear posição e tamanho",
    menu_unlock: "Desbloquear posição e tamanho",
    menu_log: "Abrir registo",
    menu_folder: "Abrir pasta de dados",
    menu_calendars: "Calendários",
    menu_tasklists: "Listas de tarefas",
    menu_copy: "Copiar agenda",
    menu_update: "Instalar atualização",
    update_available: "Versão {} disponível — clique para atualizar",
    menu_relogin: "Iniciar sessão no Google novamente",
    menu_quit: "Sair",
    auth_connected_title: "O TPMPlaner está ligado.",
    auth_connected_body: "Já pode fechar esta janela.",
    auth_cancelled_title: "Início de sessão cancelado.",
    auth_waiting: "À espera do início de sessão do Google …",
    err_missing_client: "Falta o client_secret.json. Crie um cliente OAuth (aplicação de ambiente de trabalho) na Google Cloud Console e coloque o ficheiro aqui:",
    err_not_connected: "Ainda não ligado ao Google.",
    err_grant_expired: "O Google revogou o acesso ou este expirou — inicie sessão novamente (clique com o botão direito no widget). Causa mais frequente: o cliente OAuth ainda está no estado «Testing», onde os tokens de atualização expiram ao fim de 7 dias.",
    err_no_refresh_token: "O Google não devolveu um token de atualização. Remova o acesso em myaccount.google.com/permissions e inicie sessão novamente.",
    err_timeout: "Tempo esgotado ao iniciar sessão.",
    fatal_start: "Não foi possível iniciar o TPMPlaner.",
};

/// Brazilian Portuguese. Reached by `pt-BR`; plain `pt` gets [`PT`].
pub const PT_BR: Catalog = Catalog {
    code: "pt-BR",
    rtl: false,
    section_events: "AGENDA",
    section_tasks: "TAREFAS",
    no_events: "Sem eventos hoje",
    no_tasks: "Nada para hoje",
    all_day: "dia todo",
    today: "hoje",
    yesterday: "ontem",
    tomorrow: "Amanhã",
    // Brazilian Portuguese puts zero in the singular; European Portuguese
    // does not. Same language, different rule — hence two catalogues.
    conflicts: Plural::OneIncludingZero {
        one: "{} conflito",
        other: "{} conflitos",
    },
    now_label: "AGORA",
    next_label: "A SEGUIR",
    running: "em andamento",
    in_pattern: "em {}",
    ago_pattern: "há {}",
    left_pattern: "faltam {}",
    just_now: "agora",
    unit_min: "min",
    unit_hour: "h",
    unit_day: "d",
    undo: "Desfazer",
    overdue: Plural::OneIncludingZero {
        one: "{} atrasada",
        other: "{} atrasadas",
    },
    syncing: "Sincronizando …",
    updated_next: "Atualizado {}   ·   próxima sincr. {}",
    not_synced: "Ainda não sincronizado",
    setup_needed: "Configuração necessária — clique para detalhes",
    connect_google: "Conectar ao Google — clique aqui",
    config_broken: "Configuração inválida — clique para abrir",
    menu_sync: "Sincronizar agora",
    menu_autostart: "Iniciar ao fazer login",
    menu_config: "Editar configuração",
    menu_reset_pos: "Redefinir posição",
    menu_lock: "Bloquear posição e tamanho",
    menu_unlock: "Desbloquear posição e tamanho",
    menu_log: "Abrir registro",
    menu_folder: "Abrir pasta de dados",
    menu_calendars: "Agendas",
    menu_tasklists: "Listas de tarefas",
    menu_copy: "Copiar agenda",
    menu_update: "Instalar atualização",
    update_available: "Versão {} disponível — clique para atualizar",
    menu_relogin: "Entrar novamente no Google",
    menu_quit: "Sair",
    auth_connected_title: "O TPMPlaner está conectado.",
    auth_connected_body: "Você já pode fechar esta janela.",
    auth_cancelled_title: "Login cancelado.",
    auth_waiting: "Aguardando o login do Google …",
    err_missing_client: "Falta o client_secret.json. Crie um cliente OAuth (aplicativo para computador) no Google Cloud Console e coloque o arquivo aqui:",
    err_not_connected: "Ainda não conectado ao Google.",
    err_grant_expired: "O Google revogou o acesso ou ele expirou — faça login novamente (clique com o botão direito no widget). Causa mais comum: o cliente OAuth ainda está no estado «Testing», onde os tokens de atualização expiram após 7 dias.",
    err_no_refresh_token: "O Google não retornou um token de atualização. Remova o acesso em myaccount.google.com/permissions e faça login novamente.",
    err_timeout: "Tempo esgotado ao fazer login.",
    fatal_start: "Não foi possível iniciar o TPMPlaner.",
};

pub const NL: Catalog = Catalog {
    code: "nl",
    rtl: false,
    section_events: "AGENDA",
    section_tasks: "TAKEN",
    no_events: "Geen afspraken vandaag",
    no_tasks: "Niets te doen vandaag",
    all_day: "hele dag",
    today: "vandaag",
    yesterday: "gisteren",
    tomorrow: "Morgen",
    conflicts: Plural::OneOther {
        one: "{} overlap",
        other: "{} overlappingen",
    },
    now_label: "NU",
    next_label: "HIERNA",
    running: "bezig",
    in_pattern: "over {}",
    ago_pattern: "{} geleden",
    left_pattern: "nog {}",
    just_now: "nu",
    unit_min: "min",
    unit_hour: "u",
    unit_day: "d",
    undo: "Ongedaan maken",
    overdue: Plural::OneOther {
        one: "{} te laat",
        other: "{} te laat",
    },
    syncing: "Synchroniseren …",
    updated_next: "Bijgewerkt {}   ·   volgende sync {}",
    not_synced: "Nog niet gesynchroniseerd",
    setup_needed: "Instellen vereist — klik voor details",
    connect_google: "Verbinden met Google — klik hier",
    config_broken: "Ongeldige configuratie — klik om te openen",
    menu_sync: "Nu synchroniseren",
    menu_autostart: "Starten bij aanmelden",
    menu_config: "Configuratie bewerken",
    menu_reset_pos: "Positie herstellen",
    menu_lock: "Positie en grootte vergrendelen",
    menu_unlock: "Positie en grootte ontgrendelen",
    menu_log: "Logboek openen",
    menu_folder: "Gegevensmap openen",
    menu_calendars: "Agenda's",
    menu_tasklists: "Takenlijsten",
    menu_copy: "Agenda kopiëren",
    menu_update: "Update installeren",
    update_available: "Versie {} beschikbaar — klik om bij te werken",
    menu_relogin: "Opnieuw aanmelden bij Google",
    menu_quit: "Afsluiten",
    auth_connected_title: "TPMPlaner is verbonden.",
    auth_connected_body: "U kunt dit venster nu sluiten.",
    auth_cancelled_title: "Aanmelden geannuleerd.",
    auth_waiting: "Wachten op de Google-aanmelding …",
    err_missing_client: "client_secret.json ontbreekt. Maak een OAuth-client (desktop-app) aan in de Google Cloud Console en plaats het bestand hier:",
    err_not_connected: "Nog niet verbonden met Google.",
    err_grant_expired: "Google heeft de toegang ingetrokken of laten verlopen — meld u opnieuw aan (rechtsklik op de widget). Meest voorkomende oorzaak: de OAuth-client staat nog op «Testing», waar vernieuwingstokens na 7 dagen verlopen.",
    err_no_refresh_token: "Google heeft geen vernieuwingstoken teruggegeven. Verwijder de toegang op myaccount.google.com/permissions en meld u opnieuw aan.",
    err_timeout: "Time-out bij het aanmelden.",
    fatal_start: "TPMPlaner kon niet worden gestart.",
};

pub const SV: Catalog = Catalog {
    code: "sv",
    rtl: false,
    section_events: "SCHEMA",
    section_tasks: "UPPGIFTER",
    no_events: "Inga händelser i dag",
    no_tasks: "Inget att göra i dag",
    all_day: "heldag",
    today: "i dag",
    yesterday: "i går",
    tomorrow: "I morgon",
    conflicts: Plural::OneOther {
        one: "{} krock",
        other: "{} krockar",
    },
    now_label: "NU",
    next_label: "NÄST",
    running: "pågår",
    in_pattern: "om {}",
    ago_pattern: "för {} sedan",
    left_pattern: "{} kvar",
    just_now: "nu",
    unit_min: "min",
    unit_hour: "tim",
    unit_day: "d",
    undo: "Ångra",
    overdue: Plural::OneOther {
        one: "{} försenad",
        other: "{} försenade",
    },
    syncing: "Synkroniserar …",
    updated_next: "Uppdaterad {}   ·   nästa synk {}",
    not_synced: "Inte synkroniserad än",
    setup_needed: "Konfiguration krävs — klicka för detaljer",
    connect_google: "Anslut till Google — klicka här",
    config_broken: "Ogiltig konfiguration — klicka för att öppna",
    menu_sync: "Synkronisera nu",
    menu_autostart: "Starta vid inloggning",
    menu_config: "Redigera konfiguration",
    menu_reset_pos: "Återställ position",
    menu_lock: "Lås position och storlek",
    menu_unlock: "Lås upp position och storlek",
    menu_log: "Öppna logg",
    menu_folder: "Öppna datamapp",
    menu_calendars: "Kalendrar",
    menu_tasklists: "Uppgiftslistor",
    menu_copy: "Kopiera schema",
    menu_update: "Installera uppdatering",
    update_available: "Version {} finns — klicka för att uppdatera",
    menu_relogin: "Logga in på Google igen",
    menu_quit: "Avsluta",
    auth_connected_title: "TPMPlaner är ansluten.",
    auth_connected_body: "Du kan stänga det här fönstret nu.",
    auth_cancelled_title: "Inloggningen avbröts.",
    auth_waiting: "Väntar på Google-inloggningen …",
    err_missing_client: "client_secret.json saknas. Skapa en OAuth-klient (skrivbordsapp) i Google Cloud Console och lägg filen här:",
    err_not_connected: "Inte ansluten till Google än.",
    err_grant_expired: "Google har återkallat åtkomsten eller låtit den gå ut — logga in igen (högerklicka på widgeten). Vanligaste orsaken: OAuth-klienten står fortfarande på «Testing», där uppdateringstoken går ut efter 7 dagar.",
    err_no_refresh_token: "Google returnerade ingen uppdateringstoken. Ta bort åtkomsten på myaccount.google.com/permissions och logga in igen.",
    err_timeout: "Tidsgränsen för inloggning överskreds.",
    fatal_start: "TPMPlaner kunde inte startas.",
};

/// Polish. Three plural forms, and not the Russian ones — see
/// [`Plural::Polish`].
pub const PL: Catalog = Catalog {
    code: "pl",
    rtl: false,
    section_events: "PLAN DNIA",
    section_tasks: "ZADANIA",
    no_events: "Brak wydarzeń na dziś",
    no_tasks: "Nic na dziś",
    all_day: "cały dzień",
    today: "dziś",
    yesterday: "wczoraj",
    tomorrow: "Jutro",
    conflicts: Plural::Polish {
        one: "{} kolizja",
        few: "{} kolizje",
        many: "{} kolizji",
    },
    now_label: "TERAZ",
    next_label: "NASTĘPNIE",
    running: "trwa",
    in_pattern: "za {}",
    ago_pattern: "{} temu",
    left_pattern: "jeszcze {}",
    just_now: "teraz",
    unit_min: "min",
    unit_hour: "godz",
    unit_day: "dn",
    undo: "Cofnij",
    overdue: Plural::Polish {
        one: "{} zaległe",
        few: "{} zaległe",
        many: "{} zaległych",
    },
    syncing: "Synchronizacja …",
    updated_next: "Zaktualizowano {}   ·   następna synchr. {}",
    not_synced: "Jeszcze nie zsynchronizowano",
    setup_needed: "Wymagana konfiguracja — kliknij po szczegóły",
    connect_google: "Połącz z Google — kliknij tutaj",
    config_broken: "Nieprawidłowa konfiguracja — kliknij, aby otworzyć",
    menu_sync: "Synchronizuj teraz",
    menu_autostart: "Uruchamiaj po zalogowaniu",
    menu_config: "Edytuj konfigurację",
    menu_reset_pos: "Resetuj pozycję",
    menu_lock: "Zablokuj pozycję i rozmiar",
    menu_unlock: "Odblokuj pozycję i rozmiar",
    menu_log: "Otwórz dziennik",
    menu_folder: "Otwórz folder danych",
    menu_calendars: "Kalendarze",
    menu_tasklists: "Listy zadań",
    menu_copy: "Kopiuj plan dnia",
    menu_update: "Zainstaluj aktualizację",
    update_available: "Dostępna wersja {} — kliknij, aby zaktualizować",
    menu_relogin: "Zaloguj się ponownie do Google",
    menu_quit: "Zakończ",
    auth_connected_title: "TPMPlaner jest połączony.",
    auth_connected_body: "Możesz już zamknąć to okno.",
    auth_cancelled_title: "Logowanie anulowane.",
    auth_waiting: "Oczekiwanie na logowanie Google …",
    err_missing_client: "Brak pliku client_secret.json. Utwórz klienta OAuth (aplikacja komputerowa) w Google Cloud Console i umieść plik tutaj:",
    err_not_connected: "Brak połączenia z Google.",
    err_grant_expired: "Google cofnął dostęp lub dostęp wygasł — zaloguj się ponownie (kliknij widżet prawym przyciskiem). Najczęstsza przyczyna: klient OAuth ma nadal status «Testing», w którym tokeny odświeżania wygasają po 7 dniach.",
    err_no_refresh_token: "Google nie zwrócił tokenu odświeżania. Usuń dostęp na myaccount.google.com/permissions i zaloguj się ponownie.",
    err_timeout: "Przekroczono czas logowania.",
    fatal_start: "Nie można uruchomić TPMPlanera.",
};

pub const CS: Catalog = Catalog {
    code: "cs",
    rtl: false,
    section_events: "PROGRAM",
    section_tasks: "ÚKOLY",
    no_events: "Dnes žádné události",
    no_tasks: "Na dnešek nic",
    all_day: "celý den",
    today: "dnes",
    yesterday: "včera",
    tomorrow: "Zítra",
    conflicts: Plural::Czech {
        one: "{} kolize",
        few: "{} kolize",
        other: "{} kolizí",
    },
    now_label: "TEĎ",
    next_label: "DALŠÍ",
    running: "probíhá",
    in_pattern: "za {}",
    ago_pattern: "před {}",
    left_pattern: "ještě {}",
    just_now: "teď",
    unit_min: "min",
    unit_hour: "h",
    unit_day: "d",
    undo: "Zpět",
    overdue: Plural::Czech {
        one: "{} po termínu",
        few: "{} po termínu",
        other: "{} po termínu",
    },
    syncing: "Synchronizace …",
    updated_next: "Aktualizováno {}   ·   další synchr. {}",
    not_synced: "Zatím nesynchronizováno",
    setup_needed: "Je potřeba nastavení — klikněte pro podrobnosti",
    connect_google: "Připojit ke Google — klikněte sem",
    config_broken: "Neplatná konfigurace — klikněte pro otevření",
    menu_sync: "Synchronizovat nyní",
    menu_autostart: "Spouštět po přihlášení",
    menu_config: "Upravit konfiguraci",
    menu_reset_pos: "Obnovit pozici",
    menu_lock: "Uzamknout pozici a velikost",
    menu_unlock: "Odemknout pozici a velikost",
    menu_log: "Otevřít protokol",
    menu_folder: "Otevřít složku s daty",
    menu_calendars: "Kalendáře",
    menu_tasklists: "Seznamy úkolů",
    menu_copy: "Kopírovat program",
    menu_update: "Nainstalovat aktualizaci",
    update_available: "K dispozici je verze {} — klikněte pro aktualizaci",
    menu_relogin: "Znovu se přihlásit ke Google",
    menu_quit: "Ukončit",
    auth_connected_title: "TPMPlaner je připojen.",
    auth_connected_body: "Toto okno můžete zavřít.",
    auth_cancelled_title: "Přihlášení zrušeno.",
    auth_waiting: "Čekání na přihlášení ke Google …",
    err_missing_client: "Chybí client_secret.json. Vytvořte v Google Cloud Console klienta OAuth (desktopová aplikace) a soubor umístěte sem:",
    err_not_connected: "Zatím nepřipojeno ke Google.",
    err_grant_expired: "Google přístup odvolal nebo jeho platnost vypršela — přihlaste se prosím znovu (klikněte pravým tlačítkem na widget). Nejčastější příčina: klient OAuth je stále ve stavu «Testing», kde obnovovací tokeny vyprší po 7 dnech.",
    err_no_refresh_token: "Google nevrátil obnovovací token. Odeberte přístup na myaccount.google.com/permissions a přihlaste se znovu.",
    err_timeout: "Vypršel časový limit přihlášení.",
    fatal_start: "TPMPlaner se nepodařilo spustit.",
};

/// Turkish. A numeral leaves the noun in the singular, so every counted string
/// has exactly one form — see [`Plural::Invariant`].
pub const TR: Catalog = Catalog {
    code: "tr",
    rtl: false,
    section_events: "PROGRAM",
    section_tasks: "GÖREVLER",
    no_events: "Bugün etkinlik yok",
    no_tasks: "Bugün için bir şey yok",
    all_day: "tüm gün",
    today: "bugün",
    yesterday: "dün",
    tomorrow: "Yarın",
    conflicts: Plural::Invariant("{} çakışma"),
    now_label: "ŞİMDİ",
    next_label: "SIRADAKİ",
    running: "sürüyor",
    in_pattern: "{} sonra",
    ago_pattern: "{} önce",
    left_pattern: "{} kaldı",
    just_now: "şimdi",
    unit_min: "dk",
    unit_hour: "sa",
    unit_day: "g",
    undo: "Geri al",
    overdue: Plural::Invariant("{} gecikmiş"),
    syncing: "Eşitleniyor …",
    updated_next: "Güncellendi {}   ·   sonraki eşitleme {}",
    not_synced: "Henüz eşitlenmedi",
    setup_needed: "Kurulum gerekiyor — ayrıntılar için tıklayın",
    connect_google: "Google'a bağlan — buraya tıklayın",
    config_broken: "Geçersiz yapılandırma — açmak için tıklayın",
    menu_sync: "Şimdi eşitle",
    menu_autostart: "Oturum açılışında başlat",
    menu_config: "Yapılandırmayı düzenle",
    menu_reset_pos: "Konumu sıfırla",
    menu_lock: "Konumu ve boyutu kilitle",
    menu_unlock: "Konum ve boyut kilidini aç",
    menu_log: "Günlüğü aç",
    menu_folder: "Veri klasörünü aç",
    menu_calendars: "Takvimler",
    menu_tasklists: "Görev listeleri",
    menu_copy: "Programı kopyala",
    menu_update: "Güncellemeyi yükle",
    update_available: "{} sürümü mevcut — güncellemek için tıklayın",
    menu_relogin: "Google'da yeniden oturum aç",
    menu_quit: "Çıkış",
    auth_connected_title: "TPMPlaner bağlandı.",
    auth_connected_body: "Bu pencereyi şimdi kapatabilirsiniz.",
    auth_cancelled_title: "Oturum açma iptal edildi.",
    auth_waiting: "Google oturum açma bekleniyor …",
    err_missing_client: "client_secret.json eksik. Google Cloud Console'da bir OAuth istemcisi (masaüstü uygulaması) oluşturun ve dosyayı buraya koyun:",
    err_not_connected: "Henüz Google'a bağlanılmadı.",
    err_grant_expired: "Google erişimi iptal etti veya erişimin süresi doldu — lütfen yeniden oturum açın (widget'a sağ tıklayın). En sık neden: OAuth istemcisi hâlâ «Testing» durumunda; orada yenileme belirteçleri 7 gün sonra geçersiz olur.",
    err_no_refresh_token: "Google bir yenileme belirteci döndürmedi. myaccount.google.com/permissions adresinden erişimi kaldırın ve yeniden oturum açın.",
    err_timeout: "Oturum açma zaman aşımına uğradı.",
    fatal_start: "TPMPlaner başlatılamadı.",
};

pub const RU: Catalog = Catalog {
    code: "ru",
    rtl: false,
    section_events: "РАСПИСАНИЕ",
    section_tasks: "ЗАДАЧИ",
    no_events: "Сегодня событий нет",
    no_tasks: "На сегодня ничего нет",
    all_day: "весь день",
    today: "сегодня",
    yesterday: "вчера",
    tomorrow: "Завтра",
    conflicts: Plural::EastSlavic {
        one: "{} наложение",
        few: "{} наложения",
        many: "{} наложений",
    },
    now_label: "СЕЙЧАС",
    next_label: "ДАЛЕЕ",
    running: "идёт",
    in_pattern: "через {}",
    ago_pattern: "{} назад",
    left_pattern: "ещё {}",
    just_now: "сейчас",
    unit_min: "мин",
    unit_hour: "ч",
    unit_day: "дн",
    undo: "Отменить",
    overdue: Plural::EastSlavic {
        one: "{} просрочена",
        few: "{} просрочены",
        many: "{} просрочено",
    },
    syncing: "Синхронизация …",
    updated_next: "Обновлено {}   ·   следующая синхр. {}",
    not_synced: "Ещё не синхронизировано",
    setup_needed: "Требуется настройка — нажмите для подробностей",
    connect_google: "Подключиться к Google — нажмите здесь",
    config_broken: "Неверная конфигурация — нажмите, чтобы открыть",
    menu_sync: "Синхронизировать сейчас",
    menu_autostart: "Запускать при входе в систему",
    menu_config: "Изменить конфигурацию",
    menu_reset_pos: "Сбросить положение",
    menu_lock: "Закрепить положение и размер",
    menu_unlock: "Открепить положение и размер",
    menu_log: "Открыть журнал",
    menu_folder: "Открыть папку данных",
    menu_calendars: "Календари",
    menu_tasklists: "Списки задач",
    menu_copy: "Скопировать расписание",
    menu_update: "Установить обновление",
    update_available: "Доступна версия {} — нажмите, чтобы обновить",
    menu_relogin: "Войти в Google заново",
    menu_quit: "Выход",
    auth_connected_title: "TPMPlaner подключён.",
    auth_connected_body: "Это окно можно закрыть.",
    auth_cancelled_title: "Вход отменён.",
    auth_waiting: "Ожидание входа в Google …",
    err_missing_client: "Файл client_secret.json отсутствует. Создайте клиент OAuth (приложение для компьютера) в Google Cloud Console и поместите файл сюда:",
    err_not_connected: "Ещё не подключено к Google.",
    err_grant_expired: "Google отозвал доступ или срок его действия истёк — войдите снова (правый щелчок по виджету). Самая частая причина: клиент OAuth всё ещё в статусе «Testing», где токены обновления действуют 7 дней.",
    err_no_refresh_token: "Google не вернул токен обновления. Удалите доступ на myaccount.google.com/permissions и войдите снова.",
    err_timeout: "Время ожидания входа истекло.",
    fatal_start: "Не удалось запустить TPMPlaner.",
};

pub const UK: Catalog = Catalog {
    code: "uk",
    rtl: false,
    section_events: "РОЗКЛАД",
    section_tasks: "ЗАВДАННЯ",
    no_events: "Сьогодні подій немає",
    no_tasks: "На сьогодні нічого немає",
    all_day: "весь день",
    today: "сьогодні",
    yesterday: "учора",
    tomorrow: "Завтра",
    conflicts: Plural::EastSlavic {
        one: "{} накладання",
        few: "{} накладання",
        many: "{} накладань",
    },
    now_label: "ЗАРАЗ",
    next_label: "ДАЛІ",
    running: "триває",
    in_pattern: "через {}",
    ago_pattern: "{} тому",
    left_pattern: "ще {}",
    just_now: "зараз",
    unit_min: "хв",
    unit_hour: "год",
    unit_day: "дн",
    undo: "Скасувати",
    overdue: Plural::EastSlavic {
        one: "{} прострочена",
        few: "{} прострочені",
        many: "{} прострочених",
    },
    syncing: "Синхронізація …",
    updated_next: "Оновлено {}   ·   наступна синхр. {}",
    not_synced: "Ще не синхронізовано",
    setup_needed: "Потрібне налаштування — натисніть для подробиць",
    connect_google: "Підключитися до Google — натисніть тут",
    config_broken: "Некоректна конфігурація — натисніть, щоб відкрити",
    menu_sync: "Синхронізувати зараз",
    menu_autostart: "Запускати після входу",
    menu_config: "Редагувати конфігурацію",
    menu_reset_pos: "Скинути позицію",
    menu_lock: "Закріпити позицію та розмір",
    menu_unlock: "Відкріпити позицію та розмір",
    menu_log: "Відкрити журнал",
    menu_folder: "Відкрити теку даних",
    menu_calendars: "Календарі",
    menu_tasklists: "Списки завдань",
    menu_copy: "Скопіювати розклад",
    menu_update: "Встановити оновлення",
    update_available: "Доступна версія {} — натисніть, щоб оновити",
    menu_relogin: "Увійти в Google знову",
    menu_quit: "Вихід",
    auth_connected_title: "TPMPlaner підключено.",
    auth_connected_body: "Це вікно можна закрити.",
    auth_cancelled_title: "Вхід скасовано.",
    auth_waiting: "Очікування входу в Google …",
    err_missing_client: "Файл client_secret.json відсутній. Створіть клієнт OAuth (застосунок для комп'ютера) у Google Cloud Console і покладіть файл сюди:",
    err_not_connected: "Ще не підключено до Google.",
    err_grant_expired: "Google відкликав доступ або строк його дії минув — увійдіть знову (клацніть віджет правою кнопкою). Найчастіша причина: клієнт OAuth досі має статус «Testing», де токени оновлення діють 7 днів.",
    err_no_refresh_token: "Google не повернув токен оновлення. Видаліть доступ на myaccount.google.com/permissions і увійдіть знову.",
    err_timeout: "Час очікування входу минув.",
    fatal_start: "Не вдалося запустити TPMPlaner.",
};

pub const JA: Catalog = Catalog {
    code: "ja",
    rtl: false,
    section_events: "予定",
    section_tasks: "タスク",
    no_events: "今日の予定はありません",
    no_tasks: "今日が期限のタスクはありません",
    all_day: "終日",
    today: "今日",
    yesterday: "昨日",
    tomorrow: "明日",
    conflicts: Plural::Invariant("{} 件重複"),
    now_label: "現在",
    next_label: "次の予定",
    running: "進行中",
    in_pattern: "{}後",
    ago_pattern: "{}前",
    left_pattern: "残り{}",
    just_now: "たった今",
    unit_min: "分",
    unit_hour: "時間",
    unit_day: "日",
    undo: "元に戻す",
    overdue: Plural::Invariant("{} 件期限切れ"),
    syncing: "同期中 …",
    updated_next: "更新 {}   ·   次回同期 {}",
    not_synced: "未同期",
    setup_needed: "設定が必要です — クリックして詳細を表示",
    connect_google: "Google に接続 — ここをクリック",
    config_broken: "設定が無効です — クリックして開く",
    menu_sync: "今すぐ同期",
    menu_autostart: "ログイン時に起動",
    menu_config: "設定を編集",
    menu_reset_pos: "位置をリセット",
    menu_lock: "位置とサイズを固定",
    menu_unlock: "位置とサイズの固定を解除",
    menu_log: "ログを開く",
    menu_folder: "データフォルダーを開く",
    menu_calendars: "カレンダー",
    menu_tasklists: "タスクリスト",
    menu_copy: "予定をコピー",
    menu_update: "更新をインストール",
    update_available: "バージョン {} が利用可能 — クリックして更新",
    menu_relogin: "Google に再ログイン",
    menu_quit: "終了",
    auth_connected_title: "TPMPlaner が接続されました。",
    auth_connected_body: "このウィンドウを閉じてかまいません。",
    auth_cancelled_title: "ログインを中止しました。",
    auth_waiting: "Google のログインを待っています …",
    err_missing_client: "client_secret.json がありません。Google Cloud Console で OAuth クライアント（デスクトップ アプリ）を作成し、ファイルをここに置いてください:",
    err_not_connected: "まだ Google に接続していません。",
    err_grant_expired: "Google がアクセスを取り消したか、有効期限が切れました — もう一度ログインしてください（ウィジェットを右クリック）。最も多い原因: OAuth クライアントが「Testing」のままで、そこではリフレッシュ トークンが 7 日で失効します。",
    err_no_refresh_token: "Google がリフレッシュ トークンを返しませんでした。myaccount.google.com/permissions でアクセスを削除し、もう一度ログインしてください。",
    err_timeout: "ログインがタイムアウトしました。",
    fatal_start: "TPMPlaner を起動できませんでした。",
};

/// Simplified Chinese. Reached by `zh`, `zh-Hans`, `zh-CN` and `zh-SG`; the
/// traditional catalogue is [`ZH_HANT`].
pub const ZH_HANS: Catalog = Catalog {
    code: "zh-Hans",
    rtl: false,
    section_events: "日程",
    section_tasks: "任务",
    no_events: "今天没有日程",
    no_tasks: "今天没有待办",
    all_day: "全天",
    today: "今天",
    yesterday: "昨天",
    tomorrow: "明天",
    conflicts: Plural::Invariant("{} 个冲突"),
    now_label: "现在",
    next_label: "接下来",
    running: "进行中",
    in_pattern: "{}后",
    ago_pattern: "{}前",
    left_pattern: "还剩 {}",
    just_now: "刚刚",
    unit_min: "分钟",
    unit_hour: "小时",
    unit_day: "天",
    undo: "撤销",
    overdue: Plural::Invariant("{} 项逾期"),
    syncing: "正在同步 …",
    updated_next: "已更新 {}   ·   下次同步 {}",
    not_synced: "尚未同步",
    setup_needed: "需要设置 — 点击查看详情",
    connect_google: "连接 Google — 点击这里",
    config_broken: "配置无效 — 点击打开",
    menu_sync: "立即同步",
    menu_autostart: "登录时启动",
    menu_config: "编辑配置",
    menu_reset_pos: "重置位置",
    menu_lock: "锁定位置和大小",
    menu_unlock: "解锁位置和大小",
    menu_log: "打开日志",
    menu_folder: "打开数据文件夹",
    menu_calendars: "日历",
    menu_tasklists: "任务列表",
    menu_copy: "复制日程",
    menu_update: "安装更新",
    update_available: "有新版本 {} — 点击更新",
    menu_relogin: "重新登录 Google",
    menu_quit: "退出",
    auth_connected_title: "TPMPlaner 已连接。",
    auth_connected_body: "现在可以关闭此窗口。",
    auth_cancelled_title: "登录已取消。",
    auth_waiting: "正在等待 Google 登录 …",
    err_missing_client: "缺少 client_secret.json。请在 Google Cloud Console 中创建 OAuth 客户端（桌面应用），并将文件放在这里：",
    err_not_connected: "尚未连接到 Google。",
    err_grant_expired: "Google 已撤销访问权限，或权限已过期 — 请重新登录（右键点击小组件）。最常见的原因：OAuth 客户端仍处于「Testing」状态，刷新令牌会在 7 天后失效。",
    err_no_refresh_token: "Google 未返回刷新令牌。请在 myaccount.google.com/permissions 移除访问权限后重新登录。",
    err_timeout: "登录超时。",
    fatal_start: "无法启动 TPMPlaner。",
};

/// Traditional Chinese. Reached by `zh-Hant` and by the regions that use it —
/// `zh-TW`, `zh-HK`, `zh-MO`.
pub const ZH_HANT: Catalog = Catalog {
    code: "zh-Hant",
    rtl: false,
    section_events: "行程",
    section_tasks: "待辦事項",
    no_events: "今天沒有行程",
    no_tasks: "今天沒有待辦事項",
    all_day: "整天",
    today: "今天",
    yesterday: "昨天",
    tomorrow: "明天",
    conflicts: Plural::Invariant("{} 個衝突"),
    now_label: "現在",
    next_label: "接下來",
    running: "進行中",
    in_pattern: "{}後",
    ago_pattern: "{}前",
    left_pattern: "還剩 {}",
    just_now: "剛剛",
    unit_min: "分鐘",
    unit_hour: "小時",
    unit_day: "天",
    undo: "復原",
    overdue: Plural::Invariant("{} 項逾期"),
    syncing: "同步中 …",
    updated_next: "已更新 {}   ·   下次同步 {}",
    not_synced: "尚未同步",
    setup_needed: "需要設定 — 點擊查看詳細資料",
    connect_google: "連線至 Google — 點擊這裡",
    config_broken: "設定無效 — 點擊開啟",
    menu_sync: "立即同步",
    menu_autostart: "登入時啟動",
    menu_config: "編輯設定",
    menu_reset_pos: "重設位置",
    menu_lock: "鎖定位置與大小",
    menu_unlock: "解鎖位置與大小",
    menu_log: "開啟記錄",
    menu_folder: "開啟資料資料夾",
    menu_calendars: "日曆",
    menu_tasklists: "工作清單",
    menu_copy: "複製行程",
    menu_update: "安裝更新",
    update_available: "有新版本 {} — 點擊更新",
    menu_relogin: "重新登入 Google",
    menu_quit: "結束",
    auth_connected_title: "TPMPlaner 已連線。",
    auth_connected_body: "現在可以關閉此視窗。",
    auth_cancelled_title: "已取消登入。",
    auth_waiting: "正在等待 Google 登入 …",
    err_missing_client: "找不到 client_secret.json。請在 Google Cloud Console 中建立 OAuth 用戶端（電腦版應用程式），並將檔案放在這裡：",
    err_not_connected: "尚未連線至 Google。",
    err_grant_expired: "Google 已撤銷存取權，或存取權已過期 — 請重新登入（在小工具上按右鍵）。最常見的原因：OAuth 用戶端仍處於「Testing」狀態，更新權杖會在 7 天後失效。",
    err_no_refresh_token: "Google 未傳回更新權杖。請在 myaccount.google.com/permissions 移除存取權後重新登入。",
    err_timeout: "登入逾時。",
    fatal_start: "無法啟動 TPMPlaner。",
};

pub const KO: Catalog = Catalog {
    code: "ko",
    rtl: false,
    section_events: "일정",
    section_tasks: "할 일",
    no_events: "오늘 일정이 없습니다",
    no_tasks: "오늘 마감할 일이 없습니다",
    all_day: "종일",
    today: "오늘",
    yesterday: "어제",
    tomorrow: "내일",
    conflicts: Plural::Invariant("겹치는 일정 {}개"),
    now_label: "지금",
    next_label: "다음 일정",
    running: "진행 중",
    in_pattern: "{} 후",
    ago_pattern: "{} 전",
    left_pattern: "{} 남음",
    just_now: "지금",
    unit_min: "분",
    unit_hour: "시간",
    unit_day: "일",
    undo: "실행 취소",
    overdue: Plural::Invariant("{}개 지연"),
    syncing: "동기화 중 …",
    updated_next: "업데이트 {}   ·   다음 동기화 {}",
    not_synced: "아직 동기화되지 않음",
    setup_needed: "설정이 필요합니다 — 자세히 보려면 클릭",
    connect_google: "Google에 연결 — 여기를 클릭",
    config_broken: "구성이 잘못되었습니다 — 클릭하여 열기",
    menu_sync: "지금 동기화",
    menu_autostart: "로그인 시 시작",
    menu_config: "구성 편집",
    menu_reset_pos: "위치 초기화",
    menu_lock: "위치 및 크기 잠금",
    menu_unlock: "위치 및 크기 잠금 해제",
    menu_log: "로그 열기",
    menu_folder: "데이터 폴더 열기",
    menu_calendars: "캘린더",
    menu_tasklists: "할 일 목록",
    menu_copy: "일정 복사",
    menu_update: "업데이트 설치",
    update_available: "버전 {} 사용 가능 — 클릭하여 업데이트",
    menu_relogin: "Google에 다시 로그인",
    menu_quit: "종료",
    auth_connected_title: "TPMPlaner가 연결되었습니다.",
    auth_connected_body: "이제 이 창을 닫아도 됩니다.",
    auth_cancelled_title: "로그인이 취소되었습니다.",
    auth_waiting: "Google 로그인을 기다리는 중 …",
    err_missing_client: "client_secret.json이 없습니다. Google Cloud Console에서 OAuth 클라이언트(데스크톱 앱)를 만들고 파일을 여기에 두세요:",
    err_not_connected: "아직 Google에 연결되지 않았습니다.",
    err_grant_expired: "Google이 액세스를 취소했거나 액세스가 만료되었습니다 — 다시 로그인하세요(위젯을 마우스 오른쪽 버튼으로 클릭). 가장 흔한 원인: OAuth 클라이언트가 아직 「Testing」 상태이며, 이 경우 갱신 토큰은 7일 후 만료됩니다.",
    err_no_refresh_token: "Google이 갱신 토큰을 반환하지 않았습니다. myaccount.google.com/permissions에서 액세스를 제거한 뒤 다시 로그인하세요.",
    err_timeout: "로그인 시간이 초과되었습니다.",
    fatal_start: "TPMPlaner를 시작할 수 없습니다.",
};

/// Arabic. The first catalogue with `rtl: true`, so this is where the layout
/// mirroring and the text finally agree — see [`Catalog::rtl`].
pub const AR: Catalog = Catalog {
    code: "ar",
    rtl: true,
    section_events: "المواعيد",
    section_tasks: "المهام",
    no_events: "لا مواعيد اليوم",
    no_tasks: "لا مهام مستحقة اليوم",
    all_day: "طوال اليوم",
    today: "اليوم",
    yesterday: "أمس",
    tomorrow: "غدًا",
    conflicts: Plural::Arabic {
        zero: "{} تعارضات",
        one: "تعارض واحد",
        two: "تعارضان",
        few: "{} تعارضات",
        many: "{} تعارضًا",
        other: "{} تعارض",
    },
    now_label: "الآن",
    next_label: "التالي",
    running: "جارٍ",
    in_pattern: "خلال {}",
    ago_pattern: "قبل {}",
    left_pattern: "يتبقى {}",
    just_now: "الآن",
    unit_min: "دق",
    unit_hour: "س",
    unit_day: "ي",
    undo: "تراجع",
    overdue: Plural::Arabic {
        zero: "{} مهمة متأخرة",
        one: "مهمة متأخرة",
        two: "مهمتان متأخرتان",
        few: "{} مهام متأخرة",
        many: "{} مهمة متأخرة",
        other: "{} مهمة متأخرة",
    },
    syncing: "جارٍ المزامنة …",
    updated_next: "آخر تحديث {}   ·   المزامنة التالية {}",
    not_synced: "لم تتم المزامنة بعد",
    setup_needed: "الإعداد مطلوب — انقر للتفاصيل",
    connect_google: "الاتصال بـ Google — انقر هنا",
    config_broken: "إعدادات غير صالحة — انقر للفتح",
    menu_sync: "مزامنة الآن",
    menu_autostart: "التشغيل عند تسجيل الدخول",
    menu_config: "تحرير الإعدادات",
    menu_reset_pos: "إعادة تعيين الموضع",
    menu_lock: "قفل الموضع والحجم",
    menu_unlock: "إلغاء قفل الموضع والحجم",
    menu_log: "فتح السجل",
    menu_folder: "فتح مجلد البيانات",
    menu_calendars: "التقويمات",
    menu_tasklists: "قوائم المهام",
    menu_copy: "نسخ جدول اليوم",
    menu_update: "تثبيت التحديث",
    update_available: "الإصدار {} متاح — انقر للتحديث",
    menu_relogin: "تسجيل الدخول إلى Google مرة أخرى",
    menu_quit: "إنهاء",
    auth_connected_title: "تم توصيل TPMPlaner.",
    auth_connected_body: "يمكنك إغلاق هذه النافذة الآن.",
    auth_cancelled_title: "تم إلغاء تسجيل الدخول.",
    auth_waiting: "في انتظار تسجيل الدخول إلى Google …",
    err_missing_client: "الملف client_secret.json غير موجود. أنشئ عميل OAuth (تطبيق سطح مكتب) في Google Cloud Console وضع الملف هنا:",
    err_not_connected: "لم يتم الاتصال بـ Google بعد.",
    err_grant_expired: "ألغى Google الوصول أو انتهت صلاحيته — يرجى تسجيل الدخول مرة أخرى (انقر بزر الفأرة الأيمن على الأداة). السبب الأكثر شيوعًا: عميل OAuth ما زال في حالة «Testing»، حيث تنتهي صلاحية رموز التحديث بعد 7 أيام.",
    err_no_refresh_token: "لم يُرجع Google رمز تحديث. أزل الوصول من myaccount.google.com/permissions ثم سجّل الدخول مرة أخرى.",
    err_timeout: "انتهت مهلة تسجيل الدخول.",
    fatal_start: "تعذّر بدء تشغيل TPMPlaner.",
};

/// Hebrew. Reached by `he` and by the superseded ISO code `iw`, which some
/// systems still report.
pub const HE: Catalog = Catalog {
    code: "he",
    rtl: true,
    section_events: "לוח זמנים",
    section_tasks: "משימות",
    no_events: "אין אירועים היום",
    no_tasks: "אין משימות להיום",
    all_day: "כל היום",
    today: "היום",
    yesterday: "אתמול",
    tomorrow: "מחר",
    conflicts: Plural::Hebrew {
        one: "חפיפה אחת",
        two: "שתי חפיפות",
        other: "{} חפיפות",
    },
    now_label: "עכשיו",
    next_label: "הבא",
    running: "מתקיים",
    in_pattern: "בעוד {}",
    ago_pattern: "לפני {}",
    left_pattern: "נותרו {}",
    just_now: "עכשיו",
    unit_min: "דק'",
    unit_hour: "ש'",
    unit_day: "י'",
    undo: "בטל",
    overdue: Plural::Hebrew {
        one: "משימה אחת באיחור",
        two: "שתי משימות באיחור",
        other: "{} משימות באיחור",
    },
    syncing: "מסנכרן …",
    updated_next: "עודכן {}   ·   סנכרון הבא {}",
    not_synced: "עדיין לא סונכרן",
    setup_needed: "נדרשת הגדרה — לחץ לפרטים",
    connect_google: "התחבר ל-Google — לחץ כאן",
    config_broken: "תצורה לא תקינה — לחץ לפתיחה",
    menu_sync: "סנכרן עכשיו",
    menu_autostart: "הפעל בעת ההתחברות",
    menu_config: "ערוך תצורה",
    menu_reset_pos: "אפס מיקום",
    menu_lock: "נעל מיקום וגודל",
    menu_unlock: "בטל נעילת מיקום וגודל",
    menu_log: "פתח יומן",
    menu_folder: "פתח תיקיית נתונים",
    menu_calendars: "לוחות שנה",
    menu_tasklists: "רשימות משימות",
    menu_copy: "העתק לוח זמנים",
    menu_update: "התקן עדכון",
    update_available: "גרסה {} זמינה — לחץ לעדכון",
    menu_relogin: "התחבר שוב ל-Google",
    menu_quit: "יציאה",
    auth_connected_title: "TPMPlaner מחובר.",
    auth_connected_body: "אפשר לסגור את החלון הזה.",
    auth_cancelled_title: "ההתחברות בוטלה.",
    auth_waiting: "ממתין להתחברות ל-Google …",
    err_missing_client: "הקובץ client_secret.json חסר. צור לקוח OAuth (אפליקציית שולחן עבודה) ב-Google Cloud Console והנח את הקובץ כאן:",
    err_not_connected: "עדיין לא מחובר ל-Google.",
    err_grant_expired: "Google ביטל את הגישה או שתוקפה פג — יש להתחבר שוב (לחיצה ימנית על הווידג'ט). הסיבה הנפוצה ביותר: לקוח ה-OAuth עדיין במצב «Testing», שבו אסימוני רענון פגים לאחר 7 ימים.",
    err_no_refresh_token: "Google לא החזיר אסימון רענון. הסר את הגישה בכתובת myaccount.google.com/permissions והתחבר שוב.",
    err_timeout: "פג הזמן הקצוב להתחברות.",
    fatal_start: "לא ניתן היה להפעיל את TPMPlaner.",
};

/// Every shipped catalogue, English first.
///
/// The consistency checks walk this list, so a catalogue that is written but
/// never listed here is only half added — and [`catalog_for`] would be the
/// other half missing.
pub const CATALOGS: &[&Catalog] = &[
    &EN, &DE, &FR, &ES, &IT, &PT, &PT_BR, &NL, &SV, &PL, &CS, &TR, &RU, &UK, &JA, &ZH_HANS,
    &ZH_HANT, &KO, &AR, &HE,
];

/// Picks a catalogue for a BCP-47 tag.
///
/// Regional variants normally share one catalogue: `de-AT` and `de-CH` both
/// get [`DE`], and dates and times still differ correctly because those come
/// from the operating system rather than from here.
///
/// Two languages need more than the primary subtag:
///
/// * **Chinese** is split by *script*, not by region. `zh-TW` and `zh-HK` must
///   not land on the simplified catalogue, and a script subtag outranks a
///   region — `zh-Hans-HK` is simplified text that happens to be read in Hong
///   Kong.
/// * **Portuguese** is split by region, because Brazilian and European
///   Portuguese differ in vocabulary and in the plural rule for zero.
pub fn catalog_for(tag: &str) -> &'static Catalog {
    let mut parts = tag.split(['-', '_']).map(|p| p.to_ascii_lowercase());
    let primary = parts.next().unwrap_or_default();
    let rest: Vec<String> = parts.collect();
    let has = |subtag: &str| rest.iter().any(|p| p == subtag);

    match primary.as_str() {
        "de" => &DE,
        "fr" => &FR,
        "es" => &ES,
        "it" => &IT,
        "pt" => {
            if has("br") {
                &PT_BR
            } else {
                &PT
            }
        }
        "nl" => &NL,
        "sv" => &SV,
        "pl" => &PL,
        "cs" => &CS,
        "tr" => &TR,
        "ru" => &RU,
        "uk" => &UK,
        "ja" => &JA,
        "zh" => {
            if has("hans") {
                &ZH_HANS
            } else if has("hant") || has("tw") || has("hk") || has("mo") {
                &ZH_HANT
            } else {
                // Bare `zh`, `zh-CN` and `zh-SG`.
                &ZH_HANS
            }
        }
        "ko" => &KO,
        "ar" => &AR,
        // `iw` is the superseded ISO 639 code for Hebrew. Still emitted by
        // some systems, so it must not fall through to English.
        "he" | "iw" => &HE,
        _ => &EN,
    }
}

/// Puts a tag into the shape the platform expects: BCP-47 separators, and
/// superseded ISO 639 codes rewritten to the ones in use today.
///
/// Both halves exist for the same reason. Windows locale lookups —
/// `GetLocaleInfoEx`, `GetTimeFormatEx`, `IsValidLocaleName`, and the locale
/// DirectWrite shapes with — accept only a well-formed BCP-47 tag. They reject
/// the pre-1989 codes some systems still emit (the JVM does, and so do older
/// Unix locale settings), and they reject the underscore that Unix locale
/// names carry, `he_IL`. [`catalog_for`] is deliberately more forgiving than
/// that: it splits on either separator, so it would happily hand back the
/// Hebrew catalogue for a tag every platform call then fails on. The window
/// would draw Hebrew labels left to right, with an empty DirectWrite locale
/// and no localised dates.
///
/// Canonicalising once, where the tag enters the program, keeps the catalogue
/// and the platform looking at the same language. Only the separator and the
/// primary subtag are touched, so `IW_IL` becomes `he-IL` and casing is
/// otherwise left alone — the platform matches case-insensitively.
fn canonical_tag(tag: &str) -> String {
    // Unix locale names also carry an encoding or a modifier (`he_IL.UTF-8`,
    // `ca_ES@valencia`). Neither is part of a language tag.
    let tag = tag
        .split(['.', '@'])
        .next()
        .unwrap_or(tag)
        .replace('_', "-");
    let end = tag.find('-').unwrap_or(tag.len());
    let canonical = match tag[..end].to_ascii_lowercase().as_str() {
        "iw" => "he",
        "in" => "id",
        "ji" => "yi",
        _ => return tag,
    };
    format!("{canonical}{}", &tag[end..])
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
        // Before anything asks the platform about this tag: it has to be one
        // the platform recognises. See [`canonical_tag`].
        let tag = canonical_tag(&tag);
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
    /// Only applies while the layout is mirrored but the catalogue is not: an
    /// Arabic locale reading the Arabic catalogue needs no bracket, and adding
    /// one there would force right-to-left text through a left-to-right
    /// embedding. User data such as event titles and task names is
    /// deliberately left alone: it may well be Arabic and has to run freely.
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

    /// Human-readable distance: "in 25 min", "2 hr 10 ago", "encore 5 min".
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

    /// Time remaining in the event running right now ("32 min left").
    pub fn time_left(&self, minutes: i64) -> String {
        let body = self.duration_raw(minutes.max(0));
        self.iso(self.cat.left_pattern.replacen("{}", &body, 1))
    }

    /// The bare amount without a direction word, and without the bidi bracket.
    ///
    /// The minute remainder carries its own unit. Leaving it bare reads well
    /// enough in the languages this started out in — "2 hr 10" — but the unit
    /// is not optional everywhere: Japanese, both Chinese catalogues, Korean,
    /// Arabic and Hebrew all attach the following direction word straight to
    /// the number, so a bare remainder produced "2 時間 10後" instead of
    /// "2 時間 10 分後".
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
                format!("{h} {} {rest} {}", c.unit_hour, c.unit_min)
            }
        } else {
            format!("{} {}", m / (60 * 24), c.unit_day)
        }
    }

    pub fn overdue(&self, n: usize) -> String {
        self.counted(self.cat.overdue, n)
    }

    pub fn conflicts(&self, n: usize) -> String {
        self.counted(self.cat.conflicts, n)
    }

    /// Picks the plural form for `n` and fills the count in, if the form asked
    /// for it — see [`Plural`] on why `one` and `two` may not.
    fn counted(&self, plural: Plural, n: usize) -> String {
        let form = plural.select(n as u64);
        self.iso(form.replacen("{}", &n.to_string(), 1))
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

    /// Rough column width of a string, in units of one Latin character.
    ///
    /// A character count alone would call "予定はありません" short and
    /// "Nothing due today" long, when on screen it is the other way round: the
    /// East Asian and Hangul blocks are drawn at roughly twice the width of a
    /// Latin letter. Everything else — including combining marks in Arabic and
    /// Hebrew, which take no width at all — is close enough to one to be
    /// counted as one, since this only has to catch a translation that
    /// overflows its column, not lay text out.
    fn display_width(s: &str) -> usize {
        s.chars()
            .map(|c| match c as u32 {
                // CJK punctuation, Hiragana, Katakana, Hangul, CJK ideographs
                // and the fullwidth forms.
                0x1100..=0x115F
                | 0x2E80..=0x303E
                | 0x3041..=0x33FF
                | 0x3400..=0x4DBF
                | 0x4E00..=0x9FFF
                | 0xA000..=0xA4CF
                | 0xAC00..=0xD7A3
                | 0xF900..=0xFAFF
                | 0xFE30..=0xFE6F
                | 0xFF00..=0xFF60
                | 0xFFE0..=0xFFE6 => 2,
                _ => 1,
            })
            .sum()
    }

    #[test]
    fn regional_variants_share_a_catalog() {
        assert_eq!(catalog_for("de-AT").code, "de");
        assert_eq!(catalog_for("de_CH").code, "de");
        assert_eq!(catalog_for("de").code, "de");
    }

    #[test]
    fn unknown_languages_fall_back_to_english() {
        assert_eq!(catalog_for("th-TH").code, "en");
        assert_eq!(catalog_for("").code, "en");
        assert_eq!(catalog_for("xx").code, "en");
    }

    /// Chinese is the case the primary subtag cannot answer: `zh-TW` is not a
    /// regional flavour of the simplified catalogue, it is a different script.
    #[test]
    fn chinese_is_matched_by_script_not_by_region() {
        for tag in ["zh", "zh-CN", "zh-Hans", "zh-Hans-CN", "zh-SG", "zh_CN"] {
            assert_eq!(catalog_for(tag).code, "zh-Hans", "{tag}");
        }
        for tag in ["zh-TW", "zh-HK", "zh-MO", "zh-Hant", "zh-Hant-TW", "zh_TW"] {
            assert_eq!(catalog_for(tag).code, "zh-Hant", "{tag}");
        }
        // A script subtag outranks the region it is read in.
        assert_eq!(catalog_for("zh-Hans-HK").code, "zh-Hans");
        assert_eq!(catalog_for("zh-Hant-CN").code, "zh-Hant");
    }

    #[test]
    fn portuguese_splits_by_region_and_hebrew_accepts_its_old_code() {
        assert_eq!(catalog_for("pt").code, "pt");
        assert_eq!(catalog_for("pt-PT").code, "pt");
        assert_eq!(catalog_for("pt-BR").code, "pt-BR");
        assert_eq!(catalog_for("pt_br").code, "pt-BR");
        assert_eq!(catalog_for("he-IL").code, "he");
        assert_eq!(catalog_for("iw").code, "he");
    }

    #[test]
    fn relative_time_uses_the_catalog_patterns() {
        let de = Locale::resolve("de-DE");
        assert_eq!(de.relative(0), "jetzt");
        assert_eq!(de.relative(25), "in 25 Min");
        assert_eq!(de.relative(130), "in 2 Std 10 Min");
        assert_eq!(de.relative(-5), "vor 5 Min");
        assert_eq!(de.time_left(32), "noch 32 Min");

        let en = Locale::resolve("en-US");
        assert_eq!(en.relative(25), "in 25 min");
        assert_eq!(en.relative(-5), "5 min ago");
        assert_eq!(en.time_left(32), "32 min left");
    }

    #[test]
    fn the_minute_remainder_keeps_its_unit() {
        // The direction word attaches straight to the number in these
        // languages, so a unit-less remainder ran the two together.
        assert_eq!(Locale::resolve("ja-JP").relative(130), "2 時間 10 分後");
        assert_eq!(Locale::resolve("ko-KR").time_left(130), "2 시간 10 분 남음");

        // Every catalogue: an hours-plus-minutes distance must end in the
        // minute unit, never in a bare digit.
        for cat in CATALOGS {
            let loc = Locale {
                tag: cat.code.to_string(),
                cat,
                rtl: cat.rtl,
            };
            let body = loc.duration_raw(130);
            assert!(
                body.ends_with(cat.unit_min),
                "{}: `{body}` drops the minute unit",
                cat.code
            );
        }
    }

    #[test]
    fn spanish_plural_differs_by_count() {
        let es = Locale::resolve("es-ES");
        assert_eq!(es.overdue(1), "1 atrasada");
        assert_eq!(es.overdue(3), "3 atrasadas");
    }

    /// The boundaries are the whole point of the rules — 11 to 14 in the
    /// Slavic languages, and the difference between Russian's 21 and Polish's.
    #[test]
    fn plural_rules_hit_their_boundaries() {
        let two = Plural::OneOther {
            one: "one",
            other: "other",
        };
        assert_eq!(two.select(0), "other");
        assert_eq!(two.select(1), "one");
        assert_eq!(two.select(21), "other");

        let french = Plural::OneIncludingZero {
            one: "one",
            other: "other",
        };
        assert_eq!(french.select(0), "one", "zero is singular in French");
        assert_eq!(french.select(1), "one");
        assert_eq!(french.select(2), "other");

        let russian = Plural::EastSlavic {
            one: "one",
            few: "few",
            many: "many",
        };
        for (n, want) in [
            (0, "many"),
            (1, "one"),
            (2, "few"),
            (4, "few"),
            (5, "many"),
            (11, "many"),
            (12, "many"),
            (14, "many"),
            (21, "one"),
            (22, "few"),
            (25, "many"),
            (101, "one"),
            (111, "many"),
        ] {
            assert_eq!(russian.select(n), want, "Russian {n}");
        }

        // Polish agrees with Russian on 2..4 and disagrees on everything that
        // ends in 1 but is not 1.
        let polish = Plural::Polish {
            one: "one",
            few: "few",
            many: "many",
        };
        for (n, want) in [
            (0, "many"),
            (1, "one"),
            (2, "few"),
            (5, "many"),
            (12, "many"),
            (21, "many"),
            (22, "few"),
            (101, "many"),
        ] {
            assert_eq!(polish.select(n), want, "Polish {n}");
        }

        let czech = Plural::Czech {
            one: "one",
            few: "few",
            other: "other",
        };
        for (n, want) in [
            (0, "other"),
            (1, "one"),
            (2, "few"),
            (4, "few"),
            (5, "other"),
            (22, "other"),
        ] {
            assert_eq!(czech.select(n), want, "Czech {n}");
        }

        let hebrew = Plural::Hebrew {
            one: "one",
            two: "two",
            other: "other",
        };
        assert_eq!(hebrew.select(1), "one");
        assert_eq!(hebrew.select(2), "two", "Hebrew has a dual");
        assert_eq!(hebrew.select(3), "other");

        let arabic = Plural::Arabic {
            zero: "zero",
            one: "one",
            two: "two",
            few: "few",
            many: "many",
            other: "other",
        };
        for (n, want) in [
            (0, "zero"),
            (1, "one"),
            (2, "two"),
            (3, "few"),
            (10, "few"),
            (11, "many"),
            (99, "many"),
            (100, "other"),
            (102, "other"),
            (103, "few"),
        ] {
            assert_eq!(arabic.select(n), want, "Arabic {n}");
        }
    }

    /// A three-form language has to produce three different strings, or the
    /// rule was added without the forms behind it.
    #[test]
    fn russian_really_uses_all_three_forms() {
        let ru = Locale::resolve("ru-RU");
        assert_eq!(ru.overdue(1), "1 просрочена");
        assert_eq!(ru.overdue(3), "3 просрочены");
        assert_eq!(ru.overdue(7), "7 просрочено");
    }

    /// Arabic and Hebrew spell the small numbers out, which is why the
    /// placeholder is optional in those forms.
    #[test]
    fn small_counts_may_omit_the_numeral() {
        let ar = Locale::resolve("ar-SA");
        assert_eq!(ar.conflicts(1), "تعارض واحد");
        assert_eq!(ar.conflicts(2), "تعارضان");
        assert!(ar.conflicts(5).starts_with('5'));

        let he = Locale::resolve("he-IL");
        assert_eq!(he.conflicts(2), "שתי חפיפות");
        assert!(he.conflicts(7).starts_with('7'));
    }

    /// The isolation bracket exists for English labels in a mirrored layout.
    /// Once the catalogue is itself right to left it must not be applied —
    /// that would force Arabic through a left-to-right embedding.
    #[test]
    fn right_to_left_catalogs_are_not_bracketed() {
        const LRE: char = '\u{202A}';
        let ar = Locale::resolve("ar-SA");
        assert!(ar.rtl && ar.cat.rtl);
        assert!(!ar.label(ar.cat.section_tasks).contains(LRE));
        assert!(!ar.relative(25).contains(LRE));

        // Persian has no catalogue, so the layout mirrors while the labels
        // stay English — exactly the case the bracket was written for.
        let fa = Locale::resolve("fa-IR");
        assert!(fa.rtl && !fa.cat.rtl);
        assert!(fa.label(fa.cat.section_tasks).contains(LRE));
    }

    /// Every catalogue has to be complete and translated, in a way the
    /// compiler cannot check on its own.
    ///
    /// The struct of named fields makes a *forgotten* field a compile error.
    /// This covers the other half: a field left empty, a pattern that lost its
    /// placeholder, or a "translation" that is still the English text.
    #[test]
    fn every_catalog_is_complete_and_translated() {
        for cat in CATALOGS {
            let code = cat.code;
            assert!(!code.is_empty());

            for (name, value) in cat.fields() {
                assert!(!value.trim().is_empty(), "{code}: {name} is empty");
            }

            // Patterns must keep their placeholder, or the value silently
            // disappears from the rendered string.
            for (name, value) in cat.fields() {
                if matches!(
                    name,
                    "in_pattern" | "ago_pattern" | "left_pattern" | "update_available"
                ) {
                    assert!(value.contains("{}"), "{code}: {name} lost its placeholder");
                }
            }
            assert_eq!(
                cat.updated_next.matches("{}").count(),
                2,
                "{code}: updated_next needs two placeholders"
            );

            for (label, plural) in [("conflicts", cat.conflicts), ("overdue", cat.overdue)] {
                for form in plural.forms() {
                    assert!(!form.trim().is_empty(), "{code}: {label} has an empty form");
                }
                for form in plural.counted_forms() {
                    assert!(
                        form.contains("{}"),
                        "{code}: {label} form {form:?} lost its count"
                    );
                }
            }

            // Both counted messages in one language must use the same rule;
            // a language does not have two.
            assert_eq!(
                std::mem::discriminant(&cat.conflicts),
                std::mem::discriminant(&cat.overdue),
                "{code}: conflicts and overdue disagree about the plural rule"
            );
        }

        // Nothing may simply be the English text. Two fields are enough to
        // catch a catalogue copied and never filled in; checking every field
        // would fail on the ones that genuinely coincide.
        for cat in CATALOGS.iter().filter(|c| c.code != "en") {
            assert_ne!(
                cat.section_tasks, EN.section_tasks,
                "{}: section_tasks was never translated",
                cat.code
            );
            assert_ne!(
                cat.menu_quit, EN.menu_quit,
                "{}: menu_quit was never translated",
                cat.code
            );
        }

        // The two Chinese catalogues are separate for a reason.
        assert_ne!(ZH_HANS.section_events, ZH_HANT.section_events);
        assert_ne!(PT.syncing, PT_BR.syncing);
    }

    /// Every catalogue must be reachable, and no two may claim the same code.
    #[test]
    fn every_catalog_is_reachable_by_its_own_code() {
        let mut seen = Vec::new();
        for cat in CATALOGS {
            assert!(!seen.contains(&cat.code), "duplicate code {}", cat.code);
            seen.push(cat.code);
            assert_eq!(
                catalog_for(cat.code).code,
                cat.code,
                "{} is written but not listed in catalog_for",
                cat.code
            );
        }
    }

    /// The panel is roughly 380 device independent pixels wide and several
    /// labels sit in a fixed column, so a translation that is correct but long
    /// is still a defect: it arrives on screen as an ellipsis.
    ///
    /// The budgets are in units of one Latin character and were chosen to fit
    /// the shipped text with a little room, not the other way round. A new
    /// translation that trips one of them is being told to find a shorter
    /// word, not that the budget is wrong.
    #[test]
    fn tight_labels_stay_inside_their_column() {
        // (field, budget) for the labels that share a row with something else.
        const BUDGETS: &[(&str, usize)] = &[
            ("unit_min", 6),
            ("unit_hour", 6),
            ("unit_day", 6),
            ("all_day", 12),
            ("running", 12),
            ("today", 12),
            ("yesterday", 12),
            ("tomorrow", 14),
            ("now_label", 16),
            ("next_label", 16),
            ("section_events", 16),
            ("section_tasks", 16),
            ("undo", 16),
        ];

        for cat in CATALOGS {
            for (name, value) in cat.fields() {
                if let Some((_, budget)) = BUDGETS.iter().find(|(f, _)| *f == name) {
                    let width = display_width(value);
                    assert!(
                        width <= *budget,
                        "{}: {name} is {width} wide, budget is {budget} — {value:?}",
                        cat.code
                    );
                }
            }
        }
    }

    /// A long translation that is not in a fixed column still has a limit: the
    /// status line and the menu are only so wide.
    ///
    /// Generous on purpose — German and Russian are simply longer than English
    /// and that is not a defect. This catches the translation that ran away,
    /// typically because a note to the translator ended up in the string.
    #[test]
    fn no_translation_runs_away_from_its_english_original() {
        let english: Vec<(&str, &str)> = EN.fields();
        for cat in CATALOGS.iter().filter(|c| c.code != "en") {
            for (name, value) in cat.fields() {
                let (_, original) = english
                    .iter()
                    .find(|(n, _)| *n == name)
                    .expect("the field lists are built from the same struct");
                let reference = display_width(original);
                let limit = reference * 5 / 2 + 10;
                let width = display_width(value);
                assert!(
                    width <= limit,
                    "{}: {name} is {width} wide against {reference} in English \
                     (limit {limit}) — {value:?}",
                    cat.code
                );
            }
        }
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

    #[test]
    fn sign_in_page_declares_its_reading_direction() {
        assert_eq!(HE.html_attrs(), "lang=\"he\" dir=\"rtl\"");
        assert_eq!(AR.html_attrs(), "lang=\"ar\" dir=\"rtl\"");
        assert_eq!(EN.html_attrs(), "lang=\"en\" dir=\"ltr\"");
        // The tag carries the subtag where the catalogue needs one.
        assert_eq!(ZH_HANT.html_attrs(), "lang=\"zh-Hant\" dir=\"ltr\"");
        assert_eq!(PT_BR.html_attrs(), "lang=\"pt-BR\" dir=\"ltr\"");
    }

    #[test]
    fn superseded_language_codes_are_rewritten() {
        assert_eq!(canonical_tag("iw"), "he");
        assert_eq!(canonical_tag("iw-IL"), "he-IL");
        assert_eq!(
            canonical_tag("IW_IL"),
            "he-IL",
            "the platform only accepts the BCP-47 separator"
        );
        assert_eq!(canonical_tag("ji"), "yi");
        assert_eq!(canonical_tag("in-ID"), "id-ID");
        // Everything else passes through untouched, casing included.
        assert_eq!(canonical_tag("he-IL"), "he-IL");
        assert_eq!(canonical_tag("de-DE"), "de-DE");
        assert_eq!(canonical_tag(""), "");
        // `ind` starts with "in" but is a subtag in its own right, not the
        // superseded code: only a whole primary subtag is rewritten.
        assert_eq!(canonical_tag("ind"), "ind");
    }

    #[test]
    fn unix_locale_names_are_reshaped_for_the_platform() {
        // A `LANG` value, verbatim. The catalogue understands all three
        // spellings; the platform understands only the last one.
        assert_eq!(canonical_tag("he_IL.UTF-8"), "he-IL");
        assert_eq!(canonical_tag("de_CH"), "de-CH");
        assert_eq!(canonical_tag("ca_ES@valencia"), "ca-ES");
        assert_eq!(canonical_tag("zh_TW"), "zh-TW");
        // The encoding suffix is not a subtag even without a region.
        assert_eq!(canonical_tag("ar.UTF-8"), "ar");
    }

    #[test]
    fn underscore_separated_tags_still_read_right_to_left() {
        // `catalog_for` splits on either separator, so `he_IL` has always
        // selected the Hebrew catalogue. The platform's reading-direction
        // lookup rejects the underscore, which used to leave right-to-left
        // text in a left-to-right panel.
        for tag in ["he_IL", "ar_SA", "he_IL.UTF-8"] {
            let loc = Locale::resolve(tag);
            assert!(loc.cat.rtl, "{tag} must select a right-to-left catalogue");
            assert!(
                loc.rtl,
                "{tag}: layout direction must not contradict the catalogue"
            );
            assert!(
                !loc.tag.contains('_'),
                "{tag}: the platform must be handed a tag it accepts"
            );
        }
    }

    #[test]
    fn superseded_hebrew_code_reads_right_to_left() {
        // The catalogue has always understood `iw`; the reading direction and
        // the platform's locale lookups did not, which laid Hebrew text out
        // left to right. Resolving canonicalises the tag so both agree.
        let loc = Locale::resolve("iw");
        assert_eq!(loc.cat.code, "he");
        assert_eq!(loc.tag, "he", "the platform is handed a tag it knows");
        assert!(loc.rtl, "`iw` is Hebrew and must read right to left");
        assert_eq!(
            loc.rtl, loc.cat.rtl,
            "layout direction must not contradict the catalogue"
        );
    }
}
