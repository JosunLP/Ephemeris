// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! The right-click menu on Wayland: an `xdg_popup` with a grab.
//!
//! What the menu contains is [`tpmplaner_core::menu`]'s and what it looks like
//! is [`crate::unix::linux::drawn_menu`]'s, shared with the X11 back end. What
//! is here is the one construct in Wayland that behaves like a menu.
//!
//! **Why a popup and not another layer surface.** A layer surface would be
//! easier — the widget already makes one — but it has no grab, and without a
//! grab a click beside the menu goes to whatever is underneath and the menu
//! stays open with no way to know it should not have. `xdg_popup` is the only
//! object in the protocol that takes the pointer for the duration and reports
//! `popup_done` when the user clicks elsewhere. A layer surface can adopt one
//! through `zwlr_layer_surface_v1.get_popup`, which is exactly what that
//! request exists for.
//!
//! **The positioner does the placement.** On X11 the widget works out whether
//! a menu near the bottom of the screen has to flip; here the compositor does,
//! because it is the only one that knows where the surface actually is. That
//! is what `set_constraint_adjustment` asks for.

use super::ffi::*;
use super::window::{PointerEvent, Shape, WaylandShell};
use crate::unix::canvas::Canvas as _;
use crate::unix::linux::canvas::Cairo;
use crate::unix::linux::drawn_menu::{self, Picked, Row};
use std::ffi::c_void;
use std::time::Duration;
use tpmplaner_core::log;
use tpmplaner_core::menu::{Command, Entry};

