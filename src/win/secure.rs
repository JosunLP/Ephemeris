// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Ephemeris contributors
//! Windows DPAPI wrapper for stored credentials.
//!
//! A refresh token is standing access to a calendar account and has no
//! business sitting on disk in plain text. `CryptProtectData` ties it to the
//! Windows user account: another user of the same machine cannot read it, and
//! copying the file to a different machine makes it worthless.

use ephemeris_core::log;
use windows::Win32::Foundation::{HLOCAL, LocalFree};
use windows::Win32::Security::Cryptography::{
    BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom, CRYPT_INTEGER_BLOB, CryptProtectData,
    CryptUnprotectData,
};
use windows::core::PCWSTR;

/// Base entropy. The per-account tag is appended so one account's stored
/// token cannot be decrypted in the context of another, and so a file copied
/// from a different program is useless here.
const ENTROPY_BASE: &[u8] = b"Ephemeris/v1/";

/// The base the same secrets were encrypted with before the program was
/// renamed.
///
/// The entropy is part of the key: a refresh token written by the previous
/// version cannot be decrypted with the base above, and dropping it would mean
/// every existing installation silently losing its calendar sign-ins on
/// upgrade. So [`unprotect`] falls back to this one, and what it decrypts is
/// written back under the current base the next time that secret is stored.
const LEGACY_ENTROPY_BASE: &[u8] = b"TPMPlaner/v1/";

fn entropy_for(base: &[u8], tag: &[u8]) -> Vec<u8> {
    let mut buf = base.to_vec();
    buf.extend_from_slice(tag);
    buf
}

fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
    CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr() as *mut u8,
    }
}

/// Copies the result blob out and releases the memory DPAPI allocated.
unsafe fn take_blob(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
    if out.pbData.is_null() {
        return Vec::new();
    }
    let v = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize) }.to_vec();
    unsafe {
        let _ = LocalFree(Some(HLOCAL(out.pbData as *mut _)));
    }
    v
}

pub fn protect(plain: &[u8], tag: &[u8]) -> Option<Vec<u8>> {
    unsafe {
        let salt = entropy_for(ENTROPY_BASE, tag);
        let input = blob(plain);
        let entropy = blob(&salt);
        let mut out = CRYPT_INTEGER_BLOB::default();
        CryptProtectData(
            &input,
            PCWSTR::null(),
            Some(&entropy),
            None,
            None,
            0,
            &mut out,
        )
        .ok()?;
        Some(take_blob(out))
    }
}

/// Decrypts what [`protect`] wrote — under either entropy base.
///
/// The second attempt is what carries an installation across the rename; it
/// costs one extra DPAPI call, and only on the path that was about to fail
/// anyway. See [`LEGACY_ENTROPY_BASE`], which can go once no installation
/// predating the rename is plausible.
pub fn unprotect(cipher: &[u8], tag: &[u8]) -> Option<Vec<u8>> {
    with_entropy(ENTROPY_BASE, cipher, tag)
        .or_else(|| with_entropy(LEGACY_ENTROPY_BASE, cipher, tag))
}

fn with_entropy(base: &[u8], cipher: &[u8], tag: &[u8]) -> Option<Vec<u8>> {
    unsafe {
        let salt = entropy_for(base, tag);
        let input = blob(cipher);
        let entropy = blob(&salt);
        let mut out = CRYPT_INTEGER_BLOB::default();
        CryptUnprotectData(&input, None, Some(&entropy), None, None, 0, &mut out).ok()?;
        Some(take_blob(out))
    }
}

/// Cryptographically secure random bytes for PKCE verifiers and OAuth state.
///
/// Uses the system generator directly rather than carrying an RNG crate.
/// `None` rather than a short or zeroed buffer on failure — see
/// [`ephemeris_core::host::Host::random_bytes`] for why there is nothing safe
/// to substitute.
pub fn random_bytes(len: usize) -> Option<Vec<u8>> {
    let mut buf = vec![0u8; len];
    let status = unsafe { BCryptGenRandom(None, &mut buf, BCRYPT_USE_SYSTEM_PREFERRED_RNG) };
    if status.is_err() {
        log::error(&format!("BCryptGenRandom failed: {status:?}"));
        return None;
    }
    Some(buf)
}
