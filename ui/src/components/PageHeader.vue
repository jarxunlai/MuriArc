<script setup lang="ts">
import { Bot, ChevronRight } from '@lucide/vue'
import { useRoute } from 'vue-router'
import { useAiAssistant } from '@/composables/useAiAssistant'

defineProps<{ title: string; description?: string; section?: string }>()
const route = useRoute()
const ai = useAiAssistant()
</script>

<template>
  <header class="page-header">
    <div class="heading">
      <div v-if="section" class="breadcrumb">
        <span>{{ section }}</span><ChevronRight :size="13" /><strong>{{ title }}</strong>
      </div>
      <h1>{{ title }}</h1>
      <p v-if="description">{{ description }}</p>
    </div>
    <div class="actions">
      <n-button secondary class="ask-ai" @click="ai.open(title, route.fullPath)">
        <template #icon><Bot :size="17" /></template>
        问 AI
      </n-button>
      <slot name="actions" />
    </div>
  </header>
</template>

<style scoped>
.page-header { display: flex; align-items: flex-end; justify-content: space-between; gap: 20px; margin-bottom: 18px; }
.heading { min-width: 0; }
.breadcrumb { display: flex; align-items: center; gap: 4px; margin-bottom: 5px; color: var(--muri-text-tertiary); font-size: 12px; }
.breadcrumb strong { color: var(--muri-text-secondary); font-weight: 500; }
h1 { margin: 0; font-size: 24px; line-height: 1.35; letter-spacing: -0.02em; }
p { margin: 5px 0 0; color: var(--muri-text-secondary); line-height: 1.5; }
.actions { display: flex; flex: 0 0 auto; align-items: center; gap: 10px; }
@media (max-width: 900px) {
  .page-header { align-items: flex-start; margin-bottom: 14px; }
  h1 { font-size: 21px; }
  .heading p { font-size: 13px; }
  .ask-ai :deep(.n-button__content) { font-size: 0; }
  .ask-ai { width: 38px; padding: 0; }
}
</style>
