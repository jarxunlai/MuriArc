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
import {
  gateway,
  type AiModelDefaultsView,
  type AiModelProfileView,
  type PrivateImageRecord,
} from '@/services/gateway'
import { currentAuthSession } from '@/services/projectContext'

export interface AiStagedImage {
  localId: string
  file: File
  previewUrl: string
  status: 'staged' | 'uploading' | 'ready' | 'error'
  uploaded?: PrivateImageRecord
  error?: string
}

const MAX_CHAT_IMAGES = 8
const MAX_CHAT_IMAGE_BYTES = 10 * 1024 * 1024
const CHAT_IMAGE_MEDIA_TYPES = new Set([
  'image/jpeg',
  'image/png',
  'image/webp',
  'image/gif',
])
const SOURCE_IMAGE_MEDIA_TYPES = new Set([
  'image/jpeg',
  'image/png',
  'image/tiff',
])
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
const conversationDrafts = ref<AiWriteDraft[]>([])
const composerDraft = ref('')
const modelProfiles = ref<AiModelProfileView[]>([])
const modelDefaults = ref<AiModelDefaultsView>({ revision: 0 })
const modelsLoaded = ref(false)
const selectedModelProfileId = ref<string>()
const selectedModelWasExplicit = ref(false)
const selectedVisionModelProfileId = ref<string>()
const stagedImages = ref<AiStagedImage[]>([])
const imageStageError = ref<string>()
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
const updatingConversationIds = ref(new Set<string>())
const archivingSourceIds = ref(new Set<string>())
const releasingMessageSourceIds = ref(new Set<string>())
const releasedMessageSourceIds = ref(new Set<string>())
const sources = ref<AiComposerSource[]>([])
const sourceFiles = new Map<string, File>()
let conversationScopeVersion = 0
let conversationListRequest = 0
let conversationOpenRequest = 0
let modelListRequest = 0
let freshConversationRequested = false
let assistantSessionVersion = 0

const MAX_SOURCE_BYTES = 32 * 1024 * 1024
const MAX_SOURCES_PER_TURN = 10
const SUPPORTED_SOURCE_EXTENSIONS = new Set([
  'xlsx', 'csv', 'tsv', 'txt', 'md', 'json', 'pdf',
  'png', 'jpg', 'jpeg', 'tif', 'tiff',
])
let imageComposerConsumers = 0
const retainedMessagePreviewUrls = new Map<string, string>()

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
  modelListRequest += 1
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
  conversationDrafts.value = []
  composerDraft.value = ''
  modelProfiles.value = []
  modelDefaults.value = { revision: 0 }
  modelsLoaded.value = false
  selectedModelProfileId.value = undefined
  selectedModelWasExplicit.value = false
  selectedVisionModelProfileId.value = undefined
  releaseStagedImages()
  releaseConversationPreviewUrls()
  imageStageError.value = undefined
  sending.value = false
  startingConversation.value = false
  loadingModels.value = false
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

