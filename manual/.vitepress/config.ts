import { defineConfig } from 'vitepress'
import aiviGrammar from '../../tooling/packages/vscode-aivi/syntaxes/aivi.tmLanguage.json'
import aiviDarkTheme from './theme/aivi-dark-theme.json'
import { nav, sidebar } from './navigation'

function manualBase(): string {
  const configured = process.env.AIVI_MANUAL_BASE?.trim()
  if (!configured) {
    return '/'
  }

  let normalized = configured.startsWith('/') ? configured : `/${configured}`
  if (!normalized.endsWith('/')) {
    normalized = `${normalized}/`
  }
  return normalized
}

export default defineConfig({
  title: 'AIVI',
  description: 'AIVI Language Manual — a reactive, functional, GTK-first language',
  base: manualBase(),

  head: [
    ['link', { rel: 'preconnect', href: 'https://fonts.googleapis.com' }],
    ['link', { rel: 'preconnect', href: 'https://fonts.gstatic.com', crossorigin: '' }],
    ['link', { rel: 'stylesheet', href: 'https://fonts.googleapis.com/css2?family=Fira+Code:wght@300;400;500;600;700&display=swap' }],
  ],

  markdown: {
    languages: [aiviGrammar as any],
    theme: aiviDarkTheme as any,
  },

  themeConfig: {
    logo: null,
    siteTitle: 'AIVI',
    nav,
    sidebar,
    search: {
      provider: 'local',
    },

    socialLinks: [],

    footer: {
      message: 'AIVI Language Manual',
    },
  },
})
