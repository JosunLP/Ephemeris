// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! Windows DPAPI wrapper for stored credentials.
//!
//! A refresh token is standing access to a calendar account and has no
//! business sitting on disk in plain text. `CryptProtectData` ties it to the
//! Windows user account: another user of the same machine cannot read it, and
//! copying the file to a different machine makes it worthless.

use tpmplaner_core::log;
use windows::Win32::Foundation::{HLOCAL, LocalFree};
use windows::Win32::Security::Cryptography::{
    BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom, CRYPT_INTEGER_BLOB, CryptProtectData,
    CryptUnprotectData,
};
use windows::core::PCWSTR;

/// Base entropy. The per-account tag is appended so one account's stored
/// token cannot be decrypted in the context of another, and so a file copied
/// from a different program is useless here.
const ENTROPY_BASE: &[u8] = b"TPMPlaner/v1/";

fn entropy_for(tag: &[u8]) -> Vec<u8> {
    let mut buf = ENTROPY_BASE.to_vec();
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
        let salt = entropy_for(tag);
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

pub fn unprotect(cipher: &[u8], tag: &[u8]) -> Option<Vec<u8>> {
    unsafe {
        let salt = entropy_for(tag);
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
/// [`tpmplaner_core::host::Host::random_bytes`] for why there is nothing safe
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
