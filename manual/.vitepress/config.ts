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
          {
            text: 'Core Modules',
            items: [
              { text: 'aivi.core.fn', link: '/stdlib/fn' },
              { text: 'aivi.core.either', link: '/stdlib/either' },
              { text: 'aivi.core.float', link: '/stdlib/float' },
              { text: 'aivi.core.dict', link: '/stdlib/dict' },
              { text: 'aivi.core.range', link: '/stdlib/range' },
            ],
          },
          {
            text: 'Foundation Modules',
            items: [
              { text: 'aivi.bool', link: '/stdlib/bool' },
              { text: 'aivi.list', link: '/stdlib/list' },
              { text: 'aivi.math', link: '/stdlib/math' },
              { text: 'aivi.nonEmpty', link: '/stdlib/nonEmpty' },
              { text: 'aivi.option', link: '/stdlib/option' },
              { text: 'aivi.order', link: '/stdlib/order' },
              { text: 'aivi.pair', link: '/stdlib/pair' },
              { text: 'aivi.result', link: '/stdlib/result' },
              { text: 'aivi.text', link: '/stdlib/text' },
              { text: 'aivi.validation', link: '/stdlib/validation' },
              { text: 'aivi.prelude', link: '/stdlib/prelude' },
            ],
          },
          {
            text: 'I/O & Platform',
            items: [
              { text: 'aivi.fs', link: '/stdlib/fs' },
              { text: 'aivi.path', link: '/stdlib/path' },
            ],
          },
        ],
      },
    ],

    socialLinks: [],

    footer: {
      message: 'AIVI Language Manual',
    },
  },
})
