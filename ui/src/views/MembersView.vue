<script setup lang="ts">
import { computed, onMounted, reactive, ref, watch } from 'vue'
import { useMessage } from 'naive-ui'
import { FolderKey, KeyRound, Pencil, RefreshCw, ShieldCheck, Trash2, UserPlus } from '@lucide/vue'
import PageHeader from '@/components/PageHeader.vue'
import type {
  LabRole,
  ManagedProjectMembership,
  ManagedUser,
  ProjectRole,
  ProjectSummary,
} from '@/domain/models'
import { currentAuthSession, gateway } from '@/services/gateway'
import { passwordPolicyError, passwordStrength } from '@/services/passwordStrength'
import { currentProjectId, isLabAdmin } from '@/services/projectContext'

type PendingAction =
  | { kind: 'status'; user: ManagedUser; status: ManagedUser['status'] }
  | { kind: 'lab-role'; user: ManagedUser; role: LabRole | null }
  | { kind: 'project-role'; user: ManagedUser; membership: ManagedProjectMembership; role: ProjectRole }
  | { kind: 'project-revoke'; user: ManagedUser; membership: ManagedProjectMembership }
  | { kind: 'project-add'; user: ManagedUser; projectId: string; role: ProjectRole }

const message = useMessage()
const users = ref<ManagedUser[]>([])
const projects = ref<ProjectSummary[]>([])
const loading = ref(false)
const search = ref('')
const createOpen = ref(false)
const creating = ref(false)
const pending = ref<PendingAction | null>(null)
const applying = ref(false)
const currentPassword = ref('')
const profileTarget = ref<ManagedUser | null>(null)
const profileSaving = ref(false)
const profileForm = reactive({ email: '', displayName: '', currentPassword: '' })
const resetTarget = ref<ManagedUser | null>(null)
const resetSaving = ref(false)
const resetTemporaryPassword = ref('')
const resetCurrentPassword = ref('')
const currentUserId = computed(() => currentAuthSession.value?.user.id)
const currentIsEnvironmentRoot = computed(
  () => currentAuthSession.value?.user.isEnvironmentRoot === true,
)
const labAdminMode = computed(() => isLabAdmin())
const projectAdminMode = computed(() => !labAdminMode.value && Boolean(currentProjectId.value))
const available = computed(() => typeof gateway.listManagedUsers === 'function'
  && (labAdminMode.value
    ? typeof gateway.createManagedUser === 'function'
      && typeof gateway.setManagedUserStatus === 'function'
      && typeof gateway.updateManagedUserProfile === 'function'
      && typeof gateway.resetManagedUserPassword === 'function'
    : typeof gateway.grantProjectRole === 'function'
      && typeof gateway.updateProjectRole === 'function'
      && typeof gateway.revokeMembership === 'function'))

const createForm = reactive<{
  email: string
  displayName: string
  temporaryPassword: string
  currentPassword: string
  labRole: LabRole | null
  projectId: string | null
  projectRole: ProjectRole
}>({
  email: '',
  displayName: '',
  temporaryPassword: '',
  currentPassword: '',
  labRole: null,
  projectId: null,
  projectRole: 'viewer',
})

const labRoleOptions = computed(() => [
  { label: '动物管理员', value: 'animal_manager' as const },
  ...(currentIsEnvironmentRoot.value
    ? [{ label: '实验室管理员', value: 'lab_admin' as const }]
    : []),
])
const projectRoleOptions = [
  { label: '只读', value: 'viewer' },
  { label: '编辑者', value: 'editor' },
  { label: '项目管理员', value: 'project_admin' },
]
const projectOptions = computed(() => projects.value.map((project) => ({
  label: project.name,
  value: project.id,
})))
const createPasswordStrength = computed(() => passwordStrength(createForm.temporaryPassword))
const resetPasswordStrength = computed(() => passwordStrength(resetTemporaryPassword.value))
const filteredUsers = computed(() => {
  const query = search.value.trim().toLocaleLowerCase()
  if (!query) return users.value
  return users.value.filter((user) => `${user.displayName} ${user.email}`.toLocaleLowerCase().includes(query))
})
const actionTitle = computed(() => {
  switch (pending.value?.kind) {
    case 'status': return pending.value.status === 'suspended' ? '停用账号' : '重新启用账号'
    case 'lab-role': return '修改实验室角色'
    case 'project-role': return '修改项目权限'
    case 'project-revoke': return '撤销项目权限'
    case 'project-add': return '添加项目权限'
    default: return '加强确认'
  }
})

