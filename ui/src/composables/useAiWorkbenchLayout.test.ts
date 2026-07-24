import { mount } from '@vue/test-utils'
import { defineComponent, nextTick, ref } from 'vue'
import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  adjustAiWorkbenchRectByKeyboard,
  clampAiWorkbenchRect,
  defaultAiWorkbenchRect,
  useAiWorkbenchLayout,
} from './useAiWorkbenchLayout'

describe('AI workbench layout bounds', () => {
  afterEach(() => {
    globalThis.localStorage.clear()
    vi.unstubAllGlobals()
    vi.restoreAllMocks()
  })

  it('opens near the lower-right edge while remaining inside the viewport', () => {
    expect(defaultAiWorkbenchRect({ width: 1440, height: 900 })).toEqual({
      x: 500,
      y: 160,
      width: 920,
      height: 720,
    })
  })

  it('clamps dragged and resized panels back into a desktop viewport', () => {
    expect(clampAiWorkbenchRect(
      { x: -800, y: 2000, width: 1800, height: 120 },
      { width: 1024, height: 768 },
    )).toEqual({
      x: 16,
      y: 312,
      width: 992,
      height: 440,
    })
  })

  it('allows the CSS mobile fullscreen boundary to fit narrow viewports', () => {
    const result = clampAiWorkbenchRect(
      { x: 20, y: 20, width: 920, height: 720 },
      { width: 375, height: 667 },
    )
    expect(result.width).toBe(343)
    expect(result.height).toBe(635)
    expect(result.x).toBe(16)
    expect(result.y).toBe(16)
  })

  it('supports keyboard movement and resizing inside the viewport', () => {
    const initial = { x: 100, y: 100, width: 700, height: 500 }
    expect(adjustAiWorkbenchRectByKeyboard(
      initial,
      'ArrowRight',
      10,
      false,
      { width: 1200, height: 800 },
    )).toMatchObject({ x: 110, y: 100, width: 700, height: 500 })
    expect(adjustAiWorkbenchRectByKeyboard(
      initial,
      'ArrowDown',
      40,
      true,
      { width: 1200, height: 800 },
    )).toMatchObject({ x: 100, y: 100, width: 700, height: 540 })
  })

  it('persists the panel border-box without shrinking on ResizeObserver feedback', async () => {
    let resizeCallback: ResizeObserverCallback | undefined
    class ResizeObserverStub {
      constructor(callback: ResizeObserverCallback) {
        resizeCallback = callback
      }

      observe() {}
      unobserve() {}
      disconnect() {}
    }
    vi.stubGlobal('ResizeObserver', ResizeObserverStub)
    vi.stubGlobal('innerWidth', 1200)
    vi.stubGlobal('innerHeight', 800)
    globalThis.localStorage.setItem(
      'muriarc.ai.workbench.layout.v1:observer-test',
      JSON.stringify({ x: 100, y: 100, width: 700, height: 500 }),
    )

    const wrapper = mount(defineComponent({
      template: '<section ref="panel" :style="layoutStyle" />',
      setup() {
        const panel = ref<HTMLElement | null>(null)
        const layout = useAiWorkbenchLayout(panel, 'observer-test')
        return { panel, layoutStyle: layout.style }
      },
    }))
    await nextTick()

    const element = wrapper.get('section').element as HTMLElement
    vi.spyOn(element, 'getBoundingClientRect').mockReturnValue({
      x: 100,
      y: 100,
      top: 100,
      right: 800,
      bottom: 600,
      left: 100,
      width: 700,
      height: 500,
      toJSON: () => ({}),
    })
    resizeCallback?.([
      {
        target: element,
        contentRect: {
          x: 0,
          y: 0,
          top: 0,
          right: 698,
          bottom: 498,
          left: 0,
          width: 698,
          height: 498,
          toJSON: () => ({}),
        },
      } as unknown as ResizeObserverEntry,
    ], {} as ResizeObserver)
    await nextTick()

    expect(element.style.width).toBe('700px')
    expect(element.style.height).toBe('500px')
    expect(globalThis.localStorage.getItem(
      'muriarc.ai.workbench.layout.v1:observer-test',
    )).toBe(JSON.stringify({ x: 100, y: 100, width: 700, height: 500 }))
    wrapper.unmount()
  })
})
