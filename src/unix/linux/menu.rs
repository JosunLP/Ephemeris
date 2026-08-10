// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! The right-click menu, drawn.
//!
//! X11 has no menus. A menu on a Linux desktop is whatever the toolkit draws,
//! and the widget has deliberately not taken a toolkit — so it draws its own:
//! an override-redirect window, the same Cairo and Pango the panel uses, and a
//! grab so a click anywhere else closes it.
//!
//! That is less work than it sounds, because everything it needs already
//! exists. The entries come from [`tpmplaner_core::menu`], the same list the
//! Windows and macOS menus are built from; the colours come from the palette;
//! the type comes from the same [`Canvas`] the panel is painted with. What is
//! left is arithmetic — one row per entry, as wide as the widest label — and a
//! small event loop.
//!
//! **Submenus open in place.** A second window hovering beside the first is
//! what a toolkit does and needs a whole hierarchy of grabs to get right.
//! Choosing a submenu here replaces the list with its contents and puts a
//! "back" row at the top, which is one state variable and behaves correctly on
//! every window manager.

#![allow(non_upper_case_globals)]

use crate::unix::canvas::{Align, Canvas, Font};
use crate::unix::linux::canvas::Cairo;
use crate::unix::linux::ffi::*;
use crate::unix::linux::window::X11Shell;
use std::ffi::{c_int, c_uint};
use tpmplaner_core::layout::Rect;
use tpmplaner_core::menu::{Command, Entry, Item};
use tpmplaner_core::theme::Palette;

/// Height of one row, and the space around the text, in pixels.
const ROW_H: f32 = 26.0;
const SEPARATOR_H: f32 = 9.0;
const PAD_X: f32 = 14.0;
/// Room on the right for the tick and the submenu arrow.
const MARK_W: f32 = 22.0;
const MIN_W: f32 = 180.0;
const MAX_W: f32 = 420.0;

/// What one drawn row is.
enum Row<'a> {
    Item(&'a Item),
    Separator,
    /// Opens a submenu; the index addresses the entry list.
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

/// Shows the menu at the pointer and returns what was picked.
pub fn show(shell: &mut X11Shell, entries: &[Entry]) -> Option<Command> {
    let mut open: Option<usize> = None;
    loop {
        let rows = rows_for(entries, open);
        match run_level(shell, &rows)? {
            Picked::Command(command) => return Some(command),
            Picked::Submenu(index) => open = Some(index),
            Picked::Back => open = None,
        }
    }
}

enum Picked {
    Command(Command),
    Submenu(usize),
    Back,
}

/// The rows for the top level, or for one open submenu.
fn rows_for(entries: &[Entry], open: Option<usize>) -> Vec<Row<'_>> {
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

/// Opens one window, runs it until something is chosen or it is dismissed.
fn run_level(shell: &mut X11Shell, rows: &[Row]) -> Option<Picked> {
    let libs = shell.libs.clone();
    let palette = shell.look.palette;
    let metrics = shell.look.metrics;
    let appearance = shell.look.appearance.clone();
    let rtl = shell.look.rtl;

    // The size has to be known before the window exists, and measuring needs a
    // canvas — so a throwaway one on the widget's own window does the
    // measuring. Nothing is drawn with it.
    let mut measure = Cairo::new(
        libs.clone(),
        shell.display,
        shell.window,
        shell.visual,
        (1, 1),
        metrics,
        &appearance,
        rtl,
    )?;
    measure.begin();
    let width = rows
        .iter()
        .fold(MIN_W, |acc, row| {
            acc.max(measure.text_width(row.label(), Font::Row) + PAD_X * 2.0 + MARK_W)
        })
        .min(MAX_W);
    let height: f32 = rows.iter().map(Row::height).sum::<f32>() + 8.0;
    measure.present();
    drop(measure);

    let (px, py) = pointer_on_root(shell);
    let (x, y) = place(shell, px, py, width, height);

    let window = create_popup(shell, x, y, width as c_uint, height as c_uint)?;
    let canvas = Cairo::new(
        libs.clone(),
        shell.display,
        window,
        shell.visual,
        (width as c_int, height as c_int),
        metrics,
        &appearance,
        rtl,
    );
    let Some(mut canvas) = canvas else {
        unsafe { (libs.XDestroyWindow)(shell.display, window) };
        return None;
    };

    // A pointer grab is what makes a click beside the menu close it: without
    // one that click goes to whatever is underneath and the menu stays.
    unsafe {
        (libs.XGrabPointer)(
            shell.display,
            window,
            0,
            (ButtonPressMask | ButtonReleaseMask | PointerMotionMask) as c_uint,
            GrabModeAsync,
            GrabModeAsync,
            0,
            0,
            CurrentTime,
        );
        (libs.XFlush)(shell.display);
    }

    let mut hover: Option<usize> = None;
    let mut result = None;
    let mut dirty = true;
    loop {
        if dirty {
            dirty = false;
            if canvas.begin() {
                draw(&mut canvas, rows, width, height, hover, &palette);
                canvas.present();
                unsafe { (libs.XFlush)(shell.display) };
            }
        }

        let mut event = XEvent::default();
        unsafe { (libs.XNextEvent)(shell.display, &mut event) };
        match event.kind() {
            Expose => dirty = true,
            MotionNotify => {
                let e: &XMotionEvent = unsafe { event.as_ref() };
                let next = row_at(rows, e.y as f32);
                if next != hover {
                    hover = next;
                    dirty = true;
                }
            }
            ButtonPress => {
                let e: &XButtonEvent = unsafe { event.as_ref() };
                // A click outside dismisses. With the grab in place the
                // coordinates are still relative to the menu window, so
                // anything off it is negative or past its edge.
                if e.x < 0 || e.y < 0 || e.x as f32 >= width || e.y as f32 >= height {
                    break;
                }
                if let Some(index) = row_at(rows, e.y as f32) {
                    result = match &rows[index] {
                        Row::Item(item) => Some(Picked::Command(item.command)),
                        Row::Submenu(_, entry) => Some(Picked::Submenu(*entry)),
                        Row::Back(_) => Some(Picked::Back),
                        Row::Separator => None,
                    };
                    if result.is_some() {
                        break;
                    }
                }
            }
            _ => {}
        }
    }

    drop(canvas);
    unsafe {
        (libs.XUngrabPointer)(shell.display, CurrentTime);
        (libs.XDestroyWindow)(shell.display, window);
        (libs.XFlush)(shell.display);
    }
    result
}

/// Which row is at this height, if it is a usable one.
fn row_at(rows: &[Row], y: f32) -> Option<usize> {
    let mut top = 4.0;
    for (index, row) in rows.iter().enumerate() {
        let bottom = top + row.height();
        if y >= top && y < bottom {
            return row.enabled().then_some(index);
        }
        top = bottom;
    }
    None
}

fn draw(
    canvas: &mut Cairo,
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
    canvas.fill_round(all, 6.0, palette.panel_top, 1.0);
    canvas.stroke_round(all, 6.0, palette.border_outer, 0.8, 1.0);

    let mut top = 4.0;
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
                let mark_x = width - MARK_W * 0.5;
                let mid = (top + bottom) * 0.5;
                match row {
                    // A tick for a selected source.
                    Row::Item(item) if item.checked => {
                        canvas.line(
                            mark_x - 4.0,
                            mid,
                            mark_x - 1.0,
                            mid + 3.5,
                            palette.accent,
                            1.0,
                            1.8,
                            true,
                        );
                        canvas.line(
                            mark_x - 1.0,
                            mid + 3.5,
                            mark_x + 4.5,
                            mid - 4.0,
                            palette.accent,
                            1.0,
                            1.8,
                            true,
                        );
                    }
                    // An arrow for something that opens, pointing the way it
                    // goes: forwards into a submenu, back out of one.
                    Row::Submenu(..) | Row::Back(_) => {
                        let forwards = matches!(row, Row::Submenu(..));
                        let tip = if forwards { 4.0 } else { -4.0 };
                        canvas.polygon(
                            &[
                                (mark_x + tip, mid),
                                (mark_x - tip, mid - 4.0),
                                (mark_x - tip, mid + 4.0),
                            ],
                            palette.text_dim,
                            0.9,
                        );
                    }
                    _ => {}
                }
            }
        }
        top = bottom;
    }
}

