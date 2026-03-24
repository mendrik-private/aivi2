import { defineConfig } from 'vitepress'
import type { LanguageInput, ThemeInput } from 'shiki'

// Vite handles JSON imports natively; no assert/createRequire needed
import aiviGrammar from '../../tooling/packages/vscode-aivi/syntaxes/aivi.tmLanguage.json'
import aiviDark    from '../../tooling/packages/vscode-aivi/themes/aivi-dark-color-theme.json'
import aiviLight   from './theme/aivi-light-color-theme.json'

export default defineConfig({
  title: 'AIVI',
  description: 'A reactive, functional language for Linux desktop apps',
  base: '/aivi2/',

  head: [['link', { rel: 'icon', type: 'image/png', href: '/aivi2/aivi-a.png' }]],

  themeConfig: {
    nav: [
      { text: 'Introduction',     link: '/introduction' },
      { text: 'Language Tour',    link: '/tour/' },
      { text: 'The AIVI Way',     link: '/aivi-way/' },
      { text: 'Standard Library', link: '/stdlib/' },
      { text: 'Playground',       link: '/playground/' },
    ],

    sidebar: [
      {
        text: 'Getting Started',
        items: [
          { text: 'Introduction', link: '/introduction' },
        ],
      },
      {
        text: 'Language Tour',
        items: [
          { text: 'Overview',              link: '/tour/' },
          { text: '01 · Values & Types',   link: '/tour/01-values-types' },
          { text: '02 · Functions',        link: '/tour/02-functions' },
          { text: '03 · Pipes',            link: '/tour/03-pipes' },
          { text: '04 · Pattern Matching', link: '/tour/04-pattern-matching' },
          { text: '05 · Signals',          link: '/tour/05-signals' },
          { text: '06 · Sources',          link: '/tour/06-sources' },
          { text: '07 · Markup',           link: '/tour/07-markup' },
          { text: '08 · Type Classes',     link: '/tour/08-typeclasses' },
          { text: '09 · Domains',          link: '/tour/09-domains' },
        ],
      },
      {
        text: 'The AIVI Way',
        items: [
          { text: 'Overview',       link: '/aivi-way/' },
          { text: 'Async Data',     link: '/aivi-way/async-data' },
          { text: 'Forms',          link: '/aivi-way/forms' },
          { text: 'State',          link: '/aivi-way/state' },
          { text: 'List Rendering', link: '/aivi-way/list-rendering' },
          { text: 'Error Handling', link: '/aivi-way/error-handling' },
        ],
      },
      {
        text: 'Reference',
        items: [
          { text: 'Standard Library', link: '/stdlib/' },
          { text: 'Playground',       link: '/playground/' },
        ],
      },
    ],

    socialLinks: [
      { icon: 'github', link: 'https://github.com/mendrik/aivi2' },
    ],

    footer: {
      message: 'AIVI — a reactive language for native Linux desktop apps.',
    },
  },

  markdown: {
    // The grammar declares name "AIVI" (uppercase); add lowercase alias so ```aivi fences resolve.
    languages: [{ ...aiviGrammar, aliases: ['aivi', ...(aiviGrammar.aliases ?? [])] } as unknown as LanguageInput],
    theme: {
      dark:  aiviDark  as unknown as ThemeInput,
      light: aiviLight as unknown as ThemeInput,
    },
  },
})
