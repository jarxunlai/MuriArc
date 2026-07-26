<script setup lang="ts">
import { computed, onMounted, reactive, ref } from 'vue'
import { useMessage } from 'naive-ui'
import { useRouter } from 'vue-router'
import {
  Archive,
  Bot,
  ChevronRight,
  Database,
  Download,
  Dna,
  FolderKanban,
  FolderOpen,
  HardDrive,
  KeyRound,
  RotateCcw,
  RefreshCw,
  Save,
  ShieldCheck,
  Users,
} from '@lucide/vue'
import { branding } from '@/branding'
import { currentAuthSession, currentRuntimeCapabilities, gateway } from '@/services/gateway'
import type {
  DesktopUpdateStatus,
  LocalStorageSelection,
  LocalStorageStatus,
} from '@/services/gateway'
import type { WorkspaceSettings } from '@/domain/models'
import { createDataGateway } from '@/services/dataGateway'
import { passwordPolicyError, passwordStrength } from '@/services/passwordStrength'
import { hasLabRegistryAccess } from '@/services/projectContext'
import AiModelProfilesSettings from '@/components/AiModelProfilesSettings.vue'
import PageHeader from '@/components/PageHeader.vue'

const message = useMessage()
const dataGateway = createDataGateway(gateway)
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
const loadingWorkspace = ref(false)
const savingWorkspace = ref(false)
const loggingOut = ref(false)
const snapshotting = ref(false)
const storageStatus = ref<LocalStorageStatus>()
const storageSelection = ref<LocalStorageSelection>()
const loadingStorage = ref(false)
const choosingStorage = ref(false)
const schedulingStorage = ref(false)
const restoringStorage = ref(false)
const openingStorage = ref(false)
const desktopUpdate = ref<DesktopUpdateStatus>()
const checkingDesktopUpdate = ref(false)
const applyingDesktopUpdate = ref(false)
const confirmedDesktopRecovery = ref(false)
const accountUser = computed(() => currentAuthSession.value?.user)
const accountIsEnvironmentRoot = computed(() => accountUser.value?.isEnvironmentRoot === true)
const accountAvailable = gateway.mode === 'remote'
  && typeof gateway.updateProfile === 'function'
  && typeof gateway.changePassword === 'function'
const passwordMinChars = computed(() => currentRuntimeCapabilities.value.passwordMinChars)
const accountPasswordStrength = computed(() => passwordStrength(newPassword.value, passwordMinChars.value))
const canManageWorkspace = typeof gateway.getWorkspaceSettings === 'function'
  && typeof gateway.saveWorkspaceSettings === 'function'
const localStorageAvailable = gateway.mode === 'local'
  && typeof gateway.getLocalStorageStatus === 'function'
  && typeof gateway.chooseLocalStorageDirectory === 'function'
  && typeof gateway.requestLocalStorageMigration === 'function'
  && typeof gateway.requestRestoreDefaultStorage === 'function'
  && typeof gateway.openLocalStorageDirectory === 'function'
const desktopUpdaterAvailable = gateway.mode === 'local'
  && typeof gateway.checkDesktopUpdate === 'function'
  && typeof gateway.applyDesktopUpdate === 'function'
const canManageMembers = gateway.mode === 'remote'
  && currentAuthSession.value?.user.labRoles.includes('lab_admin') === true
const labRegistryAvailable = gateway.mode === 'local' || hasLabRegistryAccess()
const menu = computed(() => [
  { key: 'workspace', label: '工作空间', icon: Database },
  { key: 'account', label: '账号与安全', icon: KeyRound },
  { key: 'ai', label: 'AI 与模型', icon: Bot },
  ...(labRegistryAvailable ? [{ key: 'backup', label: '备份与迁移', icon: Archive }] : []),
  ...(desktopUpdaterAvailable ? [{ key: 'update', label: '软件更新', icon: Download }] : []),
  { key: 'security', label: '安全与审计', icon: ShieldCheck },
  { key: 'about', label: '关于 MuriArc', icon: FolderKanban },
])

const migrationClassLabels: Record<'m0' | 'm1' | 'm2' | 'm3', string> = {
  m0: 'M0 · 界面/无数据结构变化',
  m1: 'M1 · 短时冻结写入',
  m2: 'M2 · 维护期间保持只读',
  m3: 'M3 · 离线结构迁移',
}

function formatBytes(value?: number): string {
  if (value === undefined || !Number.isFinite(value)) return '—'
  const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB']
  let current = Math.max(0, value)
  let unit = 0
  while (current >= 1024 && unit < units.length - 1) {
    current /= 1024
    unit += 1
  }
  return `${current.toFixed(unit === 0 ? 0 : 1)} ${units[unit]}`
}

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
  const validation = passwordPolicyError(newPassword.value, passwordMinChars.value)
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

