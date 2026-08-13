// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! Wayland without generated code.
//!
//! Every Wayland binding in existence is produced by `wayland-scanner` from
//! the protocol XML. This project does not run a code generator, for the same
//! reason it carries no toolkit: the surface actually used is small, and a
//! build step is a thing that can break on somebody else's machine. So the
//! protocol is described here by hand.
//!
//! **Most of it does not have to be.** `libwayland-client.so.0` *exports* the
//! interface description of every core protocol object — `wl_surface`,
//! `wl_shm`, `wl_pointer` and the rest — so those are looked up with `dlsym`
//! like any other symbol, and cannot be got wrong. What is written out below
//! is only what libwayland does not ship: the layer shell, and the four
//! `xdg_shell` objects a popup menu needs.
//!
//! **How a request is sent.** `wl_proxy_marshal_flags(proxy, opcode,
//! interface, version, flags, ...)`. The opcode is the request's *position* in
//! its interface, which is why the tables below list every request up to the
//! last one used, in order, even the ones this program never sends — a gap
//! would silently renumber the rest. The same is true of events and the
//! listener structs in [`super::window`]: a listener is an array of function
//! pointers indexed by event opcode, so a missing entry is a call into the
//! wrong function.
//!
//! **Signatures.** One character per argument: `i` int, `u` uint, `f` fixed,
//! `s` string, `o` object, `n` new id, `a` array, `h` file descriptor, `?`
//! marking the following argument nullable, and a leading digit meaning "since
//! version". The `types` array beside it names the interface of every object
//! and new-id argument, with a null for everything else.
//!
//! `wl_proxy_marshal_flags` is variadic, and is called through a variadic
//! function pointer rather than a convenient fixed one: on x86-64 a variadic
//! callee reads `al` for the number of vector registers used, and a call made
//! through a non-variadic type never sets it.

#![allow(non_camel_case_types, non_upper_case_globals)]

use super::super::dl::Library;
use std::ffi::{CStr, c_char, c_int, c_void};
use std::sync::atomic::{AtomicPtr, Ordering};

pub type wl_proxy = c_void;
/// Renamed from the C `wl_display` so it does not collide with the module of
/// opcodes below, which has to keep the protocol's own name.
pub type WlDisplay = c_void;

/// One request or event.
#[repr(C)]
pub struct WlMessage {
    pub name: *const c_char,
    pub signature: *const c_char,
    pub types: *const *const WlInterface,
}

// Every one of these is a `static` built at compile time out of other
// `static`s. Nothing mutates them and libwayland only reads them.
unsafe impl Sync for WlMessage {}

/// The description of one protocol object.
#[repr(C)]
pub struct WlInterface {
    pub name: *const c_char,
    pub version: c_int,
    pub method_count: c_int,
    pub methods: *const WlMessage,
    pub event_count: c_int,
    pub events: *const WlMessage,
}

unsafe impl Sync for WlInterface {}

/// A `types` entry for an argument that is not an object.
const fn none() -> AtomicPtr<WlInterface> {
    AtomicPtr::new(std::ptr::null_mut())
}

/// A `types` entry naming an interface this file describes.
const fn known(interface: &'static WlInterface) -> AtomicPtr<WlInterface> {
    AtomicPtr::new(interface as *const WlInterface as *mut WlInterface)
}

/// A `types` entry that libwayland has to fill in: the core interfaces are
/// symbols in its shared object, so their addresses are not known until it is
/// loaded. `AtomicPtr` rather than a cast through `*mut` — a static that is
/// written to has to say so in its type.
const fn from_libwayland() -> AtomicPtr<WlInterface> {
    AtomicPtr::new(std::ptr::null_mut())
}

/// `types` arrays are read by libwayland as `*const *const wl_interface`, and
/// an `AtomicPtr<T>` is a `*mut T` with a promise attached — same size, same
/// bytes, same layout.
const fn types(slots: &'static [AtomicPtr<WlInterface>]) -> *const *const WlInterface {
    slots.as_ptr() as *const *const WlInterface
}

// --- The layer shell --------------------------------------------------------

