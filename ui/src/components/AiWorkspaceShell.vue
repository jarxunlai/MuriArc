<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useMessage } from 'naive-ui'
import {
  Archive,
  FileSignature,
  FolderKanban,
  MessageSquarePlus,
  MoreHorizontal,
  Pin,
  Search,
  ShieldCheck,
  SlidersHorizontal,
  X,
} from '@lucide/vue'
import type {
  AiConversationAction,
  AiConversationArchiveFilter,
  AiConversationSummary,
} from '@/domain/models'
import { useAiAssistant } from '@/composables/useAiAssistant'
import AiConversation from './AiConversation.vue'

withDefaults(defineProps<{ compact?: boolean }>(), { compact: false })

const ai = useAiAssistant()
const toast = useMessage()
const searchQuery = ref(ai.conversationFilter.value.titleQuery ?? '')
const renameTarget = ref<AiConversationSummary>()
const renameTitle = ref('')
const mobileManagerOpen = ref(false)
let searchTimer: ReturnType<typeof setTimeout> | undefined

const projectOptions = computed(() => [
  { label: '全部项目与跨项目会话', value: '' },
  ...ai.projects.value.map((project) => ({
    label: project.name,
    value: project.id,
  })),
])
const projectNames = computed(() => new Map(
  ai.projects.value.map((project) => [project.id, project.name]),
))
type VisibleArchiveFilter = Exclude<AiConversationArchiveFilter, 'all'>
const archiveFilter = computed<VisibleArchiveFilter>(
  () => ai.conversationFilter.value.archive === 'archived' ? 'archived' : 'active',
)

function formatConversationDate(value: string) {
  return new Intl.DateTimeFormat('zh-CN', {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  }).format(new Date(value))
}

function projectLabel(conversation: AiConversationSummary) {
  return conversation.projectId
    ? projectNames.value.get(conversation.projectId) ?? '已授权项目'
    : '跨项目只读'
}

function modelLabel(conversation: AiConversationSummary) {
  const identity = conversation.modelProfileName
    ?? conversation.modelId
    ?? (conversation.modelProfileId ? '模型版本不可用' : '旧会话模型未知')
  const version = conversation.modelProfileVersion
    ? ` · v${conversation.modelProfileVersion}`
    : ''
  return `${identity}${version}${conversation.readOnly ? ' · 只读' : ''}`
}

async function changeProject(value: string | null) {
  try {
    await ai.selectProject(value || undefined)
  } catch (error) {
    toast.error(error instanceof Error ? error.message : '无法切换科研项目')
  }
}

async function changeArchive(value: AiConversationArchiveFilter) {
  try {
    await ai.setConversationFilter({ archive: value })
  } catch (error) {
    toast.error(error instanceof Error ? error.message : '无法读取会话')
  }
}

async function moveArchiveTab(
  event: KeyboardEvent,
  current: VisibleArchiveFilter,
) {
  const order: VisibleArchiveFilter[] = ['active', 'archived']
  const currentIndex = order.indexOf(current)
  const target = event.key === 'Home'
    ? order[0]
    : event.key === 'End'
      ? order.at(-1)
      : event.key === 'ArrowRight'
        ? order[(currentIndex + 1) % order.length]
        : event.key === 'ArrowLeft'
          ? order[(currentIndex - 1 + order.length) % order.length]
          : undefined
  if (!target) return

  event.preventDefault()
  const tablist = (event.currentTarget as HTMLElement).closest('[role="tablist"]')
  await changeArchive(target)
  await nextTick()
  tablist
    ?.querySelector<HTMLElement>(`[data-archive-tab="${target}"]`)
    ?.focus()
}

async function openConversation(id: string) {
  try {
    await ai.openConversation(id)
    mobileManagerOpen.value = false
  } catch (error) {
    toast.error(error instanceof Error ? error.message : '无法读取历史会话')
  }
}

function startNewConversation() {
  ai.newConversation()
  mobileManagerOpen.value = false
}

