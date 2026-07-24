import { flushPromises, mount } from '@vue/test-utils'
import {
  create,
  NButton,
  NDropdown,
  NInput,
  NModal,
  NSelect,
  NSpin,
} from 'naive-ui'
import { ref } from 'vue'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import type { AiConversationSummary } from '@/domain/models'

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

import AiWorkspaceShell from './AiWorkspaceShell.vue'

const activeConversation = (): AiConversationSummary => ({
  id: 'conversation-active',
  projectId: 'project-1',
  title: '活动会话',
  readOnly: false,
  createdAt: '2026-07-24T01:00:00Z',
  updatedAt: '2026-07-24T02:00:00Z',
  revision: 2,
})

const archivedConversation = (): AiConversationSummary => ({
  ...activeConversation(),
  id: 'conversation-archived',
  title: '归档会话',
  archivedAt: '2026-07-24T03:00:00Z',
  revision: 4,
})

function assistantFixture() {
  const conversations = ref<AiConversationSummary[]>([
    activeConversation(),
    archivedConversation(),
  ])
  const conversationFilter = ref({
    archive: 'active' as 'active' | 'archived',
    limit: 100,
  })
  return {
    conversationFilter,
    projects: ref([{ id: 'project-1', name: 'DEMO' }]),
    conversations,
    conversationId: ref('conversation-active'),
    currentConversation: ref<AiConversationSummary>(activeConversation()),
    pendingDrafts: ref([]),
    loadingConversations: ref(false),
    loadingConversation: ref(false),
    loadProjects: vi.fn().mockResolvedValue(undefined),
    restoreLatestConversation: vi.fn().mockResolvedValue(undefined),
    refreshDrafts: vi.fn().mockResolvedValue(undefined),
    selectProject: vi.fn().mockResolvedValue(undefined),
    setConversationFilter: vi.fn().mockImplementation(async (
      input: Partial<{ archive: 'active' | 'archived'; limit: number }>,
    ) => {
      conversationFilter.value = { ...conversationFilter.value, ...input }
      return conversations.value
    }),
    openConversation: vi.fn().mockResolvedValue(undefined),
    newConversation: vi.fn(),
    updateConversation: vi.fn().mockImplementation(async (
      conversation: AiConversationSummary,
      input: { action: string },
    ) => ({
      ...conversation,
      pinnedAt: input.action === 'pin' ? '2026-07-24T04:00:00Z' : conversation.pinnedAt,
      archivedAt: input.action === 'unarchive' ? undefined : conversation.archivedAt,
      revision: conversation.revision + 1,
    })),
    conversationBusy: vi.fn(() => false),
  }
}

const naive = create({
  components: [NButton, NDropdown, NInput, NModal, NSelect, NSpin],
})

describe('AiWorkspaceShell mobile conversation management', () => {
  beforeEach(() => {
    mocks.assistant = assistantFixture()
    mocks.toastSuccess.mockReset()
    mocks.toastError.mockReset()
  })

  it('exposes a semantic mobile manager with new, search and archive controls', async () => {
    const wrapper = mount(AiWorkspaceShell, {
      global: {
        plugins: [naive],
        stubs: { AiConversation: true },
      },
    })
    await flushPromises()

    const toggle = wrapper.get('[aria-controls="mobile-ai-conversation-manager"]')
    expect(toggle.element.tagName).toBe('BUTTON')
    expect(toggle.attributes('aria-expanded')).toBe('false')

    await toggle.trigger('click')

    expect(toggle.attributes('aria-expanded')).toBe('true')
    const manager = wrapper.get('[aria-label="移动端 AI 会话管理"]')
    expect(manager.find('[aria-label="按标题搜索 AI 会话"]').exists()).toBe(true)
    expect(manager.get('[aria-label="移动端会话归档状态"]').text()).toContain('已归档')
    const newConversation = manager.get('[aria-label="新建 AI 会话"]')
    expect(newConversation.element.tagName).toBe('BUTTON')
    await newConversation.trigger('click')

    expect(mocks.assistant.newConversation).toHaveBeenCalledOnce()
    expect(wrapper.find('[aria-label="移动端 AI 会话管理"]').exists()).toBe(false)
  })

  it('exposes roving keyboard tabs and marks the current conversation', async () => {
    const wrapper = mount(AiWorkspaceShell, {
      attachTo: document.body,
      global: {
        plugins: [naive],
        stubs: { AiConversation: true },
      },
    })
    await flushPromises()

    const tablist = wrapper.get('[role="tablist"][aria-label="会话归档状态"]')
    const active = tablist.get('#ai-archive-tab-active-desktop')
    const archived = tablist.get('#ai-archive-tab-archived-desktop')
    expect(active.attributes('aria-selected')).toBe('true')
    expect(active.attributes('tabindex')).toBe('0')
    expect(archived.attributes('aria-selected')).toBe('false')
    expect(archived.attributes('tabindex')).toBe('-1')
    expect(wrapper.get('#ai-conversation-panel-desktop').attributes('aria-labelledby'))
      .toBe('ai-archive-tab-active-desktop')
    expect(wrapper.get('.conversation-main[aria-current="page"]').text()).toContain('活动会话')

    await active.trigger('keydown', { key: 'ArrowRight' })
    await flushPromises()

    expect(mocks.assistant.setConversationFilter).toHaveBeenCalledWith({ archive: 'archived' })
    expect(archived.attributes('aria-selected')).toBe('true')
    expect(archived.attributes('tabindex')).toBe('0')
    expect(document.activeElement).toBe(archived.element)
    expect(wrapper.get('#ai-conversation-panel-desktop').attributes('aria-labelledby'))
      .toBe('ai-archive-tab-archived-desktop')
    wrapper.unmount()
  })

  it('provides keyboard-triggerable rename, pin, archive and restore actions', async () => {
    const wrapper = mount(AiWorkspaceShell, {
      global: {
        plugins: [naive],
        stubs: { AiConversation: true },
      },
    })
    await wrapper.get('[aria-controls="mobile-ai-conversation-manager"]').trigger('click')

    const menus = wrapper.findAllComponents(NDropdown)
    const activeMenu = menus.find((menu) =>
      (menu.props('options') as Array<{ key: string }>).some((option) => option.key === 'archive'))
    const archivedMenu = menus.find((menu) =>
      (menu.props('options') as Array<{ key: string }>).some((option) => option.key === 'unarchive'))

    expect(activeMenu?.props('options')).toEqual(expect.arrayContaining([
      expect.objectContaining({ key: 'pin' }),
      expect.objectContaining({ key: 'rename' }),
      expect.objectContaining({ key: 'archive' }),
    ]))
    expect(archivedMenu?.props('options')).toEqual(expect.arrayContaining([
      expect.objectContaining({ key: 'rename' }),
      expect.objectContaining({ key: 'unarchive' }),
    ]))
    const managementButtons = wrapper.findAll('[aria-label^="管理会话："]')
    expect(managementButtons.length).toBeGreaterThanOrEqual(2)
    expect(managementButtons.every((button) => button.element.tagName === 'BUTTON')).toBe(true)

    activeMenu!.vm.$emit('select', 'pin')
    archivedMenu!.vm.$emit('select', 'unarchive')
    await flushPromises()

    expect(mocks.assistant.updateConversation).toHaveBeenCalledWith(
      expect.objectContaining({ id: 'conversation-active' }),
      { action: 'pin', title: undefined },
    )
    expect(mocks.assistant.updateConversation).toHaveBeenCalledWith(
      expect.objectContaining({ id: 'conversation-archived' }),
      { action: 'unarchive', title: undefined },
    )
  })
})
