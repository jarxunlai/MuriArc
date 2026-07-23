<script setup lang="ts">
import { computed, onMounted } from 'vue'
import { useMessage } from 'naive-ui'
import {
  FileSignature,
  FolderKanban,
  History,
  MessageSquarePlus,
  ShieldCheck,
  Wrench,
} from '@lucide/vue'
import PageHeader from '@/components/PageHeader.vue'
import AiConversation from '@/components/AiConversation.vue'
import { useAiAssistant } from '@/composables/useAiAssistant'

const ai = useAiAssistant()
const toast = useMessage()
const projectOptions = computed(() => ai.projects.value.map((project) => ({
  label: project.name,
  value: project.id,
})))
const conversationOptions = computed(() => ai.conversations.value.map((conversation) => ({
  label: conversation.title,
  value: conversation.id,
})))

function changeProject(value: string | null) {
  void ai.selectProject(value ?? undefined).catch((error) => {
    toast.error(error instanceof Error ? error.message : '无法切换科研项目')
  })
}

function formatConversationDate(value: string) {
  return new Intl.DateTimeFormat('zh-CN', {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  }).format(new Date(value))
}

async function openConversation(id: string) {
  try {
    await ai.openConversation(id)
  } catch (error) {
    toast.error(error instanceof Error ? error.message : '无法读取历史会话')
  }
}

function changeConversation(value: string | null) {
  if (value) void openConversation(value)
}

onMounted(async () => {
  ai.setContext('全部已授权数据', '/ai')
  try {
    await Promise.all([ai.loadModels(true), ai.loadProjects()])
    await ai.restoreLatestConversation()
  } catch (error) {
    toast.error(error instanceof Error ? error.message : '无法读取模型、科研项目或历史会话')
  }
  try {
    await ai.refreshDrafts()
  } catch {
    // AI may be intentionally disabled. A real turn or settings page provides
    // the actionable error without replacing the workspace with demo data.
  }
})
</script>

<template>
  <div class="page ai-page">
    <PageHeader title="AI 助手" description="用自然语言查询和整理数据；回答保留来源，写入先展示 diff。">
      <template #actions>
        <n-button type="primary" secondary :disabled="ai.busy.value" @click="ai.newConversation">
          <template #icon><MessageSquarePlus :size="17" /></template>新会话
        </n-button>
      </template>
    </PageHeader>
    <section class="ai-workspace surface">
      <aside>
        <div class="scope-heading"><FolderKanban :size="16" /><strong>科研项目上下文</strong></div>
        <n-select
          :value="ai.selectedProjectId.value ?? null"
          :options="projectOptions"
          clearable
          filterable
          size="small"
          :disabled="ai.busy.value"
          placeholder="跨项目只读"
          @update:value="changeProject"
        />
        <p class="scope-help">
          {{ ai.selectedProject.value
            ? '会话固定在该项目；具备权限时可生成写入草稿。'
            : '未选择项目时可跨项目查询，但强制只读。' }}
        </p>

        <section class="conversation-history" aria-label="历史会话">
          <div class="history-heading">
            <History :size="16" />
            <strong>历史会话</strong>
            <n-spin v-if="ai.loadingConversations.value" :size="13" />
          </div>
          <div v-if="ai.conversations.value.length" class="history-list">
            <button
              v-for="conversation in ai.conversations.value"
              :key="conversation.id"
              type="button"
              :class="{ active: conversation.id === ai.conversationId.value }"
              :disabled="ai.loadingConversation.value || ai.busy.value"
              @click="openConversation(conversation.id)"
            >
              <span class="history-copy">
                <strong>{{ conversation.title }}</strong>
                <small>
                  {{ conversation.modelProfileName ?? conversation.modelId
                    ?? (conversation.modelProfileId ? '模型不可用' : '旧会话模型未知') }}
                  <template v-if="conversation.modelProfileVersion">
                    · v{{ conversation.modelProfileVersion }}
                  </template>
                  <template v-if="conversation.readOnly"> · 只读</template>
                </small>
              </span>
              <small>{{ formatConversationDate(conversation.updatedAt) }}</small>
            </button>
          </div>
          <p v-else-if="!ai.loadingConversations.value" class="history-empty">当前范围还没有已保存会话</p>
        </section>

        <div class="tool-scope">
          <div><FileSignature :size="16" /><strong>待审批草稿</strong></div>
          <span v-if="ai.loadingDrafts.value">正在读取…</span>
          <span v-else-if="ai.pendingDrafts.value.length">{{ ai.pendingDrafts.value.length }} 项等待人工决定</span>
          <span v-else>当前没有待审批草稿</span>
        </div>
        <div class="tool-scope">
          <div><ShieldCheck :size="16" /><strong>安全边界</strong></div>
          <span>只读取当前账号有权访问的数据</span>
          <span>写入先创建可审阅草稿</span>
          <span class="denied">不能执行任意 SQL 或删除正式数据</span>
        </div>
        <div class="tool-scope">
          <div><Wrench :size="16" /><strong>安全领域工具</strong></div>
          <span>animal_search / timeline</span>
          <span>experiment_status</span>
          <span>measurement_query</span>
          <span>sample_inventory</span>
        </div>
      </aside>
      <div class="conversation-wrap">
        <div class="mobile-project-select">
          <span>科研项目</span>
          <n-select
            :value="ai.selectedProjectId.value ?? null"
            :options="projectOptions"
            clearable
            filterable
            size="small"
            :disabled="ai.busy.value"
            placeholder="跨项目只读"
            @update:value="changeProject"
          />
          <span>历史会话</span>
          <n-select
            :value="ai.conversationId.value ?? null"
            :options="conversationOptions"
            :loading="ai.loadingConversations.value || ai.loadingConversation.value"
            clearable
            filterable
            size="small"
            :disabled="ai.busy.value"
            placeholder="新会话"
            @update:value="changeConversation"
          />
        </div>
        <AiConversation />
      </div>
    </section>
  </div>
