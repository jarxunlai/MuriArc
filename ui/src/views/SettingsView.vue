<script setup lang="ts">
import { computed, onMounted, reactive, ref } from 'vue'
import { useDialog, useMessage } from 'naive-ui'
import { useRouter } from 'vue-router'
import { Archive, Bot, ChevronRight, Database, Dna, FolderKanban, KeyRound, Save, ShieldCheck, Users } from '@lucide/vue'
import { branding } from '@/branding'
import { currentAuthSession, gateway } from '@/services/gateway'
import type { AiProviderKind, SaveAiSettingsInput, WorkspaceSettings } from '@/domain/models'
import { createDataGateway } from '@/services/dataGateway'
import { passwordPolicyError, passwordStrength } from '@/services/passwordStrength'
import { hasLabRegistryAccess } from '@/services/projectContext'
import PageHeader from '@/components/PageHeader.vue'

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
  enabled: false,
  providerKind: 'open_ai_compatible',
  model: 'gpt-4.1-mini',
  baseUrl: 'https://api.openai.com/v1',
  supportsVision: false,
  visionModel: undefined,
})
const apiKey = ref('')
const hasApiKey = ref(false)
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
const providerOptions: Array<{ label: string; value: AiProviderKind }> = [
  { label: 'OpenAI-compatible', value: 'open_ai_compatible' },
  { label: '本地 HTTP 模型', value: 'local_http' },
]
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
    const loaded = await gateway.getAiSettings()
    Object.assign(ai, {
      enabled: loaded.enabled,
      providerKind: loaded.providerKind,
      model: loaded.model,
      baseUrl: loaded.baseUrl,
      supportsVision: loaded.supportsVision,
      visionModel: loaded.visionModel,
    })
    hasApiKey.value = loaded.hasKey
    apiKey.value = ''
  } catch (error) {
    message.error(`无法读取 AI 设置：${errorMessage(error)}`)
  } finally {
    loadingAi.value = false
  }
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
  const input: SaveAiSettingsInput = {
    enabled: ai.enabled,
    providerKind: ai.providerKind,
    model: ai.model.trim(),
    baseUrl: ai.baseUrl.trim(),
    supportsVision: ai.supportsVision,
    visionModel: ai.supportsVision ? ai.visionModel?.trim() : undefined,
  }
  if (apiKey.value.trim()) input.apiKey = apiKey.value.trim()
  savingAi.value = true
  try {
    const saved = await gateway.saveAiSettings(input)
    Object.assign(ai, {
      enabled: saved.enabled,
      providerKind: saved.providerKind,
      model: saved.model,
      baseUrl: saved.baseUrl,
      supportsVision: saved.supportsVision,
      visionModel: saved.visionModel,
    })
    hasApiKey.value = saved.hasKey
    apiKey.value = ''
    message.success('AI 设置已保存')
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
    else message.error(`连接失败：${result.errorCode ?? 'provider_error'}`)
  } catch (error) {
    message.error(`连接失败：${errorMessage(error)}`)
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
        hasApiKey.value = saved.hasKey
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
      <router-link to="/operations" class="surface"><ShieldCheck :size="18" /><span>操作与审计</span><ChevronRight :size="16" /></router-link>
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
          <div class="section-heading"><h2>AI 与模型</h2><p>凭据属于当前用户，不进入项目数据库、审计日志或快照。</p></div>
          <n-alert v-if="!canManageAi" type="info" :bordered="false" class="availability-alert">当前运行出口未提供 AI 凭据管理；界面不会读取或保存演示密钥。</n-alert>
          <div class="toggle-row"><div><strong>启用内置 AI 助手</strong><span>可随时关闭；关闭后已有数据不受影响。</span></div><n-switch v-model:value="ai.enabled" :disabled="loadingAi || !canManageAi" /></div>
          <n-form label-placement="top" class="settings-form" :disabled="loadingAi || !canManageAi || !ai.enabled">
            <n-form-item label="Provider"><n-select v-model:value="ai.providerKind" :options="providerOptions" /></n-form-item>
            <n-form-item label="模型"><n-input v-model:value="ai.model" maxlength="256" placeholder="例如 gpt-4.1-mini" /></n-form-item>
            <n-form-item label="API URL" class="full-row"><n-input v-model:value="ai.baseUrl" maxlength="2048" placeholder="https://api.example.com/v1" /></n-form-item>
            <n-form-item label="视觉能力" class="full-row"><n-switch v-model:value="ai.supportsVision">支持图片理解</n-switch></n-form-item>
            <n-form-item v-if="ai.supportsVision" label="视觉模型" class="full-row"><n-input v-model:value="ai.visionModel" maxlength="256" placeholder="例如 gpt-4.1-mini" /></n-form-item>
            <n-form-item label="API Key" class="full-row">
              <n-input v-model:value="apiKey" type="password" show-password-on="click" autocomplete="new-password" :placeholder="hasApiKey ? '已安全保存；留空可保留现有密钥' : (gateway.mode === 'local' ? '将安全存入 OS keyring' : '将加密存入个人 secret store')" />
            </n-form-item>
          </n-form>
          <div class="secret-note"><KeyRound :size="17" /><span>{{ hasApiKey
            ? (gateway.mode === 'local' ? '已在 OS keyring 中保存凭据。' : '已在当前用户的加密 secret store 中保存凭据。') + '界面和读取接口永不回显密钥。'
            : '尚未保存 API Key。凭据不会进入业务数据库响应、审计正文或快照。' }}</span></div>
          <div class="button-row">
            <n-button type="primary" :disabled="!canManageAi" :loading="savingAi || loadingAi" @click="saveAi">保存 AI 设置</n-button>
            <n-button v-if="gateway.testAiSettings" secondary :disabled="!canManageAi" :loading="testingAi" @click="testAiConnection">测试连接</n-button>
            <n-button v-if="hasApiKey" type="error" secondary :disabled="!canManageAi" :loading="clearingKey" @click="clearAiKey">清除 API Key</n-button>
          </div>
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
.toggle-row { display: flex; align-items: center; justify-content: space-between; margin-bottom: 18px; padding: 13px; border: 1px solid var(--muri-border); border-radius: 7px; }.toggle-row > div { display: flex; flex-direction: column; }.toggle-row span { color: var(--muri-text-secondary); font-size: 11px; }.secret-note { display: flex; align-items: center; gap: 8px; margin: 0 0 18px; padding: 10px; color: var(--muri-text-secondary); background: var(--muri-primary-soft); font-size: 11px; }.secret-note svg { color: var(--muri-primary); }
.availability-alert { margin-bottom: 16px; }.button-row { display: flex; flex-wrap: wrap; gap: 9px; }
.session-actions { display: flex; align-items: center; justify-content: space-between; gap: 16px; margin-top: 16px; padding: 13px; border: 1px solid var(--muri-border); border-radius: 7px; }.session-actions > div { display: flex; flex-direction: column; }.session-actions span { color: var(--muri-text-secondary); font-size: 11px; }
.action-card { display: grid; grid-template-columns: 32px 1fr auto; align-items: center; gap: 10px; margin-bottom: 10px; padding: 13px; border: 1px solid var(--muri-border); border-radius: 7px; }.action-card > svg { color: var(--muri-primary); }.action-card div { display: flex; flex-direction: column; }.action-card span { color: var(--muri-text-secondary); font-size: 11px; }
.policy-list { display: flex; flex-direction: column; gap: 8px; margin-top: 14px; }.policy-list > div { display: flex; align-items: flex-start; gap: 9px; padding: 12px; border: 1px solid var(--muri-border); border-radius: 7px; color: var(--muri-text-secondary); }.policy-list svg { flex: 0 0 auto; color: var(--muri-success); }.policy-list strong { display: block; color: var(--muri-text); }
.about-panel { display: flex; min-height: 470px; align-items: center; justify-content: center; flex-direction: column; text-align: center; }.about-panel img { width: 122px; height: 122px; object-fit: contain; }.about-panel h2 { margin: 12px 0 2px; font-size: 27px; }.about-panel p { margin: 0 0 13px; color: var(--muri-text-secondary); }.about-panel small { max-width: 460px; margin-top: 20px; color: var(--muri-text-tertiary); line-height: 1.6; }
@media (max-width: 900px) { .settings-layout { grid-template-columns: 1fr; }.settings-layout > nav { display: flex; overflow-x: auto; border-right: 0; border-bottom: 1px solid var(--muri-border); }.settings-layout nav button { width: auto; flex: 0 0 auto; }.settings-content { padding: 19px 15px; }.settings-form { grid-template-columns: 1fr; }.settings-form :deep(.n-form-item) { grid-column: 1 !important; }.action-card { grid-template-columns: 28px 1fr; }.action-card button { grid-column: 2; justify-self: start; } }
</style>
