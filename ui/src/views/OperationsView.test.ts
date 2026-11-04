import { flushPromises, mount } from '@vue/test-utils'
import { describe, expect, it, vi } from 'vitest'
import OperationsView from './OperationsView.vue'

const gatewayMock = vi.hoisted(() => ({
  mode: 'remote' as const,
  listOperations: vi.fn(async (_query: URLSearchParams) => []),
}))

vi.mock('@/services/gateway', () => ({
  gateway: gatewayMock,
  currentAuthSession: { value: undefined },
}))
vi.mock('naive-ui', async (importOriginal) => {
  const actual = await importOriginal<typeof import('naive-ui')>()
  return { ...actual, useMessage: () => ({ error: vi.fn(), success: vi.fn() }) }
})

describe('OperationsView filters', () => {
  it('labels the default "all" filters and leaves them out of the query', async () => {
    const wrapper = mount(OperationsView, { global: { stubs: { PageHeader: true } } })
    await flushPromises()

    const filters = wrapper.get('.filters').text()
    expect(filters).toContain('全部活动')
    expect(filters).toContain('全部来源')
    const query = gatewayMock.listOperations.mock.calls[0]![0]
    expect(query.toString()).toBe('')
  })
})
