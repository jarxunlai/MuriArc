import { beforeEach, describe, expect, it, vi } from 'vitest'
import type {
  AiAutonomyView,
  AiConversationDetail,
  AiConversationSummary,
  AiTurnResponse,
  AiWriteDraft,
} from '@/domain/models'
import type { AiModelProfileView } from '@/services/gateway'

const mocks = vi.hoisted(() => ({
  mode: 'local' as 'local' | 'remote',
  listProjects: vi.fn(),
  listAiDrafts: vi.fn(),
  listAiConversations: vi.fn(),
  getAiConversation: vi.fn(),
  listAiModelProfiles: vi.fn(),
  getAiModelDefaults: vi.fn(),
  startAiConversation: vi.fn(),
  getAiAutonomy: vi.fn(),
  setAiAutonomy: vi.fn(),
  aiTurn: vi.fn(),
  uploadPrivateImage: vi.fn(),
  decideAiDraft: vi.fn(),
}))

vi.mock('@/services/gateway', () => ({
  gateway: mocks,
}))

import { useAiAssistant } from './useAiAssistant'

const autonomy = (
  mode: AiAutonomyView['mode'] = 'ask',
  effectiveMode: AiAutonomyView['effectiveMode'] = mode,
): AiAutonomyView => ({
  mode,
  effectiveMode,
  maxMode: 'full',
  batchLimit: 100,
  revision: 1,
  requiresHumanApproval: [],
})

const modelProfile = (
  id = 'profile-1',
  overrides: Partial<AiModelProfileView> = {},
): AiModelProfileView => ({
  id,
  name: id === 'profile-1' ? '主对话模型' : '备用模型',
  currentVersion: 2,
  revision: 3,
  protocol: 'openai_responses',
  transport: 'open_ai_compatible',
  baseUrl: 'https://provider.example/v1',
  modelId: id === 'profile-1' ? 'model-primary' : 'model-secondary',
  supportsVision: false,
  contextWindowTokens: 131072,
  maxInputTokens: 65536,
  maxOutputTokens: 4096,
  historyTokenBudget: 32768,
  historyTurns: 20,
  temperature: 0,
  timeoutMs: 120000,
  hasKey: true,
  isDefaultConversation: id === 'profile-1',
  isDefaultVision: false,
  ...overrides,
})

const turnResponse = (
  conversationId = 'conversation-1',
  drafts: AiWriteDraft[] = [],
): AiTurnResponse => ({
  conversationId,
  content: '查询完成',
  citations: [{
    entityType: 'animal', entityId: 'animal-1', revision: 2,
    label: '动物 animal-1', route: '/animals?animal=animal-1',
  }],
  toolRuns: [{
    toolRunId: 'run-1', providerCallId: 'call-1', tool: 'animal_search',
    arguments: {}, outcome: 'read', citations: [],
  }],
  drafts,
  trace: {
    providerId: 'test-provider', model: 'test-model',
    usage: { providerCalls: 1, toolCalls: 1, inputTokens: 3, outputTokens: 2, totalTokens: 5 },
    context: { estimatedInputTokens: 3, inputTokenCountIsEstimate: true, contextTrimmed: false, trimmedHistoryTurns: 0, trimReasons: [] },
  },
  autonomy: autonomy('full', 'full'),
})

const measurementDraft = (): AiWriteDraft => ({
  id: 'draft-1',
  kind: 'measurement_result',
  projectId: 'project-1',
  changes: [{ path: '/value', before: null, after: 23.4 }],
  requirement: 'researcher_signature',
  status: 'pending_approval',
  revision: 1,
  createdAt: '2026-07-18T02:00:00Z',
  expiresAt: '2026-07-19T02:00:00Z',
})

const reinforcedDraft = (): AiWriteDraft => ({
  ...measurementDraft(),
  id: 'draft-import-1',
  kind: 'bulk_import',
  requirement: 'reinforced_confirmation',
})