/// Where the menu goes, kept on the screen.
///
/// It opens down and to the right of the pointer, and flips to the other side
/// where there is no room — the behaviour every menu on every desktop has, and
/// the reason a menu near the bottom edge is still usable.
fn place(shell: &X11Shell, px: i32, py: i32, width: f32, height: f32) -> (i32, i32) {
    let (screen_w, screen_h) = shell.screen_size();
    let mut x = px;
    let mut y = py;
    if x + width as i32 > screen_w {
        x = (px - width as i32).max(0);
    }
    if y + height as i32 > screen_h {
        y = (py - height as i32).max(0);
    }
    (x, y)
}

fn pointer_on_root(shell: &X11Shell) -> (i32, i32) {
    let mut root_return: Window = 0;
    let mut child: Window = 0;
    let (mut rx, mut ry, mut wx, mut wy) = (0, 0, 0, 0);
    let mut mask: c_uint = 0;
    unsafe {
        (shell.libs.XQueryPointer)(
            shell.display,
            shell.window,
            &mut root_return,
            &mut child,
            &mut rx,
            &mut ry,
            &mut wx,
            &mut wy,
            &mut mask,
        );
    }
    (rx, ry)
}

/// An override-redirect window: the window manager leaves it entirely alone,
/// which is what a menu wants — no frame, no placement policy, no animation.
fn create_popup(shell: &X11Shell, x: i32, y: i32, width: c_uint, height: c_uint) -> Option<Window> {
    let mut attributes = XSetWindowAttributes {
        background_pixel: 0,
        border_pixel: 0,
        colormap: shell.colormap,
        override_redirect: 1,
        event_mask: ExposureMask | ButtonPressMask | ButtonReleaseMask | PointerMotionMask,
        ..XSetWindowAttributes::default()
    };
    let window = unsafe {
        (shell.libs.XCreateWindow)(
            shell.display,
            shell.root_window(),
            x,
            y,
            width.max(1),
            height.max(1),
            0,
            shell.depth,
            InputOutput,
            shell.visual,
            CWBackPixel | CWBorderPixel | CWColormap | CWEventMask | CWOverrideRedirect,
            &mut attributes,
        )
    };
    if window == 0 {
        return None;
    }
    unsafe {
        (shell.libs.XMapRaised)(shell.display, window);
        (shell.libs.XFlush)(shell.display);
    }
    Some(window)
}
