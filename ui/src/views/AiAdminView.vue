<script setup lang="ts">
import { computed, onMounted, reactive, ref } from 'vue'
import { useMessage } from 'naive-ui'
import { Plug, Save, XCircle } from '@lucide/vue'
import PageHeader from '@/components/PageHeader.vue'
import {
  gateway,
  type AiDiagnostics,
  type AiLabSettings,
  type AiProviderEndpoint,
  type SaveAiProviderEndpointInput,
} from '@/services/gateway'

const msg = useMessage()
const loading = ref(false)
const policySaving = ref(false)
const endpointSaving = ref(false)
const diagnostics = ref<AiDiagnostics>()
const settings = reactive<AiLabSettings>({
  enabled: true,
  customUrlApprovalRequired: true,
  configuredUserCount: 0,
  enabledUserCount: 0,
  visionUserCount: 0,
  revision: 0,
  maxAutonomyMode: 'full',
})
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
const autonomyOptions = [
  { label: 'Ask', value: 'ask' },
  { label: 'Auto', value: 'auto' },
  { label: 'Full', value: 'full' },
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
    const [loadedDiagnostics, loadedSettings, loadedEndpoints] = await Promise.all([
      gateway.getAiDiagnostics?.(),
      gateway.getAiLabSettings?.(),
      gateway.listAiProviderEndpoints?.(),
    ])
    diagnostics.value = loadedDiagnostics
    if (loadedSettings) Object.assign(settings, loadedSettings)
    endpoints.value = loadedEndpoints ?? []
  } catch (error) {
    msg.error(`无法读取 AI 管理状态：${errorMessage(error)}`)
  } finally {
    loading.value = false
  }
}

