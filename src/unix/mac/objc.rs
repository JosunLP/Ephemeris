// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! The slice of the Objective-C runtime, AppKit, Core Graphics and Core Text
//! the macOS front end needs.
//!
//! No `objc2` or `cocoa` crate. The reasoning is the one this project applies
//! everywhere: the widget is a ~1.7 MB single file, the surface actually used
//! here is a few dozen selectors, and a binding crate would carry a great deal
//! more than that plus a version to keep in step. What it costs is the two
//! rules below, written down once and then followed.
//!
//! **Rule one: `objc_msgSend` has no single signature.** It is declared here
//! without one and transmuted at each call site to the shape that call
//! actually has. Getting that wrong is undefined behaviour rather than a
//! compile error, so every send goes through one of the small typed helpers
//! ([`send`], [`send1`], …) rather than being transmuted inline.
//!
//! **Rule two: structs bigger than 16 bytes come back differently on the two
//! architectures.** On `aarch64` the caller passes a hidden pointer in `x8`
//! and ordinary `objc_msgSend` is correct. On `x86_64` the System V ABI wants
//! `objc_msgSend_stret`, which takes the destination as its first argument.
//! `NSRect` is 32 bytes and both Macs are supported, so [`send_rect`] picks.
//! `NSPoint` and `NSSize` are 16 bytes and come back in registers on both,
//! which is why they need no such treatment.
//!
//! Memory: AppKit objects created with `alloc`/`init` are owned and released
//! by [`Obj`]. Objects from a `+`-constructor or a property are autoreleased
//! and must not be — the widget's own autorelease pool drains them, which is
//! what [`Pool`] is for.

#![allow(non_snake_case, non_upper_case_globals)]

use std::ffi::{CStr, c_char, c_void};

pub type Id = *mut c_void;
pub type Sel = *const c_void;
pub type Class = *mut c_void;
pub type Imp = *const c_void;
pub type CGFloat = f64;