/// `zwlr_layer_shell_v1.get_layer_surface` and `.destroy`.
static LAYER_SHELL_METHODS: [WlMessage; 2] = [
    WlMessage {
        name: c"get_layer_surface".as_ptr(),
        signature: c"no?ous".as_ptr(),
        types: types(&LAYER_SHELL_GET_TYPES),
    },
    WlMessage {
        name: c"destroy".as_ptr(),
        // Added in version 3, which is what the leading digit says.
        signature: c"3".as_ptr(),
        types: std::ptr::null(),
    },
];

/// The new layer surface, the `wl_surface` it wraps, the output it belongs to,
/// then the layer and the namespace, which are not objects.
static LAYER_SHELL_GET_TYPES: [AtomicPtr<WlInterface>; 5] = [
    known(&LAYER_SURFACE_INTERFACE),
    from_libwayland(),
    from_libwayland(),
    none(),
    none(),
];

pub static LAYER_SHELL_INTERFACE: WlInterface = WlInterface {
    name: c"zwlr_layer_shell_v1".as_ptr(),
    version: 4,
    method_count: 2,
    methods: LAYER_SHELL_METHODS.as_ptr(),
    event_count: 0,
    events: std::ptr::null(),
};

static LAYER_SURFACE_METHODS: [WlMessage; 9] = [
    WlMessage {
        name: c"set_size".as_ptr(),
        signature: c"uu".as_ptr(),
        types: types(&TWO_NONE),
    },
    WlMessage {
        name: c"set_anchor".as_ptr(),
        signature: c"u".as_ptr(),
        types: types(&TWO_NONE),
    },
    WlMessage {
        name: c"set_exclusive_zone".as_ptr(),
        signature: c"i".as_ptr(),
        types: types(&TWO_NONE),
    },
    WlMessage {
        name: c"set_margin".as_ptr(),
        signature: c"iiii".as_ptr(),
        types: types(&FOUR_NONE),
    },
    WlMessage {
        name: c"set_keyboard_interactivity".as_ptr(),
        signature: c"u".as_ptr(),
        types: types(&TWO_NONE),
    },
    WlMessage {
        name: c"get_popup".as_ptr(),
        signature: c"o".as_ptr(),
        types: types(&LAYER_SURFACE_POPUP_TYPES),
    },
    WlMessage {
        name: c"ack_configure".as_ptr(),
        signature: c"u".as_ptr(),
        types: types(&TWO_NONE),
    },
    WlMessage {
        name: c"destroy".as_ptr(),
        signature: c"".as_ptr(),
        types: std::ptr::null(),
    },
    WlMessage {
        name: c"set_layer".as_ptr(),
        signature: c"2u".as_ptr(),
        types: types(&TWO_NONE),
    },
];

static LAYER_SURFACE_POPUP_TYPES: [AtomicPtr<WlInterface>; 1] = [known(&XDG_POPUP_INTERFACE)];

static LAYER_SURFACE_EVENTS: [WlMessage; 2] = [
    WlMessage {
        name: c"configure".as_ptr(),
        signature: c"uuu".as_ptr(),
        types: types(&FOUR_NONE),
    },
    WlMessage {
        name: c"closed".as_ptr(),
        signature: c"".as_ptr(),
        types: std::ptr::null(),
    },
];

pub static LAYER_SURFACE_INTERFACE: WlInterface = WlInterface {
    name: c"zwlr_layer_surface_v1".as_ptr(),
    version: 4,
    method_count: 9,
    methods: LAYER_SURFACE_METHODS.as_ptr(),
    event_count: 2,
    events: LAYER_SURFACE_EVENTS.as_ptr(),
};

/// Opcodes, named so a call site reads as the protocol does.
pub mod layer_shell {
    pub const GET_LAYER_SURFACE: u32 = 0;
}

pub mod layer_surface {
    pub const SET_SIZE: u32 = 0;
    pub const SET_ANCHOR: u32 = 1;
    pub const SET_EXCLUSIVE_ZONE: u32 = 2;
    pub const SET_MARGIN: u32 = 3;
    pub const SET_KEYBOARD_INTERACTIVITY: u32 = 4;
    pub const GET_POPUP: u32 = 5;
    pub const ACK_CONFIGURE: u32 = 6;
}

