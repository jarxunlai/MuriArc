<script setup lang="ts">
import { computed, nextTick, onMounted, reactive, ref, watch } from 'vue'
import { useDialog, useMessage } from 'naive-ui'
import {
  Archive,
  ArrowLeft,
  Bot,
  Check,
  Eye,
  KeyRound,
  Plus,
  Save,
  ShieldCheck,
  SlidersHorizontal,
} from '@lucide/vue'
import {
  gateway,
  type AiModelDefaultsView,
  type AiModelProfileView,
  type AiModelValidationResult,
  type AiProviderProtocol,
  type AiProviderTransport,
  type SaveAiModelProfileInput,
  type ValidateAiModelProfileInput,
} from '@/services/gateway'

type EditorMode = 'list' | 'detail'

interface ModelDraft {
  name: string
  protocol: AiProviderProtocol
  transport: AiProviderTransport
  baseUrl: string
  modelId: string
  supportsVision: boolean
  contextWindowTokens: number
  maxInputTokens: number
  maxOutputTokens: number
  historyTokenBudget: number
  historyTurns: number
  temperature: number
  timeoutMs: number
}

const defaultDraft = (): ModelDraft => ({
  name: '',
  protocol: 'openai_chat_completions',
  transport: 'open_ai_compatible',
  baseUrl: '',
  modelId: '',
  supportsVision: false,
  contextWindowTokens: 131072,
  maxInputTokens: 65536,
  maxOutputTokens: 4096,
  historyTokenBudget: 32768,
  historyTurns: 20,
  temperature: 0,
  timeoutMs: 120000,
})

const message = useMessage()
const dialog = useDialog()
const mode = ref<EditorMode>('list')
const profiles = ref<AiModelProfileView[]>([])
const defaults = ref<AiModelDefaultsView>({
  revision: 0,
})
const activeProfile = ref<AiModelProfileView>()
const draft = reactive<ModelDraft>(defaultDraft())
const apiKey = ref('')
const loading = ref(false)
const saving = ref(false)
const validating = ref(false)
const clearingKey = ref(false)
const archiving = ref(false)
const savingDefault = ref<'conversation' | 'vision'>()
const validationResult = ref<AiModelValidationResult>()
const originalFingerprint = ref('')
const nameInputRef = ref<{ focus: () => void }>()

const protocolOptions = [
  { label: 'OpenAI Chat Completions', value: 'openai_chat_completions' },
  { label: 'OpenAI Responses', value: 'openai_responses' },
  { label: 'Anthropic Messages', value: 'anthropic_messages' },
]
const transportOptions = [
  { label: '云端兼容服务', value: 'open_ai_compatible' },
  { label: '本地 HTTP 服务', value: 'local_http' },
]
const protocolLabels: Record<AiProviderProtocol, string> = {
  openai_chat_completions: 'Chat Completions',
  openai_responses: 'Responses',
  anthropic_messages: 'Anthropic Messages',
}

const canManageModels = [
  gateway.listAiModelProfiles,
  gateway.getAiModelProfile,
  gateway.createAiModelProfile,
  gateway.updateAiModelProfile,
  gateway.validateAiModelProfile,
  gateway.clearAiModelProfileKey,
  gateway.archiveAiModelProfile,
  gateway.getAiModelDefaults,
  gateway.saveAiModelDefaults,
].every((method) => typeof method === 'function')

const isCreating = computed(() => !activeProfile.value)
const normalizedBaseUrl = computed(() => draft.baseUrl.trim().replace(/\/+$/, ''))
const originalCredentialIdentity = computed(() => {
  const profile = activeProfile.value
  return profile
    ? [profile.protocol, profile.transport, profile.baseUrl.trim().replace(/\/+$/, '')].join('|')
    : ''
})
const credentialIdentity = computed(() => [
  draft.protocol,
  draft.transport,
  normalizedBaseUrl.value,
].join('|'))
const credentialIdentityChanged = computed(() =>
  Boolean(activeProfile.value)
  && credentialIdentity.value !== originalCredentialIdentity.value)
const storedCredentialAvailable = computed(() =>
  Boolean(activeProfile.value?.hasKey) && !credentialIdentityChanged.value)
const keyRequiredToSave = computed(() =>
  credentialIdentityChanged.value
  || (isCreating.value && draft.transport === 'open_ai_compatible'))