pub const nil: Id = std::ptr::null_mut();

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct NSPoint {
    pub x: CGFloat,
    pub y: CGFloat,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct NSSize {
    pub width: CGFloat,
    pub height: CGFloat,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct NSRect {
    pub origin: NSPoint,
    pub size: NSSize,
}

impl NSRect {
    pub fn new(x: CGFloat, y: CGFloat, width: CGFloat, height: CGFloat) -> Self {
        Self {
            origin: NSPoint { x, y },
            size: NSSize { width, height },
        }
    }
}

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {
    pub fn objc_getClass(name: *const c_char) -> Class;
    pub fn objc_allocateClassPair(superclass: Class, name: *const c_char, extra: usize) -> Class;
    pub fn objc_registerClassPair(cls: Class);
    pub fn class_addMethod(cls: Class, name: Sel, imp: Imp, types: *const c_char) -> bool;
    pub fn sel_registerName(name: *const c_char) -> Sel;

    /// Declared without arguments on purpose — see the module note.
    pub fn objc_msgSend();
    #[cfg(target_arch = "x86_64")]
    pub fn objc_msgSend_stret();
}

// --- Sending messages -------------------------------------------------------

/// A selector for a literal name.
///
/// `sel_registerName` interns, so repeated calls with the same name return the
/// same pointer and cost a hash lookup. That is cheap enough for a widget that
/// draws only when something changed; caching them would mean either a static
/// table to keep in step with the call sites or a lazily initialised map, and
/// neither is worth it here.
pub fn sel(name: &CStr) -> Sel {
    unsafe { sel_registerName(name.as_ptr()) }
}

pub fn class(name: &CStr) -> Class {
    unsafe { objc_getClass(name.as_ptr()) }
}

/// `[obj name]`
///
/// # Safety
///
/// `obj` must be nil or a valid object that responds to `name`, and `R` must
/// be the selector's real return type — 16 bytes or fewer, or an integer or
/// pointer. Anything larger has to go through [`send_rect`].
pub unsafe fn send<R>(obj: Id, name: &CStr) -> R {
    let f: extern "C" fn(Id, Sel) -> R = unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    f(obj, sel(name))
}

/// `[obj name:a]`
///
/// # Safety
///
/// As [`send`], and `A` must be the argument's real type.
pub unsafe fn send1<A, R>(obj: Id, name: &CStr, a: A) -> R {
    let f: extern "C" fn(Id, Sel, A) -> R =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    f(obj, sel(name), a)
}

/// `[obj name:a with:b]`
///
/// # Safety
///
/// As [`send1`].
pub unsafe fn send2<A, B, R>(obj: Id, name: &CStr, a: A, b: B) -> R {
    let f: extern "C" fn(Id, Sel, A, B) -> R =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    f(obj, sel(name), a, b)
}

/// `[obj name:a with:b and:c]`
///
/// # Safety
///
/// As [`send1`].
pub unsafe fn send3<A, B, C, R>(obj: Id, name: &CStr, a: A, b: B, c: C) -> R {
    let f: extern "C" fn(Id, Sel, A, B, C) -> R =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    f(obj, sel(name), a, b, c)
}

/// `[obj name:a with:b and:c and:d]`
///
/// # Safety
///
/// As [`send1`].
pub unsafe fn send4<A, B, C, D, R>(obj: Id, name: &CStr, a: A, b: B, c: C, d: D) -> R {
    let f: extern "C" fn(Id, Sel, A, B, C, D) -> R =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    f(obj, sel(name), a, b, c, d)
}

/// `[obj name:a with:b and:c and:d and:e]`
///
/// # Safety
///
/// As [`send1`].
pub unsafe fn send5<A, B, C, D, E, R>(obj: Id, name: &CStr, a: A, b: B, c: C, d: D, e: E) -> R {
    let f: extern "C" fn(Id, Sel, A, B, C, D, E) -> R =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    f(obj, sel(name), a, b, c, d, e)
}

/// A message returning an `NSRect`, which is where the two architectures part
/// company — see the module note.
///
/// # Safety
///
/// As [`send`], and the selector must really return an `NSRect`.
pub unsafe fn send_rect(obj: Id, name: &CStr) -> NSRect {
    #[cfg(target_arch = "aarch64")]
    unsafe {
        let f: extern "C" fn(Id, Sel) -> NSRect = std::mem::transmute(objc_msgSend as *const ());
        f(obj, sel(name))
    }
    #[cfg(target_arch = "x86_64")]
    unsafe {
        let f: extern "C" fn(*mut NSRect, Id, Sel) =
            std::mem::transmute(objc_msgSend_stret as *const ());
        let mut out = NSRect::default();
        f(&mut out, obj, sel(name));
        out
    }
}

/// An `NSString` holding a copy of `s`, autoreleased.
///
/// `+stringWithUTF8String:` rather than `alloc`/`init`, so the caller has
/// nothing to release: every one of these is a menu label or a font name that
/// lives until the enclosing [`Pool`] drains.
pub fn nsstring(s: &str) -> Id {
    // Rust strings are not terminated and `+stringWithUTF8String:` reads to a
    // NUL. An interior NUL truncates the label, which is the same thing every
    // C API does with it and better than refusing to draw the menu.
    let c = std::ffi::CString::new(s).unwrap_or_else(|e| {
        let upto = e.nul_position();
        std::ffi::CString::new(&e.into_vec()[..upto]).unwrap_or_default()
    });
    unsafe {
        send1(
            class(c"NSString") as Id,
            c"stringWithUTF8String:",
            c.as_ptr(),
        )
    }
}

/// An object this code owns a reference to, released exactly once.
///
/// For the `alloc`/`init` pairs only. Everything else AppKit hands back is
/// autoreleased and releasing it would be an over-release.
pub struct Obj(Id);

impl Obj {
    /// `None` for nil, so a failure part way through a chain ends the chain
    /// rather than being carried on as a pointer to nothing.
    pub fn new(raw: Id) -> Option<Self> {
        (!raw.is_null()).then_some(Self(raw))
    }

    pub fn id(&self) -> Id {
        self.0
    }
}

impl Drop for Obj {
    fn drop(&mut self) {
        unsafe { send::<()>(self.0, c"release") }
    }
}

/// An autorelease pool.
///
/// The run loop drains one around every event, so a pool is needed only where
/// this code runs outside it: start-up before the loop begins, and the sync
/// thread's wake handler. Without one, every autoreleased string a frame
/// creates would live until the process exits.
pub struct Pool(Id);

impl Default for Pool {
    fn default() -> Self {
        Self::new()
    }
}

impl Pool {
    pub fn new() -> Self {
        unsafe {
            let cls = class(c"NSAutoreleasePool") as Id;
            let raw: Id = send(cls, c"alloc");
            Self(send(raw, c"init"))
        }
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        unsafe { send::<()>(self.0, c"drain") }
    }
}

/// Adds a method to a class being built.
///
/// The type encoding is what the runtime would need to forward the call
/// through `NSInvocation`. Nothing here is ever forwarded, but a wrong
/// encoding is the kind of thing that only shows up under an accessibility
/// tool years later, so they are written out correctly.
///
/// # Safety
///
/// `imp` must be an `extern "C"` function whose signature matches `types`,
/// beginning with the receiver and the selector.
pub unsafe fn add_method(cls: Class, name: &CStr, imp: Imp, types: &CStr) {
    let added = unsafe { class_addMethod(cls, sel(name), imp, types.as_ptr()) };
    debug_assert!(added, "could not add {name:?}");
}

// --- AppKit constants -------------------------------------------------------

/// `NSApplicationActivationPolicyAccessory`: runs with no Dock tile and no
/// menu bar, which is what `LSUIElement` does for a bundle. The widget is
/// shipped as a bare binary as well as in an `.app`, and this works for both.
pub const NSApplicationActivationPolicyAccessory: isize = 1;

pub const NSWindowStyleMaskBorderless: usize = 0;
pub const NSBackingStoreBuffered: usize = 2;

/// Follows the widget onto every Space, never moves with Exposé, and is
/// skipped by the window switcher — the three halves of "stays out of the
/// way".
pub const NSWindowCollectionBehaviorCanJoinAllSpaces: usize = 1 << 0;
pub const NSWindowCollectionBehaviorStationary: usize = 1 << 4;
pub const NSWindowCollectionBehaviorIgnoresCycle: usize = 1 << 6;

/// Above every ordinary window, for the few seconds a peek lasts.
pub const NSFloatingWindowLevel: isize = 3;

pub const NSTrackingMouseEnteredAndExited: usize = 0x01;
pub const NSTrackingMouseMoved: usize = 0x02;
pub const NSTrackingActiveAlways: usize = 0x80;
pub const NSTrackingInVisibleRect: usize = 0x200;

/// `NSEventMaskAny`.
pub const NSEventMaskAny: u64 = u64::MAX;

// --- Core Graphics ----------------------------------------------------------

pub type CGContextRef = *mut c_void;
pub type CGColorSpaceRef = *mut c_void;
pub type CGGradientRef = *mut c_void;
pub type CGPathRef = *mut c_void;
pub type CGWindowLevel = i32;

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct CGAffineTransform {
    pub a: CGFloat,
    pub b: CGFloat,
    pub c: CGFloat,
    pub d: CGFloat,
    pub tx: CGFloat,
    pub ty: CGFloat,
}

/// `kCGDesktopIconWindowLevelKey`. One above it is the level the widget sits
/// at: below every ordinary window, above the desktop and its icons, and still
/// clickable.
pub const kCGDesktopIconWindowLevelKey: i32 = 18;

/// `kCGGradientDrawsBeforeStartLocation | kCGGradientDrawsAfterEndLocation`,
/// which is Core Graphics' clamp: the end colours continue past the ends
/// rather than leaving a gap.
pub const kCGGradientClamp: u32 = 1 | 2;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    pub fn CGWindowLevelForKey(key: i32) -> CGWindowLevel;

    pub fn CGContextSaveGState(c: CGContextRef);
    pub fn CGContextRestoreGState(c: CGContextRef);
    pub fn CGContextSetRGBFillColor(
        c: CGContextRef,
        r: CGFloat,
        g: CGFloat,
        b: CGFloat,
        a: CGFloat,
    );
    pub fn CGContextSetRGBStrokeColor(
        c: CGContextRef,
        r: CGFloat,
        g: CGFloat,
        b: CGFloat,
        a: CGFloat,
    );
    pub fn CGContextSetLineWidth(c: CGContextRef, width: CGFloat);
    pub fn CGContextSetLineCap(c: CGContextRef, cap: i32);
    pub fn CGContextSetLineJoin(c: CGContextRef, join: i32);
    pub fn CGContextBeginPath(c: CGContextRef);
    pub fn CGContextClosePath(c: CGContextRef);
    pub fn CGContextMoveToPoint(c: CGContextRef, x: CGFloat, y: CGFloat);
    pub fn CGContextAddLineToPoint(c: CGContextRef, x: CGFloat, y: CGFloat);
    pub fn CGContextAddArc(
        c: CGContextRef,
        x: CGFloat,
        y: CGFloat,
        radius: CGFloat,
        start: CGFloat,
        end: CGFloat,
        clockwise: i32,
    );
    pub fn CGContextAddArcToPoint(
        c: CGContextRef,
        x1: CGFloat,
        y1: CGFloat,
        x2: CGFloat,
        y2: CGFloat,
        radius: CGFloat,
    );
    pub fn CGContextFillPath(c: CGContextRef);
    pub fn CGContextStrokePath(c: CGContextRef);
    pub fn CGContextClip(c: CGContextRef);
    pub fn CGContextClipToRect(c: CGContextRef, rect: NSRect);
    pub fn CGContextClearRect(c: CGContextRef, rect: NSRect);
    pub fn CGContextTranslateCTM(c: CGContextRef, tx: CGFloat, ty: CGFloat);
    pub fn CGContextScaleCTM(c: CGContextRef, sx: CGFloat, sy: CGFloat);
    pub fn CGContextSetTextMatrix(c: CGContextRef, t: CGAffineTransform);
    pub fn CGContextSetTextPosition(c: CGContextRef, x: CGFloat, y: CGFloat);
    pub fn CGContextDrawLinearGradient(
        c: CGContextRef,
        gradient: CGGradientRef,
        start: NSPoint,
        end: NSPoint,
        options: u32,
    );

    pub fn CGColorSpaceCreateDeviceRGB() -> CGColorSpaceRef;
    pub fn CGColorSpaceRelease(space: CGColorSpaceRef);
    pub fn CGGradientCreateWithColorComponents(
        space: CGColorSpaceRef,
        components: *const CGFloat,
        locations: *const CGFloat,
        count: usize,
    ) -> CGGradientRef;
    pub fn CGGradientRelease(gradient: CGGradientRef);
}

pub const kCGLineCapButt: i32 = 0;
pub const kCGLineCapRound: i32 = 1;
pub const kCGLineJoinRound: i32 = 1;

// --- Core Text --------------------------------------------------------------

pub type CTFontRef = *const c_void;
pub type CTLineRef = *const c_void;
pub type CTFramesetterRef = *const c_void;
pub type CTFrameRef = *const c_void;

/// `kCTFontUIFontSystem`: the interface font the rest of the system uses, at
/// whatever size is asked for. Better than naming a family, which changes
/// between releases and does not follow the user's accessibility settings.
pub const kCTFontUIFontSystem: u32 = 2;
/// `kCTFontTraitBold` in `CTFontSymbolicTraits`.
pub const kCTFontTraitBold: u32 = 1 << 1;

#[link(name = "CoreText", kind = "framework")]
unsafe extern "C" {
    pub fn CTFontCreateUIFontForLanguage(
        uiType: u32,
        size: CGFloat,
        language: crate::unix::cf::CFStringRef,
    ) -> CTFontRef;
    pub fn CTFontCreateWithName(
        name: crate::unix::cf::CFStringRef,
        size: CGFloat,
        matrix: *const CGAffineTransform,
    ) -> CTFontRef;
    pub fn CTFontCreateCopyWithSymbolicTraits(
        font: CTFontRef,
        size: CGFloat,
        matrix: *const CGAffineTransform,
        symTraitValue: u32,
        symTraitMask: u32,
    ) -> CTFontRef;

    pub fn CTLineCreateWithAttributedString(string: crate::unix::cf::CFTypeRef) -> CTLineRef;
    pub fn CTLineCreateTruncatedLine(
        line: CTLineRef,
        width: f64,
        truncationType: u32,
        truncationToken: CTLineRef,
    ) -> CTLineRef;
    pub fn CTLineGetTypographicBounds(
        line: CTLineRef,
        ascent: *mut CGFloat,
        descent: *mut CGFloat,
        leading: *mut CGFloat,
    ) -> f64;
    pub fn CTLineDraw(line: CTLineRef, context: CGContextRef);

    pub fn CTFramesetterCreateWithAttributedString(
        string: crate::unix::cf::CFTypeRef,
    ) -> CTFramesetterRef;
    pub fn CTFramesetterSuggestFrameSizeWithConstraints(
        framesetter: CTFramesetterRef,
        stringRange: CFRange,
        frameAttributes: crate::unix::cf::CFTypeRef,
        constraints: NSSize,
        fitRange: *mut CFRange,
    ) -> NSSize;
    pub fn CTFramesetterCreateFrame(
        framesetter: CTFramesetterRef,
        stringRange: CFRange,
        path: CGPathRef,
        frameAttributes: crate::unix::cf::CFTypeRef,
    ) -> CTFrameRef;
    pub fn CTFrameDraw(frame: CTFrameRef, context: CGContextRef);

    pub static kCTFontAttributeName: crate::unix::cf::CFStringRef;
    /// Take the colour from the context's fill colour, which saves creating
    /// a `CGColor` for every run.
    pub static kCTForegroundColorFromContextAttributeName: crate::unix::cf::CFStringRef;
    pub static kCTParagraphStyleAttributeName: crate::unix::cf::CFStringRef;

    pub fn CTParagraphStyleCreate(
        settings: *const CTParagraphStyleSetting,
        settingCount: usize,
    ) -> crate::unix::cf::CFTypeRef;
}

/// `kCTLineTruncationEnd` — the ellipsis goes at the end of the line.
///
/// `CTLineTruncationType` is `Start = 0, End = 1, Middle = 2`; 2 would put the
/// ellipsis in the middle of every truncated row.
pub const kCTLineTruncationEnd: u32 = 1;

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct CFRange {
    pub location: isize,
    pub length: isize,
}

#[repr(C)]
pub struct CTParagraphStyleSetting {
    pub spec: u32,
    pub valueSize: usize,
    pub value: *const c_void,
}

/// `kCTParagraphStyleSpecifierBaseWritingDirection`.
pub const kCTParagraphStyleSpecifierBaseWritingDirection: u32 = 13;

/// `CTWritingDirection`. Needed for the base direction of a paragraph: an
/// Arabic layout has to start its lines at the right even when the text in
/// them happens to be Latin.
pub const kCTWritingDirectionLeftToRight: i8 = 0;
pub const kCTWritingDirectionRightToLeft: i8 = 1;

// --- Core Foundation attributed strings -------------------------------------

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    pub fn CFAttributedStringCreate(
        alloc: crate::unix::cf::CFTypeRef,
        str: crate::unix::cf::CFStringRef,
        attributes: crate::unix::cf::CFDictionaryRef,
    ) -> crate::unix::cf::CFTypeRef;
    pub fn CFRunLoopGetMain() -> *mut c_void;
    pub fn CFRunLoopWakeUp(rl: *mut c_void);
}

