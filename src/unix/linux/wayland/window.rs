// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! The widget's surface on Wayland.
//!
//! **`wlr-layer-shell` is the whole point.** It is the only Wayland protocol
//! that lets a client say "above the wallpaper, below every ordinary window",
//! which is what the widget is defined by. On a compositor that has it — Sway,
//! Hyprland, river, Wayfire, KDE Plasma — the surface goes on the bottom layer
//! with keyboard interactivity off, and behaves exactly as the Win32 and
//! AppKit windows do.
//!
//! **Where there is none, the widget says so.** GNOME's Mutter does not
//! implement the layer shell and has said it will not. There the surface falls
//! back to an ordinary `xdg_toplevel` and two things change: it sits among the
//! windows rather than behind them, and it cannot place itself, because
//! Wayland gives no client that power — dragging is handed to the compositor
//! with `xdg_toplevel.move` instead. Both go in the log at start-up. Quietly
//! degrading was rejected in `docs/development/porting.md` and is still
//! rejected: a widget that stops doing the one thing it is for should say
//! which.
//!
//! **Drawing** is Cairo into shared memory the compositor reads directly.
//! `CAIRO_FORMAT_ARGB32` and `WL_SHM_FORMAT_ARGB8888` are the same bytes on a
//! little-endian machine, so there is no conversion. Two buffers, so a frame
//! can be drawn while the compositor still holds the last one.
//!
//! **Events arrive on C callbacks with no context**, so every listener does
//! one thing: append to [`Events`], which the loop drains. That is
//! deliberately duller than reaching back into the widget — a listener fires
//! inside `wl_display_dispatch_pending`, which the loop itself called, and
//! anything that re-entered the widget there would borrow it twice.

#![allow(non_upper_case_globals)]

use super::cursor::{CursorLib, Cursors};
use super::ffi::*;
use super::menu;
use crate::paint::canvas::Canvas as _;
use crate::paint::widget as paint;
use crate::unix::app::{self, App, Cursor as AppCursor, Shell, WindowRect};
use crate::unix::autostart;
use crate::unix::linux::canvas::{Cairo, Look};
use crate::unix::linux::ffi::Libs;
use crate::unix::linux::visuals;
use std::cell::RefCell;
use std::ffi::{CStr, c_char, c_int, c_void};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tpmplaner_core::host::Waker;
use tpmplaner_core::log;
use tpmplaner_core::theme::SystemVisuals;

/// How the surface is shown — the one thing a compositor may not let the
/// widget choose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// `wlr-layer-shell`, bottom layer. What the widget is meant to be.
    Layer,
    /// An ordinary window, because the compositor has no layer shell.
    Toplevel,
}

thread_local! {
    /// What the listeners saw since the loop last looked.
    static EVENTS: RefCell<Events> = RefCell::new(Events::default());
}

#[derive(Default)]
struct Events {
    globals: Vec<(u32, String, u32)>,
    /// `serial, width, height` from the layer surface.
    layer_configure: Option<(u32, u32, u32)>,
    /// A serial from `xdg_surface.configure`, which has to be acknowledged
    /// before the surface may show anything.
    xdg_configure: Option<u32>,
    /// A size the compositor chose for the toplevel; zero means "you decide".
    toplevel_size: Option<(i32, i32)>,
    /// `xdg_wm_base.ping`, which has to be answered or the client is killed.
    ping: Option<u32>,
    closed: bool,
    pointer: Vec<PointerEvent>,
    /// Buffer slots the compositor has finished with.
    released: Vec<usize>,
    seat_has_pointer: bool,
    output_size: Option<(i32, i32)>,
    output_scale: Option<i32>,
    /// Somebody asked for the clipboard: what format, and where to write it.
    clipboard_requests: Vec<(String, c_int)>,
    clipboard_cancelled: bool,
    /// The popup's own configure serial, kept apart from the widget's so a
    /// menu opening cannot be mistaken for the window being reconfigured.
    popup_configure: Option<u32>,
    popup_done: bool,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum PointerEvent {
    /// `serial`, the surface entered, and where on it.
    Enter(u32, *mut wl_proxy, f64, f64),
    Leave(*mut wl_proxy),
    Motion(f64, f64),
    /// `serial`, the Linux button code, and whether it went down.
    Button(u32, u32, bool),
    /// Vertical scroll, in the protocol's own units.
    Axis(f64),
}

/// `wl_fixed_t` is a 24.8 fixed-point integer.
fn fixed(value: i32) -> f64 {
    value as f64 / 256.0
}

/// Wakes the loop by writing a byte to a pipe the `poll` is watching.
///
/// Not a Wayland object: a display connection belongs to the thread that owns
/// it unless the client sets up its own event queues, and a pipe needs no such
/// arrangement.
struct PipeWaker(c_int);

impl Waker for PipeWaker {
    fn wake(&self) {
        let byte = 1u8;
        unsafe {
            libc::write(self.0, &byte as *const u8 as *const c_void, 1);
        }
    }
}

/// The connection and everything bound out of the registry.
pub struct Connection {
    pub wl: Rc<Wl>,
    pub display: *mut WlDisplay,
    pub registry: *mut wl_proxy,
    pub compositor: *mut wl_proxy,
    pub shm: *mut wl_proxy,
    pub seat: *mut wl_proxy,
    pub pointer: *mut wl_proxy,
    pub layer_shell: *mut wl_proxy,
    pub xdg_wm_base: *mut wl_proxy,
    pub data_device_manager: *mut wl_proxy,
    pub data_device: *mut wl_proxy,
    pub output: *mut wl_proxy,
}

impl Connection {
    /// A request with no new object and no arguments.
    pub fn send(&self, proxy: *mut wl_proxy, opcode: u32) {
        unsafe {
            (self.wl.wl_proxy_marshal_flags)(
                proxy,
                opcode,
                std::ptr::null(),
                self.wl.version(proxy),
                0,
            );
        }
    }

    /// A request whose arguments are all machine words: integers, strings
    /// already terminated, or objects the compositor knows.
    pub fn send1<A>(&self, proxy: *mut wl_proxy, opcode: u32, a: A) {
        unsafe {
            let f = self.wl.wl_proxy_marshal_flags;
            f(
                proxy,
                opcode,
                std::ptr::null(),
                self.wl.version(proxy),
                0,
                a,
            );
        }
    }

    pub fn send2<A, B>(&self, proxy: *mut wl_proxy, opcode: u32, a: A, b: B) {
        unsafe {
            let f = self.wl.wl_proxy_marshal_flags;
            let v = self.wl.version(proxy);
            f(proxy, opcode, std::ptr::null(), v, 0, a, b);
        }
    }

    pub fn send3<A, B, C>(&self, proxy: *mut wl_proxy, opcode: u32, a: A, b: B, c: C) {
        unsafe {
            let f = self.wl.wl_proxy_marshal_flags;
            let v = self.wl.version(proxy);
            f(proxy, opcode, std::ptr::null(), v, 0, a, b, c);
        }
    }

    pub fn send4<A, B, C, D>(&self, proxy: *mut wl_proxy, opcode: u32, a: A, b: B, c: C, d: D) {
        unsafe {
            let f = self.wl.wl_proxy_marshal_flags;
            let v = self.wl.version(proxy);
            f(proxy, opcode, std::ptr::null(), v, 0, a, b, c, d);
        }
    }

    /// A request that creates an object and takes none of its own.
    pub fn create(
        &self,
        proxy: *mut wl_proxy,
        opcode: u32,
        interface: *const WlInterface,
        version: u32,
    ) -> *mut wl_proxy {
        unsafe {
            let f = self.wl.wl_proxy_marshal_flags;
            f(
                proxy,
                opcode,
                interface,
                version,
                0,
                std::ptr::null_mut::<c_void>(),
            )
        }
    }

    /// A request that creates an object and takes one of its own.
    pub fn create1<A>(
        &self,
        proxy: *mut wl_proxy,
        opcode: u32,
        interface: *const WlInterface,
        a: A,
    ) -> *mut wl_proxy {
        unsafe {
            let f = self.wl.wl_proxy_marshal_flags;
            let v = self.wl.version(proxy);
            f(
                proxy,
                opcode,
                interface,
                v,
                0,
                std::ptr::null_mut::<c_void>(),
                a,
            )
        }
    }

    pub fn destroy(&self, proxy: *mut wl_proxy) {
        if !proxy.is_null() {
            unsafe { (self.wl.wl_proxy_destroy)(proxy) };
        }
    }

