<script setup lang="ts">
import { computed, onMounted, reactive, ref } from 'vue'
import { useMessage } from 'naive-ui'
import { ExternalLink, Plug, Save, XCircle } from '@lucide/vue'
import PageHeader from '@/components/PageHeader.vue'
import type { AiProviderPreset } from '@/domain/models'
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
const presets = ref<AiProviderPreset[]>([])
const editingId = ref<string>()
const draft = reactive<SaveAiProviderEndpointInput>({
  enabled: true,
  providerKind: 'open_ai_compatible',
  protocol: 'openai_chat_completions',
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
    protocol: 'openai_chat_completions',
    label: '',
    baseUrl: '',
  })
}

async function load() {
  loading.value = true
  try {
    const [loadedDiagnostics, loadedSettings, loadedEndpoints, loadedPresets] = await Promise.all([
      gateway.getAiDiagnostics?.(),
      gateway.getAiLabSettings?.(),
      gateway.listAiProviderEndpoints?.(),
      gateway.listAiProviderPresets?.(),
    ])
    diagnostics.value = loadedDiagnostics
    if (loadedSettings) Object.assign(settings, loadedSettings)
    endpoints.value = loadedEndpoints ?? []
    presets.value = loadedPresets ?? []
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
    protocol: endpoint.protocol,
    label: endpoint.label,
    baseUrl: endpoint.baseUrl,
  })
}

async function saveEndpoint() {
  if (!gateway.saveAiProviderEndpoint) return
  const input = {
    enabled: draft.enabled,
    providerKind: draft.providerKind,
    protocol: draft.protocol,
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

    <n-alert type="info" :bordered="false">
      所有用户（包括 Environment Root 与 LabAdmin）都受实验室总开关约束。管理员只能维护非敏感策略、预设目录与批准出口，不能读取任何用户的 API Key、个人模型或 Token 参数。
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
        <span>Provider 预设</span>
        <strong>{{ presets.filter((item) => item.enabled).length }}</strong>
        <small>内置推荐与实验室自定义目录</small>
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
        <div><strong>实验室 AI 总开关</strong><span>关闭后所有用户都不能发起 Provider 请求；非敏感管理页面仍可用于重新启用。</span></div>
        <n-switch v-model:value="settings.enabled" :disabled="loading || policySaving" />
      </div>
      <div class="policy-row">
        <div><strong>自定义 URL 必须管理员批准</strong><span>开启时仅允许官方推荐出口或管理员登记的精确 URL；关闭时用户可保存通过协议安全校验的个人出口。</span></div>
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

    <section class="preset-catalog surface">
      <div class="section-title">
        <Plug :size="18" />
        <h2>Provider 预设目录</h2>
      </div>
      <p class="catalog-intro">预设只提供显示名称、推荐出口、模型目录和官方文档，不包含或共享任何用户凭据与个人选择。</p>
      <div class="preset-grid">
        <article v-for="preset in presets" :key="preset.id" class="preset-card">
          <div><strong>{{ preset.displayName }}</strong><n-tag size="small" :type="preset.enabled ? 'success' : 'default'">{{ preset.enabled ? '启用' : '停用' }}</n-tag><n-tag v-if="preset.builtin" size="small">内置</n-tag></div>
          <code>{{ preset.recommendedBaseUrl || '由用户填写出口' }}</code>
          <span>{{ preset.models.length ? preset.models.map((model) => model.displayName).join(' · ') : '兼容模型由用户填写' }}</span>
          <a v-if="preset.documentationUrl" :href="preset.documentationUrl" target="_blank" rel="noreferrer">官方文档<ExternalLink :size="13" /></a>
        </article>
      </div>
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
.cards { display: grid; grid-template-columns: repeat(5, 1fr); gap: 10px; margin: 14px 0; }
.cards article { display: flex; padding: 16px; flex-direction: column; }
.cards strong { font-size: 25px; }
.cards span, .cards small, .policy span, .endpoint-row span, .boundary p { color: var(--muri-text-tertiary); }
.policy, .preset-catalog, .endpoint-editor, .boundary { padding: 18px; }
.policy { display: grid; gap: 16px; margin-bottom: 12px; }
.policy-row { display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: 18px; align-items: center; }
.policy-row > div { display: flex; flex-direction: column; }
.autonomy-select { width: 150px; }
.section-title { display: flex; align-items: center; gap: 8px; margin-bottom: 12px; }
.section-title h2 { margin: 0; font-size: 17px; }
.preset-catalog, .endpoint-editor { margin-bottom: 12px; }.catalog-intro { margin: -4px 0 14px; color: var(--muri-text-tertiary); }.preset-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 9px; }.preset-card { display: flex; min-width: 0; flex-direction: column; gap: 6px; padding: 12px; border: 1px solid var(--muri-border); border-radius: 7px; }.preset-card > div { display: flex; align-items: center; gap: 7px; flex-wrap: wrap; }.preset-card code, .preset-card span { overflow-wrap: anywhere; color: var(--muri-text-secondary); font-size: 11px; }.preset-card a { display: inline-flex; width: fit-content; align-items: center; gap: 4px; color: var(--muri-primary); text-decoration: none; }
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
  .preset-grid { grid-template-columns: 1fr; }
  .endpoint-form { grid-template-columns: 1fr; }
  .endpoint-row { grid-template-columns: 1fr; }
}
@media (max-width: 460px) {
  .cards { grid-template-columns: 1fr; }
  .policy-row { grid-template-columns: 1fr auto; }
  .autonomy-select { width: 120px; }
}
</style>
