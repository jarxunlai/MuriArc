import { describe, expect, it } from 'vitest'
import type { RouteLocationNormalized } from 'vue-router'
import { routeScrollPosition, routeViewKey } from './navigation'

function at(name: string, params: Record<string, string> = {}, query: Record<string, string> = {}) {
  return { name, params, query, path: `/${name}` } as unknown as RouteLocationNormalized
}

describe('routeViewKey', () => {
  it('keeps one experiment workspace mounted while switching its sections', () => {
    const overview = routeViewKey(at('experiment-detail', { experimentId: 'exp-1', section: 'overview' }), 'p-1')
    const data = routeViewKey(at('experiment-detail', { experimentId: 'exp-1', section: 'data' }), 'p-1')
    expect(data).toBe(overview)
  })

  it('remounts for another experiment, another page or another project', () => {
    const base = routeViewKey(at('experiment-detail', { experimentId: 'exp-1' }), 'p-1')
    expect(routeViewKey(at('experiment-detail', { experimentId: 'exp-2' }), 'p-1')).not.toBe(base)
    expect(routeViewKey(at('experiments'), 'p-1')).not.toBe(base)
    expect(routeViewKey(at('experiment-detail', { experimentId: 'exp-1' }), undefined)).not.toBe(base)
  })

  it('ignores query-only changes such as an open drawer or tab', () => {
    expect(routeViewKey(at('animals', {}, { animal: 'a-1', tab: 'samples' }), undefined))
      .toBe(routeViewKey(at('animals'), undefined))
  })
})

describe('routeScrollPosition', () => {
  it('restores the saved position on history navigation', () => {
    expect(routeScrollPosition(at('animals'), at('cages'), { left: 0, top: 240 })).toEqual({ left: 0, top: 240 })
  })

  it('scrolls to the top when entering another page', () => {
    expect(routeScrollPosition(at('animals'), at('cages'), null)).toEqual({ top: 0 })
  })

  it('keeps the position for query changes and section switches within one view', () => {
    expect(routeScrollPosition(at('animals', {}, { animal: 'a-1' }), at('animals'), null)).toBe(false)
    expect(routeScrollPosition(
      at('experiment-detail', { experimentId: 'exp-1', section: 'data' }),
      at('experiment-detail', { experimentId: 'exp-1', section: 'overview' }),
      null,
    )).toBe(false)
  })
})