const allocatedTokens = computed(() => draft.maxInputTokens + draft.maxOutputTokens)
const unallocatedTokens = computed(() => draft.contextWindowTokens - allocatedTokens.value)
const budgetPercent = computed(() => Math.min(
  100,
  Math.max(0, Math.round(allocatedTokens.value / Math.max(1, draft.contextWindowTokens) * 100)),
))
const connectionConfigurationError = computed(() => {
  if (!draft.baseUrl.trim()) return 'Base URL 不能为空。'
  if (draft.baseUrl.trim().length > 2048) return 'Base URL 不能超过 2,048 个字符。'
  if (!draft.modelId.trim()) return '模型 ID 不能为空。'
  if (draft.modelId.trim().length > 256) return '模型 ID 不能超过 256 个字符。'
  if (draft.contextWindowTokens < 4096 || draft.contextWindowTokens > 2_000_000) {
    return '上下文窗口必须在 4,096–2,000,000 Token 之间。'
  }
  if (draft.maxInputTokens < 1024 || draft.maxInputTokens > 1_900_000) {
    return '最大输入必须在 1,024–1,900,000 Token 之间。'
  }
  if (draft.maxOutputTokens < 1 || draft.maxOutputTokens > 131072) {
    return '最大输出必须在 1–131,072 Token 之间。'
  }
  if (allocatedTokens.value > draft.contextWindowTokens) {
    return `输入与输出合计超出上下文 ${allocatedTokens.value - draft.contextWindowTokens} Token。`
  }
  if (
    draft.historyTokenBudget < 0
    || draft.historyTokenBudget > draft.maxInputTokens
    || draft.historyTokenBudget > 1_000_000
  ) {
    return '历史消息预算不能超过最大输入 Token 或 1,000,000。'
  }
  if (draft.historyTurns < 0 || draft.historyTurns > 100) {
    return '历史保留轮数必须在 0–100 之间。'
  }
  if (draft.temperature < 0 || draft.temperature > 2) {
    return 'Temperature 必须在 0–2 之间。'
  }
  if (draft.timeoutMs < 100 || draft.timeoutMs > 600000) {
    return '请求超时必须在 100–600,000 ms 之间。'
  }
  return ''
})
const configurationError = computed(() => {
  if (!draft.name.trim()) return '配置名称不能为空。'
  if (draft.name.trim().length > 120) return '配置名称不能超过 120 个字符。'
  return connectionConfigurationError.value
})
const saveError = computed(() => {
  if (configurationError.value) return configurationError.value
  if (activeProfile.value?.isDefaultVision && !draft.supportsVision) {
    return '请先取消默认视觉模型，再关闭这个档案的视觉能力。'
  }
  if (keyRequiredToSave.value && !apiKey.value.trim()) {
    if (credentialIdentityChanged.value) {
      return '协议、连接方式或 Base URL 已变化，必须重新输入 API Key。'
    }
    return '云端模型首次保存必须填写 API Key。'
  }
  return ''
})
const canSave = computed(() =>
  canManageModels && !saveError.value && (isCreating.value || isDirty.value))

function draftFingerprint() {
  return JSON.stringify({
    ...draft,
    name: draft.name.trim(),
    baseUrl: normalizedBaseUrl.value,
    modelId: draft.modelId.trim(),
  })
}

const isDirty = computed(() =>
  apiKey.value.trim().length > 0 || draftFingerprint() !== originalFingerprint.value)

watch(
  [() => draftFingerprint(), apiKey],
  () => {
    validationResult.value = undefined
  },
)

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : '操作失败，请重试'
}

function validationLabel(code?: string) {
  const labels: Record<string, string> = {
    ai_api_key_missing: '当前配置没有可用的 API Key',
    missing_credential: '当前配置没有可用的 API Key',
    provider_exit_not_approved: 'Base URL 尚未获得实验室出口批准',
    invalid_provider: 'Provider 配置无效',
    invalid_configuration: 'Provider 配置无效',
    invalid_request: 'Provider 请求配置无效',
    api_key_rejected: 'API Key 被 Provider 拒绝',
    model_not_found: '模型不存在或当前账号无权访问',
    provider_http_error: 'Provider 返回 HTTP 错误',
    http_status: 'Provider 返回 HTTP 错误',
    provider_unreachable: 'Base URL 无法连接',
    connection: 'Base URL 无法连接',
    provider_transport_error: 'Provider 请求传输失败',
    request: 'Provider 请求传输失败',
    request_timeout: '连接验证超时',
    timeout: '连接验证超时',
    response_format_incompatible: 'Provider 响应与所选协议不兼容',
    malformed_response: 'Provider 响应与所选协议不兼容',
    empty_response: 'Provider 返回空响应',
    context_exceeded: '验证请求超过 Provider 上下文上限',
    request_too_large: '验证请求超过 Provider 上限',
    response_too_large: 'Provider 响应超过安全上限',
    output_budget_exhausted: 'Provider 未在验证输出预算内完成响应',
    provider_unavailable: 'Provider 当前不可用',
  }
  return labels[code ?? ''] ?? code ?? '未知 Provider 错误'
}