function labRoleLabel(role?: LabRole) {
  if (role === 'lab_admin') return '实验室管理员'
  if (role === 'animal_manager') return '动物管理员'
  return '仅项目成员'
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : '操作失败，请重试'
}

function canGovern(user: ManagedUser): boolean {
  if (user.id === currentUserId.value || user.isEnvironmentRoot) return false
  if (projectAdminMode.value) return true
  if (user.labRole === 'lab_admin' && !currentIsEnvironmentRoot.value) return false
  return true
}

function replaceUser(updated: ManagedUser) {
  const index = users.value.findIndex((user) => user.id === updated.id)
  if (index >= 0) users.value.splice(index, 1, updated)
  else users.value.push(updated)
}

async function load() {
  if (!gateway.listManagedUsers) return
  loading.value = true
  try {
    const [loadedUsers, loadedProjects] = await Promise.all([
      gateway.listManagedUsers(projectAdminMode.value ? currentProjectId.value : undefined),
      gateway.listProjects(),
    ])
    users.value = loadedUsers
    projects.value = projectAdminMode.value
      ? loadedProjects.filter((project) => project.id === currentProjectId.value)
      : loadedProjects
  } catch (error) {
    message.error(`无法读取成员：${errorMessage(error)}`)
  } finally {
    loading.value = false
  }
}

function clearCreatePasswords() {
  createForm.temporaryPassword = ''
  createForm.currentPassword = ''
}

function closeCreate() {
  clearCreatePasswords()
  createOpen.value = false
}

function closePending() {
  currentPassword.value = ''
  pending.value = null
}

async function createUser() {
  if (!gateway.createManagedUser || creating.value) return
  const email = createForm.email.trim()
  const displayName = createForm.displayName.trim()
  if (!email || !displayName || !createForm.temporaryPassword || !createForm.currentPassword) {
    message.warning('请完整填写姓名、邮箱、临时密码与当前管理员密码')
    clearCreatePasswords()
    return
  }
  const validation = passwordPolicyError(createForm.temporaryPassword)
  if (validation) {
    message.warning(validation)
    clearCreatePasswords()
    return
  }
  if (!createForm.labRole && !createForm.projectId) {
    message.warning('仅项目成员必须至少选择一个科研项目')
    clearCreatePasswords()
    return
  }
  creating.value = true
  try {
    const user = await gateway.createManagedUser({
      email,
      displayName,
      temporaryPassword: createForm.temporaryPassword,
      currentPassword: createForm.currentPassword,
      labRole: createForm.labRole ?? undefined,
      projectRoles: createForm.projectId
        ? [{ projectId: createForm.projectId, role: createForm.projectRole }]
        : [],
    })
    replaceUser(user)
    closeCreate()
    Object.assign(createForm, {
      email: '', displayName: '', temporaryPassword: '', currentPassword: '',
      labRole: null, projectId: null, projectRole: 'viewer',
    })
    message.success('成员账号已创建；用户首次登录必须修改临时密码')
  } catch (error) {
    message.error(`创建失败：${errorMessage(error)}`)
  } finally {
    clearCreatePasswords()
    creating.value = false
  }
}

function beginStatus(user: ManagedUser) {
  if (!canGovern(user)) return
  pending.value = {
    kind: 'status',
    user,
    status: user.status === 'active' ? 'suspended' : 'active',
  }
  currentPassword.value = ''
}

