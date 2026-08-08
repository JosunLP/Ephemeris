// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! Tests for the claims made in the changelog that unit tests did not cover.
//!
//! Several features had only ever been reasoned about: the panic hook, log
//! rotation, the completeness of the translation catalogues, and the release
//! check against the live GitHub API. Each of those is a path that runs once,
//! in a situation where nobody is watching, which is exactly the kind of code
//! that quietly does not work.
//!
//! The host trait is what makes most of this testable: a temporary directory
//! can be installed as the data directory without touching a real one.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tpmplaner_core::host::{Host, PortableHost};
use tpmplaner_core::{i18n, log, update};

/// A host that redirects the data directory into a scratch folder.
struct TempHost(PathBuf);

impl Host for TempHost {
    fn data_dir(&self) -> PathBuf {
        self.0.clone()
    }
    fn open_url(&self, _url: &str) {}
    fn protect(&self, plain: &[u8], tag: &[u8]) -> Option<Vec<u8>> {
        PortableHost.protect(plain, tag)
    }
    fn unprotect(&self, cipher: &[u8], tag: &[u8]) -> Option<Vec<u8>> {
        PortableHost.unprotect(cipher, tag)
    }
    fn random_bytes(&self, len: usize) -> Option<Vec<u8>> {
        PortableHost.random_bytes(len)
    }
}

/// The host is global state, so the two tests that replace it must not run at
/// the same time. Without this they overwrite each other's data directory and
/// fail in a way that looks like a product bug.
static HOST_LOCK: Mutex<()> = Mutex::new(());

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tpmplaner-test-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch directory");
    dir
}

/// Every catalogue has to be complete, in a way a compiler can enforce.
///
/// The catalogue is a struct of named fields rather than a map precisely so a
/// forgotten translation cannot compile. This test guards the other half: that
/// no field was filled in with the English text by accident, and that none is
/// empty.
#[test]
fn every_shipped_language_is_complete_and_distinct() {
    let catalogues = [
        ("en", &i18n::EN),
        ("de", &i18n::DE),
        ("fr", &i18n::FR),
        ("es", &i18n::ES),
        ("it", &i18n::IT),
    ];

    for (code, cat) in catalogues {
        assert_eq!(cat.code, code);
        // A handful of representative fields; empty text would show as a gap
        // in the interface.
        for (name, value) in [
            ("section_events", cat.section_events),
            ("section_tasks", cat.section_tasks),
            ("no_events", cat.no_events),
            ("no_tasks", cat.no_tasks),
            ("now_label", cat.now_label),
            ("next_label", cat.next_label),
            ("undo", cat.undo),
            ("menu_sync", cat.menu_sync),
            ("menu_quit", cat.menu_quit),
            ("setup_needed", cat.setup_needed),
            ("update_available", cat.update_available),
            ("tomorrow", cat.tomorrow),
        ] {
            assert!(!value.trim().is_empty(), "{code}: {name} is empty");
        }

        // Patterns must keep their placeholder, or the value silently
        // disappears from the rendered string.
        for (name, value) in [
            ("in_pattern", cat.in_pattern),
            ("ago_pattern", cat.ago_pattern),
            ("left_pattern", cat.left_pattern),
            ("overdue_one", cat.overdue_one),
            ("overdue_many", cat.overdue_many),
            ("conflict_one", cat.conflict_one),
            ("conflict_many", cat.conflict_many),
            ("update_available", cat.update_available),
        ] {
            assert!(value.contains("{}"), "{code}: {name} lost its placeholder");
        }
        // Two placeholders: updated time and next time.
        assert_eq!(
            cat.updated_next.matches("{}").count(),
            2,
            "{code}: updated_next needs two placeholders"
        );
    }

    // The four translations must not simply be the English text.
    for (code, cat) in &catalogues[1..] {
        assert_ne!(
            cat.section_tasks,
            i18n::EN.section_tasks,
            "{code}: section_tasks was never translated"
        );
        assert_ne!(
            cat.menu_quit,
            i18n::EN.menu_quit,
            "{code}: menu_quit was never translated"
        );
    }
}

