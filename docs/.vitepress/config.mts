// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 TPMPlaner contributors
import { defineConfig } from 'vitepress'

// The site is served from https://josunlp.github.io/TPMPlaner/, so every
// absolute path needs the repository name as a prefix.
const base = '/TPMPlaner/'

export default defineConfig({
  base,
  lang: 'en-GB',
  title: 'TPMPlaner',
  description:
    "Today's calendar events and due tasks on your desktop. Google, Outlook, Teams and CalDAV, side by side.",
  cleanUrls: true,
  lastUpdated: true,

  head: [
    ['meta', { name: 'theme-color', content: '#5AAEFF' }],
    ['meta', { property: 'og:type', content: 'website' }],
    ['meta', { property: 'og:title', content: 'TPMPlaner' }],
    [
      'meta',
      {
        property: 'og:description',
        content: "Today's calendar events and due tasks on your desktop.",
      },
    ],
  ],

  themeConfig: {
    nav: [
      { text: 'Guide', link: '/guide/getting-started' },
      { text: 'Configuration', link: '/guide/configuration' },
      { text: 'Development', link: '/development/architecture' },
      {
        text: 'Download',
        link: 'https://github.com/JosunLP/TPMPlaner/releases/latest',
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
          { text: 'Contributing', link: '/development/contributing' },
        ],
      },
    ],

    socialLinks: [
      { icon: 'github', link: 'https://github.com/JosunLP/TPMPlaner' },
    ],

    editLink: {
      pattern: 'https://github.com/JosunLP/TPMPlaner/edit/main/docs/:path',
      text: 'Suggest a change to this page',
    },

    search: { provider: 'local' },

    footer: {
      message:
        'Released under the GNU General Public License, version 3 or later.',
      copyright: 'Copyright © 2026 TPMPlaner contributors',
    },
  },
})