async function loadLocalStorageStatus() {
  if (!localStorageAvailable || !gateway.getLocalStorageStatus) return
  loadingStorage.value = true
  try {
    storageStatus.value = await gateway.getLocalStorageStatus()
  } catch (error) {
    message.error(`无法读取本地数据位置：${errorMessage(error)}`)
  } finally {
    loadingStorage.value = false
  }
}

async function chooseLocalStorageDirectory() {
  if (!gateway.chooseLocalStorageDirectory || choosingStorage.value) return
  choosingStorage.value = true
  try {
    const selection = await gateway.chooseLocalStorageDirectory()
    if (selection) storageSelection.value = selection
  } catch (error) {
    message.error(`选择目录失败：${errorMessage(error)}`)
  } finally {
    choosingStorage.value = false
  }
}

async function scheduleLocalStorageMigration() {
  if (
    !gateway.requestLocalStorageMigration
    || !storageSelection.value
    || schedulingStorage.value
  ) return
  schedulingStorage.value = true
  try {
    const result = await gateway.requestLocalStorageMigration(
      storageSelection.value.selectionToken,
    )
    if (result.scheduled) {
      storageSelection.value = undefined
      message.success('迁移已安排；请完全退出 MuriArc 后重新启动')
      await loadLocalStorageStatus()
    }
  } catch (error) {
    message.error(`安排迁移失败：${errorMessage(error)}`)
  } finally {
    schedulingStorage.value = false
  }
}

async function restoreDefaultStorage() {
  if (!gateway.requestRestoreDefaultStorage || restoringStorage.value) return
  restoringStorage.value = true
  try {
    const result = await gateway.requestRestoreDefaultStorage()
    if (result.scheduled) {
      storageSelection.value = undefined
      message.success('恢复默认目录已安排；请完全退出 MuriArc 后重新启动')
      await loadLocalStorageStatus()
    }
  } catch (error) {
    message.error(`安排恢复失败：${errorMessage(error)}`)
  } finally {
    restoringStorage.value = false
  }
}

async function openLocalStorageDirectory() {
  if (!gateway.openLocalStorageDirectory || openingStorage.value) return
  openingStorage.value = true
  try {
    await gateway.openLocalStorageDirectory()
  } catch (error) {
    message.error(`打开目录失败：${errorMessage(error)}`)
  } finally {
    openingStorage.value = false
  }
}

async function checkDesktopUpdate() {
  if (!gateway.checkDesktopUpdate || checkingDesktopUpdate.value) return
  checkingDesktopUpdate.value = true
  confirmedDesktopRecovery.value = false
  try {
    desktopUpdate.value = await gateway.checkDesktopUpdate()
    if (!desktopUpdate.value.available) message.success('当前已经是最新版本')
  } catch (error) {
    message.error(`检查更新失败：${errorMessage(error)}`)
  } finally {
    checkingDesktopUpdate.value = false
  }
}