/// Relative times have to work in every shipped language, not only the two
/// that were looked at on screen.
#[test]
fn relative_times_render_in_every_language() {
    for tag in ["en-US", "de-DE", "fr-FR", "es-ES", "it-IT"] {
        let loc = i18n::Locale::resolve(tag);
        for minutes in [-90i64, -5, 0, 25, 130, 3000] {
            let text = loc.relative(minutes);
            assert!(!text.trim().is_empty(), "{tag}: empty for {minutes}");
            assert!(!text.contains("{}"), "{tag}: placeholder left in {text}");
        }
        assert!(!loc.time_left(32).contains("{}"), "{tag}: time_left");
        assert!(!loc.overdue(1).contains("{}"), "{tag}: overdue(1)");
        assert!(!loc.overdue(3).contains("{}"), "{tag}: overdue(3)");
        assert!(!loc.conflicts(2).contains("{}"), "{tag}: conflicts");
        let updated = loc.updated_next("09:00", "09:30");
        assert!(
            updated.contains("09:00") && updated.contains("09:30"),
            "{tag}"
        );
    }
}

/// The log has to rotate, or it grows without bound on a machine that runs the
/// widget for months.
#[test]
fn the_log_rotates_once_it_grows_too_large() {
    let _guard = HOST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = scratch("log-rotation");
    tpmplaner_core::host::set_host(Arc::new(TempHost(dir.clone())));

    let path = dir.join("tpmplaner.log");
    let rotated = dir.join("tpmplaner.log.1");

    // Write past the 256 KB threshold. Each line is roughly 120 bytes.
    for i in 0..2600 {
        log::info(&format!(
            "line {i} padded so the file grows at a realistic rate {}",
            "x".repeat(60)
        ));
    }

    assert!(rotated.exists(), "no rotated file was produced");
    let live = std::fs::metadata(&path).expect("live log").len();
    assert!(
        live < 300 * 1024,
        "the live log did not shrink after rotation: {live} bytes"
    );

    // The most recent line must be in the live file, not the rotated one.
    let tail = std::fs::read_to_string(&path).expect("read live log");
    assert!(tail.contains("line 2599"), "the newest line was lost");

    tpmplaner_core::host::set_host(Arc::new(PortableHost));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The panic hook is the difference between "the widget vanished" and knowing
/// why. It runs exactly once, at the worst possible moment, so it had better
/// work.
#[test]
fn a_panic_is_recorded_before_the_process_would_die() {
    let _guard = HOST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = scratch("panic-hook");
    tpmplaner_core::host::set_host(Arc::new(TempHost(dir.clone())));
    log::install_panic_hook();

    // Under the test profile panics unwind, so the hook can be observed
    // without taking the test runner down with it.
    let result = std::panic::catch_unwind(|| {
        panic!("deliberate failure for the panic hook test");
    });
    assert!(result.is_err(), "the panic did not happen");

    let text = std::fs::read_to_string(dir.join("tpmplaner.log")).expect("log file");
    assert!(text.contains("PANIC"), "no panic line in the log:\n{text}");
    assert!(
        text.contains("deliberate failure for the panic hook test"),
        "the panic message was lost:\n{text}"
    );
    assert!(
        text.contains("changelog_claims.rs"),
        "the source location was lost:\n{text}"
    );

    let _ = std::panic::take_hook();
    tpmplaner_core::host::set_host(Arc::new(PortableHost));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The update check talks to the live GitHub API.
///
/// Ignored by default so the test suite stays offline-safe; run it with
/// `cargo test -- --ignored` to exercise the real request. A repository
/// without any release answers 404, and the check has to treat that as "no
/// update" rather than as an error worth showing anybody.
#[test]
#[ignore = "requires network access"]
fn the_release_check_survives_a_repository_without_releases() {
    let result = update::check();
    assert!(
        result.is_none(),
        "expected no update, got {result:?} — either a release now exists or \
         the check mishandled the response"
    );
}

/// The appearance switches are accessibility settings, not decoration. Each is
/// checked here because the operating system dialog cannot be driven from a
/// test, and "we read the value" is not the same as "we act on it".
#[test]
fn the_system_appearance_switches_actually_change_the_palette() {
    use tpmplaner_core::theme::{ContrastColors, Palette, SystemVisuals, ThemePref};

    let base = SystemVisuals::default();

    // Transparency off must force an opaque panel, whatever the user
    // configured.
    let opaque = Palette::resolve(
        ThemePref::Dark,
        "system",
        SystemVisuals {
            transparency: false,
            ..base
        },
    );
    assert!(opaque.force_opaque);
    assert_eq!(
        opaque.opacity(0.5),
        1.0,
        "a configured 0.5 must be overridden"
    );

    let translucent = Palette::resolve(ThemePref::Dark, "system", base);
    assert!(!translucent.force_opaque);
    assert_eq!(translucent.opacity(0.5), 0.5);

    // Animations off has to reach the palette, which is what the window reads.
    let still = Palette::resolve(
        ThemePref::Dark,
        "system",
        SystemVisuals {
            animations: false,
            ..base
        },
    );
    assert!(!still.animations);

    // Light and dark follow the system when the preference says "system".
    assert!(
        !Palette::resolve(
            ThemePref::System,
            "system",
            SystemVisuals {
                light: true,
                ..base
            }
        )
        .dark
    );
    assert!(
        Palette::resolve(
            ThemePref::System,
            "system",
            SystemVisuals {
                light: false,
                ..base
            }
        )
        .dark
    );

    // A contrast theme outranks everything: flat, opaque, system colours.
    let contrast = Palette::resolve(
        ThemePref::Light,
        "#FF0000",
        SystemVisuals {
            high_contrast: true,
            contrast: Some(ContrastColors {
                window: 0x000000,
                text: 0xFFFFFF,
                gray: 0x808080,
                highlight: 0x00FF00,
                hot: 0xFFFF00,
            }),
            ..base
        },
    );
    assert!(contrast.high_contrast, "the light preference must not win");
    assert!(contrast.force_opaque);
    assert_eq!(contrast.sheen_gloss, 0.0, "no gloss in a contrast theme");
    assert_eq!(contrast.shadow_alpha, 0.0, "no shadow in a contrast theme");
    assert_eq!(
        contrast.panel_top, 0x000000,
        "the system window colour must be used"
    );
    assert_eq!(contrast.text_primary, 0xFFFFFF);
    assert!(contrast.dark, "a black window means a dark contrast theme");
}

/// Motion has to stop entirely when the system says so, not merely run faster.
#[test]
fn animations_disabled_means_no_animation_at_all() {
    use tpmplaner_core::anim::Animations;

    let mut anim = Animations::default();
    anim.enabled = false;
    anim.hover.set(1.0);
    anim.scroll.set(250.0);
    anim.spinning = true;
    anim.restart_reveal();

    // A single tick has to settle everything and report "nothing to do", so
    // the caller switches its timer off instead of running at 60 Hz forever.
    assert!(
        !anim.tick(),
        "a disabled animation must not ask for more frames"
    );
    assert_eq!(anim.hover.value, 1.0);
    assert_eq!(anim.scroll.value, 250.0);
    assert_eq!(
        anim.reveal.value, 1.0,
        "the reveal must be complete, not starting"
    );
    assert_eq!(anim.spinner, 0.0, "the spinner must not turn");

    // With animation on, the same state needs more frames.
    let mut moving = Animations::default();
    moving.hover.set(1.0);
    assert!(
        moving.tick(),
        "an enabled animation must ask for another frame"
    );
    assert!(
        moving.hover.value < 1.0,
        "it must not jump straight to the target"
    );
}