const conversationSummary = (projectId = 'project-1'): AiConversationSummary => ({
  id: 'conversation-1',
  projectId,
  title: '总结实验进度',
  modelProfileId: 'profile-1',
  modelProfileVersion: 2,
  modelProfileName: '主对话模型',
  modelId: 'model-primary',
  readOnly: false,
  createdAt: '2026-07-18T01:00:00Z',
  updatedAt: '2026-07-18T02:00:00Z',
  revision: 2,
})

const conversationDetail = (): AiConversationDetail => ({
  conversation: conversationSummary(),
  messages: [
    {
      id: 'message-1', sequence: 1, role: 'user', content: '总结实验进度',
      createdAt: '2026-07-18T01:00:00Z',
    },
    {
      id: 'message-2', sequence: 2, role: 'assistant', content: '查询完成',
      response: turnResponse(), createdAt: '2026-07-18T01:00:01Z',
    },
  ],
})

const privateImage = (
  id = 'image-1',
  conversationId = 'conversation-1',
  previewHref = '/api/v1/ai/images/image-1/content?preview=true',
) => ({
  image: {
    id,
    conversation_id: conversationId,
    project_id: 'project-1',
    status: 'active',
    expires_at: '2026-08-18T01:00:00Z',
    meta: { revision: 1 },
  },
  fileName: 'evidence.png',
  mediaType: 'image/png',
  sizeBytes: 4,
  sha256: 'abc123',
  contentHref: `/api/v1/ai/images/${id}/content`,
  previewHref,
  retentionDays: 30,
})

