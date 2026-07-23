import { beforeEach, describe, expect, it, vi } from 'vitest'
import type {
  AiConversationDetail,
  AiConversationSummary,
  AiTurnResponse,
  AiWriteDraft,
} from '@/domain/models'

const mocks = vi.hoisted(() => ({
  mode: 'local' as 'local' | 'remote',
  listProjects: vi.fn(),
  listAiDrafts: vi.fn(),
  listAiConversations: vi.fn(),
  getAiConversation: vi.fn(),
  aiTurn: vi.fn(),
  decideAiDraft: vi.fn(),
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
})

const conversationSummary = (projectId = 'project-1'): AiConversationSummary => ({
  id: 'conversation-1',
  projectId,
  title: '总结实验进度',
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

describe('useAiAssistant', () => {
  let ai: ReturnType<typeof useAiAssistant>

  beforeEach(() => {
    vi.clearAllMocks()
    mocks.mode = 'local'
    ai = useAiAssistant()
    mocks.listProjects.mockResolvedValue([{ id: 'project-1', name: 'DEMO' }])
    mocks.listAiDrafts.mockResolvedValue([])
    mocks.listAiConversations.mockResolvedValue([])
    mocks.getAiConversation.mockResolvedValue(conversationDetail())
    mocks.aiTurn.mockResolvedValue(turnResponse())
    ai.selectedProjectId.value = 'project-1'
    ai.pendingDrafts.value = []
    ai.newConversation()
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

  it('restores persisted messages, citations and the conversation id after refresh', async () => {
    const draft = measurementDraft()
    const detail = conversationDetail()
    detail.messages[1].response = { ...turnResponse(), drafts: [draft] }
    mocks.listAiConversations.mockResolvedValue([conversationSummary()])
    mocks.getAiConversation.mockResolvedValue(detail)

    await ai.restoreLatestConversation(true)

    expect(mocks.listAiConversations).toHaveBeenCalledWith('project-1', 50)
    expect(mocks.getAiConversation).toHaveBeenCalledWith('conversation-1', 200)
    expect(ai.conversationId.value).toBe('conversation-1')
    expect(ai.messages.value).toHaveLength(2)
    expect(ai.messages.value[1]).toEqual(expect.objectContaining({
      id: 'message-2', role: 'assistant', content: '查询完成',
      citations: [expect.objectContaining({ entityId: 'animal-1' })],
      drafts: [expect.objectContaining({ id: 'draft-1' })],
    }))
    expect(ai.pendingDrafts.value).toEqual([expect.objectContaining({ id: 'draft-1' })])
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
