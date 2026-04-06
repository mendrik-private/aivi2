<script setup lang="ts">
import DefaultTheme from 'vitepress/theme'
import { useData, useRoute, onContentUpdated } from 'vitepress'
import aiviLogo from '../../../assets/aivi-logo.png'
import { watch, onMounted, nextTick } from 'vue'

const { Layout } = DefaultTheme
const { frontmatter } = useData()

function labelTableCells() {
  document.querySelectorAll('.vp-doc table').forEach(table => {
    const headers = Array.from(table.querySelectorAll('thead th')).map(
      th => th.textContent?.trim() ?? ''
    )
    if (headers.length === 0) return
    table.querySelectorAll('tbody tr').forEach(row => {
      Array.from(row.querySelectorAll('td')).forEach((td, i) => {
        if (headers[i]) td.setAttribute('data-column', headers[i])
      })
    })
  })
}

// VitePress's serializeHeader strips all HTML and keeps only textContent,
// so headings with inline code (e.g. `### \`Button\``) lose their <code>
// formatting in the "On This Page" outline. Fix this by replacing each
// outline link's text with the actual heading innerHTML (minus the anchor).
function fixOutlineCodeLabels() {
  const links = document.querySelectorAll<HTMLAnchorElement>(
    '.VPDocOutlineItem .outline-link'
  )
  for (const link of links) {
    const hash = link.getAttribute('href')
    if (!hash?.startsWith('#')) continue
    const id = decodeURIComponent(hash.slice(1))
    const heading = document.getElementById(id)
    if (!heading) continue
    const clone = heading.cloneNode(true) as HTMLElement
    clone.querySelector('.header-anchor')?.remove()
    const html = clone.innerHTML.trim()
    if (html) link.innerHTML = html
  }
}

const route = useRoute()
watch(
  () => route.path,
  () => nextTick(labelTableCells)
)
onMounted(labelTableCells)
onContentUpdated(() => nextTick(fixOutlineCodeLabels))
onMounted(() => nextTick(fixOutlineCodeLabels))
</script>

<template>
  <Layout>
    <template v-if="frontmatter.layout === 'home'" #home-hero-info>
      <h1 class="manual-home-hero-heading">
        <img
          :src="aiviLogo"
          alt="AIVI"
          class="manual-home-hero-logo"
        />
        <span
          v-if="frontmatter.hero?.text"
          v-html="frontmatter.hero.text"
          class="manual-home-hero-text"
        />
      </h1>
      <p
        v-if="frontmatter.hero?.tagline"
        v-html="frontmatter.hero.tagline"
        class="manual-home-hero-tagline"
      />
    </template>
  </Layout>
</template>

<style scoped>
.manual-home-hero-heading {
  display: flex;
  flex-direction: column;
}

.manual-home-hero-logo {
  display: block;
  width: 250px;
  max-width: 100%;
  height: auto;
}

.manual-home-hero-text {
  margin-top: 12px;
  max-width: 392px;
  letter-spacing: -0.4px;
  line-height: 40px;
  font-size: 32px;
  font-weight: 700;
  white-space: pre-wrap;
  color: var(--vp-c-text-1);
}

.manual-home-hero-tagline {
  padding-top: 8px;
  max-width: 392px;
  line-height: 28px;
  font-size: 18px;
  font-weight: 500;
  white-space: pre-wrap;
  color: var(--vp-c-text-2);
}

@media (min-width: 640px) {
  .manual-home-hero-text {
    max-width: 576px;
    line-height: 56px;
    font-size: 48px;
  }

  .manual-home-hero-tagline {
    padding-top: 12px;
    max-width: 576px;
    line-height: 32px;
    font-size: 20px;
  }
}

@media (min-width: 960px) {
  .manual-home-hero-text {
    line-height: 64px;
    font-size: 56px;
  }

  .manual-home-hero-tagline {
    line-height: 36px;
    font-size: 24px;
  }
}
</style>
