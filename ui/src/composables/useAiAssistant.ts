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
import {
  gateway,
  type AiModelDefaultsView,
  type AiModelProfileView,
} from '@/services/gateway'

const drawerOpen = ref(false)
const contextTitle = ref('MuriArc')
const contextRoute = ref('/cages')
const selectedProjectId = ref<string>()
const conversationId = ref<string>()
const currentConversation = ref<AiConversationSummary>()
const conversations = ref<AiConversationSummary[]>([])
const projects = ref<ProjectSummary[]>([])
const projectsLoaded = ref(false)
const messages = ref<AiMessage[]>([welcomeMessage()])
const pendingDrafts = ref<AiWriteDraft[]>([])
const conversationDrafts = ref<AiWriteDraft[]>([])
const composerDraft = ref('')
const modelProfiles = ref<AiModelProfileView[]>([])
const modelDefaults = ref<AiModelDefaultsView>({ revision: 0 })
const modelsLoaded = ref(false)
const selectedModelProfileId = ref<string>()
const selectedModelWasExplicit = ref(false)
const requestedMode = ref<AiAutonomyMode>('full')
const sending = ref(false)
const startingConversation = ref(false)
const loadingModels = ref(false)
const loadingDrafts = ref(false)
const loadingConversations = ref(false)
const loadingConversation = ref(false)
const autonomy = ref<AiAutonomyView>(defaultAutonomy())
const autonomyBusy = ref(false)
const reviewingDraftIds = ref(new Set<string>())
let conversationScopeVersion = 0
let conversationListRequest = 0
let conversationOpenRequest = 0
let modelListRequest = 0
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
  const replaceIn = (drafts: AiWriteDraft[]) => drafts
    .map((draft) => draft.id === updated.id ? updated : draft)
    .filter((draft) => draft.status === 'pending_approval')
  pendingDrafts.value = replaceIn(pendingDrafts.value)
  conversationDrafts.value = replaceIn(conversationDrafts.value)
  messages.value = messages.value.map((message) => ({
    ...message,
    drafts: message.drafts?.map((draft) => draft.id === updated.id ? updated : draft),
  }))
}