    /// Attaches a listener: an array of function pointers indexed by event
    /// opcode.
    ///
    /// # Safety
    ///
    /// `listener` must hold one entry for every event the object's interface
    /// declares, in order, each matching that event's signature.
    pub unsafe fn listen<L>(&self, proxy: *mut wl_proxy, listener: &'static L, data: usize) {
        unsafe {
            (self.wl.wl_proxy_add_listener)(
                proxy,
                listener as *const L as *mut *mut c_void,
                data as *mut c_void,
            );
        }
    }
}

// --- Buffers ----------------------------------------------------------------

/// A pair of shared-memory buffers and the Cairo canvas over each.
///
/// Two, so the next frame can be drawn while the compositor still holds the
/// last one. One would work — the widget draws on change rather than
/// continuously — but would halve the frame rate of the reveal animation for
/// a saving not worth having.
struct Buffers {
    /// Kept so the pool and the buffers can be given back when this set is
    /// replaced, which happens on every resize and every rebuild.
    wl: Rc<Wl>,
    fd: c_int,
    memory: *mut u8,
    length: usize,
    pool: *mut wl_proxy,
    slots: [Slot; 2],
    /// The logical size, and the density the pixels were actually allocated
    /// at.
    size: (i32, i32),
    scale: i32,
}

struct Slot {
    buffer: *mut wl_proxy,
    canvas: Option<Cairo>,
    offset: usize,
    /// The compositor still has this one.
    busy: bool,
}

impl Buffers {
    fn new(
        connection: &Connection,
        libs: Arc<Libs>,
        size: (i32, i32),
        scale: i32,
        look: &Look,
    ) -> Option<Self> {
        let scale = scale.max(1);
        let (width, height) = (size.0.max(1) * scale, size.1.max(1) * scale);
        let stride = width * 4;
        let one = (stride as usize) * (height as usize);
        let length = one * 2;

        // `memfd` rather than a file under `/tmp`: nothing is left behind, the
        // memory is anonymous, and there is no path for anything else to open.
        let fd = unsafe { libc::memfd_create(c"tpmplaner".as_ptr(), libc::MFD_CLOEXEC) };
        if fd < 0 {
            log::error("Could not create a shared-memory buffer for the widget");
            return None;
        }
        if unsafe { libc::ftruncate(fd, length as libc::off_t) } != 0 {
            unsafe { libc::close(fd) };
            log::error("Could not size the widget's shared-memory buffer");
            return None;
        }
        let mapped = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                length,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd,
                0,
            )
        };
        if mapped == libc::MAP_FAILED {
            unsafe { libc::close(fd) };
            log::error("Could not map the widget's shared-memory buffer");
            return None;
        }
        let memory = mapped as *mut u8;

        // `create_pool(new_id, fd, size)`. The file descriptor is one of the
        // three argument shapes the helpers do not cover.
        let wl = &connection.wl;
        let pool = unsafe {
            (wl.wl_proxy_marshal_flags)(
                connection.shm,
                wl_shm::CREATE_POOL,
                wl.wl_shm_pool_interface,
                wl.version(connection.shm),
                0,
                std::ptr::null_mut::<c_void>(),
                fd,
                length as i32,
            )
        };
        if pool.is_null() {
            unsafe {
                libc::munmap(mapped, length);
                libc::close(fd);
            }
            return None;
        }

        let mut me = Self {
            wl: wl.clone(),
            fd,
            memory,
            length,
            pool,
            slots: [
                Slot {
                    buffer: std::ptr::null_mut(),
                    canvas: None,
                    offset: 0,
                    busy: false,
                },
                Slot {
                    buffer: std::ptr::null_mut(),
                    canvas: None,
                    offset: one,
                    busy: false,
                },
            ],
            size,
            scale,
        };

        for index in 0..2 {
            let offset = me.slots[index].offset;
            let buffer = unsafe {
                (wl.wl_proxy_marshal_flags)(
                    pool,
                    wl_shm_pool::CREATE_BUFFER,
                    wl.wl_buffer_interface,
                    wl.version(pool),
                    0,
                    std::ptr::null_mut::<c_void>(),
                    offset as i32,
                    width,
                    height,
                    stride,
                    SHM_FORMAT_ARGB8888,
                )
            };
            if buffer.is_null() {
                return None;
            }
            unsafe { connection.listen(buffer, &BUFFER_LISTENER, index) };

            let mut canvas = unsafe {
                Cairo::for_pixels(
                    libs.clone(),
                    memory.add(offset),
                    (width, height),
                    stride,
                    look,
                )
            }?;
            canvas.set_scale(scale as f64);
            me.slots[index].buffer = buffer;
            me.slots[index].canvas = Some(canvas);
        }
        Some(me)
    }

    /// A slot the compositor is not reading, if there is one.
    fn free_slot(&self) -> Option<usize> {
        (0..2).find(|&i| !self.slots[i].busy)
    }
}

impl Drop for Buffers {
    fn drop(&mut self) {
        // The canvases point into the mapping, so they go first.
        for slot in &mut self.slots {
            slot.canvas = None;
        }
        // The compositor keeps its own mapping of the memfd for as long as the
        // pool object lives, so unmapping our side is only half of it. A set of
        // buffers is replaced on every resize, scale change and theme rebuild
        // — three seconds of dragging the resize corner would otherwise pin
        // hundreds of megabytes in the compositor for the life of the process.
        //
        // Destroying the buffers first also takes their listeners with them,
        // which is what stops a late `release` for a discarded buffer from
        // clearing `busy` on the slot that replaced it: the listener carries a
        // slot index, and libwayland discards events for a destroyed proxy.
        unsafe {
            for slot in &mut self.slots {
                if slot.buffer.is_null() {
                    continue;
                }
                (self.wl.wl_proxy_marshal_flags)(
                    slot.buffer,
                    wl_buffer::DESTROY,
                    std::ptr::null(),
                    self.wl.version(slot.buffer),
                    0,
                );
                (self.wl.wl_proxy_destroy)(slot.buffer);
                slot.buffer = std::ptr::null_mut();
            }
            if !self.pool.is_null() {
                // The pool's mapping is released once the buffers made from it
                // are gone, so this is safe even while one is still on screen.
                (self.wl.wl_proxy_marshal_flags)(
                    self.pool,
                    wl_shm_pool::DESTROY,
                    std::ptr::null(),
                    self.wl.version(self.pool),
                    0,
                );
                (self.wl.wl_proxy_destroy)(self.pool);
                self.pool = std::ptr::null_mut();
            }
            libc::munmap(self.memory as *mut c_void, self.length);
            libc::close(self.fd);
        }
    }
}

// --- The shell --------------------------------------------------------------

pub struct WaylandShell {
    pub connection: Connection,
    pub libs: Arc<Libs>,
    pub surface: *mut wl_proxy,
    /// The layer surface, or null under the toplevel fallback.
    pub layer_surface: *mut wl_proxy,
    xdg_surface: *mut wl_proxy,
    xdg_toplevel: *mut wl_proxy,
    pub shape: Shape,
    pub look: Look,

    buffers: Option<Buffers>,
    /// What the widget asked for, in logical pixels. Authoritative on a layer
    /// surface; on a toplevel the position half is a wish the compositor
    /// ignores, and the widget is told so.
    rect: WindowRect,
    output: (i32, i32),
    output_scale: i32,

    /// The serial of the most recent `wl_pointer.enter`, which is the only one
    /// `set_cursor` accepts.
    enter_serial: u32,
    /// The most recent serial from any input event, which is what a popup and
    /// a clipboard offer have to quote.
    pub last_serial: u32,
    /// Where the pointer is on the surface, in logical pixels.
    pub pointer_at: (f64, f64),
    cursors: Cursors,
    cursor_lib: Option<CursorLib>,

    clipboard: Arc<String>,
    data_source: *mut wl_proxy,

    animating: bool,
    undo_running: bool,
    next_minute: Instant,
    quit: bool,
    pub rebuild: bool,
    dirty: bool,
    /// The compositor has acknowledged the surface, so a buffer may be
    /// attached. Nothing may be shown before this.
    configured: bool,
}

