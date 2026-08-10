// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! The right-click menu as an `NSMenu`.
//!
//! What the menu contains is decided in [`tpmplaner_core::menu`]; this puts it
//! on screen. `-popUpMenuPositioningItem:atLocation:inView:` runs its own
//! tracking loop and does not return until the menu closes, which is exactly
//! the blocking behaviour [`crate::unix::app::Shell::show_menu`] is defined to
//! have — and the same shape `TrackPopupMenu` gives the Windows front end.
//!
//! The choice comes back through a thread-local rather than a return value,
//! because AppKit reports it by sending the item's action to its target. The
//! handler writes a number and nothing else, so it cannot re-enter the widget
//! while the tracking loop still owns the stack.

use crate::unix::mac::objc::*;
use std::cell::Cell;
use tpmplaner_core::menu::{Command, Entry, Item};

thread_local! {
    /// The tag of the item last chosen, if the menu has been closed by
    /// choosing one. Cleared before every menu is shown.
    static CHOICE: Cell<Option<isize>> = const { Cell::new(None) };
}

/// `NSControlStateValueOn` — the tick beside a selected item.
const STATE_ON: isize = 1;

/// Shows the menu at the pointer and returns what was picked.
pub fn show(entries: &[Entry], target: Id) -> Option<Command> {
    let _pool = Pool::new();
    CHOICE.with(|c| c.set(None));

    // Tag zero is what an item with no tag reports, so the commands are
    // numbered from one and the tag indexes this list.
    let mut commands: Vec<Command> = Vec::with_capacity(entries.len());
    let menu = new_menu();
    for entry in entries {
        match entry {
            Entry::Separator => unsafe {
                let separator: Id = send(class(c"NSMenuItem") as Id, c"separatorItem");
                send1::<Id, ()>(menu, c"addItem:", separator);
            },
            Entry::Item(item) => add_item(menu, item, target, &mut commands),
            Entry::Submenu(label, items) => unsafe {
                let sub = new_menu();
                for item in items {
                    add_item(sub, item, target, &mut commands);
                }
                let holder = add_item_raw(menu, label, 0, false, true, nil);
                send1::<Id, ()>(holder, c"setSubmenu:", sub);
            },
        }
    }

    unsafe {
        let location: NSPoint = send(class(c"NSEvent") as Id, c"mouseLocation");
        // `inView:nil` means the location is in screen coordinates, which is
        // what `+mouseLocation` reports — no conversion, and no dependence on
        // which window the pointer happens to be over.
        send3::<Id, NSPoint, Id, i8>(
            menu,
            c"popUpMenuPositioningItem:atLocation:inView:",
            nil,
            location,
            nil,
        );
    }

    let tag = CHOICE.with(|c| c.take())?;
    usize::try_from(tag)
        .ok()?
        .checked_sub(1)
        .and_then(|i| commands.get(i).copied())
}

/// The action every item is wired to.
///
/// Registered on the delegate class in [`crate::unix::mac::window`].
pub extern "C" fn action(_this: Id, _sel: Sel, sender: Id) {
    let tag: isize = unsafe { send(sender, c"tag") };
    CHOICE.with(|c| c.set(Some(tag)));
}

fn new_menu() -> Id {
    unsafe {
        let raw: Id = send(class(c"NSMenu") as Id, c"alloc");
        let menu: Id = send1(raw, c"initWithTitle:", nsstring(""));
        // Without this AppKit decides for itself which items are usable —
        // by asking a validator this program does not have — and greys out
        // everything. It also means a disabled item stays disabled.
        send1::<i8, ()>(menu, c"setAutoenablesItems:", 0);
        // The menu is autoreleased from here on, which is what the caller's
        // pool is for; `-popUpMenuPositioningItem:` retains it for as long as
        // it is on screen.
        send::<()>(menu, c"autorelease");
        menu
    }
}

fn add_item(menu: Id, item: &Item, target: Id, commands: &mut Vec<Command>) {
    commands.push(item.command);
    add_item_raw(
        menu,
        &item.label,
        commands.len() as isize,
        item.checked,
        item.enabled,
        target,
    );
}

/// Appends one item and returns it.
///
/// A submenu holder passes `target = nil` and tag zero: choosing it opens the
/// submenu rather than running anything, and an action would only get in the
/// way.
fn add_item_raw(menu: Id, label: &str, tag: isize, checked: bool, enabled: bool, target: Id) -> Id {
    unsafe {
        let raw: Id = send(class(c"NSMenuItem") as Id, c"alloc");
        let item: Id = send3(
            raw,
            c"initWithTitle:action:keyEquivalent:",
            nsstring(label),
            if target.is_null() {
                std::ptr::null()
            } else {
                sel(c"menuAction:")
            },
            nsstring(""),
        );
        send1::<Id, ()>(item, c"setTarget:", target);
        send1::<isize, ()>(item, c"setTag:", tag);
        send1::<i8, ()>(item, c"setEnabled:", enabled as i8);
        if checked {
            send1::<isize, ()>(item, c"setState:", STATE_ON);
        }
        send1::<Id, ()>(menu, c"addItem:", item);
        // The menu retains it; this reference is the `alloc` one.
        send::<()>(item, c"release");
        item
    }
}
