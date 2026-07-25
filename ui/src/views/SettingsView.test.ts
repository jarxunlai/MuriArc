import { flushPromises, shallowMount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import SettingsView from './SettingsView.vue'

const gatewayMock = vi.hoisted(() => ({
  mode: 'remote' as 'local' | 'remote',
  displayName: '共享实验室',
  getWorkspaceSettings: vi.fn(),
  saveWorkspaceSettings: vi.fn(),
  getLocalStorageStatus: vi.fn(),
  chooseLocalStorageDirectory: vi.fn(),
  requestLocalStorageMigration: vi.fn(),
  requestRestoreDefaultStorage: vi.fn(),
  openLocalStorageDirectory: vi.fn(),
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
vi.mock('@/services/projectContext', () => ({
  currentAuthSession: {
    value: {
      user: {
        id: 'editor-1', labId: 'lab-1', displayName: 'Editor', labRoles: [],
        projectRoles: [], authentication: 'session', mustChangePassword: false,
        isEnvironmentRoot: false,
      },
    },
  },
  hasLabRegistryAccess: () => false,
}))
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
    gatewayMock.mode = 'remote'
    gatewayMock.getWorkspaceSettings.mockReset().mockResolvedValue({
      operatorName: 'Editor',
      labName: '共享实验室',
    })
    gatewayMock.saveWorkspaceSettings.mockReset()
    gatewayMock.getLocalStorageStatus.mockReset()
    gatewayMock.chooseLocalStorageDirectory.mockReset()
    gatewayMock.requestLocalStorageMigration.mockReset()
    gatewayMock.requestRestoreDefaultStorage.mockReset()
    gatewayMock.openLocalStorageDirectory.mockReset()
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

  it('shows and schedules the complete data-root migration only in local mode', async () => {
    gatewayMock.mode = 'local'
    gatewayMock.getLocalStorageStatus.mockResolvedValue({
      activeDataRoot: 'C:\\Users\\Tester\\AppData\\Roaming\\org.muriarc.desktop',
      defaultDataRoot: 'C:\\Users\\Tester\\AppData\\Roaming\\org.muriarc.desktop',
      usesCustomRoot: false,
      migrationPending: false,
      requiresRestart: false,
    })
    gatewayMock.chooseLocalStorageDirectory.mockResolvedValue({
      selectionToken: 'selection-token-1',
      targetDataRoot: 'D:\\MuriArcData',
    })
    gatewayMock.requestLocalStorageMigration.mockResolvedValue({
      scheduled: true,
      requiresRestart: true,
      targetDataRoot: 'D:\\MuriArcData',
    })
    const wrapper = shallowMount(SettingsView)
    await flushPromises()

    ;(wrapper.vm as unknown as { active: string }).active = 'backup'
    await wrapper.vm.$nextTick()
    expect(wrapper.text()).toContain('Desktop 本地数据位置')
    expect(wrapper.text()).toContain('org.muriarc.desktop')

    const vm = wrapper.vm as unknown as {
      chooseLocalStorageDirectory: () => Promise<void>
      scheduleLocalStorageMigration: () => Promise<void>
    }
    await vm.chooseLocalStorageDirectory()
    await wrapper.vm.$nextTick()
    expect(wrapper.text()).toContain('D:\\MuriArcData')
    await vm.scheduleLocalStorageMigration()

    expect(gatewayMock.requestLocalStorageMigration).toHaveBeenCalledWith('selection-token-1')
  })

  it('does not expose Desktop paths in remote mode', async () => {
    const wrapper = shallowMount(SettingsView)
    await flushPromises()

    ;(wrapper.vm as unknown as { active: string }).active = 'backup'
    await wrapper.vm.$nextTick()

    expect(wrapper.text()).not.toContain('Desktop 本地数据位置')
    expect(gatewayMock.getLocalStorageStatus).not.toHaveBeenCalled()
  })

  it('shows the restart boundary for a pending local migration', async () => {
    gatewayMock.mode = 'local'
    gatewayMock.getLocalStorageStatus.mockResolvedValue({
      activeDataRoot: 'C:\\MuriArcDefault',
      defaultDataRoot: 'C:\\MuriArcDefault',
      usesCustomRoot: false,
      migrationPending: true,
      pendingTargetRoot: 'D:\\MuriArcData',
      requiresRestart: true,
    })
    const wrapper = shallowMount(SettingsView)
    await flushPromises()

    ;(wrapper.vm as unknown as { active: string }).active = 'backup'
    await wrapper.vm.$nextTick()

    expect(wrapper.text()).toContain('请先保存工作并完全退出 MuriArc')
    expect(wrapper.text()).toContain('D:\\MuriArcData')
  })
})
