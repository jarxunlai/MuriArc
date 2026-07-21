import { flushPromises, mount } from '@vue/test-utils'
import { createMemoryHistory, createRouter } from 'vue-router'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import type { Animal, AnimalDetail } from '@/domain/models'
import BreedingView from './BreedingView.vue'

const gatewayMock = vi.hoisted(() => ({
  mode: 'local' as const,
  listAnimals: vi.fn(),
  getAnimalDetail: vi.fn(),
  createPedigree: vi.fn(),
  listGeneLoci: vi.fn(),
  listAlleles: vi.fn(),
  listGenotypeDefinitions: vi.fn(),
  listGenotypingRecords: vi.fn(),
  listBreedingLines: vi.fn(),
  listColonies: vi.fn(),
  listBreedingPairs: vi.fn(),
  listMatingEvents: vi.fn(),
  listLitters: vi.fn(),
  listAnimalDrafts: vi.fn(),
}))

vi.mock('@/services/gateway', () => ({ gateway: gatewayMock }))
vi.mock('naive-ui', async (importOriginal) => {
  const actual = await importOriginal<typeof import('naive-ui')>()
  return {
    ...actual,
    useMessage: () => ({ error: vi.fn(), warning: vi.fn(), success: vi.fn() }),
  }
})

const animal: Animal = {
  id: 'animal-1',
  code: 'M-001',
  sex: 'female',
  strain: 'C57BL/6J',
  genotype: 'WT',
  birthDate: '2026-06-01',
  status: 'breeding',
  cageId: null,
  projectNames: ['DEMO'],
  timeline: [],
}
const detail: AnimalDetail = {
  timeline: [],
  experiments: [],
  measurements: [],
  pedigree: [{
    id: 'pedigree-1',
    direction: 'parent',
    parentType: 'mother',
    relatedAnimal: {
      id: 'animal-2', code: 'M-002', sex: 'female', strain: 'C57BL/6J', status: 'active',
    },
    revision: 1,
  }],
  samples: [],
  attachments: [],
  auditVisible: true,
  audits: [],
  provenance: [],
}

function testRouter(location: string) {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: '/breeding', name: 'breeding', component: BreedingView },
      { path: '/animals', name: 'animals', component: { template: '<div />' } },
    ],
  })
  void router.push(location)
  return router
}

describe('BreedingView pedigree access', () => {
  beforeEach(() => {
    gatewayMock.listAnimals.mockReset().mockResolvedValue([animal])
    gatewayMock.getAnimalDetail.mockReset().mockResolvedValue(detail)
    gatewayMock.createPedigree.mockReset()
    gatewayMock.listGeneLoci.mockReset().mockResolvedValue([])
    gatewayMock.listAlleles.mockReset().mockResolvedValue([])
    gatewayMock.listGenotypeDefinitions.mockReset().mockResolvedValue([])
    gatewayMock.listGenotypingRecords.mockReset().mockResolvedValue([])
    gatewayMock.listBreedingLines.mockReset().mockResolvedValue([])
    gatewayMock.listColonies.mockReset().mockResolvedValue([])
    gatewayMock.listBreedingPairs.mockReset().mockResolvedValue([])
    gatewayMock.listMatingEvents.mockReset().mockResolvedValue([])
    gatewayMock.listLitters.mockReset().mockResolvedValue([])
    gatewayMock.listAnimalDrafts.mockReset().mockResolvedValue([])
  })

  it('loads the selected animal and both-direction pedigree in project scope', async () => {
    const projectId = 'project-1'
    const router = testRouter(`/breeding?animal=${animal.id}&project_id=${projectId}`)
    await router.isReady()

    const wrapper = mount(BreedingView, {
      global: {
        plugins: [router],
        stubs: {
          PageHeader: { template: '<div><slot name="actions" /></div>' },
          NAlert: true,
          NButton: true,
          NEmpty: true,
          NForm: true,
          NFormItem: true,
          NInput: true,
          NModal: true,
          NSelect: true,
          NSkeleton: true,
          NSpin: { template: '<div><slot /></div>' },
          NTag: { template: '<span><slot /></span>' },
          NTabs: { template: '<div><slot /></div>' },
          NTabPane: { template: '<section><slot /></section>' },
        },
      },
    })
    await flushPromises()

    expect(gatewayMock.listAnimals).toHaveBeenCalledWith({ projectId })
    expect(gatewayMock.getAnimalDetail).toHaveBeenCalledWith(animal.id, { projectId })
    expect(wrapper.text()).toContain('M-001')
    expect(wrapper.text()).toContain('M-002')
    expect(wrapper.text()).toContain('父母')
  })
})
