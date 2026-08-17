<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
<!-- Copyright (C) 2026 Ephemeris contributors -->
<script setup lang="ts">
// Decoration only: no text, no interaction, no state. It exists so the hero
// has something behind it other than the flat page background.
</script>

<template>
  <div class="hero-aura" aria-hidden="true">
    <div class="hero-aura__grid" />
    <div class="hero-aura__blob hero-aura__blob--one" />
    <div class="hero-aura__blob hero-aura__blob--two" />
    <div class="hero-aura__blob hero-aura__blob--three" />
  </div>
</template>

<style scoped>
.hero-aura {
  position: absolute;
  top: calc(var(--vp-nav-height) * -1);
  right: 0;
  left: 0;
  height: 880px;
  overflow: hidden;
  pointer-events: none;
  /* Fades out downwards so the decoration never collides with the first
     paragraph of actual content. */
  -webkit-mask-image: linear-gradient(
    to bottom,
    #000 0%,
    #000 55%,
    transparent 100%
  );
  mask-image: linear-gradient(to bottom, #000 0%, #000 55%, transparent 100%);
}

/* The same grid the day rail is measured against, at a size that reads as
   texture rather than as a table. */
.hero-aura__grid {
  position: absolute;
  inset: 0;
  background-image:
    linear-gradient(to right, var(--tpm-grid-line) 1px, transparent 1px),
    linear-gradient(to bottom, var(--tpm-grid-line) 1px, transparent 1px);
  background-size: 64px 64px;
  -webkit-mask-image: radial-gradient(
    ellipse 80% 60% at 50% 20%,
    #000 0%,
    transparent 75%
  );
  mask-image: radial-gradient(
    ellipse 80% 60% at 50% 20%,
    #000 0%,
    transparent 75%
  );
}

.hero-aura__blob {
  position: absolute;
  border-radius: 50%;
  filter: blur(72px);
  opacity: var(--tpm-aura-opacity);
  will-change: transform;
}

.hero-aura__blob--one {
  top: -180px;
  left: -80px;
  width: 620px;
  height: 620px;
  background: radial-gradient(circle, var(--tpm-brand-400) 0%, transparent 68%);
}

.hero-aura__blob--two {
  top: -120px;
  right: -120px;
  width: 680px;
  height: 680px;
  background: radial-gradient(circle, var(--tpm-brand-700) 0%, transparent 68%);
}

/* The one warm note, the colour the widget uses for what is due next. It sits
   low and to the left, under the buttons: directly beneath the logo it haloed
   the underside of the mark and made it read as sinking towards the cards. */
.hero-aura__blob--three {
  top: 330px;
  left: 6%;
  width: 460px;
  height: 460px;
  opacity: calc(var(--tpm-aura-opacity) * 0.55);
  background: radial-gradient(circle, var(--tpm-accent) 0%, transparent 70%);
}

/* Slow enough to be noticed only if you sit and look for it, and off entirely
   for anyone who has asked for less movement. */
@media (prefers-reduced-motion: no-preference) {
  .hero-aura__blob--one {
    animation: tpm-drift-a 26s ease-in-out infinite alternate;
  }

  .hero-aura__blob--two {
    animation: tpm-drift-b 32s ease-in-out infinite alternate;
  }

  .hero-aura__blob--three {
    animation: tpm-drift-a 38s ease-in-out infinite alternate-reverse;
  }
}

@keyframes tpm-drift-a {
  from {
    transform: translate3d(0, 0, 0) scale(1);
  }
  to {
    transform: translate3d(60px, 40px, 0) scale(1.12);
  }
}

@keyframes tpm-drift-b {
  from {
    transform: translate3d(0, 0, 0) scale(1.08);
  }
  to {
    transform: translate3d(-70px, 50px, 0) scale(1);
  }
}

@media (max-width: 960px) {
  .hero-aura {
    height: 640px;
  }

  .hero-aura__blob {
    filter: blur(56px);
  }
}
</style>
