import { computed, onBeforeUnmount, onMounted, ref, watch, type Ref } from 'vue'

export interface AiWorkbenchRect {
  x: number
  y: number
  width: number
  height: number
}

const STORAGE_KEY = 'muriarc.ai.workbench.layout.v1'
const VIEWPORT_MARGIN = 16
const MIN_WIDTH = 640
const MIN_HEIGHT = 440

function viewportSize() {
  return {
    width: Math.max(320, globalThis.innerWidth || 1280),
    height: Math.max(480, globalThis.innerHeight || 800),
  }
}

export function defaultAiWorkbenchRect(
  viewport = viewportSize(),
): AiWorkbenchRect {
  const width = Math.min(920, Math.max(320, viewport.width - VIEWPORT_MARGIN * 2))
  const height = Math.min(720, Math.max(420, viewport.height - VIEWPORT_MARGIN * 2))
  return {
    x: Math.max(VIEWPORT_MARGIN, viewport.width - width - 20),
    y: Math.max(VIEWPORT_MARGIN, viewport.height - height - 20),
    width,
    height,
  }
}

export function clampAiWorkbenchRect(
  value: AiWorkbenchRect,
  viewport = viewportSize(),
): AiWorkbenchRect {
  const availableWidth = Math.max(320, viewport.width - VIEWPORT_MARGIN * 2)
  const availableHeight = Math.max(420, viewport.height - VIEWPORT_MARGIN * 2)
  const width = Math.min(availableWidth, Math.max(Math.min(MIN_WIDTH, availableWidth), value.width))
  const height = Math.min(availableHeight, Math.max(Math.min(MIN_HEIGHT, availableHeight), value.height))
  return {
    x: Math.min(
      Math.max(VIEWPORT_MARGIN, value.x),
      Math.max(VIEWPORT_MARGIN, viewport.width - width - VIEWPORT_MARGIN),
    ),
    y: Math.min(
      Math.max(VIEWPORT_MARGIN, value.y),
      Math.max(VIEWPORT_MARGIN, viewport.height - height - VIEWPORT_MARGIN),
    ),
    width,
    height,
  }
}

export function adjustAiWorkbenchRectByKeyboard(
  value: AiWorkbenchRect,
  key: string,
  delta: number,
  resize: boolean,
  viewport = viewportSize(),
): AiWorkbenchRect {
  const offsets: Record<string, [number, number]> = {
    ArrowLeft: [-delta, 0],
    ArrowRight: [delta, 0],
    ArrowUp: [0, -delta],
    ArrowDown: [0, delta],
  }
  const offset = offsets[key]
  if (!offset) return value
  return clampAiWorkbenchRect(resize
    ? {
        ...value,
        width: value.width + offset[0],
        height: value.height + offset[1],
      }
    : {
        ...value,
        x: value.x + offset[0],
        y: value.y + offset[1],
      }, viewport)
}

function storedRect(storageKey: string): AiWorkbenchRect | undefined {
  try {
    const value = JSON.parse(globalThis.localStorage?.getItem(storageKey) ?? 'null') as Partial<AiWorkbenchRect> | null
    if (!value
      || !Number.isFinite(value.x)
      || !Number.isFinite(value.y)
      || !Number.isFinite(value.width)
      || !Number.isFinite(value.height)) return undefined
    return value as AiWorkbenchRect
  } catch {
    return undefined
  }
}

