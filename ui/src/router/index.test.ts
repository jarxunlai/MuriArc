import { beforeEach, describe, expect, it, vi } from 'vitest'

const auth = vi.hoisted(() => ({
  gateway: {
    mode: 'remote' as 'remote' | 'local',
    requiresLocalWelcome: false,
    restoreSession: vi.fn(),
    listProjects: vi.fn(),
  },
}))

vi.mock('@/services/gateway', () => ({
  gateway: auth.gateway,
  HttpGatewayError: class HttpGatewayError extends Error {
    constructor(readonly status: number, message: string, readonly code?: string) {
      super(message)
    }
  },
}))

// These tests exercise only navigation guards. Loading the full async page
// trees makes the first guarded navigation depend on unrelated view transforms
// and can leave it pending past Vitest's timeout under parallel/UNC runs.
vi.mock('@/views/CagesView.vue', () => ({ default: { template: '<div />' } }))
vi.mock('@/views/AnimalsView.vue', () => ({ default: { template: '<div />' } }))
vi.mock('@/views/LoginView.vue', () => ({ default: { template: '<div />' } }))
vi.mock('@/views/ChangePasswordView.vue', () => ({ default: { template: '<div />' } }))
vi.mock('@/views/LocalWelcomeView.vue', () => ({ default: { template: '<div />' } }))

import { HttpGatewayError } from '@/services/gateway'
import { clearProjectContext, currentProjectId } from '@/services/projectContext'
import { router } from './index'

describe('remote route authentication guard', () => {
  beforeEach(async () => {
    window.scrollTo = vi.fn()
    auth.gateway.mode = 'remote'
    auth.gateway.requiresLocalWelcome = false
    sessionStorage.clear()
    auth.gateway.restoreSession.mockReset().mockRejectedValue(
      new HttpGatewayError(401, '请先登录'),
    )
    auth.gateway.listProjects.mockReset().mockResolvedValue([
      { id: 'project-1', name: 'Project one' },
    ])
    clearProjectContext()
    await router.replace('/')
    auth.gateway.restoreSession.mockClear()
  })

  it('returns an unauthenticated remote user to login and then resumes the safe route', async () => {
    auth.gateway.restoreSession.mockRejectedValue(new HttpGatewayError(401, '请先登录'))

    await router.push('/animals?project_id=project-1')

    expect(router.currentRoute.value.name).toBe('login')
    expect(router.currentRoute.value.query.redirect).toBe('/animals?project_id=project-1')

    auth.gateway.restoreSession.mockResolvedValue({
      user: {
        id: 'user-1', labId: 'lab-1', displayName: 'Manager',
        labRoles: ['animal_manager'], projectRoles: [], authentication: 'session', mustChangePassword: false, isEnvironmentRoot: false,
      },
      csrfAvailable: true,
    })
    await router.push('/animals?project_id=project-1')

    expect(router.currentRoute.value.fullPath).toBe('/animals?project_id=project-1')
  })

  it('never shows the Server login page in Local mode', async () => {
    auth.gateway.mode = 'local'

    await router.push('/login')

    expect(router.currentRoute.value.name).toBe('cages')
    expect(auth.gateway.restoreSession).not.toHaveBeenCalled()
  })

  it('allows a pure Project Viewer to use the scoped cage page', async () => {
    auth.gateway.restoreSession.mockResolvedValue({
      user: {
        id: 'viewer-1', labId: 'lab-1', displayName: 'Viewer', labRoles: [],
        projectRoles: [{ projectId: 'project-1', role: 'viewer' }],
        authentication: 'session', mustChangePassword: false, isEnvironmentRoot: false,
      },
      csrfAvailable: true,
    })

    await router.push('/cages?project-login=1')

    expect(router.currentRoute.value.name).toBe('cages')
    expect(currentProjectId.value).toBe('project-1')
  })

  it('keeps Lab operators on the Registry while accepting an allowed project deep link', async () => {
    auth.gateway.restoreSession.mockResolvedValue({
      user: {
        id: 'manager-1', labId: 'lab-1', displayName: 'Manager',
        labRoles: ['animal_manager'],
        projectRoles: [{ projectId: 'project-1', role: 'viewer' }],
        authentication: 'session', mustChangePassword: false, isEnvironmentRoot: false,
      },
      csrfAvailable: true,
    })

    await router.push('/animals?project_id=project-1')

    expect(router.currentRoute.value.name).toBe('animals')
    expect(currentProjectId.value).toBe('project-1')
  })

  it('holds a forced-password user on the dedicated route until the session is ready', async () => {
    auth.gateway.restoreSession.mockResolvedValue({
      user: {
        id: 'forced-1', labId: 'lab-1', displayName: 'Forced User',
        labRoles: ['animal_manager'], projectRoles: [], authentication: 'session',
        mustChangePassword: true, isEnvironmentRoot: false,
      },
      csrfAvailable: true,
    })

    await router.push('/animals?project-login=1')

    expect(router.currentRoute.value.name).toBe('change-password')
    expect(router.currentRoute.value.query.redirect).toBe('/animals?project-login=1')
    await router.push('/settings')
    expect(router.currentRoute.value.name).toBe('change-password')

    auth.gateway.restoreSession.mockResolvedValue({
      user: {
        id: 'forced-1', labId: 'lab-1', displayName: 'Forced User',
        labRoles: ['animal_manager'], projectRoles: [], authentication: 'session',
        mustChangePassword: false, isEnvironmentRoot: false,
      },
      csrfAvailable: true,
    })
    await router.push('/animals')
    expect(router.currentRoute.value.name).toBe('animals')
  })

  it('requires the no-password local welcome once per sessionStorage lifetime', async () => {
    auth.gateway.mode = 'local'
    auth.gateway.requiresLocalWelcome = true
    sessionStorage.clear()

    await router.push('/animals')

    expect(router.currentRoute.value.name).toBe('local-welcome')
    expect(router.currentRoute.value.query.redirect).toBe('/animals')
    sessionStorage.setItem('muriarc.local-space.entered.v1', 'true')
    await router.push('/animals')
    expect(router.currentRoute.value.name).toBe('animals')
    expect(auth.gateway.restoreSession).not.toHaveBeenCalled()
  })

})
