<script setup lang="ts">
import { defineAsyncComponent, ref } from 'vue'
import {
  ExternalLink,
  GripHorizontal,
  Maximize2,
  Minimize2,
  RotateCcw,
  Sparkles,
  X,
} from '@lucide/vue'
import { useRouter } from 'vue-router'
import { useAiAssistant } from '@/composables/useAiAssistant'
import { useAiWorkbenchLayout } from '@/composables/useAiWorkbenchLayout'
import { currentAuthSession, gateway } from '@/services/gateway'
const AiWorkspaceShell = defineAsyncComponent(() => import('./AiWorkspaceShell.vue'))

const ai = useAiAssistant()
const router = useRouter()
const panel = ref<HTMLElement | null>(null)
const layout = useAiWorkbenchLayout(
  panel,
  currentAuthSession.value?.user.id ?? gateway.mode,
)

async function expandWorkspace() {
  ai.drawerOpen.value = false
  await router.push('/ai')
}

function startDrag(event: PointerEvent) {
  const target = event.target as HTMLElement
  if (target.closest('button, a, input, select, textarea, [role="button"]')) return
  layout.startDrag(event)
}
</script>

<template>
  <Teleport to="body">
    <section
      v-if="ai.drawerOpen.value"
      ref="panel"
      class="ai-workbench"
      :class="{ maximized: layout.maximized.value }"
      :style="layout.style.value"
      role="dialog"
      aria-label="MuriArc AI 工作台"
      aria-modal="false"
    >
      <header
        class="workbench-header"
        tabindex="0"
        aria-label="AI 工作台拖动区域；按 Alt 加方向键移动，按 Ctrl 加 Alt 加方向键缩放"
        @pointerdown="startDrag"
        @keydown="layout.moveByKeyboard"
      >
        <div class="workbench-title">
          <Sparkles :size="17" />
          <strong>AI 工作台</strong>
          <span>{{ ai.contextTitle.value }}</span>
        </div>
        <GripHorizontal class="drag-indicator" :size="18" aria-hidden="true" />
        <nav class="workbench-actions" aria-label="工作台窗口操作">
          <button type="button" title="在完整页面打开" aria-label="在完整页面打开" @click="expandWorkspace">
            <ExternalLink :size="16" />
          </button>
          <button type="button" title="复位大小和位置" aria-label="复位大小和位置" @click="layout.reset">
            <RotateCcw :size="16" />
          </button>
          <button
            type="button"
            :title="layout.maximized.value ? '还原窗口' : '最大化'"
            :aria-label="layout.maximized.value ? '还原窗口' : '最大化'"
            @click="layout.toggleMaximize"
          >
            <Minimize2 v-if="layout.maximized.value" :size="16" />
            <Maximize2 v-else :size="16" />
          </button>
          <button type="button" title="关闭 AI 工作台" aria-label="关闭 AI 工作台" @click="ai.drawerOpen.value = false">
            <X :size="17" />
          </button>
        </nav>
      </header>
      <div class="workbench-body">
        <AiWorkspaceShell compact />
      </div>
    </section>
  </Teleport>
</template>

<style scoped>
.ai-workbench {
  position: fixed;
  z-index: 70;
  display: grid;
  min-width: 640px;
  min-height: 440px;
  grid-template-rows: 45px minmax(0, 1fr);
  border: 1px solid var(--muri-border-strong);
  border-radius: 11px;
  background: var(--muri-surface);
  box-shadow: 0 18px 50px rgba(30, 53, 76, .2);
  overflow: hidden;
  resize: both;
}
.ai-workbench.maximized { resize: none; }
.workbench-header {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto minmax(0, 1fr);
  align-items: center;
  padding: 0 7px 0 13px;
  border-bottom: 1px solid var(--muri-border);
  background: #fbfcfd;
  cursor: grab;
  user-select: none;
  touch-action: none;
}
.workbench-header:active { cursor: grabbing; }
.workbench-header:focus-visible {
  outline: 2px solid var(--muri-primary);
  outline-offset: -2px;
}
.workbench-title { display: flex; min-width: 0; align-items: center; gap: 7px; }.workbench-title svg { flex: 0 0 auto; color: var(--muri-primary); }.workbench-title strong { font-size: 12px; }.workbench-title span { overflow: hidden; color: var(--muri-text-tertiary); font-size: 10px; text-overflow: ellipsis; white-space: nowrap; }
.drag-indicator { color: var(--muri-border-strong); }
.workbench-actions { display: flex; justify-content: flex-end; gap: 1px; padding: 0; }
.workbench-actions button {
  display: grid;
  width: 31px;
  height: 31px;
  padding: 0;
  place-items: center;
  border: 0;
  border-radius: 6px;
  color: var(--muri-text-secondary);
  background: transparent;
  cursor: pointer;
  transition: color var(--muri-transition-fast), background var(--muri-transition-fast);
}
.workbench-actions button:hover { color: var(--muri-primary); background: var(--muri-primary-soft); }
.workbench-actions button:last-child:hover { color: var(--muri-danger); background: #fff1f2; }
.workbench-actions button:focus-visible { outline: 2px solid var(--muri-primary); outline-offset: 1px; }
.workbench-body { min-width: 0; min-height: 0; }
:global(body.ai-workbench-dragging) { cursor: grabbing !important; user-select: none !important; }
@media (max-width: 760px) {
  .ai-workbench,
  .ai-workbench.maximized {
    inset: 0 !important;
    width: 100vw !important;
    height: 100dvh !important;
    min-width: 0;
    min-height: 0;
    border: 0;
    border-radius: 0;
    resize: none;
  }
  .workbench-header { grid-template-columns: minmax(0, 1fr) auto; padding-left: 11px; cursor: default; touch-action: auto; }
  .drag-indicator,
  .workbench-actions button:nth-child(2),
  .workbench-actions button:nth-child(3) { display: none; }
}
@media (prefers-reduced-motion: reduce) {
  .workbench-actions button { transition: none; }
}
</style>
