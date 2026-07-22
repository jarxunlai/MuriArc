<script setup lang="ts">
import { computed, onMounted, reactive, ref } from 'vue'
import { useDialog, useMessage } from 'naive-ui'
import { useRouter } from 'vue-router'
import { Archive, Bot, ChevronRight, Database, Dna, ExternalLink, FolderKanban, KeyRound, Save, ShieldCheck, SlidersHorizontal, Users } from '@lucide/vue'
import { branding } from '@/branding'
import { currentAuthSession, gateway, HttpGatewayError } from '@/services/gateway'
import type { AiProviderPreset, SaveAiSettingsInput, WorkspaceSettings } from '@/domain/models'
import { createDataGateway } from '@/services/dataGateway'
import { passwordPolicyError, passwordStrength } from '@/services/passwordStrength'
import { hasLabRegistryAccess } from '@/services/projectContext'
import PageHeader from '@/components/PageHeader.vue'
import { builtinAiProviderPresets } from '@/services/aiProviderPresets'

const message = useMessage()
const dataGateway = createDataGateway(gateway)
const dialog = useDialog()
const router = useRouter()
const active = ref('workspace')
const profileDisplayName = ref(currentAuthSession.value?.user.displayName ?? '')
const currentPassword = ref('')
const newPassword = ref('')
const passwordConfirmation = ref('')
const savingProfile = ref(false)
const changingPassword = ref(false)
const workspace = reactive<WorkspaceSettings>({
  operatorName: '',
  labName: '',
})
const ai = reactive<SaveAiSettingsInput>({
  enabled: true,
  providerKind: 'open_ai_compatible',
  providerPresetId: 'deepseek',
  model: 'deepseek-chat',
  baseUrl: 'https://api.deepseek.com',
  supportsVision: false,
  visionModel: undefined,
  contextWindowTokens: 131072,
  maxInputTokens: 65536,
  maxOutputTokens: 4096,
  historyTokenBudget: 32768,
  historyTurns: 20,
  temperature: 0,
  timeoutMs: 120000,
})
const apiKey = ref('')
const hasStoredApiKey = ref(false)
const storedCredentialBinding = ref('')
const providerPresets = ref<AiProviderPreset[]>(structuredClone(builtinAiProviderPresets))
const credentialBinding = computed(() => [
  ai.providerKind,
  ai.providerPresetId,
  ai.baseUrl.trim().replace(/\/$/, ''),
].join('|'))
const hasApiKey = computed(() => hasStoredApiKey.value
  && storedCredentialBinding.value === credentialBinding.value)
const savedAiFingerprint = ref('')
function aiFingerprint() {
  return JSON.stringify({
    enabled: ai.enabled,
    providerKind: ai.providerKind,
    providerPresetId: ai.providerPresetId,
    model: ai.model.trim(),
    baseUrl: ai.baseUrl.trim().replace(/\/$/, ''),
    supportsVision: ai.supportsVision,
    visionModel: ai.supportsVision ? ai.visionModel?.trim() ?? '' : '',
    contextWindowTokens: ai.contextWindowTokens,
    maxInputTokens: ai.maxInputTokens,
    maxOutputTokens: ai.maxOutputTokens,
    historyTokenBudget: ai.historyTokenBudget,
    historyTurns: ai.historyTurns,
    temperature: ai.temperature,
    timeoutMs: ai.timeoutMs,
  })
}
const isAiDirty = computed(() => apiKey.value.trim().length > 0 || aiFingerprint() !== savedAiFingerprint.value)
const loadingWorkspace = ref(false)
const loadingAi = ref(false)
const savingWorkspace = ref(false)
const savingAi = ref(false)
const testingAi = ref(false)
const clearingKey = ref(false)
const loggingOut = ref(false)
const snapshotting = ref(false)
const accountUser = computed(() => currentAuthSession.value?.user)
const accountIsEnvironmentRoot = computed(() => accountUser.value?.isEnvironmentRoot === true)
const accountAvailable = gateway.mode === 'remote'
  && typeof gateway.updateProfile === 'function'
  && typeof gateway.changePassword === 'function'
const accountPasswordStrength = computed(() => passwordStrength(newPassword.value))
const canManageWorkspace = typeof gateway.getWorkspaceSettings === 'function'
  && typeof gateway.saveWorkspaceSettings === 'function'
const canManageAi = typeof gateway.getAiSettings === 'function'
  && typeof gateway.saveAiSettings === 'function'
  && typeof gateway.clearAiApiKey === 'function'
const canManageMembers = gateway.mode === 'remote'
  && currentAuthSession.value?.user.labRoles.includes('lab_admin') === true
