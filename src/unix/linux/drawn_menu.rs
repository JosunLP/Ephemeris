// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Ephemeris contributors
//! The right-click menu's rows, measurements and drawing.
//!
//! Linux has no menus to ask for. X11 never had any, and Wayland deliberately
//! has none — a menu on a Linux desktop is whatever the toolkit draws, and this
//! widget carries no toolkit. So it draws its own, and both Linux back ends
//! draw the same one: the rows, the arithmetic and the painting are here, and
//! the two window systems differ only in what they put it on.
//!
//! What each back end still owns is the surface and the dismissal. X11 uses an
//! override-redirect window with a pointer grab; Wayland uses an `xdg_popup`,
//! which is the only construct in that protocol that comes with a grab and a
//! "the user clicked elsewhere" event. Neither of those has anything to do with
//! what a menu *looks* like, which is why the split falls here.
//!
//! **Submenus open in place.** A second window hovering beside the first is
//! what a toolkit does and needs a whole hierarchy of grabs to get right.
//! Choosing a submenu replaces the list with its contents and puts a row back
//! to the top level, which is one state variable and behaves the same on both.

use crate::paint::canvas::{Align, Canvas, Font};
use ephemeris_core::layout::Rect;
use ephemeris_core::menu::{Command, Entry, Item};
use ephemeris_core::theme::Palette;

/// Height of one row and of a separator, and the space around the text, in
/// pixels.
const ROW_H: f32 = 26.0;
const SEPARATOR_H: f32 = 9.0;
const PAD_X: f32 = 14.0;
/// Room on the right for the tick and the submenu arrow.
const MARK_W: f32 = 22.0;
/// A margin at the top and bottom, so the first and last rows do not sit
/// against the border.
const PAD_Y: f32 = 4.0;
const MIN_W: f32 = 180.0;
const MAX_W: f32 = 420.0;

/// What one drawn row is.
pub enum Row<'a> {
    Item(&'a Item),
    Separator,
    /// Opens a submenu; the index addresses the entry list it came from.
    Submenu(&'a str, usize),
    /// Goes back from a submenu to the top level.
    Back(&'a str),
}

impl Row<'_> {
    fn height(&self) -> f32 {
        match self {
            Row::Separator => SEPARATOR_H,
            _ => ROW_H,
        }
    }

    fn label(&self) -> &str {
        match self {
            Row::Item(item) => &item.label,
            Row::Submenu(label, _) | Row::Back(label) => label,
            Row::Separator => "",
        }
    }

    fn enabled(&self) -> bool {
        match self {
            Row::Item(item) => item.enabled,
            Row::Separator => false,
            _ => true,
        }
    }
}

/// What clicking a row means.
pub enum Picked {
    Command(Command),
    /// Open the submenu at this index in the entry list.
    Submenu(usize),
    /// Leave a submenu again.
    Back,
}

/// The rows for the top level, or for one open submenu.
pub fn rows_for(entries: &[Entry], open: Option<usize>) -> Vec<Row<'_>> {
    if let Some(index) = open
        && let Some(Entry::Submenu(label, items)) = entries.get(index)
    {
        let mut rows = vec![Row::Back(label), Row::Separator];
        rows.extend(items.iter().map(Row::Item));
        return rows;
    }
    entries
        .iter()
        .enumerate()
        .map(|(i, entry)| match entry {
            Entry::Item(item) => Row::Item(item),
            Entry::Separator => Row::Separator,
            Entry::Submenu(label, _) => Row::Submenu(label, i),
        })
        .collect()
}

/// How big the menu has to be, in pixels.
///
/// The width comes from the widest label, because a menu that truncates
/// "Unlock position and size" is a menu nobody can read. The cap stops one
/// long calendar name making the whole thing the width of the screen.
pub fn size(canvas: &mut impl Canvas, rows: &[Row]) -> (f32, f32) {
    let width = rows
        .iter()
        .fold(MIN_W, |acc, row| {
            acc.max(canvas.text_width(row.label(), Font::Row) + PAD_X * 2.0 + MARK_W)
        })
        .min(MAX_W);
    let height = rows.iter().map(Row::height).sum::<f32>() + PAD_Y * 2.0;
    (width, height)
}

