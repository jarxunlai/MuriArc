import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { AuthSession, Experiment, ProjectRole } from '@/domain/models'
import { currentAuthSession, currentProjectId } from '@/services/projectContext'
import ExperimentsView from './ExperimentsView.vue'

const routerMock = vi.hoisted(() => ({
  currentRoute: { value: { params: {}, query: {} } },
  push: vi.fn(),
  replace: vi.fn(),
}))

vi.mock('@/router', () => ({ router: routerMock }))

const gatewayMock = vi.hoisted(() => ({
  mode: 'remote' as const,
  listExperiments: vi.fn(),
  listProjects: vi.fn(),
  listPublishedTemplates: vi.fn(),
  listAnimals: vi.fn(),
  listGenotypeDefinitions: vi.fn(),
  listCohorts: vi.fn(),
  listParticipations: vi.fn(),
  listProcedures: vi.fn(),
  listExperimentEvents: vi.fn(),
  listObservationDefinitions: vi.fn(),
  listObservations: vi.fn(),
  listObservationValues: vi.fn(),
}))

vi.mock('@/services/gateway', () => ({ gateway: gatewayMock }))
vi.mock('naive-ui', async (importOriginal) => {
  const actual = await importOriginal<typeof import('naive-ui')>()
  return { ...actual, useMessage: () => ({ error: vi.fn(), success: vi.fn() }) }
})

const experiment: Experiment = {
  id: 'experiment-1',
  projectId: 'project-1',
  code: 'EXP-001',
  name: 'Project experiment',
  project: 'Project one',
  status: 'active',
  startDate: '2026-07-19',
  animalCount: 2,
  completedSteps: 0,
  totalSteps: 1,
  groups: [],
  revision: 1,
}

function session(role: ProjectRole): AuthSession {
  return {
    user: {
      id: `user-${role}`, labId: 'lab-1', displayName: role, labRoles: [],
      projectRoles: [{ projectId: 'project-1', role }], authentication: 'session', mustChangePassword: false, isEnvironmentRoot: false,
    },
    csrfAvailable: true,
  }
}

const stubs = {
  PageHeader: { template: '<header><slot name="actions" /></header>' },
  NButton: { template: '<button><slot /></button>' },
  NDrawer: { props: ['show'], template: '<div v-if="show"><slot /></div>' },
  NDrawerContent: { template: '<div><slot /></div>' },
  NModal: { props: ['show'], template: '<div v-if="show"><slot /><slot name="footer" /></div>' },
  NPopconfirm: { template: '<span><slot name="trigger" /><slot /></span>' },
  NSpin: { template: '<div><slot /></div>' },
  NTag: { template: '<span><slot /></span>' },
  NProgress: true,
  NRadioGroup: { template: '<div><slot /></div>' },
  NRadioButton: { template: '<button><slot /></button>' },
  NEmpty: true,
}

describe('ExperimentsView project roles', () => {
  beforeEach(() => {
    currentProjectId.value = 'project-1'
    gatewayMock.listExperiments.mockReset().mockResolvedValue([experiment])
    gatewayMock.listProjects.mockReset().mockResolvedValue([{ id: 'project-1', name: 'Project one' }])
    gatewayMock.listPublishedTemplates.mockReset().mockResolvedValue([
      { id: 'template-1', name: 'Template one', version: 1 },
    ])
    gatewayMock.listAnimals.mockReset().mockResolvedValue([])
    gatewayMock.listGenotypeDefinitions.mockReset().mockResolvedValue([])
    gatewayMock.listCohorts.mockReset().mockResolvedValue([])
    gatewayMock.listParticipations.mockReset().mockResolvedValue([])
    gatewayMock.listProcedures.mockReset().mockResolvedValue([])
    gatewayMock.listExperimentEvents.mockReset().mockResolvedValue([])
    gatewayMock.listObservationDefinitions.mockReset().mockResolvedValue([])
    gatewayMock.listObservations.mockReset().mockResolvedValue([])
    gatewayMock.listObservationValues.mockReset().mockResolvedValue([])
    routerMock.push.mockReset()
    routerMock.replace.mockReset()
    routerMock.currentRoute.value = { params: {}, query: {} }
  })

  it('renders the scoped list read-only for a Project Viewer', async () => {
    currentAuthSession.value = session('viewer')
    const wrapper = mount(ExperimentsView, { global: { stubs } })
    await flushPromises()

    expect(gatewayMock.listExperiments).toHaveBeenCalledOnce()
    expect(wrapper.text()).toContain('Project experiment')
    expect(wrapper.text()).not.toContain('创建实验')
  })

  it('renders the same scoped list with write actions for a Project Editor', async () => {
    currentAuthSession.value = session('editor')
    const wrapper = mount(ExperimentsView, { global: { stubs } })
    await flushPromises()

    expect(gatewayMock.listExperiments).toHaveBeenCalledOnce()
    expect(wrapper.text()).toContain('Project experiment')
    expect(wrapper.text()).toContain('创建实验')
  })

  it('opens an experiment in the independent full-page workspace', async () => {
    currentAuthSession.value = session('editor')
    const wrapper = mount(ExperimentsView, { global: { stubs } })
    await flushPromises()

    const openButton = wrapper.findAll('button').find((button) => button.text() === '打开实验')
    expect(openButton).toBeTruthy()
    await openButton!.trigger('click')

    expect(routerMock.push).toHaveBeenCalledWith({
      name: 'experiment-detail',
      params: { experimentId: 'experiment-1', section: 'overview' },
      query: {},
    })
  })

  it('loads the full-page experiment workspace from a deep link', async () => {
    currentAuthSession.value = session('editor')
    routerMock.currentRoute.value = {
      params: { experimentId: 'experiment-1', section: 'overview' },
      query: {},
    }
    const wrapper = mount(ExperimentsView, { global: { stubs } })
    await flushPromises()

    expect(gatewayMock.listCohorts).toHaveBeenCalledWith('experiment-1')
    expect(gatewayMock.listObservations).toHaveBeenCalledWith({ experimentId: 'experiment-1' })
    expect(wrapper.text()).toContain('返回实验列表')
    expect(wrapper.text()).toContain('下一步实验执行')
    expect(wrapper.text()).toContain('数据工作表')
  })


})
