import { flushPromises, mount } from '@vue/test-utils'
import {
  create,
  NAlert,
  NButton,
  NCheckbox,
  NInput,
  NModal,
  NProgress,
  NSelect,
  NTag,
} from 'naive-ui'
import { computed, ref } from 'vue'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import type {
  AiAutonomyView,
  AiComposerSource,
  AiConversationSummary,
  AiMessage,
  AiWriteDraft,
} from '@/domain/models'

const mocks = vi.hoisted(() => ({
  assistant: null as unknown as ReturnType<typeof assistantFixture>,
  toastSuccess: vi.fn(),
  toastError: vi.fn(),
}))

vi.mock('@/composables/useAiAssistant', () => ({
  useAiAssistant: () => mocks.assistant,
}))

vi.mock('naive-ui', async (importOriginal) => {
  const actual = await importOriginal<typeof import('naive-ui')>()
  return {
    ...actual,
    useMessage: () => ({ success: mocks.toastSuccess, error: mocks.toastError }),
  }
})

import AiConversation from './AiConversation.vue'

const draft = (): AiWriteDraft => ({
  id: 'reinforced-draft-1',
  kind: 'bulk_import',
  projectId: 'project-1',
  changes: [{ path: '/data/imports/job-1', before: 'preview', after: 'committed' }],
  importPreview: {
    importKind: 'measurement',
    projectId: 'project-1',
    experimentId: 'experiment-1',
    fileName: 'measurements.csv',
    sheetName: 'Sheet1',
    totalRows: 25,
    acceptedRows: 25,
    issueCount: 22,
    issuesTruncated: true,
    canConfirm: true,
    previewRowsTruncated: true,
    previewRows: Array.from({ length: 20 }, (_, index) => ({
      rowNumber: index + 2,
      animalId: `animal-${index + 1}`,
      animalDisplayId: `M-${String(index + 1).padStart(3, '0')}`,
      measurementKey: 'body_weight',
      value: String(20 + index / 10),
      unit: 'g',
      measuredAt: `2026-07-18T01:${String(index).padStart(2, '0')}:00Z`,
    })),
    issues: Array.from({ length: 20 }, (_, index) => ({
      row: index + 2,
      field: 'unit',
      severity: 'warning',
      code: 'normalized_unit',
      message: `第 ${index + 1} 项单位已规范化`,
    })),
  },
  requirement: 'reinforced_confirmation',
  status: 'pending_approval',
  revision: 1,
  createdAt: '2026-07-18T02:00:00Z',
  expiresAt: '2026-07-19T02:00:00Z',
})