/// `ZWLR_LAYER_SHELL_V1_LAYER_BOTTOM`: above the wallpaper, below every
/// ordinary window. Exactly what the widget is defined by, and the reason this
/// protocol is worth the trouble.
pub const LAYER_BOTTOM: u32 = 1;
/// `LAYER_OVERLAY`, for the menu's parent while it is open.
pub const LAYER_OVERLAY: u32 = 3;

pub const ANCHOR_TOP: u32 = 1;
pub const ANCHOR_LEFT: u32 = 4;

/// `KEYBOARD_INTERACTIVITY_NONE`: the widget never takes the keyboard, which
/// is this protocol's spelling of `WS_EX_NOACTIVATE`.
pub const KEYBOARD_NONE: u32 = 0;

// --- xdg_shell, for the menu ------------------------------------------------

static XDG_WM_BASE_METHODS: [WlMessage; 4] = [
    WlMessage {
        name: c"destroy".as_ptr(),
        signature: c"".as_ptr(),
        types: std::ptr::null(),
    },
    WlMessage {
        name: c"create_positioner".as_ptr(),
        signature: c"n".as_ptr(),
        types: types(&XDG_POSITIONER_TYPES),
    },
    WlMessage {
        name: c"get_xdg_surface".as_ptr(),
        signature: c"no".as_ptr(),
        types: types(&XDG_GET_SURFACE_TYPES),
    },
    WlMessage {
        name: c"pong".as_ptr(),
        signature: c"u".as_ptr(),
        types: types(&TWO_NONE),
    },
];

static XDG_POSITIONER_TYPES: [AtomicPtr<WlInterface>; 1] = [known(&XDG_POSITIONER_INTERFACE)];
/// The new `xdg_surface`, then the `wl_surface` it wraps — whose interface is
/// libwayland's and is patched in at start-up.
static XDG_GET_SURFACE_TYPES: [AtomicPtr<WlInterface>; 2] =
    [known(&XDG_SURFACE_INTERFACE), from_libwayland()];

static XDG_WM_BASE_EVENTS: [WlMessage; 1] = [WlMessage {
    name: c"ping".as_ptr(),
    signature: c"u".as_ptr(),
    types: types(&TWO_NONE),
}];

pub static XDG_WM_BASE_INTERFACE: WlInterface = WlInterface {
    name: c"xdg_wm_base".as_ptr(),
    version: 2,
    method_count: 4,
    methods: XDG_WM_BASE_METHODS.as_ptr(),
    event_count: 1,
    events: XDG_WM_BASE_EVENTS.as_ptr(),
};

static XDG_POSITIONER_METHODS: [WlMessage; 7] = [
    WlMessage {
        name: c"destroy".as_ptr(),
        signature: c"".as_ptr(),
        types: std::ptr::null(),
    },
    WlMessage {
        name: c"set_size".as_ptr(),
        signature: c"ii".as_ptr(),
        types: types(&TWO_NONE),
    },
    WlMessage {
        name: c"set_anchor_rect".as_ptr(),
        signature: c"iiii".as_ptr(),
        types: types(&FOUR_NONE),
    },
    WlMessage {
        name: c"set_anchor".as_ptr(),
        signature: c"u".as_ptr(),
        types: types(&TWO_NONE),
    },
    WlMessage {
        name: c"set_gravity".as_ptr(),
        signature: c"u".as_ptr(),
        types: types(&TWO_NONE),
    },
    WlMessage {
        name: c"set_constraint_adjustment".as_ptr(),
        signature: c"u".as_ptr(),
        types: types(&TWO_NONE),
    },
    WlMessage {
        name: c"set_offset".as_ptr(),
        signature: c"ii".as_ptr(),
        types: types(&TWO_NONE),
    },
];

pub static XDG_POSITIONER_INTERFACE: WlInterface = WlInterface {
    name: c"xdg_positioner".as_ptr(),
    version: 2,
    method_count: 7,
    methods: XDG_POSITIONER_METHODS.as_ptr(),
    event_count: 0,
    events: std::ptr::null(),
};