/// Which row is at this height, if it is one that can be chosen.
///
/// `None` for a separator, for a greyed-out entry and for the padding at
/// either end — all three are places where a click should do nothing rather
/// than pick the nearest row.
pub fn row_at(rows: &[Row], y: f32) -> Option<usize> {
    let mut top = PAD_Y;
    for (index, row) in rows.iter().enumerate() {
        let bottom = top + row.height();
        if y >= top && y < bottom {
            return row.enabled().then_some(index);
        }
        top = bottom;
    }
    None
}

/// What choosing the row at `index` means.
pub fn pick(rows: &[Row], index: usize) -> Option<Picked> {
    match rows.get(index)? {
        Row::Item(item) => Some(Picked::Command(item.command)),
        Row::Submenu(_, entry) => Some(Picked::Submenu(*entry)),
        Row::Back(_) => Some(Picked::Back),
        Row::Separator => None,
    }
}

/// Paints the menu, filling the whole surface it was given.
pub fn draw(
    canvas: &mut impl Canvas,
    rows: &[Row],
    width: f32,
    height: f32,
    hover: Option<usize>,
    palette: &Palette,
) {
    canvas.clear();
    let all = Rect {
        left: 0.0,
        top: 0.0,
        right: width,
        bottom: height,
    };
    // Opaque, unlike the panel: a menu has to be readable over whatever it
    // happens to be covering, and the panel's own translucency is a property
    // of the widget rather than of everything it draws.
    canvas.fill_round(all, 6.0, palette.panel_top, 1.0);
    canvas.stroke_round(all, 6.0, palette.border_outer, 0.8, 1.0);

    let mut top = PAD_Y;
    for (index, row) in rows.iter().enumerate() {
        let bottom = top + row.height();
        match row {
            Row::Separator => {
                let y = (top + bottom) * 0.5;
                canvas.line(
                    PAD_X * 0.5,
                    y,
                    width - PAD_X * 0.5,
                    y,
                    palette.rule,
                    palette.rule_alpha,
                    1.0,
                    false,
                );
            }
            _ => {
                if hover == Some(index) {
                    canvas.fill_round(
                        Rect {
                            left: 3.0,
                            top,
                            right: width - 3.0,
                            bottom,
                        },
                        4.0,
                        palette.hover,
                        palette.hover_alpha * 1.6,
                    );
                }
                let color = if row.enabled() {
                    palette.text_primary
                } else {
                    palette.text_faint
                };
                canvas.text(
                    row.label(),
                    Font::Row,
                    Align::Left,
                    Rect {
                        left: PAD_X,
                        top,
                        right: width - MARK_W,
                        bottom,
                    },
                    color,
                    1.0,
                );
                draw_mark(canvas, row, width, (top + bottom) * 0.5, palette);
            }
        }
        top = bottom;
    }
}

/// The tick beside a selected source, or the arrow beside something that
/// opens.
fn draw_mark(canvas: &mut impl Canvas, row: &Row, width: f32, mid: f32, palette: &Palette) {
    let x = width - MARK_W * 0.5;
    match row {
        Row::Item(item) if item.checked => {
            canvas.line(
                x - 4.0,
                mid,
                x - 1.0,
                mid + 3.5,
                palette.accent,
                1.0,
                1.8,
                true,
            );
            canvas.line(
                x - 1.0,
                mid + 3.5,
                x + 4.5,
                mid - 4.0,
                palette.accent,
                1.0,
                1.8,
                true,
            );
        }
        // Pointing the way it goes: forwards into a submenu, back out of one.
        Row::Submenu(..) | Row::Back(_) => {
            let tip = if matches!(row, Row::Submenu(..)) {
                4.0
            } else {
                -4.0
            };
            canvas.polygon(
                &[(x + tip, mid), (x - tip, mid - 4.0), (x - tip, mid + 4.0)],
                palette.text_dim,
                0.9,
            );
        }
        _ => {}
    }
}

