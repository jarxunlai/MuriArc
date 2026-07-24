import { flushPromises, shallowMount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import type { AiProviderEndpoint, SaveAiProviderEndpointInput } from '@/services/gateway'
import AiAdminView from './AiAdminView.vue'

const gatewayMock = vi.hoisted(() => ({
  getAiDiagnostics: vi.fn(),
  getAiLabSettings: vi.fn(),
  listAiProviderEndpoints: vi.fn(),
  listAiProviderPresets: vi.fn(),
  saveAiProviderEndpoint: vi.fn(),
  saveAiLabSettings: vi.fn(),
  disableAiProviderEndpoint: vi.fn(),
}))

vi.mock('@/services/gateway', () => ({ gateway: gatewayMock }))
vi.mock('naive-ui', async (importOriginal) => {
  const actual = await importOriginal<typeof import('naive-ui')>()
  return {
    ...actual,
    useMessage: () => ({ error: vi.fn(), success: vi.fn(), warning: vi.fn() }),
  }
})

type AiAdminVm = {
  draft: SaveAiProviderEndpointInput
  editEndpoint: (endpoint: AiProviderEndpoint) => void
  saveEndpoint: () => Promise<void>
}

describe('AiAdminView provider endpoint editing', () => {
  const responsesEndpoint: AiProviderEndpoint = {
    id: 'endpoint-1',
    builtin: false,
    enabled: true,
    providerKind: 'open_ai_compatible',
    protocol: 'openai_responses',
    label: 'Responses 出口',
    baseUrl: 'https://api.example.test/v1',
    revision: 3,
  }

  beforeEach(() => {
    gatewayMock.getAiDiagnostics.mockReset().mockResolvedValue(undefined)
    gatewayMock.getAiLabSettings.mockReset().mockResolvedValue(undefined)
    gatewayMock.listAiProviderEndpoints.mockReset().mockResolvedValue([responsesEndpoint])
    gatewayMock.listAiProviderPresets.mockReset().mockResolvedValue([])
    gatewayMock.saveAiProviderEndpoint.mockReset().mockResolvedValue(responsesEndpoint)
    gatewayMock.saveAiLabSettings.mockReset()
    gatewayMock.disableAiProviderEndpoint.mockReset()
  })

  it('preserves a non-default protocol when only the endpoint label changes', async () => {
    const wrapper = shallowMount(AiAdminView)
    await flushPromises()
    const vm = wrapper.vm as unknown as AiAdminVm

    vm.editEndpoint(responsesEndpoint)
    vm.draft.label = '更新后的 Responses 出口'
    await vm.saveEndpoint()

    expect(gatewayMock.saveAiProviderEndpoint).toHaveBeenCalledWith({
      enabled: true,
      providerKind: 'open_ai_compatible',
      protocol: 'openai_responses',
      label: '更新后的 Responses 出口',
      baseUrl: 'https://api.example.test/v1',
    }, 'endpoint-1')
  })
})