function mergeConversationDrafts(additions: AiWriteDraft[]) {
  const merged = new Map(conversationDrafts.value.map((draft) => [draft.id, draft]))
  for (const draft of additions) {
    if (draft.status === 'pending_approval') merged.set(draft.id, draft)
    else merged.delete(draft.id)
  }
  conversationDrafts.value = [...merged.values()].sort((left, right) =>
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
  const busy = computed(() =>
    startingConversation.value
    || sending.value
    || messages.value.some((message) => message.pending))
  const selectedProject = computed(() =>
    projects.value.find((project) => project.id === selectedProjectId.value))
  const selectedModel = computed(() =>
    modelProfiles.value.find((profile) => profile.id === selectedModelProfileId.value))
  const reinforcedPasswordRequired = computed(() => gateway.mode === 'remote')
  const hasPersistedMessages = computed(() =>
    Boolean(conversationId.value)
    && messages.value.some((message) => message.id !== 'welcome' && !message.pending))

  const modelOptions = computed(() => {
    const conversation = currentConversation.value
    const options = modelProfiles.value.map((profile) => {
      const isBoundConversationProfile = conversation?.modelProfileId === profile.id
      const name = isBoundConversationProfile
        ? (conversation.modelProfileName ?? profile.name)
        : profile.name
      const modelId = isBoundConversationProfile
        ? (conversation.modelId ?? profile.modelId)
        : profile.modelId
      const version = isBoundConversationProfile
        ? conversation.modelProfileVersion
        : profile.currentVersion
      return {
        label: `${name} · ${modelId}${version ? ` · v${version}` : ''}${profile.archivedAt ? '（已归档）' : ''}`,
        value: profile.id,
        disabled: Boolean(profile.archivedAt),
      }
    })
    if (conversation?.modelProfileId
      && !options.some((option) => option.value === conversation.modelProfileId)) {
      options.push({
        label: `${conversation.modelProfileName ?? '历史模型'}${conversation.modelId ? ` · ${conversation.modelId}` : ''}${conversation.modelProfileVersion ? ` · v${conversation.modelProfileVersion}` : ''}（不可用）`,
        value: conversation.modelProfileId,
        disabled: true,
      })
    }
    return options
  })

  const conversationReadOnlyReason = computed(() => {
    const conversation = currentConversation.value
    if (!conversation) return undefined
    const reason = conversation.readOnlyReason
    if (reason === 'legacy_model_unknown') return '旧会话没有可识别的模型绑定，只能查看历史内容'
    if (reason === 'model_archived') return '该会话使用的模型已归档，只能查看历史内容'
    if (reason === 'model_unavailable') return '该会话使用的模型版本当前不可用，只能查看历史内容'
    if (!conversation.modelProfileId) return '旧会话没有可识别的模型绑定，只能查看历史内容'
    const profile = modelProfiles.value.find((item) => item.id === conversation.modelProfileId)
    if (!profile) return '该会话使用的模型版本当前不可用，只能查看历史内容'
    if (profile.archivedAt) return '该会话使用的模型已归档，只能查看历史内容'
    if (conversation.readOnly) return '该历史会话当前只能查看，不能继续发送'
    return undefined
  })

  const composerDisabledReason = computed(() => {
    if (loadingModels.value) return '正在读取模型配置…'
    if (conversationReadOnlyReason.value) return conversationReadOnlyReason.value
    if (!selectedModelProfileId.value) return '请先明确选择一个可用的对话模型'
    const profile = selectedModel.value
    if (!profile) return '所选模型当前不可用，请重新选择'
    if (profile.archivedAt) return '所选模型已归档，请重新选择'
    return undefined
  })

  const fullActivationRequired = computed(() =>
    !conversationId.value && requestedMode.value === 'full')

  function setContext(title: string, route: string) {
    contextTitle.value = title
    contextRoute.value = route
  }

  function defaultConversationProfileId() {
    const profileId = modelDefaults.value.defaultConversationProfileId ?? undefined
    return modelProfiles.value.some((profile) =>
      profile.id === profileId && !profile.archivedAt)
      ? profileId
      : undefined
  }

  function resetConversation(
    explicit: boolean,
    profileId: string | undefined = defaultConversationProfileId(),
    explicitModelSelection = false,
  ) {
    conversationOpenRequest += 1
    conversationId.value = undefined
    currentConversation.value = undefined
    messages.value = [welcomeMessage()]
    conversationDrafts.value = []
    selectedModelProfileId.value = profileId
    selectedModelWasExplicit.value = explicitModelSelection
    requestedMode.value = 'full'
    freshConversationRequested = explicit
    autonomy.value = defaultAutonomy()
  }

  function open(title?: string, route?: string) {
    if (title && route) setContext(title, route)
    drawerOpen.value = true
    void loadModels(true).catch(() => undefined)
    void loadProjects().catch(() => undefined)
    void refreshDrafts().catch(() => undefined)
    void restoreLatestConversation().catch(() => undefined)
  }

  async function loadModels(force = false) {
    if (modelsLoaded.value && !force) return
    if (!gateway.listAiModelProfiles || !gateway.getAiModelDefaults) {
      throw new Error('当前运行模式不支持模型选择')
    }
    const request = ++modelListRequest
    loadingModels.value = true
    try {
      const [profiles, defaults] = await Promise.all([
        gateway.listAiModelProfiles(true),
        gateway.getAiModelDefaults(),
      ])
      if (request !== modelListRequest) return
      modelProfiles.value = profiles
      modelDefaults.value = defaults
      modelsLoaded.value = true
      if (!currentConversation.value) {
        const selectedIsActive = profiles.some((profile) =>
          profile.id === selectedModelProfileId.value && !profile.archivedAt)
        if (!selectedModelWasExplicit.value || !selectedIsActive) {
          selectedModelProfileId.value = defaultConversationProfileId()
          selectedModelWasExplicit.value = false
        }
      }
    } finally {
      if (request === modelListRequest) loadingModels.value = false
    }
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

  function newConversation() {
    resetConversation(true)
  }

  function modelSwitchNeedsConfirmation(profileId: string) {
    return profileId !== selectedModelProfileId.value && hasPersistedMessages.value
  }

  function selectModel(profileId: string, confirmed = false) {
    const profile = modelProfiles.value.find((item) =>
      item.id === profileId && !item.archivedAt)
    if (!profile) throw new Error('所选模型当前不可用')
    if (profileId === selectedModelProfileId.value) return true
    if (modelSwitchNeedsConfirmation(profileId) && !confirmed) return false
    if (conversationId.value) {
      resetConversation(true, profileId, true)
    } else {
      selectedModelProfileId.value = profileId
      selectedModelWasExplicit.value = true
    }
    return true
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
      await loadModels()
      const detail = await gateway.getAiConversation(id, 200)
      if (request !== conversationOpenRequest) return
      const restoredAutonomy = gateway.getAiAutonomy
        ? await gateway.getAiAutonomy(detail.conversation.id)
        : detail.messages
          .map((message) => message.response?.autonomy)
          .filter((value): value is AiAutonomyView => Boolean(value))
          .at(-1) ?? defaultAutonomy()
      if (request !== conversationOpenRequest) return
      const projectChanged = selectedProjectId.value !== detail.conversation.projectId
      if (projectChanged) {
        selectedProjectId.value = detail.conversation.projectId
        conversationScopeVersion += 1
        pendingDrafts.value = []
      }
      currentConversation.value = detail.conversation
      conversationId.value = detail.conversation.id
      selectedModelProfileId.value = detail.conversation.modelProfileId
      selectedModelWasExplicit.value = false
      freshConversationRequested = false
      const restored = restoredMessages(detail)
      messages.value = restored.length ? restored : [welcomeMessage()]
      conversationDrafts.value = []
      mergeConversationDrafts(detail.messages.flatMap((message) =>
        message.response?.drafts ?? []))
      autonomy.value = restoredAutonomy
      requestedMode.value = autonomy.value.mode
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

  async function startConversation(options: {
    fullConfirmed?: boolean
    currentPassword?: string
  } = {}) {
    if (conversationId.value) return
    const reason = composerDisabledReason.value
    if (reason) throw new Error(reason)
    if (requestedMode.value === 'full' && !options.fullConfirmed) {
      throw new Error('发送前请确认启用本次新会话的 Full 模式')
    }
    if (requestedMode.value === 'full'
      && reinforcedPasswordRequired.value
      && !options.currentPassword) {
      throw new Error('共享服务器启用 Full 前必须输入当前密码')
    }
    const request = ++conversationOpenRequest
    const projectId = selectedProjectId.value
    const modelProfileId = selectedModelProfileId.value
    const mode = requestedMode.value
    startingConversation.value = true
    try {
      const started = await gateway.startAiConversation({
        projectId,
        modelProfileId,
        requestedMode: mode,
        ...(reinforcedPasswordRequired.value && options.currentPassword
          ? { currentPassword: options.currentPassword }
          : {}),
      })
      if (request !== conversationOpenRequest) {
        throw new Error('会话范围已变化，请重新发送')
      }
      conversationId.value = started.conversation.id
      currentConversation.value = started.conversation
      selectedModelProfileId.value = started.conversation.modelProfileId
        ?? selectedModelProfileId.value
      autonomy.value = started.autonomy
      freshConversationRequested = false
    } finally {
      startingConversation.value = false
    }
  }

  async function send(
    prompt: string,
    startOptions: { fullConfirmed?: boolean; currentPassword?: string } = {},
  ) {
    const value = prompt.trim()
    if (!value || busy.value) return
    if (!conversationId.value) await startConversation(startOptions)

    const turnStateVersion = conversationOpenRequest
    const turnConversationId = conversationId.value
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
    if (composerDraft.value.trim() === value) composerDraft.value = ''
    sending.value = true
    try {
      const response = await gateway.aiTurn({
        conversationId: turnConversationId,
        projectId: selectedProjectId.value,
        message: value,
      })
      if (turnStateVersion !== conversationOpenRequest
        || turnConversationId !== conversationId.value) return
      conversationId.value = response.conversationId
      autonomy.value = response.autonomy ?? autonomy.value
      freshConversationRequested = false
      mergeConversationDrafts(response.drafts)
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
      if (turnStateVersion !== conversationOpenRequest
        || turnConversationId !== conversationId.value) return
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

  async function requestMode(
    mode: AiAutonomyMode,
    options: { currentPassword?: string } = {},
  ) {
    if (!conversationId.value) {
      requestedMode.value = mode
      return
    }
    await updateAutonomy(mode, options)
  }

  async function updateAutonomy(
    mode: AiAutonomyMode,
    options: { currentPassword?: string } = {},
  ): Promise<AiAutonomyView> {
    if (!conversationId.value) throw new Error('请先开始会话，再更新当前会话的 AI 授权')
    if (!gateway.setAiAutonomy) throw new Error('当前运行模式不支持会话授权设置')
    const stateVersion = conversationOpenRequest
    const activeConversationId = conversationId.value
    autonomyBusy.value = true
    try {
      const updated = await gateway.setAiAutonomy(activeConversationId, {
        mode,
        expectedRevision: autonomy.value.revision,
        currentPassword: options.currentPassword,
      })
      if (stateVersion === conversationOpenRequest
        && activeConversationId === conversationId.value) {
        autonomy.value = updated
        requestedMode.value = mode
      }
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
    currentConversation,
    conversations,
    projects,
    projectsLoaded,
    messages,
    pendingDrafts,
    conversationDrafts,
    composerDraft,
    modelProfiles,
    modelDefaults,
    modelsLoaded,
    selectedModelProfileId,
    selectedModel,
    modelOptions,
    loadingModels,
    requestedMode,
    startingConversation,
    loadingDrafts,
    loadingConversations,
    loadingConversation,
    autonomy,
    autonomyBusy,
    busy,
    hasPersistedMessages,
    conversationReadOnlyReason,
    composerDisabledReason,
    fullActivationRequired,
    reinforcedPasswordRequired,
    open,
    setContext,
    loadModels,
    loadProjects,
    selectProject,
    newConversation,
    modelSwitchNeedsConfirmation,
    selectModel,
    refreshConversations,
    openConversation,
    restoreLatestConversation,
    refreshDrafts,
    startConversation,
    send,
    requestMode,
    updateAutonomy,
    decideDraft,
    draftBusy,
  }
}
