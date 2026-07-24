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

import type { AiAutonomyView, AiMessage, AiWriteDraft } from '@/domain/models'

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
  requirement: 'reinforced_confirmation',
  status: 'pending_approval',
  revision: 1,
  createdAt: '2026-07-18T02:00:00Z',
  expiresAt: '2026-07-19T02:00:00Z',
})

function assistantFixture(mode: 'local' | 'remote' = 'local') {
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
    messages: ref<AiMessage[]>([]),
    pendingDrafts: ref<AiWriteDraft[]>([]),
    conversationDrafts,
    composerDraft,
    stagedImages,
    imageStageError: ref<string>(),
    contextTitle: ref('实验数据'),
    selectedProject: computed(() => ({ id: 'project-1', name: 'DEMO' })),
    conversationId,
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
    disabledReason,
    composerDisabledReason: computed(() => disabledReason.value),
    conversationReadOnlyReason: computed(() => disabledReason.value),
    fullActivationRequired: computed(() =>
      !conversationId.value && requestedMode.value === 'full'),
    reinforcedPasswordRequired: computed(() => mode === 'remote'),
    send: vi.fn(),
    requestMode: vi.fn(async (selectedMode: 'ask' | 'auto' | 'full') => {
      requestedMode.value = selectedMode
    }),
    updateAutonomy: vi.fn(),
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
