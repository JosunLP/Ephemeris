// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! What the right-click menu contains, decided once for every platform.
//!
//! The menu is the widget's only settings dialogue, so its contents are a
//! product decision rather than a rendering one: which entries appear at all,
//! which are ticked, which are greyed out, and in what order. None of that is
//! platform-specific — a `HMENU`, an `NSMenu` and a panel drawn with Cairo
//! differ in how they are *shown*, not in what they say.
//!
//! So this module produces the model and each front end presents it. What the
//! front end still owns is the presentation and the platform capability check:
//! [`Inputs::can_autostart`] exists because "start at login" is a registry
//! value on Windows, a login item on macOS and an XDG autostart file on Linux,
//! and a system with no such mechanism should not be offered the entry at all.
//!
//! Acting on a choice stays with the front end too, because every command ends
//! in something platform-specific — opening a folder, moving a window,
//! destroying one. What this module guarantees is that the same choice means
//! the same thing everywhere, and that a new entry cannot be added to one
//! platform and forgotten on the other two.

use crate::i18n::Catalog;

/// What the user picked.
///
/// `#[non_exhaustive]` is deliberately *not* used: a new command must break
/// every front end's `match` until it is handled, which is the whole point of
/// keeping the list here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Fetch now rather than waiting for the interval.
    Sync,
    /// Toggle starting the widget with the session.
    Autostart,
    /// Toggle [`crate::config::Config::locked`].
    Lock,
    /// Open `config.json` in the system editor.
    OpenConfig,
    /// Forget a hand-placed position and go back to the default corner.
    ResetPosition,
    OpenLog,
    OpenDataFolder,
    /// Run the published installer for the release the update check found.
    InstallUpdate,
    /// Put today's agenda on the clipboard as plain text.
    CopyAgenda,
    /// Start the sign-in flow again.
    Relogin,
    Quit,
    /// Select or deselect the calendar at this index in
    /// [`Inputs::calendars`].
    Calendar(usize),
    /// Select or deselect the task list at this index in
    /// [`Inputs::tasklists`].
    Tasklist(usize),
}

/// One clickable line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub label: String,
    pub command: Command,
    /// Draw a tick beside it.
    pub checked: bool,
    /// Greyed out and not selectable.
    ///
    /// Used rather than hiding the entry: an item that disappears looks like a
    /// version difference, whereas a greyed one says "this exists, but not
    /// right now" — which is exactly the message a locked widget should send
    /// about *Reset position*.
    pub enabled: bool,
}

impl Item {
    fn new(label: &str, command: Command) -> Self {
        Self {
            label: label.to_owned(),
            command,
            checked: false,
            enabled: true,
        }
    }

    fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}

/// One line of the menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    Item(Item),
    /// A named list of items, one level deep. Nothing here needs more, and a
    /// menu drawn by hand on Linux is much simpler for the limit being real.
    Submenu(String, Vec<Item>),
    Separator,
}

/// Everything the menu's contents depend on.
///
/// Gathered by the front end because most of it lives behind the sync mutex,
/// and passed as one value so the decisions below have no way to reach for
/// anything else.
#[derive(Clone, Copy)]
pub struct Inputs<'a> {
    pub cat: &'static Catalog,
    /// Does the widget start with the session at the moment?
    pub autostart: bool,
    /// Can this platform arrange that at all? When false the entry is left
    /// out entirely rather than offered and quietly doing nothing.
    pub can_autostart: bool,
    /// Is the position and size currently pinned?
    pub locked: bool,
    /// `(id, display name)` for every calendar the accounts offer.
    pub calendars: &'a [(String, String)],
    pub tasklists: &'a [(String, String)],
    /// The configured selection. Empty means "all of them", which is why it
    /// cannot simply be compared for membership.
    pub selected_calendars: &'a [String],
    pub selected_tasklists: &'a [String],
    /// Did the daily check find a newer release?
    pub update_available: bool,
    /// Is a sign-in flow available? False in demo mode, where there is no sync
    /// thread to send the command to.
    pub can_relogin: bool,
}

