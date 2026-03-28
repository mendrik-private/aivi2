<script setup lang="ts">
import DefaultTheme from 'vitepress/theme'
import { useRoute } from 'vitepress'
import { watch, onMounted, nextTick } from 'vue'

const { Layout } = DefaultTheme

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

const route = useRoute()
watch(
  () => route.path,
  () => nextTick(labelTableCells)
)
onMounted(labelTableCells)
</script>

<template>
  <Layout />
</template>