const labRegistryAvailable = gateway.mode === 'local' || hasLabRegistryAccess()
const enabledProviderPresets = computed(() => providerPresets.value.filter((preset) => preset.enabled))
const providerOptions = computed(() => enabledProviderPresets.value.map((preset) => ({
  label: preset.displayName,
  value: preset.id,
})))
const selectedPreset = computed(() => enabledProviderPresets.value.find(
  (preset) => preset.id === ai.providerPresetId,
))
const modelOptions = computed(() => (selectedPreset.value?.models ?? []).map((model) => ({
  label: model.displayName,
  value: model.id,
})))
const selectedModel = computed(() => selectedPreset.value?.models.find((model) => model.id === ai.model))
const allocatedTokens = computed(() => ai.maxInputTokens + ai.maxOutputTokens)
const unallocatedTokens = computed(() => ai.contextWindowTokens - allocatedTokens.value)
const budgetPercent = computed(() => Math.min(100, Math.max(0, Math.round(allocatedTokens.value / Math.max(1, ai.contextWindowTokens) * 100))))
const budgetError = computed(() => {
  if (ai.contextWindowTokens < 4096 || ai.contextWindowTokens > 2000000) return '上下文窗口必须在 4,096–2,000,000 Token 之间。'
  if (ai.maxInputTokens < 1024 || ai.maxInputTokens > 1900000) return '最大输入必须在 1,024–1,900,000 Token 之间。'
  if (ai.maxOutputTokens < 1 || ai.maxOutputTokens > 131072) return '最大输出必须在 1–131,072 Token 之间。'
  if (allocatedTokens.value > ai.contextWindowTokens) return `输入预算与输出预留合计超出上下文 ${allocatedTokens.value - ai.contextWindowTokens} Token。请降低最大输入或最大输出。`
  if (ai.historyTokenBudget < 0 || ai.historyTokenBudget > ai.maxInputTokens) return '历史消息预算不能超过最大输入 Token。'
  if (ai.historyTurns < 0 || ai.historyTurns > 100) return '历史保留轮数必须在 0–100 之间。'
  if (ai.temperature < 0 || ai.temperature > 2) return 'Temperature 必须在 0–2 之间。'
  if (ai.timeoutMs < 100 || ai.timeoutMs > 600000) return '请求超时必须在 100–600,000 ms 之间。'
  return ''
})
const basicAiError = computed(() => {
  if (!ai.providerPresetId) return '请选择 Provider。'
  if (!ai.baseUrl.trim()) return 'API 出口不能为空。'
  if (!ai.model.trim()) return '模型不能为空。'
  if (ai.supportsVision && !ai.visionModel?.trim()) return '启用视觉能力后必须填写视觉模型。'
  return ''
})
const canSaveAi = computed(() => canManageAi && !budgetError.value && !basicAiError.value)
const menu = computed(() => [
  { key: 'workspace', label: '工作空间', icon: Database },
  { key: 'account', label: '账号与安全', icon: KeyRound },
  { key: 'ai', label: 'AI 与模型', icon: Bot },
  ...(labRegistryAvailable ? [{ key: 'backup', label: '备份与迁移', icon: Archive }] : []),
  { key: 'security', label: '安全与审计', icon: ShieldCheck },
  { key: 'about', label: '关于 MuriArc', icon: FolderKanban },
])

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : '操作失败，请重试'
}

async function loadWorkspace() {
  if (!gateway.getWorkspaceSettings) return
  loadingWorkspace.value = true
  try {
    Object.assign(workspace, await gateway.getWorkspaceSettings())
  } catch (error) {
    message.error(`无法读取工作空间设置：${errorMessage(error)}`)
  } finally {
    loadingWorkspace.value = false
  }
}

async function loadAi() {
  if (!gateway.getAiSettings) return
  loadingAi.value = true
  try {
    const [loaded, presets] = await Promise.all([
      gateway.getAiSettings(),
      gateway.listAiProviderPresets
        ? gateway.listAiProviderPresets()
        : Promise.resolve(structuredClone(builtinAiProviderPresets)),
    ])
    providerPresets.value = presets.length ? presets : structuredClone(builtinAiProviderPresets)
    Object.assign(ai, {
      enabled: loaded.enabled,
      providerKind: loaded.providerKind,
      providerPresetId: loaded.providerPresetId,
      model: loaded.model,
      baseUrl: loaded.baseUrl,
      supportsVision: loaded.supportsVision,
      visionModel: loaded.visionModel,
      contextWindowTokens: loaded.contextWindowTokens,
      maxInputTokens: loaded.maxInputTokens,
      maxOutputTokens: loaded.maxOutputTokens,
      historyTokenBudget: loaded.historyTokenBudget,
      historyTurns: loaded.historyTurns,
      temperature: loaded.temperature,
      timeoutMs: loaded.timeoutMs,
    })
    hasStoredApiKey.value = loaded.hasKey
    storedCredentialBinding.value = credentialBinding.value
    apiKey.value = ''
    savedAiFingerprint.value = aiFingerprint()
  } catch (error) {
    message.error(`无法读取 AI 设置：${errorMessage(error)}`)
  } finally {
    loadingAi.value = false
  }
}

function selectProvider(presetId: string) {
  const preset = enabledProviderPresets.value.find((item) => item.id === presetId)
  if (!preset) return
  ai.providerPresetId = preset.id
  ai.providerKind = preset.providerKind
  if (preset.recommendedBaseUrl) ai.baseUrl = preset.recommendedBaseUrl
  const model = preset.models[0]
  if (model) {
    ai.model = model.id
    ai.contextWindowTokens = model.contextWindowTokens
    ai.maxInputTokens = Math.min(65536, model.contextWindowTokens - Math.min(4096, model.maxOutputTokens))
    ai.maxOutputTokens = Math.min(4096, model.maxOutputTokens)
    ai.supportsVision = model.supportsVision
    ai.visionModel = model.supportsVision ? model.id : undefined
  } else {
    ai.model = ''
    ai.supportsVision = false
    ai.visionModel = undefined
  }
  apiKey.value = ''
}

function selectModel(modelId: string) {
  ai.model = modelId
  const model = selectedPreset.value?.models.find((item) => item.id === modelId)
  if (!model) return
  ai.contextWindowTokens = model.contextWindowTokens
  ai.maxOutputTokens = Math.min(ai.maxOutputTokens, model.maxOutputTokens)
  ai.maxInputTokens = Math.min(ai.maxInputTokens, model.contextWindowTokens - ai.maxOutputTokens)
  if (model.supportsVision) {
    ai.supportsVision = true
    ai.visionModel ||= model.id
  }
}