</template>

<style scoped>
.ai-page { height: calc(100vh - var(--muri-topbar-height)); display: flex; flex-direction: column; }
.ai-workspace { display: grid; min-height: 520px; flex: 1; grid-template-columns: 248px minmax(0, 1fr); overflow: hidden; }
.ai-workspace > aside { padding: 14px 12px; border-right: 1px solid var(--muri-border); background: var(--muri-surface-muted); }
.scope-heading { display: flex; align-items: center; gap: 6px; margin-bottom: 9px; color: var(--muri-text); font-size: 12px; }.scope-heading svg { color: var(--muri-primary); }
.scope-help { margin: 7px 2px 15px; color: var(--muri-text-tertiary); font-size: 11px; line-height: 1.55; }
.conversation-history { margin: 0 -4px 8px; padding: 11px 4px 4px; border-top: 1px solid var(--muri-border); }
.history-heading { display: flex; align-items: center; gap: 6px; margin-bottom: 7px; color: var(--muri-text); font-size: 12px; }.history-heading svg { color: var(--muri-primary); }.history-heading :deep(.n-spin-container) { margin-left: auto; }
.history-list { display: flex; max-height: 190px; flex-direction: column; gap: 3px; overflow-y: auto; }
.history-list button { display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: 7px; width: 100%; padding: 7px 8px; border: 1px solid transparent; border-radius: 6px; color: var(--muri-text-secondary); background: transparent; cursor: pointer; text-align: left; transition: color var(--muri-transition-fast), background var(--muri-transition-fast), border-color var(--muri-transition-fast); }.history-list button:hover { color: var(--muri-text); background: var(--muri-surface); }.history-list button.active { border-color: #c8deef; color: var(--muri-primary); background: var(--muri-primary-soft); }.history-list button:disabled { cursor: wait; opacity: .65; }.history-list .history-copy { display: flex; min-width: 0; flex-direction: column; }.history-list .history-copy strong, .history-list .history-copy small { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }.history-list .history-copy strong { font-size: 11px; font-weight: 600; }.history-list small { color: var(--muri-text-tertiary); font-size: 10px; }
.history-empty { margin: 4px 8px 8px; color: var(--muri-text-tertiary); font-size: 11px; line-height: 1.5; }
.tool-scope { display: flex; padding: 11px 4px; flex-direction: column; gap: 5px; margin-top: 4px; border-top: 1px solid var(--muri-border); color: var(--muri-text-secondary); font-size: 11px; }.tool-scope div { display: flex; align-items: center; gap: 6px; margin-bottom: 3px; color: var(--muri-text); }.tool-scope div svg { color: var(--muri-primary); }.tool-scope > span::before { margin-right: 6px; color: var(--muri-success); content: '•'; }.tool-scope span.denied::before { color: var(--muri-danger); }
.conversation-wrap { min-width: 0; min-height: 0; }.mobile-project-select { display: none; }
@media (max-width: 800px) {
  .ai-page { height: auto; min-height: calc(100vh - 54px); }.ai-workspace { min-height: calc(100vh - 190px); grid-template-columns: 1fr; }.ai-workspace > aside { display: none; }.conversation-wrap { display: grid; grid-template-rows: auto minmax(0, 1fr); }.mobile-project-select { display: grid; grid-template-columns: auto minmax(0, 1fr); align-items: center; gap: 7px 10px; padding: 9px 12px; border-bottom: 1px solid var(--muri-border); background: var(--muri-surface-muted); color: var(--muri-text-secondary); font-size: 11px; }.mobile-project-select :deep(.n-select) { max-width: 360px; }
}
</style>
