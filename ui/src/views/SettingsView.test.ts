import { flushPromises, shallowMount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { builtinAiProviderPresets } from '@/services/aiProviderPresets'
import type { AiSettings, SaveAiSettingsInput } from '@/domain/models'
import SettingsView from './SettingsView.vue'

const gatewayMock = vi.hoisted(() => ({
  mode: 'remote' as const,
  displayName: '共享实验室',
  getAiSettings: vi.fn(),
  saveAiSettings: vi.fn(),
  clearAiApiKey: vi.fn(),
  listAiProviderPresets: vi.fn(),
  testAiSettings: vi.fn(),
}))

vi.mock('@/services/gateway', () => ({
  gateway: gatewayMock,
  currentAuthSession: {
    value: {
      user: {
        id: 'editor-1', labId: 'lab-1', displayName: 'Editor', labRoles: [],
        projectRoles: [], authentication: 'session', mustChangePassword: false,
        isEnvironmentRoot: false,
      },
    },
  },
  HttpGatewayError: class HttpGatewayError extends Error {
    constructor(readonly status: number, message: string, readonly code?: string) {
      super(message)
    }
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
    useDialog: () => ({ warning: vi.fn() }),
  }
})

const loadedSettings = (): AiSettings => ({
  enabled: true,
  providerKind: 'open_ai_compatible',
  providerPresetId: 'deepseek',
  model: 'deepseek-chat',
  baseUrl: 'https://api.deepseek.com',
  hasKey: true,
  supportsVision: false,
  contextWindowTokens: 131072,
  maxInputTokens: 65536,
  maxOutputTokens: 4096,
  historyTokenBudget: 32768,
  historyTurns: 20,
  temperature: 0,
  timeoutMs: 120000,
  revision: 1,
})

type SettingsVm = {
  ai: SaveAiSettingsInput
  apiKey: string
  hasApiKey: boolean
  saveAi: () => Promise<void>
  selectProvider: (presetId: string) => void
}

describe('SettingsView personal AI settings', () => {
  let persisted: AiSettings

  beforeEach(() => {
    persisted = loadedSettings()
    gatewayMock.getAiSettings.mockReset().mockImplementation(async () => structuredClone(persisted))
    gatewayMock.listAiProviderPresets.mockReset().mockResolvedValue(structuredClone(builtinAiProviderPresets))
    gatewayMock.saveAiSettings.mockReset().mockImplementation(async (input: SaveAiSettingsInput) => {
      const sameCredentialBinding = persisted.providerKind === input.providerKind
        && persisted.providerPresetId === input.providerPresetId
        && persisted.baseUrl.replace(/\/$/, '') === input.baseUrl.replace(/\/$/, '')
      persisted = {
        ...input,
        supportsVision: input.supportsVision ?? false,
        hasKey: Boolean(input.apiKey) || (sameCredentialBinding && persisted.hasKey),
        revision: persisted.revision + 1,
      }
      delete (persisted as AiSettings & { apiKey?: string }).apiKey
      return structuredClone(persisted)
    })
    gatewayMock.clearAiApiKey.mockReset()
    gatewayMock.testAiSettings.mockReset()
  })

  it('saves every advanced parameter, clears the plaintext field, and reloads persisted values', async () => {
    const first = shallowMount(SettingsView)
    await flushPromises()
    const vm = first.vm as unknown as SettingsVm
    vm.ai.contextWindowTokens = 262144
    vm.ai.maxInputTokens = 100000
    vm.ai.maxOutputTokens = 8192
    vm.ai.historyTokenBudget = 48000
    vm.ai.historyTurns = 12
    vm.ai.temperature = 0.6
    vm.ai.timeoutMs = 180000
    vm.apiKey = 'write-only-personal-key'

    await vm.saveAi()

    expect(gatewayMock.saveAiSettings).toHaveBeenCalledWith(expect.objectContaining({
      providerPresetId: 'deepseek',
      contextWindowTokens: 262144,
      maxInputTokens: 100000,
      maxOutputTokens: 8192,
      historyTokenBudget: 48000,
      historyTurns: 12,
      temperature: 0.6,
      timeoutMs: 180000,
      apiKey: 'write-only-personal-key',
    }))
    expect(vm.apiKey).toBe('')
    expect(first.html()).not.toContain('write-only-personal-key')
    first.unmount()

    const refreshed = shallowMount(SettingsView)
    await flushPromises()
    const refreshedVm = refreshed.vm as unknown as SettingsVm
    expect(refreshedVm.ai).toEqual(expect.objectContaining({
      contextWindowTokens: 262144,
      maxInputTokens: 100000,
      maxOutputTokens: 8192,
      historyTokenBudget: 48000,
      historyTurns: 12,
      temperature: 0.6,
      timeoutMs: 180000,
    }))
    expect(refreshedVm.apiKey).toBe('')
  })

  it('never reuses a stored key after switching Provider identity', async () => {
    const wrapper = shallowMount(SettingsView)
    await flushPromises()
    const vm = wrapper.vm as unknown as SettingsVm
    expect(vm.hasApiKey).toBe(true)

    vm.selectProvider('zhipu-glm')
    expect(vm.hasApiKey).toBe(false)
    await vm.saveAi()

    const input = gatewayMock.saveAiSettings.mock.calls.at(-1)?.[0] as SaveAiSettingsInput
    expect(input.providerPresetId).toBe('zhipu-glm')
    expect(input.model).toBe('glm-5.2')
    expect(input.baseUrl).toBe('https://open.bigmodel.cn/api/paas/v4')
    expect(input).not.toHaveProperty('apiKey')
    expect(persisted.hasKey).toBe(false)
    expect(vm.hasApiKey).toBe(false)
  })
})
