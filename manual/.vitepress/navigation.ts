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
    text: 'Getting Started',
    collapsed: false,
    items: [
      { text: 'Why AIVI?', link: '/guide/why-aivi' },
      { text: 'What is AIVI?', link: '/guide/getting-started' },
      { text: 'Your First App', link: '/guide/your-first-app' },
    ],
  },
  {
    text: 'Story 1 — Functional Programming',
    collapsed: false,
    items: [
      { text: 'Thinking in AIVI', link: '/guide/thinking-in-aivi' },
      { text: 'Values & Functions', link: '/guide/values-and-functions' },
      { text: 'Types', link: '/guide/types' },
    ],
  },
  {
    text: 'Story 2 — Pipe Algebra',
    collapsed: false,
    items: [
      { text: 'Pipes & Operators', link: '/guide/pipes' },
      { text: 'Pattern Matching', link: '/guide/pattern-matching' },
      { text: 'Record Patterns', link: '/guide/record-patterns' },
      { text: 'Predicates & Selectors', link: '/guide/predicates' },
    ],
  },
  {
    text: 'Story 3 — Domains',
    collapsed: false,
    items: [
      { text: 'Domains', link: '/guide/domains' },
    ],
  },
  {
    text: 'Story 4 — Signals & Reactivity',
    collapsed: false,
    items: [
      { text: 'Signals', link: '/guide/signals' },
      { text: 'Sources', link: '/guide/sources' },
    ],
  },
  {
    text: 'Story 5 — GTK & Markup',
    collapsed: false,
    items: [
      { text: 'Markup & UI', link: '/guide/markup' },
    ],
  },
  {
    text: 'External Integrations',
    collapsed: false,
    items: [
      { text: 'Integration Patterns', link: '/guide/integrations' },
      { text: 'Source Catalog', link: '/guide/source-catalog' },
    ],
  },
  {
    text: 'Abstractions',
    collapsed: true,
    items: [
      { text: 'Classes', link: '/guide/classes' },
      { text: 'Typeclasses & HKTs', link: '/guide/typeclasses' },
      { text: 'Modules', link: '/guide/modules' },
    ],
  },
  {
    text: 'Examples',
    collapsed: false,
    items: [
      { text: 'Building Snake', link: '/guide/building-snake' },
    ],
  },
  {
    text: 'Reference',
    collapsed: true,
    items: [
      { text: 'Surface Feature Matrix', link: '/guide/surface-feature-matrix' },
    ],
  },
]