function assistantFixture(mode: 'local' | 'remote' = 'local') {
  const pendingDrafts = ref<AiWriteDraft[]>([draft()])
  const messages = ref<AiMessage[]>([])
  const conversationDrafts = ref<AiWriteDraft[]>([draft()])
  const conversationId = ref<string | undefined>('conversation-1')
  const requestedMode = ref<'ask' | 'auto' | 'full'>('ask')
  const composerDraft = ref('')
  const stagedImages = ref<Array<{
    localId: string
    file: File
    previewUrl: string
    status: 'staged'
  }>>([])
  const disabledReason = ref<string>()
  const autonomy = ref<AiAutonomyView>({
    mode: 'ask',
    effectiveMode: 'ask',
    maxMode: 'full',
    batchLimit: 1,
    revision: 0,
    requiresHumanApproval: [],
  })
  return {
    messages,
    pendingDrafts,
    conversationDrafts,
    composerDraft,
    stagedImages,
    imageStageError: ref<string>(),
    contextTitle: ref('实验数据'),
    selectedProjectId: ref<string | undefined>('project-1'),
    selectedProject: computed(() => ({ id: 'project-1', name: 'DEMO' })),
    conversationId,
    currentConversation: ref<AiConversationSummary>({
      id: 'conversation-1',
      projectId: 'project-1',
      title: '实验数据总结',
      modelProfileId: 'profile-1',
      modelProfileVersion: 2,
      modelProfileName: '主对话模型',
      modelId: 'model-primary',
      readOnly: false,
      createdAt: '2026-07-18T01:00:00Z',
      updatedAt: '2026-07-18T02:00:00Z',
      revision: 2,
    }),
    conversationArchived: ref(false),
    loadingConversation: ref(false),
    selectedModelProfileId: ref<string | undefined>('profile-1'),
    modelOptions: computed(() => [
      { label: '主对话模型 · model-primary', value: 'profile-1', disabled: false },
      { label: '备用模型 · model-secondary', value: 'profile-2', disabled: false },
    ]),
    selectedVisionModelProfileId: ref<string | undefined>('vision-profile-1'),
    visionModelOptions: computed(() => [
      { label: '视觉模型 · vision-model', value: 'vision-profile-1' },
    ]),
    visionRoute: computed(() => stagedImages.value.length ? 'relay' : 'none'),
    loadingModels: ref(false),
    requestedMode,
    autonomy,
    autonomyBusy: ref(false),
    startingConversation: ref(false),
    busy: ref(false),
    sources: ref<AiComposerSource[]>([]),
    sourceUploading: ref(false),
    readySourceCount: ref(0),
    disabledReason,
    composerDisabledReason: computed(() => disabledReason.value),
    conversationReadOnlyReason: computed(() => disabledReason.value),
    fullActivationRequired: computed(() =>
      !conversationId.value && requestedMode.value === 'full'),
    reinforcedPasswordRequired: computed(() => mode === 'remote'),
    addFiles: vi.fn(),
    removeSource: vi.fn(),
    archiveSource: vi.fn(),
    sourceArchiving: vi.fn(() => false),
    releaseMessageSource: vi.fn(async (_messageId: string, sourceId: string) => {
      messages.value = messages.value.map((message) => ({
        ...message,
        sources: message.sources?.map((source) =>
          source.sourceId === sourceId ? { ...source, released: true } : source),
      }))
    }),
    messageSourceReleasing: vi.fn(() => false),
    retrySource: vi.fn(),
    send: vi.fn(),
    requestMode: vi.fn(async (selectedMode: 'ask' | 'auto' | 'full') => {
      requestedMode.value = selectedMode
    }),
    updateAutonomy: vi.fn(),
    updateConversation: vi.fn(),
    conversationBusy: vi.fn(() => false),
    modelSwitchNeedsConfirmation: vi.fn(() => false),
    selectModel: vi.fn(),
    selectVisionModel: vi.fn(),
    stageImages: vi.fn(),
    removeStagedImage: vi.fn(),
    retainImageComposer: vi.fn(() => () => undefined),
    decideDraft: vi.fn().mockResolvedValue({
      draft: { ...conversationDrafts.value[0], status: 'applied', revision: 3 },
      jobId: 'job-1',
    }),
    draftBusy: vi.fn(() => false),
  }
}

const naive = create({
  components: [NAlert, NButton, NCheckbox, NInput, NModal, NProgress, NSelect, NTag],
})

async function fillStatementAndCheckbox(wrapper: ReturnType<typeof mount>) {
  await wrapper
    .get('[data-testid="reinforced-statement-reinforced-draft-1"] textarea')
    .setValue('我已核对导入预览、冲突和目标项目')
  await wrapper
    .get('[data-testid="reinforced-checkbox-reinforced-draft-1"]')
    .trigger('click')
  await flushPromises()
}

async function fillRemoteReinforcedForm(wrapper: ReturnType<typeof mount>) {
  await fillStatementAndCheckbox(wrapper)
  await wrapper
    .get('[data-testid="reinforced-password-reinforced-draft-1"] input')
    .setValue('one-request-password')
  await flushPromises()
}

