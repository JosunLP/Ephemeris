// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Ephemeris contributors
import { defineConfig } from 'vitepress'

// The site is served from https://josunlp.github.io/Ephemeris/, so every
// absolute path needs the repository name as a prefix.
const base = '/Ephemeris/'

export default defineConfig({
  base,
  lang: 'en-GB',
  title: 'Ephemeris',
  description:
    "Today's calendar events and due tasks on your desktop. Google, Outlook, Teams and CalDAV, side by side.",
  cleanUrls: true,
  lastUpdated: true,

  markdown: {
    // Code blocks are dark on both themes, the way a terminal is, so the
    // theme layer can give them one treatment rather than two. Both entries
    // have to be the dark theme: the light one would put dark tokens on the
    // dark background the stylesheet sets.
    theme: { light: 'github-dark', dark: 'github-dark' },
  },

  // Nothing here goes through `withBase`, unlike `themeConfig.logo` and the
  // home page's hero image, so every path is written out with the prefix.
  head: [
    ['link', { rel: 'icon', type: 'image/svg+xml', href: `${base}logo.svg` }],
    [
      'link',
      {
        rel: 'icon',
        type: 'image/png',
        sizes: '32x32',
        href: `${base}favicon-32.png`,
      },
    ],
    [
      'link',
      {
        rel: 'icon',
        type: 'image/png',
        sizes: '16x16',
        href: `${base}favicon-16.png`,
      },
    ],
    ['link', { rel: 'apple-touch-icon', href: `${base}apple-touch-icon.png` }],
    ['meta', { name: 'theme-color', content: '#5AAEFF' }],
    ['meta', { property: 'og:type', content: 'website' }],
    ['meta', { property: 'og:title', content: 'Ephemeris' }],
    [
      'meta',
      {
        property: 'og:description',
        content: "Today's calendar events and due tasks on your desktop.",
      },
    ],
    // Link previews need somewhere they can actually fetch it from.
    [
      'meta',
      {
        property: 'og:image',
        content: 'https://josunlp.github.io/Ephemeris/logo.png',
      },
    ],
  ],

  themeConfig: {
    logo: { src: '/logo.svg', alt: '' },

    nav: [
      { text: 'Guide', link: '/guide/getting-started' },
      { text: 'Configuration', link: '/guide/configuration' },
      { text: 'Development', link: '/development/architecture' },
      {
        text: 'Download',
        link: 'https://github.com/JosunLP/Ephemeris/releases/latest',
      },
    ],

    sidebar: [
      {
        text: 'Guide',
        items: [
          { text: 'Getting started', link: '/guide/getting-started' },
          { text: 'Connecting accounts', link: '/guide/accounts' },
          { text: 'Using the widget', link: '/guide/using' },
          { text: 'Configuration', link: '/guide/configuration' },
          { text: 'Updating and removing', link: '/guide/updating' },
          { text: 'Troubleshooting', link: '/guide/troubleshooting' },
        ],
      },
      {
        text: 'Development',
        items: [
          { text: 'Architecture', link: '/development/architecture' },
          { text: 'Porting', link: '/development/porting' },
          { text: 'Contributing', link: '/development/contributing' },
        ],
      },
    ],

    socialLinks: [
      { icon: 'github', link: 'https://github.com/JosunLP/Ephemeris' },
    ],

    editLink: {
      pattern: 'https://github.com/JosunLP/Ephemeris/edit/main/docs/:path',
      text: 'Suggest a change to this page',
    },

    search: { provider: 'local' },

    footer: {
      message:
        'Released under the GNU General Public License, version 3 or later.',
      copyright: 'Copyright © 2026 Ephemeris contributors',
    },
  },
})
