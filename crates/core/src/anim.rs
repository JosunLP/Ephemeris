// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! Animation state.
//!
//! Deliberately without a render loop: the window starts a 16 ms timer *only*
//! while [`Animations::tick`] reports that something is still moving, and
//! falls back to the one minute tick afterwards. A widget at rest therefore
//! still costs no CPU time at all.

use std::time::Instant;

/// Exponential approach towards a target value.
///
/// The `1 - e^(-rate·dt)` form, rather than a fixed step per frame, makes the
/// motion independent of the actual frame rate: a missed frame moves the value
/// correspondingly further instead of stretching the animation.
#[derive(Debug, Clone, Copy)]
pub struct Spring {
    pub value: f32,
    pub target: f32,
    /// Larger is faster. 10 is crisp, 4 is soft.
    rate: f32,
}

impl Spring {
    pub fn new(value: f32, rate: f32) -> Self {
        Self {
            value,
            target: value,
            rate,
        }
    }

    pub fn set(&mut self, target: f32) {
        self.target = target;
    }

    /// Set directly without animating, for the first frame for instance.
    pub fn jump(&mut self, value: f32) {
        self.value = value;
        self.target = value;
    }

    pub fn tick(&mut self, dt: f32) -> bool {
        let delta = self.target - self.value;
        if delta.abs() < 0.0015 {
            self.value = self.target;
            return false;
        }
        self.value += delta * (1.0 - (-self.rate * dt).exp());
        true
    }
}

pub struct Animations {
    /// 0 to 1 when new data arrives: the content fades in and slides up
    /// slightly.
    pub reveal: Spring,
    /// Smoothed scroll offset.
    pub scroll: Spring,
    /// Opacity of the hover highlight.
    pub hover: Spring,
    /// Opacity of the scrollbar; fades out again after scrolling.
    pub scrollbar: Spring,
    /// Rotation of the refresh glyph, in radians.
    pub spinner: f32,
    pub spinning: bool,

    /// Follows the system's "show animations" setting. With it off every
    /// value jumps straight to its target — motion can trigger symptoms in
    /// people with vestibular disorders, which is exactly why the system
    /// asks.
    pub enabled: bool,

    last: Option<Instant>,
}

impl Default for Animations {
    fn default() -> Self {
        Self {
            reveal: Spring::new(0.0, 9.0),
            scroll: Spring::new(0.0, 18.0),
            hover: Spring::new(0.0, 16.0),
            scrollbar: Spring::new(0.0, 7.0),
            spinner: 0.0,
            spinning: false,
            enabled: true,
            last: None,
        }
    }
}

impl Animations {
    /// New data: fade in from below.
    pub fn restart_reveal(&mut self) {
        if !self.enabled {
            self.reveal.jump(1.0);
            return;
        }
        self.reveal.value = 0.0;
        self.reveal.target = 1.0;
    }

    /// Advances every value and reports whether animation must continue.
    pub fn tick(&mut self) -> bool {
        if !self.enabled {
            // Everything straight to its target; the caller then stops the
            // timer and draws exactly once.
            self.reveal.jump(self.reveal.target);
            self.scroll.jump(self.scroll.target);
            self.hover.jump(self.hover.target);
            self.scrollbar.jump(self.scrollbar.target);
            self.spinner = 0.0;
            self.last = None;
            return false;
        }

        let now = Instant::now();
        // Do not allow a huge time step on the first call or after a long
        // pause such as standby, or everything would snap to its target at
        // once.
        let dt = match self.last {
            Some(prev) => (now - prev).as_secs_f32().min(0.1),
            None => 1.0 / 60.0,
        };
        self.last = Some(now);

        let mut active = false;
        active |= self.reveal.tick(dt);
        active |= self.scroll.tick(dt);
        active |= self.hover.tick(dt);
        active |= self.scrollbar.tick(dt);

        if self.spinning {
            self.spinner = (self.spinner + dt * 5.0) % std::f32::consts::TAU;
            active = true;
        } else if self.spinner != 0.0 {
            // Do not stop mid-rotation; coast to the next full turn.
            self.spinner = (self.spinner + dt * 5.0) % std::f32::consts::TAU;
            if self.spinner < 0.25 {
                self.spinner = 0.0;
            } else {
                active = true;
            }
        }

        if !active {
            self.last = None;
        }
        active
    }
}