/// Builds the surface and runs the event loop.
pub fn run() -> Result<(), String> {
    let libs = Arc::new(Libs::load().ok_or("the desktop's libraries could not be loaded")?);
    // `Rc`, not `Arc`: a Wayland display connection belongs to the thread
    // that opened it, and everything reached through it belongs there too.
    let wl = Rc::new(Wl::load().ok_or("libwayland-client could not be loaded")?);

    let display = unsafe { (wl.wl_display_connect)(std::ptr::null()) };
    if display.is_null() {
        return Err("could not connect to the Wayland compositor".into());
    }

    let mut connection = Connection {
        wl: wl.clone(),
        display,
        registry: std::ptr::null_mut(),
        compositor: std::ptr::null_mut(),
        shm: std::ptr::null_mut(),
        seat: std::ptr::null_mut(),
        pointer: std::ptr::null_mut(),
        layer_shell: std::ptr::null_mut(),
        xdg_wm_base: std::ptr::null_mut(),
        data_device_manager: std::ptr::null_mut(),
        data_device: std::ptr::null_mut(),
        output: std::ptr::null_mut(),
    };

    // One round trip to hear every global, then bind what is wanted. Binding
    // inside the listener would mean threading the connection through a C
    // callback for no gain.
    connection.registry = connection.create(
        display,
        wl_display::GET_REGISTRY,
        wl.wl_registry_interface,
        1,
    );
    unsafe { connection.listen(connection.registry, &REGISTRY_LISTENER, 0) };
    unsafe { (wl.wl_display_roundtrip)(display) };
    bind_globals(&mut connection);

    if connection.compositor.is_null() || connection.shm.is_null() {
        return Err("this compositor offers no wl_compositor or wl_shm".into());
    }
    if connection.layer_shell.is_null() && connection.xdg_wm_base.is_null() {
        return Err("this compositor offers neither wlr-layer-shell nor xdg_shell".into());
    }

    let shape = if connection.layer_shell.is_null() {
        Shape::Toplevel
    } else {
        Shape::Layer
    };
    super::report_shape(shape);

    let (wake_read, wake_write) = pipe().ok_or("could not create the wake pipe")?;
    let (mut widget, cfg) = App::new(visuals::read(), Arc::new(PipeWaker(wake_write)));

    let surface = connection.create(
        connection.compositor,
        wl_compositor::CREATE_SURFACE,
        wl.wl_surface_interface,
        wl.version(connection.compositor),
    );
    if surface.is_null() {
        return Err("the compositor refused a surface".into());
    }

    let (output, output_scale) = EVENTS.with(|e| {
        let events = e.borrow();
        (
            events.output_size.unwrap_or((0, 0)),
            events.output_scale.unwrap_or(1).max(1),
        )
    });

    let mut shell = WaylandShell {
        look: Look {
            metrics: widget.metrics,
            palette: widget.palette,
            appearance: widget.appearance.clone(),
            rtl: widget.loc.rtl,
        },
        connection,
        libs: libs.clone(),
        surface,
        layer_surface: std::ptr::null_mut(),
        xdg_surface: std::ptr::null_mut(),
        xdg_toplevel: std::ptr::null_mut(),
        shape,
        buffers: None,
        rect: WindowRect {
            x: 0,
            y: 0,
            width: 400,
            height: 640,
        },
        output,
        output_scale,
        enter_serial: 0,
        last_serial: 0,
        pointer_at: (0.0, 0.0),
        cursors: Cursors::default(),
        cursor_lib: CursorLib::load(),
        clipboard: Arc::new(String::new()),
        data_source: std::ptr::null_mut(),
        animating: false,
        undo_running: false,
        next_minute: next_minute_boundary(),
        quit: false,
        rebuild: false,
        dirty: true,
        configured: false,
    };

    shell.rect = widget.target_geometry(&cfg, &shell);
    shell.create_role();
    {
        let (wl, shm, compositor) = (
            shell.connection.wl.clone(),
            shell.connection.shm,
            shell.connection.compositor,
        );
        let lib = shell.cursor_lib.take();
        shell
            .cursors
            .load(&wl, lib.as_ref(), shm, compositor, shell.output_scale);
        shell.cursor_lib = lib;
    }
    // Nothing may be attached before the compositor has configured the
    // surface, so the first commit is empty: it is what *asks* for that
    // configure.
    shell.connection.send(shell.surface, wl_surface::COMMIT);
    unsafe { (wl.wl_display_roundtrip)(display) };

    super::report_hotkey(&cfg.peek_hotkey);
    event_loop(&mut widget, &mut shell, wake_read);

    widget.commit_pending_on_exit();
    widget.shut_down();
    shell.buffers = None;
    let lib = shell.cursor_lib.take();
    shell.cursors.destroy(&wl, lib.as_ref());
    unsafe { (wl.wl_display_disconnect)(display) };
    Ok(())
}

/// Binds the globals the widget uses out of what the registry announced.
///
/// Each at the lowest version that carries what is needed, so a compositor
/// offering only that much still works.
fn bind_globals(connection: &mut Connection) {
    let globals = EVENTS.with(|e| std::mem::take(&mut e.borrow_mut().globals));
    let wl = connection.wl.clone();

    for (name, interface, offered) in globals {
        // Which field, which description, and the highest version the widget
        // knows what to do with.
        let (which, want, most): (Global, *const WlInterface, u32) = match interface.as_str() {
            "wl_compositor" => (Global::Compositor, wl.wl_compositor_interface, 4),
            "wl_shm" => (Global::Shm, wl.wl_shm_interface, 1),
            "wl_seat" => (Global::Seat, wl.wl_seat_interface, 1),
            "wl_output" => (Global::Output, wl.wl_output_interface, 2),
            // Version 2 is the lowest that carries `set_layer`, and the layer
            // surface inherits the shell's version — bound at 1, the peek could
            // never raise the widget off the bottom layer, which is the whole
            // of what the peek shortcut does under Wayland.
            "zwlr_layer_shell_v1" => (Global::LayerShell, &LAYER_SHELL_INTERFACE, 2),
            "xdg_wm_base" => (Global::XdgWmBase, &XDG_WM_BASE_INTERFACE, 1),
            "wl_data_device_manager" => (
                Global::DataDeviceManager,
                wl.wl_data_device_manager_interface,
                1,
            ),
            _ => continue,
        };
        if !which.slot(connection).is_null() {
            continue;
        }
        let proxy = bind(connection, name, want, offered.min(most));
        *which.slot(connection) = proxy;
    }

    if !connection.seat.is_null() {
        unsafe { connection.listen(connection.seat, &SEAT_LISTENER, 0) };
    }
    if !connection.output.is_null() {
        unsafe { connection.listen(connection.output, &OUTPUT_LISTENER, 0) };
    }
    if !connection.xdg_wm_base.is_null() {
        unsafe { connection.listen(connection.xdg_wm_base, &XDG_WM_BASE_LISTENER, 0) };
    }
    if !connection.data_device_manager.is_null() && !connection.seat.is_null() {
        connection.data_device = connection.create1(
            connection.data_device_manager,
            wl_data_device_manager::GET_DATA_DEVICE,
            wl.wl_data_device_interface,
            connection.seat,
        );
    }

    // A second round trip, so the seat's capabilities and the output's mode
    // have arrived before the first frame is sized.
    unsafe { (connection.wl.wl_display_roundtrip)(connection.display) };
    take_pointer(connection);
}

/// The globals the widget binds, so the field to fill in can be chosen
/// before the connection is borrowed to do the binding.
#[derive(Clone, Copy)]
enum Global {
    Compositor,
    Shm,
    Seat,
    Output,
    LayerShell,
    XdgWmBase,
    DataDeviceManager,
}

impl Global {
    fn slot(self, connection: &mut Connection) -> &mut *mut wl_proxy {
        match self {
            Global::Compositor => &mut connection.compositor,
            Global::Shm => &mut connection.shm,
            Global::Seat => &mut connection.seat,
            Global::Output => &mut connection.output,
            Global::LayerShell => &mut connection.layer_shell,
            Global::XdgWmBase => &mut connection.xdg_wm_base,
            Global::DataDeviceManager => &mut connection.data_device_manager,
        }
    }
}

/// `wl_registry.bind(name, interface_name, version, new_id)`.
fn bind(
    connection: &Connection,
    name: u32,
    interface: *const WlInterface,
    version: u32,
) -> *mut wl_proxy {
    let wl = &connection.wl;
    unsafe {
        (wl.wl_proxy_marshal_flags)(
            connection.registry,
            wl_registry::BIND,
            interface,
            version,
            0,
            name,
            (*interface).name,
            version,
            std::ptr::null_mut::<c_void>(),
        )
    }
}

fn take_pointer(connection: &mut Connection) {
    let has_pointer = EVENTS.with(|e| e.borrow().seat_has_pointer);
    if !has_pointer || connection.seat.is_null() || !connection.pointer.is_null() {
        return;
    }
    connection.pointer = connection.create(
        connection.seat,
        wl_seat::GET_POINTER,
        connection.wl.wl_pointer_interface,
        connection.wl.version(connection.seat),
    );
    if !connection.pointer.is_null() {
        unsafe { connection.listen(connection.pointer, &POINTER_LISTENER, 0) };
    }
}

impl WaylandShell {
    /// Gives the surface its role — the thing that decides where it appears.
    fn create_role(&mut self) {
        match self.shape {
            Shape::Layer => self.create_layer_surface(),
            Shape::Toplevel => self.create_toplevel(),
        }
    }

