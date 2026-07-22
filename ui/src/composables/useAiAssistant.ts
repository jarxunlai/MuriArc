import { computed, ref } from 'vue'
import type {
  AiAutonomyMode,
  AiAutonomyView,
  AiConversationDetail,
  AiConversationSummary,
  AiDraftDecisionInput,
  AiDraftDecisionResponse,
  AiMessage,
  AiWriteDraft,
  ProjectSummary,
} from '@/domain/models'
import { gateway } from '@/services/gateway'

const drawerOpen = ref(false)
const contextTitle = ref('MuriArc')
const contextRoute = ref('/cages')
const selectedProjectId = ref<string>()
const conversationId = ref<string>()
const conversations = ref<AiConversationSummary[]>([])
const projects = ref<ProjectSummary[]>([])
const projectsLoaded = ref(false)
const messages = ref<AiMessage[]>([welcomeMessage()])
const pendingDrafts = ref<AiWriteDraft[]>([])
const sending = ref(false)
const loadingDrafts = ref(false)
const loadingConversations = ref(false)
const loadingConversation = ref(false)
const autonomy = ref<AiAutonomyView>(defaultAutonomy())
const autonomyBusy = ref(false)
const reviewingDraftIds = ref(new Set<string>())
let conversationScopeVersion = 0
let conversationListRequest = 0
let conversationOpenRequest = 0
let freshConversationRequested = false

function welcomeMessage(): AiMessage {
  return {
    id: 'welcome',
    role: 'assistant',
    content: '你好，我可以基于你有权限访问的动物、实验和测量数据进行查询。选择科研项目后，涉及写入的请求只会生成可审阅草稿。',
    createdAt: new Date().toISOString(),
  }
}

function defaultAutonomy(): AiAutonomyView {
  return {
    mode: 'ask',
    effectiveMode: 'ask',
    maxMode: 'full',
    batchLimit: 1,
    revision: 0,
    requiresHumanApproval: [],
  }
}

function readableError(error: unknown): string {
  if (error instanceof Error) return error.message
  if (typeof error === 'string') return error
  if (error && typeof error === 'object') {
    const value = error as Record<string, unknown>
    if (typeof value.message === 'string') return value.message
  }
  return 'AI 请求失败，请检查模型设置或稍后重试'
}

function replaceDraft(updated: AiWriteDraft) {
  pendingDrafts.value = pendingDrafts.value
    .map((draft) => draft.id === updated.id ? updated : draft)
    .filter((draft) => draft.status === 'pending_approval')
  messages.value = messages.value.map((message) => ({
    ...message,
    drafts: message.drafts?.map((draft) => draft.id === updated.id ? updated : draft),
  }))
}

function mergePendingDrafts(additions: AiWriteDraft[]) {
  const merged = new Map(pendingDrafts.value.map((draft) => [draft.id, draft]))
  for (const draft of additions) {
    if (draft.status === 'pending_approval') merged.set(draft.id, draft)
    else merged.delete(draft.id)
  }
  pendingDrafts.value = [...merged.values()].sort((left, right) =>
    right.createdAt.localeCompare(left.createdAt))
}

function restoredMessages(detail: AiConversationDetail): AiMessage[] {
  return detail.messages.map((message) => {
    const response = message.response
    return {
      id: message.id,
      role: message.role,
      content: message.content,
      createdAt: message.createdAt,
      citations: response?.citations,
      toolRuns: response?.toolRuns,
      drafts: response?.drafts,
      trace: response?.trace,
    }
  })
}