describe('useAiAssistant', () => {
  let ai: ReturnType<typeof useAiAssistant>

  beforeEach(async () => {
    vi.clearAllMocks()
    mocks.mode = 'local'
    mocks.listProjects.mockResolvedValue([{ id: 'project-1', name: 'DEMO' }])
    mocks.listAiDrafts.mockResolvedValue([])
    mocks.listAiConversations.mockResolvedValue([])
    mocks.getAiConversation.mockResolvedValue(conversationDetail())
    mocks.listAiModelProfiles.mockResolvedValue([
      modelProfile(),
      modelProfile('profile-2'),
    ])
    mocks.getAiModelDefaults.mockResolvedValue({
      defaultConversationProfileId: 'profile-1',
      revision: 1,
    })
    mocks.startAiConversation.mockResolvedValue({
      conversation: conversationSummary(),
      autonomy: autonomy('full', 'full'),
    })
    mocks.getAiAutonomy.mockResolvedValue(autonomy())
    mocks.setAiAutonomy.mockImplementation(async (_id, input) =>
      autonomy(input.mode, input.mode))
    mocks.aiTurn.mockResolvedValue(turnResponse())
    mocks.uploadPrivateImage.mockResolvedValue(privateImage())
    ai = useAiAssistant()
    ai.selectedProjectId.value = 'project-1'
    ai.pendingDrafts.value = []
    ai.conversationDrafts.value = []
    ai.composerDraft.value = ''
    ai.newConversation()
    await ai.loadModels(true)
  })

  it('starts with the explicit model and carries the conversation id into later turns', async () => {
    await ai.send('总结实验进度', { fullConfirmed: true })
    await ai.send('哪些动物缺少体重？')

    expect(mocks.startAiConversation).toHaveBeenCalledWith({
      projectId: 'project-1',
      modelProfileId: 'profile-1',
      requestedMode: 'full',
    })
    expect(mocks.aiTurn).toHaveBeenNthCalledWith(1, {
      conversationId: 'conversation-1',
      projectId: 'project-1',
      message: '总结实验进度',
      imageIds: [],
    })
    expect(mocks.aiTurn).toHaveBeenNthCalledWith(2, {
      conversationId: 'conversation-1',
      projectId: 'project-1',
      message: '哪些动物缺少体重？',
      imageIds: [],
    })
    expect(ai.messages.value.at(-1)).toEqual(expect.objectContaining({
      role: 'assistant', content: '查询完成',
    }))
  })

  it('does not call a turn or consume the shared prompt when Full start verification fails', async () => {
    ai.composerDraft.value = '查询动物'
    mocks.startAiConversation.mockRejectedValueOnce(new Error('当前密码验证失败'))

    await expect(ai.send('查询动物', {
      fullConfirmed: true,
    })).rejects.toThrow('当前密码验证失败')

    expect(mocks.aiTurn).not.toHaveBeenCalled()
    expect(ai.conversationId.value).toBeUndefined()
    expect(ai.messages.value).toHaveLength(1)
    expect(ai.composerDraft.value).toBe('查询动物')
  })

  it('turns provider failures after a successful start into an explicit non-demo error', async () => {
    mocks.aiTurn.mockRejectedValueOnce(new Error('请先启用 AI 并配置所需密钥'))

    await ai.send('查询动物', { fullConfirmed: true })

    expect(ai.messages.value.at(-1)).toEqual(expect.objectContaining({
      role: 'assistant', error: true, content: '请先启用 AI 并配置所需密钥',
    }))
    expect(mocks.startAiConversation).toHaveBeenCalledOnce()
  })

  it('does not silently choose the first vision model when no default exists', async () => {
    const releaseComposer = ai.retainImageComposer()
    mocks.listAiModelProfiles.mockResolvedValue([
      modelProfile(),
      modelProfile('vision-1', {
        name: '视觉模型',
        supportsVision: true,
        isDefaultConversation: false,
      }),
    ])
    mocks.getAiModelDefaults.mockResolvedValue({
      defaultConversationProfileId: 'profile-1',
      revision: 2,
    })
    await ai.loadModels(true)
    ai.stageImages([new File(['data'], 'evidence.png', { type: 'image/png' })])

    expect(ai.selectedVisionModelProfileId.value).toBeUndefined()
    expect(ai.composerDisabledReason.value).toContain('明确选择')
    await expect(ai.send('分析图片', { fullConfirmed: true })).rejects.toThrow('明确选择')
    expect(mocks.startAiConversation).not.toHaveBeenCalled()
    expect(mocks.uploadPrivateImage).not.toHaveBeenCalled()
    expect(mocks.aiTurn).not.toHaveBeenCalled()
    releaseComposer()
  })

  it('preserves the prompt and staged images when the provider call fails', async () => {
    const releaseComposer = ai.retainImageComposer()
    mocks.listAiModelProfiles.mockResolvedValue([
      modelProfile('profile-1', { supportsVision: true }),
      modelProfile('profile-2'),
    ])
    await ai.loadModels(true)
    ai.composerDraft.value = '从图片读取体重'
    ai.stageImages([new File(['data'], 'evidence.png', { type: 'image/png' })])
    mocks.aiTurn.mockRejectedValueOnce(new Error('视觉 Provider 暂时不可用'))

    await ai.send(ai.composerDraft.value, { fullConfirmed: true })

    expect(ai.composerDraft.value).toBe('从图片读取体重')
    expect(ai.stagedImages.value).toHaveLength(1)
    expect(ai.stagedImages.value[0]).toEqual(expect.objectContaining({
      status: 'ready',
      uploaded: expect.objectContaining({ image: expect.objectContaining({ id: 'image-1' }) }),
    }))
    expect(ai.messages.value.at(-1)).toEqual(expect.objectContaining({
      error: true,
      content: '视觉 Provider 暂时不可用',
    }))
    releaseComposer()
  })

  it('keeps staged files but clears their conversation upload binding after a confirmed model switch', async () => {
    const releaseComposer = ai.retainImageComposer()
    mocks.listAiModelProfiles.mockResolvedValue([
      modelProfile('profile-1', { supportsVision: true }),
      modelProfile('profile-2', { supportsVision: true }),
    ])
    await ai.loadModels(true)
    ai.composerDraft.value = '保留这段未发送输入'
    ai.stageImages([new File(['data'], 'evidence.png', { type: 'image/png' })])
    mocks.aiTurn.mockRejectedValueOnce(new Error('请求失败'))
    await ai.send('先尝试一次', { fullConfirmed: true })

    expect(ai.selectModel('profile-2')).toBe(false)
    expect(ai.selectModel('profile-2', true)).toBe(true)
    expect(ai.conversationId.value).toBeUndefined()
    expect(ai.composerDraft.value).toBe('保留这段未发送输入')
    expect(ai.stagedImages.value).toEqual([
      expect.objectContaining({ status: 'staged', uploaded: undefined }),
    ])
    releaseComposer()
  })

  it('rejects image formats outside the common provider protocol set', () => {
    expect(() => ai.stageImages([
      new File(['bitmap'], 'legacy.bmp', { type: 'image/bmp' }),
    ])).toThrow('JPEG、PNG、WebP 或 GIF')
    expect(ai.stagedImages.value).toHaveLength(0)
    expect(mocks.uploadPrivateImage).not.toHaveBeenCalled()
  })

  it('rejects chat images above 10 MiB before upload or Provider work', async () => {
    const oversized = new File(['x'], 'oversized.png', { type: 'image/png' })
    Object.defineProperty(oversized, 'size', { value: 10 * 1024 * 1024 + 1 })

    expect(() => ai.stageImages([oversized])).toThrow('10 MiB')
    expect(ai.stagedImages.value).toHaveLength(0)
    expect(mocks.uploadPrivateImage).not.toHaveBeenCalled()
    expect(mocks.startAiConversation).not.toHaveBeenCalled()
    expect(mocks.aiTurn).not.toHaveBeenCalled()
  })

  it('revokes the local preview immediately when a successful upload has a private preview URL', async () => {
    const createPreview = vi.spyOn(URL, 'createObjectURL').mockReturnValue('blob:success-preview')
    const revokePreview = vi.spyOn(URL, 'revokeObjectURL').mockImplementation(() => undefined)
    const releaseComposer = ai.retainImageComposer()
    mocks.listAiModelProfiles.mockResolvedValue([
      modelProfile('profile-1', { supportsVision: true }),
    ])
    await ai.loadModels(true)
    ai.stageImages([new File(['data'], 'evidence.png', { type: 'image/png' })])

    await ai.send('分析图片', { fullConfirmed: true })

    expect(createPreview).toHaveBeenCalledOnce()
    expect(revokePreview).toHaveBeenCalledWith('blob:success-preview')
    expect(ai.stagedImages.value).toHaveLength(0)
    expect(ai.messages.value.find((message) => message.role === 'user')?.images?.[0]?.previewHref)
      .toBe('/api/v1/ai/images/image-1/content?preview=true')
    releaseComposer()
    createPreview.mockRestore()
    revokePreview.mockRestore()
  })

  it('releases fallback message previews on conversation switch and staged previews on final unmount', async () => {
    const createPreview = vi.spyOn(URL, 'createObjectURL')
      .mockReturnValueOnce('blob:fallback-message')
      .mockReturnValueOnce('blob:still-staged')
    const revokePreview = vi.spyOn(URL, 'revokeObjectURL').mockImplementation(() => undefined)
    const releaseComposer = ai.retainImageComposer()
    mocks.listAiModelProfiles.mockResolvedValue([
      modelProfile('profile-1', { supportsVision: true }),
      modelProfile('profile-2', { supportsVision: true }),
    ])
    mocks.uploadPrivateImage.mockResolvedValueOnce(privateImage('image-1', 'conversation-1', ''))
    await ai.loadModels(true)
    ai.stageImages([new File(['data'], 'first.png', { type: 'image/png' })])
    await ai.send('分析第一张', { fullConfirmed: true })

    expect(revokePreview).not.toHaveBeenCalledWith('blob:fallback-message')
    expect(ai.selectModel('profile-2', true)).toBe(true)
    expect(revokePreview).toHaveBeenCalledWith('blob:fallback-message')

    ai.stageImages([new File(['next'], 'second.png', { type: 'image/png' })])
    releaseComposer()
    expect(revokePreview).toHaveBeenCalledWith('blob:still-staged')
    expect(ai.stagedImages.value).toHaveLength(0)
    createPreview.mockRestore()
    revokePreview.mockRestore()
  })

  it('requires an explicit selection when the saved default is unavailable', async () => {
    mocks.listAiModelProfiles.mockResolvedValue([
      modelProfile('profile-1', { archivedAt: '2026-07-20T00:00:00Z' }),
    ])
    await ai.loadModels(true)
    ai.newConversation()

    expect(ai.selectedModelProfileId.value).toBeUndefined()
    expect(ai.composerDisabledReason.value).toContain('明确选择')
    await expect(ai.send('不能静默选第一个', {
      fullConfirmed: true,
    })).rejects.toThrow('明确选择')
    expect(mocks.startAiConversation).not.toHaveBeenCalled()
    expect(mocks.aiTurn).not.toHaveBeenCalled()
  })

  it('does not fall back to the first active profile when the saved default no longer exists', async () => {
    mocks.listAiModelProfiles.mockResolvedValue([
      modelProfile('profile-1', { isDefaultConversation: false }),
      modelProfile('profile-2', { isDefaultConversation: false }),
    ])
    mocks.getAiModelDefaults.mockResolvedValue({
      defaultConversationProfileId: 'missing-profile',
      revision: 7,
    })

    await ai.loadModels(true)
    ai.newConversation()

    expect(ai.selectedModelProfileId.value).toBeUndefined()
    expect(ai.modelOptions.value.map((option) => option.value)).toEqual([
      'profile-1',
      'profile-2',
    ])
    await expect(ai.send('不能回退到列表第一项', {
      fullConfirmed: true,
    })).rejects.toThrow('明确选择')
    expect(mocks.startAiConversation).not.toHaveBeenCalled()
    expect(mocks.aiTurn).not.toHaveBeenCalled()
  })

  it('restores persisted content without injecting project-level drafts into the conversation', async () => {
    const draft = measurementDraft()
    const unrelatedProjectDraft = { ...measurementDraft(), id: 'project-draft' }
    const detail = conversationDetail()
    detail.messages[1].response = { ...turnResponse(), drafts: [draft] }
    mocks.listAiConversations.mockResolvedValue([conversationSummary()])
    mocks.getAiConversation.mockResolvedValue(detail)
    ai.pendingDrafts.value = [unrelatedProjectDraft]

    await ai.restoreLatestConversation(true)

    expect(mocks.listAiConversations).toHaveBeenCalledWith('project-1', 50)
    expect(mocks.getAiConversation).toHaveBeenCalledWith('conversation-1', 200)
    expect(ai.conversationId.value).toBe('conversation-1')
    expect(ai.messages.value[1]).toEqual(expect.objectContaining({
      id: 'message-2',
      drafts: [expect.objectContaining({ id: 'draft-1' })],
    }))
    expect(ai.conversationDrafts.value).toEqual([expect.objectContaining({ id: 'draft-1' })])
    expect(ai.pendingDrafts.value).toEqual([expect.objectContaining({ id: 'project-draft' })])
  })

  it('shows the immutable conversation version instead of the profiles current model identity', async () => {
    mocks.listAiModelProfiles.mockResolvedValue([
      modelProfile('profile-1', {
        name: '当前配置名称',
        modelId: 'model-current-v3',
        currentVersion: 3,
      }),
      modelProfile('profile-2'),
    ])
    await ai.loadModels(true)
    const detail = conversationDetail()
    detail.conversation = {
      ...conversationSummary(),
      modelProfileVersion: 1,
      modelProfileName: '会话绑定名称',
      modelId: 'model-pinned-v1',
    }
    mocks.getAiConversation.mockResolvedValue(detail)

    await ai.openConversation('conversation-1')

    expect(ai.selectedModelProfileId.value).toBe('profile-1')
    expect(ai.modelOptions.value.find((option) => option.value === 'profile-1')?.label)
      .toBe('会话绑定名称 · model-pinned-v1 · v1')
    expect(mocks.startAiConversation).not.toHaveBeenCalled()
  })

  it('switches project scope before restoring that projects latest conversation', async () => {
    mocks.listProjects.mockResolvedValue([
      { id: 'project-1', name: 'DEMO' },
      { id: 'project-2', name: '肺纤维化' },
    ])
    const summary = conversationSummary('project-2')
    summary.id = 'conversation-2'
    const detail = conversationDetail()
    detail.conversation = summary
    detail.messages = []
    mocks.listAiConversations.mockResolvedValue([summary])
    mocks.getAiConversation.mockResolvedValue(detail)

    await ai.selectProject('project-2')

    expect(mocks.listAiConversations).toHaveBeenCalledWith('project-2', 50)
    expect(ai.selectedProjectId.value).toBe('project-2')
    expect(ai.conversationId.value).toBe('conversation-2')
  })

  it('switches an empty conversation directly but requires confirmation after persisted messages', async () => {
    await ai.requestMode('ask')
    await ai.startConversation()
    expect(ai.hasPersistedMessages.value).toBe(false)
    expect(ai.selectModel('profile-2')).toBe(true)
    expect(ai.conversationId.value).toBeUndefined()
    expect(ai.selectedModelProfileId.value).toBe('profile-2')

    mocks.startAiConversation.mockResolvedValueOnce({
      conversation: {
        ...conversationSummary(),
        modelProfileId: 'profile-2',
        modelProfileName: '备用模型',
        modelId: 'model-secondary',
      },
      autonomy: autonomy('full', 'auto'),
    })
    ai.composerDraft.value = '尚未发送的输入'
    ai.pendingDrafts.value = [{ ...measurementDraft(), id: 'project-level-draft' }]
    await ai.send('已发送的消息', { fullConfirmed: true })
    ai.conversationDrafts.value = [measurementDraft()]

    expect(ai.selectModel('profile-1')).toBe(false)
    expect(ai.conversationId.value).toBe('conversation-1')
    expect(ai.selectedModelProfileId.value).toBe('profile-2')

    expect(ai.selectModel('profile-1', true)).toBe(true)
    expect(ai.conversationId.value).toBeUndefined()
    expect(ai.selectedModelProfileId.value).toBe('profile-1')
    expect(ai.messages.value).toHaveLength(1)
    expect(ai.conversationDrafts.value).toEqual([])
    expect(ai.pendingDrafts.value).toEqual([expect.objectContaining({ id: 'project-level-draft' })])
    expect(ai.composerDraft.value).toBe('尚未发送的输入')
    expect(ai.requestedMode.value).toBe('full')
    expect(ai.autonomy.value.effectiveMode).toBe('ask')
  })

  it('keeps archived and legacy histories readable while disabling their composer', async () => {
    const archived = modelProfile('profile-1', { archivedAt: '2026-07-20T00:00:00Z' })
    mocks.listAiModelProfiles.mockResolvedValue([archived, modelProfile('profile-2')])
    await ai.loadModels(true)
    const detail = conversationDetail()
    detail.conversation = {
      ...conversationSummary(),
      readOnly: true,
      readOnlyReason: 'model_archived',
    }
    mocks.getAiConversation.mockResolvedValue(detail)

    await ai.openConversation('conversation-1')

    expect(ai.messages.value).toHaveLength(2)
    expect(ai.conversationReadOnlyReason.value).toContain('已归档')
    expect(ai.composerDisabledReason.value).toContain('已归档')
    await expect(ai.send('不能继续归档会话')).rejects.toThrow('已归档')
    expect(mocks.aiTurn).not.toHaveBeenCalled()
  })

  it.each([
    ['legacy_model_unknown', undefined, '旧会话没有可识别的模型绑定'],
    ['model_unavailable', 'missing-profile', '模型版本当前不可用'],
  ] as const)(
    'keeps %s history readable and blocks direct send calls',
    async (readOnlyReason, modelProfileId, expectedReason) => {
      const detail = conversationDetail()
      detail.conversation = {
        ...conversationSummary(),
        modelProfileId,
        readOnly: true,
        readOnlyReason,
      }
      mocks.getAiConversation.mockResolvedValue(detail)

      await ai.openConversation('conversation-1')

      expect(ai.messages.value).toHaveLength(2)
      expect(ai.conversationReadOnlyReason.value).toContain(expectedReason)
      expect(ai.composerDisabledReason.value).toContain(expectedReason)
      await expect(ai.send('不能继续只读历史')).rejects.toThrow(expectedReason)
      expect(mocks.aiTurn).not.toHaveBeenCalled()
    },
  )

  it('shares the unsent composer draft between Drawer and Workspace consumers', () => {
    const drawer = useAiAssistant()
    const workspace = useAiAssistant()
    drawer.composerDraft.value = '从 Drawer 输入但尚未发送'

    expect(workspace.composerDraft.value).toBe('从 Drawer 输入但尚未发送')
    workspace.newConversation()
    expect(drawer.composerDraft.value).toBe('从 Drawer 输入但尚未发送')
  })

  it('requires a researcher statement without sending any client step-up claim', async () => {
    const draft = measurementDraft()
    mocks.decideAiDraft.mockResolvedValue({
      draft: { ...draft, status: 'applied', revision: 3 },
      measurementId: 'measurement-1',
    })

    await expect(ai.decideDraft(draft, 'approve')).rejects.toThrow('签署声明')
    const result = await ai.decideDraft(draft, 'approve', '我已核对动物、数值与单位')

    expect(result.measurementId).toBe('measurement-1')
    expect(mocks.decideAiDraft).toHaveBeenCalledWith('draft-1', {
      expectedRevision: 1,
      decision: 'approve',
      statement: '我已核对动物、数值与单位',
    })
    expect(mocks.decideAiDraft.mock.calls[0][1]).not.toHaveProperty('stepUpVerified')
  })

  it('requires only a statement locally and never sends a password to Tauri', async () => {
    const draft = reinforcedDraft()
    mocks.decideAiDraft.mockResolvedValue({
      draft: { ...draft, status: 'applied', revision: 3 },
      jobId: 'job-1',
    })

    await expect(ai.decideDraft(draft, 'approve')).rejects.toThrow('确认声明')
    const result = await ai.decideDraft(
      draft,
      'approve',
      '我已核对完整导入预览',
      'must-not-leave-local-mode',
    )

    expect(result.jobId).toBe('job-1')
    expect(mocks.decideAiDraft).toHaveBeenCalledWith('draft-import-1', {
      expectedRevision: 1,
      decision: 'approve',
      statement: '我已核对完整导入预览',
    })
    expect(mocks.decideAiDraft.mock.calls.at(-1)?.[1]).not.toHaveProperty('currentPassword')
  })

  it('requires and sends the current password only for a remote reinforced approval', async () => {
    mocks.mode = 'remote'
    const draft = reinforcedDraft()
    mocks.decideAiDraft.mockResolvedValue({
      draft: { ...draft, status: 'applied', revision: 3 },
      jobId: 'job-1',
    })

    await expect(ai.decideDraft(draft, 'approve', '我已核对完整导入预览')).rejects.toThrow('当前密码')
    await ai.decideDraft(draft, 'approve', '我已核对完整导入预览', 'one-request-password')

    expect(mocks.decideAiDraft).toHaveBeenCalledWith('draft-import-1', {
      expectedRevision: 1,
      decision: 'approve',
      statement: '我已核对完整导入预览',
      currentPassword: 'one-request-password',
    })
  })

  it('never sends a password when rejecting a reinforced draft', async () => {
    mocks.mode = 'remote'
    const draft = reinforcedDraft()
    mocks.decideAiDraft.mockResolvedValue({
      draft: { ...draft, status: 'rejected', revision: 2 },
    })

    await ai.decideDraft(draft, 'reject', undefined, 'must-not-leave-the-client')

    expect(mocks.decideAiDraft).toHaveBeenCalledWith('draft-import-1', {
      expectedRevision: 1,
      decision: 'reject',
      statement: undefined,
    })
  })
})