static XDG_SURFACE_METHODS: [WlMessage; 5] = [
    WlMessage {
        name: c"destroy".as_ptr(),
        signature: c"".as_ptr(),
        types: std::ptr::null(),
    },
    WlMessage {
        name: c"get_toplevel".as_ptr(),
        signature: c"n".as_ptr(),
        types: types(&XDG_TOPLEVEL_TYPES),
    },
    WlMessage {
        name: c"get_popup".as_ptr(),
        signature: c"n?oo".as_ptr(),
        types: types(&XDG_GET_POPUP_TYPES),
    },
    WlMessage {
        name: c"set_window_geometry".as_ptr(),
        signature: c"iiii".as_ptr(),
        types: types(&FOUR_NONE),
    },
    WlMessage {
        name: c"ack_configure".as_ptr(),
        signature: c"u".as_ptr(),
        types: types(&TWO_NONE),
    },
];

static XDG_TOPLEVEL_TYPES: [AtomicPtr<WlInterface>; 1] = [known(&XDG_TOPLEVEL_INTERFACE)];
static XDG_GET_POPUP_TYPES: [AtomicPtr<WlInterface>; 3] = [
    known(&XDG_POPUP_INTERFACE),
    known(&XDG_SURFACE_INTERFACE),
    known(&XDG_POSITIONER_INTERFACE),
];

static XDG_SURFACE_EVENTS: [WlMessage; 1] = [WlMessage {
    name: c"configure".as_ptr(),
    signature: c"u".as_ptr(),
    types: types(&TWO_NONE),
}];

pub static XDG_SURFACE_INTERFACE: WlInterface = WlInterface {
    name: c"xdg_surface".as_ptr(),
    version: 2,
    method_count: 5,
    methods: XDG_SURFACE_METHODS.as_ptr(),
    event_count: 1,
    events: XDG_SURFACE_EVENTS.as_ptr(),
};

static XDG_TOPLEVEL_METHODS: [WlMessage; 14] = [
    WlMessage {
        name: c"destroy".as_ptr(),
        signature: c"".as_ptr(),
        types: std::ptr::null(),
    },
    WlMessage {
        name: c"set_parent".as_ptr(),
        signature: c"?o".as_ptr(),
        types: types(&XDG_TOPLEVEL_TYPES),
    },
    WlMessage {
        name: c"set_title".as_ptr(),
        signature: c"s".as_ptr(),
        types: types(&TWO_NONE),
    },
    WlMessage {
        name: c"set_app_id".as_ptr(),
        signature: c"s".as_ptr(),
        types: types(&TWO_NONE),
    },
    WlMessage {
        name: c"show_window_menu".as_ptr(),
        signature: c"ouii".as_ptr(),
        types: types(&FOUR_NONE),
    },
    WlMessage {
        name: c"move".as_ptr(),
        signature: c"ou".as_ptr(),
        types: types(&TWO_NONE),
    },
    WlMessage {
        name: c"resize".as_ptr(),
        signature: c"ouu".as_ptr(),
        types: types(&FOUR_NONE),
    },
    WlMessage {
        name: c"set_max_size".as_ptr(),
        signature: c"ii".as_ptr(),
        types: types(&TWO_NONE),
    },
    WlMessage {
        name: c"set_min_size".as_ptr(),
        signature: c"ii".as_ptr(),
        types: types(&TWO_NONE),
    },
    WlMessage {
        name: c"set_maximized".as_ptr(),
        signature: c"".as_ptr(),
        types: std::ptr::null(),
    },
    WlMessage {
        name: c"unset_maximized".as_ptr(),
        signature: c"".as_ptr(),
        types: std::ptr::null(),
    },
    WlMessage {
        name: c"set_fullscreen".as_ptr(),
        signature: c"?o".as_ptr(),
        types: types(&TWO_NONE),
    },
    WlMessage {
        name: c"unset_fullscreen".as_ptr(),
        signature: c"".as_ptr(),
        types: std::ptr::null(),
    },
    WlMessage {
        name: c"set_minimized".as_ptr(),
        signature: c"".as_ptr(),
        types: std::ptr::null(),
    },
];