/// Shows the menu at the pointer and returns what was picked.
pub fn show(shell: &mut WaylandShell, entries: &[Entry]) -> Option<Command> {
    if shell.connection.xdg_wm_base.is_null() {
        log::warn("This compositor has no xdg_shell, so the widget cannot open its menu");
        return None;
    }
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

/// One popup, from creation to dismissal.
fn run_level(shell: &mut WaylandShell, rows: &[Row]) -> Option<Picked> {
    let (width, height) = measure(shell, rows)?;
    let mut popup = Popup::open(shell, width, height)?;
    let outcome = popup.run(shell, rows, width, height);
    popup.close(shell);
    outcome
}

/// How big the menu wants to be.
///
/// Measuring needs a Pango context, which needs a Cairo surface. A one-pixel
/// image surface is the cheapest thing that provides one, and nothing is ever
/// drawn on it.
fn measure(shell: &mut WaylandShell, rows: &[Row]) -> Option<(i32, i32)> {
    let mut scratch = [0u8; 4];
    let mut canvas = unsafe {
        Cairo::for_pixels(
            shell.libs.clone(),
            scratch.as_mut_ptr(),
            (1, 1),
            4,
            &shell.look,
        )
    }?;
    if !canvas.begin() {
        return None;
    }
    let (width, height) = drawn_menu::size(&mut canvas, rows);
    canvas.present();
    Some((width.ceil() as i32, height.ceil() as i32))
}

/// The four objects a popup is made of, and the buffer it draws into.
struct Popup {
    surface: *mut wl_proxy,
    xdg_surface: *mut wl_proxy,
    xdg_popup: *mut wl_proxy,
    buffer: Option<super::window::SingleBuffer>,
    configured: bool,
    done: bool,
}

impl Popup {
    fn open(shell: &mut WaylandShell, width: i32, height: i32) -> Option<Self> {
        let connection = &shell.connection;
        let wl = connection.wl.clone();

        let surface = connection.create(
            connection.compositor,
            wl_compositor::CREATE_SURFACE,
            wl.wl_surface_interface,
            wl.version(connection.compositor),
        );
        if surface.is_null() {
            return None;
        }

        let positioner = connection.create(
            connection.xdg_wm_base,
            xdg_wm_base::CREATE_POSITIONER,
            &XDG_POSITIONER_INTERFACE,
            wl.version(connection.xdg_wm_base),
        );
        if positioner.is_null() {
            connection.destroy(surface);
            return None;
        }
        // The anchor is a one-pixel rectangle at the pointer, in the parent
        // surface's own coordinates, and the popup hangs down and to the right
        // of it. `set_constraint_adjustment` is what lets the compositor slide
        // or flip it to keep it on the screen — the job X11 makes the client
        // do by hand.
        let (x, y) = (
            shell.pointer_at.0.round() as i32,
            shell.pointer_at.1.round() as i32,
        );
        connection.send2(positioner, xdg_positioner::SET_SIZE, width, height);
        connection.send4(positioner, xdg_positioner::SET_ANCHOR_RECT, x, y, 1, 1);
        connection.send1(positioner, xdg_positioner::SET_ANCHOR, XDG_ANCHOR_TOP_LEFT);
        connection.send1(
            positioner,
            xdg_positioner::SET_GRAVITY,
            XDG_GRAVITY_BOTTOM_RIGHT,
        );
        connection.send1(
            positioner,
            xdg_positioner::SET_CONSTRAINT_ADJUSTMENT,
            XDG_CONSTRAINT_ADJUST,
        );

        let xdg_surface = connection.create1(
            connection.xdg_wm_base,
            xdg_wm_base::GET_XDG_SURFACE,
            &XDG_SURFACE_INTERFACE,
            surface,
        );
        if xdg_surface.is_null() {
            connection.send(positioner, xdg_positioner::DESTROY);
            connection.destroy(surface);
            return None;
        }
        unsafe { connection.listen(xdg_surface, super::window::xdg_surface_listener(), 1) };

        // A layer surface adopts the popup with a request of its own; a
        // toplevel is the popup's parent in the ordinary way.
        let parent = match shell.shape {
            Shape::Layer => std::ptr::null_mut(),
            Shape::Toplevel => shell.xdg_surface(),
        };
        let xdg_popup = unsafe {
            (wl.wl_proxy_marshal_flags)(
                xdg_surface,
                xdg_surface::GET_POPUP,
                &XDG_POPUP_INTERFACE,
                wl.version(xdg_surface),
                0,
                std::ptr::null_mut::<c_void>(),
                parent,
                positioner,
            )
        };
        connection.send(positioner, xdg_positioner::DESTROY);
        connection.destroy(positioner);
        if xdg_popup.is_null() {
            connection.destroy(xdg_surface);
            connection.destroy(surface);
            return None;
        }
        unsafe { connection.listen(xdg_popup, super::window::xdg_popup_listener(), 0) };

        if shell.shape == Shape::Layer && !shell.layer_surface.is_null() {
            connection.send1(shell.layer_surface, layer_surface::GET_POPUP, xdg_popup);
        }
        // The grab is the whole reason for using a popup: it takes the pointer
        // for the duration and turns a click anywhere else into `popup_done`.
        connection.send2(
            xdg_popup,
            xdg_popup::GRAB,
            connection.seat,
            shell.last_serial,
        );
        connection.send(surface, wl_surface::COMMIT);

        Some(Self {
            surface,
            xdg_surface,
            xdg_popup,
            buffer: None,
            configured: false,
            done: false,
        })
    }

    /// Pumps the connection until a row is chosen or the popup is dismissed.
    fn run(
        &mut self,
        shell: &mut WaylandShell,
        rows: &[Row],
        width: i32,
        height: i32,
    ) -> Option<Picked> {
        let mut hover: Option<usize> = None;
        let mut dirty = true;
        let mut at = (0.0f64, 0.0f64);

        loop {
            if dirty && self.configured {
                dirty = false;
                self.draw(shell, rows, width, height, hover);
            }
            if self.done {
                return None;
            }

            // The widget's own loop is not running while this one is, so the
            // connection is pumped here. Everything that arrives for the
            // widget rather than the popup waits in the queue, which is
            // exactly where a modal menu should leave it.
            if !shell.pump(Duration::from_millis(50)) {
                return None;
            }

            let (popup_done, configured) = super::window::take_popup_state();
            if configured {
                self.acknowledge(shell);
                dirty = true;
            }
            if popup_done {
                self.done = true;
                return None;
            }

            for event in super::window::take_pointer_events() {
                match event {
                    PointerEvent::Enter(serial, surface, x, y) if surface == self.surface => {
                        shell.note_serial(serial);
                        at = (x, y);
                        let next = drawn_menu::row_at(rows, y as f32);
                        if next != hover {
                            hover = next;
                            dirty = true;
                        }
                    }
                    PointerEvent::Leave(surface)
                        if surface == self.surface && hover.take().is_some() =>
                    {
                        dirty = true;
                    }
                    PointerEvent::Motion(x, y) => {
                        at = (x, y);
                        let next = drawn_menu::row_at(rows, y as f32);
                        if next != hover {
                            hover = next;
                            dirty = true;
                        }
                    }
                    PointerEvent::Button(serial, button, down) => {
                        shell.note_serial(serial);
                        if button != BTN_LEFT || !down {
                            continue;
                        }
                        // Off the popup entirely: the compositor will send
                        // `popup_done` for it, but answering here as well
                        // keeps a compositor that is slower about it from
                        // leaving the menu on screen.
                        if at.0 < 0.0 || at.1 < 0.0 || at.0 >= width as f64 || at.1 >= height as f64
                        {
                            return None;
                        }
                        if let Some(index) = drawn_menu::row_at(rows, at.1 as f32)
                            && let Some(picked) = drawn_menu::pick(rows, index)
                        {
                            return Some(picked);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    fn acknowledge(&mut self, shell: &WaylandShell) {
        if let Some(serial) = super::window::take_xdg_configure() {
            shell
                .connection
                .send1(self.xdg_surface, xdg_surface::ACK_CONFIGURE, serial);
        }
        self.configured = true;
    }

    fn draw(
        &mut self,
        shell: &mut WaylandShell,
        rows: &[Row],
        width: i32,
        height: i32,
        hover: Option<usize>,
    ) {
        if self.buffer.is_none() {
            self.buffer = super::window::SingleBuffer::new(
                &shell.connection,
                shell.libs.clone(),
                (width, height),
                shell.buffer_scale(),
                &shell.look,
            );
        }
        let Some(buffer) = self.buffer.as_mut() else {
            return;
        };
        let Some(canvas) = buffer.canvas() else {
            return;
        };
        if !canvas.begin() {
            return;
        }
        drawn_menu::draw(
            canvas,
            rows,
            width as f32,
            height as f32,
            hover,
            &shell.look.palette,
        );
        canvas.present();

        let wl_buffer = buffer.buffer();
        shell
            .connection
            .send3(self.surface, wl_surface::ATTACH, wl_buffer, 0i32, 0i32);
        shell.connection.send4(
            self.surface,
            wl_surface::DAMAGE,
            0i32,
            0i32,
            i32::MAX,
            i32::MAX,
        );
        shell.connection.send(self.surface, wl_surface::COMMIT);
    }

    fn close(&mut self, shell: &mut WaylandShell) {
        let connection = &shell.connection;
        self.buffer = None;
        if !self.xdg_popup.is_null() {
            connection.send(self.xdg_popup, xdg_popup::DESTROY);
            connection.destroy(self.xdg_popup);
        }
        if !self.xdg_surface.is_null() {
            connection.send(self.xdg_surface, xdg_surface::DESTROY);
            connection.destroy(self.xdg_surface);
        }
        if !self.surface.is_null() {
            connection.send(self.surface, wl_surface::DESTROY);
            connection.destroy(self.surface);
        }
        // The widget's own surface is still there and has not been drawn for
        // as long as the menu was up.
        shell.request_redraw_now();
    }
}