function beginLabRole(user: ManagedUser, role: LabRole | null) {
  if (!canGovern(user) || role === (user.labRole ?? null)) return
  pending.value = { kind: 'lab-role', user, role }
  currentPassword.value = ''
}

function beginProjectRole(
  user: ManagedUser,
  membership: ManagedProjectMembership,
  role: ProjectRole,
) {
  if (!canGovern(user) || role === membership.role) return
  pending.value = { kind: 'project-role', user, membership, role }
  currentPassword.value = ''
}

function beginProjectRevoke(user: ManagedUser, membership: ManagedProjectMembership) {
  if (!canGovern(user)) return
  pending.value = { kind: 'project-revoke', user, membership }
  currentPassword.value = ''
}

function beginProjectAdd(user: ManagedUser) {
  if (!canGovern(user)) return
  const assigned = new Set(user.projectMemberships.map((membership) => membership.projectId))
  const project = projects.value.find((candidate) => !assigned.has(candidate.id))
  if (!project) {
    message.info('该成员已拥有全部现有项目的权限')
    return
  }
  pending.value = { kind: 'project-add', user, projectId: project.id, role: 'viewer' }
  currentPassword.value = ''
}

async function applyAction() {
  const action = pending.value
  const password = currentPassword.value
  if (!action || !password || applying.value) {
    if (!password) message.warning('请输入当前管理员密码')
    return
  }
  if (!canGovern(action.user)) {
    closePending()
    return
  }
  applying.value = true
  try {
    let updated: ManagedUser
    switch (action.kind) {
      case 'status':
        updated = await gateway.setManagedUserStatus!(action.user.id, {
          expectedRevision: action.user.revision,
          status: action.status,
          currentPassword: password,
        })
        break
      case 'lab-role':
        if (action.role && action.user.labMembershipId) {
          updated = await gateway.updateLabRole!(action.user.labMembershipId, {
            expectedRevision: action.user.labMembershipRevision!,
            role: action.role,
            currentPassword: password,
          })
        } else if (action.role) {
          updated = await gateway.grantLabRole!(action.user.id, {
            expectedUserRevision: action.user.revision,
            role: action.role,
            currentPassword: password,
          })
        } else {
          updated = await gateway.revokeMembership!(action.user.labMembershipId!, {
            expectedRevision: action.user.labMembershipRevision!,
            currentPassword: password,
          })
        }
        break
      case 'project-role':
        updated = await gateway.updateProjectRole!(action.membership.membershipId, {
          expectedRevision: action.membership.revision,
          role: action.role,
          currentPassword: password,
        })
        break
      case 'project-revoke':
        updated = await gateway.revokeMembership!(action.membership.membershipId, {
          expectedRevision: action.membership.revision,
          currentPassword: password,
        })
        break
      case 'project-add':
        updated = await gateway.grantProjectRole!(action.user.id, {
          expectedUserRevision: action.user.revision,
          projectId: action.projectId,
          role: action.role,
          currentPassword: password,
        })
        break
    }
    replaceUser(updated)
    closePending()
    message.success('权限变更已保存并写入审计')
  } catch (error) {
    message.error(`操作失败：${errorMessage(error)}`)
  } finally {
    currentPassword.value = ''
    applying.value = false
  }
}

function beginProfile(user: ManagedUser) {
  if (!canGovern(user)) return
  profileTarget.value = user
  Object.assign(profileForm, {
    email: user.email,
    displayName: user.displayName,
    currentPassword: '',
  })
}

function closeProfile() {
  profileForm.currentPassword = ''
  profileTarget.value = null
}

async function saveProfile() {
  const user = profileTarget.value
  if (!user || !gateway.updateManagedUserProfile || profileSaving.value) return
  const email = profileForm.email.trim()
  const displayName = profileForm.displayName.trim()
  if (!email || !displayName || !profileForm.currentPassword) {
    message.warning('请完整填写显示名称、邮箱和当前管理员密码')
    profileForm.currentPassword = ''
    return
  }
  profileSaving.value = true
  try {
    const updated = await gateway.updateManagedUserProfile(user.id, {
      expectedRevision: user.revision,
      email,
      displayName,
      currentPassword: profileForm.currentPassword,
    })
    replaceUser(updated)
    closeProfile()
    message.success('成员账号资料已更新')
  } catch (error) {
    message.error(`保存失败：${errorMessage(error)}`)
  } finally {
    profileForm.currentPassword = ''
    profileSaving.value = false
  }
}

