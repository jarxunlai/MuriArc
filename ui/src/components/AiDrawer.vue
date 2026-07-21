<script setup lang="ts">
import { ExternalLink, Sparkles } from '@lucide/vue'
import { useRouter } from 'vue-router'
import { useAiAssistant } from '@/composables/useAiAssistant'
import AiConversation from './AiConversation.vue'

const ai = useAiAssistant()
const router = useRouter()

async function expandWorkspace() {
  ai.drawerOpen.value = false
  await router.push('/ai')
}
</script>

<template>
  <n-drawer v-model:show="ai.drawerOpen.value" :width="440" :default-width="440" placement="right" resizable>
    <n-drawer-content :native-scrollbar="false" body-content-style="padding: 0; height: 100%;">
      <template #header>
        <div class="drawer-title"><Sparkles :size="18" /><span>AI 助手</span><small>{{ ai.contextTitle.value }}</small></div>
      </template>
      <template #header-extra>
        <n-button quaternary size="small" @click="expandWorkspace"><template #icon><ExternalLink :size="15" /></template>展开</n-button>
      </template>
      <AiConversation compact />
    </n-drawer-content>
  </n-drawer>
</template>

<style scoped>
.drawer-title { display: flex; align-items: center; gap: 8px; color: var(--muri-text); }
.drawer-title > svg { color: var(--muri-primary); }
.drawer-title small { max-width: 150px; overflow: hidden; color: var(--muri-text-tertiary); font-weight: 400; text-overflow: ellipsis; white-space: nowrap; }
@media (max-width: 560px) { :global(.n-drawer) { width: 100% !important; max-width: 100% !important; } }
</style>
