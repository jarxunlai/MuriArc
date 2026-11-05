<script setup lang="ts">
import { computed } from 'vue'
import { Check, CirclePlus } from '@lucide/vue'
import { NPopover } from 'naive-ui'

export interface FacetOption {
  value: string
  label: string
  count?: number
}

// Multi-select facet in the style of shadcn-admin's faceted filter: filters
// narrow a list and are not tabs, so they sit beside the search field.
const props = defineProps<{
  label: string
  options: FacetOption[]
  modelValue: string[]
}>()
const emit = defineEmits<{ 'update:modelValue': [value: string[]] }>()

const selected = computed(() => new Set(props.modelValue))
const summary = computed(() => {
  if (!props.modelValue.length) return ''
  if (props.modelValue.length > 2) return `已选 ${props.modelValue.length} 项`
  return props.options.filter((option) => selected.value.has(option.value)).map((option) => option.label).join('、')
})

function toggle(value: string) {
  const next = new Set(selected.value)
  if (next.has(value)) next.delete(value)
  else next.add(value)
  emit('update:modelValue', props.options.map((option) => option.value).filter((option) => next.has(option)))
}
</script>

<template>
  <n-popover trigger="click" placement="bottom-start" :show-arrow="false" raw class="facet-popover">
    <template #trigger>
      <button type="button" class="facet-trigger" :class="{ active: modelValue.length }" aria-haspopup="listbox">
        <CirclePlus :size="15" aria-hidden="true" /><span>{{ label }}</span><b v-if="summary" class="facet-summary">{{ summary }}</b>
      </button>
    </template>
    <div class="facet-panel" role="listbox" :aria-label="label" aria-multiselectable="true">
      <button
        v-for="option in options" :key="option.value" type="button" role="option" class="facet-option"
        :aria-selected="selected.has(option.value)" @click="toggle(option.value)"
      >
        <span class="facet-check" aria-hidden="true"><Check v-if="selected.has(option.value)" :size="12" /></span>
        <span class="facet-label">{{ option.label }}</span>
        <span v-if="option.count != null" class="facet-count">{{ option.count }}</span>
      </button>
      <button v-if="modelValue.length" type="button" class="facet-clear" @click="emit('update:modelValue', [])">清除筛选</button>
    </div>
  </n-popover>
</template>

<style scoped>
.facet-trigger { display: inline-flex; align-items: center; gap: 6px; height: 34px; padding: 0 11px; border: 1px dashed var(--muri-border-strong); border-radius: var(--muri-radius-sm); color: var(--muri-text); background: var(--muri-surface); font-size: 13px; white-space: nowrap; cursor: pointer; transition: border-color var(--muri-transition-fast); }
.facet-trigger:hover { border-color: var(--muri-text-tertiary); }
.facet-trigger svg { color: var(--muri-text-tertiary); }
.facet-trigger.active { border-style: solid; }
.facet-summary { padding: 1px 6px; border-radius: var(--muri-radius-xs); color: var(--muri-text); background: var(--muri-surface-sunken); font-size: 12px; font-weight: 500; }
.facet-panel { display: flex; width: 220px; flex-direction: column; padding: 4px; border: 1px solid var(--muri-border); border-radius: var(--muri-radius); background: var(--muri-surface); box-shadow: var(--muri-shadow); }
.facet-option { display: flex; align-items: center; gap: 8px; padding: 7px 8px; border: 0; border-radius: var(--muri-radius-sm); color: var(--muri-text); background: transparent; font-size: 13px; text-align: left; cursor: pointer; }
.facet-option:hover { background: var(--muri-surface-hover); }
.facet-check { display: grid; width: 15px; height: 15px; place-items: center; border: 1px solid var(--muri-border-strong); border-radius: var(--muri-radius-xs); color: var(--muri-on-primary); }
.facet-option[aria-selected="true"] .facet-check { border-color: var(--muri-primary); background: var(--muri-primary); }
.facet-label { flex: 1; }
.facet-count { color: var(--muri-text-tertiary); font-size: 12px; font-variant-numeric: tabular-nums; }
.facet-clear { margin-top: 4px; padding: 7px 8px; border: 0; border-top: 1px solid var(--muri-border); border-radius: 0; color: var(--muri-text-secondary); background: transparent; font-size: 13px; cursor: pointer; }
.facet-clear:hover { color: var(--muri-text); }
</style>