function conversationTitleFromMessage(message: string): string {
  const normalized = message.replace(/[\p{Cc}\s]+/gu, ' ').trim()
  return Array.from(normalized).slice(0, 256).join('') || 'MuriArc AI conversation'
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

function isImageSource(source: AiComposerSource): boolean {
  const mediaType = source.mediaType.split(';', 1)[0]?.trim().toLowerCase() ?? ''
  if (SOURCE_IMAGE_MEDIA_TYPES.has(mediaType)) return true
  if (mediaType && mediaType !== 'application/octet-stream') return false
  const extension = source.fileName.split('.').at(-1)?.toLowerCase()
  return extension === 'png'
    || extension === 'jpg'
    || extension === 'jpeg'
    || extension === 'tif'
    || extension === 'tiff'
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

function createPreviewUrl(file: File): string {
  return typeof URL.createObjectURL === 'function' ? URL.createObjectURL(file) : ''
}

function revokePreviewUrl(value: string) {
  if (value && typeof URL.revokeObjectURL === 'function') URL.revokeObjectURL(value)
}

function resetUploadedImagesForNewConversation() {
  stagedImages.value = stagedImages.value.map((image) => ({
    ...image,
    status: 'staged',
    uploaded: undefined,
    error: undefined,
  }))
}

function resetUploadedSourcesForNewConversation() {
  sources.value = sources.value.flatMap((source) => {
    if (!sourceFiles.has(source.clientId)) return []
    const {
      sourceId: _sourceId,
      projectId: _projectId,
      revision: _revision,
      expiresAt: _expiresAt,
      ...staged
    } = source
    return [{
      ...staged,
      status: 'staged' as const,
      error: undefined,
      retryable: undefined,
    }]
  })
}

function releaseStagedImages(images: AiStagedImage[] = stagedImages.value) {
  for (const image of images) revokePreviewUrl(image.previewUrl)
  const released = new Set(images.map((image) => image.localId))
  stagedImages.value = stagedImages.value.filter((image) => !released.has(image.localId))
  if (!stagedImages.value.length) imageStageError.value = undefined
}

function consumeStagedImages(
  images: AiStagedImage[],
  uploaded: PrivateImageRecord[],
  ownerConversationId: string,
) {
  const consumed = new Set(images.map((image) => image.localId))
  for (const [index, image] of images.entries()) {
    if (!image.previewUrl) continue
    if (uploaded[index]?.previewHref) {
      revokePreviewUrl(image.previewUrl)
    } else {
      retainedMessagePreviewUrls.set(image.previewUrl, ownerConversationId)
    }
  }
  stagedImages.value = stagedImages.value.filter((image) => !consumed.has(image.localId))
  imageStageError.value = undefined
}

function releaseConversationPreviewUrls(conversationId?: string) {
  for (const [previewUrl, owner] of retainedMessagePreviewUrls) {
    if (!conversationId || owner === conversationId) {
      revokePreviewUrl(previewUrl)
      retainedMessagePreviewUrls.delete(previewUrl)
    }
  }
}

function replaceDraft(updated: AiWriteDraft) {
  const projectId = selectedProjectId.value
  const replaceIn = (drafts: AiWriteDraft[]) => drafts
    .map((draft) => draft.id === updated.id ? updated : draft)
    .filter((draft) =>
      draft.projectId === projectId && draft.status === 'pending_approval')
  pendingDrafts.value = projectId && updated.projectId === projectId
    ? replaceIn(pendingDrafts.value)
    : []
  conversationDrafts.value = replaceIn(conversationDrafts.value)
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

function mergeConversationDrafts(additions: AiWriteDraft[]) {
  const projectId = selectedProjectId.value
  const merged = new Map(
    conversationDrafts.value
      .filter((draft) => draft.projectId === projectId)
      .map((draft) => [draft.id, draft]),
  )
  for (const draft of draftsForProject(additions, projectId)) {
    if (draft.status === 'pending_approval') merged.set(draft.id, draft)
    else merged.delete(draft.id)
  }
  conversationDrafts.value = [...merged.values()].sort((left, right) =>
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
    || startingConversation.value
    || sending.value
    || messages.value.some((message) => message.pending))
  const sourceUploading = computed(() =>
    sources.value.some((source) => source.status === 'uploading'))
  const readySourceCount = computed(() => sources.value.filter((source) =>
    source.status === 'staged' || isUsableComposerSource(source)).length)
  const selectedProject = computed(() =>
    projects.value.find((project) => project.id === selectedProjectId.value))
  const conversationArchived = computed(() => Boolean(currentConversation.value?.archivedAt))
  const selectedModel = computed(() =>
    modelProfiles.value.find((profile) => profile.id === selectedModelProfileId.value))
  const visionModels = computed(() => modelProfiles.value.filter((profile) =>
    profile.supportsVision && !profile.archivedAt))
  const selectedVisionModel = computed(() => visionModels.value.find((profile) =>
    profile.id === selectedVisionModelProfileId.value))
  const visionModelOptions = computed(() => visionModels.value.map((profile) => ({
    label: `${profile.name} · ${profile.modelId} · v${profile.currentVersion}${profile.isDefaultVision ? '（默认）' : ''}`,
    value: profile.id,
  })))
  const currentModelSupportsVision = computed(() => Boolean(selectedModel.value?.supportsVision))
  const hasVisualComposerInput = computed(() =>
    Boolean(stagedImages.value.length)
    || sources.value.some((source) =>
      (source.status === 'staged' || isUsableComposerSource(source)) && isImageSource(source)))
  const visionRoute = computed<'none' | 'direct' | 'relay'>(() => {
    if (!hasVisualComposerInput.value) return 'none'
    return currentModelSupportsVision.value ? 'direct' : 'relay'
  })
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
    if (conversation.archivedAt) return '已归档会话为只读，请先恢复会话'
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
    if (stagedImages.value.length && !gateway.uploadPrivateImage) {
      return '当前运行模式不支持私人图片上传'
    }
    if (sources.value.some((source) => source.status === 'staged') && !gateway.uploadAiSource) {
      return '当前运行模式不支持会话文件上传'
    }
    if (visionRoute.value === 'relay' && !selectedVisionModel.value) {
      return '当前对话模型不支持视觉，请明确选择一个可用的视觉模型'
    }
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

  function defaultVisionProfileId() {
    const profileId = modelDefaults.value.defaultVisionProfileId ?? undefined
    return visionModels.value.some((profile) => profile.id === profileId)
      ? profileId
      : undefined
  }

  function resetConversation(
    explicit: boolean,
    profileId: string | undefined = defaultConversationProfileId(),
    explicitModelSelection = false,
  ) {
    const changesConversation = Boolean(conversationId.value)
    releaseConversationPreviewUrls(conversationId.value)
    conversationScopeVersion += 1
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
    sending.value = false
    startingConversation.value = false
    loadingConversation.value = false
    autonomyBusy.value = false
    if (changesConversation) {
      resetUploadedImagesForNewConversation()
      resetUploadedSourcesForNewConversation()
    }
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
      const selectedVisionIsActive = profiles.some((profile) =>
        profile.id === selectedVisionModelProfileId.value
        && profile.supportsVision
        && !profile.archivedAt)
      if (!selectedVisionIsActive) {
        selectedVisionModelProfileId.value = defaultVisionProfileId()
      }
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

  function newConversation() {
    clearSources()
    resetConversation(true)
  }

  function clearSources() {
    sources.value = []
    sourceFiles.clear()
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

  function selectVisionModel(profileId?: string) {
    if (!profileId) {
      selectedVisionModelProfileId.value = undefined
      return
    }
    const profile = visionModels.value.find((item) => item.id === profileId)
    if (!profile) throw new Error('所选视觉模型当前不可用')
    selectedVisionModelProfileId.value = profile.id
  }

  function stageImages(files: File[]) {
    imageStageError.value = undefined
    const available = MAX_CHAT_IMAGES - stagedImages.value.length
    if (available <= 0) {
      imageStageError.value = '每次最多暂存 8 张图片'
      throw new Error(imageStageError.value)
    }
    const selected = files.slice(0, available)
    if (files.length > available) {
      imageStageError.value = `每次最多暂存 8 张图片，已保留前 ${available} 张`
    }
    const additions: AiStagedImage[] = []
    for (const file of selected) {
      if (!CHAT_IMAGE_MEDIA_TYPES.has(file.type.toLowerCase())) {
        imageStageError.value = `${file.name} 不是三种协议共同支持的 JPEG、PNG、WebP 或 GIF`
        continue
      }
      if (!file.size || file.size > MAX_CHAT_IMAGE_BYTES) {
        imageStageError.value = `${file.name} 必须不超过 10 MiB`
        continue
      }
      additions.push({
        localId: crypto.randomUUID(),
        file,
        previewUrl: createPreviewUrl(file),
        status: 'staged',
      })
    }
    stagedImages.value.push(...additions)
    if (!additions.length && imageStageError.value) throw new Error(imageStageError.value)
  }

  function removeStagedImage(localId: string) {
    const image = stagedImages.value.find((entry) => entry.localId === localId)
    if (!image) return
    revokePreviewUrl(image.previewUrl)
    stagedImages.value = stagedImages.value.filter((entry) => entry.localId !== localId)
    if (!stagedImages.value.length) imageStageError.value = undefined
  }

  function retainImageComposer() {
    imageComposerConsumers += 1
    let released = false
    return () => {
      if (released) return
      released = true
      imageComposerConsumers = Math.max(0, imageComposerConsumers - 1)
      if (imageComposerConsumers === 0) {
        releaseStagedImages()
        releaseConversationPreviewUrls()
      }
    }
  }

  async function uploadStagedImages(
    snapshot: AiStagedImage[] = [...stagedImages.value],
  ): Promise<PrivateImageRecord[]> {
    if (!snapshot.length) return []
    if (!gateway.uploadPrivateImage) throw new Error('当前运行模式不支持私人图片上传')
    const activeConversationId = conversationId.value
    if (!activeConversationId) throw new Error('请先开始会话，再上传图片')
    const uploaded: PrivateImageRecord[] = []
    for (const image of snapshot) {
      if (image.uploaded?.image.conversation_id === activeConversationId) {
        uploaded.push(image.uploaded)
        continue
      }
      image.status = 'uploading'
      image.error = undefined
      try {
        const record = await gateway.uploadPrivateImage(image.file, activeConversationId)
        image.uploaded = record
        image.status = 'ready'
        uploaded.push(record)
      } catch (error) {
        image.status = 'error'
        image.error = readableError(error)
        imageStageError.value = `${image.file.name}：${image.error}`
        throw error
      }
    }
    return uploaded
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
      await loadModels()
      const detail = await gateway.getAiConversation(id, 200)
      if (request !== conversationOpenRequest
        || !isCurrentConversationScope(sessionVersion, scopeVersion)) return
      const restoredAutonomy = gateway.getAiAutonomy
        ? await gateway.getAiAutonomy(detail.conversation.id)
        : detail.messages
          .map((message) => message.response?.autonomy)
          .filter((value): value is AiAutonomyView => Boolean(value))
          .at(-1) ?? defaultAutonomy()
      if (request !== conversationOpenRequest
        || !isCurrentConversationScope(sessionVersion, scopeVersion)) return
      if (stagedImages.value.some((image) =>
        image.uploaded?.image.conversation_id
        && image.uploaded.image.conversation_id !== detail.conversation.id)) {
        resetUploadedImagesForNewConversation()
      }
      if (conversationId.value !== detail.conversation.id) {
        releaseConversationPreviewUrls(conversationId.value)
      }
      clearSources()
      const projectChanged = selectedProjectId.value !== detail.conversation.projectId
      if (projectChanged) {
        selectedProjectId.value = detail.conversation.projectId
        pendingDrafts.value = []
      }
      currentConversation.value = detail.conversation
      conversationId.value = detail.conversation.id
      selectedModelProfileId.value = detail.conversation.modelProfileId
      selectedModelWasExplicit.value = false
      freshConversationRequested = false
      const restored = restoredMessages(detail, detail.conversation.projectId)
      messages.value = restored.length ? restored : [welcomeMessage()]
      const restoredDrafts = detail.messages.flatMap((message) => message.response?.drafts ?? [])
      mergePendingDrafts(restoredDrafts)
      conversationDrafts.value = []
      mergeConversationDrafts(restoredDrafts)
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
      autonomy.value = restoredAutonomy
      requestedMode.value = autonomy.value.mode
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

  async function startConversation(
    title: string,
    options: { fullConfirmed?: boolean; currentPassword?: string } = {},
  ): Promise<string> {
    if (conversationId.value) return conversationId.value
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

    const sessionVersion = assistantSessionVersion
    const scopeVersion = conversationScopeVersion
    const request = ++conversationOpenRequest
    const projectId = selectedProjectId.value
    const modelProfileId = selectedModelProfileId.value
    const mode = requestedMode.value
    startingConversation.value = true
    try {
      const started = await gateway.startAiConversation({
        projectId,
        title: conversationTitleFromMessage(title),
        modelProfileId,
        requestedMode: mode,
        ...(reinforcedPasswordRequired.value && options.currentPassword
          ? { currentPassword: options.currentPassword }
          : {}),
      })
      if (request !== conversationOpenRequest
        || !isCurrentConversationScope(sessionVersion, scopeVersion)
        || selectedProjectId.value !== projectId) {
        throw new Error('会话范围已变化，请重新发送')
      }
      conversationId.value = started.conversation.id
      currentConversation.value = started.conversation
      selectedModelProfileId.value = started.conversation.modelProfileId
        ?? selectedModelProfileId.value
      autonomy.value = started.autonomy
      requestedMode.value = started.autonomy.mode
      freshConversationRequested = false
      const merged = conversations.value.filter((item) =>
        item.id !== started.conversation.id)
      conversations.value = filterConversations(
        [started.conversation, ...merged],
        conversationFilter.value,
      )
      return started.conversation.id
    } finally {
      if (request === conversationOpenRequest
        && isCurrentConversationScope(sessionVersion, scopeVersion)) {
        startingConversation.value = false
      }
    }
  }

  async function uploadSource(
    clientId: string,
    file: File,
    boundConversationId: string,
  ): Promise<AiSource | undefined> {
    if (!gateway.uploadAiSource) {
      sources.value = sources.value.map((source) => source.clientId === clientId
        ? {
            ...source,
            status: 'error',
            error: '当前运行模式尚未启用对话文件上传',
            retryable: true,
          }
        : source)
      throw new Error('当前运行模式尚未启用对话文件上传')
    }
    const sessionVersion = assistantSessionVersion
    const scopeVersion = conversationScopeVersion
    const projectId = selectedProjectId.value
    sources.value = sources.value.map((source) => source.clientId === clientId
      ? { ...source, status: 'uploading', error: undefined, retryable: undefined }
      : source)
    try {
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
      return uploaded
    } catch (error) {
      if (isCurrentConversationScope(sessionVersion, scopeVersion, boundConversationId)
        && sources.value.some((source) => source.clientId === clientId)) {
        sources.value = sources.value.map((source) => source.clientId === clientId
          ? { ...source, status: 'error', error: readableError(error), retryable: true }
          : source)
      }
      throw error
    }
  }

  async function addFiles(files: File[]) {
    if (conversationReadOnlyReason.value) throw new Error(conversationReadOnlyReason.value)
    const available = Math.max(0, MAX_SOURCES_PER_TURN - sources.value.length)
    const accepted = files.slice(0, available)
    const overflow = files.slice(available)
    for (const file of accepted) {
      const clientId = crypto.randomUUID()
      const error = sourceValidationError(file)
      const source: AiComposerSource = {
        clientId,
        fileName: file.name,
        mediaType: file.type || 'application/octet-stream',
        sizeBytes: file.size,
        status: error ? 'error' : 'staged',
        error,
      }
      sources.value = [...sources.value, source]
      if (!error) {
        sourceFiles.set(clientId, file)
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
    if (conversationReadOnlyReason.value) throw new Error(conversationReadOnlyReason.value)
    const file = sourceFiles.get(clientId)
    if (!file) throw new Error('无法重试此文件，请移除后重新选择')
    sources.value = sources.value.map((source) => source.clientId === clientId
      ? { ...source, status: 'staged', error: undefined, retryable: undefined }
      : source)
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

  async function uploadStagedSources(
    snapshot: AiComposerSource[],
    boundConversationId: string,
  ): Promise<AiComposerSource[]> {
    await Promise.all(snapshot
      .filter((source) => source.status === 'staged')
      .map(async (source) => {
        const file = sourceFiles.get(source.clientId)
        if (!file) throw new Error(`${source.fileName} 的本地文件已释放，请重新选择`)
        await uploadSource(source.clientId, file, boundConversationId)
      }))
    const selected = new Set(snapshot.map((source) => source.clientId))
    return sources.value.filter((source) =>
      selected.has(source.clientId) && isUsableComposerSource(source))
  }

  async function send(
    prompt: string,
    startOptions: { fullConfirmed?: boolean; currentPassword?: string } = {},
  ) {
    if (conversationReadOnlyReason.value) throw new Error(conversationReadOnlyReason.value)
    if (loadingConversation.value) return
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
    const enteredValue = prompt.trim()
    const imageSnapshot = [...stagedImages.value]
    const sourceSnapshot = sources.value.filter((source) =>
      source.status === 'staged' || isUsableComposerSource(source, now))
    if ((!enteredValue && !imageSnapshot.length && !sourceSnapshot.length)
      || busy.value
      || sourceUploading.value) return
    const disabledReason = composerDisabledReason.value
    if (disabledReason) throw new Error(disabledReason)
    const value = enteredValue
      || (sourceSnapshot.length
        ? '请分析我提供的文件，并说明可识别的数据、错误与下一步录入预览。'
        : '请分析这些图片。')
    if (!conversationId.value) await startConversation(value, startOptions)

    const sessionVersion = assistantSessionVersion
    const scopeVersion = conversationScopeVersion
    const turnStateVersion = conversationOpenRequest
    const turnConversationId = conversationId.value
    const turnProjectId = selectedProjectId.value
    let pendingId: string | undefined
    if (!turnConversationId) throw new Error('会话启动失败，请重新发送')
    sending.value = true
    try {
      const [readySources, uploadedImages] = await Promise.all([
        uploadStagedSources(sourceSnapshot, turnConversationId),
        uploadStagedImages(imageSnapshot),
      ])
      if (turnStateVersion !== conversationOpenRequest
        || !isCurrentConversationScope(sessionVersion, scopeVersion, turnConversationId)
        || turnProjectId !== selectedProjectId.value) return

      const hasVisualTurnInput = Boolean(imageSnapshot.length)
        || readySources.some(isImageSource)
      const messageSources = readySources.map((source) => ({
        sourceId: source.sourceId,
        fileName: source.fileName,
        mediaType: source.mediaType,
        sizeBytes: source.sizeBytes,
      }))
      const userImages = imageSnapshot.map((image, index) => ({
        id: uploadedImages[index]?.image.id ?? image.localId,
        fileName: image.file.name,
        previewHref: uploadedImages[index]?.previewHref || image.previewUrl,
      }))
      messages.value.push({
        id: crypto.randomUUID(),
        role: 'user',
        content: value,
        images: userImages.length ? userImages : undefined,
        sources: messageSources.length ? messageSources : undefined,
        createdAt: new Date().toISOString(),
      })
      pendingId = crypto.randomUUID()
      messages.value.push({
        id: pendingId,
        role: 'assistant',
        content: imageSnapshot.length
          ? '正在安全处理图片证据…'
          : readySources.length
            ? '正在分析已授权的来源文件…'
            : '正在查询已授权的数据…',
        createdAt: new Date().toISOString(),
        pending: true,
      })
      const response = await gateway.aiTurn({
        conversationId: turnConversationId,
        projectId: turnProjectId,
        message: value,
        ...(readySources.length
          ? { sourceRefs: readySources.flatMap((source) => source.sourceId ? [source.sourceId] : []) }
          : {}),
        imageIds: uploadedImages.map((entry) => entry.image.id),
        ...(hasVisualTurnInput
          && !currentModelSupportsVision.value
          && selectedVisionModelProfileId.value
          ? { visionModelProfileId: selectedVisionModelProfileId.value }
          : {}),
      })
      if (turnStateVersion !== conversationOpenRequest
        || !isCurrentConversationScope(sessionVersion, scopeVersion, turnConversationId)
        || turnProjectId !== selectedProjectId.value) return
      conversationId.value = response.conversationId
      autonomy.value = response.autonomy ?? autonomy.value
      freshConversationRequested = false
      const responseDrafts = draftsForProject(response.drafts, selectedProjectId.value)
      mergePendingDrafts(responseDrafts)
      mergeConversationDrafts(responseDrafts)
      const index = messages.value.findIndex((message) => message.id === pendingId)
      if (index >= 0) {
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
      }
      const submitted = new Set(readySources.map((source) => source.clientId))
      sources.value = sources.value.filter((source) => !submitted.has(source.clientId))
      for (const clientId of submitted) sourceFiles.delete(clientId)
      if (composerDraft.value.trim() === enteredValue) composerDraft.value = ''
      consumeStagedImages(imageSnapshot, uploadedImages, response.conversationId)
      void refreshConversations().catch(() => undefined)
    } catch (error) {
      if (turnStateVersion !== conversationOpenRequest
        || !isCurrentConversationScope(sessionVersion, scopeVersion, turnConversationId)) return
      if (pendingId) {
        const index = messages.value.findIndex((message) => message.id === pendingId)
        if (index >= 0) {
          messages.value[index] = {
            id: pendingId,
            role: 'assistant',
            content: readableError(error),
            createdAt: new Date().toISOString(),
            error: true,
          }
        }
      } else {
        throw error
      }
    } finally {
      if (isCurrentAssistantSession(sessionVersion)
        && scopeVersion === conversationScopeVersion) sending.value = false
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
    if (conversationReadOnlyReason.value) throw new Error(conversationReadOnlyReason.value)
    if (loadingConversation.value) throw new Error('正在切换会话，请稍后再调整 AI 授权')
    if (!conversationId.value) throw new Error('请先开始会话，再更新当前会话的 AI 授权')
    if (!gateway.setAiAutonomy) throw new Error('当前运行模式不支持会话授权设置')
    const sessionVersion = assistantSessionVersion
    const scopeVersion = conversationScopeVersion
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
        && isCurrentConversationScope(
          sessionVersion,
          scopeVersion,
          activeConversationId,
        )) {
        autonomy.value = updated
        requestedMode.value = mode
      }
      return updated
    } finally {
      if (stateVersion === conversationOpenRequest
        && isCurrentConversationScope(
          sessionVersion,
          scopeVersion,
          activeConversationId,
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
    conversationDrafts,
    composerDraft,
    modelProfiles,
    modelDefaults,
    modelsLoaded,
    selectedModelProfileId,
    selectedModel,
    modelOptions,
    selectedVisionModelProfileId,
    selectedVisionModel,
    visionModelOptions,
    currentModelSupportsVision,
    visionRoute,
    stagedImages,
    imageStageError,
    loadingModels,
    requestedMode,
    startingConversation,
    loadingDrafts,
    loadingConversations,
    loadingConversation,
    autonomy,
    autonomyBusy,
    busy,
    sources,
    sourceUploading,
    readySourceCount,
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
    selectVisionModel,
    stageImages,
    removeStagedImage,
    retainImageComposer,
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
    startConversation,
    send,
    requestMode,
    updateAutonomy,
    decideDraft,
    draftBusy,
  }
}