function beginPasswordReset(user: ManagedUser) {
  if (!canGovern(user)) return
  resetTarget.value = user
  resetTemporaryPassword.value = ''
  resetCurrentPassword.value = ''
}

function closePasswordReset() {
  resetTemporaryPassword.value = ''
  resetCurrentPassword.value = ''
  resetTarget.value = null
}

async function resetPassword() {
  const user = resetTarget.value
  if (!user || !gateway.resetManagedUserPassword || resetSaving.value) return
  const validation = passwordPolicyError(resetTemporaryPassword.value)
  if (!resetCurrentPassword.value) {
    message.warning('请输入当前管理员密码')
    resetTemporaryPassword.value = ''
    resetCurrentPassword.value = ''
    return
  }
  if (validation) {
    message.warning(validation)
    resetTemporaryPassword.value = ''
    resetCurrentPassword.value = ''
    return
  }
  resetSaving.value = true
  try {
    const updated = await gateway.resetManagedUserPassword(user.id, {
      expectedCredentialRevision: user.credentialRevision,
      temporaryPassword: resetTemporaryPassword.value,
      currentPassword: resetCurrentPassword.value,
    })
    replaceUser(updated)
    closePasswordReset()
    message.success('临时密码已设置；目标会话与 external token 已撤销')
  } catch (error) {
    message.error(`重置失败：${errorMessage(error)}`)
  } finally {
    resetTemporaryPassword.value = ''
    resetCurrentPassword.value = ''
    resetSaving.value = false
  }
}

onMounted(() => void load())
watch(currentProjectId, () => {
  if (projectAdminMode.value) void load()
})
</script>

