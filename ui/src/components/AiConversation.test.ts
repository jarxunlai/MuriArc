import { flushPromises, mount } from '@vue/test-utils'
import { create, NAlert, NButton, NCheckbox, NInput, NModal, NSelect, NTag } from 'naive-ui'
import { computed, ref } from 'vue'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import type { AiWriteDraft } from '@/domain/models'

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
  const pendingDrafts = ref<AiWriteDraft[]>([draft()])
  return {
    messages: ref([]),
    pendingDrafts,
    contextTitle: ref('实验数据'),
    selectedProject: computed(() => ({ id: 'project-1', name: 'DEMO' })),
    conversationId: ref('conversation-1'),
    autonomy: ref({
      mode: 'ask' as const,
      effectiveMode: 'ask' as const,
      maxMode: 'full' as const,
      batchLimit: 1,
      revision: 0,
      requiresHumanApproval: [],
    }),
    autonomyBusy: ref(false),
    busy: ref(false),
    reinforcedPasswordRequired: computed(() => mode === 'remote'),
    send: vi.fn(),
    updateAutonomy: vi.fn(),
    decideDraft: vi.fn().mockResolvedValue({
      draft: { ...pendingDrafts.value[0], status: 'applied', revision: 3 },
      jobId: 'job-1',
    }),
    draftBusy: vi.fn(() => false),
  }
}

const naive = create({ components: [NAlert, NButton, NCheckbox, NInput, NModal, NSelect, NTag] })

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