function connectionErrorLabel(code?: string) {
  const labels: Record<string, string> = {
    ai_runtime_not_configured: 'AI 运行时未配置',
    ai_lab_disabled: '实验室 AI 已关闭',
    ai_user_disabled: '当前用户已关闭 AI',
    ai_provider_not_selected: '尚未选择 Provider',
    ai_api_key_missing: '尚未填写个人 API Key',
    provider_exit_not_approved: 'API 出口未获实验室批准',
    invalid_provider: 'Provider 配置无效',
    api_key_rejected: 'API Key 被 Provider 拒绝',
    model_not_found: '模型不存在或无权访问',
    provider_http_error: 'Provider 返回 HTTP 错误',
    provider_unreachable: 'API 出口无法连接',
    provider_transport_error: 'Provider 请求传输失败',
    request_timeout: '请求超时',
    context_exceeded: '上下文超限',
    response_format_incompatible: '响应格式不兼容',
    output_budget_exhausted: '推理模型输出额度不足',
    response_too_large: 'Provider 响应超过安全上限',
    provider_unavailable: 'Provider 当前不可用',
  }
  return labels[code ?? ''] ?? code ?? '未知 Provider 错误'
}

async function saveWorkspace() {
  if (!gateway.saveWorkspaceSettings) return
  const input = {
    operatorName: workspace.operatorName.trim(),
    labName: workspace.labName.trim(),
  }
  if (!input.operatorName || !input.labName) {
    message.warning('操作者和实验室名称均不能为空')
    return
  }
  savingWorkspace.value = true
  try {
    Object.assign(workspace, await gateway.saveWorkspaceSettings(input))
    message.success('工作空间设置已保存')
  } catch (error) {
    message.error(`保存失败：${errorMessage(error)}`)
  } finally {
    savingWorkspace.value = false
  }
}

async function saveProfile() {
  if (!gateway.updateProfile || savingProfile.value || accountIsEnvironmentRoot.value) return
  const displayName = profileDisplayName.value.trim()
  if (!displayName) {
    message.warning('显示名称不能为空')
    return
  }
  savingProfile.value = true
  try {
    const session = await gateway.updateProfile({ displayName })
    profileDisplayName.value = session.user.displayName
    message.success('显示名称已更新')
  } catch (error) {
    message.error(`保存失败：${errorMessage(error)}`)
  } finally {
    savingProfile.value = false
  }
}

function clearAccountPasswords() {
  currentPassword.value = ''
  newPassword.value = ''
  passwordConfirmation.value = ''
}

async function changeAccountPassword() {
  if (!gateway.changePassword || changingPassword.value || accountIsEnvironmentRoot.value) return
  const validation = passwordPolicyError(newPassword.value)
  if (!currentPassword.value) {
    message.warning('请输入当前密码')
    clearAccountPasswords()
    return
  }
  if (validation) {
    message.warning(validation)
    clearAccountPasswords()
    return
  }
  if (newPassword.value !== passwordConfirmation.value) {
    message.warning('两次输入的新密码不一致')
    clearAccountPasswords()
    return
  }
  if (newPassword.value === currentPassword.value) {
    message.warning('新密码必须与当前密码不同')
    clearAccountPasswords()
    return
  }
  changingPassword.value = true
  try {
    await gateway.changePassword({
      currentPassword: currentPassword.value,
      newPassword: newPassword.value,
    })
    message.success('密码已修改，其他浏览器会话已撤销')
  } catch (error) {
    message.error(`修改失败：${errorMessage(error)}`)
  } finally {
    clearAccountPasswords()
    changingPassword.value = false
  }
}

async function saveAi() {
  if (!gateway.saveAiSettings) return
  const validation = basicAiError.value || budgetError.value
  if (validation) {
    message.warning(validation)
    return
  }
  const input: SaveAiSettingsInput = {
    enabled: ai.enabled,
    providerKind: ai.providerKind,
    providerPresetId: ai.providerPresetId,
    model: ai.model.trim(),
    baseUrl: ai.baseUrl.trim(),
    supportsVision: ai.supportsVision,
    visionModel: ai.supportsVision ? ai.visionModel?.trim() : undefined,
    contextWindowTokens: ai.contextWindowTokens,
    maxInputTokens: ai.maxInputTokens,
    maxOutputTokens: ai.maxOutputTokens,
    historyTokenBudget: ai.historyTokenBudget,
    historyTurns: ai.historyTurns,
    temperature: ai.temperature,
    timeoutMs: ai.timeoutMs,
  }
  if (apiKey.value.trim()) input.apiKey = apiKey.value.trim()
  savingAi.value = true
  try {
    const saved = await gateway.saveAiSettings(input)
    Object.assign(ai, {
      enabled: saved.enabled,
      providerKind: saved.providerKind,
      providerPresetId: saved.providerPresetId,
      model: saved.model,
      baseUrl: saved.baseUrl,
      supportsVision: saved.supportsVision,
      visionModel: saved.visionModel,
      contextWindowTokens: saved.contextWindowTokens,
      maxInputTokens: saved.maxInputTokens,
      maxOutputTokens: saved.maxOutputTokens,
      historyTokenBudget: saved.historyTokenBudget,
      historyTurns: saved.historyTurns,
      temperature: saved.temperature,
      timeoutMs: saved.timeoutMs,
    })
    hasStoredApiKey.value = saved.hasKey
    storedCredentialBinding.value = credentialBinding.value
    apiKey.value = ''
    savedAiFingerprint.value = aiFingerprint()
    message.success(saved.hasKey ? 'AI 设置已保存' : 'AI 已启用，等待配置个人 API')
  } catch (error) {
    message.error(`保存失败：${errorMessage(error)}`)
  } finally {
    savingAi.value = false
  }
}

