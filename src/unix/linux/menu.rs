// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! The right-click menu on X11: an override-redirect window with a pointer
//! grab.
//!
//! What the menu contains is [`tpmplaner_core::menu`]'s; what it looks like is
//! [`super::drawn_menu`]'s, shared with the Wayland back end. What is here is
//! the two things X11 does its own way — a window the window manager leaves
//! entirely alone, and a grab that makes a click beside the menu close it.

#![allow(non_upper_case_globals)]

use crate::paint::canvas::Canvas as _;
use crate::unix::linux::canvas::Cairo;
use crate::unix::linux::drawn_menu::{self, Picked, Row};
use crate::unix::linux::ffi::*;
use crate::unix::linux::window::X11Shell;
use std::ffi::{c_int, c_uint};
use tpmplaner_core::log;
use tpmplaner_core::menu::{Command, Entry};

/// Shows the menu at the pointer and returns what was picked.
pub fn show(shell: &mut X11Shell, entries: &[Entry]) -> Option<Command> {
    let mut open: Option<usize> = None;
    loop {
        let rows = drawn_menu::rows_for(entries, open);
        match run_level(shell, &rows)? {
            Picked::Command(command) => return Some(command),
            Picked::Submenu(index) => open = Some(index),
            Picked::Back => open = None,
        }
    }
}

/// Opens one window and runs it until something is chosen or it is dismissed.
fn run_level(shell: &mut X11Shell, rows: &[Row]) -> Option<Picked> {
    let libs = shell.libs.clone();
    let look = shell.look.clone();

    // The size has to be known before the window exists, and measuring needs a
    // canvas — so a throwaway one on the widget's own window does it. Nothing
    // is drawn with that one.
    let mut measure = Cairo::for_window(
        libs.clone(),
        shell.display,
        shell.window,
        shell.visual,
        (1, 1),
        &look,
    )?;
    // A context is what Pango lays out against. Without one every measurement
    // is zero, which is not worth refusing to open a menu over — the rows
    // simply come out at the minimum width.
    if !measure.begin() {
        log::warn("Could not measure the menu — falling back to a minimum width");
    }
    let (width, height) = drawn_menu::size(&mut measure, rows);
    measure.present();
    drop(measure);

    let (x, y) = drawn_menu::place(pointer_on_root(shell), (width, height), shell.screen_size());
    let window = create_popup(shell, x, y, width as c_uint, height as c_uint)?;
    let canvas = Cairo::for_window(
        libs.clone(),
        shell.display,
        window,
        shell.visual,
        (width as c_int, height as c_int),
        &look,
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
                drawn_menu::draw(&mut canvas, rows, width, height, hover, &look.palette);
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
                let next = drawn_menu::row_at(rows, e.y as f32);
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
                if let Some(index) = drawn_menu::row_at(rows, e.y as f32) {
                    result = drawn_menu::pick(rows, index);
                    if result.is_some() {
                        break;
                    }
                }
            }
            // This loop has the display to itself while it runs, so anything
            // addressed to the widget's own window arrives here instead of in
            // the main loop. Almost all of it can wait — an `Expose` is made
            // good by the redraw after the menu closes, and a `ConfigureNotify`
            // by the size the next frame reads back from the server.
            //
            // A selection request cannot. Another application asking for the
            // agenda this widget copied is *blocked* until it is answered or
            // its own timeout runs out, and a menu can be open for a while.
            SelectionRequest => {
                let e: &XSelectionRequestEvent = unsafe { event.as_ref() };
                shell.answer_selection(e);
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
