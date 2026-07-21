import { flushPromises, mount } from '@vue/test-utils'
import { create, NAlert, NButton, NForm, NFormItem, NInput } from 'naive-ui'
import { createMemoryHistory, createRouter } from 'vue-router'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import LoginView from './LoginView.vue'

const gatewayMock = vi.hoisted(() => ({
  mode: 'remote' as const,
  displayName: '共享实验室',
  login: vi.fn(),
}))

vi.mock('@/services/gateway', () => ({ gateway: gatewayMock }))

const naive = create({ components: [NAlert, NButton, NForm, NFormItem, NInput] })

describe('Remote LoginView', () => {
  beforeEach(() => {
    gatewayMock.login.mockReset().mockResolvedValue({
      user: {
        id: 'user-1', labId: 'lab-1', displayName: '研究员',
        labRoles: ['animal_manager'], projectRoles: [], authentication: 'session', mustChangePassword: false, isEnvironmentRoot: false,
      },
      csrfAvailable: true,
    })
  })

  it('submits credentials only to the gateway, clears the password, and returns to a safe route', async () => {
    const router = createRouter({
      history: createMemoryHistory(),
      routes: [
        { path: '/login', component: LoginView },
        { path: '/animals', component: { template: '<div />' } },
      ],
    })
    await router.push('/login?redirect=%2Fanimals%3Fproject_id%3Dproject-1')
    await router.isReady()
    const storage = vi.spyOn(Storage.prototype, 'setItem')
    const wrapper = mount(LoginView, { global: { plugins: [naive, router] } })

    await wrapper.get('input[autocomplete="username"]').setValue(' researcher@example.org ')
    await wrapper.get('input[autocomplete="current-password"]').setValue('not-persisted')
    await wrapper.get('form').trigger('submit')
    await flushPromises()

    expect(gatewayMock.login).toHaveBeenCalledWith({
      email: 'researcher@example.org',
      password: 'not-persisted',
    })
    expect(wrapper.get('input[autocomplete="current-password"]').element).toHaveProperty('value', '')
    expect(storage).not.toHaveBeenCalled()
    expect(router.currentRoute.value.fullPath).toBe('/animals?project_id=project-1')
    storage.mockRestore()
  })

  it('rejects an external redirect target', async () => {
    const router = createRouter({
      history: createMemoryHistory(),
      routes: [
        { path: '/login', component: LoginView },
        { path: '/cages', component: { template: '<div />' } },
      ],
    })
    await router.push('/login?redirect=https%3A%2F%2Fevil.example')
    await router.isReady()
    const wrapper = mount(LoginView, { global: { plugins: [naive, router] } })

    await wrapper.get('input[autocomplete="username"]').setValue('researcher@example.org')
    await wrapper.get('input[autocomplete="current-password"]').setValue('not-persisted')
    await wrapper.get('form').trigger('submit')
    await flushPromises()

    expect(router.currentRoute.value.fullPath).toBe('/cages')
  })

  it('sends a pure Project Viewer to the scoped animal workspace after login', async () => {
    gatewayMock.login.mockResolvedValueOnce({
      user: {
        id: 'viewer-1', labId: 'lab-1', displayName: 'Viewer', labRoles: [],
        projectRoles: [{ projectId: 'project-1', role: 'viewer' }],
        authentication: 'session', mustChangePassword: false, isEnvironmentRoot: false,
      },
      csrfAvailable: true,
    })
    const router = createRouter({
      history: createMemoryHistory(),
      routes: [
        { path: '/login', component: LoginView },
        { path: '/animals', component: { template: '<div />' } },
        { path: '/cages', component: { template: '<div />' } },
      ],
    })
    await router.push('/login')
    await router.isReady()
    const wrapper = mount(LoginView, { global: { plugins: [naive, router] } })

    await wrapper.get('input[autocomplete="username"]').setValue('viewer@example.org')
    await wrapper.get('input[autocomplete="current-password"]').setValue('not-persisted')
    await wrapper.get('form').trigger('submit')
    await flushPromises()

    expect(router.currentRoute.value.fullPath).toBe('/animals')
  })
})