    fn create_layer_surface(&mut self) {
        let wl = self.connection.wl.clone();
        let namespace = c"tpmplaner";
        // `get_layer_surface(new_id, surface, output, layer, namespace)`. The
        // output is null: "wherever the compositor thinks best", which for a
        // single-monitor desktop is the only monitor and for several is the
        // one the user is on.
        self.layer_surface = unsafe {
            (wl.wl_proxy_marshal_flags)(
                self.connection.layer_shell,
                layer_shell::GET_LAYER_SURFACE,
                &LAYER_SURFACE_INTERFACE,
                wl.version(self.connection.layer_shell),
                0,
                std::ptr::null_mut::<c_void>(),
                self.surface,
                std::ptr::null_mut::<c_void>(),
                LAYER_BOTTOM,
                namespace.as_ptr(),
            )
        };
        if self.layer_surface.is_null() {
            log::error("The compositor refused a layer surface");
            return;
        }
        unsafe {
            self.connection
                .listen(self.layer_surface, &LAYER_SURFACE_LISTENER, 0)
        };
        self.connection.send1(
            self.layer_surface,
            layer_surface::SET_KEYBOARD_INTERACTIVITY,
            KEYBOARD_NONE,
        );
        // Minus one: "place me as though no panel had reserved anything".
        // Without it a dock at the top of the screen would push the widget
        // down and the saved position would mean something different on every
        // desktop.
        self.connection
            .send1(self.layer_surface, layer_surface::SET_EXCLUSIVE_ZONE, -1i32);
        self.apply_layer_geometry();
    }

    /// A layer surface has no position: it has an anchor and a margin. Anchored
    /// to the top left, the margin *is* the position, which is what makes the
    /// saved `x` and `y` mean the same thing here as everywhere else.
    fn apply_layer_geometry(&self) {
        if self.layer_surface.is_null() {
            return;
        }
        self.connection.send2(
            self.layer_surface,
            layer_surface::SET_SIZE,
            self.rect.width.max(1) as u32,
            self.rect.height.max(1) as u32,
        );
        self.connection.send1(
            self.layer_surface,
            layer_surface::SET_ANCHOR,
            ANCHOR_TOP | ANCHOR_LEFT,
        );
        self.connection.send4(
            self.layer_surface,
            layer_surface::SET_MARGIN,
            self.rect.y,
            0i32,
            0i32,
            self.rect.x,
        );
    }

    fn create_toplevel(&mut self) {
        let wl = self.connection.wl.clone();
        self.xdg_surface = self.connection.create1(
            self.connection.xdg_wm_base,
            xdg_wm_base::GET_XDG_SURFACE,
            &XDG_SURFACE_INTERFACE,
            self.surface,
        );
        if self.xdg_surface.is_null() {
            log::error("The compositor refused an xdg_surface");
            return;
        }
        unsafe {
            self.connection
                .listen(self.xdg_surface, &XDG_SURFACE_LISTENER, 0)
        };
        self.xdg_toplevel = self.connection.create(
            self.xdg_surface,
            xdg_surface::GET_TOPLEVEL,
            &XDG_TOPLEVEL_INTERFACE,
            wl.version(self.xdg_surface),
        );
        if self.xdg_toplevel.is_null() {
            return;
        }
        unsafe {
            self.connection
                .listen(self.xdg_toplevel, &XDG_TOPLEVEL_LISTENER, 0)
        };
        self.connection.send1(
            self.xdg_toplevel,
            xdg_toplevel::SET_TITLE,
            c"TPMPlaner".as_ptr(),
        );
        // The application identifier a desktop matches against its `.desktop`
        // file, which is what gives the window the right icon and groups it
        // with the autostart entry.
        self.connection.send1(
            self.xdg_toplevel,
            xdg_toplevel::SET_APP_ID,
            c"tpmplaner".as_ptr(),
        );
        self.connection.send2(
            self.xdg_toplevel,
            xdg_toplevel::SET_MIN_SIZE,
            240i32,
            180i32,
        );
    }

    /// Rebuilds the buffers when the size, the density or the typography
    /// changed.
    fn ensure_buffers(&mut self) {
        let wanted = (self.rect.width.max(1), self.rect.height.max(1));
        let stale = match &self.buffers {
            Some(b) => b.size != wanted || b.scale != self.output_scale,
            None => true,
        };
        if !stale && !self.rebuild {
            return;
        }
        self.rebuild = false;
        self.buffers = Buffers::new(
            &self.connection,
            self.libs.clone(),
            wanted,
            self.output_scale,
            &self.look,
        );
        if self.buffers.is_none() {
            log::error("The widget could not allocate its drawing buffers");
        }
        // The compositor has to be told the buffer is denser than the surface,
        // or a high-density display shows it at a quarter of the size. Sent
        // whenever the buffers are rebuilt rather than only when the scale is
        // above one: the surface keeps whatever it was last told, so a move
        // from 200 % back to 100 % would leave it at 2 while the buffers are
        // allocated at 1 — half size, and a fatal `invalid_size` the moment
        // the width is odd.
        if self.connection.wl.version(self.surface) >= 3 {
            self.connection.send1(
                self.surface,
                wl_surface::SET_BUFFER_SCALE,
                self.output_scale.max(1),
            );
        }
    }

    /// Draws one frame into a free buffer and shows it.
    ///
    /// Returns false when every buffer is still with the compositor, in which
    /// case the frame is owed and [`Self::dirty`] stays set — the release
    /// event brings the loop back round.
    fn draw(&mut self, app: &mut App) -> bool {
        if !self.configured {
            return false;
        }
        self.ensure_buffers();
        let Some(buffers) = self.buffers.as_mut() else {
            return true;
        };
        let Some(index) = buffers.free_slot() else {
            return false;
        };

        let size = (buffers.size.0 as f32, buffers.size.1 as f32);
        let Some(canvas) = buffers.slots[index].canvas.as_mut() else {
            return true;
        };
        if !canvas.begin() {
            return true;
        }
        let (metrics, palette, rtl) = (app.metrics, app.palette, app.loc.rtl);
        let appearance = app.appearance.clone();
        let result = app.with_frame(|frame, hits| {
            paint::draw(
                canvas,
                size,
                metrics,
                palette,
                &appearance,
                rtl,
                frame,
                hits,
            )
        });
        canvas.present();
        app.frame_drawn(result.content_height, result.viewport_height);

        let buffer = buffers.slots[index].buffer;
        buffers.slots[index].busy = true;
        self.connection
            .send3(self.surface, wl_surface::ATTACH, buffer, 0i32, 0i32);
        self.connection.send4(
            self.surface,
            wl_surface::DAMAGE,
            0i32,
            0i32,
            i32::MAX,
            i32::MAX,
        );
        self.connection.send(self.surface, wl_surface::COMMIT);
        true
    }

    /// Keeps the look the drawn menu and the buffers are built from in step
    /// with the widget.
    fn sync_look(&mut self, app: &App) {
        let fresh = Look {
            metrics: app.metrics,
            palette: app.palette,
            appearance: app.appearance.clone(),
            rtl: app.loc.rtl,
        };
        self.look = fresh;
    }

    /// The pointer's position on the output, which is what a drag and a menu
    /// need.
    ///
    /// Wayland never reports one: a client is told where the pointer is on its
    /// own surface and nothing more. Adding the surface's own position gives
    /// the same number back, and it stays right *while* the surface moves,
    /// because the two change in opposite directions by the same amount.
    pub fn pointer_on_output(&self) -> (i32, i32) {
        (
            self.rect.x + self.pointer_at.0.round() as i32,
            self.rect.y + self.pointer_at.1.round() as i32,
        )
    }

    pub fn output_size(&self) -> (i32, i32) {
        if self.output == (0, 0) {
            (1920, 1080)
        } else {
            self.output
        }
    }

    fn offer_clipboard(&mut self) {
        if self.connection.data_device.is_null() || self.connection.data_device_manager.is_null() {
            return;
        }
        // A previous offer is replaced rather than added to.
        if !self.data_source.is_null() {
            self.connection
                .send(self.data_source, wl_data_source::DESTROY);
            self.connection.destroy(self.data_source);
        }
        self.data_source = self.connection.create(
            self.connection.data_device_manager,
            wl_data_device_manager::CREATE_DATA_SOURCE,
            self.connection.wl.wl_data_source_interface,
            self.connection
                .wl
                .version(self.connection.data_device_manager),
        );
        if self.data_source.is_null() {
            return;
        }
        unsafe {
            self.connection
                .listen(self.data_source, &DATA_SOURCE_LISTENER, 0)
        };
        for mime in [c"text/plain;charset=utf-8", c"text/plain", c"UTF8_STRING"] {
            self.connection
                .send1(self.data_source, wl_data_source::OFFER, mime.as_ptr());
        }
        self.connection.send2(
            self.connection.data_device,
            wl_data_device::SET_SELECTION,
            self.data_source,
            self.last_serial,
        );
    }