async function testAiConnection() {
  if (!gateway.testAiSettings) return
  testingAi.value = true
  try {
    const result = await gateway.testAiSettings()
    if (result.ok) message.success(`连接成功（${result.latencyMs} ms）`)
    else message.error(`连接失败：${connectionErrorLabel(result.errorCode)}`)
  } catch (error) {
    const code = error instanceof HttpGatewayError ? error.code : undefined
    message.error(`连接失败：${connectionErrorLabel(code)}${code ? '' : `（${errorMessage(error)}）`}`)
  } finally {
    testingAi.value = false
  }
}

function clearAiKey() {
  if (!gateway.clearAiApiKey || !hasApiKey.value) return
  dialog.warning({
    title: '清除已保存的 API Key？',
    content: '清除后 AI 请求将无法使用该凭据，除非你再次输入并保存。',
    positiveText: '清除密钥',
    negativeText: '取消',
    async onPositiveClick() {
      clearingKey.value = true
      try {
        const saved = await gateway.clearAiApiKey!()
        hasStoredApiKey.value = saved.hasKey
        storedCredentialBinding.value = credentialBinding.value
        apiKey.value = ''
        message.success(gateway.mode === 'local' ? 'API Key 已从 OS keyring 清除' : 'API Key 已从个人加密 secret store 清除')
      } catch (error) {
        message.error(`清除失败：${errorMessage(error)}`)
      } finally {
        clearingKey.value = false
      }
    },
  })
}

async function createSnapshot() {
  if (snapshotting.value) return
  snapshotting.value = true
  try {
    const artifact = await dataGateway.createSnapshot()
    await dataGateway.downloadArtifact(artifact)
    message.success(`校验快照已生成（SHA-256 ${artifact.sha256.slice(0, 12)}…）`)
  } catch (error) {
    message.error(`创建快照失败：${errorMessage(error)}`)
  } finally {
    snapshotting.value = false
  }
}

async function logout() {
  if (!gateway.logout || loggingOut.value) return
  loggingOut.value = true
  try {
    await gateway.logout()
    await router.replace('/login')
  } catch (error) {
    message.error(`退出失败：${errorMessage(error)}`)
  } finally {
    loggingOut.value = false
  }
}

onMounted(() => {
  void loadWorkspace()
  void loadAi()
})
</script>

