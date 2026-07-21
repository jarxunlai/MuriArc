<script setup lang="ts">
import { computed, onMounted, reactive, ref } from 'vue'
import { useMessage } from 'naive-ui'
import { Plug, Save, XCircle } from '@lucide/vue'
import PageHeader from '@/components/PageHeader.vue'
import { gateway, type AiDiagnostics, type AiLabSettings, type AiProviderEndpoint, type SaveAiProviderEndpointInput } from '@/services/gateway'

const msg = useMessage()
const loading = ref(false)
const saving = ref(false)
const diagnostics = ref<AiDiagnostics>()
const summary = ref<AiLabSettings>()
const endpoints = ref<AiProviderEndpoint[]>([])
const editingId = ref<string>()
const draft = reactive<SaveAiProviderEndpointInput>({
  enabled: true,
  providerKind: 'open_ai_compatible',
  label: '',
  baseUrl: '',
})
const providerOptions = [
  { label: 'OpenAI-compatible', value: 'open_ai_compatible' },
  { label: '本地 HTTP 模型', value: 'local_http' },
]
const enabledEndpointCount = computed(() => endpoints.value.filter((item) => item.enabled).length)

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : '操作失败'
}

function resetDraft() {
  editingId.value = undefined
  Object.assign(draft, {
    enabled: true,
    providerKind: 'open_ai_compatible',
    label: '',
    baseUrl: '',
  })
}

async function load() {
  loading.value = true
  try {
    const [loadedDiagnostics, loadedSummary, loadedEndpoints] = await Promise.all([
      gateway.getAiDiagnostics?.(),
      gateway.getAiLabSettings?.(),
      gateway.listAiProviderEndpoints?.(),
    ])
    diagnostics.value = loadedDiagnostics
    summary.value = loadedSummary
    endpoints.value = loadedEndpoints ?? []
  } catch (error) {
    msg.error(`无法读取 AI 管理状态：${errorMessage(error)}`)
  } finally {
    loading.value = false
  }
}

function editEndpoint(endpoint: AiProviderEndpoint) {
  if (endpoint.builtin) return
  editingId.value = endpoint.id
  Object.assign(draft, {
    enabled: endpoint.enabled,
    providerKind: endpoint.providerKind,
    label: endpoint.label,
    baseUrl: endpoint.baseUrl,
  })
}

async function saveEndpoint() {
  if (!gateway.saveAiProviderEndpoint) return
  const input = {
    enabled: draft.enabled,
    providerKind: draft.providerKind,
    label: draft.label.trim(),
    baseUrl: draft.baseUrl.trim(),
  }
  if (!input.label || !input.baseUrl) {
    msg.warning('名称和 API URL 不能为空')
    return
  }
  saving.value = true
  try {
    await gateway.saveAiProviderEndpoint(input, editingId.value)
    msg.success(editingId.value ? 'Provider 出口已更新' : 'Provider 出口已添加')
    resetDraft()
    await load()
  } catch (error) {
    msg.error(`保存失败：${errorMessage(error)}`)
  } finally {
    saving.value = false
  }
}

async function disableEndpoint(endpoint: AiProviderEndpoint) {
  if (!gateway.disableAiProviderEndpoint || endpoint.builtin) return
  saving.value = true
  try {
    await gateway.disableAiProviderEndpoint(endpoint.id)
    msg.success('Provider 出口已停用')
    if (editingId.value === endpoint.id) resetDraft()
    await load()
  } catch (error) {
    msg.error(`停用失败：${errorMessage(error)}`)
  } finally {
    saving.value = false
  }
}

onMounted(load)
</script>

