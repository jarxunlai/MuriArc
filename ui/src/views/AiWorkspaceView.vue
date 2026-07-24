<script setup lang="ts">
import { onMounted } from 'vue'
import { useMessage } from 'naive-ui'
import { MessageSquarePlus } from '@lucide/vue'
import PageHeader from '@/components/PageHeader.vue'
import AiWorkspaceShell from '@/components/AiWorkspaceShell.vue'
import { useAiAssistant } from '@/composables/useAiAssistant'

const ai = useAiAssistant()
const toast = useMessage()

onMounted(async () => {
  ai.setContext('AI 工作台', '/ai')
  try {
    await ai.loadModels(true)
  } catch (error) {
    toast.error(error instanceof Error ? error.message : '无法读取 AI 模型配置')
  }
})
</script>

<template>
  <div class="page ai-page">
    <PageHeader
      title="AI 工作台"
      description="查询当前权限内的正式业务数据；实验室级会话只读，项目内 AI 来源导入仅生成实验测量草稿并由人确认。"
      :show-ask-ai="false"
    >
      <template #actions>
        <n-button type="primary" secondary :disabled="ai.busy.value" @click="ai.newConversation">
          <template #icon><MessageSquarePlus :size="17" /></template>
          新会话
        </n-button>
      </template>
    </PageHeader>
    <AiWorkspaceShell class="workspace-surface surface" />
  </div>
</template>

<style scoped>
.ai-page {
  display: flex;
  height: calc(100vh - var(--muri-topbar-height));
  min-height: 640px;
  flex-direction: column;
}
.workspace-surface {
  min-height: 520px;
  flex: 1;
}
@media (max-width: 900px) {
  .ai-page {
    height: auto;
    min-height: calc(100dvh - 54px);
  }
  .workspace-surface {
    min-height: calc(100dvh - 190px);
  }
}
</style>
