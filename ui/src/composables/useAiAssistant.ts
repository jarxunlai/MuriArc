import { computed, ref, watch } from 'vue'
import type {
  AiAutonomyMode,
  AiAutonomyView,
  AiComposerSource,
  AiConversationDetail,
  AiConversationListInput,
  AiConversationSummary,
  AiConversationUpdateInput,
  AiDraftDecisionInput,
  AiDraftDecisionResponse,
  AiMessage,
  AiSource,
  AiWriteDraft,
  ProjectSummary,
} from '@/domain/models'
import { aiImportApprovalBlockReason } from '@/domain/aiDrafts'
import { gateway } from '@/services/gateway'
import { currentAuthSession } from '@/services/projectContext'

const drawerOpen = ref(false)
const contextTitle = ref('MuriArc')
const contextRoute = ref('/cages')
const selectedProjectId = ref<string>()
const conversationId = ref<string>()
const currentConversation = ref<AiConversationSummary>()
const conversations = ref<AiConversationSummary[]>([])
const conversationFilter = ref<AiConversationListInput>({
  archive: 'active',
  limit: 100,
})
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
const updatingConversationIds = ref(new Set<string>())
const archivingSourceIds = ref(new Set<string>())
const releasingMessageSourceIds = ref(new Set<string>())
const releasedMessageSourceIds = ref(new Set<string>())
const sources = ref<AiComposerSource[]>([])
const sourceFiles = new Map<string, File>()
let conversationScopeVersion = 0
let conversationListRequest = 0
let conversationOpenRequest = 0
let freshConversationRequested = false
let assistantSessionVersion = 0
let conversationCreation: {
  scopeVersion: number
  projectId?: string
  promise: Promise<string>
} | undefined

const MAX_SOURCE_BYTES = 32 * 1024 * 1024
const MAX_SOURCES_PER_TURN = 10
const NEW_CONVERSATION_TITLE = '新对话'
const SUPPORTED_SOURCE_EXTENSIONS = new Set([
  'xlsx', 'csv', 'tsv', 'txt', 'md', 'json', 'pdf',
  'png', 'jpg', 'jpeg', 'tif', 'tiff',
])

