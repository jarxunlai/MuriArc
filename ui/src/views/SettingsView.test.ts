import { flushPromises, shallowMount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import SettingsView from './SettingsView.vue'

const gatewayMock = vi.hoisted(() => ({
  mode: 'remote' as const,
  displayName: '共享实验室',
  getWorkspaceSettings: vi.fn(),
  saveWorkspaceSettings: vi.fn(),
}))

vi.mock('@/services/gateway', () => ({
  gateway: gatewayMock,
  currentAuthSession: {
    value: {
      user: {
        id: 'editor-1',
        labId: 'lab-1',
        displayName: 'Editor',
        email: 'editor@example.test',
        labRoles: [],
        projectRoles: [],
        authentication: 'session',
        mustChangePassword: false,
        isEnvironmentRoot: false,
      },
    },
  },
}))
vi.mock('@/services/dataGateway', () => ({ createDataGateway: () => ({}) }))
vi.mock('@/services/projectContext', () => ({ hasLabRegistryAccess: () => false }))
vi.mock('vue-router', () => ({ useRouter: () => ({ replace: vi.fn() }) }))
vi.mock('naive-ui', async (importOriginal) => {
  const actual = await importOriginal<typeof import('naive-ui')>()
  return {
    ...actual,
    useMessage: () => ({ error: vi.fn(), success: vi.fn(), warning: vi.fn() }),
  }
})

describe('SettingsView', () => {
  beforeEach(() => {
    gatewayMock.getWorkspaceSettings.mockReset().mockResolvedValue({
      operatorName: 'Editor',
      labName: '共享实验室',
    })
    gatewayMock.saveWorkspaceSettings.mockReset()
  })

  it('routes the AI settings tab to the multi-model list and detail component', async () => {
    const wrapper = shallowMount(SettingsView)
    await flushPromises()

    ;(wrapper.vm as unknown as { active: string }).active = 'ai'
    await wrapper.vm.$nextTick()

    expect(wrapper.findComponent({ name: 'AiModelProfilesSettings' }).exists()).toBe(true)
    expect(wrapper.text()).not.toContain('选择推荐模型或输入兼容模型 ID')
  })

  it('keeps the existing workspace settings behavior outside the AI tab', async () => {
    const wrapper = shallowMount(SettingsView)
    await flushPromises()

    expect(gatewayMock.getWorkspaceSettings).toHaveBeenCalledOnce()
    expect(wrapper.text()).toContain('工作空间')
    expect(wrapper.text()).toContain('共享实验室')
  })
})