describe('AiConversation reinforced approval', () => {
  beforeEach(() => {
    mocks.assistant = assistantFixture()
    mocks.toastSuccess.mockReset()
    mocks.toastError.mockReset()
  })

  it('requires only declaration and checkbox locally without rendering or passing a password', async () => {
    const wrapper = mount(AiConversation, {
      global: { plugins: [naive], stubs: { RouterLink: true } },
    })
    const approve = wrapper.get('[data-testid="approve-draft-reinforced-draft-1"]')
    expect(approve.attributes('disabled')).toBeDefined()
    expect(wrapper.find('[data-testid="reinforced-password-reinforced-draft-1"]').exists()).toBe(false)
    expect(wrapper.text()).toContain('本地原生边界')
    expect(wrapper.text()).toContain('项目 ID：project-1')

    await fillStatementAndCheckbox(wrapper)
    expect(approve.attributes('disabled')).toBeUndefined()
    await approve.trigger('click')
    await flushPromises()

    expect(mocks.assistant.decideDraft).toHaveBeenCalledWith(
      expect.objectContaining({ id: 'reinforced-draft-1' }),
      'approve',
      '我已核对导入预览、冲突和目标项目',
    )
  })

  it('renders at most 20 trusted measurement rows and explicitly reports truncation', () => {
    const wrapper = mount(AiConversation, {
      global: { plugins: [naive], stubs: { RouterLink: true } },
    })

    expect(wrapper.text()).toContain('批量测量导入草稿')
    expect(wrapper.text()).toContain('正式导入预览')
    expect(wrapper.text()).toContain('measurements.csv')
    expect(wrapper.text()).toContain('目标实验')
    expect(wrapper.findAll('.import-preview-table tbody tr')).toHaveLength(20)
    expect(wrapper.text()).toContain('仅显示前 20 行，共 25 条可接受记录')
    expect(wrapper.text()).toContain('仅显示 20 项，共 22 项')
  })

  it('keeps bulk approval disabled without a confirmable server preview', () => {
    mocks.assistant = assistantFixture()
    mocks.assistant.pendingDrafts.value = [{
      ...draft(),
      importPreview: undefined,
    }]
    const missing = mount(AiConversation, {
      global: { plugins: [naive], stubs: { RouterLink: true } },
    })

    expect(missing.text()).toContain('缺少正式导入预览')
    expect(
      missing.get('[data-testid="approve-draft-reinforced-draft-1"]').attributes('disabled'),
    ).toBeDefined()
    missing.unmount()

    mocks.assistant = assistantFixture()
    mocks.assistant.pendingDrafts.value = [{
      ...draft(),
      importPreview: { ...draft().importPreview!, canConfirm: false },
    }]
    const blocked = mount(AiConversation, {
      global: { plugins: [naive], stubs: { RouterLink: true } },
    })

    expect(blocked.text()).toContain('尚未通过服务端校验')
    expect(
      blocked.get('[data-testid="approve-draft-reinforced-draft-1"]').attributes('disabled'),
    ).toBeDefined()
  })

  it('requires the current password remotely and clears it after success', async () => {
    mocks.assistant = assistantFixture('remote')
    const wrapper = mount(AiConversation, {
      global: { plugins: [naive], stubs: { RouterLink: true } },
    })
    const approve = wrapper.get('[data-testid="approve-draft-reinforced-draft-1"]')
    await fillStatementAndCheckbox(wrapper)
    expect(approve.attributes('disabled')).toBeDefined()
    expect(wrapper.text()).toContain('当前登录账号的密码')

    await wrapper
      .get('[data-testid="reinforced-password-reinforced-draft-1"] input')
      .setValue('one-request-password')
    await approve.trigger('click')
    await flushPromises()

    expect(mocks.assistant.decideDraft).toHaveBeenCalledWith(
      expect.objectContaining({ id: 'reinforced-draft-1' }),
      'approve',
      '我已核对导入预览、冲突和目标项目',
      'one-request-password',
    )
    expect(
      wrapper.get('[data-testid="reinforced-password-reinforced-draft-1"] input').element,
    ).toHaveProperty('value', '')
    expect(wrapper.text()).not.toContain('one-request-password')
  })

  it('clears the remote password even when server verification fails', async () => {
    mocks.assistant = assistantFixture('remote')
    mocks.assistant.decideDraft.mockRejectedValueOnce(new Error('当前密码验证失败'))
    const wrapper = mount(AiConversation, {
      global: { plugins: [naive], stubs: { RouterLink: true } },
    })
    await fillRemoteReinforcedForm(wrapper)
    await wrapper.get('[data-testid="approve-draft-reinforced-draft-1"]').trigger('click')
    await flushPromises()

    expect(
      wrapper.get('[data-testid="reinforced-password-reinforced-draft-1"] input').element,
    ).toHaveProperty('value', '')
    expect(mocks.toastError).toHaveBeenCalledWith('当前密码验证失败')
  })

  it('labels a bounded partial answer without treating it as a failed message', () => {
    mocks.assistant = assistantFixture()
    mocks.assistant.messages.value = [{
      id: 'partial-1',
      role: 'assistant',
      content: '已完成前两项查询。',
      createdAt: '2026-07-23T10:00:00Z',
      incompleteReason: 'iteration_limit_exceeded',
      toolRuns: [{
        toolRunId: 'run-1',
        providerCallId: 'call-1',
        tool: 'animal_search',
        arguments: { q: 'M-1' },
        outcome: 'read',
        citations: [],
      }],
    }]

    const wrapper = mount(AiConversation, {
      global: { plugins: [naive], stubs: { RouterLink: true } },
    })

    expect(wrapper.text()).toContain('已完成前两项查询。')
    expect(wrapper.get('[aria-label="AI 部分结果提示"]').text()).toContain(
      '已保留本轮成功查询的部分结果',
    )
  })

  it('labels the opening location as display-only and avoids page-aware shortcut claims', () => {
    const wrapper = mount(AiConversation, {
      props: { compact: true },
      global: { plugins: [naive], stubs: { RouterLink: true } },
    })

    expect(wrapper.text()).toContain('打开入口：实验数据')
    expect(wrapper.text()).not.toContain('当前上下文')
    expect(wrapper.text()).toContain('已授权范围内有哪些异常？')
    expect(wrapper.text()).not.toContain('这个页面有哪些异常？')
  })

  it('does not claim partial results when a bounded turn ran no tools', () => {
    mocks.assistant = assistantFixture()
    mocks.assistant.messages.value = [{
      id: 'bounded-no-result-1',
      role: 'assistant',
      content: '本轮未能完成请求。',
      createdAt: '2026-07-23T10:00:00Z',
      incompleteReason: 'tool_call_limit_exceeded',
      toolRuns: [],
    }]

    const wrapper = mount(AiConversation, {
      global: { plugins: [naive], stubs: { RouterLink: true } },
    })
    const notice = wrapper.get('[aria-label="AI 未完成提示"]')

    expect(notice.text()).toContain('本轮未执行数据变更，请缩小范围重试')
    expect(notice.text()).not.toContain('已保留')
  })

  it('renders persisted source file name, media type and size on its original turn', () => {
    mocks.assistant = assistantFixture()
    mocks.assistant.messages.value = [{
      id: 'source-message-1',
      role: 'user',
      content: '检查历史文件',
      createdAt: '2026-07-23T10:00:00Z',
      sources: [{
        sourceId: 'source-1',
        fileName: 'weights.csv',
        mediaType: 'text/csv',
        sizeBytes: 2048,
      }],
    }]

    const wrapper = mount(AiConversation, {
      global: { plugins: [naive], stubs: { RouterLink: true } },
    })

    const sourceCard = wrapper.get('[aria-label="随消息发送的文件"]')
    expect(sourceCard.text()).toContain('weights.csv')
    expect(sourceCard.text()).toContain('text/csv')
    expect(sourceCard.text()).toContain('2.0 KiB')
  })

  it('releases a historical source once while keeping its message metadata visible', async () => {
    mocks.assistant = assistantFixture()
    mocks.assistant.messages.value = [{
      id: 'source-message-release',
      role: 'user',
      content: '检查历史文件',
      createdAt: '2026-07-23T10:00:00Z',
      sources: [{
        sourceId: 'source-1',
        fileName: 'weights.csv',
        mediaType: 'text/csv',
        sizeBytes: 2048,
      }],
    }]
    const wrapper = mount(AiConversation, {
      global: { plugins: [naive], stubs: { RouterLink: true } },
    })

    await wrapper.get('[aria-label="释放暂存文件 weights.csv"]').trigger('click')
    await flushPromises()
    expect(mocks.assistant.releaseMessageSource).not.toHaveBeenCalled()
    expect(wrapper.text()).toContain('任何仍依赖该文件的待审批导入将无法完成')
    await wrapper.get('[data-testid="confirm-release-source"]').trigger('click')
    await flushPromises()

    expect(mocks.assistant.releaseMessageSource).toHaveBeenCalledWith(
      'source-message-release',
      'source-1',
    )
    expect(wrapper.text()).toContain('weights.csv')
    expect(wrapper.text()).toContain('2.0 KiB')
    expect(wrapper.get('[aria-label="暂存文件 weights.csv 已释放"]').attributes('disabled'))
      .toBeDefined()
    expect(mocks.toastSuccess).toHaveBeenCalledWith(
      '暂存文件已释放；文件名和大小仍保留在消息记录中',
    )
  })

  it('keeps an archived source actionable and surfaces a server cleanup rejection', async () => {
    mocks.assistant = assistantFixture()
    mocks.assistant.conversationArchived.value = true
    mocks.assistant.currentConversation.value = {
      ...mocks.assistant.currentConversation.value!,
      archivedAt: '2026-07-24T01:00:00Z',
    }
    mocks.assistant.messages.value = [{
      id: 'source-message-archived',
      role: 'user',
      content: '检查正式附件来源',
      createdAt: '2026-07-23T10:00:00Z',
      sources: [{
        sourceId: 'source-archived',
        fileName: 'formal-attachment.csv',
        mediaType: 'text/csv',
        sizeBytes: 4096,
      }],
    }]
    mocks.assistant.releaseMessageSource.mockRejectedValueOnce(
      new Error('archived AI source cannot be discarded'),
    )
    const wrapper = mount(AiConversation, {
      global: { plugins: [naive], stubs: { RouterLink: true } },
    })

    const release = wrapper.get('[aria-label="释放暂存文件 formal-attachment.csv"]')
    await release.trigger('click')
    await flushPromises()
    expect(wrapper.text()).toContain('释放后原文件会从暂存区删除')
    await wrapper.get('[data-testid="confirm-release-source"]').trigger('click')
    await flushPromises()

    expect(release.attributes('disabled')).toBeUndefined()
    expect(wrapper.text()).toContain('formal-attachment.csv')
    expect(mocks.toastError).toHaveBeenCalledWith(
      'archived AI source cannot be discarded',
    )
  })

  it('shows citation revision and leaves route-less citations as plain text', () => {
    mocks.assistant = assistantFixture()
    mocks.assistant.messages.value = [{
      id: 'citation-message',
      role: 'assistant',
      content: '已找到项目动物关系。',
      createdAt: '2026-07-23T10:00:00Z',
      citations: [{
        entityType: 'project_animal_assignment',
        entityId: 'assignment-1',
        label: '项目动物关系 assignment',
        revision: 7,
      }],
    }]
    const wrapper = mount(AiConversation, {
      global: { plugins: [naive], stubs: { RouterLink: true } },
    })
    const citations = wrapper.get('[aria-label="数据引用"]')

    expect(citations.text()).toContain('项目动物关系 assignment · r7')
    expect(citations.find('a').exists()).toBe(false)
  })

  it('separates project attachment archive from destructive staged-file removal', async () => {
    mocks.assistant = assistantFixture()
    mocks.assistant.sources.value = [{
      clientId: 'client-source-1',
      sourceId: 'source-1',
      projectId: 'project-1',
      fileName: 'weights.csv',
      mediaType: 'text/csv',
      sizeBytes: 2048,
      status: 'ready',
      revision: 1,
      expiresAt: '2999-08-22T09:00:00Z',
    }]

    const wrapper = mount(AiConversation, {
      global: { plugins: [naive], stubs: { RouterLink: true } },
    })
    const archive = wrapper.get('[aria-label="归档 weights.csv 为项目附件"]')
    const remove = wrapper.get('[aria-label="移除并删除暂存文件 weights.csv"]')

    expect(archive.attributes('title')).toBe('归档为项目附件')
    expect(remove.attributes('title')).toBe('移除并删除暂存文件')
    await archive.trigger('click')
    await flushPromises()

    expect(mocks.assistant.archiveSource).toHaveBeenCalledWith('client-source-1')
    expect(mocks.assistant.removeSource).not.toHaveBeenCalled()
    expect(mocks.toastSuccess).toHaveBeenCalledWith(
      '来源已归档为项目附件，不会继续随下一轮发送',
    )
  })

  it('renders an archived conversation as read-only with a restore action', async () => {
    mocks.assistant = assistantFixture()
    mocks.assistant.conversationArchived.value = true
    mocks.assistant.currentConversation.value = {
      ...mocks.assistant.currentConversation.value!,
      archivedAt: '2026-07-24T01:00:00Z',
    }
    mocks.assistant.updateConversation.mockResolvedValue({
      ...mocks.assistant.currentConversation.value,
      archivedAt: undefined,
      revision: 3,
    })

    const wrapper = mount(AiConversation, {
      global: { plugins: [naive], stubs: { RouterLink: true } },
    })

    expect(wrapper.get('[aria-label="已归档会话只读"]').text()).toContain('业务交互只读')
    expect(wrapper.text()).toContain('已归档会话保持只读')
    expect(wrapper.find('textarea[aria-label="发送给 AI 的消息"]').exists()).toBe(false)
    expect(wrapper.find('[aria-label="添加数据、文档或图片"]').exists()).toBe(false)
    expect(wrapper.find('[data-testid="approve-draft-reinforced-draft-1"]').exists()).toBe(false)

    const restore = wrapper.findAll('button').find((button) => button.text().includes('恢复会话'))
    expect(restore).toBeDefined()
    await restore!.trigger('click')
    await flushPromises()

    expect(mocks.assistant.updateConversation).toHaveBeenCalledWith(
      expect.objectContaining({ id: 'conversation-1' }),
      { action: 'unarchive' },
    )
  })

  it('does not render project drafts while the conversation scope is lab-wide', () => {
    mocks.assistant = assistantFixture()
    mocks.assistant.selectedProjectId.value = undefined

    const wrapper = mount(AiConversation, {
      global: { plugins: [naive], stubs: { RouterLink: true } },
    })

    expect(wrapper.find('[aria-label="AI 写入草稿"]').exists()).toBe(false)
    expect(wrapper.text()).not.toContain('reinforced-draft-1')
  })
})