export function useAiAssistant() {
  const busy = computed(() => sending.value || messages.value.some((message) => message.pending))
  const selectedProject = computed(() => projects.value.find((project) => project.id === selectedProjectId.value))
  const reinforcedPasswordRequired = computed(() => gateway.mode === 'remote')

  function setContext(title: string, route: string) {
    contextTitle.value = title
    contextRoute.value = route
  }

  function open(title?: string, route?: string) {
    if (title && route) setContext(title, route)
    drawerOpen.value = true
    void loadProjects().catch(() => undefined)
    void refreshDrafts().catch(() => undefined)
    void restoreLatestConversation().catch(() => undefined)
  }

  async function loadProjects(force = false) {
    if (projectsLoaded.value && !force) return
    const loaded = await gateway.listProjects()
    projects.value = loaded
    projectsLoaded.value = true
    if (selectedProjectId.value && !loaded.some((project) => project.id === selectedProjectId.value)) {
      selectedProjectId.value = undefined
      conversationScopeVersion += 1
      resetConversation(false)
      void restoreLatestConversation(true).catch(() => undefined)
    }
  }

  async function selectProject(projectId?: string) {
    const normalized = projectId || undefined
    if (selectedProjectId.value === normalized) return
    selectedProjectId.value = normalized
    conversationScopeVersion += 1
    pendingDrafts.value = []
    resetConversation(false)
    await Promise.all([
      restoreLatestConversation(true),
      refreshDrafts(),
    ])
  }

  function resetConversation(explicit: boolean) {
    conversationOpenRequest += 1
    conversationId.value = undefined
    messages.value = [welcomeMessage()]
    freshConversationRequested = explicit
    autonomy.value = defaultAutonomy()
  }

  function newConversation() {
    resetConversation(true)
  }

  async function refreshConversations(): Promise<AiConversationSummary[]> {
    const request = ++conversationListRequest
    const scope = conversationScopeVersion
    const projectId = selectedProjectId.value
    loadingConversations.value = true
    try {
      const loaded = await gateway.listAiConversations(projectId, 50)
      if (request !== conversationListRequest
        || scope !== conversationScopeVersion
        || projectId !== selectedProjectId.value) return []
      conversations.value = loaded
      return loaded
    } finally {
      if (request === conversationListRequest) loadingConversations.value = false
    }
  }

  async function openConversation(id: string) {
    const request = ++conversationOpenRequest
    loadingConversation.value = true
    try {
      const detail = await gateway.getAiConversation(id, 200)
      if (request !== conversationOpenRequest) return
      const projectChanged = selectedProjectId.value !== detail.conversation.projectId
      if (projectChanged) {
        selectedProjectId.value = detail.conversation.projectId
        conversationScopeVersion += 1
        pendingDrafts.value = []
      }
      conversationId.value = detail.conversation.id
      freshConversationRequested = false
      const restored = restoredMessages(detail)
      messages.value = restored.length ? restored : [welcomeMessage()]
      mergePendingDrafts(detail.messages.flatMap((message) => message.response?.drafts ?? []))
      if (gateway.getAiAutonomy) {
        autonomy.value = await gateway.getAiAutonomy(detail.conversation.id)
      } else {
        autonomy.value = detail.messages
          .map((message) => message.response?.autonomy)
          .filter((value): value is AiAutonomyView => Boolean(value))
          .at(-1) ?? defaultAutonomy()
      }
      if (projectChanged) {
        void refreshConversations().catch(() => undefined)
        void refreshDrafts().catch(() => undefined)
      }
    } finally {
      if (request === conversationOpenRequest) loadingConversation.value = false
    }
  }

  async function restoreLatestConversation(force = false) {
    const loaded = await refreshConversations()
    if (conversationId.value || (freshConversationRequested && !force)) return
    const latest = loaded[0]
    if (latest) await openConversation(latest.id)
  }

  async function refreshDrafts() {
    const projectId = selectedProjectId.value
    loadingDrafts.value = true
    try {
      const loaded = await gateway.listAiDrafts(projectId, 'pending_approval')
      if (projectId === selectedProjectId.value) pendingDrafts.value = loaded
    } finally {
      loadingDrafts.value = false
    }
  }

  async function send(prompt: string) {
    const value = prompt.trim()
    if (!value || busy.value) return
    messages.value.push({
      id: crypto.randomUUID(),
      role: 'user',
      content: value,
      createdAt: new Date().toISOString(),
    })
    const pendingId = crypto.randomUUID()
    messages.value.push({
      id: pendingId,
      role: 'assistant',
      content: '正在查询已授权的数据…',
      createdAt: new Date().toISOString(),
      pending: true,
    })
    sending.value = true
    try {
      const response = await gateway.aiTurn({
        conversationId: conversationId.value,
        projectId: selectedProjectId.value,
        message: value,
      })
      conversationId.value = response.conversationId
      autonomy.value = response.autonomy ?? defaultAutonomy()
      freshConversationRequested = false
      mergePendingDrafts(response.drafts)
      const index = messages.value.findIndex((message) => message.id === pendingId)
      messages.value[index] = {
        id: pendingId,
        role: 'assistant',
        content: response.content,
        citations: response.citations,
        toolRuns: response.toolRuns,
        drafts: response.drafts,
        trace: response.trace,
        createdAt: new Date().toISOString(),
      }
      void refreshConversations().catch(() => undefined)
    } catch (error) {
      const index = messages.value.findIndex((message) => message.id === pendingId)
      messages.value[index] = {
        id: pendingId,
        role: 'assistant',
        content: readableError(error),
        createdAt: new Date().toISOString(),
        error: true,
      }
    } finally {
      sending.value = false
    }
  }

  async function updateAutonomy(
    mode: AiAutonomyMode,
    options: { currentPassword?: string; declared?: boolean } = {},
  ): Promise<AiAutonomyView> {
    if (!conversationId.value) throw new Error('请先发送一条消息，再设置当前会话的 AI 授权')
    if (!gateway.setAiAutonomy) throw new Error('当前运行模式不支持会话授权设置')
    autonomyBusy.value = true
    try {
      const updated = await gateway.setAiAutonomy(conversationId.value, {
        mode,
        expectedRevision: autonomy.value.revision,
        currentPassword: options.currentPassword,
        declared: options.declared,
      })
      autonomy.value = updated
      return updated
    } finally {
      autonomyBusy.value = false
    }
  }

  async function decideDraft(
    draft: AiWriteDraft,
    decision: 'approve' | 'reject',
    statement?: string,
    currentPassword?: string,
  ): Promise<AiDraftDecisionResponse> {
    if (reviewingDraftIds.value.has(draft.id)) throw new Error('该草稿正在处理')
    const normalizedStatement = statement?.trim() || undefined
    if (decision === 'approve' && draft.requirement === 'researcher_signature' && !normalizedStatement) {
      throw new Error('批准科研测量前必须填写研究者签署声明')
    }
    if (decision === 'approve' && draft.requirement === 'reinforced_confirmation') {
      if (!normalizedStatement) throw new Error('加强确认前必须填写确认声明')
      if (reinforcedPasswordRequired.value && !currentPassword) {
        throw new Error('共享服务器加强确认前必须输入当前密码')
      }
    }
    reviewingDraftIds.value = new Set(reviewingDraftIds.value).add(draft.id)
    try {
      const input: AiDraftDecisionInput = {
        expectedRevision: draft.revision,
        decision,
        statement: normalizedStatement,
      }
      if (decision === 'approve'
        && draft.requirement === 'reinforced_confirmation'
        && reinforcedPasswordRequired.value) {
        input.currentPassword = currentPassword
      }
      const response = await gateway.decideAiDraft(draft.id, input)
      replaceDraft(response.draft)
      return response
    } finally {
      const next = new Set(reviewingDraftIds.value)
      next.delete(draft.id)
      reviewingDraftIds.value = next
    }
  }

  function draftBusy(draftId: string) {
    return reviewingDraftIds.value.has(draftId)
  }

  return {
    drawerOpen,
    contextTitle,
    contextRoute,
    selectedProjectId,
    selectedProject,
    conversationId,
    conversations,
    projects,
    projectsLoaded,
    messages,
    pendingDrafts,
    loadingDrafts,
    loadingConversations,
    loadingConversation,
    autonomy,
    autonomyBusy,
    busy,
    reinforcedPasswordRequired,
    open,
    setContext,
    loadProjects,
    selectProject,
    newConversation,
    refreshConversations,
    openConversation,
    restoreLatestConversation,
    refreshDrafts,
    send,
    updateAutonomy,
    decideDraft,
    draftBusy,
  }
}
