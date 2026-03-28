import { defineConfig } from 'vitepress'

export default defineConfig({
  title: 'AIVI',
  description: 'AIVI Language Manual — a reactive, functional, GTK-first language',
  base: '/',

  themeConfig: {
    logo: null,
    siteTitle: 'AIVI',
    nav: [
      { text: 'Guide', link: '/guide/getting-started' },
      { text: 'Standard Library', link: '/stdlib/' },
    ],

    sidebar: [
      {
        text: 'Introduction',
        items: [
          { text: 'What is AIVI?', link: '/guide/getting-started' },
          { text: 'Values & Functions', link: '/guide/values-and-functions' },
          { text: 'Types', link: '/guide/types' },
        ],
      },
      {
        text: 'Core Language',
        items: [
          { text: 'Pattern Matching', link: '/guide/pattern-matching' },
          { text: 'Pipes & Operators', link: '/guide/pipes' },
          { text: 'Signals', link: '/guide/signals' },
          { text: 'Sources', link: '/guide/sources' },
        ],
      },
      {
        text: 'Advanced',
        items: [
          { text: 'Markup & UI', link: '/guide/markup' },
          { text: 'Domains', link: '/guide/domains' },
          { text: 'Classes', link: '/guide/classes' },
          { text: 'Modules', link: '/guide/modules' },
        ],
      },
      {
        text: 'Reference',
        items: [
          { text: 'Standard Library', link: '/stdlib/' },
        ],
      },
    ],

    socialLinks: [],

    footer: {
      message: 'AIVI Language Manual',
    },
  },
})