<template>
  <div class="page">
    <PageHeader title="AI 管理" description="管理实验室可用的 Provider 出口；用户自行保存个人模型和 API Key。" />

    <section class="cards">
      <article class="surface">
        <span>运行时</span>
        <strong>{{ diagnostics?.runtimeConfigured ? '已配置' : '未配置' }}</strong>
        <small>master key 仅显示状态</small>
      </article>
      <article class="surface">
        <span>用户配置</span>
        <strong>{{ summary?.enabledUserCount ?? 0 }} / {{ summary?.configuredUserCount ?? 0 }}</strong>
        <small>已启用 / 已配置</small>
      </article>
      <article class="surface">
        <span>视觉用户</span>
        <strong>{{ summary?.visionUserCount ?? 0 }}</strong>
        <small>启用 supports_vision</small>
      </article>
      <article class="surface">
        <span>Provider 出口</span>
        <strong>{{ enabledEndpointCount }}</strong>
        <small>{{ diagnostics?.localEndpointCount ?? 0 }} 本地 + {{ diagnostics?.cloudEndpointCount ?? 0 }} 云端</small>
      </article>
    </section>

    <section class="endpoint-editor surface">
      <div class="section-title">
        <Plug :size="18" />
        <h2>{{ editingId ? '编辑 Provider 出口' : '添加 Provider 出口' }}</h2>
      </div>
      <n-form label-placement="top" class="endpoint-form" :disabled="loading || saving">
        <n-form-item label="Provider">
          <n-select v-model:value="draft.providerKind" :options="providerOptions" />
        </n-form-item>
        <n-form-item label="名称">
          <n-input v-model:value="draft.label" maxlength="120" placeholder="例如 实验室本地模型" />
        </n-form-item>
        <n-form-item label="API URL" class="full-row">
          <n-input v-model:value="draft.baseUrl" maxlength="2048" placeholder="https://api.example.com/v1" />
        </n-form-item>
        <n-form-item label="启用">
          <n-switch v-model:value="draft.enabled" />
        </n-form-item>
      </n-form>
      <div class="button-row">
        <n-button type="primary" :loading="saving" :disabled="!gateway.saveAiProviderEndpoint" @click="saveEndpoint">
          <template #icon><Save :size="16" /></template>
          保存出口
        </n-button>
        <n-button v-if="editingId" secondary :disabled="saving" @click="resetDraft">
          <template #icon><XCircle :size="16" /></template>
          取消编辑
        </n-button>
      </div>
    </section>

    <section class="endpoint-list">
      <article v-for="endpoint in endpoints" :key="endpoint.id" class="surface endpoint-row">
        <div>
          <div class="endpoint-title">
            <strong>{{ endpoint.label }}</strong>
            <n-tag size="small" :type="endpoint.enabled ? 'success' : 'default'">{{ endpoint.enabled ? '启用' : '停用' }}</n-tag>
            <n-tag v-if="endpoint.builtin" size="small">内置</n-tag>
          </div>
          <span>{{ endpoint.providerKind === 'local_http' ? '本地 HTTP 模型' : 'OpenAI-compatible' }}</span>
          <code>{{ endpoint.baseUrl }}</code>
        </div>
        <div class="row-actions">
          <n-button v-if="!endpoint.builtin" secondary size="small" :disabled="saving" @click="editEndpoint(endpoint)">编辑</n-button>
          <n-button v-if="!endpoint.builtin && endpoint.enabled" secondary type="warning" size="small" :disabled="saving" @click="disableEndpoint(endpoint)">停用</n-button>
        </div>
      </article>
    </section>
  </div>
</template>

<style scoped>
.cards { display: grid; grid-template-columns: repeat(4, 1fr); gap: 10px; margin: 14px 0; }
.cards article { display: flex; padding: 16px; flex-direction: column; }
.cards strong { font-size: 25px; }
.cards span, .cards small, .endpoint-row span { color: var(--muri-text-tertiary); }
.endpoint-editor { padding: 18px; margin-bottom: 12px; }
.section-title { display: flex; align-items: center; gap: 8px; margin-bottom: 12px; }
.section-title h2 { margin: 0; font-size: 17px; }
.endpoint-form { display: grid; grid-template-columns: 220px 1fr 120px; gap: 0 14px; align-items: end; }
.endpoint-form .full-row { grid-column: 1 / -1; }
.button-row, .row-actions { display: flex; gap: 8px; flex-wrap: wrap; }
.endpoint-list { display: grid; gap: 10px; }
.endpoint-row { padding: 14px 16px; display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: 12px; align-items: center; }
.endpoint-title { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; }
.endpoint-row code { display: block; margin-top: 4px; overflow-wrap: anywhere; color: var(--muri-text-secondary); }
@media (max-width: 800px) { .cards { grid-template-columns: 1fr 1fr; }.endpoint-form { grid-template-columns: 1fr; }.endpoint-row { grid-template-columns: 1fr; } }
@media (max-width: 460px) { .cards { grid-template-columns: 1fr; } }
</style>