async function applyDesktopUpdate() {
  const update = desktopUpdate.value
  if (
    !gateway.applyDesktopUpdate
    || !update?.available
    || !update.targetVersion
    || !update.migrationClass
    || applyingDesktopUpdate.value
  ) return
  if (!confirmedDesktopRecovery.value) {
    message.warning('请先确认恢复验证、维护窗口和首次写入边界')
    return
  }
  if (update.space && !update.space.sufficient) {
    message.error('空间预检未通过，不能下载或安装更新')
    return
  }
  applyingDesktopUpdate.value = true
  try {
    message.info('正在下载并验证签名更新包；请勿强制结束程序')
    await gateway.applyDesktopUpdate({
      version: update.targetVersion,
      maintenanceClass: update.migrationClass,
      confirmVerifiedRecovery: true,
    })
    message.success('安全安装器已启动，MuriArc 将自动退出并重新打开')
  } catch (error) {
    message.error(`安装更新失败：${errorMessage(error)}`)
  } finally {
    applyingDesktopUpdate.value = false
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
  void loadLocalStorageStatus()
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

            <div class="subsection-heading password-heading"><h3>修改密码</h3><p>只要求至少 {{ passwordMinChars }} 个字符且不含控制字符；强度等级仅为建议。</p></div>
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
          <AiModelProfilesSettings />
        </template>

        <template v-else-if="active === 'backup'">
          <div class="section-heading"><h2>备份与迁移</h2><p>快照包含版本化 manifest、完整业务 JSONL、附件和 SHA-256 校验；当前版本不支持 restore/apply。</p></div>
          <section v-if="localStorageAvailable" class="storage-card" aria-label="Desktop 本地数据位置">
            <div class="storage-card-heading">
              <HardDrive :size="22" />
              <div>
                <strong>Desktop 本地数据位置</strong>
                <span>数据库、附件、数据产物与非敏感 AI 配置使用同一个数据根目录。</span>
              </div>
              <n-tag :bordered="false" :type="storageStatus?.usesCustomRoot ? 'success' : 'default'">
                {{ storageStatus?.usesCustomRoot ? '自定义目录' : '默认目录' }}
              </n-tag>
            </div>

            <n-skeleton v-if="loadingStorage && !storageStatus" text :repeat="2" />
            <template v-else-if="storageStatus">
              <dl class="storage-paths">
                <div><dt>当前数据目录</dt><dd>{{ storageStatus.activeDataRoot }}</dd></div>
                <div><dt>系统默认目录</dt><dd>{{ storageStatus.defaultDataRoot }}</dd></div>
                <div v-if="storageStatus.pendingTargetRoot">
                  <dt>等待迁移到</dt><dd>{{ storageStatus.pendingTargetRoot }}</dd>
                </div>
              </dl>

              <n-alert
                v-if="storageStatus.migrationPending"
                type="warning"
                :bordered="false"
                title="迁移将在下次启动前执行"
              >
                请先保存工作并完全退出 MuriArc，再重新启动。校验成功前不会切换数据目录，也不会创建新的空数据库。
              </n-alert>
              <n-alert v-else type="info" :bordered="false">
                请选择本机固定磁盘上的独立空目录。迁移成功后旧目录仍会保留；API Key 继续由操作系统安全存储管理，WebView 缓存不会迁移。
              </n-alert>

              <div v-if="storageSelection" class="storage-selection">
                <span>已选择的新位置</span>
                <strong>{{ storageSelection.targetDataRoot }}</strong>
                <small>确认后只会写入迁移安排；实际复制和校验要等完全退出并重新启动。</small>
              </div>

              <div class="storage-actions">
                <n-button
                  secondary
                  :disabled="storageStatus.migrationPending"
                  :loading="choosingStorage"
                  @click="chooseLocalStorageDirectory"
                >
                  <template #icon><FolderOpen :size="16" /></template>
                  选择新位置
                </n-button>
                <n-button
                  v-if="storageSelection"
                  type="primary"
                  :disabled="storageStatus.migrationPending"
                  :loading="schedulingStorage"
                  @click="scheduleLocalStorageMigration"
                >
                  确认迁移
                </n-button>
                <n-button :loading="openingStorage" @click="openLocalStorageDirectory">
                  打开数据目录
                </n-button>
                <n-button
                  v-if="storageStatus.usesCustomRoot"
                  quaternary
                  :disabled="storageStatus.migrationPending"
                  :loading="restoringStorage"
                  @click="restoreDefaultStorage"
                >
                  <template #icon><RotateCcw :size="16" /></template>
                  恢复默认位置
                </n-button>
              </div>
            </template>
          </section>
          <div class="action-card"><Archive :size="22" /><div><strong>创建完整业务归档快照</strong><span>用于完整性校验与离线留存；当前不能导入、恢复或直接迁移到 Server，正式恢复仍需数据库与附件联合备份。</span></div><n-button v-if="labRegistryAvailable" type="primary" secondary :loading="snapshotting" @click="createSnapshot">创建快照</n-button></div>
          <div class="action-card"><Database :size="22" /><div><strong>旧版数据库迁移</strong><span>为避免误改原库，V1 仅通过 muriarc-legacy-migrator CLI 对副本执行审计与单向迁移；完整步骤见 docs/MIGRATION.md。</span></div><n-tag :bordered="false">CLI · 只读源库</n-tag></div>
        </template>

        <template v-else-if="active === 'update'">
          <div class="section-heading"><h2>软件更新</h2><p>只安装经签名验证的正式更新；数据 Candidate、恢复副本和旧程序恢复副本全部通过后才会切换。</p></div>
          <div class="update-actions">
            <n-button type="primary" secondary :loading="checkingDesktopUpdate" @click="checkDesktopUpdate">
              <template #icon><RefreshCw :size="16" /></template>
              检查更新
            </n-button>
            <span>当前版本 {{ desktopUpdate?.currentVersion ?? branding.version }}</span>
          </div>

          <n-alert v-if="desktopUpdate && !desktopUpdate.available" type="success" :bordered="false" title="已是最新版本">
            当前安装包无需升级，数据和程序均未发生变化。
          </n-alert>

          <section v-else-if="desktopUpdate?.available" class="update-card" aria-label="Desktop 安全更新确认">
            <div class="storage-card-heading">
              <Download :size="22" />
              <div>
                <strong>{{ desktopUpdate.currentVersion }} → {{ desktopUpdate.targetVersion }}</strong>
                <span>{{ desktopUpdate.migrationClass ? migrationClassLabels[desktopUpdate.migrationClass] : '维护等级未知' }}</span>
              </div>
              <n-tag :bordered="false" :type="desktopUpdate.space?.sufficient ? 'success' : 'error'">
                {{ desktopUpdate.space?.sufficient ? '空间通过' : '空间不足' }}
              </n-tag>
            </div>

            <dl class="upgrade-facts">
              <div><dt>签名更新包</dt><dd>{{ formatBytes(desktopUpdate.artifactSizeBytes) }}</dd></div>
              <div><dt>数据卷空间</dt><dd>需要 {{ formatBytes(desktopUpdate.space?.dataRequiredBytes) }}；可用 {{ formatBytes(desktopUpdate.space?.dataFreeBytes) }}</dd></div>
              <div><dt>控制目录空间</dt><dd>需要 {{ formatBytes(desktopUpdate.space?.controlRequiredBytes) }}；可用 {{ formatBytes(desktopUpdate.space?.controlFreeBytes) }}</dd></div>
              <div><dt>服务影响</dt><dd>Desktop 会退出；M3 会在重新打开前完成离线 Candidate 验证。</dd></div>
            </dl>

            <n-alert type="warning" :bordered="false" title="数据保护是强制门禁">
              更新会先保留旧程序、checkpoint SQLite，并实际恢复完整数据副本。Candidate 的数据库、附件、AI 历史、审计和继续写入验证任一失败，均不会切换数据；首次新写入后禁止自动降级。
            </n-alert>
            <n-checkbox v-model:checked="confirmedDesktopRecovery">
              我已保存当前工作，并确认维护等级、磁盘空间、完整恢复验证和首次写入边界
            </n-checkbox>
            <n-button
              type="primary"
              :disabled="!confirmedDesktopRecovery || desktopUpdate.space?.sufficient !== true"
              :loading="applyingDesktopUpdate"
              @click="applyDesktopUpdate"
            >
              下载、验证并安装
            </n-button>
          </section>
          <n-alert v-else type="info" :bordered="false">
            MuriArc 不会静默安装更新。请先检查更新，再由你确认维护等级、空间和恢复策略。
          </n-alert>
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
.settings-content { width: 100%; max-width: 920px; min-width: 0; padding: 24px 28px; }.section-heading { margin-bottom: 20px; }.section-heading h2 { margin: 0 0 4px; font-size: 19px; }.section-heading p { margin: 0; color: var(--muri-text-secondary); }
.settings-form { display: grid; grid-template-columns: 1fr 1fr; gap: 0 15px; }.settings-form :deep(.full-row) { grid-column: 1 / -1; }
.mode-info { display: flex; align-items: center; gap: 10px; margin: 2px 0 18px; padding: 11px; border: 1px solid var(--muri-border); border-radius: 7px; background: var(--muri-surface-muted); }.status-dot { width: 8px; height: 8px; border-radius: 50%; background: var(--muri-success); box-shadow: 0 0 0 3px rgba(56,134,107,.14); }.mode-info div { display: flex; flex-direction: column; }.mode-info span { color: var(--muri-text-secondary); font-size: 11px; }
.subsection-heading { margin: 6px 0 14px; }.subsection-heading h3 { margin: 0 0 3px; font-size: 15px; }.subsection-heading p { margin: 0; color: var(--muri-text-secondary); font-size: 11px; }.password-heading { margin-top: 28px; padding-top: 22px; border-top: 1px solid var(--muri-border); }
.account-identity { display: grid; gap: 8px; }.account-identity > div { display: grid; grid-template-columns: 120px minmax(0, 1fr); gap: 12px; padding: 12px; border: 1px solid var(--muri-border); border-radius: 7px; }.account-identity span { color: var(--muri-text-tertiary); font-size: 11px; }.account-identity strong { overflow-wrap: anywhere; }
.password-strength { display: grid; grid-template-columns: auto minmax(120px, 1fr); align-items: center; gap: 12px; margin: -10px 0 12px; color: var(--muri-text-tertiary); font-size: 11px; }.password-strength .n-progress { width: 100%; }
.availability-alert { margin-bottom: 16px; }
.session-actions { display: flex; align-items: center; justify-content: space-between; gap: 16px; margin-top: 16px; padding: 13px; border: 1px solid var(--muri-border); border-radius: 7px; }.session-actions > div { display: flex; flex-direction: column; }.session-actions span { color: var(--muri-text-secondary); font-size: 11px; }
.storage-card { display: grid; gap: 14px; margin-bottom: 18px; padding: 16px; border: 1px solid var(--muri-border); border-radius: 9px; background: var(--muri-surface-muted); }
.storage-card-heading { display: grid; grid-template-columns: 30px minmax(0, 1fr) auto; align-items: center; gap: 10px; }.storage-card-heading > svg { color: var(--muri-primary); }.storage-card-heading > div { display: flex; min-width: 0; flex-direction: column; }.storage-card-heading span { color: var(--muri-text-secondary); font-size: 11px; }
.storage-paths { display: grid; gap: 8px; margin: 0; }.storage-paths > div { display: grid; grid-template-columns: 110px minmax(0, 1fr); gap: 12px; }.storage-paths dt { color: var(--muri-text-tertiary); font-size: 11px; }.storage-paths dd { min-width: 0; margin: 0; overflow-wrap: anywhere; font-family: ui-monospace, SFMono-Regular, Consolas, monospace; font-size: 12px; }
.storage-selection { display: flex; flex-direction: column; gap: 3px; padding: 11px; border: 1px solid rgba(42,104,137,.22); border-radius: 7px; background: rgba(42,104,137,.06); }.storage-selection span,.storage-selection small { color: var(--muri-text-secondary); font-size: 11px; }.storage-selection strong { overflow-wrap: anywhere; font-family: ui-monospace, SFMono-Regular, Consolas, monospace; font-size: 12px; }
.storage-actions { display: flex; flex-wrap: wrap; gap: 8px; }
.update-actions { display: flex; align-items: center; gap: 12px; margin-bottom: 16px; }.update-actions span { color: var(--muri-text-secondary); font-size: 12px; }
.update-card { display: grid; gap: 14px; padding: 16px; border: 1px solid var(--muri-border); border-radius: 9px; background: var(--muri-surface-muted); }.upgrade-facts { display: grid; gap: 8px; margin: 0; }.upgrade-facts > div { display: grid; grid-template-columns: 120px minmax(0, 1fr); gap: 12px; }.upgrade-facts dt { color: var(--muri-text-tertiary); font-size: 11px; }.upgrade-facts dd { margin: 0; overflow-wrap: anywhere; font-size: 12px; }
.action-card { display: grid; grid-template-columns: 32px 1fr auto; align-items: center; gap: 10px; margin-bottom: 10px; padding: 13px; border: 1px solid var(--muri-border); border-radius: 7px; }.action-card > svg { color: var(--muri-primary); }.action-card div { display: flex; flex-direction: column; }.action-card span { color: var(--muri-text-secondary); font-size: 11px; }
.policy-list { display: flex; flex-direction: column; gap: 8px; margin-top: 14px; }.policy-list > div { display: flex; align-items: flex-start; gap: 9px; padding: 12px; border: 1px solid var(--muri-border); border-radius: 7px; color: var(--muri-text-secondary); }.policy-list svg { flex: 0 0 auto; color: var(--muri-success); }.policy-list strong { display: block; color: var(--muri-text); }
.about-panel { display: flex; min-height: 470px; align-items: center; justify-content: center; flex-direction: column; text-align: center; }.about-panel img { width: 122px; height: 122px; object-fit: contain; }.about-panel h2 { margin: 12px 0 2px; font-size: 27px; }.about-panel p { margin: 0 0 13px; color: var(--muri-text-secondary); }.about-panel small { max-width: 460px; margin-top: 20px; color: var(--muri-text-tertiary); line-height: 1.6; }
@media (max-width: 900px) { .settings-layout { grid-template-columns: 1fr; }.settings-layout > nav { display: flex; overflow-x: auto; border-right: 0; border-bottom: 1px solid var(--muri-border); }.settings-layout nav button { width: auto; flex: 0 0 auto; }.settings-content { padding: 19px 15px; }.settings-form { grid-template-columns: 1fr; }.settings-form :deep(.n-form-item) { grid-column: 1 !important; }.storage-card-heading { grid-template-columns: 28px minmax(0, 1fr); }.storage-card-heading .n-tag { grid-column: 2; justify-self: start; }.storage-paths > div,.upgrade-facts > div { grid-template-columns: 1fr; gap: 2px; }.action-card { grid-template-columns: 28px 1fr; }.action-card button { grid-column: 2; justify-self: start; } }
</style>
