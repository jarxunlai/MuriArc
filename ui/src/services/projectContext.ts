import { computed, shallowRef, type Ref } from 'vue'
import type { AuthSession, ProjectRole, ProjectSummary } from '@/domain/models'

const STORAGE_PREFIX = 'muriarc.current-project.'

export const currentAuthSession = shallowRef<AuthSession>()
export const availableProjects = shallowRef<ProjectSummary[]>([])
export const currentProjectId = shallowRef<string>()

let initializedUserId: string | undefined
let initialization: Promise<void> | undefined

export const currentProject = computed(() => availableProjects.value.find(
  (project) => project.id === currentProjectId.value,
))

export function hasLabRegistryAccess(session = currentAuthSession.value): boolean {
  return session?.user.labRoles.some((role) => role === 'lab_admin' || role === 'animal_manager')
    ?? false
}

export function isLabAdmin(session = currentAuthSession.value): boolean {
  return session?.user.labRoles.includes('lab_admin') ?? false
}

export function activeProjectRole(session = currentAuthSession.value): ProjectRole | undefined {
  const projectId = currentProjectId.value
  if (!session || !projectId) return undefined
  const roles = session.user.projectRoles
    .filter((membership) => membership.projectId === projectId)
    .map((membership) => membership.role)
  if (roles.includes('project_admin')) return 'project_admin'
  if (roles.includes('editor')) return 'editor'
  return roles.includes('viewer') ? 'viewer' : undefined
}

export function canWriteAnimal(session = currentAuthSession.value): boolean {
  if (!session) return true
  return isLabAdmin(session)
    || session.user.labRoles.includes('animal_manager')
    || activeProjectRole(session) === 'project_admin'
}

export function canManageBreeding(session = currentAuthSession.value): boolean {
  if (!session) return true
  return isLabAdmin(session)
    || session.user.labRoles.includes('animal_manager')
    || activeProjectRole(session) === 'project_admin'
}

export function canWriteExperiment(session = currentAuthSession.value): boolean {
  if (!session) return true
  const projectRole = activeProjectRole(session)
  return isLabAdmin(session) || projectRole === 'project_admin' || projectRole === 'editor'
}

export function canWriteProjectData(session = currentAuthSession.value): boolean {
  if (!session) return true
  const projectRole = activeProjectRole(session)
  return isLabAdmin(session) || projectRole === 'project_admin' || projectRole === 'editor'
}

export function canPublishTemplate(session = currentAuthSession.value): boolean {
  if (!session) return true
  return isLabAdmin(session) || activeProjectRole(session) === 'project_admin'
}

export function canCreateProject(session = currentAuthSession.value): boolean {
  return !session || isLabAdmin(session)
}

export function canImportData(session = currentAuthSession.value): boolean {
  if (!session) return true
  return hasLabRegistryAccess(session)
    || activeProjectRole(session) === 'project_admin'
    || activeProjectRole(session) === 'editor'
}

export function canExportData(session = currentAuthSession.value): boolean {
  if (!session) return true
  return hasLabRegistryAccess(session) || activeProjectRole(session) != null
}

export function canCreateSnapshot(session = currentAuthSession.value): boolean {
  return !session || hasLabRegistryAccess(session)
}

export function setCurrentProject(projectId?: string): void {
  const session = currentAuthSession.value
  if (projectId && !availableProjects.value.some((project) => project.id === projectId)) {
    throw new Error('当前账号无权访问所选科研项目')
  }
  if (!projectId && session && !hasLabRegistryAccess(session)) {
    throw new Error('纯项目成员必须选择一个科研项目')
  }
  currentProjectId.value = projectId
  if (!session) return
  try {
    const key = `${STORAGE_PREFIX}${session.user.id}`
    if (projectId) sessionStorage.setItem(key, projectId)
    else sessionStorage.removeItem(key)
  } catch {
    // Session storage can be unavailable in hardened browsers. The in-memory
    // context remains authoritative for this page lifecycle.
  }
}

export async function initializeProjectContext(
  session: AuthSession,
  loadProjects: () => Promise<ProjectSummary[]>,
): Promise<void> {
  currentAuthSession.value = session
  if (initializedUserId === session.user.id) return
  if (initialization) return initialization

  initialization = (async () => {
    const projects = await loadProjects()
    availableProjects.value = projects
    let stored: string | null = null
    try {
      stored = sessionStorage.getItem(`${STORAGE_PREFIX}${session.user.id}`)
    } catch {
      stored = null
    }
    const storedIsAllowed = stored != null && projects.some((project) => project.id === stored)
    const next = storedIsAllowed
      ? stored!
      : hasLabRegistryAccess(session)
        ? undefined
        : projects[0]?.id
    if (!next && !hasLabRegistryAccess(session)) {
      throw new Error('当前账号没有可访问的科研项目，请联系实验室管理员')
    }
    currentProjectId.value = next
    initializedUserId = session.user.id
  })().finally(() => {
    initialization = undefined
  })
  return initialization
}

export function clearProjectContext(): void {
  initializedUserId = undefined
  initialization = undefined
  availableProjects.value = []
  currentProjectId.value = undefined
  currentAuthSession.value = undefined
}

export function activeProjectId(explicit?: string): string | undefined {
  return explicit ?? currentProjectId.value
}

/** Exposed for components that need a stable reactive dependency in tests. */
export function useCurrentProjectId(): Readonly<Ref<string | undefined>> {
  return currentProjectId
}