function conversationActions(conversation: AiConversationSummary) {
  if (conversation.archivedAt) {
    return [
      { label: '重命名', key: 'rename' },
      { label: '恢复会话', key: 'unarchive' },
    ]
  }
  return [
    { label: conversation.pinnedAt ? '取消置顶' : '置顶', key: conversation.pinnedAt ? 'unpin' : 'pin' },
    { label: '重命名', key: 'rename' },
    { label: '归档', key: 'archive' },
  ]
}

function beginRename(conversation: AiConversationSummary) {
  renameTarget.value = conversation
  renameTitle.value = conversation.title
}

async function applyConversationAction(
  conversation: AiConversationSummary,
  action: AiConversationAction,
  title?: string,
) {
  try {
    await ai.updateConversation(conversation, { action, title })
    const messages: Partial<Record<AiConversationAction, string>> = {
      pin: '会话已置顶',
      unpin: '已取消置顶',
      archive: '会话已归档',
      unarchive: '会话已恢复',
      rename: '会话已重命名',
    }
    toast.success(messages[action] ?? '会话已更新')
  } catch (error) {
    toast.error(error instanceof Error ? error.message : '无法更新会话')
  }
}

function handleConversationAction(conversation: AiConversationSummary, key: string) {
  if (key === 'rename') {
    beginRename(conversation)
    return
  }
  void applyConversationAction(conversation, key as AiConversationAction)
}

async function submitRename() {
  const target = renameTarget.value
  const title = renameTitle.value.trim()
  if (!target || !title) return
  await applyConversationAction(target, 'rename', title)
  renameTarget.value = undefined
  renameTitle.value = ''
}

watch(searchQuery, (value) => {
  if (searchTimer) clearTimeout(searchTimer)
  searchTimer = setTimeout(() => {
    void ai.setConversationFilter({ titleQuery: value.trim() || undefined })
      .catch((error) => toast.error(error instanceof Error ? error.message : '无法搜索会话'))
  }, 220)
})

onMounted(async () => {
  try {
    await ai.loadProjects()
    await ai.restoreLatestConversation()
  } catch (error) {
    toast.error(error instanceof Error ? error.message : '无法读取科研项目或历史会话')
  }
  try {
    await ai.refreshDrafts()
  } catch {
    // A disabled AI runtime is surfaced by the first real turn or settings,
    // without replacing the workspace with simulated approval data.
  }
})

onBeforeUnmount(() => {
  if (searchTimer) clearTimeout(searchTimer)
})
</script>

