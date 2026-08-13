// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! What the widget looks like, and how it is drawn.
//!
//! One description of the interface, for every front end. [`widget::draw`]
//! paints the whole panel against [`canvas::Canvas`], and each platform
//! implements that trait over whatever it has: Direct2D on Windows, Core
//! Graphics on macOS, Cairo on Wayland and X11.
//!
//! Nothing here calls an operating system API, so it lives beside the front
//! ends rather than inside one. It is not in `tpmplaner-core` either, and the
//! line is worth naming: the core answers *what to show* and *where it goes* —
//! which rows are visible, how wide a column has to be, which rectangle a
//! click landed in — and can be tested without a device. This answers *what it
//! looks like*, which cannot be tested without one, and would drag a notion of
//! drawing into a crate whose whole point is not having one.

pub mod canvas;
pub mod widget;