static XDG_TOPLEVEL_EVENTS: [WlMessage; 2] = [
    WlMessage {
        name: c"configure".as_ptr(),
        signature: c"iia".as_ptr(),
        types: types(&FOUR_NONE),
    },
    WlMessage {
        name: c"close".as_ptr(),
        signature: c"".as_ptr(),
        types: std::ptr::null(),
    },
];

pub static XDG_TOPLEVEL_INTERFACE: WlInterface = WlInterface {
    name: c"xdg_toplevel".as_ptr(),
    version: 2,
    method_count: 14,
    methods: XDG_TOPLEVEL_METHODS.as_ptr(),
    event_count: 2,
    events: XDG_TOPLEVEL_EVENTS.as_ptr(),
};

static XDG_POPUP_METHODS: [WlMessage; 2] = [
    WlMessage {
        name: c"destroy".as_ptr(),
        signature: c"".as_ptr(),
        types: std::ptr::null(),
    },
    WlMessage {
        name: c"grab".as_ptr(),
        signature: c"ou".as_ptr(),
        types: types(&TWO_NONE),
    },
];

static XDG_POPUP_EVENTS: [WlMessage; 2] = [
    WlMessage {
        name: c"configure".as_ptr(),
        signature: c"iiii".as_ptr(),
        types: types(&FOUR_NONE),
    },
    WlMessage {
        name: c"popup_done".as_ptr(),
        signature: c"".as_ptr(),
        types: std::ptr::null(),
    },
];

pub static XDG_POPUP_INTERFACE: WlInterface = WlInterface {
    name: c"xdg_popup".as_ptr(),
    version: 2,
    method_count: 2,
    methods: XDG_POPUP_METHODS.as_ptr(),
    event_count: 2,
    events: XDG_POPUP_EVENTS.as_ptr(),
};

pub mod xdg_wm_base {
    pub const CREATE_POSITIONER: u32 = 1;
    pub const GET_XDG_SURFACE: u32 = 2;
    pub const PONG: u32 = 3;
}

pub mod xdg_positioner {
    pub const DESTROY: u32 = 0;
    pub const SET_SIZE: u32 = 1;
    pub const SET_ANCHOR_RECT: u32 = 2;
    pub const SET_ANCHOR: u32 = 3;
    pub const SET_GRAVITY: u32 = 4;
    pub const SET_CONSTRAINT_ADJUSTMENT: u32 = 5;
}

pub mod xdg_surface {
    pub const DESTROY: u32 = 0;
    pub const GET_TOPLEVEL: u32 = 1;
    pub const GET_POPUP: u32 = 2;
    pub const ACK_CONFIGURE: u32 = 4;
}

pub mod xdg_toplevel {
    pub const SET_TITLE: u32 = 2;
    pub const SET_APP_ID: u32 = 3;
    pub const SET_MIN_SIZE: u32 = 8;
}

pub mod xdg_popup {
    pub const DESTROY: u32 = 0;
    pub const GRAB: u32 = 1;
}

/// `XDG_POSITIONER_ANCHOR_TOP_LEFT` and `GRAVITY_BOTTOM_RIGHT`: the popup
/// hangs down and to the right of the point it was anchored at, which is what
/// a context menu does.
pub const XDG_ANCHOR_TOP_LEFT: u32 = 5;
pub const XDG_GRAVITY_BOTTOM_RIGHT: u32 = 8;
/// `SLIDE_X | SLIDE_Y | FLIP_X | FLIP_Y`: the compositor keeps the menu on the
/// screen, which is the one piece of menu placement Wayland does for you.
pub const XDG_CONSTRAINT_ADJUST: u32 = 1 | 2 | 4 | 8;

// --- Core protocol opcodes --------------------------------------------------
//
// The interfaces themselves come from libwayland; only the numbering is
// needed here, and it is fixed by the protocol.

pub mod wl_display {
    pub const GET_REGISTRY: u32 = 1;
}

pub mod wl_registry {
    pub const BIND: u32 = 0;
}

pub mod wl_compositor {
    pub const CREATE_SURFACE: u32 = 0;
}

pub mod wl_surface {
    pub const DESTROY: u32 = 0;
    pub const ATTACH: u32 = 1;
    pub const DAMAGE: u32 = 2;
    pub const COMMIT: u32 = 6;
    pub const SET_BUFFER_SCALE: u32 = 8;
}

