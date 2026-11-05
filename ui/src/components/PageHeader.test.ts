import { mount } from '@vue/test-utils'
import { createMemoryHistory, createRouter } from 'vue-router'
import { describe, expect, it, vi } from 'vitest'
import PageHeader from './PageHeader.vue'

vi.mock('@/composables/useAiAssistant', () => ({ useAiAssistant: () => ({ open: vi.fn() }) }))

async function mountAt(meta: Record<string, unknown>, props: Record<string, unknown> = {}) {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [{ path: '/page', component: { template: '<div />' }, meta }],
  })
  await router.push('/page')
  await router.isReady()
  return mount(PageHeader, { props: { title: '数据中心', ...props }, global: { plugins: [router] } })
}

describe('PageHeader breadcrumb', () => {
  it('falls back to the route section so every page shows the same breadcrumb', async () => {
    const wrapper = await mountAt({ section: '实验与数据' })
    expect(wrapper.get('.breadcrumb').text()).toBe('实验与数据数据中心')
  })

  it('prefers an explicit section and omits the breadcrumb without one', async () => {
    expect((await mountAt({ section: '实验与数据' }, { section: '动物管理' })).get('.breadcrumb').text()).toContain('动物管理')
    expect((await mountAt({})).find('.breadcrumb').exists()).toBe(false)
  })
})
