import { mount } from '@vue/test-utils'
import {
  NConfigProvider,
  NDialogProvider,
  NMessageProvider,
  NNotificationProvider,
  NSelect,
  create,
} from 'naive-ui'
import { createMemoryHistory, createRouter } from 'vue-router'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { AuthSession, ProjectRole } from '@/domain/models'
import {
  availableProjects,
  currentAuthSession as contextSession,
  currentProjectId,
} from '@/services/projectContext'

const gatewayState = vi.hoisted(() => ({
  gateway: { mode: 'remote' as const, displayName: '共享实验室' },
  currentAuthSession: { value: undefined as AuthSession | undefined },
}))

vi.mock('@/services/gateway', () => gatewayState)
vi.mock('@/components/AiDrawer.vue', () => ({ default: { template: '<div />' } }))

import App from './App.vue'

const naive = create({
  components: [
    NConfigProvider,
    NDialogProvider,
    NMessageProvider,
    NNotificationProvider,
    NSelect,
  ],
})

function projectSession(role: ProjectRole): AuthSession {
  return {
    user: {
      id: `user-${role}`,
      labId: 'lab-1',
      displayName: role,
      labRoles: [],
      projectRoles: [{ projectId: 'project-1', role }],
      authentication: 'session', mustChangePassword: false, isEnvironmentRoot: false,
    },
    csrfAvailable: true,
  }
}

afterEach(() => {
  gatewayState.currentAuthSession.value = undefined
  contextSession.value = undefined
  availableProjects.value = []
  currentProjectId.value = undefined
})

describe('project-only navigation shell', () => {
  for (const role of ['viewer', 'editor'] as const) {
    it(`shows ${role} a project selector and no Lab Registry navigation`, async () => {
      const session = projectSession(role)
      gatewayState.currentAuthSession.value = session
      contextSession.value = session
      availableProjects.value = [{ id: 'project-1', name: 'Project one' }]
      currentProjectId.value = 'project-1'
      const router = createRouter({
        history: createMemoryHistory(),
        routes: [{ path: '/animals', component: { template: '<div />' } }],
      })
      await router.push('/animals')
      await router.isReady()

      const wrapper = mount(App, { global: { plugins: [naive, router] } })

      expect(wrapper.find('[data-testid="project-switcher"]').exists()).toBe(true)
      expect(wrapper.text()).toContain('小鼠档案')
      expect(wrapper.text()).toContain('实验管理')
      expect(wrapper.text()).not.toContain('笼位视图')
      expect(wrapper.text()).not.toContain('繁育管理')
    })
  }
})