<template>
  <div class="page settings-page">
    <PageHeader title="设置" description="管理工作空间、账号安全、个人 AI 凭据与数据保护。" />

    <section class="more-links mobile-only">
      <router-link v-if="labRegistryAvailable" to="/breeding" class="surface"><Dna :size="18" /><span>繁育管理</span><ChevronRight :size="16" /></router-link>
      <router-link to="/data" class="surface"><Database :size="18" /><span>数据中心</span><ChevronRight :size="16" /></router-link>
      <router-link to="/library" class="surface"><FolderKanban :size="18" /><span>项目资料库</span><ChevronRight :size="16" /></router-link>
      <router-link to="/operations" class="surface"><ShieldCheck :size="18" /><span>活动记录</span><ChevronRight :size="16" /></router-link>
      <router-link to="/ai/images" class="surface"><Bot :size="18" /><span>私人 AI 图片</span><ChevronRight :size="16" /></router-link>
      <router-link v-if="canManageMembers" to="/members" class="surface"><Users :size="18" /><span>成员管理</span><ChevronRight :size="16" /></router-link>
      <router-link v-if="canManageMembers" to="/admin/ai" class="surface"><Bot :size="18" /><span>AI 管理</span><ChevronRight :size="16" /></router-link>
    </section>

    <section class="settings-layout surface">
      <nav aria-label="设置导航">
        <button v-for="item in menu" :key="item.key" type="button" :class="{ active: active === item.key }" @click="active = item.key"><component :is="item.icon" :size="17" /><span>{{ item.label }}</span></button>
      </nav>

      <div class="settings-content">
        <template v-if="active === 'workspace'">
          <div class="section-heading"><h2>工作空间</h2><p>本地版无需登录，操作者资料用于事件和审计记录。</p></div>
          <n-alert v-if="!canManageWorkspace" type="info" :bordered="false" class="availability-alert">共享 Server 的实验室与个人资料由账号和权限页管理，当前 Web 出口不会伪造本地设置。</n-alert>
          <n-form label-placement="top" class="settings-form" :disabled="loadingWorkspace || !canManageWorkspace">
            <n-form-item label="操作者名称"><n-input v-model:value="workspace.operatorName" maxlength="128" /></n-form-item>
            <n-form-item label="实验室显示名称"><n-input v-model:value="workspace.labName" maxlength="128" /></n-form-item>
          </n-form>
          <div class="mode-info"><span class="status-dot" /><div><strong>{{ gateway.displayName }}</strong><span>{{ gateway.mode === 'local' ? '数据存储在本机 SQLite，完全离线可用。' : '通过 HTTPS 连接共享 MuriArc Server。' }}</span></div></div>
          <n-button type="primary" :disabled="!canManageWorkspace" :loading="savingWorkspace || loadingWorkspace" @click="saveWorkspace"><template #icon><Save :size="16" /></template>保存设置</n-button>
        </template>

        <template v-else-if="active === 'account'">
          <div class="section-heading"><h2>账号与安全</h2><p>管理当前操作者资料与登录凭据。</p></div>
          <n-alert v-if="gateway.mode === 'local'" type="info" :bordered="false" class="availability-alert">
            Desktop 本地空间不建立密码或认证表。进入本地空间只是无密码欢迎步骤，不是安全锁；操作者名称可在“工作空间”中修改。
          </n-alert>
          <n-alert v-else-if="!accountAvailable" type="warning" :bordered="false" class="availability-alert">当前 Server 出口未提供账号安全接口。</n-alert>
          <template v-else-if="accountIsEnvironmentRoot">
            <n-alert type="info" :bordered="false" title="由部署配置管理" class="availability-alert">
              Environment Root 的邮箱、名称与密码来自宿主机 .env。请由部署所有者修改配置并重启 Server；应用内不能修改、停用、降级或重置该账号。
            </n-alert>
            <div class="account-identity">
              <div><span>显示名称</span><strong>{{ accountUser?.displayName }}</strong></div>
              <div><span>登录邮箱</span><strong>{{ accountUser?.email }}</strong></div>
              <div><span>账号级别</span><strong>Environment Root</strong></div>
            </div>
          </template>
          <template v-else-if="gateway.mode === 'remote'">
            <div class="subsection-heading"><h3>个人资料</h3><p>邮箱由有权治理你的实验室管理员维护。</p></div>
            <n-form label-placement="top" class="settings-form">
              <n-form-item label="显示名称"><n-input v-model:value="profileDisplayName" maxlength="200" /></n-form-item>
              <n-form-item label="登录邮箱"><n-input :value="accountUser?.email" disabled /></n-form-item>
            </n-form>
            <n-button type="primary" :loading="savingProfile" @click="saveProfile"><template #icon><Save :size="16" /></template>保存显示名称</n-button>

            <div class="subsection-heading password-heading"><h3>修改密码</h3><p>只要求至少 8 个字符且不含控制字符；强度等级仅为建议。</p></div>
            <n-form label-placement="top" class="settings-form">
              <n-form-item label="当前密码"><n-input v-model:value="currentPassword" type="password" show-password-on="click" :input-props="{ autocomplete: 'current-password' }" /></n-form-item>
              <n-form-item label="新密码"><n-input v-model:value="newPassword" type="password" show-password-on="click" maxlength="1024" :input-props="{ autocomplete: 'new-password' }" /></n-form-item>
              <div class="password-strength full-row"><span>建议强度：{{ accountPasswordStrength.label }}</span><n-progress type="line" :show-indicator="false" :percentage="accountPasswordStrength.percentage" :status="accountPasswordStrength.status" /></div>
              <n-form-item label="确认新密码" class="full-row"><n-input v-model:value="passwordConfirmation" type="password" show-password-on="click" maxlength="1024" :input-props="{ autocomplete: 'new-password' }" @keyup.enter="changeAccountPassword" /></n-form-item>
            </n-form>
            <n-button type="primary" :loading="changingPassword" @click="changeAccountPassword"><template #icon><KeyRound :size="16" /></template>修改密码</n-button>
          </template>
        </template>

        <template v-else-if="active === 'ai'">
          <div class="section-heading"><h2>AI 与模型</h2><p>Provider、出口、模型、Token 参数和加密凭据仅属于当前用户，Root 配置不会共享。</p></div>
          <n-alert v-if="!canManageAi" type="info" :bordered="false" class="availability-alert">当前运行出口未提供 AI 凭据管理；界面不会读取或保存演示密钥。</n-alert>
          <n-alert v-else-if="ai.enabled && !hasApiKey && !apiKey.trim()" type="info" :bordered="false" title="AI 已启用，等待配置个人 API" class="availability-alert">选择 Provider，填写你自己的 API Key 并保存后即可调用；在此之前不会发出任何外部请求或产生费用。</n-alert>
          <n-alert v-if="hasStoredApiKey && !hasApiKey" type="warning" :bordered="false" title="Provider 已变更，旧密钥不会复用" class="availability-alert">为避免把一个服务的凭据发给另一个服务，请输入新 Provider 的 API Key。保存后，原 Provider 的密钥绑定会被删除。</n-alert>

          <div class="toggle-row"><div><strong>启用内置 AI 助手</strong><span>新用户默认启用；关闭只影响当前用户，不删除配置或已有数据。</span></div><n-switch v-model:value="ai.enabled" :disabled="loadingAi || !canManageAi" /></div>
          <n-form label-placement="top" class="settings-form ai-settings-form" :disabled="loadingAi || !canManageAi || !ai.enabled">
            <n-form-item label="Provider">
              <n-select v-model:value="ai.providerPresetId" :options="providerOptions" @update:value="selectProvider" />
            </n-form-item>
            <n-form-item label="模型">
              <n-select v-model:value="ai.model" :options="modelOptions" filterable tag placeholder="选择推荐模型或输入兼容模型 ID" @update:value="selectModel" />
            </n-form-item>
            <n-form-item label="API 出口" class="full-row">
              <n-input v-model:value="ai.baseUrl" maxlength="2048" placeholder="https://api.example.com/v1" />
              <template #feedback>推荐出口可按个人需要覆盖；自定义 URL 仍受实验室审批策略约束。</template>
            </n-form-item>
            <div v-if="selectedPreset" class="provider-meta full-row">
              <div><strong>{{ selectedPreset.displayName }}</strong><span>{{ selectedPreset.builtin ? '内置预设' : '实验室预设' }} · {{ selectedPreset.supportsVision ? '包含视觉模型' : '文本模型' }}</span></div>
              <a v-if="selectedPreset.documentationUrl" :href="selectedPreset.documentationUrl" target="_blank" rel="noreferrer">官方文档<ExternalLink :size="14" /></a>
            </div>
            <n-form-item label="个人 API Key" class="full-row">
              <n-input v-model:value="apiKey" type="password" show-password-on="click" autocomplete="new-password" :placeholder="hasApiKey ? '已配置；留空保持现有密钥' : (gateway.mode === 'local' ? '将安全存入 OS keyring' : '将使用部署级 Master Key 加密后按用户保存')" />
              <template #feedback>读取 API 只返回“已配置/未配置”，绝不回显明文；更新其他参数不会删除同一 Provider 的密钥。</template>
            </n-form-item>

            <n-collapse class="advanced-settings full-row" arrow-placement="right">
              <n-collapse-item name="advanced">
                <template #header><span class="advanced-title"><SlidersHorizontal :size="16" />高级设置</span></template>
                <div class="token-budget-card">
                  <div class="token-budget-heading"><strong>上下文预算</strong><span>已分配 {{ allocatedTokens.toLocaleString() }} / {{ ai.contextWindowTokens.toLocaleString() }} Token</span></div>
                  <n-progress type="line" :percentage="budgetPercent" :show-indicator="false" :status="budgetError ? 'error' : 'success'" />
                  <div class="token-budget-breakdown"><span>输入预算 {{ ai.maxInputTokens.toLocaleString() }}</span><span>输出预留 {{ ai.maxOutputTokens.toLocaleString() }}</span><span>未分配 {{ Math.max(0, unallocatedTokens).toLocaleString() }}</span></div>
                  <p>最大输入包含系统提示、历史、工具结果与当前问题。超限时只裁剪最旧的完整历史轮次，不会静默截断当前问题。</p>
                  <n-alert v-if="budgetError" type="error" :bordered="false">{{ budgetError }}</n-alert>
                </div>
                <div class="advanced-grid">
                  <n-form-item label="上下文窗口 Token">
                    <n-input-number v-model:value="ai.contextWindowTokens" :min="4096" :max="2000000" :step="1024" />
                    <template #feedback>Provider 模型可接收的总上下文，范围 4,096–2,000,000。</template>
                  </n-form-item>
                  <n-form-item label="最大输入 Token">
                    <n-input-number v-model:value="ai.maxInputTokens" :min="1024" :max="1900000" :step="1024" />
                    <template #feedback>调用前允许的估算输入上限，范围 1,024–1,900,000。</template>
                  </n-form-item>
                  <n-form-item label="最大输出 Token">
                    <n-input-number v-model:value="ai.maxOutputTokens" :min="1" :max="131072" :step="256" />
                    <template #feedback>作为 max_tokens 真实发送给 Provider，范围 1–131,072。</template>
                  </n-form-item>
                  <n-form-item label="历史消息预算">
                    <n-input-number v-model:value="ai.historyTokenBudget" :min="0" :max="ai.maxInputTokens" :step="1024" />
                    <template #feedback>最多为历史保留的估算 Token；0 表示不保留历史消息。</template>
                  </n-form-item>
                  <n-form-item label="历史保留轮数">
                    <n-input-number v-model:value="ai.historyTurns" :min="0" :max="100" />
                    <template #feedback>优先保留最近 0–100 个完整 user/assistant 轮次。</template>
                  </n-form-item>
                  <n-form-item label="请求超时（毫秒）">
                    <n-input-number v-model:value="ai.timeoutMs" :min="100" :max="600000" :step="1000" />
                    <template #feedback>等待 Provider 响应的最长时间，范围 100–600,000 ms。</template>
                  </n-form-item>
                  <n-form-item label="Temperature" class="full-row">
                    <div class="temperature-control"><n-slider v-model:value="ai.temperature" :min="0" :max="2" :step="0.1" /><n-input-number v-model:value="ai.temperature" :min="0" :max="2" :step="0.1" /></div>
                    <template #feedback>0 更稳定，2 更发散；部分推理模型可能忽略此参数。</template>
                  </n-form-item>
                  <n-form-item label="视觉能力" class="full-row"><n-switch v-model:value="ai.supportsVision">允许当前配置处理图片</n-switch></n-form-item>
                  <n-form-item v-if="ai.supportsVision" label="视觉模型" class="full-row">
                    <n-input v-model:value="ai.visionModel" maxlength="256" placeholder="输入支持视觉的模型 ID" />
                    <template #feedback>{{ selectedModel?.supportsVision ? '当前推荐模型支持视觉，可使用同一模型。' : '请确认该模型在当前 Provider 中支持图片输入。' }}</template>
                  </n-form-item>
                </div>
              </n-collapse-item>
            </n-collapse>
          </n-form>

          <div class="secret-note"><KeyRound :size="17" /><span>{{ hasApiKey
            ? (gateway.mode === 'local' ? 'API Key 已配置并保存在 OS keyring。' : 'API Key 已配置并按当前用户加密保存。') + '界面、日志、错误响应和读取接口均不会回显密钥。'
            : 'API Key 未配置。个人设置仍会保存，但任何 AI 操作都会明确提示等待个人 API，不会请求外部服务。' }}</span></div>
          <div v-if="basicAiError" class="validation-message">{{ basicAiError }}</div>
          <div class="button-row">
            <n-button type="primary" :disabled="!canSaveAi" :loading="savingAi || loadingAi" @click="saveAi"><template #icon><Save :size="16" /></template>保存 AI 设置</n-button>
            <n-button v-if="gateway.testAiSettings" secondary :disabled="!canManageAi || !ai.enabled || !hasApiKey || isAiDirty" :loading="testingAi" @click="testAiConnection">测试已保存配置</n-button>
            <n-button v-if="hasApiKey" type="error" secondary :disabled="!canManageAi" :loading="clearingKey" @click="clearAiKey">清除 API Key</n-button>
          </div>
          <p v-if="gateway.testAiSettings && isAiDirty" class="test-hint">请先保存当前设置，再测试连接；连接测试只使用已保存的个人配置。</p>
        </template>

        <template v-else-if="active === 'backup'">
          <div class="section-heading"><h2>备份与迁移</h2><p>快照包含版本化 manifest、完整业务 JSONL、附件和 SHA-256 校验；当前版本不支持 restore/apply。</p></div>
          <div class="action-card"><Archive :size="22" /><div><strong>创建完整业务归档快照</strong><span>用于完整性校验与离线留存；当前不能导入、恢复或直接迁移到 Server，正式恢复仍需数据库与附件联合备份。</span></div><n-button v-if="labRegistryAvailable" type="primary" secondary :loading="snapshotting" @click="createSnapshot">创建快照</n-button></div>
          <div class="action-card"><Database :size="22" /><div><strong>旧版数据库迁移</strong><span>为避免误改原库，V1 仅通过 muriarc-legacy-migrator CLI 对副本执行审计与单向迁移；完整步骤见 docs/MIGRATION.md。</span></div><n-tag :bordered="false">CLI · 只读源库</n-tag></div>
        </template>

        <template v-else-if="active === 'security'">
          <div class="section-heading"><h2>安全与审计</h2><p>正式记录默认软删除，写入包含操作者、来源、revision 与时间。</p></div>
          <n-alert v-if="gateway.mode === 'local'" type="info" :bordered="false" title="本地个人模式">本地版使用操作者资料而非完整账号体系；共享 Server 会额外实施角色与项目权限。</n-alert>
          <n-alert v-else type="info" :bordered="false" title="共享实验室模式">Server 使用实时角色、项目权限、HttpOnly Session、CSRF 与可撤销 external token。</n-alert>
          <div class="policy-list"><div><ShieldCheck :size="18" /><span><strong>AI 写入保护</strong>普通写入先显示 diff，科研测量先保存为草稿。</span></div><div><ShieldCheck :size="18" /><span><strong>冲突阻断</strong>UUID 相同但内容不同不会被静默覆盖。</span></div><div><ShieldCheck :size="18" /><span><strong>来源留痕</strong>人工、导入和 AI 生成的数据均记录 provenance。</span></div></div>
          <div v-if="gateway.logout" class="session-actions"><div><strong>共享 Server 会话</strong><span>退出会撤销当前 HttpOnly Cookie 会话。</span></div><n-button secondary :loading="loggingOut" @click="logout">退出登录</n-button></div>
        </template>

        <template v-else>
          <div class="about-panel"><img :src="branding.logoMarkPath" :alt="`${branding.productName} Logo`" /><h2>{{ branding.productName }}</h2><p>{{ branding.tagline }}</p><n-tag :bordered="false">Version {{ branding.version }} · {{ branding.releaseStage }}</n-tag><small>{{ branding.sourceNotice }}<br />依据 Apache License 2.0 发布；完整归属见 LICENSE、NOTICE 与关于页。</small></div>
        </template>
      </div>
    </section>
  </div>
