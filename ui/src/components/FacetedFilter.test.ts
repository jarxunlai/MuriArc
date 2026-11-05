import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'
import FacetedFilter from './FacetedFilter.vue'

const options = [
  { value: 'active', label: '在养', count: 12 },
  { value: 'breeding', label: '繁育中', count: 3 },
  { value: 'archived', label: '已归档', count: 0 },
]

function mountFilter(modelValue: string[] = []) {
  return mount(FacetedFilter, {
    props: { label: '状态', options, modelValue, 'onUpdate:modelValue': () => undefined },
    global: { stubs: { Popover: { template: '<div><slot name="trigger" /><slot /></div>' } } },
  })
}

describe('FacetedFilter', () => {
  it('shows the facet name and per-option counts', () => {
    const wrapper = mountFilter()
    expect(wrapper.get('.facet-trigger').text()).toBe('状态')
    expect(wrapper.findAll('.facet-option').map((option) => option.text())).toEqual(['在养12', '繁育中3', '已归档0'])
  })

  it('toggles values and emits the new selection in option order', async () => {
    const wrapper = mountFilter(['breeding'])
    await wrapper.findAll('.facet-option')[0]!.trigger('click')
    expect(wrapper.emitted('update:modelValue')!.at(-1)).toEqual([['active', 'breeding']])

    await wrapper.findAll('.facet-option')[1]!.trigger('click')
    expect(wrapper.emitted('update:modelValue')!.at(-1)).toEqual([[]])
  })

  it('summarises the selection on the trigger and can clear it', async () => {
    expect(mountFilter(['active']).get('.facet-trigger').text()).toBe('状态在养')
    const many = mountFilter(['active', 'breeding', 'archived'])
    expect(many.get('.facet-trigger').text()).toBe('状态已选 3 项')

    await many.get('.facet-clear').trigger('click')
    expect(many.emitted('update:modelValue')!.at(-1)).toEqual([[]])
  })
})