    /// Writes the agenda to whoever asked for it and closes the pipe.
    ///
    /// The compositor hands over a file descriptor and steps out of the way;
    /// the two clients talk through it directly. It is set non-blocking and
    /// written once — a paste target that has gone away leaves a broken pipe,
    /// which is not this program's problem to report.
    fn answer_clipboard(&self, fd: c_int) {
        let text = self.clipboard.clone();
        unsafe {
            let mut written = 0usize;
            while written < text.len() {
                let n = libc::write(
                    fd,
                    text.as_ptr().add(written) as *const c_void,
                    text.len() - written,
                );
                if n <= 0 {
                    break;
                }
                written += n as usize;
            }
            libc::close(fd);
        }
    }
}

// --- The loop ---------------------------------------------------------------

fn event_loop(app: &mut App, shell: &mut WaylandShell, wake: c_int) {
    let wl = shell.connection.wl.clone();
    let display = shell.connection.display;
    let wayland_fd = unsafe { (wl.wl_display_get_fd)(display) };

    loop {
        if shell.quit {
            return;
        }
        shell.sync_look(app);
        if shell.dirty && shell.draw(app) {
            shell.dirty = false;
        }

        // The documented way to wait on a Wayland connection alongside
        // anything else: announce the intention to read, flush, sleep, then
        // either read or cancel. Skipping it races — another thread or a
        // nested dispatch could consume the socket in between.
        // Every one of these is checked, as `WaylandShell::pump` checks them:
        // once the compositor is gone the socket is permanently at end of
        // file, so `poll` returns at once, the reads fail, and a loop that
        // discarded the results would spin at full speed and never quit.
        let mut lost = false;
        unsafe {
            while (wl.wl_display_prepare_read)(display) != 0 {
                if (wl.wl_display_dispatch_pending)(display) < 0 {
                    lost = true;
                    break;
                }
            }
            if !lost
                && (wl.wl_display_flush)(display) < 0
                && std::io::Error::last_os_error().raw_os_error() != Some(libc::EAGAIN)
            {
                (wl.wl_display_cancel_read)(display);
                lost = true;
            }
        }
        if lost {
            shell.quit = true;
            return;
        }

        let timeout = shell
            .next_deadline(app.peek_deadline())
            .map(|d| d.saturating_duration_since(Instant::now()))
            .unwrap_or(Duration::from_secs(60));
        let readable = wait(wayland_fd, wake, timeout);

        unsafe {
            if readable {
                if (wl.wl_display_read_events)(display) < 0 {
                    lost = true;
                }
            } else {
                (wl.wl_display_cancel_read)(display);
            }
            if !lost && (wl.wl_display_dispatch_pending)(display) < 0 {
                lost = true;
            }
        }
        if lost {
            log::warn("The Wayland connection was lost — closing.");
            shell.quit = true;
            return;
        }

        if drain(wake) {
            if visuals::appearance_changed() {
                app.refresh_palette(shell);
                shell.rebuild = true;
            }
            app.on_sync_done(shell);
            shell.dirty = true;
        }

        handle_events(app, shell);

        let now = Instant::now();
        if shell.animating {
            app.pump(shell);
            shell.dirty = true;
        }
        if shell.undo_running {
            app.on_undo_tick(shell);
            shell.dirty = true;
        }
        if now >= shell.next_minute {
            shell.next_minute = next_minute_boundary();
            app.on_minute(shell);
            shell.dirty = true;
        }
        if app.peek_deadline().is_some_and(|d| now >= d) {
            app.end_peek(shell);
            shell.dirty = true;
        }
    }
}

/// Drains everything the listeners recorded and turns it into widget calls.
fn handle_events(app: &mut App, shell: &mut WaylandShell) {
    let events = EVENTS.with(|e| std::mem::take(&mut *e.borrow_mut()));

    if let Some(serial) = events.ping {
        shell
            .connection
            .send1(shell.connection.xdg_wm_base, xdg_wm_base::PONG, serial);
    }
    if events.closed {
        shell.quit = true;
        return;
    }
    for slot in events.released {
        if let Some(buffers) = shell.buffers.as_mut()
            && let Some(s) = buffers.slots.get_mut(slot)
        {
            s.busy = false;
        }
        shell.dirty = true;
    }
    if let Some(scale) = events.output_scale
        && scale.max(1) != shell.output_scale
    {
        shell.output_scale = scale.max(1);
        shell.rebuild = true;
        shell.dirty = true;
    }
    if let Some(size) = events.output_size {
        shell.output = size;
    }

    if let Some((serial, width, height)) = events.layer_configure {
        shell
            .connection
            .send1(shell.layer_surface, layer_surface::ACK_CONFIGURE, serial);
        // Zero means "the size you asked for", which is already `rect`.
        if width > 0 && height > 0 {
            shell.rect.width = width as i32;
            shell.rect.height = height as i32;
        }
        shell.configured = true;
        shell.dirty = true;
    }
    if let Some((width, height)) = events.toplevel_size
        && width > 0
        && height > 0
    {
        shell.rect.width = width;
        shell.rect.height = height;
    }
    if let Some(serial) = events.xdg_configure {
        shell
            .connection
            .send1(shell.xdg_surface, xdg_surface::ACK_CONFIGURE, serial);
        shell.configured = true;
        shell.dirty = true;
    }

    for request in events.clipboard_requests {
        shell.answer_clipboard(request.1);
    }
    if events.clipboard_cancelled && !shell.data_source.is_null() {
        shell
            .connection
            .send(shell.data_source, wl_data_source::DESTROY);
        shell.connection.destroy(shell.data_source);
        shell.data_source = std::ptr::null_mut();
    }

    let mut menu_wanted = false;
    for event in events.pointer {
        match event {
            PointerEvent::Enter(serial, surface, x, y) => {
                if surface != shell.surface {
                    continue;
                }
                shell.enter_serial = serial;
                shell.last_serial = serial;
                shell.pointer_at = (x, y);
                shell.cursors.forget();
                // A compositor forgets a client's cursor between visits, so
                // the shape has to be set again on every entry.
                shell.show_cursor(AppCursor::Arrow);
                let at = shell.pointer_on_output();
                app.on_pointer_move(shell, x as f32, y as f32, at);
            }
            PointerEvent::Leave(surface) => {
                if surface != shell.surface {
                    continue;
                }
                shell.cursors.forget();
                app.on_pointer_leave(shell);
            }
            PointerEvent::Motion(x, y) => {
                shell.pointer_at = (x, y);
                let at = shell.pointer_on_output();
                app.on_pointer_move(shell, x as f32, y as f32, at);
            }
            PointerEvent::Button(serial, button, down) => {
                shell.last_serial = serial;
                match (button, down) {
                    (BTN_LEFT, true) => {
                        let (x, y) = shell.pointer_at;
                        let at = shell.pointer_on_output();
                        app.on_press(shell, x as f32, y as f32, at);
                    }
                    (BTN_LEFT, false) => app.on_release(shell),
                    // Opened after this loop: the menu runs a nested
                    // dispatch, and the remaining events belong to it.
                    (BTN_RIGHT, true) => menu_wanted = true,
                    _ => {}
                }
            }
            PointerEvent::Axis(value) => {
                // The protocol counts pixels downwards; the widget counts
                // notches upwards, and ten pixels is one notch on every
                // wheel this has been tried on.
                app.on_scroll(shell, (-value / 10.0) as f32);
            }
        }
        shell.dirty = true;
    }

    if menu_wanted {
        app.show_menu(shell);
        shell.dirty = true;
    }
}

impl WaylandShell {
    fn next_deadline(&self, peek: Option<Instant>) -> Option<Instant> {
        [
            self.animating.then(|| Instant::now() + app::ANIM_INTERVAL),
            self.undo_running
                .then(|| Instant::now() + app::UNDO_INTERVAL),
            Some(self.next_minute),
            peek,
        ]
        .into_iter()
        .flatten()
        .min()
    }

    fn show_cursor(&mut self, cursor: AppCursor) {
        let wl = self.connection.wl.clone();
        self.cursors
            .show(&wl, self.connection.pointer, cursor, self.enter_serial);
    }

    /// The widget's own `xdg_surface`, which a popup on a toplevel needs as
    /// its parent. Null under the layer shell, where the layer surface adopts
    /// the popup instead.
    pub(super) fn xdg_surface(&self) -> *mut wl_proxy {
        self.xdg_surface
    }

    pub(super) fn buffer_scale(&self) -> i32 {
        self.output_scale
    }