function hydrateDraft(profile?: AiModelProfileView) {
  Object.assign(draft, profile
    ? {
        name: profile.name,
        protocol: profile.protocol,
        transport: profile.transport,
        baseUrl: profile.baseUrl,
        modelId: profile.modelId,
        supportsVision: profile.supportsVision,
        contextWindowTokens: profile.contextWindowTokens,
        maxInputTokens: profile.maxInputTokens,
        maxOutputTokens: profile.maxOutputTokens,
        historyTokenBudget: profile.historyTokenBudget,
        historyTurns: profile.historyTurns,
        temperature: profile.temperature,
        timeoutMs: profile.timeoutMs,
      }
    : defaultDraft())
  apiKey.value = ''
  validationResult.value = undefined
  originalFingerprint.value = draftFingerprint()
}

async function load() {
  if (!canManageModels) return
  loading.value = true
  try {
    const [loadedProfiles, loadedDefaults] = await Promise.all([
      gateway.listAiModelProfiles!(),
      gateway.getAiModelDefaults!(),
    ])
    defaults.value = loadedDefaults
    profiles.value = loadedProfiles
      .filter((profile) => !profile.archivedAt)
      .map((profile) => ({
        ...profile,
        isDefaultConversation:
          loadedDefaults.defaultConversationProfileId === profile.id,
        isDefaultVision: loadedDefaults.defaultVisionProfileId === profile.id,
      }))
  } catch (error) {
    message.error(`无法读取模型配置：${errorMessage(error)}`)
  } finally {
    loading.value = false
  }
}

async function openProfile(profile: AiModelProfileView) {
  if (!gateway.getAiModelProfile) return
  loading.value = true
  let opened = false
  try {
    const loaded = await gateway.getAiModelProfile(profile.id)
    activeProfile.value = {
      ...loaded,
      isDefaultConversation: defaults.value.defaultConversationProfileId === loaded.id,
      isDefaultVision: defaults.value.defaultVisionProfileId === loaded.id,
    }
    hydrateDraft(activeProfile.value)
    mode.value = 'detail'
    opened = true
  } catch (error) {
    message.error(`无法打开模型配置：${errorMessage(error)}`)
  } finally {
    loading.value = false
  }
  if (opened) {
    await nextTick()
    nameInputRef.value?.focus()
  }
}

async function createProfile() {
  activeProfile.value = undefined
  hydrateDraft()
  mode.value = 'detail'
  await nextTick()
  nameInputRef.value?.focus()
}

function closeEditor() {
  mode.value = 'list'
  activeProfile.value = undefined
  hydrateDraft()
}

function backToList() {
  if (!isDirty.value) {
    closeEditor()
    return
  }
  dialog.warning({
    title: '放弃未保存的更改？',
    content: '当前模型配置中的修改和新输入的 API Key 都不会保存。',
    positiveText: '放弃更改',
    negativeText: '继续编辑',
    onPositiveClick: closeEditor,
  })
}

function saveInput(): SaveAiModelProfileInput {
  const input: SaveAiModelProfileInput = {
    name: draft.name.trim(),
    protocol: draft.protocol,
    transport: draft.transport,
    baseUrl: draft.baseUrl.trim(),
    modelId: draft.modelId.trim(),
    supportsVision: draft.supportsVision,
    contextWindowTokens: draft.contextWindowTokens,
    maxInputTokens: draft.maxInputTokens,
    maxOutputTokens: draft.maxOutputTokens,
    historyTokenBudget: draft.historyTokenBudget,
    historyTurns: draft.historyTurns,
    temperature: draft.temperature,
    timeoutMs: draft.timeoutMs,
  }
  if (activeProfile.value) input.expectedRevision = activeProfile.value.revision
  if (apiKey.value.trim()) input.apiKey = apiKey.value.trim()
  return input
}

function validationInput(): ValidateAiModelProfileInput {
  const {
    name: _name,
    expectedRevision: _expectedRevision,
    ...configuration
  } = saveInput()
  const input: ValidateAiModelProfileInput = configuration
  if (activeProfile.value && !credentialIdentityChanged.value) {
    input.profileId = activeProfile.value.id
    input.currentVersion = activeProfile.value.currentVersion
  }
  return input
}