/// The full menu, top to bottom.
pub fn context_menu(inputs: &Inputs) -> Vec<Entry> {
    let c = inputs.cat;
    let mut entries = Vec::with_capacity(16);

    entries.push(Entry::Item(Item::new(c.menu_sync, Command::Sync)));
    if inputs.can_autostart {
        entries.push(Entry::Item(
            Item::new(c.menu_autostart, Command::Autostart).checked(inputs.autostart),
        ));
    }

    // Selecting sources straight from the menu: the ids are long, email-like
    // strings, and copying them into the JSON by hand was the most unpleasant
    // part of the setup.
    let sources = [
        (
            c.menu_calendars,
            inputs.calendars,
            inputs.selected_calendars,
            Command::Calendar as fn(usize) -> Command,
        ),
        (
            c.menu_tasklists,
            inputs.tasklists,
            inputs.selected_tasklists,
            Command::Tasklist as fn(usize) -> Command,
        ),
    ];
    let mut any_source = false;
    for (label, entries_of, selected, command) in sources {
        if entries_of.is_empty() {
            continue;
        }
        if !any_source {
            entries.push(Entry::Separator);
            any_source = true;
        }
        let items = entries_of
            .iter()
            .enumerate()
            .map(|(i, (id, name))| {
                // An empty selection means "all", so everything is ticked.
                let checked = selected.is_empty() || selected.iter().any(|s| s == id);
                Item::new(name, command(i)).checked(checked)
            })
            .collect();
        entries.push(Entry::Submenu(label.to_owned(), items));
    }

    entries.push(Entry::Separator);
    // Only offered when there is something to install.
    if inputs.update_available {
        entries.push(Entry::Item(Item::new(
            c.menu_update,
            Command::InstallUpdate,
        )));
    }
    entries.push(Entry::Item(Item::new(c.menu_copy, Command::CopyAgenda)));
    // The lock reads as a state, so the label says what a click would do:
    // "Lock position and size" while it is free, "Unlock" while it is pinned.
    // A single label with a tick beside it would be shorter, but every
    // platform draws that tick differently and half of them not at all in a
    // menu this small.
    entries.push(Entry::Item(Item::new(
        if inputs.locked {
            c.menu_unlock
        } else {
            c.menu_lock
        },
        Command::Lock,
    )));
    entries.push(Entry::Item(Item::new(c.menu_config, Command::OpenConfig)));
    // Moving the widget is exactly what the lock forbids, so this greys out
    // with it. Leaving it live would make the lock a suggestion.
    entries.push(Entry::Item(
        Item::new(c.menu_reset_pos, Command::ResetPosition).enabled(!inputs.locked),
    ));
    entries.push(Entry::Item(Item::new(c.menu_log, Command::OpenLog)));
    entries.push(Entry::Item(Item::new(
        c.menu_folder,
        Command::OpenDataFolder,
    )));
    if inputs.can_relogin {
        entries.push(Entry::Item(Item::new(c.menu_relogin, Command::Relogin)));
    }
    entries.push(Entry::Separator);
    entries.push(Entry::Item(Item::new(c.menu_quit, Command::Quit)));

    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{DE, EN};

    /// `(id, display name)` pairs, as the front ends collect them.
    type Sources = Vec<(String, String)>;

    fn sources() -> (Sources, Sources) {
        (
            vec![
                ("work@example.com".into(), "Work".into()),
                ("home@example.com".into(), "Home".into()),
            ],
            vec![("list-1".into(), "Errands".into())],
        )
    }

    fn inputs<'a>(
        calendars: &'a [(String, String)],
        tasklists: &'a [(String, String)],
        selected: &'a [String],
    ) -> Inputs<'a> {
        Inputs {
            cat: &EN,
            autostart: false,
            can_autostart: true,
            locked: false,
            calendars,
            tasklists,
            selected_calendars: selected,
            selected_tasklists: &[],
            update_available: false,
            can_relogin: true,
        }
    }

    fn items(entries: &[Entry]) -> Vec<&Item> {
        entries
            .iter()
            .flat_map(|e| match e {
                Entry::Item(i) => vec![i],
                Entry::Submenu(_, items) => items.iter().collect(),
                Entry::Separator => Vec::new(),
            })
            .collect()
    }

    fn find(entries: &[Entry], command: Command) -> &Item {
        items(entries)
            .into_iter()
            .find(|i| i.command == command)
            .unwrap_or_else(|| panic!("{command:?} is not in the menu"))
    }

    /// The label has to say what a click does, or the entry is a riddle: with
    /// one fixed label the only difference between the two states would be a
    /// tick, and the widget is not the sort of menu that draws one reliably on
    /// every platform.
    #[test]
    fn the_lock_entry_names_the_action_rather_than_the_state() {
        let (cals, lists) = sources();
        let mut i = inputs(&cals, &lists, &[]);

        assert_eq!(find(&context_menu(&i), Command::Lock).label, EN.menu_lock);
        i.locked = true;
        assert_eq!(find(&context_menu(&i), Command::Lock).label, EN.menu_unlock);

        // And it is translated like everything else.
        i.cat = &DE;
        assert_eq!(find(&context_menu(&i), Command::Lock).label, DE.menu_unlock);
    }

    /// A lock that leaves a way to move the widget is not a lock.
    #[test]
    fn locking_greys_out_the_one_entry_that_would_move_the_widget() {
        let (cals, lists) = sources();
        let mut i = inputs(&cals, &lists, &[]);

        assert!(find(&context_menu(&i), Command::ResetPosition).enabled);
        i.locked = true;
        let menu = context_menu(&i);
        assert!(!find(&menu, Command::ResetPosition).enabled);

        // Everything else keeps working — the widget is locked, not disabled.
        for command in [
            Command::Sync,
            Command::CopyAgenda,
            Command::OpenConfig,
            Command::OpenLog,
            Command::Quit,
        ] {
            assert!(
                find(&menu, command).enabled,
                "{command:?} must survive the lock"
            );
        }
    }

    /// An empty selection means "every source", which is not the same as
    /// "none" — reading it as membership would leave the whole submenu
    /// unticked on a fresh installation.
    #[test]
    fn an_empty_selection_ticks_every_source() {
        let (cals, lists) = sources();
        let all = context_menu(&inputs(&cals, &lists, &[]));
        assert!(
            items(&all).iter().all(|i| !matches!(
                i.command,
                Command::Calendar(_) | Command::Tasklist(_)
            ) || i.checked)
        );

        let one = [cals[1].0.clone()];
        let picked = context_menu(&inputs(&cals, &lists, &one));
        assert!(!find(&picked, Command::Calendar(0)).checked);
        assert!(find(&picked, Command::Calendar(1)).checked);
    }

    /// Nothing to show means no submenu at all, and no stray separator either
    /// — before the first sync there are neither calendars nor task lists.
    #[test]
    fn a_menu_with_nothing_to_offer_has_no_empty_sections() {
        let empty: [(String, String); 0] = [];
        let menu = context_menu(&Inputs {
            can_autostart: false,
            can_relogin: false,
            ..inputs(&empty, &empty, &[])
        });

        assert!(
            !menu
                .iter()
                .any(|e| matches!(e, Entry::Submenu(_, items) if items.is_empty())),
            "an empty submenu would be a dead end"
        );
        assert!(
            !menu.iter().any(|e| matches!(e, Entry::Submenu(..))),
            "no sources means no submenus"
        );
        for command in [Command::Autostart, Command::Relogin] {
            assert!(
                !items(&menu).iter().any(|i| i.command == command),
                "{command:?} must not be offered where it cannot work"
            );
        }

        // No two separators in a row, and none at either end: each of those
        // draws as a stray line.
        assert!(!matches!(menu.first(), Some(Entry::Separator)));
        assert!(!matches!(menu.last(), Some(Entry::Separator)));
        assert!(
            !menu
                .windows(2)
                .any(|w| matches!(w, [Entry::Separator, Entry::Separator])),
            "{menu:?}"
        );
    }

    /// Every command has to be reachable, or a front end could implement one
    /// that no user can ever pick.
    #[test]
    fn every_command_appears_in_some_menu() {
        let (cals, lists) = sources();
        let full = context_menu(&Inputs {
            update_available: true,
            ..inputs(&cals, &lists, &[])
        });
        let reached: Vec<Command> = items(&full).iter().map(|i| i.command).collect();

        for command in [
            Command::Sync,
            Command::Autostart,
            Command::Lock,
            Command::OpenConfig,
            Command::ResetPosition,
            Command::OpenLog,
            Command::OpenDataFolder,
            Command::InstallUpdate,
            Command::CopyAgenda,
            Command::Relogin,
            Command::Quit,
            Command::Calendar(0),
            Command::Tasklist(0),
        ] {
            assert!(reached.contains(&command), "{command:?} is unreachable");
        }
    }

    /// An index in a command addresses [`Inputs::calendars`], so it has to
    /// stay inside it however the selection is written.
    #[test]
    fn source_indices_stay_inside_the_lists_they_address() {
        let (cals, lists) = sources();
        let menu = context_menu(&inputs(&cals, &lists, &["home@example.com".into()]));
        for item in items(&menu) {
            match item.command {
                Command::Calendar(i) => assert!(i < cals.len()),
                Command::Tasklist(i) => assert!(i < lists.len()),
                _ => {}
            }
        }
    }
}
