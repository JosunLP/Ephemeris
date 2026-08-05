// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! Windows-DPAPI-Wrapper fuer den Refresh-Token.
//!
//! Der Token ist ein Dauerzugang zum Google-Konto und hat als Klartext auf der
//! Platte nichts verloren. `CryptProtectData` bindet ihn an das Windows-
//! Benutzerkonto: ein anderer Benutzer desselben Rechners kann ihn nicht lesen,
//! und kopiert man die Datei auf eine andere Maschine, ist sie wertlos.

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

/// Kopiert das Ergebnis-Blob heraus und gibt den von DPAPI allozierten
/// Speicher wieder frei.
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

/// Kryptographisch sichere Zufallsbytes fuer PKCE-Verifier und State.
///
/// Nutzt den System-RNG direkt, statt eine RNG-Crate mitzuschleppen.
pub fn random_bytes(len: usize) -> Vec<u8> {
    let mut buf = vec![0u8; len];
    unsafe {
        let status = BCryptGenRandom(None, &mut buf, BCRYPT_USE_SYSTEM_PREFERRED_RNG);
        assert!(status.is_ok(), "BCryptGenRandom fehlgeschlagen");
    }
    buf
}