    /// Records a serial from an input event the menu handled, so the next
    /// request that needs one quotes something recent enough.
    pub(super) fn note_serial(&mut self, serial: u32) {
        if serial != 0 {
            self.last_serial = serial;
        }
    }

    /// Redraws as soon as the loop comes round, which is what a menu closing
    /// over the widget asks for.
    pub(super) fn request_redraw_now(&mut self) {
        self.dirty = true;
    }

    /// Services the connection once, for a loop that is not the main one.
    ///
    /// False if the connection has gone, which is the one thing a nested loop
    /// must not keep spinning on.
    pub(super) fn pump(&mut self, timeout: Duration) -> bool {
        let wl = self.connection.wl.clone();
        let display = self.connection.display;
        let fd = unsafe { (wl.wl_display_get_fd)(display) };
        unsafe {
            while (wl.wl_display_prepare_read)(display) != 0 {
                if (wl.wl_display_dispatch_pending)(display) < 0 {
                    return false;
                }
            }
            if (wl.wl_display_flush)(display) < 0
                && std::io::Error::last_os_error().raw_os_error() != Some(libc::EAGAIN)
            {
                (wl.wl_display_cancel_read)(display);
                return false;
            }
            let readable = wait(fd, -1, timeout);
            if readable {
                if (wl.wl_display_read_events)(display) < 0 {
                    return false;
                }
            } else {
                (wl.wl_display_cancel_read)(display);
            }
            (wl.wl_display_dispatch_pending)(display) >= 0
        }
    }
}

impl Shell for WaylandShell {
    fn window_rect(&self) -> WindowRect {
        self.rect
    }

    fn set_window_rect(&mut self, rect: WindowRect) {
        let resized = (rect.width, rect.height) != (self.rect.width, self.rect.height);
        match self.shape {
            Shape::Layer => {
                self.rect = rect;
                self.apply_layer_geometry();
                self.connection.send(self.surface, wl_surface::COMMIT);
            }
            // Wayland gives no client the power to place its own window, so a
            // toplevel keeps whatever the compositor decided and takes only
            // the size. `begin_system_drag` is how moving it works instead.
            Shape::Toplevel => {
                self.rect.width = rect.width;
                self.rect.height = rect.height;
            }
        }
        if resized {
            self.dirty = true;
        }
    }

    /// One buffer pixel per logical pixel as far as the widget is concerned.
    /// A high-density display is handled by allocating a denser buffer and
    /// scaling the Cairo context, which nothing above this has to know about.
    fn scale(&self) -> f32 {
        1.0
    }

    fn work_area(&self) -> Option<WindowRect> {
        let (width, height) = self.output_size();
        Some(WindowRect {
            x: 0,
            y: 0,
            width,
            height,
        })
    }

    fn is_on_screen(&self, rect: WindowRect) -> bool {
        // A toplevel is placed by the compositor, which never puts a window
        // off the screen — so there is nothing to rescue it from.
        if self.shape == Shape::Toplevel {
            return true;
        }
        let (width, height) = self.output_size();
        rect.x < width && rect.y < height && rect.x + rect.width > 0 && rect.y + rect.height > 0
    }

    fn request_redraw(&mut self) {
        self.dirty = true;
    }

    fn set_cursor(&mut self, cursor: AppCursor) {
        self.show_cursor(cursor);
    }

    fn set_anim_timer(&mut self, running: bool) {
        self.animating = running;
    }

    fn set_undo_timer(&mut self, running: bool) {
        self.undo_running = running;
    }

    /// The poll loop watches [`App::peek_deadline`] itself, so there is no
    /// timer to arrange.
    ///
    /// A layer surface rises to the overlay layer and drops back to the
    /// bottom. A toplevel cannot: raising a window is another thing Wayland
    /// keeps for the compositor, so a peek there is only the fade — which is
    /// no loss, because a toplevel was never behind anything to begin with.
    fn set_peek(&mut self, for_at_most: Option<Duration>) {
        if self.shape != Shape::Layer || self.layer_surface.is_null() {
            return;
        }
        let layer = if for_at_most.is_some() {
            LAYER_OVERLAY
        } else {
            LAYER_BOTTOM
        };
        // `set_layer` arrived in version 2 of the protocol; on version 1 the
        // widget simply stays where it is, which is the honest outcome.
        if self.connection.wl.version(self.layer_surface) >= 2 {
            self.connection.send1(self.layer_surface, 8, layer);
            self.connection.send(self.surface, wl_surface::COMMIT);
        }
    }

    fn begin_system_drag(&mut self) -> bool {
        if self.shape != Shape::Toplevel || self.xdg_toplevel.is_null() {
            return false;
        }
        // `xdg_toplevel.move(seat, serial)` — the compositor takes over, which
        // is the only way a window moves under Wayland.
        self.connection
            .send2(self.xdg_toplevel, 5, self.connection.seat, self.last_serial);
        true
    }

    fn show_menu(
        &mut self,
        entries: &[tpmplaner_core::menu::Entry],
    ) -> Option<tpmplaner_core::menu::Command> {
        menu::show(self, entries)
    }

    fn open_path(&self, path: &std::path::Path) {
        crate::unix::linux::open_path(path);
    }

    fn set_clipboard(&mut self, text: &str) {
        self.clipboard = Arc::new(text.to_owned());
        CLIPBOARD.with(|c| *c.borrow_mut() = self.clipboard.clone());
        self.offer_clipboard();
    }

    fn autostart_enabled(&self) -> bool {
        autostart::enabled()
    }

    fn set_autostart(&mut self, enabled: bool) {
        autostart::set(enabled);
    }

    fn quit(&mut self) {
        self.quit = true;
        log::info("Shut down");
    }

    fn system_visuals(&self) -> SystemVisuals {
        visuals::read()
    }
}

thread_local! {
    /// What the clipboard would hand out. Kept beside the shell because the
    /// data-source listener has no way back to it.
    static CLIPBOARD: RefCell<Arc<String>> = RefCell::new(Arc::new(String::new()));
}

// --- Listeners --------------------------------------------------------------
//
// Every one of these is an array of function pointers indexed by event
// opcode, so each has to hold an entry for *every* event its interface
// declares, in order — a missing one is a call into the wrong function.

type Data = *mut c_void;

#[repr(C)]
struct RegistryListener {
    global: extern "C" fn(Data, *mut wl_proxy, u32, *const c_char, u32),
    global_remove: extern "C" fn(Data, *mut wl_proxy, u32),
}

static REGISTRY_LISTENER: RegistryListener = RegistryListener {
    global: on_global,
    global_remove: on_global_remove,
};

extern "C" fn on_global(
    _data: Data,
    _registry: *mut wl_proxy,
    name: u32,
    interface: *const c_char,
    version: u32,
) {
    let Ok(interface) = (unsafe { CStr::from_ptr(interface) }).to_str() else {
        return;
    };
    EVENTS.with(|e| {
        e.borrow_mut()
            .globals
            .push((name, interface.to_owned(), version))
    });
}

extern "C" fn on_global_remove(_data: Data, _registry: *mut wl_proxy, _name: u32) {}

#[repr(C)]
struct SeatListener {
    capabilities: extern "C" fn(Data, *mut wl_proxy, u32),
    name: extern "C" fn(Data, *mut wl_proxy, *const c_char),
}

static SEAT_LISTENER: SeatListener = SeatListener {
    capabilities: on_seat_capabilities,
    name: on_seat_name,
};

extern "C" fn on_seat_capabilities(_data: Data, _seat: *mut wl_proxy, capabilities: u32) {
    EVENTS.with(|e| {
        e.borrow_mut().seat_has_pointer = capabilities & SEAT_CAPABILITY_POINTER != 0;
    });
}

extern "C" fn on_seat_name(_data: Data, _seat: *mut wl_proxy, _name: *const c_char) {}

#[repr(C)]
struct OutputListener {
    geometry: extern "C" fn(
        Data,
        *mut wl_proxy,
        i32,
        i32,
        i32,
        i32,
        i32,
        *const c_char,
        *const c_char,
        i32,
    ),
    mode: extern "C" fn(Data, *mut wl_proxy, u32, i32, i32, i32),
    done: extern "C" fn(Data, *mut wl_proxy),
    scale: extern "C" fn(Data, *mut wl_proxy, i32),
}

static OUTPUT_LISTENER: OutputListener = OutputListener {
    geometry: on_output_geometry,
    mode: on_output_mode,
    done: on_output_done,
    scale: on_output_scale,
};

#[allow(clippy::too_many_arguments)]
extern "C" fn on_output_geometry(
    _data: Data,
    _output: *mut wl_proxy,
    _x: i32,
    _y: i32,
    _physical_width: i32,
    _physical_height: i32,
    _subpixel: i32,
    _make: *const c_char,
    _model: *const c_char,
    _transform: i32,
) {
}