describe('AiConversation model and mode state', () => {
  beforeEach(() => {
    mocks.assistant = assistantFixture()
    mocks.toastSuccess.mockReset()
    mocks.toastError.mockReset()
  })

  it('shows requested and effective modes separately after an administrator downgrade', () => {
    mocks.assistant.requestedMode.value = 'full'
    mocks.assistant.autonomy.value = {
      ...mocks.assistant.autonomy.value,
      mode: 'full',
      effectiveMode: 'auto',
      maxMode: 'auto',
    }
    const wrapper = mount(AiConversation, {
      global: { plugins: [naive], stubs: { RouterLink: true } },
    })

    const status = wrapper.get('[data-testid="conversation-mode-status"]')
    expect(status.text()).toContain('请求 Full')
    expect(status.text()).toContain('实际 Auto')
    const fullOption = (wrapper.findAllComponents(NSelect)[1].props('options') as Array<{
      value: string
      disabled?: boolean
    }>).find((option) => option.value === 'full')
    expect(fullOption?.disabled).not.toBe(true)
  })

  it('keeps the shared prompt and does not send when a first Full activation is cancelled', async () => {
    mocks.assistant.conversationId.value = undefined
    mocks.assistant.requestedMode.value = 'full'
    mocks.assistant.composerDraft.value = '总结当前项目'
    const wrapper = mount(AiConversation, {
      global: { plugins: [naive], stubs: { RouterLink: true } },
    })

    expect(wrapper.get('[data-testid="conversation-mode-status"]').text()).toContain('Full（待启用）')
    await wrapper.get('[data-testid="ai-composer-send"]').trigger('click')
    await flushPromises()
    expect(wrapper.text()).toContain('以 Full 请求开始新会话')
    expect(wrapper.text()).toContain('原生边界确认本次启动声明')

    await wrapper.get('[data-testid="cancel-full-start"]').trigger('click')
    await flushPromises()
    expect(mocks.assistant.send).not.toHaveBeenCalled()
    expect(mocks.assistant.composerDraft.value).toBe('总结当前项目')
  })

  it('passes the one-request password only when confirming a remote first Full start', async () => {
    mocks.assistant = assistantFixture('remote')
    mocks.assistant.conversationId.value = undefined
    mocks.assistant.requestedMode.value = 'full'
    mocks.assistant.composerDraft.value = '总结当前项目'
    const wrapper = mount(AiConversation, {
      global: { plugins: [naive], stubs: { RouterLink: true } },
    })

    await wrapper.get('[data-testid="ai-composer-send"]').trigger('click')
    await wrapper.get('[data-testid="full-start-declaration"]').trigger('click')
    await wrapper.get('[data-testid="full-start-password"] input').setValue('one-request-password')
    await wrapper.get('[data-testid="confirm-full-start"]').trigger('click')
    await flushPromises()

    expect(mocks.assistant.send).toHaveBeenCalledWith('总结当前项目', {
      fullConfirmed: true,
      currentPassword: 'one-request-password',
    })
    expect(wrapper.text()).not.toContain('one-request-password')
  })

  it('leaves all state untouched when a persisted-message model switch is cancelled', async () => {
    mocks.assistant.modelSwitchNeedsConfirmation.mockReturnValue(true)
    mocks.assistant.composerDraft.value = '尚未发送的补充问题'
    const wrapper = mount(AiConversation, {
      global: { plugins: [naive], stubs: { RouterLink: true } },
    })
    const modelSelect = wrapper.findAllComponents(NSelect)[0]

    modelSelect.vm.$emit('update:value', 'profile-2')
    await flushPromises()
    expect(wrapper.text()).toContain('模型绑定不能修改')
    await wrapper.get('[data-testid="cancel-model-switch"]').trigger('click')
    await flushPromises()

    expect(mocks.assistant.selectModel).not.toHaveBeenCalled()
    expect(mocks.assistant.selectedModelProfileId.value).toBe('profile-1')
    expect(mocks.assistant.composerDraft.value).toBe('尚未发送的补充问题')
  })

  it('confirms a model switch as a new-session transition', async () => {
    mocks.assistant.modelSwitchNeedsConfirmation.mockReturnValue(true)
    const wrapper = mount(AiConversation, {
      global: { plugins: [naive], stubs: { RouterLink: true } },
    })
    const modelSelect = wrapper.findAllComponents(NSelect)[0]

    modelSelect.vm.$emit('update:value', 'profile-2')
    await flushPromises()
    await wrapper.get('[data-testid="confirm-model-switch"]').trigger('click')
    await flushPromises()

    expect(mocks.assistant.selectModel).toHaveBeenCalledWith('profile-2', true)
    expect(mocks.toastSuccess).toHaveBeenCalledWith(
      '已创建新的空会话，项目范围和未发送输入已保留',
    )
  })

  it('keeps unavailable history readable and clearly disables the composer', () => {
    mocks.assistant.disabledReason.value = '该会话使用的模型已归档，只能查看历史内容'
    mocks.assistant.messages.value = [{
      id: 'history-1',
      role: 'assistant',
      content: '历史回答仍可查看',
      createdAt: '2026-07-20T00:00:00Z',
    }]
    const wrapper = mount(AiConversation, {
      global: { plugins: [naive], stubs: { RouterLink: true } },
    })

    expect(wrapper.text()).toContain('历史回答仍可查看')
    expect(wrapper.get('[data-testid="composer-disabled-reason"]').text()).toContain('已归档')
    expect(wrapper.get('[data-testid="ai-composer-input"]').attributes('disabled')).toBeDefined()
    expect(wrapper.get('[data-testid="ai-composer-send"]').attributes('disabled')).toBeDefined()
    expect(wrapper.findAllComponents(NSelect)[1].props('disabled')).toBe(true)
  })

  it.each([375, 768, 1024, 1440])(
    'uses min-width-zero responsive control groups at %ipx',
    (width) => {
      Object.defineProperty(window, 'innerWidth', { configurable: true, value: width })
      const wrapper = mount(AiConversation, {
        global: { plugins: [naive], stubs: { RouterLink: true } },
      })

      expect(wrapper.find('.context-strip').exists()).toBe(true)
      expect(wrapper.find('.conversation-controls').exists()).toBe(true)
      expect(wrapper.find('.model-field').exists()).toBe(true)
      expect(wrapper.find('.mode-field').exists()).toBe(true)
      expect(wrapper.find('.mode-status').exists()).toBe(true)
      expect(wrapper.find('.input-wrap').exists()).toBe(true)
    },
  )
})