pub mod wl_shm {
    pub const CREATE_POOL: u32 = 0;
}

pub mod wl_shm_pool {
    pub const CREATE_BUFFER: u32 = 0;
}

pub mod wl_seat {
    pub const GET_POINTER: u32 = 0;
}

pub mod wl_pointer {
    pub const SET_CURSOR: u32 = 0;
}

pub mod wl_data_device_manager {
    pub const CREATE_DATA_SOURCE: u32 = 0;
    pub const GET_DATA_DEVICE: u32 = 1;
}

pub mod wl_data_device {
    pub const SET_SELECTION: u32 = 1;
}

pub mod wl_data_source {
    pub const OFFER: u32 = 0;
    pub const DESTROY: u32 = 1;
}

/// `WL_SHM_FORMAT_ARGB8888`. The one format every compositor must support, and
/// the one Cairo's `ARGB32` already is.
pub const SHM_FORMAT_ARGB8888: u32 = 0;

/// `WL_POINTER_BUTTON_STATE_PRESSED`.
pub const BUTTON_PRESSED: u32 = 1;

/// Linux input codes, which is what `wl_pointer.button` reports.
pub const BTN_LEFT: u32 = 0x110;
pub const BTN_RIGHT: u32 = 0x111;

/// `WL_POINTER_AXIS_VERTICAL_SCROLL`.
pub const AXIS_VERTICAL: u32 = 0;

/// `WL_SEAT_CAPABILITY_POINTER`.
pub const SEAT_CAPABILITY_POINTER: u32 = 1;

/// Shared `types` arrays for the messages whose arguments are all plain
/// numbers or strings. libwayland reads as many entries as the signature has
/// arguments, so one array of nulls serves every such message of that length
/// or fewer.
static TWO_NONE: [AtomicPtr<WlInterface>; 2] = [none(), none()];
static FOUR_NONE: [AtomicPtr<WlInterface>; 4] = [none(), none(), none(), none()];

/// The symbols and interface pointers the Wayland front end resolves at
/// start-up.
#[allow(non_snake_case)]
pub struct Wl {
    pub wl_display_connect: unsafe extern "C" fn(*const c_char) -> *mut WlDisplay,
    pub wl_display_disconnect: unsafe extern "C" fn(*mut WlDisplay),
    pub wl_display_get_fd: unsafe extern "C" fn(*mut WlDisplay) -> c_int,
    pub wl_display_roundtrip: unsafe extern "C" fn(*mut WlDisplay) -> c_int,
    pub wl_display_flush: unsafe extern "C" fn(*mut WlDisplay) -> c_int,
    pub wl_display_dispatch_pending: unsafe extern "C" fn(*mut WlDisplay) -> c_int,
    pub wl_display_prepare_read: unsafe extern "C" fn(*mut WlDisplay) -> c_int,
    pub wl_display_read_events: unsafe extern "C" fn(*mut WlDisplay) -> c_int,
    pub wl_display_cancel_read: unsafe extern "C" fn(*mut WlDisplay),

    /// Variadic on purpose — see the module note.
    pub wl_proxy_marshal_flags: unsafe extern "C" fn(
        *mut wl_proxy,
        u32,
        *const WlInterface,
        u32,
        u32,
        ...
    ) -> *mut wl_proxy,
    pub wl_proxy_add_listener:
        unsafe extern "C" fn(*mut wl_proxy, *mut *mut c_void, *mut c_void) -> c_int,
    pub wl_proxy_destroy: unsafe extern "C" fn(*mut wl_proxy),
    pub wl_proxy_get_version: unsafe extern "C" fn(*mut wl_proxy) -> u32,

    /// libwayland's own descriptions of the core objects.
    pub wl_compositor_interface: *const WlInterface,
    pub wl_surface_interface: *const WlInterface,
    pub wl_shm_interface: *const WlInterface,
    pub wl_shm_pool_interface: *const WlInterface,
    pub wl_buffer_interface: *const WlInterface,
    pub wl_seat_interface: *const WlInterface,
    pub wl_pointer_interface: *const WlInterface,
    pub wl_output_interface: *const WlInterface,
    pub wl_registry_interface: *const WlInterface,
    pub wl_data_device_manager_interface: *const WlInterface,
    pub wl_data_device_interface: *const WlInterface,
    pub wl_data_source_interface: *const WlInterface,

