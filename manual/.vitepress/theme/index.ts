import DefaultTheme from 'vitepress/theme'
import Layout from './Layout.vue'
import '@fontsource/inter/400.css'
import '@fontsource/inter/500.css'
import '@fontsource/inter/600.css'
import '@fontsource/inter/700.css'
import './table-cards.css'
import './fira-code.css'

export default {
  extends: DefaultTheme,
  Layout,
}
