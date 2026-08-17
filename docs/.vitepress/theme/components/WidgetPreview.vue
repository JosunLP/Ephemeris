<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
<!-- Copyright (C) 2026 Ephemeris contributors -->
<script setup lang="ts">
// A drawing of the widget, not a screenshot: it stays in step with the
// documentation's own theme, sharpens on any display, and costs one HTTP
// request less than a picture of the same thing.
//
// Every value is fixed. Nothing here reads the clock, because a value that
// differs between the server render and the browser render tears the page.

type Source = 'google' | 'microsoft' | 'caldav'

const upcoming: {
  time: string
  title: string
  source: Source
  note?: string
}[] = [
  { time: '15:00', title: 'Sprint planning', source: 'microsoft' },
  {
    time: '16:30',
    title: '1:1 with Ana',
    source: 'google',
    note: 'overlaps 16:30 Vendor call',
  },
  { time: '18:15', title: 'Pick up parcel', source: 'caldav' },
]

const due: { title: string; when: string; late?: boolean }[] = [
  { title: 'Send the release notes', when: 'today', late: true },
  { title: 'Renew the domain', when: 'tomorrow' },
]

// One segment per event, placed along a 07:00–21:00 rail. `start` and `span`
// are percentages of that window, worked out once by hand.
const segments = [
  { start: 8, span: 12, tone: 'rest' },
  { start: 26, span: 9, tone: 'rest' },
  { start: 46, span: 8, tone: 'now' },
  { start: 57, span: 11, tone: 'rest' },
  { start: 68, span: 7, tone: 'rest' },
  { start: 80, span: 6, tone: 'rest' },
]
</script>

<template>
  <div class="preview">
    <div class="preview__stage">
      <div class="panel" role="img" aria-label="The Ephemeris widget showing
        Thursday 14 August: a design review running now with 31 minutes left,
        three later events and two due tasks.">
        <div class="panel__head">
          <div class="panel__day">
            <span class="panel__weekday">Thursday</span>
            <span class="panel__date">14 August</span>
          </div>
          <span class="panel__clock">13:42</span>
        </div>

        <div class="rail" aria-hidden="true">
          <span
            v-for="(segment, index) in segments"
            :key="index"
            class="rail__segment"
            :class="`rail__segment--${segment.tone}`"
            :style="{ left: `${segment.start}%`, width: `${segment.span}%` }"
          />
          <span class="rail__now" :style="{ left: '48%' }" />
        </div>

        <div class="now">
          <div class="now__label">
            <span class="now__pulse" aria-hidden="true" />
            Now
          </div>
          <div class="now__title">Design review</div>
          <div class="now__meta">
            <span>13:30 – 14:15</span>
            <span class="now__countdown">31 min left</span>
          </div>
        </div>

        <ul class="list">
          <li v-for="item in upcoming" :key="item.title" class="row">
            <span class="row__time">{{ item.time }}</span>
            <span class="row__dot" :class="`row__dot--${item.source}`" />
            <span class="row__body">
              <span class="row__title">{{ item.title }}</span>
              <span v-if="item.note" class="row__note">{{ item.note }}</span>
            </span>
          </li>
        </ul>

        <div class="panel__rule" aria-hidden="true" />

        <ul class="list">
          <li v-for="task in due" :key="task.title" class="row row--task">
            <span class="row__box" aria-hidden="true" />
            <span class="row__body">
              <span class="row__title">{{ task.title }}</span>
            </span>
            <span class="row__when" :class="{ 'row__when--late': task.late }">
              {{ task.when }}
            </span>
          </li>
        </ul>

        <div class="panel__foot">
          <span class="panel__status">Synced 2 min ago · 3 accounts</span>
        </div>
      </div>
    </div>

    <p class="preview__caption">
      Drawn, not photographed — the widget picks up your accent colour, your
      contrast setting and your locale, so no single screenshot is what you
      would see.
    </p>
  </div>
</template>

<style scoped>
.preview {
  margin: 32px 0 8px;
}

/* The wallpaper the panel is shown against. Without something behind it, a
   frameless panel just looks like an unstyled div. */
.preview__stage {
  display: flex;
  justify-content: center;
  padding: 44px 20px 52px;
  border: 1px solid var(--vp-c-divider);
  border-radius: 20px;
  background:
    radial-gradient(
      ellipse 70% 90% at 22% 0%,
      var(--tpm-stage-glow-a) 0%,
      transparent 62%
    ),
    radial-gradient(
      ellipse 70% 90% at 88% 100%,
      var(--tpm-stage-glow-b) 0%,
      transparent 62%
    ),
    var(--tpm-stage-bg);
}