export function useAiWorkbenchLayout(
  panel: Ref<HTMLElement | null>,
  storageScope = 'local',
) {
  const storageKey = `${STORAGE_KEY}:${storageScope}`
  const rect = ref(defaultAiWorkbenchRect())
  const maximized = ref(false)
  let dragStart: { pointerX: number; pointerY: number; x: number; y: number } | undefined
  let resizeObserver: ResizeObserver | undefined

  const style = computed(() => maximized.value
    ? {
        left: `${VIEWPORT_MARGIN}px`,
        top: `${VIEWPORT_MARGIN}px`,
        width: `calc(100vw - ${VIEWPORT_MARGIN * 2}px)`,
        height: `calc(100vh - ${VIEWPORT_MARGIN * 2}px)`,
      }
    : {
        left: `${rect.value.x}px`,
        top: `${rect.value.y}px`,
        width: `${rect.value.width}px`,
        height: `${rect.value.height}px`,
      })

  function persist() {
    try {
      globalThis.localStorage?.setItem(storageKey, JSON.stringify(rect.value))
    } catch {
      // A denied storage area must not prevent the workbench from opening.
    }
  }

  function move(event: PointerEvent) {
    if (!dragStart || maximized.value) return
    rect.value = clampAiWorkbenchRect({
      ...rect.value,
      x: dragStart.x + event.clientX - dragStart.pointerX,
      y: dragStart.y + event.clientY - dragStart.pointerY,
    })
  }

  function finishDrag() {
    if (!dragStart) return
    dragStart = undefined
    document.body.classList.remove('ai-workbench-dragging')
    globalThis.removeEventListener('pointermove', move)
    globalThis.removeEventListener('pointerup', finishDrag)
    persist()
  }

  function startDrag(event: PointerEvent) {
    if (maximized.value || event.button !== 0) return
    dragStart = {
      pointerX: event.clientX,
      pointerY: event.clientY,
      x: rect.value.x,
      y: rect.value.y,
    }
    document.body.classList.add('ai-workbench-dragging')
    globalThis.addEventListener('pointermove', move)
    globalThis.addEventListener('pointerup', finishDrag, { once: true })
  }

  function moveByKeyboard(event: KeyboardEvent) {
    if (!event.altKey || maximized.value) return
    const delta = event.shiftKey ? 40 : 10
    if (!event.key.startsWith('Arrow')) return
    event.preventDefault()
    rect.value = adjustAiWorkbenchRectByKeyboard(
      rect.value,
      event.key,
      delta,
      event.ctrlKey,
    )
    persist()
  }

  function toggleMaximize() {
    maximized.value = !maximized.value
    if (!maximized.value) rect.value = clampAiWorkbenchRect(rect.value)
  }

  function reset() {
    maximized.value = false
    rect.value = defaultAiWorkbenchRect()
    persist()
  }

  function handleViewportResize() {
    rect.value = clampAiWorkbenchRect(rect.value)
  }

  function observePanel(element: HTMLElement | null) {
    resizeObserver?.disconnect()
    resizeObserver = undefined
    if (typeof ResizeObserver === 'undefined' || !element) return
    resizeObserver = new ResizeObserver((entries) => {
      if (maximized.value || dragStart || !panel.value) return
      const entry = entries[0]
      if (!entry) return
      const bounds = element.getBoundingClientRect()
      if (bounds.width <= 0 || bounds.height <= 0) return
      const next = clampAiWorkbenchRect({
        ...rect.value,
        // The inline dimensions and global box sizing both use border-box.
        // ResizeObserver.contentRect excludes borders and would therefore
        // shrink the panel on every observer feedback cycle.
        width: bounds.width,
        height: bounds.height,
      })
      if (next.width !== rect.value.width || next.height !== rect.value.height) {
        rect.value = next
        persist()
      }
    })
    resizeObserver.observe(element)
  }

  watch(panel, observePanel)

  onMounted(() => {
    rect.value = clampAiWorkbenchRect(storedRect(storageKey) ?? defaultAiWorkbenchRect())
    globalThis.addEventListener('resize', handleViewportResize)
    observePanel(panel.value)
  })

  onBeforeUnmount(() => {
    finishDrag()
    resizeObserver?.disconnect()
    globalThis.removeEventListener('resize', handleViewportResize)
  })

  return {
    rect,
    maximized,
    style,
    startDrag,
    moveByKeyboard,
    toggleMaximize,
    reset,
  }
}
