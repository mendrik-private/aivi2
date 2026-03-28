import type { DefaultTheme } from 'vitepress'

type DocItem = {
  text: string
  link: string
}

type DocGroup = {
  text: string
  collapsed?: boolean
  items: DocItem[]
}

function links(items: DocItem[]): DefaultTheme.SidebarItem[] {
  return items.map(item => ({ text: item.text, link: item.link }))
}

function group(section: DocGroup): DefaultTheme.SidebarItem {
  return {
    text: section.text,
    collapsed: section.collapsed,
    items: links(section.items),
  }
}

const guideSections: DocGroup[] = [
  {
    text: 'Introduction',
    collapsed: false,
    items: [
      { text: 'What is AIVI?', link: '/guide/getting-started' },
      { text: 'Values & Functions', link: '/guide/values-and-functions' },
      { text: 'Types', link: '/guide/types' },
    ],
  },
  {
    text: 'Core Language',
    collapsed: false,
    items: [
      { text: 'Pattern Matching', link: '/guide/pattern-matching' },
      { text: 'Pipes & Operators', link: '/guide/pipes' },
      { text: 'Signals', link: '/guide/signals' },
      { text: 'Sources', link: '/guide/sources' },
    ],
  },
  {
    text: 'Advanced',
    collapsed: false,
    items: [
      { text: 'Markup & UI', link: '/guide/markup' },
      { text: 'Domains', link: '/guide/domains' },
      { text: 'Classes', link: '/guide/classes' },
      { text: 'Modules', link: '/guide/modules' },
    ],
  },
]

const stdlibSections: DocGroup[] = [
  {
    text: 'Core Modules',
    collapsed: true,
    items: [
      { text: 'Function Combinators', link: '/stdlib/fn' },
      { text: 'Either Values', link: '/stdlib/either' },
      { text: 'Floating-Point Numbers', link: '/stdlib/float' },
      { text: 'Dictionaries', link: '/stdlib/dict' },
      { text: 'Ranges', link: '/stdlib/range' },
      { text: 'Byte Buffers', link: '/stdlib/bytes' },
      { text: 'Sets', link: '/stdlib/set' },
    ],
  },
  {
    text: 'Foundation Modules',
    collapsed: true,
    items: [
      { text: 'Boolean Logic', link: '/stdlib/bool' },
      { text: 'Lists', link: '/stdlib/list' },
      { text: 'Math', link: '/stdlib/math' },
      { text: 'Non-Empty Lists', link: '/stdlib/nonEmpty' },
      { text: 'Optional Values', link: '/stdlib/option' },
      { text: 'Ordering & Comparison', link: '/stdlib/order' },
      { text: 'Pairs', link: '/stdlib/pair' },
      { text: 'Result Values', link: '/stdlib/result' },
      { text: 'Text Processing', link: '/stdlib/text' },
      { text: 'Validation', link: '/stdlib/validation' },
      { text: 'Prelude', link: '/stdlib/prelude' },
    ],
  },
  {
    text: 'I/O & Platform',
    collapsed: true,
    items: [
      { text: 'File System', link: '/stdlib/fs' },
      { text: 'Paths', link: '/stdlib/path' },
    ],
  },
  {
    text: 'Data',
    collapsed: true,
    items: [
      { text: 'JSON', link: '/stdlib/json' },
    ],
  },
  {
    text: 'Desktop',
    collapsed: true,
    items: [
      { text: 'XDG Directories', link: '/stdlib/xdg' },
    ],
  },
  {
    text: 'App Framework',
    collapsed: true,
    items: [
      { text: 'Application Framework', link: '/stdlib/app' },
    ],
  },
]

export const nav: DefaultTheme.NavItem[] = [
  { text: 'Guide', link: '/guide/getting-started' },
  { text: 'Standard Library', link: '/stdlib/' },
]

export const sidebar: DefaultTheme.SidebarMulti = {
  '/guide/': [
    ...guideSections.map(group),
    group({
      text: 'Reference',
      collapsed: false,
      items: [{ text: 'Standard Library Overview', link: '/stdlib/' }],
    }),
  ],
  '/stdlib/': [
    group({
      text: 'Standard Library',
      collapsed: false,
      items: [{ text: 'Overview', link: '/stdlib/' }],
    }),
    ...stdlibSections.map(group),
  ],
  '/': [
    group({
      text: 'Manual',
      collapsed: false,
      items: [
        { text: 'Guide', link: '/guide/getting-started' },
        { text: 'Standard Library Overview', link: '/stdlib/' },
      ],
    }),
  ],
}
