import { beforeEach, describe, expect, it } from 'vitest'
import type { AuthSession, ProjectRole } from '@/domain/models'
import {
  canExportData,
  canImportData,
  canManageBreeding,
  canWriteAnimal,
  canWriteExperiment,
  clearProjectContext,
  currentProjectId,
  hasLabRegistryAccess,
  initializeProjectContext,
  setCurrentProject,
} from './projectContext'

function projectSession(role: ProjectRole, userId = `user-${role}`): AuthSession {
  return {
    user: {
      id: userId,
      labId: 'lab-1',
      displayName: role,
      labRoles: [],
      projectRoles: [
        { projectId: 'project-1', role },
        { projectId: 'project-2', role },
      ],
      authentication: 'session', mustChangePassword: false, isEnvironmentRoot: false,
    },
    csrfAvailable: true,
  }
}

const projects = [
  { id: 'project-1', name: 'Project one' },
  { id: 'project-2', name: 'Project two' },
]

describe('current Project context', () => {
  beforeEach(() => {
    clearProjectContext()
    sessionStorage.clear()
  })

  it('requires a Project Viewer to remain in an allowed project and grants scoped export only', async () => {
    const session = projectSession('viewer')
    await initializeProjectContext(session, async () => projects)

    expect(currentProjectId.value).toBe('project-1')
    expect(hasLabRegistryAccess(session)).toBe(false)
    expect(canExportData(session)).toBe(true)
    expect(canImportData(session)).toBe(false)
    expect(canWriteAnimal(session)).toBe(false)
    expect(canManageBreeding(session)).toBe(false)
    expect(canWriteExperiment(session)).toBe(false)
    expect(() => setCurrentProject()).toThrow('必须选择一个科研项目')
  })

  it('restores an Editor project selection within the same browser session', async () => {
    const session = projectSession('editor')
    await initializeProjectContext(session, async () => projects)
    setCurrentProject('project-2')
    clearProjectContext()

    await initializeProjectContext(session, async () => projects)

    expect(currentProjectId.value).toBe('project-2')
    expect(canImportData(session)).toBe(true)
    expect(canWriteExperiment(session)).toBe(true)
    expect(canWriteAnimal(session)).toBe(false)
    expect(canManageBreeding(session)).toBe(false)
  })

  it('keeps a Lab operator in the lab Registry unless a project is selected', async () => {
    const session: AuthSession = {
      user: {
        id: 'manager-1',
        labId: 'lab-1',
        displayName: 'Manager',
        labRoles: ['animal_manager'],
        projectRoles: [],
        authentication: 'session', mustChangePassword: false, isEnvironmentRoot: false,
      },
      csrfAvailable: true,
    }
    await initializeProjectContext(session, async () => projects)

    expect(currentProjectId.value).toBeUndefined()
    expect(hasLabRegistryAccess(session)).toBe(true)
    expect(canManageBreeding(session)).toBe(true)
    setCurrentProject('project-2')
    expect(currentProjectId.value).toBe('project-2')
    setCurrentProject()
    expect(currentProjectId.value).toBeUndefined()
  })
})