extern "C" fn on_output_mode(
    _data: Data,
    _output: *mut wl_proxy,
    flags: u32,
    width: i32,
    height: i32,
    _refresh: i32,
) {
    // `WL_OUTPUT_MODE_CURRENT`. An output lists every mode it supports and
    // this is the one it is actually in.
    if flags & 1 == 0 {
        return;
    }
    EVENTS.with(|e| e.borrow_mut().output_size = Some((width, height)));
}

extern "C" fn on_output_done(_data: Data, _output: *mut wl_proxy) {}

extern "C" fn on_output_scale(_data: Data, _output: *mut wl_proxy, factor: i32) {
    EVENTS.with(|e| e.borrow_mut().output_scale = Some(factor));
}

#[repr(C)]
struct PointerListener {
    enter: extern "C" fn(Data, *mut wl_proxy, u32, *mut wl_proxy, i32, i32),
    leave: extern "C" fn(Data, *mut wl_proxy, u32, *mut wl_proxy),
    motion: extern "C" fn(Data, *mut wl_proxy, u32, i32, i32),
    button: extern "C" fn(Data, *mut wl_proxy, u32, u32, u32, u32),
    axis: extern "C" fn(Data, *mut wl_proxy, u32, u32, i32),
    frame: extern "C" fn(Data, *mut wl_proxy),
    axis_source: extern "C" fn(Data, *mut wl_proxy, u32),
    axis_stop: extern "C" fn(Data, *mut wl_proxy, u32, u32),
    axis_discrete: extern "C" fn(Data, *mut wl_proxy, u32, i32),
    axis_value120: extern "C" fn(Data, *mut wl_proxy, u32, i32),
    axis_relative_direction: extern "C" fn(Data, *mut wl_proxy, u32, u32),
}

/// Eleven entries although the pointer is bound at version one: a listener is
/// indexed by opcode, and an array shorter than the interface is a call past
/// its end if a compositor ever sends a later event.
static POINTER_LISTENER: PointerListener = PointerListener {
    enter: on_pointer_enter,
    leave: on_pointer_leave,
    motion: on_pointer_motion,
    button: on_pointer_button,
    axis: on_pointer_axis,
    frame: on_pointer_nothing,
    axis_source: on_pointer_u32,
    axis_stop: on_pointer_two_u32,
    axis_discrete: on_pointer_u32_i32,
    axis_value120: on_pointer_u32_i32,
    axis_relative_direction: on_pointer_two_u32,
};

extern "C" fn on_pointer_enter(
    _data: Data,
    _pointer: *mut wl_proxy,
    serial: u32,
    surface: *mut wl_proxy,
    x: i32,
    y: i32,
) {
    EVENTS.with(|e| {
        e.borrow_mut()
            .pointer
            .push(PointerEvent::Enter(serial, surface, fixed(x), fixed(y)))
    });
}

extern "C" fn on_pointer_leave(
    _data: Data,
    _pointer: *mut wl_proxy,
    _serial: u32,
    surface: *mut wl_proxy,
) {
    EVENTS.with(|e| e.borrow_mut().pointer.push(PointerEvent::Leave(surface)));
}

extern "C" fn on_pointer_motion(_data: Data, _pointer: *mut wl_proxy, _time: u32, x: i32, y: i32) {
    EVENTS.with(|e| {
        e.borrow_mut()
            .pointer
            .push(PointerEvent::Motion(fixed(x), fixed(y)))
    });
}

extern "C" fn on_pointer_button(
    _data: Data,
    _pointer: *mut wl_proxy,
    serial: u32,
    _time: u32,
    button: u32,
    state: u32,
) {
    EVENTS.with(|e| {
        e.borrow_mut().pointer.push(PointerEvent::Button(
            serial,
            button,
            state == BUTTON_PRESSED,
        ))
    });
}

extern "C" fn on_pointer_axis(
    _data: Data,
    _pointer: *mut wl_proxy,
    _time: u32,
    axis: u32,
    value: i32,
) {
    if axis != AXIS_VERTICAL {
        return;
    }
    EVENTS.with(|e| {
        e.borrow_mut()
            .pointer
            .push(PointerEvent::Axis(fixed(value)))
    });
}

extern "C" fn on_pointer_nothing(_data: Data, _pointer: *mut wl_proxy) {}
extern "C" fn on_pointer_u32(_data: Data, _pointer: *mut wl_proxy, _a: u32) {}
extern "C" fn on_pointer_two_u32(_data: Data, _pointer: *mut wl_proxy, _a: u32, _b: u32) {}
extern "C" fn on_pointer_u32_i32(_data: Data, _pointer: *mut wl_proxy, _a: u32, _b: i32) {}

#[repr(C)]
struct BufferListener {
    release: extern "C" fn(Data, *mut wl_proxy),
}

static BUFFER_LISTENER: BufferListener = BufferListener {
    release: on_buffer_release,
};

extern "C" fn on_buffer_release(data: Data, _buffer: *mut wl_proxy) {
    let slot = data as usize;
    EVENTS.with(|e| e.borrow_mut().released.push(slot));
}

#[repr(C)]
struct LayerSurfaceListener {
    configure: extern "C" fn(Data, *mut wl_proxy, u32, u32, u32),
    closed: extern "C" fn(Data, *mut wl_proxy),
}

static LAYER_SURFACE_LISTENER: LayerSurfaceListener = LayerSurfaceListener {
    configure: on_layer_configure,
    closed: on_layer_closed,
};

extern "C" fn on_layer_configure(
    _data: Data,
    _surface: *mut wl_proxy,
    serial: u32,
    width: u32,
    height: u32,
) {
    EVENTS.with(|e| e.borrow_mut().layer_configure = Some((serial, width, height)));
}

extern "C" fn on_layer_closed(_data: Data, _surface: *mut wl_proxy) {
    EVENTS.with(|e| e.borrow_mut().closed = true);
}

#[repr(C)]
struct XdgWmBaseListener {
    ping: extern "C" fn(Data, *mut wl_proxy, u32),
}

static XDG_WM_BASE_LISTENER: XdgWmBaseListener = XdgWmBaseListener { ping: on_ping };

extern "C" fn on_ping(_data: Data, _base: *mut wl_proxy, serial: u32) {
    EVENTS.with(|e| e.borrow_mut().ping = Some(serial));
}

#[repr(C)]
pub(super) struct XdgSurfaceListener {
    configure: extern "C" fn(Data, *mut wl_proxy, u32),
}

static XDG_SURFACE_LISTENER: XdgSurfaceListener = XdgSurfaceListener {
    configure: on_xdg_configure,
};

/// The same listener serves the widget's surface and the menu's, told apart by
/// the `data` the listener was attached with: zero for the widget, one for a
/// popup. Without that a menu opening would look like the window being
/// reconfigured.
extern "C" fn on_xdg_configure(data: Data, _surface: *mut wl_proxy, serial: u32) {
    EVENTS.with(|e| {
        let mut events = e.borrow_mut();
        if data as usize == 1 {
            events.popup_configure = Some(serial);
        } else {
            events.xdg_configure = Some(serial);
        }
    });
}

/// Handed to the popup, which attaches it to its own `xdg_surface`.
pub(super) fn xdg_surface_listener() -> &'static XdgSurfaceListener {
    &XDG_SURFACE_LISTENER
}

#[repr(C)]
pub(super) struct XdgPopupListener {
    configure: extern "C" fn(Data, *mut wl_proxy, i32, i32, i32, i32),
    popup_done: extern "C" fn(Data, *mut wl_proxy),
}

static XDG_POPUP_LISTENER: XdgPopupListener = XdgPopupListener {
    configure: on_popup_configure,
    popup_done: on_popup_done,
};

pub(super) fn xdg_popup_listener() -> &'static XdgPopupListener {
    &XDG_POPUP_LISTENER
}

extern "C" fn on_popup_configure(
    _data: Data,
    _popup: *mut wl_proxy,
    _x: i32,
    _y: i32,
    _width: i32,
    _height: i32,
) {
    // Where the compositor put it and how big it let it be. The menu asked
    // for a size it had already measured and the constraint adjustment only
    // slides it, so there is nothing to adopt — but the event has to be in
    // the listener, because the one after it is not optional.
}

extern "C" fn on_popup_done(_data: Data, _popup: *mut wl_proxy) {
    EVENTS.with(|e| e.borrow_mut().popup_done = true);
}

#[repr(C)]
struct XdgToplevelListener {
    configure: extern "C" fn(Data, *mut wl_proxy, i32, i32, *mut c_void),
    close: extern "C" fn(Data, *mut wl_proxy),
}

static XDG_TOPLEVEL_LISTENER: XdgToplevelListener = XdgToplevelListener {
    configure: on_toplevel_configure,
    close: on_toplevel_close,
};