async function savePolicy() {
  if (!gateway.saveAiLabSettings) return
  policySaving.value = true
  try {
    Object.assign(settings, await gateway.saveAiLabSettings({
      enabled: settings.enabled,
      customUrlApprovalRequired: settings.customUrlApprovalRequired,
      maxAutonomyMode: settings.maxAutonomyMode,
    }))
    msg.success('实验室 AI 策略已保存')
  } catch (error) {
    msg.error(`保存策略失败：${errorMessage(error)}`)
  } finally {
    policySaving.value = false
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
  endpointSaving.value = true
  try {
    await gateway.saveAiProviderEndpoint(input, editingId.value)
    msg.success(editingId.value ? 'Provider 出口已更新' : 'Provider 出口已添加')
    resetDraft()
    await load()
  } catch (error) {
    msg.error(`保存出口失败：${errorMessage(error)}`)
  } finally {
    endpointSaving.value = false
  }
}

async function disableEndpoint(endpoint: AiProviderEndpoint) {
  if (!gateway.disableAiProviderEndpoint || endpoint.builtin) return
  endpointSaving.value = true
  try {
    await gateway.disableAiProviderEndpoint(endpoint.id)
    msg.success('Provider 出口已停用')
    if (editingId.value === endpoint.id) resetDraft()
    await load()
  } catch (error) {
    msg.error(`停用失败：${errorMessage(error)}`)
  } finally {
    endpointSaving.value = false
  }
}

onMounted(load)
</script>

<template>
  <div class="page">
    <PageHeader
      title="AI 管理"
      description="管理实验室 AI 策略与 Provider 出口；个人模型配置和 API Key 始终归用户私有。"
    />

    <n-alert type="warning" :bordered="false">
      普通用户受实验室总开关和会话授权上限约束；LabAdmin 的个人 AI 配置不受总开关限制。自定义地址仍受 Server allowlist 与批准策略约束。
    </n-alert>

    <section class="cards">
      <article class="surface">
        <span>运行时</span>
        <strong>{{ diagnostics?.runtimeConfigured ? '已配置' : '未配置' }}</strong>
        <small>master key 仅显示状态</small>
      </article>
      <article class="surface">
        <span>用户配置</span>
        <strong>{{ settings.enabledUserCount }} / {{ settings.configuredUserCount }}</strong>
        <small>已启用 / 已配置</small>
      </article>
      <article class="surface">
        <span>视觉用户</span>
        <strong>{{ settings.visionUserCount }}</strong>
        <small>启用 supports_vision</small>
      </article>
      <article class="surface">
        <span>Provider 出口</span>
        <strong>{{ enabledEndpointCount }}</strong>
        <small>{{ diagnostics?.localEndpointCount ?? 0 }} 本地 + {{ diagnostics?.cloudEndpointCount ?? 0 }} 云端</small>
      </article>
    </section>

    <section class="policy surface">
      <div class="section-title">
        <h2>实验室 AI 策略</h2>
      </div>
      <div class="policy-row">
        <div><strong>实验室 AI 总开关</strong><span>关闭后普通用户不能解析 Provider；LabAdmin 保留管理与个人使用能力。</span></div>
        <n-switch v-model:value="settings.enabled" :disabled="loading || policySaving" />
      </div>
      <div class="policy-row">
        <div><strong>自定义 URL 必须管理员批准</strong><span>实验室出口和预置 Provider 仍按 Server allowlist 精确匹配。</span></div>
        <n-switch v-model:value="settings.customUrlApprovalRequired" :disabled="loading || policySaving" />
      </div>
      <div class="policy-row">
        <div><strong>会话授权上限</strong><span>用户只能在自身权限内选择不高于该级别的 Ask / Auto / Full。</span></div>
        <n-select v-model:value="settings.maxAutonomyMode" class="autonomy-select" :options="autonomyOptions" :disabled="loading || policySaving" />
      </div>
      <n-button type="primary" :loading="policySaving" :disabled="loading || !gateway.saveAiLabSettings" @click="savePolicy">
        <template #icon><Save :size="16" /></template>
        保存策略
      </n-button>
    </section>

    <section class="endpoint-editor surface">
      <div class="section-title">
        <Plug :size="18" />
        <h2>{{ editingId ? '编辑 Provider 出口' : '添加 Provider 出口' }}</h2>
      </div>
      <n-form label-placement="top" class="endpoint-form" :disabled="loading || endpointSaving">
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
        <n-button type="primary" :loading="endpointSaving" :disabled="!gateway.saveAiProviderEndpoint" @click="saveEndpoint">
          <template #icon><Save :size="16" /></template>
          保存出口
        </n-button>
        <n-button v-if="editingId" secondary :disabled="endpointSaving" @click="resetDraft">
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
          <n-button v-if="!endpoint.builtin" secondary size="small" :disabled="endpointSaving" @click="editEndpoint(endpoint)">编辑</n-button>
          <n-button v-if="!endpoint.builtin && endpoint.enabled" secondary type="warning" size="small" :disabled="endpointSaving" @click="disableEndpoint(endpoint)">停用</n-button>
        </div>
      </article>
    </section>

    <section class="boundary surface">
      <h3>明确边界</h3>
      <p>实验室出口是 LabAdmin 显式维护的公共配置；个人 API Key、个人模型和个人自定义地址不会在此展示，也不会被管理员读取或用于冒充用户调用。</p>
    </section>
  </div>
</template>

<style scoped>
.cards { display: grid; grid-template-columns: repeat(4, 1fr); gap: 10px; margin: 14px 0; }
.cards article { display: flex; padding: 16px; flex-direction: column; }
.cards strong { font-size: 25px; }
.cards span, .cards small, .policy span, .endpoint-row span, .boundary p { color: var(--muri-text-tertiary); }
.policy, .endpoint-editor, .boundary { padding: 18px; }
.policy { display: grid; gap: 16px; margin-bottom: 12px; }
.policy-row { display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: 18px; align-items: center; }
.policy-row > div { display: flex; flex-direction: column; }
.autonomy-select { width: 150px; }
.section-title { display: flex; align-items: center; gap: 8px; margin-bottom: 12px; }
.section-title h2 { margin: 0; font-size: 17px; }
.endpoint-editor { margin-bottom: 12px; }
.endpoint-form { display: grid; grid-template-columns: 220px 1fr 120px; gap: 0 14px; align-items: end; }
.endpoint-form .full-row { grid-column: 1 / -1; }
.button-row, .row-actions { display: flex; gap: 8px; flex-wrap: wrap; }
.endpoint-list { display: grid; gap: 10px; }
.endpoint-row { display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: 12px; align-items: center; padding: 14px 16px; }
.endpoint-title { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; }
.endpoint-row code { display: block; margin-top: 4px; overflow-wrap: anywhere; color: var(--muri-text-secondary); }
.boundary { margin-top: 12px; }
.boundary h3 { margin: 0 0 6px; }
.boundary p { margin: 0; }
@media (max-width: 800px) {
  .cards { grid-template-columns: 1fr 1fr; }
  .endpoint-form { grid-template-columns: 1fr; }
  .endpoint-row { grid-template-columns: 1fr; }
}
@media (max-width: 460px) {
  .cards { grid-template-columns: 1fr; }
  .policy-row { grid-template-columns: 1fr auto; }
  .autonomy-select { width: 120px; }
}
</style>