.panel {
  width: 100%;
  max-width: 380px;
  padding: 18px 18px 14px;
  border: 1px solid var(--tpm-panel-border);
  border-radius: 16px;
  background: var(--tpm-panel-bg);
  box-shadow: var(--tpm-panel-shadow);
  backdrop-filter: blur(14px);
  color: var(--tpm-panel-text);
  font-size: 13px;
  line-height: 1.35;
  /* Tabular figures, so the times in the list line up in a column the way
     they do in the widget itself. */
  font-variant-numeric: tabular-nums;
}

.panel__head {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 12px;
}

.panel__day {
  display: flex;
  align-items: baseline;
  gap: 8px;
  min-width: 0;
}

.panel__weekday {
  font-size: 19px;
  font-weight: 650;
  letter-spacing: -0.01em;
}

.panel__date {
  color: var(--tpm-panel-text-dim);
}

.panel__clock {
  font-size: 19px;
  font-weight: 650;
  letter-spacing: -0.02em;
}

.rail {
  position: relative;
  height: 8px;
  margin: 14px 0 16px;
  border-radius: 4px;
  background: var(--tpm-rail-track);
}

.rail__segment {
  position: absolute;
  top: 0;
  height: 100%;
  border-radius: 4px;
  background: var(--tpm-rail-rest);
}

.rail__segment--now {
  background: linear-gradient(
    90deg,
    var(--tpm-brand-400),
    var(--tpm-brand-500)
  );
}

/* The marker for right now, drawn through the rail rather than on it. */
.rail__now {
  position: absolute;
  top: -4px;
  width: 2px;
  height: 16px;
  border-radius: 1px;
  background: var(--tpm-accent);
  box-shadow: 0 0 8px var(--tpm-accent);
}

.now {
  padding: 12px 14px;
  border: 1px solid var(--tpm-now-border);
  border-radius: 12px;
  background: var(--tpm-now-bg);
}

.now__label {
  display: flex;
  align-items: center;
  gap: 6px;
  color: var(--tpm-brand-400);
  font-size: 10px;
  font-weight: 700;
  letter-spacing: 0.09em;
  text-transform: uppercase;
}

.now__pulse {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--tpm-accent);
}

@media (prefers-reduced-motion: no-preference) {
  .now__pulse {
    animation: tpm-pulse 2.6s ease-in-out infinite;
  }
}

@keyframes tpm-pulse {
  0%,
  100% {
    opacity: 1;
    box-shadow: 0 0 0 0 var(--tpm-accent-soft);
  }
  50% {
    opacity: 0.75;
    box-shadow: 0 0 0 4px transparent;
  }
}

.now__title {
  margin-top: 5px;
  font-size: 15px;
  font-weight: 620;
}

.now__meta {
  display: flex;
  justify-content: space-between;
  gap: 10px;
  margin-top: 3px;
  color: var(--tpm-panel-text-dim);
  font-size: 12px;
}

.now__countdown {
  color: var(--tpm-accent);
  font-weight: 600;
}

.list {
  margin: 12px 0 0;
  padding: 0;
  list-style: none;
}

.row {
  display: flex;
  align-items: baseline;
  gap: 9px;
  padding: 6px 2px;
}

.row__time {
  width: 38px;
  flex: none;
  color: var(--tpm-panel-text-dim);
  font-size: 12px;
}

.row__dot {
  width: 7px;
  height: 7px;
  flex: none;
  border-radius: 50%;
  transform: translateY(-1px);
}

/* One colour per back end, which is how you tell at a glance that all three
   are actually reaching the widget. */
.row__dot--google {
  background: #4b87f5;
}

.row__dot--microsoft {
  background: #8b7cf6;
}

.row__dot--caldav {
  background: #3fbf9a;
}

.row__body {
  min-width: 0;
  flex: 1;
}

.row__title {
  display: block;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.row__note {
  display: block;
  margin-top: 1px;
  color: var(--tpm-accent);
  font-size: 11px;
}

.row--task .row__title {
  color: var(--tpm-panel-text);
}

.row__box {
  width: 12px;
  height: 12px;
  flex: none;
  border: 1.5px solid var(--tpm-panel-text-dim);
  border-radius: 3.5px;
  transform: translateY(1px);
}

.row__when {
  flex: none;
  color: var(--tpm-panel-text-dim);
  font-size: 11px;
}

.row__when--late {
  color: var(--tpm-accent);
  font-weight: 600;
}

.panel__rule {
  height: 1px;
  margin: 12px 0 2px;
  background: var(--tpm-panel-border);
}

.panel__foot {
  margin-top: 10px;
  padding-top: 9px;
  border-top: 1px solid var(--tpm-panel-border);
}

.panel__status {
  color: var(--tpm-panel-text-dim);
  font-size: 11px;
}

.preview__caption {
  margin: 14px auto 0;
  max-width: 46em;
  color: var(--vp-c-text-2);
  font-size: 14px;
  line-height: 1.6;
  text-align: center;
}

@media (max-width: 640px) {
  .preview__stage {
    padding: 24px 12px 30px;
    border-radius: 16px;
  }
}
</style>