<template>
  <div class="page members-page">
    <PageHeader title="成员管理" :description="projectAdminMode ? '管理当前项目成员及项目角色；不能修改实验室账号或其他项目。' : '按 Environment Root、LabAdmin 与项目层级管理共享实验室账号。'">
      <template #actions>
        <n-button secondary :loading="loading" @click="load"><template #icon><RefreshCw :size="16" /></template>刷新</n-button>
        <n-button v-if="labAdminMode" type="primary" :disabled="!available" @click="createOpen = true"><template #icon><UserPlus :size="16" /></template>新建成员</n-button>
      </template>
    </PageHeader>

    <n-alert type="info" :bordered="false" class="security-alert">
      {{ projectAdminMode
        ? '项目权限变更要求当前密码并写入审计；最后一名有效 ProjectAdmin 不能被移除或降级。'
        : '账号治理要求当前管理员密码。只有 Environment Root 能创建或治理 LabAdmin；管理员永远不能查看任何用户的现有密码。' }}
    </n-alert>

    <section class="toolbar surface">
      <n-input v-model:value="search" clearable placeholder="搜索姓名或邮箱" />
      <span>{{ filteredUsers.length }} 位成员</span>
    </section>

    <section class="member-list" :class="{ loading }">
      <n-empty v-if="!loading && filteredUsers.length === 0" description="没有匹配的成员" />
      <article v-for="user in filteredUsers" :key="user.id" class="member-card surface">
        <div class="identity">
          <span class="member-avatar">{{ user.displayName.charAt(0) || '用' }}</span>
          <div><strong>{{ user.displayName }}</strong><span>{{ user.email }}</span></div>
          <div class="badges">
            <n-tag v-if="user.isEnvironmentRoot" size="small" type="error" :bordered="false">Environment Root</n-tag>
            <n-tag v-else-if="user.labRole === 'lab_admin'" size="small" type="warning" :bordered="false">LabAdmin</n-tag>
            <n-tag v-if="user.mustChangePassword" size="small" type="info" :bordered="false">必须改密</n-tag>
            <n-tag size="small" :type="user.status === 'active' ? 'success' : 'default'" :bordered="false">{{ user.status === 'active' ? '正常' : '已停用' }}</n-tag>
          </div>
        </div>

        <div v-if="labAdminMode" class="role-block">
          <label>实验室角色</label>
          <n-select
            :value="user.labRole ?? null"
            :options="labRoleOptions"
            clearable
            placeholder="仅项目成员"
            :disabled="!canGovern(user)"
            @update:value="(role: LabRole | null) => beginLabRole(user, role)"
          />
          <small>{{ labRoleLabel(user.labRole) }}<template v-if="!canGovern(user)"> · {{ user.isEnvironmentRoot ? '部署配置管理' : user.id === currentUserId ? '当前账号请使用账号安全' : '同级 LabAdmin 仅 Root 可治理' }}</template></small>
        </div>

        <div class="projects-block">
          <div class="block-heading"><label>项目权限</label><n-button text type="primary" :disabled="!canGovern(user)" @click="beginProjectAdd(user)"><FolderKey :size="15" />添加项目</n-button></div>
          <div v-if="user.projectMemberships.length" class="project-grants">
            <div v-for="membership in user.projectMemberships" :key="membership.membershipId" class="project-grant">
              <span :title="membership.projectName">{{ membership.projectName }}</span>
              <n-select
                size="small"
                :value="membership.role"
                :options="projectRoleOptions"
                :disabled="!canGovern(user)"
                @update:value="(role: ProjectRole) => beginProjectRole(user, membership, role)"
              />
              <n-button quaternary circle size="small" aria-label="撤销项目权限" :disabled="!canGovern(user)" @click="beginProjectRevoke(user, membership)"><Trash2 :size="15" /></n-button>
            </div>
          </div>
          <span v-else class="empty-grants">尚未加入科研项目</span>
        </div>

        <div v-if="labAdminMode" class="member-actions">
          <n-button secondary :disabled="!canGovern(user)" @click="beginProfile(user)"><template #icon><Pencil :size="15" /></template>编辑账号</n-button>
          <n-button secondary :disabled="!canGovern(user)" @click="beginPasswordReset(user)"><template #icon><KeyRound :size="15" /></template>重置密码</n-button>
          <n-button secondary :type="user.status === 'active' ? 'warning' : 'primary'" :disabled="!canGovern(user)" @click="beginStatus(user)">{{ user.status === 'active' ? '停用账号' : '重新启用' }}</n-button>
        </div>
      </article>
    </section>

    <n-modal :show="createOpen" preset="card" title="新建共享成员" class="modal-card" :mask-closable="!creating" @update:show="(show: boolean) => { if (!show) closeCreate() }">
      <n-form label-placement="top">
        <div class="form-grid">
          <n-form-item label="显示名称"><n-input v-model:value="createForm.displayName" maxlength="200" /></n-form-item>
          <n-form-item label="登录邮箱"><n-input v-model:value="createForm.email" maxlength="320" /></n-form-item>
          <n-form-item label="临时密码"><n-input v-model:value="createForm.temporaryPassword" type="password" show-password-on="click" autocomplete="new-password" maxlength="1024" placeholder="至少 8 个字符" /></n-form-item>
          <n-form-item label="实验室角色"><n-select v-model:value="createForm.labRole" clearable placeholder="仅项目成员" :options="labRoleOptions" /></n-form-item>
          <div class="password-strength full-row"><span>建议强度：{{ createPasswordStrength.label }}</span><n-progress type="line" :show-indicator="false" :percentage="createPasswordStrength.percentage" :status="createPasswordStrength.status" /></div>
          <n-form-item label="初始科研项目"><n-select v-model:value="createForm.projectId" clearable :options="projectOptions" placeholder="仅项目成员必须选择" /></n-form-item>
          <n-form-item label="项目角色"><n-select v-model:value="createForm.projectRole" :disabled="!createForm.projectId" :options="projectRoleOptions" /></n-form-item>
          <n-form-item label="当前管理员密码" class="full-row"><n-input v-model:value="createForm.currentPassword" type="password" show-password-on="click" autocomplete="current-password" /></n-form-item>
        </div>
      </n-form>
      <n-alert type="warning" :bordered="false">临时密码不会回显；新成员首次登录必须自行修改。请通过安全渠道传递。</n-alert>
      <template #footer><div class="modal-actions"><n-button @click="closeCreate">取消</n-button><n-button type="primary" :loading="creating" @click="createUser">创建账号</n-button></div></template>
    </n-modal>

    <n-modal :show="Boolean(profileTarget)" preset="card" title="编辑成员账号" class="confirm-card" :mask-closable="!profileSaving" @update:show="(show: boolean) => { if (!show) closeProfile() }">
      <n-form label-placement="top">
        <n-form-item label="显示名称"><n-input v-model:value="profileForm.displayName" maxlength="200" /></n-form-item>
        <n-form-item label="登录邮箱"><n-input v-model:value="profileForm.email" maxlength="320" /></n-form-item>
        <n-form-item label="当前管理员密码"><n-input v-model:value="profileForm.currentPassword" type="password" show-password-on="click" autocomplete="current-password" @keyup.enter="saveProfile" /></n-form-item>
      </n-form>
      <template #footer><div class="modal-actions"><n-button @click="closeProfile">取消</n-button><n-button type="primary" :loading="profileSaving" @click="saveProfile">保存账号</n-button></div></template>
    </n-modal>

    <n-modal :show="Boolean(resetTarget)" preset="card" title="强制重置密码" class="confirm-card" :mask-closable="!resetSaving" @update:show="(show: boolean) => { if (!show) closePasswordReset() }">
      <n-alert type="warning" :bordered="false" class="reset-alert">重置会立即撤销该用户全部 Session 与 external token，并要求下次登录修改临时密码。管理员不能查看原密码。</n-alert>
      <n-form label-placement="top">
        <n-form-item label="新临时密码"><n-input v-model:value="resetTemporaryPassword" type="password" show-password-on="click" autocomplete="new-password" maxlength="1024" /></n-form-item>
        <div class="password-strength"><span>建议强度：{{ resetPasswordStrength.label }}</span><n-progress type="line" :show-indicator="false" :percentage="resetPasswordStrength.percentage" :status="resetPasswordStrength.status" /></div>
        <n-form-item label="当前管理员密码"><n-input v-model:value="resetCurrentPassword" type="password" show-password-on="click" autocomplete="current-password" @keyup.enter="resetPassword" /></n-form-item>
      </n-form>
      <template #footer><div class="modal-actions"><n-button @click="closePasswordReset">取消</n-button><n-button type="primary" :loading="resetSaving" @click="resetPassword">确认重置</n-button></div></template>
    </n-modal>

    <n-modal :show="Boolean(pending)" preset="card" :title="actionTitle" class="confirm-card" :mask-closable="!applying" @update:show="(show: boolean) => { if (!show) closePending() }">
      <template v-if="pending?.kind === 'project-add'">
        <n-form label-placement="top">
          <n-form-item label="科研项目"><n-select v-model:value="pending.projectId" :options="projectOptions" /></n-form-item>
          <n-form-item label="项目角色"><n-select v-model:value="pending.role" :options="projectRoleOptions" /></n-form-item>
        </n-form>
      </template>
      <p class="confirm-copy"><ShieldCheck :size="18" />该操作会立即改变实时权限，并记录操作者、revision 和审计信息。</p>
      <n-form-item label="当前管理员密码"><n-input v-model:value="currentPassword" type="password" show-password-on="click" autocomplete="current-password" @keyup.enter="applyAction" /></n-form-item>
      <template #footer><div class="modal-actions"><n-button @click="closePending">取消</n-button><n-button type="primary" :loading="applying" @click="applyAction">确认变更</n-button></div></template>
    </n-modal>
  </div>
