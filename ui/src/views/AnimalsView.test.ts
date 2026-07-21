import { flushPromises, mount } from '@vue/test-utils'
import { createMemoryHistory, createRouter } from 'vue-router'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import type { Animal, AnimalDetail } from '@/domain/models'
import AnimalsView from './AnimalsView.vue'

const gatewayMock = vi.hoisted(() => ({
  mode: 'remote' as const,
  displayName: '共享实验室',
  listAnimals: vi.fn(),
  listCages: vi.fn(),
  listProjects: vi.fn(),
  getAnimal: vi.fn(),
  getAnimalDetail: vi.fn(),
}))

vi.mock('@/services/gateway', () => ({ gateway: gatewayMock }))
vi.mock('naive-ui', async (importOriginal) => {
  const actual = await importOriginal<typeof import('naive-ui')>()
  return {
    ...actual,
    useMessage: () => ({ error: vi.fn() }),
  }
})

const animal: Animal = {
  id: 'animal-1',
  code: 'M-001',
  sex: 'female',
  strain: 'C57BL/6J',
  genotype: '待确认',
  birthDate: '2026-06-01',
  status: 'active',
  cageId: null,
  projectNames: ['DEMO'],
  timeline: [],
}

const detail: AnimalDetail = {
  timeline: [],
  experiments: [],
  measurements: [],
  pedigree: [],
  samples: [],
  attachments: [],
  auditVisible: true,
  audits: [],
  provenance: [],
}

function testRouter(location: string) {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [{ path: '/animals', component: AnimalsView }],
  })
  void router.push(location)
  return router
}

describe('AnimalsView remote access context', () => {
  beforeEach(() => {
    gatewayMock.listAnimals.mockReset().mockResolvedValue([animal])
    gatewayMock.listCages.mockReset().mockResolvedValue([])
    gatewayMock.listProjects.mockReset().mockResolvedValue([])
    gatewayMock.getAnimal.mockReset().mockResolvedValue(animal)
    gatewayMock.getAnimalDetail.mockReset().mockResolvedValue(detail)
  })

  it('forwards the route project_id when hydrating an animal detail', async () => {
    const projectId = 'ed8f6474-a192-4f1e-bb5c-51032ca94c80'
    const router = testRouter(`/animals?animal=${animal.id}&project_id=${projectId}`)
    await router.isReady()

    mount(AnimalsView, {
      global: {
        plugins: [router],
        stubs: {
          PageHeader: true,
          NButton: true,
          NDataTable: true,
          NDrawer: { template: '<div><slot /></div>' },
          NDrawerContent: { template: '<div><slot name="header" /><slot /></div>' },
          NEmpty: true,
          NInput: true,
          NSelect: true,
          NTabPane: { template: '<div><slot /></div>' },
          NTabs: { template: '<div><slot /></div>' },
          NTag: { template: '<span><slot /></span>' },
          NTimeline: { template: '<div><slot /></div>' },
          NTimelineItem: { template: '<div><slot /></div>' },
        },
      },
    })
    await flushPromises()

    expect(gatewayMock.getAnimal).toHaveBeenCalledWith(animal.id, { projectId })
    expect(gatewayMock.getAnimalDetail).toHaveBeenCalledWith(animal.id, { projectId })
  })
})
