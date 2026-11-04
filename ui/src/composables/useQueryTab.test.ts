import { flushPromises, mount } from '@vue/test-utils'
import { defineComponent, h } from 'vue'
import { createMemoryHistory, createRouter } from 'vue-router'
import { describe, expect, it } from 'vitest'
import { useQueryTab } from './useQueryTab'

const tabs = ['timeline', 'samples', 'audit'] as const

async function setup(location: string) {
  let tab!: ReturnType<typeof useQueryTab<(typeof tabs)[number]>>
  const Host = defineComponent({
    setup() {
      tab = useQueryTab(tabs, 'timeline')
      return () => h('div')
    },
  })
  const router = createRouter({ history: createMemoryHistory(), routes: [{ path: '/animals', component: Host }] })
  await router.push(location)
  await router.isReady()
  mount({ render: () => h(Host) }, { global: { plugins: [router] } })
  return { router, tab: () => tab }
}

describe('useQueryTab', () => {
  it('reads an allowed tab from the query and falls back otherwise', async () => {
    expect((await setup('/animals?tab=samples')).tab().value).toBe('samples')
    expect((await setup('/animals?tab=unknown')).tab().value).toBe('timeline')
    expect((await setup('/animals')).tab().value).toBe('timeline')
  })

  it('writes the tab with replace, keeps other query keys and omits the default', async () => {
    const { router, tab } = await setup('/animals?animal=a-1')
    tab().value = 'audit'
    await flushPromises()
    expect(router.currentRoute.value.query).toEqual({ animal: 'a-1', tab: 'audit' })

    tab().value = 'timeline'
    await flushPromises()
    expect(router.currentRoute.value.query).toEqual({ animal: 'a-1' })
  })
})