async function saveProfile() {
  if (!gateway.createAiModelProfile || !gateway.updateAiModelProfile) return
  if (saveError.value) {
    message.warning(saveError.value)
    return
  }
  saving.value = true
  try {
    const saved = activeProfile.value
      ? await gateway.updateAiModelProfile(activeProfile.value.id, saveInput())
      : await gateway.createAiModelProfile(saveInput())
    apiKey.value = ''
    message.success(activeProfile.value ? '模型配置已更新' : '模型配置已创建')
    await load()
    await openProfile(saved)
  } catch (error) {
    message.error(`保存失败：${errorMessage(error)}`)
  } finally {
    saving.value = false
  }
}

async function validateProfile() {
  if (!gateway.validateAiModelProfile) return
  if (connectionConfigurationError.value) {
    message.warning(connectionConfigurationError.value)
    return
  }
  validating.value = true
  validationResult.value = undefined
  try {
    const result = await gateway.validateAiModelProfile(validationInput())
    validationResult.value = result
    if (result.ok) message.success(`连接验证通过（${result.latencyMs} ms）`)
    else message.error(`连接验证失败：${validationLabel(result.errorCode)}`)
  } catch (error) {
    message.error(`连接验证失败：${errorMessage(error)}`)
  } finally {
    validating.value = false
  }
}

function clearKey() {
  const profile = activeProfile.value
  if (!gateway.clearAiModelProfileKey || !profile?.hasKey) return
  if (isDirty.value) {
    message.warning('请先保存或放弃当前表单更改，再清除密钥')
    return
  }
  dialog.warning({
    title: '清除这个模型的 API Key？',
    content: '只清除当前模型档案绑定的凭据，不会删除档案或影响其他模型。',
    positiveText: '清除密钥',
    negativeText: '取消',
    async onPositiveClick() {
      clearingKey.value = true
      try {
        activeProfile.value = await gateway.clearAiModelProfileKey!(profile.id)
        hydrateDraft(activeProfile.value)
        await load()
        message.success(gateway.mode === 'local'
          ? 'API Key 已从 OS keyring 清除'
          : 'API Key 已从个人加密 secret store 清除')
      } catch (error) {
        message.error(`清除失败：${errorMessage(error)}`)
      } finally {
        clearingKey.value = false
      }
    },
  })
}

function archiveProfile() {
  const profile = activeProfile.value
  if (!gateway.archiveAiModelProfile || !profile) return
  if (isDirty.value) {
    message.warning('请先保存或放弃当前表单更改，再停用模型')
    return
  }
  const defaultNotice = profile.isDefaultConversation || profile.isDefaultVision
    ? ' 该配置当前是默认模型；归档后对应默认值将被取消。'
    : ''
  dialog.warning({
    title: '停用这个模型配置？',
    content: `停用后历史会话仍可读取，但不能再用它发起新请求。${defaultNotice}`,
    positiveText: '停用模型',
    negativeText: '取消',
    async onPositiveClick() {
      archiving.value = true
      try {
        await gateway.archiveAiModelProfile!(profile.id, profile.revision)
        await load()
        closeEditor()
        message.success('模型配置已停用')
      } catch (error) {
        message.error(`停用失败：${errorMessage(error)}`)
      } finally {
        archiving.value = false
      }
    },
  })
}

async function setDefault(purpose: 'conversation' | 'vision') {
  const profile = activeProfile.value
  if (!gateway.saveAiModelDefaults || !profile) return
  if (isDirty.value) {
    message.warning('请先保存或放弃当前表单更改，再更新默认模型')
    return
  }
  if (purpose === 'vision' && !profile.supportsVision) {
    message.warning('只有明确启用视觉能力的模型才能设为默认视觉模型')
    return
  }
  savingDefault.value = purpose
  try {
    defaults.value = await gateway.saveAiModelDefaults({
      defaultConversationProfileId: purpose === 'conversation'
        ? (profile.isDefaultConversation ? null : profile.id)
        : (defaults.value.defaultConversationProfileId ?? null),
      defaultVisionProfileId: purpose === 'vision'
        ? (profile.isDefaultVision ? null : profile.id)
        : (defaults.value.defaultVisionProfileId ?? null),
      expectedRevision: defaults.value.revision,
    })
    await load()
    const refreshed = profiles.value.find((item) => item.id === profile.id)
    if (refreshed) {
      activeProfile.value = refreshed
      hydrateDraft(refreshed)
    }
    message.success(purpose === 'conversation' ? '默认对话模型已更新' : '默认视觉模型已更新')
  } catch (error) {
    message.error(`更新默认模型失败：${errorMessage(error)}`)
  } finally {
    savingDefault.value = undefined
  }
}

onMounted(load)
</script>

