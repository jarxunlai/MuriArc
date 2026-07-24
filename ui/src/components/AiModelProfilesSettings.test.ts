import { flushPromises, shallowMount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type {
  AiModelDefaultsView,
  AiModelProfileView,
  SaveAiModelProfileInput,
  SaveAiModelDefaultsInput,
  ValidateAiModelProfileInput,
} from '@/services/gateway'
import AiModelProfilesSettings from './AiModelProfilesSettings.vue'

const warningDialog = vi.hoisted(() => vi.fn())
const messages = vi.hoisted(() => ({
  error: vi.fn(),
  success: vi.fn(),
  warning: vi.fn(),
}))
const gatewayMock = vi.hoisted(() => ({
  mode: 'remote' as const,
  listAiModelProfiles: vi.fn(),
  getAiModelProfile: vi.fn(),
  createAiModelProfile: vi.fn(),
  updateAiModelProfile: vi.fn(),
  validateAiModelProfile: vi.fn(),
  clearAiModelProfileKey: vi.fn(),
  archiveAiModelProfile: vi.fn(),
  getAiModelDefaults: vi.fn(),
  saveAiModelDefaults: vi.fn(),
}))

vi.mock('@/services/gateway', () => ({ gateway: gatewayMock }))
vi.mock('naive-ui', async (importOriginal) => {
  const actual = await importOriginal<typeof import('naive-ui')>()
  return {
    ...actual,
    useMessage: () => messages,
    useDialog: () => ({ warning: warningDialog }),
  }
})

const conversationProfile = (): AiModelProfileView => ({
  id: 'profile-chat',
  name: '主对话模型',
  currentVersion: 2,
  revision: 3,
  protocol: 'openai_chat_completions',
  transport: 'open_ai_compatible',
  baseUrl: 'https://api.example.test/v1',
  modelId: 'any-model-id',
  supportsVision: false,
  contextWindowTokens: 131072,
  maxInputTokens: 65536,
  maxOutputTokens: 4096,
  historyTokenBudget: 32768,
  historyTurns: 20,
  temperature: 0,
  timeoutMs: 120000,
  hasKey: true,
  isDefaultConversation: true,
  isDefaultVision: false,
})

const visionProfile = (): AiModelProfileView => ({
  ...conversationProfile(),
  id: 'profile-vision',
  name: '视觉模型',
  modelId: 'vision-free-form',
  supportsVision: true,
  hasKey: false,
  isDefaultConversation: false,
  isDefaultVision: true,
  transport: 'local_http',
  baseUrl: 'http://127.0.0.1:11434/v1',
})

type SettingsVm = {
  profiles: AiModelProfileView[]
  activeProfile?: AiModelProfileView
  defaults: AiModelDefaultsView
  mode: 'list' | 'detail'
  validationResult?: { ok: boolean; latencyMs: number; errorCode?: string }
  draft: {
    name: string
    protocol: AiModelProfileView['protocol']
    transport: AiModelProfileView['transport']
    baseUrl: string
    modelId: string
    supportsVision: boolean
    contextWindowTokens: number
    maxInputTokens: number
    maxOutputTokens: number
    historyTokenBudget: number
  }
  apiKey: string
  saveError: string
  openProfile: (profile: AiModelProfileView) => Promise<void>
  createProfile: () => Promise<void>
  saveProfile: () => Promise<void>
  validateProfile: () => Promise<void>
  setDefault: (purpose: 'conversation' | 'vision') => Promise<void>
  clearKey: () => void
  archiveProfile: () => void
}

describe('AiModelProfilesSettings', () => {
  let profiles: AiModelProfileView[]
  let defaults: AiModelDefaultsView

  beforeEach(() => {
    profiles = [conversationProfile(), visionProfile()]
    defaults = {
      defaultConversationProfileId: 'profile-chat',
      defaultVisionProfileId: 'profile-vision',
      revision: 4,
    }
    for (const method of Object.values(messages)) method.mockReset()
    warningDialog.mockReset()
    gatewayMock.listAiModelProfiles.mockReset().mockImplementation(async () => structuredClone(profiles))
    gatewayMock.getAiModelProfile.mockReset().mockImplementation(async (id: string) => {
      const profile = profiles.find((item) => item.id === id)
      if (!profile) throw new Error('not found')
      return structuredClone(profile)
    })
    gatewayMock.getAiModelDefaults.mockReset().mockImplementation(async () => structuredClone(defaults))
    gatewayMock.updateAiModelProfile.mockReset().mockImplementation(
      async (id: string, input: SaveAiModelProfileInput) => {
        const current = profiles.find((item) => item.id === id)!
        const { apiKey: _apiKey, expectedRevision: _expectedRevision, ...configuration } = input
        const updated = {
          ...current,
          ...configuration,
          currentVersion: current.currentVersion + 1,
          revision: current.revision + 1,
          hasKey: Boolean(input.apiKey) || current.hasKey,
        }
        profiles = profiles.map((item) => item.id === id ? updated : item)
        return structuredClone(updated)
      },
    )
    gatewayMock.createAiModelProfile.mockReset().mockImplementation(
      async (input: SaveAiModelProfileInput) => {
        const { apiKey: _apiKey, expectedRevision: _expectedRevision, ...configuration } = input
        const created: AiModelProfileView = {
          ...configuration,
          id: 'created-profile',
          currentVersion: 1,
          revision: 1,
          hasKey: Boolean(input.apiKey),
          isDefaultConversation: false,
          isDefaultVision: false,
        }
        profiles.push(created)
        return structuredClone(created)
      },
    )
    gatewayMock.validateAiModelProfile.mockReset().mockResolvedValue({ ok: true, latencyMs: 23 })
    gatewayMock.clearAiModelProfileKey.mockReset().mockImplementation(async (id: string) => ({
      ...profiles.find((item) => item.id === id)!,
      hasKey: false,
    }))
    gatewayMock.archiveAiModelProfile.mockReset().mockResolvedValue({
      ...conversationProfile(),
      archivedAt: '2026-07-23T00:00:00Z',
    })
    gatewayMock.saveAiModelDefaults.mockReset().mockImplementation(
      async (input: SaveAiModelDefaultsInput) => {
        defaults = {
          defaultConversationProfileId: input.defaultConversationProfileId ?? undefined,
          defaultVisionProfileId: input.defaultVisionProfileId ?? undefined,
          revision: input.expectedRevision + 1,
        }
        return structuredClone(defaults)
      },
    )
  })

  it('loads a model list with independent credential and default status', async () => {
    const wrapper = shallowMount(AiModelProfilesSettings)
    await flushPromises()

    expect(gatewayMock.listAiModelProfiles).toHaveBeenCalledOnce()
    expect(gatewayMock.getAiModelDefaults).toHaveBeenCalledOnce()
    expect(wrapper.text()).toContain('主对话模型')
    expect(wrapper.text()).toContain('视觉模型')
    expect(wrapper.text()).toContain('密钥已配置')
    expect(wrapper.text()).toContain('默认视觉')
  })

  it('preserves a stored key when only free-form model ID and runtime data change', async () => {
    const wrapper = shallowMount(AiModelProfilesSettings)
    await flushPromises()
    const vm = wrapper.vm as unknown as SettingsVm
    await vm.openProfile(vm.profiles[0]!)
    vm.draft.modelId = 'provider/custom-model:2026'

    await vm.saveProfile()

    const input = gatewayMock.updateAiModelProfile.mock.calls[0]![1] as SaveAiModelProfileInput
    expect(input).toEqual(expect.objectContaining({
      modelId: 'provider/custom-model:2026',
      expectedRevision: 3,
    }))
    expect(input).not.toHaveProperty('apiKey')
  })

  it('allows an existing keyless cloud profile to stay keyless when identity is unchanged', async () => {
    profiles[0]!.hasKey = false
    const wrapper = shallowMount(AiModelProfilesSettings)
    await flushPromises()
    const vm = wrapper.vm as unknown as SettingsVm
    await vm.openProfile(vm.profiles[0]!)
    vm.draft.name = '仍待配置密钥的模型'

    expect(vm.saveError).toBe('')
    await vm.saveProfile()

    const input = gatewayMock.updateAiModelProfile.mock.calls[0]![1] as SaveAiModelProfileInput
    expect(input).not.toHaveProperty('apiKey')
  })

  it('requires a new key after protocol, transport, or Base URL changes', async () => {
    const wrapper = shallowMount(AiModelProfilesSettings)
    await flushPromises()
    const vm = wrapper.vm as unknown as SettingsVm
    await vm.openProfile(vm.profiles[0]!)
    vm.draft.protocol = 'anthropic_messages'
    vm.draft.baseUrl = 'https://api.anthropic.example'

    expect(vm.saveError).toContain('必须重新输入 API Key')
    await vm.saveProfile()
    expect(gatewayMock.updateAiModelProfile).not.toHaveBeenCalled()

    vm.apiKey = 'write-only-new-key'
    await vm.saveProfile()
    expect(gatewayMock.updateAiModelProfile).toHaveBeenCalledWith(
      'profile-chat',
      expect.objectContaining({
        protocol: 'anthropic_messages',
        apiKey: 'write-only-new-key',
      }),
    )
    expect(wrapper.html()).not.toContain('write-only-new-key')
  })

  it('validates the unsaved form without creating or updating a profile', async () => {
    const wrapper = shallowMount(AiModelProfilesSettings)
    await flushPromises()
    const vm = wrapper.vm as unknown as SettingsVm
    await vm.openProfile(vm.profiles[0]!)
    vm.draft.modelId = 'unsaved-model-id'
    vm.apiKey = 'transient-validation-key'

    await vm.validateProfile()

    expect(gatewayMock.validateAiModelProfile).toHaveBeenCalledWith(expect.objectContaining({
      profileId: 'profile-chat',
      currentVersion: 2,
      modelId: 'unsaved-model-id',
      apiKey: 'transient-validation-key',
    } satisfies Partial<ValidateAiModelProfileInput>))
    expect(gatewayMock.createAiModelProfile).not.toHaveBeenCalled()
    expect(gatewayMock.updateAiModelProfile).not.toHaveBeenCalled()
    expect(vm.apiKey).toBe('transient-validation-key')
    expect(vm.validationResult?.ok).toBe(true)

    vm.draft.baseUrl = 'https://changed-after-validation.example'
    await wrapper.vm.$nextTick()
    expect(vm.validationResult).toBeUndefined()
  })

  it('can validate a new connection before the display name is chosen', async () => {
    const wrapper = shallowMount(AiModelProfilesSettings)
    await flushPromises()
    const vm = wrapper.vm as unknown as SettingsVm
    await vm.createProfile()
    Object.assign(vm.draft, {
      name: '',
      baseUrl: 'https://unsaved.example/v1',
      modelId: 'free-form-before-save',
    })
    vm.apiKey = 'transient-key'

    await vm.validateProfile()

    const input = gatewayMock.validateAiModelProfile.mock.calls[0]![0]
    expect(input).toEqual(expect.objectContaining({
      baseUrl: 'https://unsaved.example/v1',
      modelId: 'free-form-before-save',
      apiKey: 'transient-key',
    }))
    expect(input).not.toHaveProperty('name')
    expect(gatewayMock.createAiModelProfile).not.toHaveBeenCalled()
  })

  it('uses the shared 120-character model name limit', async () => {
    const wrapper = shallowMount(AiModelProfilesSettings)
    await flushPromises()
    const vm = wrapper.vm as unknown as SettingsVm
    await vm.createProfile()
    Object.assign(vm.draft, {
      name: 'a'.repeat(121),
      baseUrl: 'https://provider.example/v1',
      modelId: 'free-form-model',
    })
    vm.apiKey = 'write-only-key'

    expect(vm.saveError).toBe('配置名称不能超过 120 个字符。')
    await vm.saveProfile()
    expect(gatewayMock.createAiModelProfile).not.toHaveBeenCalled()

    vm.draft.name = 'a'.repeat(120)
    expect(vm.saveError).toBe('')
  })

  it('uses the shared 1,000,000-token history budget limit', async () => {
    const wrapper = shallowMount(AiModelProfilesSettings)
    await flushPromises()
    const vm = wrapper.vm as unknown as SettingsVm
    await vm.createProfile()
    Object.assign(vm.draft, {
      name: 'Large-context model',
      baseUrl: 'https://provider.example/v1',
      modelId: 'free-form-model',
      contextWindowTokens: 2_000_000,
      maxInputTokens: 1_500_000,
      maxOutputTokens: 4_096,
      historyTokenBudget: 1_000_001,
    })
    vm.apiKey = 'write-only-key'

    expect(vm.saveError).toContain('1,000,000')
    vm.draft.historyTokenBudget = 1_000_000
    expect(vm.saveError).toBe('')
  })

  it('renders the stable context-exceeded validation error', async () => {
    gatewayMock.validateAiModelProfile.mockResolvedValueOnce({
      ok: false,
      latencyMs: 23,
      errorCode: 'context_exceeded',
    })
    const wrapper = shallowMount(AiModelProfilesSettings)
    await flushPromises()
    const vm = wrapper.vm as unknown as SettingsVm
    await vm.openProfile(vm.profiles[0]!)

    await vm.validateProfile()

    expect(messages.error).toHaveBeenCalledWith(
      '连接验证失败：验证请求超过 Provider 上下文上限',
    )
    expect(wrapper.text()).toContain('验证请求超过 Provider 上下文上限')
  })

  it('updates the sole default vision model explicitly', async () => {
    const wrapper = shallowMount(AiModelProfilesSettings)
    await flushPromises()
    const vm = wrapper.vm as unknown as SettingsVm
    const nextVision = {
      ...conversationProfile(),
      id: 'profile-vision-next',
      name: '备用视觉',
      supportsVision: true,
      isDefaultConversation: false,
    }
    profiles.push(nextVision)
    await vm.openProfile(nextVision)

    await vm.setDefault('vision')

    expect(gatewayMock.saveAiModelDefaults).toHaveBeenCalledWith({
      defaultConversationProfileId: 'profile-chat',
      defaultVisionProfileId: 'profile-vision-next',
      expectedRevision: 4,
    })
  })

  it('sends explicit null when the user clears a default', async () => {
    const wrapper = shallowMount(AiModelProfilesSettings)
    await flushPromises()
    const vm = wrapper.vm as unknown as SettingsVm
    await vm.openProfile(vm.profiles[1]!)

    await vm.setDefault('vision')

    expect(gatewayMock.saveAiModelDefaults).toHaveBeenCalledWith({
      defaultConversationProfileId: 'profile-chat',
      defaultVisionProfileId: null,
      expectedRevision: 4,
    })
  })

  it('requires clearing the vision default before disabling vision capability', async () => {
    const wrapper = shallowMount(AiModelProfilesSettings)
    await flushPromises()
    const vm = wrapper.vm as unknown as SettingsVm
    await vm.openProfile(vm.profiles[1]!)
    vm.draft.supportsVision = false

    expect(vm.saveError).toContain('先取消默认视觉模型')
    await vm.saveProfile()
    expect(gatewayMock.updateAiModelProfile).not.toHaveBeenCalled()
  })

  it('creates a cloud profile only after receiving its write-only key', async () => {
    const wrapper = shallowMount(AiModelProfilesSettings)
    await flushPromises()
    const vm = wrapper.vm as unknown as SettingsVm
    await vm.createProfile()
    Object.assign(vm.draft, {
      name: '自由模型',
      protocol: 'openai_responses',
      transport: 'open_ai_compatible',
      baseUrl: 'https://provider.example/v1',
      modelId: 'research/model-unlisted',
    })

    await vm.saveProfile()
    expect(gatewayMock.createAiModelProfile).not.toHaveBeenCalled()

    vm.apiKey = 'create-only-secret'
    await vm.saveProfile()
    expect(gatewayMock.createAiModelProfile).toHaveBeenCalledWith(expect.objectContaining({
      modelId: 'research/model-unlisted',
      protocol: 'openai_responses',
      apiKey: 'create-only-secret',
    }))
  })

  it('confirms key clearing and soft archive as separate actions', async () => {
    const wrapper = shallowMount(AiModelProfilesSettings)
    await flushPromises()
    const vm = wrapper.vm as unknown as SettingsVm
    await vm.openProfile(vm.profiles[0]!)

    vm.clearKey()
    const clearConfirmation = warningDialog.mock.calls[0]![0] as {
      onPositiveClick: () => Promise<void>
    }
    await clearConfirmation.onPositiveClick()
    expect(gatewayMock.clearAiModelProfileKey).toHaveBeenCalledWith('profile-chat')

    vm.archiveProfile()
    const archiveConfirmation = warningDialog.mock.calls[1]![0] as {
      onPositiveClick: () => Promise<void>
    }
    await archiveConfirmation.onPositiveClick()
    expect(gatewayMock.archiveAiModelProfile).toHaveBeenCalledWith('profile-chat', 3)
  })
})