</template>

<style scoped>
.security-alert { margin-bottom: 12px; }
.toolbar { display: flex; align-items: center; gap: 16px; margin-bottom: 12px; padding: 10px 12px; }.toolbar .n-input { max-width: 360px; }.toolbar span { margin-left: auto; color: var(--muri-text-tertiary); font-size: 12px; }
.member-list { display: flex; flex-direction: column; gap: 9px; transition: opacity var(--muri-transition-fast); }.member-list.loading { opacity: .58; }
.member-card { display: grid; grid-template-columns: minmax(260px, 1.15fr) minmax(180px, .75fr) minmax(300px, 1.4fr) 130px; align-items: center; gap: 16px; padding: 14px; }
.identity { display: grid; min-width: 0; grid-template-columns: 38px minmax(0, 1fr); align-items: center; gap: 10px; }.member-avatar { display: grid; width: 38px; height: 38px; place-items: center; border-radius: 50%; color: var(--muri-primary); background: var(--muri-primary-soft); font-weight: 700; }.identity > div:not(.badges) { display: flex; min-width: 0; flex-direction: column; }.identity strong,.identity span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }.identity > div span { color: var(--muri-text-tertiary); font-size: 11px; }.badges { display: flex; grid-column: 2; flex-wrap: wrap; gap: 4px; }
.role-block { min-width: 0; }.role-block label,.projects-block label { display: block; margin-bottom: 5px; color: var(--muri-text-secondary); font-size: 11px; font-weight: 600; }.role-block small { display: block; margin-top: 4px; color: var(--muri-text-tertiary); font-size: 10px; }
.block-heading { display: flex; align-items: center; justify-content: space-between; }.block-heading button { display: inline-flex; align-items: center; gap: 4px; font-size: 11px; }.project-grants { display: flex; flex-direction: column; gap: 5px; }.project-grant { display: grid; grid-template-columns: minmax(100px, 1fr) 118px 30px; align-items: center; gap: 6px; }.project-grant > span { overflow: hidden; font-size: 12px; text-overflow: ellipsis; white-space: nowrap; }.empty-grants { color: var(--muri-text-tertiary); font-size: 11px; }
.member-actions { display: flex; align-self: stretch; justify-content: center; flex-direction: column; gap: 6px; }.member-actions button { width: 100%; }
.modal-card { width: min(680px, calc(100vw - 28px)); }.confirm-card { width: min(480px, calc(100vw - 28px)); }.form-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 0 14px; }.form-grid :deep(.full-row) { grid-column: 1 / -1; }.modal-actions { display: flex; justify-content: flex-end; gap: 8px; }.confirm-copy { display: flex; align-items: flex-start; gap: 8px; margin: 0 0 16px; padding: 10px; color: var(--muri-text-secondary); background: var(--muri-primary-soft); font-size: 12px; }.confirm-copy svg { flex: 0 0 auto; color: var(--muri-primary); }.reset-alert { margin-bottom: 15px; }
.password-strength { display: grid; grid-template-columns: auto minmax(100px, 1fr); align-items: center; gap: 12px; margin: -10px 0 14px; color: var(--muri-text-tertiary); font-size: 11px; }.password-strength .n-progress { width: 100%; }
@media (max-width: 1180px) { .member-card { grid-template-columns: minmax(240px, 1fr) minmax(180px, .8fr) 130px; }.projects-block { grid-column: 1 / -1; }.member-actions { grid-column: 3; grid-row: 1; } }
@media (max-width: 900px) { .toolbar { gap: 8px; }.member-card { grid-template-columns: 1fr; gap: 13px; }.member-actions,.projects-block { grid-column: 1; grid-row: auto; justify-self: stretch; }.member-actions { flex-direction: row; flex-wrap: wrap; }.member-actions button { width: auto; flex: 1 1 120px; }.project-grant { grid-template-columns: minmax(90px, 1fr) 112px 30px; }.form-grid { grid-template-columns: 1fr; }.form-grid :deep(.n-form-item),.form-grid :deep(.full-row) { grid-column: 1 !important; } }
</style>
