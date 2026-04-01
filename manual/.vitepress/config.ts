import { fileURLToPath } from 'node:url'
import { defineConfig } from 'vitepress'
import aiviGrammar from '../../tooling/packages/vscode-aivi/syntaxes/aivi.tmLanguage.json'
import aiviDarkTheme from './theme/aivi-dark-theme.json'
import { nav, sidebar } from './navigation'

const repoRoot = fileURLToPath(new URL('../../', import.meta.url))

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

  markdown: {
    languages: [aiviGrammar as any],
    theme: aiviDarkTheme as any,
    config(md) {
      md.core.ruler.push('pipe_operator', (state) => {
        for (const blockToken of state.tokens) {
          if (blockToken.type !== 'inline' || !blockToken.children) continue
          const next: typeof blockToken.children = []
          for (const tok of blockToken.children) {
            if (tok.type !== 'text' || !tok.content.includes('|>')) {
              next.push(tok)
              continue
            }
            const parts = tok.content.split('|>')
            for (let i = 0; i < parts.length; i++) {
              if (parts[i].length > 0) {
                const t = new state.Token('text', '', 0)
                t.content = parts[i]
                next.push(t)
              }
              if (i < parts.length - 1) {
                const span = new state.Token('html_inline', '', 0)
                span.content = '<span class="pipe-op">|></span>'
                next.push(span)
              }
            }
          }
          blockToken.children = next
        }
      })
    },
  },

  vite: {
    server: {
      fs: {
        allow: [repoRoot],
      },
    },
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