<template>
  <section class="model-settings" aria-label="AI 模型设置">
    <n-alert
      v-if="!canManageModels"
      type="info"
      :bordered="false"
      class="availability-alert"
    >
      当前运行出口尚未提供多模型管理接口；界面不会回退到单一旧配置或读取演示密钥。
    </n-alert>

    <template v-else-if="mode === 'list'">
      <header class="model-list-heading">
        <div>
          <h2 id="ai-model-settings-title">AI 与模型</h2>
          <p>每个模型独立保存协议、Base URL、运行参数和加密凭据。</p>
        </div>
        <n-button type="primary" :disabled="loading" @click="createProfile">
          <template #icon><Plus :size="16" /></template>
          添加模型
        </n-button>
      </header>

      <n-alert type="info" :bordered="false" class="model-boundary">
        模型 ID 完全自由填写。MuriArc 不会根据模型名称套用或覆盖参数；验证连接也不会保存当前表单。
      </n-alert>

      <div v-if="loading" class="loading-state" aria-live="polite">
        <n-spin size="small" />
        <span>正在读取模型配置…</span>
      </div>
      <div v-else-if="profiles.length === 0" class="empty-state">
        <Bot :size="24" />
        <strong>还没有模型配置</strong>
        <span>添加对话模型或视觉模型；没有默认对话模型时，发起新会话前需要明确选择。</span>
        <n-button type="primary" secondary @click="createProfile">添加第一个模型</n-button>
      </div>
      <ul v-else class="model-list" aria-label="模型配置">
        <li v-for="profile in profiles" :key="profile.id">
          <button
            type="button"
            class="model-row"
            @click="openProfile(profile)"
          >
            <span class="model-icon" aria-hidden="true">
              <Eye v-if="profile.supportsVision" :size="18" />
              <Bot v-else :size="18" />
            </span>
            <span class="model-identity">
              <span class="model-name-line">
                <strong>{{ profile.name }}</strong>
                <n-tag v-if="profile.isDefaultConversation" size="small" :bordered="false">默认对话</n-tag>
                <n-tag v-if="profile.isDefaultVision" size="small" :bordered="false" type="success">默认视觉</n-tag>
              </span>
              <code>{{ profile.modelId }}</code>
              <small>{{ protocolLabels[profile.protocol] }} · {{ profile.supportsVision ? '支持视觉' : '仅文本' }}</small>
            </span>
            <span class="credential-status" :class="{ configured: profile.hasKey }">
              <KeyRound :size="14" />
              {{ profile.hasKey ? '密钥已配置' : '未配置密钥' }}
            </span>
          </button>
        </li>
      </ul>
    </template>

    <template v-else>
      <header class="model-detail-heading">
        <button type="button" class="back-button" aria-label="返回模型列表" @click="backToList">
          <ArrowLeft :size="18" />
        </button>
        <div>
          <span>模型配置</span>
          <h2 id="ai-model-settings-title">{{ isCreating ? '添加模型' : activeProfile?.name }}</h2>
          <small v-if="activeProfile">档案版本 {{ activeProfile.currentVersion }} · revision {{ activeProfile.revision }}</small>
          <small v-else>先填写完整配置，可在保存前验证连接</small>
        </div>
      </header>

      <div v-if="activeProfile" class="default-actions" aria-label="默认模型">
        <button
          type="button"
          :class="{ active: activeProfile.isDefaultConversation }"
          :disabled="Boolean(savingDefault)"
          @click="setDefault('conversation')"
        >
          <Check :size="15" />
          {{ activeProfile.isDefaultConversation ? '默认对话模型' : '设为默认对话' }}
        </button>
        <button
          type="button"
          :class="{ active: activeProfile.isDefaultVision }"
          :disabled="!activeProfile.supportsVision || Boolean(savingDefault)"
          @click="setDefault('vision')"
        >
          <Eye :size="15" />
          {{ activeProfile.isDefaultVision ? '默认视觉模型' : '设为默认视觉' }}
        </button>
      </div>

      <n-form label-placement="top" class="model-form" :disabled="loading || saving">
        <div class="form-section">
          <div class="form-section-heading">
            <Bot :size="17" />
            <div><strong>基本信息</strong><span>名称用于界面识别，模型 ID 原样发送给 Provider。</span></div>
          </div>
          <div class="form-grid">
            <n-form-item label="配置名称">
              <n-input ref="nameInputRef" v-model:value="draft.name" maxlength="120" placeholder="例如：实验室对话模型" />
            </n-form-item>
            <n-form-item label="模型 ID">
              <n-input v-model:value="draft.modelId" maxlength="256" placeholder="自由输入 Provider 支持的模型 ID" />
            </n-form-item>
            <n-form-item label="协议">
              <n-select v-model:value="draft.protocol" :options="protocolOptions" />
            </n-form-item>
            <n-form-item label="连接方式">
              <n-select v-model:value="draft.transport" :options="transportOptions" />
            </n-form-item>
            <n-form-item label="Base URL" class="full-row">
              <n-input v-model:value="draft.baseUrl" maxlength="2048" placeholder="https://api.example.com/v1" />
              <template #feedback>只填写服务根地址；标准协议端点由适配器拼接，自定义地址仍受实验室出口审批约束。</template>
            </n-form-item>
          </div>
        </div>

        <div class="form-section">
          <div class="form-section-heading">
            <KeyRound :size="17" />
            <div><strong>个人凭据</strong><span>密钥只写入受保护的 secret store，读取接口永不回显明文。</span></div>
          </div>
          <n-form-item label="API Key">
            <n-input
              v-model:value="apiKey"
              type="password"
              show-password-on="click"
              autocomplete="new-password"
              :placeholder="storedCredentialAvailable ? '已配置；留空保持当前密钥' : (draft.transport === 'local_http' ? '本地服务无鉴权时可留空' : '请输入这个 Provider 的 API Key')"
            />
            <template #feedback>
              {{ credentialIdentityChanged
                ? '协议、连接方式或 Base URL 已变化；为避免把旧凭据发送给其他服务，必须重新输入。'
                : '修改名称、模型 ID 或运行参数时，留空会保留当前版本的凭据。' }}
            </template>
          </n-form-item>
          <n-alert
            v-if="credentialIdentityChanged"
            type="warning"
            :bordered="false"
            title="Provider 身份已变化"
          >
            保存和验证都不会复用旧密钥。请输入新 API Key，旧版本仍保留自己的独立凭据。
          </n-alert>
        </div>

        <div class="form-section">
          <div class="form-section-heading">
            <SlidersHorizontal :size="17" />
            <div><strong>运行参数</strong><span>参数完全由你维护，不会根据模型名称自动覆盖。</span></div>
          </div>
          <div class="token-budget-card">
            <div class="token-budget-heading">
              <strong>上下文预算</strong>
              <span>已分配 {{ allocatedTokens.toLocaleString() }} / {{ draft.contextWindowTokens.toLocaleString() }} Token</span>
            </div>
            <n-progress
              type="line"
              :percentage="budgetPercent"
              :show-indicator="false"
              :status="connectionConfigurationError ? 'error' : 'success'"
            />
            <div class="token-budget-breakdown">
              <span>输入 {{ draft.maxInputTokens.toLocaleString() }}</span>
              <span>输出 {{ draft.maxOutputTokens.toLocaleString() }}</span>
              <span>未分配 {{ Math.max(0, unallocatedTokens).toLocaleString() }}</span>
            </div>
          </div>
          <div class="form-grid runtime-grid">
            <n-form-item label="上下文窗口 Token">
              <n-input-number v-model:value="draft.contextWindowTokens" :min="4096" :max="2000000" :step="1024" />
            </n-form-item>
            <n-form-item label="最大输入 Token">
              <n-input-number v-model:value="draft.maxInputTokens" :min="1024" :max="1900000" :step="1024" />
            </n-form-item>
            <n-form-item label="最大输出 Token">
              <n-input-number v-model:value="draft.maxOutputTokens" :min="1" :max="131072" :step="256" />
            </n-form-item>
            <n-form-item label="历史消息预算">
              <n-input-number
                v-model:value="draft.historyTokenBudget"
                :min="0"
                :max="Math.min(draft.maxInputTokens, 1_000_000)"
                :step="1024"
              />
            </n-form-item>
            <n-form-item label="历史保留轮数">
              <n-input-number v-model:value="draft.historyTurns" :min="0" :max="100" />
            </n-form-item>
            <n-form-item label="请求超时（毫秒）">
              <n-input-number v-model:value="draft.timeoutMs" :min="100" :max="600000" :step="1000" />
            </n-form-item>
            <n-form-item label="Temperature" class="full-row">
              <n-input-number v-model:value="draft.temperature" :min="0" :max="2" :step="0.1" />
            </n-form-item>
            <n-form-item label="视觉能力" class="full-row">
              <div class="vision-toggle">
                <div><strong>允许这个模型处理图片</strong><span>可同时保存多个视觉模型，但默认视觉模型最多一个。</span></div>
                <n-switch v-model:value="draft.supportsVision" />
              </div>
            </n-form-item>
          </div>
        </div>
      </n-form>

      <div v-if="configurationError || saveError" class="validation-message" role="alert">
        {{ saveError || configurationError }}
      </div>
      <div
        v-if="validationResult"
        class="validation-result"
        :class="{ success: validationResult.ok }"
        role="status"
      >
        <ShieldCheck :size="17" />
        <span>{{ validationResult.ok
          ? `连接验证通过，耗时 ${validationResult.latencyMs} ms；当前表单仍未保存。`
          : `连接验证失败：${validationLabel(validationResult.errorCode)}；当前表单未保存。` }}</span>
      </div>

      <footer class="editor-actions">
        <div class="primary-actions">
          <n-button type="primary" :disabled="!canSave" :loading="saving" @click="saveProfile">
            <template #icon><Save :size="16" /></template>
            {{ isCreating ? '创建模型' : '保存新版本' }}
          </n-button>
          <n-button secondary :disabled="Boolean(connectionConfigurationError)" :loading="validating" @click="validateProfile">
            验证当前表单
          </n-button>
        </div>
        <div v-if="activeProfile" class="danger-actions">
          <n-button
            v-if="activeProfile.hasKey"
            secondary
            :loading="clearingKey"
            @click="clearKey"
          >
            清除密钥
          </n-button>
          <n-button type="error" secondary :loading="archiving" @click="archiveProfile">
            <template #icon><Archive :size="16" /></template>
            停用模型
          </n-button>
        </div>
      </footer>
      <p class="validation-footnote">连接验证使用当前未保存表单，不会写入档案，也不是保存前置条件。</p>
    </template>
  </section>
