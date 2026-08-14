// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Ephemeris contributors
import type { Theme } from 'vitepress'
import DefaultTheme from 'vitepress/theme'
import { h } from 'vue'

import HeroAura from './components/HeroAura.vue'
import PlatformSupport from './components/PlatformSupport.vue'
import WidgetPreview from './components/WidgetPreview.vue'
import './style.css'

export default {
  extends: DefaultTheme,

  // The aura sits in the one slot that renders before the hero, so it can be
  // positioned against the home page rather than against the viewport and
  // scroll away with the content it belongs to.
  Layout: () =>
    h(DefaultTheme.Layout, null, {
      'home-hero-before': () => h(HeroAura),
    }),

  enhanceApp({ app }) {
    // Usable from any Markdown page, not just the home page.
    app.component('WidgetPreview', WidgetPreview)
    app.component('PlatformSupport', PlatformSupport)
  },
} satisfies Theme