<template>
  <section class="ai-shell" :class="{ compact }">
    <aside class="conversation-sidebar" aria-label="AI 会话导航">
      <header class="sidebar-header">
        <div>
          <strong>会话</strong>
          <span>{{ ai.conversations.value.length }} 项</span>
        </div>
        <n-button size="tiny" type="primary" secondary aria-label="新建 AI 会话" @click="startNewConversation">
          <template #icon><MessageSquarePlus :size="15" /></template>
          新建
        </n-button>
      </header>

      <label class="field-label" for="ai-project-filter">
        <FolderKanban :size="14" />
        项目范围
      </label>
      <n-select
        id="ai-project-filter"
        :value="ai.conversationFilter.value.projectId ?? ''"
        :options="projectOptions"
        filterable
        size="small"
        @update:value="changeProject"
      />

      <n-input
        v-model:value="searchQuery"
        class="conversation-search"
        size="small"
        clearable
        aria-label="按标题搜索 AI 会话"
        placeholder="搜索会话标题"
      >
        <template #prefix><Search :size="14" /></template>
      </n-input>

      <div class="archive-tabs" role="tablist" aria-label="会话归档状态">
        <button
          id="ai-archive-tab-active-desktop"
          type="button"
          role="tab"
          data-archive-tab="active"
          aria-controls="ai-conversation-panel-desktop"
          :aria-selected="archiveFilter === 'active'"
          :tabindex="archiveFilter === 'active' ? 0 : -1"
          :class="{ active: archiveFilter === 'active' }"
          @click="changeArchive('active')"
          @keydown="moveArchiveTab($event, 'active')"
        >进行中</button>
        <button
          id="ai-archive-tab-archived-desktop"
          type="button"
          role="tab"
          data-archive-tab="archived"
          aria-controls="ai-conversation-panel-desktop"
          :aria-selected="archiveFilter === 'archived'"
          :tabindex="archiveFilter === 'archived' ? 0 : -1"
          :class="{ active: archiveFilter === 'archived' }"
          @click="changeArchive('archived')"
          @keydown="moveArchiveTab($event, 'archived')"
        >已归档</button>
      </div>

      <div
        id="ai-conversation-panel-desktop"
        class="conversation-list"
        role="tabpanel"
        :aria-labelledby="`ai-archive-tab-${archiveFilter}-desktop`"
        aria-live="polite"
      >
        <div v-if="ai.loadingConversations.value" class="conversation-state">
          <n-spin :size="18" />
          正在读取会话
        </div>
        <article
          v-for="conversation in ai.conversations.value"
          v-else
          :key="conversation.id"
          class="conversation-row"
          :class="{ active: conversation.id === ai.conversationId.value }"
        >
          <button
            type="button"
            class="conversation-main"
            :disabled="ai.loadingConversation.value"
            :aria-current="conversation.id === ai.conversationId.value ? 'page' : undefined"
            @click="openConversation(conversation.id)"
          >
            <span class="conversation-title">
              <Pin v-if="conversation.pinnedAt" :size="12" aria-label="已置顶" />
              <strong>{{ conversation.title }}</strong>
            </span>
            <span class="conversation-meta">
              <small>{{ projectLabel(conversation) }} · {{ modelLabel(conversation) }}</small>
              <time :datetime="conversation.updatedAt">{{ formatConversationDate(conversation.updatedAt) }}</time>
            </span>
          </button>
          <n-dropdown
            trigger="click"
            :options="conversationActions(conversation)"
            @select="handleConversationAction(conversation, $event)"
          >
            <button
              type="button"
              class="conversation-menu"
              :disabled="ai.conversationBusy(conversation.id)"
              :aria-label="`管理会话：${conversation.title}`"
            ><MoreHorizontal :size="16" /></button>
          </n-dropdown>
        </article>
        <div
          v-if="!ai.loadingConversations.value && !ai.conversations.value.length"
          class="conversation-state empty"
        >
          <Archive v-if="archiveFilter === 'archived'" :size="20" />
          <Search v-else-if="searchQuery" :size="20" />
          <MessageSquarePlus v-else :size="20" />
          <span>{{ searchQuery ? '没有匹配的会话' : archiveFilter === 'archived' ? '还没有归档会话' : '从一个科研问题开始' }}</span>
        </div>
      </div>

      <footer class="sidebar-footer">
        <div><FileSignature :size="14" />待审批草稿 <strong>{{ ai.pendingDrafts.value.length }}</strong></div>
        <span><ShieldCheck :size="13" />查询和写入始终受当前权限与人工审批约束</span>
      </footer>
    </aside>

    <div class="conversation-panel">
      <div class="mobile-scope">
        <n-select
          :value="ai.conversationFilter.value.projectId ?? ''"
          :options="projectOptions"
          size="small"
          aria-label="项目范围"
          @update:value="changeProject"
        />
        <button
          type="button"
          class="mobile-manager-toggle"
          aria-controls="mobile-ai-conversation-manager"
          :aria-expanded="mobileManagerOpen"
          @click="mobileManagerOpen = !mobileManagerOpen"
        >
          <SlidersHorizontal :size="16" />
          <span>{{ ai.currentConversation.value?.title ?? '会话管理' }}</span>
          <small>{{ ai.conversations.value.length }}</small>
        </button>
      </div>
      <section
        v-if="mobileManagerOpen"
        id="mobile-ai-conversation-manager"
        class="mobile-manager"
        aria-label="移动端 AI 会话管理"
      >
        <header>
          <strong>会话管理</strong>
          <div>
            <button type="button" class="mobile-new" aria-label="新建 AI 会话" @click="startNewConversation">
              <MessageSquarePlus :size="16" />
              新建
            </button>
            <button
              type="button"
              class="mobile-close"
              aria-label="关闭会话管理"
              @click="mobileManagerOpen = false"
            ><X :size="17" /></button>
          </div>
        </header>
        <n-input
          v-model:value="searchQuery"
          size="small"
          clearable
          aria-label="按标题搜索 AI 会话"
          placeholder="搜索会话标题"
        >
          <template #prefix><Search :size="14" /></template>
        </n-input>
        <div
          class="archive-tabs mobile-archive-tabs"
          role="tablist"
          aria-label="移动端会话归档状态"
        >
          <button
            id="ai-archive-tab-active-mobile"
            type="button"
            role="tab"
            data-archive-tab="active"
            aria-controls="ai-conversation-panel-mobile"
            :aria-selected="archiveFilter === 'active'"
            :tabindex="archiveFilter === 'active' ? 0 : -1"
            :class="{ active: archiveFilter === 'active' }"
            @click="changeArchive('active')"
            @keydown="moveArchiveTab($event, 'active')"
          >进行中</button>
          <button
            id="ai-archive-tab-archived-mobile"
            type="button"
            role="tab"
            data-archive-tab="archived"
            aria-controls="ai-conversation-panel-mobile"
            :aria-selected="archiveFilter === 'archived'"
            :tabindex="archiveFilter === 'archived' ? 0 : -1"
            :class="{ active: archiveFilter === 'archived' }"
            @click="changeArchive('archived')"
            @keydown="moveArchiveTab($event, 'archived')"
          >已归档</button>
        </div>
        <div
          id="ai-conversation-panel-mobile"
          class="mobile-conversation-list"
          role="tabpanel"
          :aria-labelledby="`ai-archive-tab-${archiveFilter}-mobile`"
          aria-live="polite"
        >
          <article
            v-for="conversation in ai.conversations.value"
            :key="conversation.id"
            class="conversation-row"
            :class="{ active: conversation.id === ai.conversationId.value }"
          >
            <button
              type="button"
              class="conversation-main"
              :disabled="ai.loadingConversation.value"
              :aria-current="conversation.id === ai.conversationId.value ? 'page' : undefined"
              @click="openConversation(conversation.id)"
            >
              <span class="conversation-title">
                <Pin v-if="conversation.pinnedAt" :size="12" aria-label="已置顶" />
                <strong>{{ conversation.title }}</strong>
              </span>
              <span class="conversation-meta">
                <small>{{ projectLabel(conversation) }} · {{ modelLabel(conversation) }}</small>
                <time :datetime="conversation.updatedAt">{{ formatConversationDate(conversation.updatedAt) }}</time>
              </span>
            </button>
            <n-dropdown
              trigger="click"
              :options="conversationActions(conversation)"
              @select="handleConversationAction(conversation, $event)"
            >
              <button
                type="button"
                class="conversation-menu"
                :disabled="ai.conversationBusy(conversation.id)"
                :aria-label="`管理会话：${conversation.title}`"
              ><MoreHorizontal :size="16" /></button>
            </n-dropdown>
          </article>
          <div v-if="!ai.conversations.value.length" class="conversation-state">
            {{ searchQuery ? '没有匹配的会话' : archiveFilter === 'archived' ? '还没有归档会话' : '从一个科研问题开始' }}
          </div>
        </div>
      </section>
      <AiConversation :compact="compact" />
    </div>

    <n-modal
      :show="Boolean(renameTarget)"
      preset="card"
      title="重命名会话"
      class="rename-modal"
      @update:show="(show: boolean) => { if (!show) renameTarget = undefined }"
    >
      <n-input
        v-model:value="renameTitle"
        maxlength="120"
        show-count
        autofocus
        placeholder="输入便于检索的会话标题"
        aria-label="会话标题"
        @keydown.enter.prevent="submitRename"
      />
      <template #footer>
        <div class="rename-actions">
          <n-button @click="renameTarget = undefined">取消</n-button>
          <n-button
            type="primary"
            :disabled="!renameTitle.trim()"
            :loading="renameTarget ? ai.conversationBusy(renameTarget.id) : false"
            @click="submitRename"
          >保存</n-button>
        </div>
      </template>
    </n-modal>
  </section>