</template>

<style scoped>
.model-settings { min-width: 0; }
.availability-alert, .model-boundary { margin-bottom: 16px; }
.model-list-heading, .model-detail-heading { display: flex; min-width: 0; align-items: flex-start; justify-content: space-between; gap: 16px; margin-bottom: 18px; }
.model-list-heading h2, .model-detail-heading h2 { margin: 0; font-size: 19px; line-height: 1.35; overflow-wrap: anywhere; }
.model-list-heading p { margin: 4px 0 0; color: var(--muri-text-secondary); line-height: 1.55; }
.loading-state, .empty-state { display: flex; min-height: 220px; align-items: center; justify-content: center; flex-direction: column; gap: 9px; color: var(--muri-text-secondary); text-align: center; }
.empty-state > svg { color: var(--muri-primary); }
.empty-state span { max-width: 440px; color: var(--muri-text-tertiary); font-size: 12px; line-height: 1.6; }
.model-list { display: flex; margin: 0; padding: 0; flex-direction: column; gap: 9px; list-style: none; }
.model-row { display: grid; width: 100%; min-width: 0; min-height: 78px; grid-template-columns: 38px minmax(0, 1fr) auto; align-items: center; gap: 11px; padding: 11px 13px; text-align: left; border: 1px solid var(--muri-border); border-radius: 8px; color: var(--muri-text); background: white; cursor: pointer; transition: border-color var(--muri-transition-fast), box-shadow var(--muri-transition-fast), background var(--muri-transition-fast); }
.model-row:hover { border-color: var(--muri-border-strong); background: var(--muri-surface-muted); box-shadow: 0 3px 12px rgba(30, 53, 76, .06); }
.model-row:focus-visible, .back-button:focus-visible, .default-actions button:focus-visible { outline: 3px solid rgba(15, 95, 170, .22); outline-offset: 2px; }
.model-icon { display: grid; width: 36px; height: 36px; place-items: center; border-radius: 8px; color: var(--muri-primary); background: var(--muri-primary-soft); }
.model-identity, .model-name-line { display: flex; min-width: 0; }
.model-identity { flex-direction: column; gap: 3px; }
.model-name-line { align-items: center; flex-wrap: wrap; gap: 5px; }
.model-identity code { overflow: hidden; color: var(--muri-text-secondary); font-size: 11px; text-overflow: ellipsis; white-space: nowrap; }
.model-identity small { color: var(--muri-text-tertiary); font-size: 10px; }
.credential-status { display: inline-flex; align-items: center; gap: 5px; color: var(--muri-text-tertiary); font-size: 11px; white-space: nowrap; }
.credential-status.configured { color: var(--muri-success); }
.model-detail-heading { justify-content: flex-start; }
.back-button { display: grid; width: 38px; height: 38px; flex: 0 0 auto; place-items: center; border: 1px solid var(--muri-border); border-radius: 8px; color: var(--muri-text-secondary); background: white; cursor: pointer; transition: color var(--muri-transition-fast), border-color var(--muri-transition-fast), background var(--muri-transition-fast); }
.back-button:hover { border-color: var(--muri-border-strong); color: var(--muri-primary); background: var(--muri-primary-soft); }
.model-detail-heading > div { min-width: 0; }
.model-detail-heading span, .model-detail-heading small { display: block; color: var(--muri-text-tertiary); font-size: 11px; }
.default-actions { display: flex; flex-wrap: wrap; gap: 7px; margin: -3px 0 16px 54px; }
.default-actions button { display: inline-flex; min-height: 34px; align-items: center; gap: 6px; padding: 0 10px; border: 1px solid var(--muri-border); border-radius: 7px; color: var(--muri-text-secondary); background: white; cursor: pointer; transition: color var(--muri-transition-fast), border-color var(--muri-transition-fast), background var(--muri-transition-fast); }
.default-actions button:hover:not(:disabled), .default-actions button.active { border-color: #a9c8e1; color: var(--muri-primary); background: var(--muri-primary-soft); }
.default-actions button:disabled { cursor: not-allowed; opacity: .55; }
.model-form { display: flex; flex-direction: column; gap: 13px; }
.form-section { padding: 15px; border: 1px solid var(--muri-border); border-radius: 9px; background: white; }
.form-section-heading { display: flex; align-items: flex-start; gap: 9px; margin-bottom: 15px; }
.form-section-heading > svg { flex: 0 0 auto; margin-top: 1px; color: var(--muri-primary); }
.form-section-heading > div { display: flex; min-width: 0; flex-direction: column; }
.form-section-heading span { color: var(--muri-text-tertiary); font-size: 11px; line-height: 1.5; }
.form-grid { display: grid; min-width: 0; grid-template-columns: 1fr 1fr; gap: 0 14px; }
.form-grid :deep(.full-row) { grid-column: 1 / -1; }
.form-grid :deep(.n-input-number) { width: 100%; }
.token-budget-card { margin-bottom: 14px; padding: 12px; border: 1px solid var(--muri-border); border-radius: 7px; background: var(--muri-surface-muted); }
.token-budget-heading, .token-budget-breakdown { display: flex; justify-content: space-between; flex-wrap: wrap; gap: 7px 12px; }
.token-budget-heading { margin-bottom: 8px; }
.token-budget-heading span, .token-budget-breakdown { color: var(--muri-text-secondary); font-size: 11px; }
.token-budget-breakdown { margin-top: 7px; }
.vision-toggle { display: flex; width: 100%; align-items: center; justify-content: space-between; gap: 14px; padding: 10px 12px; border: 1px solid var(--muri-border); border-radius: 7px; }
.vision-toggle > div { display: flex; min-width: 0; flex-direction: column; }
.vision-toggle span { color: var(--muri-text-tertiary); font-size: 11px; line-height: 1.5; }
.validation-message { margin: 12px 0 0; color: var(--muri-danger, #c2413b); font-size: 12px; line-height: 1.5; }
.validation-result { display: flex; align-items: flex-start; gap: 8px; margin-top: 12px; padding: 10px 12px; border-left: 3px solid var(--muri-danger, #c2413b); color: var(--muri-text-secondary); background: #fff7f7; font-size: 12px; line-height: 1.5; }
.validation-result.success { border-color: var(--muri-success); background: #f2fbf7; }
.validation-result > svg { flex: 0 0 auto; color: var(--muri-danger, #c2413b); }
.validation-result.success > svg { color: var(--muri-success); }
.editor-actions { display: flex; align-items: center; justify-content: space-between; flex-wrap: wrap; gap: 9px 14px; margin-top: 17px; padding-top: 15px; border-top: 1px solid var(--muri-border); }
.primary-actions, .danger-actions { display: flex; flex-wrap: wrap; gap: 8px; }
.validation-footnote { margin: 8px 0 0; color: var(--muri-text-tertiary); font-size: 11px; line-height: 1.5; }
@media (max-width: 700px) {
  .model-settings :deep(.n-button) { min-height: 44px; }
  .model-list-heading { align-items: stretch; flex-direction: column; }
  .model-list-heading :deep(.n-button) { align-self: flex-start; }
  .back-button { width: 44px; height: 44px; }
  .model-row { grid-template-columns: 36px minmax(0, 1fr); }
  .credential-status { grid-column: 2; white-space: normal; }
  .default-actions { margin-left: 0; }
  .default-actions button { min-height: 44px; }
  .form-grid { grid-template-columns: 1fr; }
  .form-grid :deep(.n-form-item) { grid-column: 1 !important; }
  .editor-actions { align-items: stretch; flex-direction: column; }
  .primary-actions, .danger-actions { display: grid; grid-template-columns: 1fr; }
  .primary-actions :deep(.n-button), .danger-actions :deep(.n-button) { width: 100%; }
}
@media (max-width: 420px) {
  .form-section { padding: 12px; }
  .model-row { padding: 10px; }
  .vision-toggle { align-items: flex-start; }
}
@media (prefers-reduced-motion: reduce) {
  .model-row, .back-button, .default-actions button { transition: none; }
}
</style>