const stdlibSections: DocGroup[] = [
  {
    text: 'Core Values & Collections',
    collapsed: true,
    items: [
      { text: 'Async Tracker', link: '/stdlib/async' },
      { text: 'Boolean Logic', link: '/stdlib/bool' },
      { text: 'Optional Values', link: '/stdlib/option' },
      { text: 'Result Values', link: '/stdlib/result' },
      { text: 'Validation', link: '/stdlib/validation' },
      { text: 'Either Values', link: '/stdlib/either' },
      { text: 'Lists', link: '/stdlib/list' },
      { text: 'Matrices', link: '/stdlib/matrix' },
      { text: 'Non-Empty Lists', link: '/stdlib/nonEmpty' },
      { text: 'Pairs', link: '/stdlib/pair' },
      { text: 'Ordering & Comparison', link: '/stdlib/order' },
      { text: 'Dictionaries', link: '/stdlib/dict' },
      { text: 'Sets', link: '/stdlib/set' },
      { text: 'Ranges', link: '/stdlib/range' },
      { text: 'Function Helpers', link: '/stdlib/fn' },
    ],
  },
  {
    text: 'Numbers, Text & Data',
    collapsed: true,
    items: [
      { text: 'Arithmetic Intrinsics', link: '/stdlib/arithmetic' },
      { text: 'Bitwise Intrinsics', link: '/stdlib/bits' },
      { text: 'Math', link: '/stdlib/math' },
      { text: 'Floating-Point Numbers', link: '/stdlib/float' },
      { text: 'Big Integers', link: '/stdlib/bigint' },
      { text: 'JSON', link: '/stdlib/json' },
      { text: 'Text Processing', link: '/stdlib/text' },
      { text: 'Regular Expressions', link: '/stdlib/regex' },
      { text: 'Byte Buffers', link: '/stdlib/bytes' },
    ],
  },
  {
    text: 'Time, Randomness & Scheduling',
    collapsed: true,
    items: [
      { text: 'Dates & Calendar', link: '/stdlib/date' },
      { text: 'Durations', link: '/stdlib/duration' },
      { text: 'Time', link: '/stdlib/time' },
      { text: 'Timers', link: '/stdlib/timer' },
      { text: 'Randomness', link: '/stdlib/random' },
    ],
  },
  {
    text: 'Files, Environment & Processes',
    collapsed: true,
    items: [
      { text: 'File System', link: '/stdlib/fs' },
      { text: 'Paths', link: '/stdlib/path' },
      { text: 'Environment Variables', link: '/stdlib/env' },
      { text: 'Standard I/O', link: '/stdlib/stdio' },
      { text: 'Logging', link: '/stdlib/log' },
      { text: 'Processes', link: '/stdlib/process' },
    ],
  },
  {
    text: 'Network & Services',
    collapsed: true,
    items: [
      { text: 'URLs', link: '/stdlib/url' },
      { text: 'HTTP', link: '/stdlib/http' },
      { text: 'API Vocabulary', link: '/stdlib/api' },
      { text: 'Authentication', link: '/stdlib/auth' },
      { text: 'Databases', link: '/stdlib/db' },
      { text: 'IMAP', link: '/stdlib/imap' },
      { text: 'SMTP', link: '/stdlib/smtp' },
    ],
  },
  {
    text: 'Desktop, UI & GNOME',
    collapsed: true,
    items: [
      { text: 'Application Framework', link: '/stdlib/app' },
      { text: 'Application Lifecycle', link: '/stdlib/lifecycle' },
      { text: 'XDG Directories', link: '/stdlib/xdg' },
      { text: 'Portals', link: '/stdlib/portal' },
      { text: 'D-Bus', link: '/stdlib/dbus' },
      { text: 'GNOME Settings', link: '/stdlib/settings' },
      { text: 'Online Accounts', link: '/stdlib/onlineAccounts' },
      { text: 'Desktop Notifications', link: '/stdlib/notifications' },
      { text: 'Clipboard', link: '/stdlib/clipboard' },
      { text: 'Colors', link: '/stdlib/color' },
      { text: 'Images', link: '/stdlib/image' },
      { text: 'GResources', link: '/stdlib/gresource' },
      { text: 'Internationalization', link: '/stdlib/i18n' },
    ],
  },
]

export const nav: DefaultTheme.NavItem[] = [
  { text: 'Guide', link: '/guide/why-aivi' },
  { text: 'Standard Library', link: '/stdlib/' },
]

export const sidebar: DefaultTheme.SidebarMulti = {
  '/guide/': [
    ...guideSections.map(group),
    group({
      text: 'Standard Library',
      collapsed: true,
      items: [{ text: 'Overview', link: '/stdlib/' }],
    }),
  ],
  '/stdlib/': [
    group({
      text: 'Start Here',
      collapsed: false,
      items: [
        { text: 'Overview', link: '/stdlib/' },
        { text: 'Prelude', link: '/stdlib/prelude' },
        { text: 'Default Values', link: '/stdlib/defaults' },
      ],
    }),
    ...stdlibSections.map(group),
  ],
  '/': [
    group({
      text: 'Manual',
      collapsed: false,
      items: [
        { text: 'Guide', link: '/guide/why-aivi' },
        { text: 'Standard Library', link: '/stdlib/' },
      ],
    }),
  ],
}
