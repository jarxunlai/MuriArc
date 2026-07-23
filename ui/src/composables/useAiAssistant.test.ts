import { beforeEach, describe, expect, it, vi } from 'vitest'
import type {
  AiConversationDetail,
  AiConversationSummary,
  AiTurnResponse,
  AiWriteDraft,
  AuthSession,
} from '@/domain/models'
import { currentAuthSession } from '@/services/projectContext'

const mocks = vi.hoisted(() => ({
  mode: 'local' as 'local' | 'remote',
  listProjects: vi.fn(),
  listAiDrafts: vi.fn(),
  listAiConversations: vi.fn(),
  getAiConversation: vi.fn(),
  createAiConversation: vi.fn(),
  updateAiConversation: vi.fn(),
  getAiAutonomy: vi.fn(),
  setAiAutonomy: vi.fn(),
  aiTurn: vi.fn(),
  decideAiDraft: vi.fn(),
  uploadAiSource: vi.fn(),
  listAiSources: vi.fn(),
  archiveAiSource: vi.fn(),
  deleteAiSource: vi.fn(),
}))

vi.mock('@/services/gateway', () => ({
  gateway: mocks,
}))

import { useAiAssistant } from './useAiAssistant'

const turnResponse = (conversationId = 'conversation-1'): AiTurnResponse => ({
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
  drafts: [],
  trace: {
    providerId: 'test-provider', model: 'test-model',
    usage: { providerCalls: 1, toolCalls: 1, inputTokens: 3, outputTokens: 2, totalTokens: 5 },
    context: { estimatedInputTokens: 3, inputTokenCountIsEstimate: true, contextTrimmed: false, trimmedHistoryTurns: 0, trimReasons: [] },
  },
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
  importPreview: {
    importKind: 'measurement',
    projectId: 'project-1',
    experimentId: 'experiment-1',
    fileName: 'measurements.csv',
    sheetName: 'Sheet1',
    totalRows: 2,
    acceptedRows: 2,
    issueCount: 0,
    issuesTruncated: false,
    canConfirm: true,
    previewRowsTruncated: false,
    previewRows: [{
      rowNumber: 2,
      animalId: 'animal-1',
      animalDisplayId: 'M-001',
      measurementKey: 'body_weight',
      value: '23.4',
      unit: 'g',
      measuredAt: '2026-07-18T01:00:00Z',
    }, {
      rowNumber: 3,
      animalId: 'animal-2',
      animalDisplayId: 'M-002',
      measurementKey: 'body_weight',
      value: '24.1',
      unit: 'g',
      measuredAt: '2026-07-18T01:05:00Z',
    }],
    issues: [],
  },
})

