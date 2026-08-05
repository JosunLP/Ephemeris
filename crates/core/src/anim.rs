// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
//! Animationszustand.
//!
//! Bewusst ohne Render-Schleife: der Fenster-Code startet einen 16-ms-Timer
//! *nur* solange [`Animations::tick`] meldet, dass noch Bewegung stattfindet,
//! und faellt danach auf den Minutentakt zurueck. Ein ruhendes Widget kostet
//! damit weiterhin null CPU-Zeit.

use std::time::Instant;

/// Exponentielle Annaeherung an einen Zielwert.
///
/// Die Form `1 - e^(-rate·dt)` statt eines festen Schritts pro Frame macht die
/// Bewegung unabhaengig von der tatsaechlichen Bildrate — bei einem verpassten
/// Frame springt der Wert entsprechend weiter, statt die Animation zu dehnen.
#[derive(Debug, Clone, Copy)]
pub struct Spring {
    pub value: f32,
    pub target: f32,
    /// Groesser = schneller. 10 ≈ knackig, 4 ≈ weich.
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

    /// Ohne Animation direkt setzen (z. B. beim ersten Frame).
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
    /// 0 → 1 beim Eintreffen neuer Daten: Inhalt blendet auf und schiebt sich
    /// leicht nach oben.
    pub reveal: Spring,
    /// Geglaetteter Scroll-Offset.
    pub scroll: Spring,
    /// Deckkraft der Hover-Hinterlegung.
    pub hover: Spring,
    /// Deckkraft des Scrollbalkens; blendet nach dem Scrollen wieder aus.
    pub scrollbar: Spring,
    /// Drehwinkel des Aktualisieren-Symbols in Radiant.
    pub spinner: f32,
    pub spinning: bool,

    /// Folgt "Animationseffekte in Windows anzeigen". Ist der Schalter aus,
    /// springt jeder Wert sofort ans Ziel — Bewegung kann bei vestibulaeren
    /// Stoerungen Beschwerden ausloesen, und Windows fragt genau deshalb.
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
    /// Neue Daten: von unten einblenden.
    pub fn restart_reveal(&mut self) {
        if !self.enabled {
            self.reveal.jump(1.0);
            return;
        }
        self.reveal.value = 0.0;
        self.reveal.target = 1.0;
    }

    /// Rechnet alle Werte fort und meldet, ob weiter animiert werden muss.
    pub fn tick(&mut self) -> bool {
        if !self.enabled {
            // Alles sofort auf den Zielwert; der Aufrufer schaltet den Timer
            // daraufhin ab und zeichnet genau einmal.
            self.reveal.jump(self.reveal.target);
            self.scroll.jump(self.scroll.target);
            self.hover.jump(self.hover.target);
            self.scrollbar.jump(self.scrollbar.target);
            self.spinner = 0.0;
            self.last = None;
            return false;
        }

        let now = Instant::now();
        // Beim ersten Aufruf und nach langen Pausen (Standby) keinen riesigen
        // Zeitschritt zulassen, sonst springt alles auf einen Schlag ans Ziel.
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
            // Nicht mitten in der Drehung stehenbleiben, sondern zur naechsten
            // vollen Umdrehung auslaufen.
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