    _library: Library,
}

impl Wl {
    pub fn load() -> Option<Self> {
        let wl = Library::open(&["libwayland-client.so.0", "libwayland-client.so"])?;
        let me = unsafe {
            Self {
                wl_display_connect: wl.symbol(c"wl_display_connect")?,
                wl_display_disconnect: wl.symbol(c"wl_display_disconnect")?,
                wl_display_get_fd: wl.symbol(c"wl_display_get_fd")?,
                wl_display_roundtrip: wl.symbol(c"wl_display_roundtrip")?,
                wl_display_flush: wl.symbol(c"wl_display_flush")?,
                wl_display_dispatch_pending: wl.symbol(c"wl_display_dispatch_pending")?,
                wl_display_prepare_read: wl.symbol(c"wl_display_prepare_read")?,
                wl_display_read_events: wl.symbol(c"wl_display_read_events")?,
                wl_display_cancel_read: wl.symbol(c"wl_display_cancel_read")?,

                wl_proxy_marshal_flags: wl.symbol(c"wl_proxy_marshal_flags")?,
                wl_proxy_add_listener: wl.symbol(c"wl_proxy_add_listener")?,
                wl_proxy_destroy: wl.symbol(c"wl_proxy_destroy")?,
                wl_proxy_get_version: wl.symbol(c"wl_proxy_get_version")?,

                wl_compositor_interface: wl.data(c"wl_compositor_interface")?,
                wl_surface_interface: wl.data(c"wl_surface_interface")?,
                wl_shm_interface: wl.data(c"wl_shm_interface")?,
                wl_shm_pool_interface: wl.data(c"wl_shm_pool_interface")?,
                wl_buffer_interface: wl.data(c"wl_buffer_interface")?,
                wl_seat_interface: wl.data(c"wl_seat_interface")?,
                wl_pointer_interface: wl.data(c"wl_pointer_interface")?,
                wl_output_interface: wl.data(c"wl_output_interface")?,
                wl_registry_interface: wl.data(c"wl_registry_interface")?,
                wl_data_device_manager_interface: wl.data(c"wl_data_device_manager_interface")?,
                wl_data_device_interface: wl.data(c"wl_data_device_interface")?,
                wl_data_source_interface: wl.data(c"wl_data_source_interface")?,

                _library: wl,
            }
        };
        me.patch_core_types();
        Some(me)
    }

    /// Fills in the entries of the hand-written `types` arrays that name a
    /// *core* interface.
    ///
    /// Those interfaces live in libwayland and are only known once it is
    /// loaded, so they cannot be written into a `static` at compile time. The
    /// arrays are read by libwayland while marshalling and by nothing else,
    /// and this runs once, before the first request is sent.
    fn patch_core_types(&self) {
        let set = |slot: &AtomicPtr<WlInterface>, value: *const WlInterface| {
            slot.store(value as *mut WlInterface, Ordering::Relaxed);
        };
        // `zwlr_layer_shell_v1.get_layer_surface(id, surface, output, …)`
        set(&LAYER_SHELL_GET_TYPES[1], self.wl_surface_interface);
        set(&LAYER_SHELL_GET_TYPES[2], self.wl_output_interface);
        // `xdg_wm_base.get_xdg_surface(id, surface)`
        set(&XDG_GET_SURFACE_TYPES[1], self.wl_surface_interface);
    }

    /// The version of an object, which is what a request that creates another
    /// one has to pass on.
    pub fn version(&self, proxy: *mut wl_proxy) -> u32 {
        unsafe { (self.wl_proxy_get_version)(proxy) }
    }
}

/// Data symbols, as opposed to functions. `dlsym` makes no distinction, but
/// the two are used differently enough to be worth separate names.
impl Library {
    pub unsafe fn data<T>(&self, name: &CStr) -> Option<*const T> {
        unsafe { self.symbol::<*const T>(name) }
    }
}