function welcomeMessage(): AiMessage {
  return {
    id: 'welcome',
    role: 'assistant',
    content: '你好，我可以查询你有权限访问的动物、项目、实验、测量、样本和基因型等正式业务数据。实验室级会话严格只读；选择科研项目后，AI 来源导入仅支持实验测量，并且只会生成可审阅草稿。动物登记仍请使用正式导入流程。',
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

function resetAssistantStateForIdentityChange() {
  assistantSessionVersion += 1
  conversationScopeVersion += 1
  conversationListRequest += 1
  conversationOpenRequest += 1
  conversationCreation = undefined
  freshConversationRequested = false

  drawerOpen.value = false
  contextTitle.value = 'MuriArc'
  contextRoute.value = '/cages'
  selectedProjectId.value = undefined
  conversationId.value = undefined
  currentConversation.value = undefined
  conversations.value = []
  conversationFilter.value = { archive: 'active', limit: 100 }
  projects.value = []
  projectsLoaded.value = false
  messages.value = [welcomeMessage()]
  pendingDrafts.value = []
  sending.value = false
  loadingDrafts.value = false
  loadingConversations.value = false
  loadingConversation.value = false
  autonomy.value = defaultAutonomy()
  autonomyBusy.value = false
  reviewingDraftIds.value = new Set()
  updatingConversationIds.value = new Set()
  archivingSourceIds.value = new Set()
  releasingMessageSourceIds.value = new Set()
  releasedMessageSourceIds.value = new Set()
  sources.value = []
  sourceFiles.clear()
}

function isCurrentAssistantSession(version: number) {
  return version === assistantSessionVersion
}

function isCurrentConversationScope(
  sessionVersion: number,
  scopeVersion: number,
  expectedConversationId?: string,
) {
  return isCurrentAssistantSession(sessionVersion)
    && scopeVersion === conversationScopeVersion
    && (expectedConversationId === undefined || conversationId.value === expectedConversationId)
}

watch(
  () => currentAuthSession.value?.user.id,
  (userId, previousUserId) => {
    if (userId !== previousUserId) resetAssistantStateForIdentityChange()
  },
  { flush: 'sync' },
)

function readableError(error: unknown): string {
  if (error instanceof Error) return error.message
  if (typeof error === 'string') return error
  if (error && typeof error === 'object') {
    const value = error as Record<string, unknown>
    if (typeof value.message === 'string') return value.message
  }
  return 'AI 请求失败，请检查模型设置或稍后重试'
}

function sourceValidationError(file: File): string | undefined {
  const extension = file.name.split('.').at(-1)?.toLocaleLowerCase() ?? ''
  if (!SUPPORTED_SOURCE_EXTENSIONS.has(extension)) {
    return '不支持此格式。可使用 XLSX、CSV、TSV、TXT、MD、JSON、PDF、PNG、JPEG 或 TIFF'
  }
  if (file.size <= 0) return '文件为空'
  if (file.size > MAX_SOURCE_BYTES) return '单个文件不能超过 32 MiB'
  return undefined
}

function sourceExpiresInFuture(expiresAt?: string, now = Date.now()): boolean {
  if (!expiresAt) return false
  const expires = Date.parse(expiresAt)
  return Number.isFinite(expires) && expires > now
}

function isReadyAiSource(source: AiSource, now = Date.now()): boolean {
  return source.status === 'ready' && sourceExpiresInFuture(source.expiresAt, now)
}

function isUsableComposerSource(source: AiComposerSource, now = Date.now()): boolean {
  return source.status === 'ready'
    && Boolean(source.sourceId)
    && sourceExpiresInFuture(source.expiresAt, now)
}

function composerSource(source: AiSource): AiComposerSource {
  return {
    clientId: source.id,
    sourceId: source.id,
    projectId: source.projectId,
    fileName: source.fileName,
    mediaType: source.mediaType,
    sizeBytes: source.sizeBytes,
    status: source.status,
    revision: source.revision,
    expiresAt: source.expiresAt,
    error: source.status === 'failed'
      ? '服务端无法处理该文件'
      : source.status === 'expired'
        ? '文件已过期，请重新上传'
        : source.status === 'archived'
          ? '文件已归档，不再作为待发送来源'
          : undefined,
  }
}

function sortConversations(items: AiConversationSummary[]): AiConversationSummary[] {
  return [...items].sort((left, right) => {
    const pinned = Number(Boolean(right.pinnedAt)) - Number(Boolean(left.pinnedAt))
    return pinned || right.updatedAt.localeCompare(left.updatedAt)
  })
}

function filterConversations(
  items: AiConversationSummary[],
  input: AiConversationListInput,
): AiConversationSummary[] {
  const query = input.titleQuery?.trim().toLocaleLowerCase()
  return sortConversations(items)
    .filter((conversation) => !input.projectId || conversation.projectId === input.projectId)
    .filter((conversation) => {
      if ((input.archive ?? 'active') === 'all') return true
      return (input.archive === 'archived') === Boolean(conversation.archivedAt)
    })
    .filter((conversation) => !query || conversation.title.toLocaleLowerCase().includes(query))
    .slice(0, input.limit ?? 100)
}

function draftsForProject(drafts: AiWriteDraft[], projectId?: string): AiWriteDraft[] {
  if (!projectId) return []
  return drafts.filter((draft) => draft.projectId === projectId)
}

function replaceDraft(updated: AiWriteDraft) {
  const projectId = selectedProjectId.value
  if (!projectId || updated.projectId !== projectId) {
    pendingDrafts.value = []
    return
  }
  pendingDrafts.value = pendingDrafts.value
    .map((draft) => draft.id === updated.id ? updated : draft)
    .filter((draft) =>
      draft.projectId === projectId && draft.status === 'pending_approval')
  messages.value = messages.value.map((message) => ({
    ...message,
    drafts: message.drafts
      ?.filter((draft) => draft.projectId === projectId)
      .map((draft) => draft.id === updated.id ? updated : draft),
  }))
}

function mergePendingDrafts(additions: AiWriteDraft[]) {
  const projectId = selectedProjectId.value
  if (!projectId) {
    pendingDrafts.value = []
    return
  }
  const merged = new Map(
    pendingDrafts.value
      .filter((draft) => draft.projectId === projectId)
      .map((draft) => [draft.id, draft]),
  )
  for (const draft of draftsForProject(additions, projectId)) {
    if (draft.status === 'pending_approval') merged.set(draft.id, draft)
    else merged.delete(draft.id)
  }
  pendingDrafts.value = [...merged.values()].sort((left, right) =>
    right.createdAt.localeCompare(left.createdAt))
}

function restoredMessages(detail: AiConversationDetail, projectId?: string): AiMessage[] {
  return detail.messages.map((message) => {
    const response = message.response
    const scopedDrafts = draftsForProject(response?.drafts ?? [], projectId)
    return {
      id: message.id,
      role: message.role,
      content: message.content,
      createdAt: message.createdAt,
      citations: response?.citations,
      toolRuns: response?.toolRuns,
      drafts: scopedDrafts.length ? scopedDrafts : undefined,
      trace: response?.trace,
      incompleteReason: response?.incompleteReason,
      sources: message.sourceRefs?.map((source) => ({
        sourceId: source.sourceId,
        fileName: source.fileName,
        mediaType: source.mediaType ?? 'application/octet-stream',
        sizeBytes: source.sizeBytes,
        ...(releasedMessageSourceIds.value.has(source.sourceId)
          ? { released: true }
          : {}),
      })),
    }
  })
}

export function useAiAssistant() {
  const busy = computed(() =>
    loadingConversation.value
    || sending.value
    || messages.value.some((message) => message.pending))
  const sourceUploading = computed(() =>
    sources.value.some((source) => source.status === 'uploading' || source.status === 'staged'))
  const readySourceCount = computed(() => sources.value.filter((source) =>
    isUsableComposerSource(source)).length)
  const selectedProject = computed(() => projects.value.find((project) => project.id === selectedProjectId.value))
  const conversationArchived = computed(() => Boolean(currentConversation.value?.archivedAt))
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
    const sessionVersion = assistantSessionVersion
    const loaded = await gateway.listProjects()
    if (!isCurrentAssistantSession(sessionVersion)) return
    projects.value = loaded
    projectsLoaded.value = true
    if (selectedProjectId.value && !loaded.some((project) => project.id === selectedProjectId.value)) {
      clearSources()
      selectedProjectId.value = undefined
      resetConversation(false)
      void restoreLatestConversation(true).catch(() => undefined)
    }
  }

  async function selectProject(projectId?: string) {
    const normalized = projectId || undefined
    if (selectedProjectId.value === normalized
      && conversationFilter.value.projectId === normalized) return
    clearSources()
    selectedProjectId.value = normalized
    conversationFilter.value = { ...conversationFilter.value, projectId: normalized }
    pendingDrafts.value = []
    resetConversation(false)
    await Promise.all([
      restoreLatestConversation(true),
      refreshDrafts(),
    ])
  }

  function resetConversation(explicit: boolean) {
    conversationScopeVersion += 1
    conversationCreation = undefined
    conversationOpenRequest += 1
    conversationId.value = undefined
    currentConversation.value = undefined
    messages.value = [welcomeMessage()]
    sending.value = false
    loadingConversation.value = false
    autonomyBusy.value = false
    freshConversationRequested = explicit
    autonomy.value = defaultAutonomy()
  }

  function newConversation() {
    clearSources()
    resetConversation(true)
  }

  function clearSources() {
    sources.value = []
    sourceFiles.clear()
  }

  async function refreshConversations(
    nextFilter?: Partial<AiConversationListInput>,
  ): Promise<AiConversationSummary[]> {
    if (nextFilter) {
      conversationFilter.value = {
        ...conversationFilter.value,
        ...nextFilter,
        limit: nextFilter.limit ?? conversationFilter.value.limit ?? 100,
      }
    }
    const request = ++conversationListRequest
    const sessionVersion = assistantSessionVersion
    const filter = { ...conversationFilter.value }
    loadingConversations.value = true
    try {
      const loaded = gateway.queryAiConversations
        ? await gateway.queryAiConversations(filter)
        : filterConversations(
            await gateway.listAiConversations(filter.projectId, filter.limit ?? 100),
            filter,
          )
      if (request !== conversationListRequest || !isCurrentAssistantSession(sessionVersion)) return []
      conversations.value = sortConversations(loaded)
      const active = conversations.value.find((conversation) =>
        conversation.id === conversationId.value)
      if (active) currentConversation.value = active
      return conversations.value
    } finally {
      if (request === conversationListRequest && isCurrentAssistantSession(sessionVersion)) {
        loadingConversations.value = false
      }
    }
  }

  function setConversationFilter(nextFilter: Partial<AiConversationListInput>) {
    return refreshConversations(nextFilter)
  }

  async function openConversation(id: string) {
    const sessionVersion = assistantSessionVersion
    if (id !== conversationId.value) clearSources()
    conversationScopeVersion += 1
    const scopeVersion = conversationScopeVersion
    conversationCreation = undefined
    const request = ++conversationOpenRequest
    messages.value = messages.value.map((message) => message.pending
      ? {
          ...message,
          content: '会话切换已停止等待上一轮结果。',
          pending: false,
          error: true,
        }
      : message)
    sending.value = false
    autonomyBusy.value = false
    loadingConversation.value = true
    try {
      const detail = await gateway.getAiConversation(id, 200)
      if (request !== conversationOpenRequest
        || !isCurrentConversationScope(sessionVersion, scopeVersion)) return
      clearSources()
      const projectChanged = selectedProjectId.value !== detail.conversation.projectId
      if (projectChanged) {
        selectedProjectId.value = detail.conversation.projectId
        pendingDrafts.value = []
      }
      conversationId.value = detail.conversation.id
      currentConversation.value = detail.conversation
      freshConversationRequested = false
      const restored = restoredMessages(detail, detail.conversation.projectId)
      messages.value = restored.length ? restored : [welcomeMessage()]
      mergePendingDrafts(detail.messages.flatMap((message) => message.response?.drafts ?? []))
      if (gateway.listAiSources && !detail.conversation.archivedAt) {
        const submittedSourceIds = new Set(detail.messages.flatMap((message) =>
          message.sourceRefs?.map((source) => source.sourceId) ?? []))
        const available = await gateway.listAiSources({
          conversationId: detail.conversation.id,
          projectId: detail.conversation.projectId,
        })
        if (request !== conversationOpenRequest
          || !isCurrentConversationScope(
            sessionVersion,
            scopeVersion,
            detail.conversation.id,
          )) return
        sources.value = available
          .filter((source) =>
            isReadyAiSource(source) && !submittedSourceIds.has(source.id))
          .map(composerSource)
      }
      if (gateway.getAiAutonomy) {
        const loadedAutonomy = await gateway.getAiAutonomy(detail.conversation.id)
        if (request !== conversationOpenRequest
          || !isCurrentConversationScope(
            sessionVersion,
            scopeVersion,
            detail.conversation.id,
          )) return
        autonomy.value = loadedAutonomy
      } else {
        if (request !== conversationOpenRequest
          || !isCurrentConversationScope(
            sessionVersion,
            scopeVersion,
            detail.conversation.id,
          )) return
        autonomy.value = detail.messages
          .map((message) => message.response?.autonomy)
          .filter((value): value is AiAutonomyView => Boolean(value))
          .at(-1) ?? defaultAutonomy()
      }
      if (projectChanged) {
        void refreshDrafts().catch(() => undefined)
      }
    } finally {
      if (request === conversationOpenRequest
        && isCurrentConversationScope(sessionVersion, scopeVersion)) {
        loadingConversation.value = false
      }
    }
  }

  async function restoreLatestConversation(force = false) {
    const loaded = await refreshConversations()
    if (conversationId.value || (freshConversationRequested && !force)) return
    const latest = selectedProjectId.value
      ? loaded.find((conversation) => conversation.projectId === selectedProjectId.value)
      : loaded[0]
    if (latest) await openConversation(latest.id)
  }

  async function updateConversation(
    summary: AiConversationSummary,
    input: Omit<AiConversationUpdateInput, 'expectedRevision'>,
  ): Promise<AiConversationSummary> {
    if (!gateway.updateAiConversation) throw new Error('当前运行模式尚未启用会话管理')
    if (updatingConversationIds.value.has(summary.id)) throw new Error('该会话正在更新')
    const sessionVersion = assistantSessionVersion
    updatingConversationIds.value = new Set(updatingConversationIds.value).add(summary.id)
    try {
      const updated = await gateway.updateAiConversation(summary.id, {
        ...input,
        expectedRevision: summary.revision,
      })
      if (!isCurrentAssistantSession(sessionVersion)) return updated
      const merged = conversations.value.map((conversation) =>
        conversation.id === updated.id ? updated : conversation)
      conversations.value = filterConversations(merged, conversationFilter.value)
      if (summary.id === conversationId.value) {
        currentConversation.value = updated
        if (updated.archivedAt) clearSources()
      }
      return updated
    } finally {
      if (isCurrentAssistantSession(sessionVersion)) {
        const next = new Set(updatingConversationIds.value)
        next.delete(summary.id)
        updatingConversationIds.value = next
      }
    }
  }

  function conversationBusy(id: string) {
    return updatingConversationIds.value.has(id)
  }

  async function refreshDrafts() {
    const sessionVersion = assistantSessionVersion
    const projectId = selectedProjectId.value
    if (!projectId) {
      pendingDrafts.value = []
      loadingDrafts.value = false
      return
    }
    loadingDrafts.value = true
    try {
      const loaded = await gateway.listAiDrafts(projectId, 'pending_approval')
      if (isCurrentAssistantSession(sessionVersion)
        && projectId === selectedProjectId.value) {
        pendingDrafts.value = draftsForProject(loaded, projectId)
      }
    } finally {
      if (isCurrentAssistantSession(sessionVersion)) loadingDrafts.value = false
    }
  }

  async function ensureConversation(): Promise<string> {
    if (conversationArchived.value) throw new Error('已归档会话为只读，请先恢复会话')
    if (conversationId.value) return conversationId.value
    const scopeVersion = conversationScopeVersion
    const projectId = selectedProjectId.value
    if (conversationCreation
      && conversationCreation.scopeVersion === scopeVersion
      && conversationCreation.projectId === projectId) {
      return conversationCreation.promise
    }

    let promise!: Promise<string>
    promise = gateway.createAiConversation({
      projectId,
      title: NEW_CONVERSATION_TITLE,
    }).then((created) => {
      if (conversationScopeVersion !== scopeVersion
        || selectedProjectId.value !== projectId
        || (conversationId.value && conversationId.value !== created.id)) {
        throw new Error('对话范围已变化，请在当前对话重新选择文件')
      }
      conversationId.value = created.id
      currentConversation.value = created
      freshConversationRequested = false
      const merged = conversations.value.filter((item) => item.id !== created.id)
      conversations.value = filterConversations([created, ...merged], conversationFilter.value)
      return created.id
    }).finally(() => {
      if (conversationCreation?.promise === promise) conversationCreation = undefined
    })
    conversationCreation = { scopeVersion, projectId, promise }
    return promise
  }

  async function uploadSource(clientId: string, file: File) {
    if (!gateway.uploadAiSource) {
      sources.value = sources.value.map((source) => source.clientId === clientId
        ? {
            ...source,
            status: 'error',
            error: '当前运行模式尚未启用对话文件上传',
            retryable: true,
          }
        : source)
      return
    }
    try {
      const sessionVersion = assistantSessionVersion
      const scopeVersion = conversationScopeVersion
      const projectId = selectedProjectId.value
      const boundConversationId = await ensureConversation()
      if (scopeVersion !== conversationScopeVersion || projectId !== selectedProjectId.value) {
        throw new Error('对话范围已变化，请在当前对话重新选择文件')
      }
      const uploaded = await gateway.uploadAiSource({
        file,
        conversationId: boundConversationId,
        projectId,
      })
      const current = sources.value.find((source) => source.clientId === clientId)
      if (!isCurrentAssistantSession(sessionVersion)
        || !current
        || scopeVersion !== conversationScopeVersion
        || projectId !== selectedProjectId.value
        || conversationId.value !== boundConversationId) {
        await gateway.deleteAiSource?.(uploaded.id).catch(() => undefined)
        return
      }
      sources.value = sources.value.map((source) => source.clientId === clientId
        ? {
            ...source,
            ...composerSource(uploaded),
            clientId,
            retryable: uploaded.status === 'failed' ? true : undefined,
          }
        : source)
    } catch (error) {
      if (!sources.value.some((source) => source.clientId === clientId)) return
      sources.value = sources.value.map((source) => source.clientId === clientId
        ? { ...source, status: 'error', error: readableError(error), retryable: true }
        : source)
    }
  }

  async function addFiles(files: File[]) {
    if (conversationArchived.value) return
    const available = Math.max(0, MAX_SOURCES_PER_TURN - sources.value.length)
    const accepted = files.slice(0, available)
    const overflow = files.slice(available)
    const uploads: Promise<void>[] = []
    for (const file of accepted) {
      const clientId = crypto.randomUUID()
      const error = sourceValidationError(file)
      const source: AiComposerSource = {
        clientId,
        fileName: file.name,
        mediaType: file.type || 'application/octet-stream',
        sizeBytes: file.size,
        status: error ? 'error' : 'uploading',
        error,
      }
      sources.value = [...sources.value, source]
      if (!error) {
        sourceFiles.set(clientId, file)
        uploads.push(uploadSource(clientId, file))
      }
    }
    for (const file of overflow) {
      sources.value = [...sources.value, {
        clientId: crypto.randomUUID(),
        fileName: file.name,
        mediaType: file.type || 'application/octet-stream',
        sizeBytes: file.size,
        status: 'error',
        error: `每轮最多选择 ${MAX_SOURCES_PER_TURN} 个文件`,
      }]
    }
    await Promise.all(uploads)
  }

  async function removeSource(clientId: string) {
    if (conversationArchived.value) throw new Error('已归档会话为只读，请先恢复会话')
    const sessionVersion = assistantSessionVersion
    const source = sources.value.find((candidate) => candidate.clientId === clientId)
    if (source?.sourceId && gateway.deleteAiSource) {
      await gateway.deleteAiSource(source.sourceId)
    }
    if (!isCurrentAssistantSession(sessionVersion)) return
    sources.value = sources.value.filter((candidate) => candidate.clientId !== clientId)
    sourceFiles.delete(clientId)
  }

  async function archiveSource(clientId: string) {
    if (conversationArchived.value) throw new Error('已归档会话为只读，请先恢复会话')
    const sessionVersion = assistantSessionVersion
    const source = sources.value.find((candidate) => candidate.clientId === clientId)
    const projectId = source?.projectId ?? selectedProjectId.value
    if (!source?.sourceId
      || source.status !== 'ready'
      || !source.revision
      || !projectId
      || !gateway.archiveAiSource) {
      throw new Error('只有项目内已就绪的暂存文件可以归档')
    }
    archivingSourceIds.value = new Set(archivingSourceIds.value).add(clientId)
    try {
      const archived = await gateway.archiveAiSource(source.sourceId, {
        projectId,
        expectedRevision: source.revision,
      })
      if (!isCurrentAssistantSession(sessionVersion)) return archived
      if (archived.status !== 'archived') {
        throw new Error('来源归档未完成，请刷新后重试')
      }
      sources.value = sources.value.filter((candidate) => candidate.clientId !== clientId)
      sourceFiles.delete(clientId)
      return archived
    } finally {
      if (isCurrentAssistantSession(sessionVersion)) {
        const next = new Set(archivingSourceIds.value)
        next.delete(clientId)
        archivingSourceIds.value = next
      }
    }
  }

  function sourceArchiving(clientId: string) {
    return archivingSourceIds.value.has(clientId)
  }

  async function retrySource(clientId: string) {
    if (conversationArchived.value) throw new Error('已归档会话为只读，请先恢复会话')
    const file = sourceFiles.get(clientId)
    if (!file) throw new Error('无法重试此文件，请移除后重新选择')
    sources.value = sources.value.map((source) => source.clientId === clientId
      ? { ...source, status: 'uploading', error: undefined, retryable: undefined }
      : source)
    await uploadSource(clientId, file)
  }

  async function discardSources() {
    const sessionVersion = assistantSessionVersion
    const sourceIds = sources.value.flatMap((source) => source.sourceId ? [source.sourceId] : [])
    if (gateway.deleteAiSource) {
      await Promise.all(sourceIds.map((sourceId) =>
        gateway.deleteAiSource?.(sourceId).catch(() => undefined)))
    }
    if (isCurrentAssistantSession(sessionVersion)) clearSources()
  }

  async function releaseMessageSource(messageId: string, sourceId: string) {
    const source = messages.value
      .find((message) => message.id === messageId)
      ?.sources?.find((candidate) => candidate.sourceId === sourceId)
    if (!source) throw new Error('找不到这条消息中的暂存文件')
    if (source.released || releasedMessageSourceIds.value.has(sourceId)) return
    if (!gateway.deleteAiSource) throw new Error('当前运行模式尚未启用暂存文件释放')
    if (releasingMessageSourceIds.value.has(sourceId)) throw new Error('暂存文件正在释放')

    const sessionVersion = assistantSessionVersion
    releasingMessageSourceIds.value = new Set(releasingMessageSourceIds.value).add(sourceId)
    try {
      await gateway.deleteAiSource(sourceId)
      if (!isCurrentAssistantSession(sessionVersion)) return
      releasedMessageSourceIds.value = new Set(releasedMessageSourceIds.value).add(sourceId)
      messages.value = messages.value.map((message) => ({
        ...message,
        sources: message.sources?.map((candidate) =>
          candidate.sourceId === sourceId
            ? { ...candidate, released: true }
            : candidate),
      }))
    } finally {
      if (isCurrentAssistantSession(sessionVersion)) {
        const next = new Set(releasingMessageSourceIds.value)
        next.delete(sourceId)
        releasingMessageSourceIds.value = next
      }
    }
  }

  function messageSourceReleasing(sourceId: string) {
    return releasingMessageSourceIds.value.has(sourceId)
  }

  async function send(prompt: string) {
    if (conversationArchived.value || loadingConversation.value) return
    const sessionVersion = assistantSessionVersion
    const scopeVersion = conversationScopeVersion
    const targetConversationId = conversationId.value
    const targetProjectId = selectedProjectId.value
    const now = Date.now()
    sources.value = sources.value.map((source) =>
      source.status === 'ready' && !sourceExpiresInFuture(source.expiresAt, now)
        ? {
            ...source,
            status: 'expired',
            error: '文件已过期，请重新上传',
            retryable: false,
          }
        : source)
    const readySources = sources.value.filter((source) => isUsableComposerSource(source, now))
    const value = prompt.trim() || (readySources.length
      ? '请分析我提供的文件，并说明可识别的数据、错误与下一步录入预览。'
      : '')
    if (!value || busy.value || sourceUploading.value) return
    const messageSources = readySources.map((source) => ({
      sourceId: source.sourceId,
      fileName: source.fileName,
      mediaType: source.mediaType,
      sizeBytes: source.sizeBytes,
    }))
    messages.value.push({
      id: crypto.randomUUID(),
      role: 'user',
      content: value,
      createdAt: new Date().toISOString(),
      sources: messageSources,
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
        conversationId: targetConversationId,
        projectId: targetProjectId,
        message: value,
        ...(readySources.length
          ? { sourceRefs: readySources.flatMap((source) => source.sourceId ? [source.sourceId] : []) }
          : {}),
      })
      if (!isCurrentAssistantSession(sessionVersion)
        || scopeVersion !== conversationScopeVersion) return
      conversationId.value = response.conversationId
      autonomy.value = response.autonomy ?? defaultAutonomy()
      freshConversationRequested = false
      const responseDrafts = draftsForProject(response.drafts, selectedProjectId.value)
      mergePendingDrafts(responseDrafts)
      const index = messages.value.findIndex((message) => message.id === pendingId)
      messages.value[index] = {
        id: pendingId,
        role: 'assistant',
        content: response.content,
        citations: response.citations,
        toolRuns: response.toolRuns,
        drafts: responseDrafts.length ? responseDrafts : undefined,
        trace: response.trace,
        incompleteReason: response.incompleteReason,
        createdAt: new Date().toISOString(),
      }
      const submitted = new Set(readySources.map((source) => source.clientId))
      sources.value = sources.value.filter((source) => !submitted.has(source.clientId))
      for (const clientId of submitted) sourceFiles.delete(clientId)
      void refreshConversations().catch(() => undefined)
    } catch (error) {
      if (!isCurrentAssistantSession(sessionVersion)
        || scopeVersion !== conversationScopeVersion) return
      const index = messages.value.findIndex((message) => message.id === pendingId)
      messages.value[index] = {
        id: pendingId,
        role: 'assistant',
        content: readableError(error),
        createdAt: new Date().toISOString(),
        error: true,
      }
    } finally {
      if (isCurrentAssistantSession(sessionVersion)
        && scopeVersion === conversationScopeVersion) sending.value = false
    }
  }

  async function updateAutonomy(
    mode: AiAutonomyMode,
    options: { currentPassword?: string; declared?: boolean } = {},
  ): Promise<AiAutonomyView> {
    if (conversationArchived.value) throw new Error('已归档会话为只读，请先恢复会话')
    if (loadingConversation.value) throw new Error('正在切换会话，请稍后再调整 AI 授权')
    if (!conversationId.value) throw new Error('请先发送一条消息，再设置当前会话的 AI 授权')
    if (!gateway.setAiAutonomy) throw new Error('当前运行模式不支持会话授权设置')
    const sessionVersion = assistantSessionVersion
    const scopeVersion = conversationScopeVersion
    const targetConversationId = conversationId.value
    autonomyBusy.value = true
    try {
      const updated = await gateway.setAiAutonomy(targetConversationId, {
        mode,
        expectedRevision: autonomy.value.revision,
        currentPassword: options.currentPassword,
        declared: options.declared,
      })
      if (!isCurrentConversationScope(
        sessionVersion,
        scopeVersion,
        targetConversationId,
      )) return updated
      autonomy.value = updated
      return updated
    } finally {
      if (isCurrentConversationScope(
        sessionVersion,
        scopeVersion,
        targetConversationId,
      )) autonomyBusy.value = false
    }
  }

  async function decideDraft(
    draft: AiWriteDraft,
    decision: 'approve' | 'reject',
    statement?: string,
    currentPassword?: string,
  ): Promise<AiDraftDecisionResponse> {
    if (conversationArchived.value) throw new Error('已归档会话为只读，请先恢复会话')
    if (!selectedProjectId.value || draft.projectId !== selectedProjectId.value) {
      throw new Error('只能审批当前科研项目内的 AI 写入草稿')
    }
    if (reviewingDraftIds.value.has(draft.id)) throw new Error('该草稿正在处理')
    if (decision === 'approve') {
      const importBlockReason = aiImportApprovalBlockReason(draft, selectedProjectId.value)
      if (importBlockReason) throw new Error(importBlockReason)
    }
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
    const sessionVersion = assistantSessionVersion
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
      if (isCurrentAssistantSession(sessionVersion)) replaceDraft(response.draft)
      return response
    } finally {
      if (isCurrentAssistantSession(sessionVersion)) {
        const next = new Set(reviewingDraftIds.value)
        next.delete(draft.id)
        reviewingDraftIds.value = next
      }
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
    conversationArchived,
    conversations,
    conversationFilter,
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
    sources,
    sourceUploading,
    readySourceCount,
    reinforcedPasswordRequired,
    open,
    setContext,
    loadProjects,
    selectProject,
    newConversation,
    refreshConversations,
    setConversationFilter,
    openConversation,
    updateConversation,
    conversationBusy,
    restoreLatestConversation,
    refreshDrafts,
    addFiles,
    removeSource,
    archiveSource,
    sourceArchiving,
    retrySource,
    discardSources,
    releaseMessageSource,
    messageSourceReleasing,
    send,
    updateAutonomy,
    decideDraft,
    draftBusy,
  }
}