extern "C" fn on_toplevel_configure(
    _data: Data,
    _toplevel: *mut wl_proxy,
    width: i32,
    height: i32,
    _states: *mut c_void,
) {
    EVENTS.with(|e| e.borrow_mut().toplevel_size = Some((width, height)));
}

extern "C" fn on_toplevel_close(_data: Data, _toplevel: *mut wl_proxy) {
    EVENTS.with(|e| e.borrow_mut().closed = true);
}

#[repr(C)]
struct DataSourceListener {
    target: extern "C" fn(Data, *mut wl_proxy, *const c_char),
    send: extern "C" fn(Data, *mut wl_proxy, *const c_char, c_int),
    cancelled: extern "C" fn(Data, *mut wl_proxy),
    dnd_drop_performed: extern "C" fn(Data, *mut wl_proxy),
    dnd_finished: extern "C" fn(Data, *mut wl_proxy),
    action: extern "C" fn(Data, *mut wl_proxy, u32),
}

static DATA_SOURCE_LISTENER: DataSourceListener = DataSourceListener {
    target: on_source_target,
    send: on_source_send,
    cancelled: on_source_cancelled,
    dnd_drop_performed: on_source_nothing,
    dnd_finished: on_source_nothing,
    action: on_source_action,
};

extern "C" fn on_source_target(_data: Data, _source: *mut wl_proxy, _mime: *const c_char) {}

extern "C" fn on_source_send(_data: Data, _source: *mut wl_proxy, mime: *const c_char, fd: c_int) {
    let mime = unsafe { CStr::from_ptr(mime) }
        .to_str()
        .unwrap_or_default()
        .to_owned();
    EVENTS.with(|e| e.borrow_mut().clipboard_requests.push((mime, fd)));
}

extern "C" fn on_source_cancelled(_data: Data, _source: *mut wl_proxy) {
    EVENTS.with(|e| e.borrow_mut().clipboard_cancelled = true);
}

extern "C" fn on_source_nothing(_data: Data, _source: *mut wl_proxy) {}
extern "C" fn on_source_action(_data: Data, _source: *mut wl_proxy, _action: u32) {}

// --- Waiting ----------------------------------------------------------------

/// Sleeps until either descriptor has something or the timeout runs out.
/// True if the Wayland connection is the one with something to say.
fn wait(wayland: c_int, wake: c_int, timeout: Duration) -> bool {
    let mut fds = [
        libc::pollfd {
            fd: wayland,
            events: libc::POLLIN,
            revents: 0,
        },
        libc::pollfd {
            fd: wake,
            events: libc::POLLIN,
            revents: 0,
        },
    ];
    let ms = timeout.as_millis().min(60_000) as c_int;
    let ready = unsafe { libc::poll(fds.as_mut_ptr(), 2, ms) };
    ready > 0 && fds[0].revents & libc::POLLIN != 0
}

/// Empties the wake pipe; true if there was anything in it.
fn drain(wake: c_int) -> bool {
    let mut buffer = [0u8; 64];
    let mut woken = false;
    loop {
        let read = unsafe { libc::read(wake, buffer.as_mut_ptr() as *mut c_void, buffer.len()) };
        if read <= 0 {
            return woken;
        }
        woken = true;
    }
}

fn next_minute_boundary() -> Instant {
    use chrono::Timelike;
    let now = chrono::Local::now();
    let ms = 60_000 - (now.second() * 1000 + now.timestamp_subsec_millis()) as i64;
    Instant::now() + Duration::from_millis(ms.clamp(1_000, 60_000) as u64)
}

/// A non-blocking pipe: `(read, write)`.
fn pipe() -> Option<(c_int, c_int)> {
    let mut fds = [0 as c_int; 2];
    let ok = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) };
    (ok == 0).then_some((fds[0], fds[1]))
}

/// The pointer events the menu's own loop needs, taken from the same queue
/// the widget's loop drains.
pub(super) fn take_pointer_events() -> Vec<PointerEvent> {
    EVENTS.with(|e| std::mem::take(&mut e.borrow_mut().pointer))
}

/// `(dismissed, reconfigured)` for the popup.
pub(super) fn take_popup_state() -> (bool, bool) {
    EVENTS.with(|e| {
        let mut events = e.borrow_mut();
        (
            std::mem::take(&mut events.popup_done),
            events.popup_configure.is_some(),
        )
    })
}

pub(super) fn take_xdg_configure() -> Option<u32> {
    EVENTS.with(|e| e.borrow_mut().popup_configure.take())
}

/// One shared-memory buffer, for a surface that is drawn rarely and lives
/// briefly — which is what a menu is.
///
/// The widget's own surface keeps two, so a frame can be drawn while the
/// compositor holds the last one. A menu redraws when the pointer moves from
/// row to row, and waiting for the release between two of those costs nothing
/// anybody can see.
pub(super) struct SingleBuffer {
    /// Kept so the pool and the buffer can be given back: one of these is made
    /// per context menu, and the compositor holds its own mapping of the memfd
    /// for as long as the pool lives.
    wl: Rc<Wl>,
    fd: c_int,
    memory: *mut u8,
    length: usize,
    pool: *mut wl_proxy,
    buffer: *mut wl_proxy,
    canvas: Option<Cairo>,
}

impl SingleBuffer {
    pub(super) fn new(
        connection: &Connection,
        libs: Arc<Libs>,
        size: (i32, i32),
        scale: i32,
        look: &Look,
    ) -> Option<Self> {
        let scale = scale.max(1);
        let (width, height) = (size.0.max(1) * scale, size.1.max(1) * scale);
        let stride = width * 4;
        let length = (stride as usize) * (height as usize);

        let fd = unsafe { libc::memfd_create(c"tpmplaner-menu".as_ptr(), libc::MFD_CLOEXEC) };
        if fd < 0 {
            return None;
        }
        if unsafe { libc::ftruncate(fd, length as libc::off_t) } != 0 {
            unsafe { libc::close(fd) };
            return None;
        }
        let mapped = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                length,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd,
                0,
            )
        };
        if mapped == libc::MAP_FAILED {
            unsafe { libc::close(fd) };
            return None;
        }

        let wl = &connection.wl;
        let pool = unsafe {
            (wl.wl_proxy_marshal_flags)(
                connection.shm,
                wl_shm::CREATE_POOL,
                wl.wl_shm_pool_interface,
                wl.version(connection.shm),
                0,
                std::ptr::null_mut::<c_void>(),
                fd,
                length as i32,
            )
        };
        if pool.is_null() {
            unsafe {
                libc::munmap(mapped, length);
                libc::close(fd);
            }
            return None;
        }
        let buffer = unsafe {
            (wl.wl_proxy_marshal_flags)(
                pool,
                wl_shm_pool::CREATE_BUFFER,
                wl.wl_buffer_interface,
                wl.version(pool),
                0,
                std::ptr::null_mut::<c_void>(),
                0i32,
                width,
                height,
                stride,
                SHM_FORMAT_ARGB8888,
            )
        };
        if buffer.is_null() {
            unsafe {
                libc::munmap(mapped, length);
                libc::close(fd);
            }
            return None;
        }

        let mut canvas =
            unsafe { Cairo::for_pixels(libs, mapped as *mut u8, (width, height), stride, look) }?;
        canvas.set_scale(scale as f64);
        Some(Self {
            wl: wl.clone(),
            fd,
            memory: mapped as *mut u8,
            length,
            pool,
            buffer,
            canvas: Some(canvas),
        })
    }

    pub(super) fn canvas(&mut self) -> Option<&mut Cairo> {
        self.canvas.as_mut()
    }

    pub(super) fn buffer(&self) -> *mut wl_proxy {
        self.buffer
    }
}

impl Drop for SingleBuffer {
    fn drop(&mut self) {
        // The canvas points into the mapping, so it goes first.
        self.canvas = None;
        unsafe {
            if !self.buffer.is_null() {
                (self.wl.wl_proxy_marshal_flags)(
                    self.buffer,
                    wl_buffer::DESTROY,
                    std::ptr::null(),
                    self.wl.version(self.buffer),
                    0,
                );
                (self.wl.wl_proxy_destroy)(self.buffer);
                self.buffer = std::ptr::null_mut();
            }
            if !self.pool.is_null() {
                (self.wl.wl_proxy_marshal_flags)(
                    self.pool,
                    wl_shm_pool::DESTROY,
                    std::ptr::null(),
                    self.wl.version(self.pool),
                    0,
                );
                (self.wl.wl_proxy_destroy)(self.pool);
                self.pool = std::ptr::null_mut();
            }
            libc::munmap(self.memory as *mut c_void, self.length);
            libc::close(self.fd);
        }
    }
}
