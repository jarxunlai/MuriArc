import { createRouter, createWebHashHistory, type RouteRecordRaw } from 'vue-router'
import { gateway, HttpGatewayError } from '@/services/gateway'
import type { AuthSession } from '@/domain/models'
import { hasEnteredLocalSpace } from '@/services/localWelcome'
import {
  availableProjects,
  hasLabRegistryAccess,
  initializeProjectContext,
  isActiveProjectAdmin,
  isLabAdmin,
  setCurrentProject,
} from '@/services/projectContext'

const routes: RouteRecordRaw[] = [
  { path: '/', redirect: '/cages' },
  { path: '/cages', name: 'cages', component: () => import('@/views/CagesView.vue'), meta: { title: '笼位视图', section: '动物管理' } },
  { path: '/animals', name: 'animals', component: () => import('@/views/AnimalsView.vue'), meta: { title: '动物档案', section: '动物管理' } },
  { path: '/animal-data', name: 'animal-data', component: () => import('@/views/DataCenterView.vue'), meta: { title: '动物数据', section: '动物管理', animalData: true } },
  { path: '/breeding', name: 'breeding', component: () => import('@/views/BreedingView.vue'), meta: { title: '繁育管理', section: '动物管理' } },
  { path: '/experiments', name: 'experiments', component: () => import('@/views/ExperimentsView.vue'), meta: { title: '实验管理' } },
  { path: '/experiments/:experimentId/:section?', name: 'experiment-detail', component: () => import('@/views/ExperimentsView.vue'), meta: { title: '实验工作区', section: '实验管理' } },
  { path: '/data', name: 'data', component: () => import('@/views/DataCenterView.vue'), meta: { title: '数据中心' } },
  { path: '/library', name: 'library', component: () => import('@/views/LibraryView.vue'), meta: { title: '项目资料库' } },
  { path: '/operations', name: 'operations', component: () => import('@/views/OperationsView.vue'), meta: { title: '活动记录', section: '管理与工具' } },
  { path: '/ai', name: 'ai', component: () => import('@/views/AiWorkspaceView.vue'), meta: { title: 'AI 助手' } },
  { path: '/ai/images', name: 'ai-images', component: () => import('@/views/AiImagesView.vue'), meta: { title: '私人 AI 图片' } },
  { path: '/admin/ai', name: 'ai-admin', component: () => import('@/views/AiAdminView.vue'), meta: { title: 'AI 管理', section: '实验室管理' } },
  { path: '/settings', name: 'settings', component: () => import('@/views/SettingsView.vue'), meta: { title: '设置' } },
  { path: '/members', name: 'members', component: () => import('@/views/MembersView.vue'), meta: { title: '成员管理', section: '实验室管理' } },
  { path: '/login', name: 'login', component: () => import('@/views/LoginView.vue'), meta: { title: '登录', authShell: true } },
  { path: '/change-password', name: 'change-password', component: () => import('@/views/ChangePasswordView.vue'), meta: { title: '修改密码', authShell: true } },
  { path: '/welcome', name: 'local-welcome', component: () => import('@/views/LocalWelcomeView.vue'), meta: { title: '进入本地空间', authShell: true } },
  { path: '/:pathMatch(.*)*', redirect: '/cages' },
]

export const router = createRouter({
  history: createWebHashHistory(),
  routes,
  scrollBehavior: () => ({ top: 0 }),
})

function defaultAuthenticatedRoute(session: AuthSession): string {
  return '/cages'
}

function safeRedirect(value: unknown, fallback = '/cages'): string {
  return typeof value === 'string'
    && value.startsWith('/')
    && !value.startsWith('//')
    && !value.startsWith('/login')
    && !value.startsWith('/change-password')
    && !value.startsWith('/welcome')
    ? value
    : fallback
}

router.beforeEach(async (to) => {
  if (gateway.mode !== 'remote') {
    const needsWelcome = gateway.requiresLocalWelcome === true
    const entered = !needsWelcome || hasEnteredLocalSpace()
    if (!entered) {
      return to.name === 'local-welcome'
        ? true
        : { name: 'local-welcome', query: { redirect: safeRedirect(to.fullPath) } }
    }
    if (to.name === 'local-welcome' || to.name === 'login' || to.name === 'change-password') {
      return safeRedirect(to.query.redirect)
    }
    return to.name === 'members' ? { name: 'settings' } : true
  }

  if (to.name === 'local-welcome') return { name: 'login' }
  if (!gateway.restoreSession) {
    return to.name === 'login'
      ? true
      : { name: 'login', query: { redirect: safeRedirect(to.fullPath) } }
  }

  try {
    const session = await gateway.restoreSession()
    const fallback = defaultAuthenticatedRoute(session)
    if (session.user.mustChangePassword) {
      return to.name === 'change-password'
        ? true
        : { name: 'change-password', query: { redirect: safeRedirect(to.fullPath, fallback) } }
    }
    if (to.name === 'change-password') return safeRedirect(to.query.redirect, fallback)

    await initializeProjectContext(session, () => gateway.listProjects())
    const requestedProjectId = typeof to.query.project_id === 'string'
      ? to.query.project_id
      : undefined
    if (requestedProjectId) {
      if (!availableProjects.value.some((project) => project.id === requestedProjectId)) {
        return fallback
      }
      setCurrentProject(requestedProjectId)
    }
    if (!hasLabRegistryAccess(session) && (to.name === 'breeding' || to.name === 'animal-data')) {
      return { name: 'animals' }
    }
    if (to.name === 'members' && !isLabAdmin(session) && !isActiveProjectAdmin(session)) {
      return fallback
    }
    if (to.name === 'ai-admin' && !isLabAdmin(session)) {
      return fallback
    }
    return to.name === 'login' ? safeRedirect(to.query.redirect, fallback) : true
  } catch (error) {
    if (error instanceof HttpGatewayError && error.code === 'password_change_required') {
      return { name: 'change-password', query: { redirect: safeRedirect(to.fullPath) } }
    }
    if (error instanceof HttpGatewayError && error.status === 401) {
      return to.name === 'login'
        ? true
        : { name: 'login', query: { redirect: safeRedirect(to.fullPath) } }
    }
    throw error
  }
})