</template>

<style scoped>
.ai-shell { display: grid; width: 100%; height: 100%; min-width: 0; min-height: 0; grid-template-columns: 254px minmax(0, 1fr); overflow: hidden; background: var(--muri-surface); }
.conversation-sidebar { display: flex; min-width: 0; min-height: 0; padding: 12px 10px 10px; border-right: 1px solid var(--muri-border); flex-direction: column; background: var(--muri-surface-muted); }
.sidebar-header { display: flex; align-items: center; justify-content: space-between; gap: 8px; margin-bottom: 12px; }.sidebar-header > div { display: flex; align-items: baseline; gap: 6px; }.sidebar-header strong { color: var(--muri-text); }.sidebar-header span { color: var(--muri-text-tertiary); font-size: 10px; }
.field-label { display: flex; align-items: center; gap: 5px; margin: 0 2px 6px; color: var(--muri-text-secondary); font-size: 11px; font-weight: 600; }.field-label svg { color: var(--muri-primary); }
.conversation-search { margin-top: 8px; }
.archive-tabs { display: grid; grid-template-columns: 1fr 1fr; gap: 3px; margin: 9px 0 7px; padding: 3px; border-radius: 7px; background: #edf1f4; }.archive-tabs button { min-height: 44px; padding: 5px 8px; border: 0; border-radius: 5px; color: var(--muri-text-secondary); background: transparent; cursor: pointer; font-size: 11px; }.archive-tabs button:hover { color: var(--muri-primary); }.archive-tabs button.active { color: var(--muri-text); background: white; box-shadow: 0 1px 3px rgba(30,53,76,.1); }.archive-tabs button:focus-visible, .conversation-main:focus-visible, .conversation-menu:focus-visible { outline: 3px solid var(--muri-primary); outline-offset: 2px; }
.conversation-list { min-height: 0; flex: 1; overflow-y: auto; }
.conversation-row { display: grid; grid-template-columns: minmax(0, 1fr) 30px; align-items: center; margin-bottom: 3px; border: 1px solid transparent; border-radius: 7px; transition: border-color var(--muri-transition-fast), background var(--muri-transition-fast); }.conversation-row:hover { background: white; }.conversation-row.active { border-color: #c8deef; background: var(--muri-primary-soft); }
.conversation-main { display: flex; min-width: 0; padding: 8px 5px 8px 8px; border: 0; flex-direction: column; gap: 4px; color: var(--muri-text-secondary); background: transparent; cursor: pointer; text-align: left; }.conversation-main:disabled { cursor: wait; opacity: .65; }.conversation-title { display: flex; width: 100%; min-width: 0; align-items: center; gap: 5px; }.conversation-title svg { flex: 0 0 auto; color: var(--muri-primary); }.conversation-title strong { overflow: hidden; color: var(--muri-text); font-size: 11px; font-weight: 600; text-overflow: ellipsis; white-space: nowrap; }.conversation-meta { display: flex; width: 100%; min-width: 0; justify-content: space-between; gap: 6px; color: var(--muri-text-tertiary); font-size: 9px; }.conversation-meta small { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }.conversation-meta time { flex: 0 0 auto; }
.conversation-menu { display: grid; width: 28px; height: 28px; padding: 0; place-items: center; border: 0; border-radius: 5px; color: var(--muri-text-tertiary); background: transparent; cursor: pointer; }.conversation-menu:hover { color: var(--muri-primary); background: rgba(255,255,255,.8); }.conversation-menu:disabled { cursor: wait; opacity: .55; }
.conversation-state { display: flex; min-height: 80px; align-items: center; justify-content: center; flex-direction: column; gap: 7px; color: var(--muri-text-tertiary); font-size: 11px; text-align: center; }.conversation-state.empty { min-height: 140px; }
.sidebar-footer { display: flex; padding: 9px 2px 0; border-top: 1px solid var(--muri-border); flex-direction: column; gap: 6px; color: var(--muri-text-secondary); font-size: 10px; }.sidebar-footer div, .sidebar-footer span { display: flex; align-items: center; gap: 5px; }.sidebar-footer svg { flex: 0 0 auto; color: var(--muri-primary); }.sidebar-footer div strong { margin-left: auto; color: var(--muri-primary); }
.conversation-panel { display: grid; min-width: 0; min-height: 0; grid-template-rows: minmax(0, 1fr); }.mobile-scope { display: none; }
.mobile-manager { display: none; }
.rename-modal { width: min(460px, calc(100vw - 28px)); }.rename-actions { display: flex; justify-content: flex-end; gap: 8px; }
.compact { grid-template-columns: 238px minmax(0, 1fr); }
@media (max-width: 760px) {
  .ai-shell { grid-template-columns: 1fr; }
  .conversation-sidebar { display: none; }
  .conversation-panel { grid-template-rows: auto auto minmax(0, 1fr); }
  .mobile-scope { display: grid; grid-template-columns: minmax(0, 1fr) minmax(0, 1fr); gap: 7px; padding: 8px 10px; border-bottom: 1px solid var(--muri-border); background: var(--muri-surface-muted); }
  .mobile-scope :deep(.n-base-selection) { min-height: 44px; }
  .mobile-manager-toggle { display: flex; min-width: 0; min-height: 44px; align-items: center; gap: 7px; padding: 6px 9px; border: 1px solid var(--muri-border-strong); border-radius: 6px; color: var(--muri-text-secondary); background: white; cursor: pointer; }.mobile-manager-toggle span { min-width: 0; flex: 1; overflow: hidden; color: var(--muri-text); font-size: 12px; text-align: left; text-overflow: ellipsis; white-space: nowrap; }.mobile-manager-toggle small { min-width: 20px; padding: 1px 5px; border-radius: 999px; color: var(--muri-primary); background: var(--muri-primary-soft); text-align: center; }
  .mobile-manager { display: flex; max-height: min(58dvh, 440px); padding: 10px; border-bottom: 1px solid var(--muri-border); flex-direction: column; gap: 9px; background: var(--muri-surface-muted); overflow: hidden; }.mobile-manager > header { display: flex; min-height: 44px; align-items: center; justify-content: space-between; gap: 8px; }.mobile-manager > header > div { display: flex; gap: 5px; }.mobile-manager :deep(.n-input) { min-height: 44px; }.mobile-new, .mobile-close { display: flex; min-height: 44px; align-items: center; justify-content: center; gap: 5px; padding: 6px 10px; border: 1px solid var(--muri-border); border-radius: 7px; color: var(--muri-primary); background: white; cursor: pointer; }.mobile-close { width: 44px; padding: 0; color: var(--muri-text-secondary); }.mobile-conversation-list { min-height: 0; overflow-y: auto; }.mobile-conversation-list .conversation-row { min-height: 48px; grid-template-columns: minmax(0, 1fr) 44px; background: white; }.mobile-conversation-list .conversation-menu { width: 44px; height: 44px; }.mobile-conversation-list .conversation-main { min-height: 48px; }
  .mobile-manager-toggle:focus-visible, .mobile-new:focus-visible, .mobile-close:focus-visible { outline: 3px solid var(--muri-primary); outline-offset: 2px; }
  .mobile-archive-tabs { margin: 0; }.mobile-archive-tabs button { min-height: 44px; }
}
@media (max-width: 430px) {
  .mobile-scope { grid-template-columns: 1fr; }
}
@media (prefers-reduced-motion: reduce) {
  .conversation-row { transition: none; }
}
</style>