/// Keeps a menu of this size on the screen when it opens at the pointer.
///
/// It opens down and to the right, and flips to the other side where there is
/// no room — the behaviour every menu on every desktop has, and the reason one
/// opened near the bottom edge is still usable.
pub fn place(pointer: (i32, i32), size: (f32, f32), screen: (i32, i32)) -> (i32, i32) {
    let (width, height) = (size.0 as i32, size.1 as i32);
    let x = if pointer.0 + width > screen.0 {
        (pointer.0 - width).max(0)
    } else {
        pointer.0
    };
    let y = if pointer.1 + height > screen.1 {
        (pointer.1 - height).max(0)
    } else {
        pointer.1
    };
    (x, y)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ephemeris_core::menu::Command;

    fn item(label: &str, enabled: bool) -> Item {
        Item {
            label: label.to_owned(),
            command: Command::Sync,
            checked: false,
            enabled,
        }
    }

    /// A click has to land on the row the user is looking at. The padding at
    /// the top is what makes this easy to get wrong by one row.
    #[test]
    fn a_click_lands_on_the_row_under_it() {
        let first = item("first", true);
        let second = item("second", true);
        let rows = vec![Row::Item(&first), Row::Separator, Row::Item(&second)];

        // Above the first row: the border, not a row.
        assert!(row_at(&rows, 0.0).is_none());
        assert_eq!(row_at(&rows, PAD_Y + 1.0), Some(0));
        assert_eq!(row_at(&rows, PAD_Y + ROW_H - 0.1), Some(0));
        // The separator is not selectable, and neither is the space it takes.
        assert!(row_at(&rows, PAD_Y + ROW_H + 1.0).is_none());
        assert_eq!(row_at(&rows, PAD_Y + ROW_H + SEPARATOR_H + 1.0), Some(2));
        // Past the last row.
        assert!(row_at(&rows, 1000.0).is_none());
    }

    /// A greyed-out entry — *Reset position* while the widget is locked — has
    /// to be unclickable, not merely grey.
    #[test]
    fn a_disabled_row_cannot_be_chosen() {
        let off = item("locked away", false);
        let rows = vec![Row::Item(&off)];
        assert!(row_at(&rows, PAD_Y + 1.0).is_none());
    }

    /// A menu near an edge flips rather than hanging off it.
    #[test]
    fn a_menu_near_an_edge_opens_the_other_way() {
        let screen = (1920, 1080);
        let size = (200.0, 300.0);

        // Room for it: straight down and to the right of the pointer.
        assert_eq!(place((100, 100), size, screen), (100, 100));
        // No room on the right, so it opens leftwards.
        assert_eq!(place((1900, 100), size, screen), (1700, 100));
        // Nor at the bottom.
        assert_eq!(place((100, 1000), size, screen), (100, 700));
        // A screen too small for it at all still starts on it rather than
        // off the top left.
        assert_eq!(place((10, 10), (400.0, 2000.0), (300, 300)), (0, 0));
    }

    /// Opening a submenu replaces the list and offers a way back; the index
    /// still addresses the original entries.
    #[test]
    fn a_submenu_replaces_the_list_and_offers_a_way_back() {
        let entries = vec![
            Entry::Item(item("Sync now", true)),
            Entry::Submenu("Calendars".into(), vec![item("Work", true)]),
        ];

        let top = rows_for(&entries, None);
        assert_eq!(top.len(), 2);
        assert!(matches!(top[1], Row::Submenu("Calendars", 1)));

        let inside = rows_for(&entries, Some(1));
        assert!(matches!(inside[0], Row::Back("Calendars")));
        assert!(matches!(inside[1], Row::Separator));
        assert_eq!(inside.len(), 3);
        assert!(matches!(pick(&inside, 0), Some(Picked::Back)));

        // An index that is not a submenu falls back to the top level rather
        // than showing an empty menu.
        assert_eq!(rows_for(&entries, Some(0)).len(), 2);
    }
}