</template>

<style scoped>
.more-links { display: none; flex-direction: column; gap: 8px; margin-bottom: 12px; }.more-links a { display: grid; grid-template-columns: 24px 1fr auto; align-items: center; min-height: 48px; padding: 0 13px; }.more-links svg:first-child { color: var(--muri-primary); }
.settings-layout { display: grid; min-height: 560px; grid-template-columns: 210px minmax(0, 1fr); overflow: hidden; }
.settings-layout > nav { padding: 12px; border-right: 1px solid var(--muri-border); background: var(--muri-surface-muted); }.settings-layout nav button { display: flex; width: 100%; min-height: 40px; align-items: center; gap: 9px; padding: 0 10px; text-align: left; border: 0; border-radius: 7px; color: var(--muri-text-secondary); background: transparent; cursor: pointer; }.settings-layout nav button.active { color: var(--muri-primary); background: white; box-shadow: 0 1px 5px rgba(30,53,76,.08); }
.settings-content { max-width: 720px; padding: 24px 28px; }.section-heading { margin-bottom: 20px; }.section-heading h2 { margin: 0 0 4px; font-size: 19px; }.section-heading p { margin: 0; color: var(--muri-text-secondary); }
.settings-form { display: grid; grid-template-columns: 1fr 1fr; gap: 0 15px; }.settings-form :deep(.full-row) { grid-column: 1 / -1; }
.mode-info { display: flex; align-items: center; gap: 10px; margin: 2px 0 18px; padding: 11px; border: 1px solid var(--muri-border); border-radius: 7px; background: var(--muri-surface-muted); }.status-dot { width: 8px; height: 8px; border-radius: 50%; background: var(--muri-success); box-shadow: 0 0 0 3px rgba(56,134,107,.14); }.mode-info div { display: flex; flex-direction: column; }.mode-info span { color: var(--muri-text-secondary); font-size: 11px; }
.subsection-heading { margin: 6px 0 14px; }.subsection-heading h3 { margin: 0 0 3px; font-size: 15px; }.subsection-heading p { margin: 0; color: var(--muri-text-secondary); font-size: 11px; }.password-heading { margin-top: 28px; padding-top: 22px; border-top: 1px solid var(--muri-border); }
.account-identity { display: grid; gap: 8px; }.account-identity > div { display: grid; grid-template-columns: 120px minmax(0, 1fr); gap: 12px; padding: 12px; border: 1px solid var(--muri-border); border-radius: 7px; }.account-identity span { color: var(--muri-text-tertiary); font-size: 11px; }.account-identity strong { overflow-wrap: anywhere; }
.password-strength { display: grid; grid-template-columns: auto minmax(120px, 1fr); align-items: center; gap: 12px; margin: -10px 0 12px; color: var(--muri-text-tertiary); font-size: 11px; }.password-strength .n-progress { width: 100%; }
.toggle-row { display: flex; align-items: center; justify-content: space-between; margin-bottom: 18px; padding: 13px; border: 1px solid var(--muri-border); border-radius: 7px; }.toggle-row > div { display: flex; flex-direction: column; }.toggle-row span { color: var(--muri-text-secondary); font-size: 11px; }
.provider-meta { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin: -4px 0 16px; padding: 10px 12px; border: 1px solid var(--muri-border); border-radius: 7px; background: var(--muri-surface-muted); }.provider-meta > div { display: flex; flex-direction: column; }.provider-meta span { color: var(--muri-text-tertiary); font-size: 11px; }.provider-meta a { display: inline-flex; align-items: center; gap: 5px; color: var(--muri-primary); text-decoration: none; white-space: nowrap; }
.advanced-settings { margin: 2px 0 16px; border-top: 1px solid var(--muri-border); border-bottom: 1px solid var(--muri-border); }.advanced-title { display: inline-flex; align-items: center; gap: 7px; font-weight: 650; }.token-budget-card { margin: 4px 0 16px; padding: 13px; border: 1px solid var(--muri-border); border-radius: 7px; background: var(--muri-surface-muted); }.token-budget-heading, .token-budget-breakdown { display: flex; justify-content: space-between; gap: 12px; }.token-budget-heading { margin-bottom: 8px; }.token-budget-heading span, .token-budget-breakdown, .token-budget-card p { color: var(--muri-text-secondary); font-size: 11px; }.token-budget-breakdown { flex-wrap: wrap; margin-top: 7px; }.token-budget-card p { margin: 9px 0 0; line-height: 1.6; }.token-budget-card .n-alert { margin-top: 10px; }.advanced-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 0 15px; }.advanced-grid :deep(.n-input-number) { width: 100%; }.temperature-control { display: grid; width: 100%; grid-template-columns: minmax(0, 1fr) 110px; align-items: center; gap: 18px; }
.secret-note { display: flex; align-items: center; gap: 8px; margin: 0 0 18px; padding: 10px; color: var(--muri-text-secondary); background: var(--muri-primary-soft); font-size: 11px; }.secret-note svg { flex: 0 0 auto; color: var(--muri-primary); }.validation-message { margin: -8px 0 14px; color: var(--muri-danger, #c2413b); font-size: 12px; }.test-hint { margin: 9px 0 0; color: var(--muri-text-tertiary); font-size: 11px; }
.availability-alert { margin-bottom: 16px; }.button-row { display: flex; flex-wrap: wrap; gap: 9px; }
.session-actions { display: flex; align-items: center; justify-content: space-between; gap: 16px; margin-top: 16px; padding: 13px; border: 1px solid var(--muri-border); border-radius: 7px; }.session-actions > div { display: flex; flex-direction: column; }.session-actions span { color: var(--muri-text-secondary); font-size: 11px; }
.action-card { display: grid; grid-template-columns: 32px 1fr auto; align-items: center; gap: 10px; margin-bottom: 10px; padding: 13px; border: 1px solid var(--muri-border); border-radius: 7px; }.action-card > svg { color: var(--muri-primary); }.action-card div { display: flex; flex-direction: column; }.action-card span { color: var(--muri-text-secondary); font-size: 11px; }
.policy-list { display: flex; flex-direction: column; gap: 8px; margin-top: 14px; }.policy-list > div { display: flex; align-items: flex-start; gap: 9px; padding: 12px; border: 1px solid var(--muri-border); border-radius: 7px; color: var(--muri-text-secondary); }.policy-list svg { flex: 0 0 auto; color: var(--muri-success); }.policy-list strong { display: block; color: var(--muri-text); }
.about-panel { display: flex; min-height: 470px; align-items: center; justify-content: center; flex-direction: column; text-align: center; }.about-panel img { width: 122px; height: 122px; object-fit: contain; }.about-panel h2 { margin: 12px 0 2px; font-size: 27px; }.about-panel p { margin: 0 0 13px; color: var(--muri-text-secondary); }.about-panel small { max-width: 460px; margin-top: 20px; color: var(--muri-text-tertiary); line-height: 1.6; }
@media (max-width: 900px) { .advanced-grid { grid-template-columns: 1fr; }.advanced-grid :deep(.n-form-item) { grid-column: 1 !important; }.provider-meta { align-items: flex-start; flex-direction: column; }.temperature-control { grid-template-columns: minmax(0, 1fr) 92px; }.settings-layout { grid-template-columns: 1fr; }.settings-layout > nav { display: flex; overflow-x: auto; border-right: 0; border-bottom: 1px solid var(--muri-border); }.settings-layout nav button { width: auto; flex: 0 0 auto; }.settings-content { padding: 19px 15px; }.settings-form { grid-template-columns: 1fr; }.settings-form :deep(.n-form-item) { grid-column: 1 !important; }.action-card { grid-template-columns: 28px 1fr; }.action-card button { grid-column: 2; justify-self: start; } }
</style>