const conversationSummary = (projectId = 'project-1'): AiConversationSummary => ({
  id: 'conversation-1',
  projectId,
  title: '总结实验进度',
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

const autonomyView = (mode: 'ask' | 'auto' | 'full' = 'ask') => ({
  mode,
  effectiveMode: mode,
  maxMode: 'full' as const,
  batchLimit: mode === 'full' ? 100 : 1,
  revision: mode === 'ask' ? 0 : 1,
  requiresHumanApproval: [],
})

function session(userId: string): AuthSession {
  return {
    user: {
      id: userId,
      labId: 'lab-1',
      displayName: userId,
      labRoles: [],
      projectRoles: [],
      authentication: 'session',
      mustChangePassword: false,
      isEnvironmentRoot: false,
    },
    csrfAvailable: true,
  }
}

describe('useAiAssistant', () => {
  let ai: ReturnType<typeof useAiAssistant>

  beforeEach(() => {
    vi.clearAllMocks()
    currentAuthSession.value = session('test-reset')
    currentAuthSession.value = undefined
    mocks.mode = 'local'
    ai = useAiAssistant()
    mocks.listProjects.mockResolvedValue([{ id: 'project-1', name: 'DEMO' }])
    mocks.listAiDrafts.mockResolvedValue([])
    mocks.listAiConversations.mockResolvedValue([])
    mocks.getAiConversation.mockResolvedValue(conversationDetail())
    mocks.createAiConversation.mockResolvedValue(conversationSummary())
    mocks.updateAiConversation.mockImplementation(async (
      _id: string,
      input: { action: string; title?: string; expectedRevision: number },
    ) => ({
      ...conversationSummary(),
      title: input.action === 'rename' ? input.title : conversationSummary().title,
      archivedAt: input.action === 'archive' ? '2026-07-24T01:00:00Z' : undefined,
      revision: input.expectedRevision + 1,
    }))
    mocks.getAiAutonomy.mockResolvedValue(autonomyView())
    mocks.setAiAutonomy.mockResolvedValue(autonomyView('auto'))
    mocks.aiTurn.mockResolvedValue(turnResponse())
    mocks.uploadAiSource.mockImplementation(async ({
      file,
      conversationId,
      projectId,
    }: {
      file: File
      conversationId?: string
      projectId?: string
    }) => ({
      id: 'source-1',
      conversationId,
      projectId,
      fileName: file.name,
      mediaType: file.type,
      sizeBytes: file.size,
      status: 'ready',
      revision: 1,
      createdAt: '2026-07-23T01:00:00Z',
      expiresAt: '2999-08-22T01:00:00Z',
    }))
    mocks.listAiSources.mockResolvedValue([])
    mocks.archiveAiSource.mockImplementation(async (
      sourceId: string,
      { projectId, expectedRevision }: { projectId: string; expectedRevision: number },
    ) => ({
      id: sourceId,
      conversationId: 'conversation-1',
      projectId,
      fileName: 'measurements.csv',
      mediaType: 'text/csv',
      sizeBytes: 24,
      status: 'archived',
      revision: expectedRevision + 1,
      createdAt: '2026-07-23T01:00:00Z',
      expiresAt: '2999-08-22T01:00:00Z',
    }))
    mocks.deleteAiSource.mockResolvedValue(undefined)
    ai.selectedProjectId.value = 'project-1'
    ai.conversationFilter.value = { archive: 'active', limit: 100 }
    ai.pendingDrafts.value = []
    ai.newConversation()
  })

  it('clears owner-scoped state and ignores an in-flight turn when identity changes', async () => {
    currentAuthSession.value = session('user-a')
    ai = useAiAssistant()
    ai.selectedProjectId.value = 'project-a'
    ai.projects.value = [{ id: 'project-a', name: 'Private project A' }]
    ai.projectsLoaded.value = true
    ai.conversations.value = [conversationSummary('project-a')]
    ai.pendingDrafts.value = [measurementDraft()]
    ai.sources.value = [{
      clientId: 'source-a',
      sourceId: 'source-a',
      projectId: 'project-a',
      fileName: 'private-a.csv',
      mediaType: 'text/csv',
      sizeBytes: 24,
      status: 'ready',
      revision: 1,
      expiresAt: '2999-08-22T01:00:00Z',
    }]
    let resolveTurn!: (response: AiTurnResponse) => void
    mocks.aiTurn.mockImplementationOnce(() => new Promise<AiTurnResponse>((resolve) => {
      resolveTurn = resolve
    }))

    const pendingTurn = ai.send('查询 A 的私有数据')
    await vi.waitFor(() => expect(mocks.aiTurn).toHaveBeenCalledTimes(1))
    currentAuthSession.value = undefined
    currentAuthSession.value = session('user-b')

    expect(ai.selectedProjectId.value).toBeUndefined()
    expect(ai.projects.value).toEqual([])
    expect(ai.projectsLoaded.value).toBe(false)
    expect(ai.conversations.value).toEqual([])
    expect(ai.pendingDrafts.value).toEqual([])
    expect(ai.sources.value).toEqual([])
    expect(ai.messages.value).toHaveLength(1)
    expect(ai.messages.value[0]?.content).not.toContain('私有数据')

    resolveTurn(turnResponse('conversation-a'))
    await pendingTurn

    expect(ai.conversationId.value).toBeUndefined()
    expect(ai.pendingDrafts.value).toEqual([])
    expect(ai.messages.value).toHaveLength(1)
    expect(ai.messages.value[0]?.content).not.toContain('查询完成')
  })

  it('uses the real gateway and carries the server conversation id into later turns', async () => {
    await ai.send('总结实验进度')
    await ai.send('哪些动物缺少体重？')

    expect(mocks.aiTurn).toHaveBeenNthCalledWith(1, {
      conversationId: undefined,
      projectId: 'project-1',
      message: '总结实验进度',
    })
    expect(mocks.aiTurn).toHaveBeenNthCalledWith(2, {
      conversationId: 'conversation-1',
      projectId: 'project-1',
      message: '哪些动物缺少体重？',
    })
    expect(ai.messages.value.at(-1)).toEqual(expect.objectContaining({
      role: 'assistant', content: '查询完成',
    }))
    expect(ai.messages.value.at(-1)?.citations?.[0].entityId).toBe('animal-1')
  })

  it('turns provider failures into an explicit non-demo assistant error', async () => {
    mocks.aiTurn.mockRejectedValueOnce(new Error('请先启用 AI 并配置所需密钥'))

    await ai.send('查询动物')

    expect(ai.messages.value.at(-1)).toEqual(expect.objectContaining({
      role: 'assistant', error: true, content: '请先启用 AI 并配置所需密钥',
    }))
    expect(mocks.listAiConversations).not.toHaveBeenCalled()
  })

  it('keeps successful partial results visible when the bounded tool loop stops', async () => {
    mocks.aiTurn.mockResolvedValueOnce({
      ...turnResponse(),
      content: '已保留本轮完成的查询结果。',
      incompleteReason: 'tool_call_limit_exceeded',
    })

    await ai.send('汇总全部项目状态')

    expect(ai.messages.value.at(-1)).toEqual(expect.objectContaining({
      role: 'assistant',
      content: '已保留本轮完成的查询结果。',
      incompleteReason: 'tool_call_limit_exceeded',
    }))
    expect(ai.messages.value.at(-1)?.error).toBeFalsy()
  })

  it('uploads supported research files and sends only opaque source references', async () => {
    const file = new File(['animal_id,weight\\nM-001,23.4'], 'measurements.csv', {
      type: 'text/csv',
    })

    await ai.addFiles([file])

    expect(mocks.uploadAiSource).toHaveBeenCalledWith({
      file,
      conversationId: 'conversation-1',
      projectId: 'project-1',
    })
    expect(ai.sources.value).toEqual([
      expect.objectContaining({
        sourceId: 'source-1',
        fileName: 'measurements.csv',
        status: 'ready',
      }),
    ])

    await ai.send('检查后生成录入预览')

    expect(mocks.aiTurn).toHaveBeenCalledWith({
      conversationId: 'conversation-1',
      projectId: 'project-1',
      message: '检查后生成录入预览',
      sourceRefs: ['source-1'],
    })
    expect(ai.sources.value).toEqual([])
    expect(ai.messages.value[1]?.sources).toEqual([
      expect.objectContaining({ sourceId: 'source-1', fileName: 'measurements.csv' }),
    ])
  })

  it('keeps explicit archive separate from removing a temporary source', async () => {
    const file = new File(['animal_id,weight\\nM-001,23.4'], 'measurements.csv', {
      type: 'text/csv',
    })
    await ai.addFiles([file])
    const source = ai.sources.value[0]!

    await ai.archiveSource(source.clientId)

    expect(mocks.archiveAiSource).toHaveBeenCalledWith('source-1', {
      projectId: 'project-1',
      expectedRevision: 1,
    })
    expect(mocks.deleteAiSource).not.toHaveBeenCalled()
    expect(ai.sources.value).toEqual([])
  })

  it('creates exactly one real conversation before concurrent first-file uploads', async () => {
    const first = new File(['id,value\nM-1,1'], 'first.csv', { type: 'text/csv' })
    const second = new File(['id,value\nM-2,2'], 'second.csv', { type: 'text/csv' })
    mocks.uploadAiSource
      .mockImplementationOnce(async ({ file, conversationId, projectId }) => ({
        id: 'source-first',
        conversationId,
        projectId,
        fileName: file.name,
        mediaType: file.type,
        sizeBytes: file.size,
        status: 'ready',
        revision: 1,
        createdAt: '2026-07-23T01:00:00Z',
        expiresAt: '2999-08-22T01:00:00Z',
      }))
      .mockImplementationOnce(async ({ file, conversationId, projectId }) => ({
        id: 'source-second',
        conversationId,
        projectId,
        fileName: file.name,
        mediaType: file.type,
        sizeBytes: file.size,
        status: 'ready',
        revision: 1,
        createdAt: '2026-07-23T01:00:00Z',
        expiresAt: '2999-08-22T01:00:00Z',
      }))

    await ai.addFiles([first, second])

    expect(mocks.createAiConversation).toHaveBeenCalledTimes(1)
    expect(mocks.createAiConversation).toHaveBeenCalledWith({
      projectId: 'project-1',
      title: '新对话',
    })
    expect(mocks.uploadAiSource).toHaveBeenCalledTimes(2)
    expect(mocks.uploadAiSource.mock.calls.map((call) => call[0].conversationId))
      .toEqual(['conversation-1', 'conversation-1'])
    expect(ai.conversationId.value).toBe('conversation-1')
  })

  it('never binds an upload to a conversation created for a stale project scope', async () => {
    let resolveFirst!: (value: AiConversationSummary) => void
    mocks.createAiConversation
      .mockImplementationOnce(() => new Promise<AiConversationSummary>((resolve) => {
        resolveFirst = resolve
      }))
      .mockResolvedValueOnce({
        ...conversationSummary('project-2'),
        id: 'conversation-2',
      })
    mocks.listAiConversations.mockResolvedValue([])
    const staleUpload = ai.addFiles([
      new File(['M-1'], 'stale.csv', { type: 'text/csv' }),
    ])
    await vi.waitFor(() => expect(mocks.createAiConversation).toHaveBeenCalledTimes(1))

    await ai.selectProject('project-2')
    await ai.addFiles([
      new File(['M-2'], 'current.csv', { type: 'text/csv' }),
    ])
    resolveFirst(conversationSummary('project-1'))
    await staleUpload

    expect(mocks.createAiConversation).toHaveBeenCalledTimes(2)
    expect(mocks.createAiConversation).toHaveBeenNthCalledWith(2, {
      projectId: 'project-2',
      title: '新对话',
    })
    expect(mocks.uploadAiSource).toHaveBeenCalledTimes(1)
    expect(mocks.uploadAiSource).toHaveBeenCalledWith(expect.objectContaining({
      conversationId: 'conversation-2',
      projectId: 'project-2',
    }))
    expect(ai.conversationId.value).toBe('conversation-2')
  })

  it('rejects unsupported or oversized files before calling the source gateway', async () => {
    const unsupported = new File(['binary'], 'unsafe.exe', {
      type: 'application/octet-stream',
    })

    await ai.addFiles([unsupported])

    expect(mocks.uploadAiSource).not.toHaveBeenCalled()
    expect(ai.sources.value).toEqual([
      expect.objectContaining({
        fileName: 'unsafe.exe',
        status: 'error',
        error: expect.stringContaining('不支持此格式'),
      }),
    ])
  })

  it('restores persisted messages and only currently usable ready sources after refresh', async () => {
    const draft = measurementDraft()
    const detail = conversationDetail()
    detail.messages[0].sourceRefs = [{
      sourceId: 'source-history-1',
      sourceRevision: 2,
      fileName: 'historical-weights.csv',
      mediaType: 'text/csv',
      sizeBytes: 2048,
    }]
    detail.messages[1].response = {
      ...turnResponse(),
      drafts: [draft],
      incompleteReason: 'provider_failure',
    }
    mocks.listAiConversations.mockResolvedValue([conversationSummary()])
    mocks.getAiConversation.mockResolvedValue(detail)
    const sourceBase = {
      conversationId: 'conversation-1',
      projectId: 'project-1',
      mediaType: 'text/csv',
      sizeBytes: 128,
      revision: 1,
      createdAt: '2026-07-23T01:00:00Z',
      expiresAt: '2999-08-22T01:00:00Z',
    }
    mocks.listAiSources.mockResolvedValue([
      { ...sourceBase, id: 'source-ready', fileName: 'ready.csv', status: 'ready' },
      {
        ...sourceBase,
        id: 'source-history-1',
        fileName: 'historical-weights.csv',
        status: 'ready',
      },
      { ...sourceBase, id: 'source-archived', fileName: 'archived.csv', status: 'archived' },
      { ...sourceBase, id: 'source-failed', fileName: 'failed.csv', status: 'failed' },
      { ...sourceBase, id: 'source-expired', fileName: 'expired.csv', status: 'expired' },
      {
        ...sourceBase,
        id: 'source-past-expiry',
        fileName: 'past-expiry.csv',
        status: 'ready',
        expiresAt: '2000-01-01T00:00:00Z',
      },
    ])

    await ai.restoreLatestConversation(true)

    expect(mocks.listAiConversations).toHaveBeenCalledWith(undefined, 100)
    expect(mocks.getAiConversation).toHaveBeenCalledWith('conversation-1', 200)
    expect(mocks.listAiSources).toHaveBeenCalledWith({
      conversationId: 'conversation-1',
      projectId: 'project-1',
    })
    expect(ai.conversationId.value).toBe('conversation-1')
    expect(ai.messages.value).toHaveLength(2)
    expect(ai.messages.value[1]).toEqual(expect.objectContaining({
      id: 'message-2', role: 'assistant', content: '查询完成',
      citations: [expect.objectContaining({ entityId: 'animal-1' })],
      drafts: [expect.objectContaining({ id: 'draft-1' })],
      incompleteReason: 'provider_failure',
    }))
    expect(ai.messages.value[0]?.sources).toEqual([{
      sourceId: 'source-history-1',
      fileName: 'historical-weights.csv',
      mediaType: 'text/csv',
      sizeBytes: 2048,
    }])
    expect(ai.sources.value).toEqual([
      expect.objectContaining({
        sourceId: 'source-ready',
        fileName: 'ready.csv',
        status: 'ready',
      }),
    ])
    expect(ai.pendingDrafts.value).toEqual([expect.objectContaining({ id: 'draft-1' })])

    await ai.send('继续检查当前可用文件')
    expect(mocks.aiTurn).toHaveBeenLastCalledWith({
      conversationId: 'conversation-1',
      projectId: 'project-1',
      message: '继续检查当前可用文件',
      sourceRefs: ['source-ready'],
    })
  })

  it('keeps the latest opened conversation when autonomy responses resolve out of order', async () => {
    const firstDetail = conversationDetail()
    const secondDetail = conversationDetail()
    secondDetail.conversation = {
      ...conversationSummary(),
      id: 'conversation-2',
      title: '第二个会话',
      revision: 4,
    }
    secondDetail.messages = [{
      id: 'message-second',
      sequence: 1,
      role: 'assistant',
      content: '第二个会话内容',
      createdAt: '2026-07-24T01:00:00Z',
    }]
    mocks.getAiConversation.mockImplementation(async (id: string) =>
      id === 'conversation-1' ? firstDetail : secondDetail)
    let resolveFirstAutonomy!: (value: ReturnType<typeof autonomyView>) => void
    mocks.getAiAutonomy
      .mockImplementationOnce(() => new Promise((resolve) => {
        resolveFirstAutonomy = resolve
      }))
      .mockResolvedValueOnce(autonomyView('auto'))

    const firstOpen = ai.openConversation('conversation-1')
    await vi.waitFor(() => expect(mocks.getAiAutonomy).toHaveBeenCalledTimes(1))
    await ai.openConversation('conversation-2')
    resolveFirstAutonomy(autonomyView('full'))
    await firstOpen

    expect(ai.conversationId.value).toBe('conversation-2')
    expect(ai.currentConversation.value?.title).toBe('第二个会话')
    expect(ai.messages.value).toEqual([
      expect.objectContaining({ id: 'message-second', content: '第二个会话内容' }),
    ])
    expect(ai.autonomy.value.mode).toBe('auto')
  })

  it('does not let a deferred conversation open overwrite a newly reset scope', async () => {
    let resolveDetail!: (value: AiConversationDetail) => void
    mocks.getAiConversation.mockImplementationOnce(() =>
      new Promise<AiConversationDetail>((resolve) => {
        resolveDetail = resolve
      }))

    const opening = ai.openConversation('conversation-1')
    await vi.waitFor(() => expect(ai.loadingConversation.value).toBe(true))
    ai.newConversation()

    expect(ai.loadingConversation.value).toBe(false)
    expect(ai.busy.value).toBe(false)
    resolveDetail(conversationDetail())
    await opening

    expect(ai.conversationId.value).toBeUndefined()
    expect(ai.currentConversation.value).toBeUndefined()
    expect(ai.messages.value).toHaveLength(1)
    expect(ai.messages.value[0]?.id).toBe('welcome')
    expect(ai.loadingConversation.value).toBe(false)
  })

  it('ignores a deferred turn after starting a new conversation scope', async () => {
    let resolveTurn!: (value: AiTurnResponse) => void
    mocks.aiTurn.mockImplementationOnce(() => new Promise<AiTurnResponse>((resolve) => {
      resolveTurn = resolve
    }))

    const sending = ai.send('旧会话中的查询')
    await vi.waitFor(() => expect(mocks.aiTurn).toHaveBeenCalledOnce())
    ai.newConversation()
    resolveTurn(turnResponse('stale-conversation'))
    await sending

    expect(ai.conversationId.value).toBeUndefined()
    expect(ai.messages.value).toHaveLength(1)
    expect(ai.messages.value[0]?.id).toBe('welcome')
    expect(ai.busy.value).toBe(false)
  })

  it('keeps the newly opened conversation isolated from an older deferred turn', async () => {
    let resolveTurn!: (value: AiTurnResponse) => void
    mocks.aiTurn.mockImplementationOnce(() => new Promise<AiTurnResponse>((resolve) => {
      resolveTurn = resolve
    }))
    const oldTurn = ai.send('旧会话中的查询')
    await vi.waitFor(() => expect(mocks.aiTurn).toHaveBeenCalledOnce())

    const detail = conversationDetail()
    detail.conversation = {
      ...detail.conversation,
      id: 'conversation-2',
      title: '当前会话',
    }
    detail.messages = [{
      id: 'message-current',
      sequence: 1,
      role: 'assistant',
      content: '当前会话内容',
      createdAt: '2026-07-24T01:00:00Z',
    }]
    mocks.getAiConversation.mockResolvedValueOnce(detail)
    await ai.openConversation('conversation-2')
    resolveTurn(turnResponse('conversation-stale'))
    await oldTurn

    expect(ai.conversationId.value).toBe('conversation-2')
    expect(ai.messages.value).toEqual([
      expect.objectContaining({ id: 'message-current', content: '当前会话内容' }),
    ])
    expect(ai.messages.value.some((message) => message.pending)).toBe(false)
    expect(ai.busy.value).toBe(false)
  })

  it('ignores a deferred autonomy update after the conversation scope changes', async () => {
    await ai.openConversation('conversation-1')
    let resolveAutonomy!: (value: ReturnType<typeof autonomyView>) => void
    mocks.setAiAutonomy.mockImplementationOnce(() => new Promise((resolve) => {
      resolveAutonomy = resolve
    }))

    const updating = ai.updateAutonomy('auto')
    await vi.waitFor(() => expect(ai.autonomyBusy.value).toBe(true))
    ai.newConversation()
    resolveAutonomy(autonomyView('full'))
    await updating

    expect(ai.conversationId.value).toBeUndefined()
    expect(ai.autonomy.value.mode).toBe('ask')
    expect(ai.autonomyBusy.value).toBe(false)
  })

  it('ignores a conversation autonomy response after the authenticated identity changes', async () => {
    currentAuthSession.value = session('open-user-a')
    ai = useAiAssistant()
    let resolveAutonomy!: (value: ReturnType<typeof autonomyView>) => void
    mocks.getAiAutonomy.mockImplementationOnce(() => new Promise((resolve) => {
      resolveAutonomy = resolve
    }))

    const opening = ai.openConversation('conversation-1')
    await vi.waitFor(() => expect(mocks.getAiAutonomy).toHaveBeenCalledTimes(1))
    currentAuthSession.value = undefined
    currentAuthSession.value = session('open-user-b')
    resolveAutonomy(autonomyView('full'))
    await opening

    expect(ai.conversationId.value).toBeUndefined()
    expect(ai.currentConversation.value).toBeUndefined()
    expect(ai.autonomy.value.mode).toBe('ask')
    expect(ai.messages.value).toHaveLength(1)
  })

  it('keeps archived conversations visible but blocks all conversation writes until restore', async () => {
    const detail = conversationDetail()
    detail.conversation = {
      ...detail.conversation,
      archivedAt: '2026-07-24T01:00:00Z',
    }
    mocks.getAiConversation.mockResolvedValueOnce(detail)
    mocks.updateAiConversation.mockResolvedValueOnce({
      ...detail.conversation,
      archivedAt: undefined,
      revision: detail.conversation.revision + 1,
    })

    await ai.openConversation(detail.conversation.id)

    expect(ai.conversationArchived.value).toBe(true)
    expect(ai.messages.value).toHaveLength(2)
    expect(mocks.listAiSources).not.toHaveBeenCalled()
    await ai.send('不应发送')
    await ai.addFiles([new File(['M-1'], 'blocked.csv', { type: 'text/csv' })])
    await expect(ai.updateAutonomy('auto')).rejects.toThrow('已归档会话为只读')
    await expect(ai.decideDraft(measurementDraft(), 'reject')).rejects.toThrow('已归档会话为只读')
    expect(mocks.aiTurn).not.toHaveBeenCalled()
    expect(mocks.uploadAiSource).not.toHaveBeenCalled()
    expect(mocks.setAiAutonomy).not.toHaveBeenCalled()
    expect(mocks.decideAiDraft).not.toHaveBeenCalled()

    await ai.updateConversation(detail.conversation, { action: 'unarchive' })

    expect(ai.conversationArchived.value).toBe(false)
    expect(ai.currentConversation.value?.archivedAt).toBeUndefined()
  })

  it('unloads ready sources without deleting them on conversation scope switches', async () => {
    const readySource = (clientId: string, sourceId: string) => ({
      clientId,
      sourceId,
      projectId: 'project-1',
      fileName: `${sourceId}.csv`,
      mediaType: 'text/csv',
      sizeBytes: 24,
      status: 'ready' as const,
      revision: 1,
      expiresAt: '2999-08-22T01:00:00Z',
    })

    ai.sources.value = [readySource('client-new', 'source-new')]
    mocks.deleteAiSource.mockClear()
    ai.newConversation()
    expect(ai.sources.value).toEqual([])

    ai.sources.value = [readySource('client-project', 'source-project')]
    mocks.listAiConversations.mockResolvedValueOnce([])
    await ai.selectProject('project-2')
    expect(ai.sources.value).toEqual([])

    ai.sources.value = [readySource('client-open', 'source-open')]
    const detail = conversationDetail()
    detail.conversation = { ...detail.conversation, id: 'conversation-open' }
    mocks.getAiConversation.mockResolvedValueOnce(detail)
    await ai.openConversation('conversation-open')
    expect(ai.sources.value).toEqual([])
    expect(mocks.deleteAiSource).not.toHaveBeenCalled()
  })

  it('deletes a ready source only when the user explicitly removes it', async () => {
    ai.sources.value = [{
      clientId: 'client-remove',
      sourceId: 'source-remove',
      projectId: 'project-1',
      fileName: 'remove.csv',
      mediaType: 'text/csv',
      sizeBytes: 24,
      status: 'ready',
      revision: 1,
      expiresAt: '2999-08-22T01:00:00Z',
    }]
    mocks.deleteAiSource.mockClear()

    await ai.removeSource('client-remove')

    expect(mocks.deleteAiSource).toHaveBeenCalledWith('source-remove')
    expect(ai.sources.value).toEqual([])
  })

  it('releases a historical message source once and preserves its display metadata', async () => {
    ai.messages.value = [{
      id: 'message-with-source',
      role: 'user',
      content: '检查历史来源',
      createdAt: '2026-07-23T10:00:00Z',
      sources: [{
        sourceId: 'source-history',
        fileName: 'weights.csv',
        mediaType: 'text/csv',
        sizeBytes: 2048,
      }],
    }]

    await ai.releaseMessageSource('message-with-source', 'source-history')
    await ai.releaseMessageSource('message-with-source', 'source-history')

    expect(mocks.deleteAiSource).toHaveBeenCalledTimes(1)
    expect(mocks.deleteAiSource).toHaveBeenCalledWith('source-history')
    expect(ai.messages.value[0]?.sources).toEqual([{
      sourceId: 'source-history',
      fileName: 'weights.csv',
      mediaType: 'text/csv',
      sizeBytes: 2048,
      released: true,
    }])
    expect(ai.messageSourceReleasing('source-history')).toBe(false)
  })

  it('keeps an archived historical source unreleased when the server rejects cleanup', async () => {
    ai.messages.value = [{
      id: 'message-with-archived-source',
      role: 'user',
      content: '已归档来源',
      createdAt: '2026-07-23T10:00:00Z',
      sources: [{
        sourceId: 'source-archived',
        fileName: 'formal-attachment.csv',
        mediaType: 'text/csv',
        sizeBytes: 4096,
      }],
    }]
    ai.currentConversation.value = {
      ...conversationSummary(),
      archivedAt: '2026-07-24T01:00:00Z',
    }
    mocks.deleteAiSource.mockRejectedValueOnce(
      new Error('archived AI source cannot be discarded'),
    )

    await expect(
      ai.releaseMessageSource('message-with-archived-source', 'source-archived'),
    ).rejects.toThrow('archived AI source cannot be discarded')

    expect(ai.messages.value[0]?.sources?.[0]).toEqual(expect.objectContaining({
      sourceId: 'source-archived',
      fileName: 'formal-attachment.csv',
    }))
    expect(ai.messages.value[0]?.sources?.[0]?.released).not.toBe(true)
    expect(ai.messageSourceReleasing('source-archived')).toBe(false)
  })

  it('does not issue source deletion after an unauthenticated identity reset', () => {
    currentAuthSession.value = session('source-user-a')
    ai = useAiAssistant()
    ai.sources.value = [{
      clientId: 'client-private',
      sourceId: 'source-private',
      projectId: 'project-1',
      fileName: 'private.csv',
      mediaType: 'text/csv',
      sizeBytes: 24,
      status: 'ready',
      revision: 1,
      expiresAt: '2999-08-22T01:00:00Z',
    }]
    mocks.deleteAiSource.mockClear()

    currentAuthSession.value = undefined

    expect(ai.sources.value).toEqual([])
    expect(mocks.deleteAiSource).not.toHaveBeenCalled()
  })

  it('clears drafts and refuses draft decisions in a lab-wide conversation', async () => {
    ai.selectedProjectId.value = undefined
    ai.pendingDrafts.value = [measurementDraft()]
    mocks.listAiDrafts.mockClear()

    await ai.refreshDrafts()

    expect(ai.pendingDrafts.value).toEqual([])
    expect(mocks.listAiDrafts).not.toHaveBeenCalled()

    const detail = conversationDetail()
    detail.conversation = { ...detail.conversation, projectId: undefined }
    detail.messages[1].response = {
      ...turnResponse(),
      drafts: [measurementDraft()],
    }
    mocks.getAiConversation.mockResolvedValueOnce(detail)
    await ai.openConversation('conversation-1')

    expect(ai.selectedProjectId.value).toBeUndefined()
    expect(ai.pendingDrafts.value).toEqual([])
    expect(ai.messages.value.flatMap((message) => message.drafts ?? [])).toEqual([])
    await expect(ai.decideDraft(measurementDraft(), 'reject')).rejects.toThrow(
      '只能审批当前科研项目',
    )
    expect(mocks.decideAiDraft).not.toHaveBeenCalled()
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

    expect(mocks.listAiConversations).toHaveBeenCalledWith('project-2', 100)
    expect(ai.selectedProjectId.value).toBe('project-2')
    expect(ai.conversationId.value).toBe('conversation-2')
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
    expect(mocks.decideAiDraft.mock.calls.at(-1)?.[1]).not.toHaveProperty('stepUpVerified')
    expect(mocks.decideAiDraft.mock.calls.at(-1)?.[1]).not.toHaveProperty('currentPassword')
  })

  it('fails closed when a bulk measurement draft lacks a confirmable import preview', async () => {
    const missingPreview = { ...reinforcedDraft(), importPreview: undefined }
    const blockedPreview = {
      ...reinforcedDraft(),
      importPreview: {
        ...reinforcedDraft().importPreview!,
        canConfirm: false,
      },
    }

    await expect(ai.decideDraft(
      missingPreview,
      'approve',
      '我已核对完整导入预览',
    )).rejects.toThrow('缺少正式导入预览')
    await expect(ai.decideDraft(
      blockedPreview,
      'approve',
      '我已核对完整导入预览',
    )).rejects.toThrow('尚未通过服务端校验')

    expect(mocks.decideAiDraft).not.toHaveBeenCalled()
  })

  it('rejects inconsistent truncation flags and error-severity import issues', async () => {
    const inconsistentFlags = {
      ...reinforcedDraft(),
      importPreview: {
        ...reinforcedDraft().importPreview!,
        totalRows: 3,
        acceptedRows: 3,
        previewRowsTruncated: false,
      },
    }
    const errorIssue = {
      ...reinforcedDraft(),
      importPreview: {
        ...reinforcedDraft().importPreview!,
        issueCount: 1,
        issuesTruncated: false,
        issues: [{
          row: 2,
          field: 'value',
          severity: 'error' as const,
          code: 'invalid_value',
          message: '数值无法解析',
        }],
      },
    }

    await expect(ai.decideDraft(
      inconsistentFlags,
      'approve',
      '我已核对完整导入预览',
    )).rejects.toThrow('统计不完整或不一致')
    await expect(ai.decideDraft(
      errorIssue,
      'approve',
      '我已核对完整导入预览',
    )).rejects.toThrow('仍包含错误')
    expect(mocks.decideAiDraft).not.toHaveBeenCalled()
  })

  it('requires and sends the current password only for a remote reinforced approval', async () => {
    mocks.mode = 'remote'
    ai = useAiAssistant()
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
    expect(mocks.decideAiDraft.mock.calls.at(-1)?.[1]).not.toHaveProperty('stepUpVerified')
  })

  it('never sends a password when rejecting a reinforced draft', async () => {
    mocks.mode = 'remote'
    ai = useAiAssistant()
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
