<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
<!-- Copyright (C) 2026 Ephemeris contributors -->
<script setup lang="ts">
// The support table, as tiles rather than prose, so the one platform with a
// real caveat is visible without reading three paragraphs to find it.

type Level = 'full' | 'partial'

const platforms: {
  name: string
  level: Level
  status: string
  detail: string
}[] = [
  {
    name: 'Windows',
    level: 'full',
    status: 'Behind your windows',
    detail: '10 version 1809 or newer. One self-contained binary, no runtime.',
  },
  {
    name: 'macOS',
    level: 'partial',
    status: 'Not notarised',
    detail:
      'Works fully, but Gatekeeper refuses the first launch until you ' +
      'right-click and choose Open.',
  },
  {
    name: 'Linux · Wayland',
    level: 'full',
    status: 'Behind your windows',
    detail:
      'Anywhere wlr-layer-shell exists: KDE, Sway, Hyprland, wlroots ' +
      'compositors.',
  },
  {
    name: 'Linux · X11',
    level: 'full',
    status: 'Behind your windows',
    detail: 'Desktop-level window hints, no taskbar and no Alt-Tab entry.',
  },
  {
    name: 'GNOME',
    level: 'partial',
    status: 'An ordinary window',
    detail:
      'Mutter does not implement wlr-layer-shell and has said it will not, ' +
      'so the widget sits among your windows rather than behind them. It ' +
      'says so in its log rather than pretending.',
  },
]
</script>

<template>
  <div class="grid">
    <div
      v-for="platform in platforms"
      :key="platform.name"
      class="tile"
      :class="`tile--${platform.level}`"
    >
      <div class="tile__head">
        <h3 class="tile__name">{{ platform.name }}</h3>
        <span class="tile__badge">
          <span class="tile__dot" aria-hidden="true" />
          {{ platform.status }}
        </span>
      </div>
      <p class="tile__detail">{{ platform.detail }}</p>
    </div>
  </div>
</template>

<style scoped>
.grid {
  display: grid;
  gap: 14px;
  margin: 26px 0 8px;
  grid-template-columns: repeat(auto-fit, minmax(258px, 1fr));
}

.tile {
  padding: 18px 18px 16px;
  border: 1px solid var(--vp-c-divider);
  border-radius: 14px;
  background: var(--vp-c-bg-soft);
  transition:
    border-color 0.22s ease,
    transform 0.22s ease;
}

.tile:hover {
  border-color: var(--tpm-border-strong);
  transform: translateY(-2px);
}

@media (prefers-reduced-motion: reduce) {
  .tile:hover {
    transform: none;
  }
}

.tile__head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  flex-wrap: wrap;
}

/* Reset the heading: it is a label in a card, not a section of the page, and
   it must not join the outline the sidebar builds. */
.tile__name {
  margin: 0;
  padding: 0;
  border: 0;
  font-size: 15px;
  font-weight: 620;
  letter-spacing: -0.01em;
  line-height: 1.3;
}

.tile__badge {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 3px 9px 3px 7px;
  border-radius: 999px;
  font-size: 11.5px;
  font-weight: 600;
  white-space: nowrap;
}

.tile__dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: currentColor;
}

.tile--full .tile__badge {
  background: var(--vp-c-tip-soft);
  color: var(--vp-c-tip-1);
}

.tile--partial .tile__badge {
  background: var(--vp-c-warning-soft);
  color: var(--vp-c-warning-1);
}

.tile__detail {
  margin: 10px 0 0;
  color: var(--vp-c-text-2);
  font-size: 14px;
  line-height: 1.6;
}
</style>
