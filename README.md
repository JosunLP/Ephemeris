# TPMPlaner

Desktop-Widget für Windows 11, das die **Termine des heutigen Tages** aus Google Kalender und die
**heute oder früher fälligen Aufgaben** aus Google Tasks anzeigt. Rust + Direct2D/DirectComposition,
eine einzelne `.exe` von ~1,5 MB ohne Runtime-Abhängigkeit.

---

## Verhalten

| | |
|---|---|
| Position | Liegt unter allen normalen Fenstern, über dem Desktop. Kein Taskleisten-Eintrag, kein Alt-Tab, stiehlt nie den Fokus. |
| Sync | Alle 30 Minuten (konfigurierbar), zusätzlich beim Aufwachen aus dem Standby, bei Tageswechsel um Mitternacht und per Klick auf ⟳. |
| Aufgaben | Nur mit Fälligkeit **≤ heute**. Überfällige stehen oben, sind rot markiert und werden in der Abschnittszeile gezählt. Sortierung: älteste Fälligkeit zuerst. |
| Termine | Nur der heutige Tag. Ganztägige zuerst, dann nach Startzeit. Rechts steht die Restzeit („in 1 Std 27"), eine Jetzt-Linie mit Uhrzeit trennt Vergangenes von Kommendem. |
| Kopfbereich | Wochentag, Datum, Uhr und ein Tagesfortschrittsbalken (06–22 Uhr). Darunter eine Karte mit dem **laufenden** bzw. **nächsten** Termin samt Countdown und Fortschrittsbalken. |
| Überschneidungen | Doppelbuchungen werden erkannt, in der Abschnittszeile gezählt und färben die betroffenen Uhrzeiten. Aufeinanderfolgende Termine (10:00–11:00 / 11:00–12:00) zählen nicht. |
| Morgen | Sobald der heutige Tag durch ist, wäre die Fläche leer — deshalb steht darunter der Ausblick auf die ersten beiden Termine von morgen. Kostet keine zusätzliche Anfrage. |
| Gekürzter Text | Zeigt beim Überfahren den vollständigen Titel in einer eingeblendeten Sprechblase. |
| Sprache | Folgt der Windows-Anzeigesprache. Mitgeliefert: **Deutsch, Englisch, Französisch, Spanisch, Italienisch**. Datum, Uhrzeit und Leserichtung stimmen in **jedem** Gebietsschema, auch ohne passenden Textkatalog. |
| Aussehen | Folgt dem Windows-App-Design (hell/dunkel), dem **Kontrastdesign**, den **Transparenz-** und **Animationseinstellungen** und übernimmt die **Systemakzentfarbe** — alles zur Laufzeit, ohne Neustart. |
| Bedienung | Ziehen = verschieben · **Kante ziehen = Größe ändern** · Mausrad = scrollen · Kreis anklicken = Aufgabe abhaken · Termin-/Hero-Karte = im Kalender öffnen · Rechtsklick = Menü |
| Rückgängig | Ein Abhaken geht erst nach 4 Sekunden an Google. In der Zeit steht **„Rückgängig"** in der Zeile — ein Klick darauf nimmt es zurück, ohne dass je etwas gesendet wurde. |

Bei Netzwerkfehlern bleiben die letzten Daten stehen, es färbt sich nur die Fußzeile. Wiederholversuche
laufen mit exponentiellem Backoff (1, 2, 4, 8 … Minuten, gedeckelt auf das Sync-Intervall).

**Animationen** (Aufblenden neuer Daten, Hover, weiches Scrollen, rotierendes Sync-Symbol) laufen mit
~60 Hz — aber nur, solange sich tatsächlich etwas bewegt. Danach schaltet sich der Animationstimer ab
und das Widget verbraucht wieder messbar **0,00 s CPU**.

## Vorschaumodus

Um Größe, Skalierung und Deckkraft einzustellen, bevor der Google-Zugang steht:

```powershell
$env:TPMPLANER_DEMO = "1"; .\tpmplaner.exe
```

Zeigt Beispieldaten und synchronisiert nichts.

---

## Einrichtung

Google erlaubt keinen API-Zugriff ohne eigenen OAuth-Client. Der folgende Teil ist **einmalig** nötig.

### 1. Google-Cloud-Projekt anlegen

1. [console.cloud.google.com](https://console.cloud.google.com/) → neues Projekt anlegen.
2. **APIs und Dienste → Bibliothek**: *Google Calendar API* und *Google Tasks API* jeweils aktivieren.

### 2. OAuth-Zustimmungsbildschirm

1. **APIs und Dienste → OAuth-Zustimmungsbildschirm**, Nutzertyp *Extern*.
2. Scopes hinzufügen:
   - `.../auth/calendar.readonly`
   - `.../auth/calendar.events.readonly`
   - `.../auth/tasks`
3. Eigene Adresse als Testnutzer eintragen.

> ⚠️ **Wichtig:** Anschließend auf **„App veröffentlichen" / Status *In Produktion*** stellen.
> Solange das Projekt im Status *Testing* steht, **verfallen Refresh-Tokens nach 7 Tagen** und das
> Widget verlangt wöchentlich eine neue Anmeldung. Der einmalige Hinweis „Google hat diese App nicht
> überprüft" beim ersten Login ist normal und über *Erweitert → Weiter zu …* zu bestätigen —
> eine Verifizierung ist für die private Nutzung nicht erforderlich.

### 3. Client-Datei ablegen

1. **Anmeldedaten → Anmeldedaten erstellen → OAuth-Client-ID**, Anwendungstyp **Desktop-App**.
2. JSON herunterladen und ablegen als:

```text
%APPDATA%\TPMPlaner\client_secret.json
```

### 4. Starten

`tpmplaner.exe` ausführen. Beim ersten Start öffnet sich der Browser zur Google-Anmeldung
(Loopback-Redirect auf `127.0.0.1` mit PKCE). Danach steht die Agenda.

Autostart: Rechtsklick auf das Widget → *Mit Windows starten*.

---

## Konten

Ohne `accounts`-Eintrag verhält sich das Widget wie bisher: ein Google-Konto. Mehrere Konten werden
**parallel** abgefragt; fällt eines aus, bleiben die anderen sichtbar.

```jsonc
"accounts": [
  { "kind": "google",    "id": "privat", "label": "Privat" },
  { "kind": "microsoft", "id": "arbeit", "label": "Arbeit" },
  { "kind": "caldav",    "id": "icloud", "label": "iCloud" }
]
```

| Dienst | Zugangsdatei in `%APPDATA%\TPMPlaner\` | Woher |
|---|---|---|
| `google` | `client_secret.json` | Google Cloud Console, OAuth-Client „Desktop-App" |
| `microsoft` | `microsoft_client.json` mit `{"client_id":"…"}` | Azure-Portal, App-Registrierung als **öffentlicher Client**, Umleitungs-URI `http://localhost` |
| `caldav` | `caldav-<id>.json` mit `{"url":"…","username":"…","password":"…"}` | Server-URL und ein **app-spezifisches Passwort** |

**Outlook und Teams** laufen beide über `microsoft`. Einen eigenen Teams-Kalender gibt es nicht — eine
Teams-Besprechung ist ein Outlook-Termin mit Beitrittslink. Der Klick auf eine solche Zeile öffnet
die Besprechung statt des Kalendereintrags.

**CalDAV** deckt iCloud, Nextcloud, Fastmail, Synology und mailbox.org ab. Das Passwort wird beim
ersten Start aus der JSON-Datei in den verschlüsselten Speicher verschoben und dort entfernt.
Serientermine löst der Server auf (`<C:expand>`), nicht das Widget.

## Installation

```powershell
irm https://github.com/JosunLP/TPMPlaner/releases/latest/download/install.ps1 | iex
```

Installiert nach `%LOCALAPPDATA%\Programs\TPMPlaner`, ohne Administratorrechte. Die SHA256-Prüfsumme
wird **vor** dem Schreiben geprüft. Das Widget sucht danach täglich nach neuen Versionen und bietet
sie im Menü und in der Fußzeile an — installiert aber nichts von selbst.

Entfernen:

```powershell
irm https://github.com/JosunLP/TPMPlaner/releases/latest/download/uninstall.ps1 | iex
```

Einstellungen und Zugangsdaten bleiben erhalten; `-Purge` löscht auch die.

## Konfiguration

`%APPDATA%\TPMPlaner\config.json` (Rechtsklick → *Konfiguration bearbeiten*). Änderungen werden
**ohne Neustart** übernommen — das Widget prüft die Datei im Minutentakt und baut sich bei geänderter
Skalierung selbst neu auf.

```jsonc
{
  "x": 1516, "y": 24,          // Position; wird beim Verschieben gespeichert
  "width": 380, "height": 620, // Größe in DIPs (skaliert mit der Monitor-DPI)
  "sync_minutes": 30,
  "calendar_ids": [],          // leer = alle im Google-Web-UI aktivierten Kalender
  "tasklist_ids": [],          // leer = alle Aufgabenlisten
  "show_undated_tasks": false, // Aufgaben ohne Fälligkeitsdatum mit anzeigen
  "show_past_events": true,    // beendete Termine gedimmt stehen lassen
  "hide_declined": true,       // selbst abgesagte Termine ausblenden
  "opacity": 0.82,
  "backdrop": "acrylic",       // "acrylic" | "none"
  "scale": 1.0,                // zusätzliche Layout-Skalierung
  "language": "system",        // "system" oder BCP-47, z.B. "en-US", "ar-SA"
  "theme": "system",           // "system" | "dark" | "light" | "contrast"
  "accent": "system",          // "system" | "#RRGGBB"
  "undo_seconds": 4            // Bedenkzeit beim Abhaken; 0 schaltet sie ab
}
```

Bei einem Syntaxfehler meldet die Fußzeile **„Konfiguration fehlerhaft"** und ein Klick öffnet die
Datei — die genaue Fehlerstelle steht im Protokoll. Eine vorangestellte Byte-Order-Mark (schreiben
viele Windows-Editoren) wird toleriert.

---

## Dateien

| Pfad | Inhalt |
|---|---|
| `%APPDATA%\TPMPlaner\client_secret.json` | OAuth-Client (von Ihnen abgelegt) |
| `%APPDATA%\TPMPlaner\token.bin` | Refresh-Token, **DPAPI-verschlüsselt** an das Windows-Benutzerkonto gebunden |
| `%APPDATA%\TPMPlaner\config.json` | Konfiguration |
| `%APPDATA%\TPMPlaner\cache.json` | Letzter Sync-Stand, damit beim Start sofort etwas dasteht |
| `%APPDATA%\TPMPlaner\tpmplaner.log` | Protokoll (Rechtsklick → *Protokoll öffnen*), rotiert bei 256 KB |

Der Access-Token liegt ausschließlich im RAM und wird nie geschrieben.

---

## Zwei API-Fallstricke, die hier gelöst sind

**Das Fälligkeitsdatum in Google Tasks ist kein Zeitpunkt.** Das Feld `due` kommt als
`2026-08-04T00:00:00.000Z`, die Uhrzeit wird serverseitig weggeworfen. Wer den Wert als echten
UTC-Zeitstempel parst und in die lokale Zone konvertiert, landet in Mitteleuropa **einen Tag zu
früh** — aus „heute fällig" wird „gestern fällig". `model::parse_task_due` wertet deshalb nur die
ersten zehn Zeichen aus und konvertiert nie. Abgesichert durch Unit-Tests.

**`dueMax` blendet undatierte Aufgaben aus.** Sobald einer der Parameter `dueMin`/`dueMax` gesetzt
ist, verschwinden Aufgaben ohne Fälligkeitsdatum vollständig aus der Antwort. Deshalb filtert
`tasks::list_tasks` nur dann serverseitig, wenn `show_undated_tasks` aus ist; sonst wird ungefiltert
geladen und clientseitig aussortiert.

---

## Bauen

```powershell
cargo test           # Filter- und Datumslogik
cargo build --release
```

Ergebnis: `target\release\tpmplaner.exe`, ~1,5 MB, keine weiteren Dateien nötig.

---

## Architektur

```text
main.rs      Einstiegspunkt, COM-Init, Einzelinstanz-Sperre
window.rs    Win32-Fenster: Bottom-Most, Drag, Hit-Test, Timer, Kontextmenü
render.rs    Direct2D auf DirectComposition-Swapchain (Per-Pixel-Alpha)
anim.rs      Federn für Ein-/Ausblenden, Scrollen, Hover, Sync-Symbol
theme.rs     Palette (hell/dunkel/Kontrast, Systemakzent), Maße (+ Tests)
i18n.rs      Sprachkataloge, NLS-Datum/Zeit, Leserichtung, Bidi (+ Tests)
log.rs       Dateiprotokoll mit Größenrotation
sync.rs      Sync-Thread, Zeitplanung, Backoff, Cache
model.rs     Domänenlogik: Filtern, Sortieren, Datumsparsing (+ Tests)
google/      auth.rs (OAuth/PKCE/DPAPI) · calendar.rs · tasks.rs
secure.rs    DPAPI-Wrapper, System-RNG
platform.rs  Browser, Autostart, Monitor-Prüfung, Working-Set
config.rs    Konfiguration und Pfade
demo.rs      Beispieldaten für den Vorschaumodus
```

Zwei Threads: der UI-Thread zeichnet und besitzt die gesamte Zeitplanung, der Sync-Thread macht
ausschließlich Netzwerk. Kein async-Runtime.

Zwei Timer: einer im **Minutentakt** (Uhr, Relativzeiten, Sync-Fälligkeit, Config-Prüfung), einer mit
**~60 Hz**, der ausschließlich während einer laufenden Animation existiert. Es gibt keine
Render-Schleife.

### Robustheit

- **Geräteverlust** (Treiberwechsel, GPU-Reset, Wechsel in eine RDP-Sitzung) wird an den
  HRESULTs `D2DERR_RECREATE_TARGET` / `DXGI_ERROR_DEVICE_*` erkannt; die gesamte Gerätekette wird
  dann neu aufgebaut. Ohne diese Behandlung bliebe das Fenster dauerhaft schwarz.
- **Monitor abgezogen**: liegt die gespeicherte Position auf keinem angeschlossenen Bildschirm mehr
  (`MonitorFromRect` → `MONITOR_DEFAULTTONULL`), holt sich das Widget beim Start, bei
  `WM_DISPLAYCHANGE` und nach dem Standby selbst zurück. Zusätzlich im Menü: *Position zurücksetzen*.
- **Einzelinstanz** über einen benannten Mutex — sonst läge ein zweites, deckungsgleiches Fenster
  auf dem ersten, und Klicks schienen ins Leere zu gehen.
- **Pinsel und Textlayouts** werden zwischengespeichert; DirectWrite-Layout ist der teuerste
  Einzelschritt pro Frame, und während einer Animation ändert sich der Text nicht.
- **Unternehmensproxy**: `ureq` übernimmt die Windows-Interneteinstellungen, sonst käme in vielen
  Firmennetzen kein einziger Abruf durch.
- **Verzeichnis-Cache**: Kalender- und Aufgabenlisten werden 6 Stunden gehalten. Sie ändern sich
  praktisch nie und kosteten sonst zwei von rund zehn Anfragen pro Sync.
- **Vollständige Fehlertexte** landen im Protokoll; die Fußzeile hat nur Platz für rund 56 Zeichen,
  eine Google-Fehlermeldung ist regelmäßig länger.
- **Zeit- oder Zeitzonenwechsel** (`WM_TIMECHANGE`, etwa auf Reisen oder bei der Sommerzeit) löst
  einen sofortigen Abgleich aus — die Tagesgrenze und sämtliche Relativzeiten hängen daran.
- **Systemabmeldung** (`WM_ENDSESSION`) schickt ein wartendes Abhaken noch ab; `WM_DESTROY` kommt
  beim Herunterfahren nicht zuverlässig.
- **Aufgestaute Sync-Anforderungen** werden zusammengefasst. Während einer bis zu fünf Minuten
  langen Browser-Anmeldung sammeln sich Anfragen im Kanal; zehnmal hintereinander abzugleichen
  liefert zehnmal dasselbe Ergebnis und kostet nur Kontingent. Erledigungen und Neuanmeldungen
  bleiben dabei vollständig erhalten.

### Internationalisierung

Zwei getrennte Zuständigkeiten, und die Trennung ist der Kern:

- **Oberflächentexte** kommen aus einkompilierten Katalogen ([i18n.rs](src/i18n.rs)). Jede Sprache ist
  ein `Catalog`-Literal mit benannten Feldern — eine vergessene Übersetzung ist dadurch ein
  Kompilierfehler, kein zur Laufzeit fehlender Schlüssel. Eine Sprache hinzuzufügen heißt: eine
  Konstante schreiben und in `catalog_for` eintragen.
- **Datum, Uhrzeit und Leserichtung** kommen vom Betriebssystem (`GetDateFormatEx`,
  `GetTimeFormatEx`, `GetLocaleInfoEx`). Deshalb stimmen sie in *jedem* Gebietsschema, auch in einem
  ohne Katalog: `ar-SA` bekommt den Hidschri-Kalender, `en-US` eine 12-Stunden-Uhr und
  Monat-vor-Tag, `de-DE` 24 Stunden und Tag-vor-Monat. Ein fest verdrahtetes `{:02}:{:02}` hätte
  einem Benutzer in den USA „20:09" statt „8:09 PM" gezeigt.

**Rechts-nach-links** (Arabisch, Hebräisch, Persisch) spiegelt das gesamte Layout: Kopfzeile,
Farbmarken, Abhak-Kreise, Spalten und Kontextmenü. Gespiegelt wird ausschließlich in den
Zeichenprimitiven — der Layoutcode rechnet unverändert von links nach rechts.

Enthält der Katalog lateinische Texte, während das Layout gespiegelt ist, werden diese in
`U+202A`/`U+202C` geklammert. Ohne das schiebt der Bidi-Algorithmus führende Ziffern ans andere Ende
und aus „32 min left" wird sichtbar „min left 32". *(Die neueren Isolate `U+2066`/`U+2069` wertet
DirectWrite nicht aus — mit ihnen blieb der Fehler bestehen.)*

**Spaltenbreiten werden gemessen, nicht geraten.** Feste Breiten sind die klassische i18n-Falle: was
für „gestern" reicht, schneidet „yesterday" ab. Fälligkeits- und Zeitspalten bestimmen ihre Breite
pro Bild aus dem breitesten tatsächlichen Eintrag.

### Windows-Theme-Bereitschaft

| Systemeinstellung | Verhalten |
|---|---|
| App-Modus hell/dunkel | Palette wechselt zur Laufzeit |
| Akzentfarbe | Wird übernommen und in ein lesbares Helligkeitsband gezwungen |
| **Kontrastdesign** | Alle Farben aus `GetSysColor`; Verlauf, Glanz und Schatten entfallen, das Panel wird deckend, Kalenderfarben weichen der Systemfarbe |
| **Transparenzeffekte aus** | Panel zeichnet deckend statt durchscheinend |
| **Animationen aus** | Jeder Wert springt sofort ans Ziel, der 60-Hz-Timer startet gar nicht erst |

Alle fünf werden bei `WM_SETTINGCHANGE` / `WM_THEMECHANGED` neu eingelesen — mit Vergleich gegen den
letzten Stand, damit fremde Systemhinweise kein Neuzeichnen auslösen.

`"theme": "contrast"` erzwingt die Kontrastdarstellung auch ohne aktives Windows-Kontrastdesign —
für alle, die maximalen Kontrast wollen, ohne das ganze System umzustellen.

### Farbwahl

Die Windows-Akzentfarbe darf beliebig sein — Schwarz, Neongelb und Weiß sind gültige Einstellungen
und als Textfarbe alle unbrauchbar. Deshalb wird nur die **Helligkeit** in ein lesbares Band
gezwungen, der Farbton bleibt unangetastet: der Nutzer erkennt seine Farbe wieder.

Ist der Akzent rot, kollidiert er mit dem Rot für „überfällig" und beide Bedeutungen sind optisch
nicht mehr zu trennen. In dem Fall weicht die Warnfarbe automatisch auf Bernstein oder Magenta aus —
je nachdem, was weiter vom Akzent entfernt liegt. Beides ist durch Unit-Tests abgesichert.
