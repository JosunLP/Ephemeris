// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Ephemeris contributors
//! The slice of Core Foundation the macOS front end needs.
//!
//! Two things reach for it — the date formatter in [`super::locale`] and the
//! Keychain in [`super::secure`] — and both need the same handful of pieces:
//! strings, data, dictionaries, and a way to release what they own. Declared
//! once here rather than twice, because the second copy is where the release
//! rule gets forgotten.
//!
//! **The release rule.** Core Foundation's naming convention is the whole
//! contract: a function with `Create` or `Copy` in its name hands over a
//! reference this code owns and must release; a function with `Get` does not.
//! [`Owned`] carries the first kind, and every `Get` below is commented where
//! it is used. The widget formats a clock every minute for days, so this is not
//! a rounding error.
//!
//! Core Foundation rather than the Objective-C classes on top of it: it is a
//! plain C API, so there is no message-send machinery to get right and no
//! additional crate to carry.

#![allow(non_snake_case, non_upper_case_globals)]

use std::ffi::{c_char, c_void};

pub type CFTypeRef = *const c_void;
pub type CFStringRef = CFTypeRef;
pub type CFDataRef = CFTypeRef;
pub type CFDictionaryRef = CFTypeRef;
pub type CFAllocatorRef = CFTypeRef;
pub type CFIndex = isize;
pub type CFOptionFlags = usize;
pub type CFAbsoluteTime = f64;
pub type CFStringEncoding = u32;
pub type Boolean = u8;

pub const kCFStringEncodingUTF8: CFStringEncoding = 0x0800_0100;

/// `CFAbsoluteTime` counts seconds from 2001-01-01 UTC; Unix time counts from
/// 1970.
pub const EPOCH_OFFSET: f64 = 978_307_200.0;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    pub fn CFRelease(cf: CFTypeRef);

    pub fn CFStringCreateWithBytes(
        alloc: CFAllocatorRef,
        bytes: *const u8,
        numBytes: CFIndex,
        encoding: CFStringEncoding,
        isExternalRepresentation: Boolean,
    ) -> CFStringRef;
    pub fn CFStringGetLength(theString: CFStringRef) -> CFIndex;
    pub fn CFStringGetCString(
        theString: CFStringRef,
        buffer: *mut c_char,
        bufferSize: CFIndex,
        encoding: CFStringEncoding,
    ) -> Boolean;

    pub fn CFDataCreate(alloc: CFAllocatorRef, bytes: *const u8, length: CFIndex) -> CFDataRef;
    pub fn CFDataGetBytePtr(theData: CFDataRef) -> *const u8;
    pub fn CFDataGetLength(theData: CFDataRef) -> CFIndex;

    pub fn CFDictionaryCreate(
        allocator: CFAllocatorRef,
        keys: *const CFTypeRef,
        values: *const CFTypeRef,
        numValues: CFIndex,
        keyCallBacks: *const c_void,
        valueCallBacks: *const c_void,
    ) -> CFDictionaryRef;

    pub static kCFTypeDictionaryKeyCallBacks: c_void;
    pub static kCFTypeDictionaryValueCallBacks: c_void;
    pub static kCFBooleanTrue: CFTypeRef;

    pub fn CFGetTypeID(cf: CFTypeRef) -> usize;
    pub fn CFDataGetTypeID() -> usize;
}

/// A Core Foundation value this code owns a reference to.
///
/// Constructed from the return of a `Create` or `Copy` function and released
/// exactly once when it goes out of scope.
pub struct Owned(CFTypeRef);

impl Owned {
    /// `None` for a null return, so a failure part-way through a chain ends the
    /// chain rather than being carried on as a pointer to nothing.
    ///
    /// `then` rather than `then_some`, which takes a value and would build the
    /// `Owned` either way — including around the null this is here to catch,
    /// and that one is dropped a moment later into `CFRelease(NULL)`, which
    /// Core Foundation treats as a fatal error rather than as nothing to do.
    pub fn new(raw: CFTypeRef) -> Option<Self> {
        (!raw.is_null()).then(|| Self(raw))
    }

    pub fn as_raw(&self) -> CFTypeRef {
        self.0
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        unsafe { CFRelease(self.0) };
    }
}

/// A `CFString` holding a copy of `s`.
pub fn string(s: &str) -> Option<Owned> {
    Owned::new(unsafe {
        CFStringCreateWithBytes(
            std::ptr::null(),
            s.as_ptr(),
            s.len() as CFIndex,
            kCFStringEncodingUTF8,
            false as Boolean,
        )
    })
}

/// A `CFData` holding a copy of `bytes`.
pub fn data(bytes: &[u8]) -> Option<Owned> {
    Owned::new(unsafe { CFDataCreate(std::ptr::null(), bytes.as_ptr(), bytes.len() as CFIndex) })
}

/// The bytes of a `CFData`, copied out.
///
/// A copy rather than a borrow: the caller releases the data, and a slice into
/// it would outlive what it points at. The type is checked before the bytes are
/// read, because `SecItemCopyMatching` returns whatever the query asked for and
/// a changed query would hand back a dictionary.
///
/// # Safety
///
/// `d` must be null or a valid Core Foundation object that stays alive for the
/// duration of the call.
pub unsafe fn data_bytes(d: CFDataRef) -> Option<Vec<u8>> {
    if d.is_null() || unsafe { CFGetTypeID(d) } != unsafe { CFDataGetTypeID() } {
        return None;
    }
    let ptr = unsafe { CFDataGetBytePtr(d) };
    let len = unsafe { CFDataGetLength(d) };
    if ptr.is_null() || len < 0 {
        return None;
    }
    Some(unsafe { std::slice::from_raw_parts(ptr, len as usize) }.to_vec())
}

/// A Rust string from a `CFString`.
///
/// `CFStringGetCString` needs a buffer big enough for the encoded form, and the
/// length it reports counts UTF-16 units. Four bytes per unit is the most UTF-8
/// can ever need, plus the terminator.
///
/// # Safety
///
/// `s` must be null or a valid `CFStringRef` that stays alive for the duration
/// of the call.
pub unsafe fn to_string(s: CFStringRef) -> Option<String> {
    if s.is_null() {
        return None;
    }
    let capacity = (unsafe { CFStringGetLength(s) } as usize) * 4 + 1;
    let mut buf = vec![0u8; capacity];
    let ok = unsafe {
        CFStringGetCString(
            s,
            buf.as_mut_ptr() as *mut c_char,
            capacity as CFIndex,
            kCFStringEncodingUTF8,
        )
    };
    if ok == 0 {
        return None;
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    buf.truncate(end);
    String::from_utf8(buf).ok()
}

/// An immutable dictionary from parallel key and value slices.
///
/// The type callbacks are what make the dictionary retain its contents, so the
/// `Owned` values the caller built may be dropped as soon as this returns.
pub fn dictionary(keys: &[CFTypeRef], values: &[CFTypeRef]) -> Option<Owned> {
    if keys.len() != values.len() {
        return None;
    }
    Owned::new(unsafe {
        CFDictionaryCreate(
            std::ptr::null(),
            keys.as_ptr(),
            values.as_ptr(),
            keys.len() as CFIndex,
            &raw const kCFTypeDictionaryKeyCallBacks,
            &raw const kCFTypeDictionaryValueCallBacks,
        )
    })
}