// --- Carbon: the global hotkey ----------------------------------------------

pub type EventTargetRef = *mut c_void;
pub type EventHandlerRef = *mut c_void;
pub type EventHotKeyRef = *mut c_void;
pub type EventRef = *mut c_void;
pub type EventHandlerCallRef = *mut c_void;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct EventTypeSpec {
    pub eventClass: u32,
    pub eventKind: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct EventHotKeyID {
    pub signature: u32,
    pub id: u32,
}

/// `kEventClassKeyboard` / `kEventHotKeyPressed`.
pub const kEventClassKeyboard: u32 = u32::from_be_bytes(*b"keyb");
pub const kEventHotKeyPressed: u32 = 5;

/// Carbon modifier masks, which are not the same numbers as AppKit's.
pub const cmdKey: u32 = 0x0100;
pub const shiftKey: u32 = 0x0200;
pub const optionKey: u32 = 0x0800;
pub const controlKey: u32 = 0x1000;

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    pub fn GetApplicationEventTarget() -> EventTargetRef;
    pub fn InstallEventHandler(
        target: EventTargetRef,
        handler: Imp,
        numTypes: u32,
        list: *const EventTypeSpec,
        userData: *mut c_void,
        outRef: *mut EventHandlerRef,
    ) -> i32;
    pub fn RegisterEventHotKey(
        code: u32,
        modifiers: u32,
        id: EventHotKeyID,
        target: EventTargetRef,
        options: u32,
        outRef: *mut EventHotKeyRef,
    ) -> i32;
    pub fn UnregisterEventHotKey(hotKey: EventHotKeyRef) -> i32;
}
