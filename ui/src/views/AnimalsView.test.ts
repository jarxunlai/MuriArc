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
  listGeneLoci: vi.fn(),
  listGenotypes: vi.fn(),
  listGenotypeDefinitions: vi.fn(),
  listGenotypingRecords: vi.fn(),
  getGenotypingBatchForRecord: vi.fn(),
  listAttachments: vi.fn(),
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
    gatewayMock.listGeneLoci.mockReset().mockResolvedValue([])
    gatewayMock.listGenotypes.mockReset().mockResolvedValue([])
    gatewayMock.listGenotypeDefinitions.mockReset().mockResolvedValue([])
    gatewayMock.listGenotypingRecords.mockReset().mockResolvedValue([])
    gatewayMock.getGenotypingBatchForRecord.mockReset().mockResolvedValue(undefined)
    gatewayMock.listAttachments.mockReset().mockResolvedValue([])
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
          NSpin: { template: '<div><slot /></div>' },
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

  it('shows the source batch and its gel evidence for a batch-created genotype record', async () => {
    const projectId = 'ed8f6474-a192-4f1e-bb5c-51032ca94c80'
    gatewayMock.listGenotypeDefinitions.mockResolvedValue([{
      id: 'definition-1',
      name: 'Cre/loxP',
      components: [],
      revision: 1,
      createdAt: '2026-07-25T08:00:00.000Z',
      updatedAt: '2026-07-25T08:00:00.000Z',
    }])
    gatewayMock.listGenotypingRecords.mockResolvedValue([{
      id: 'record-1',
      projectId,
      animalId: animal.id,
      genotypeDefinitionId: 'definition-1',
      state: 'confirmed',
      assessedAt: '2026-07-25T08:00:00.000Z',
      method: 'PCR + 凝胶电泳',
      notes: '条带清晰',
      revision: 1,
      createdAt: '2026-07-25T08:05:00.000Z',
      updatedAt: '2026-07-25T08:05:00.000Z',
    }])
    gatewayMock.getGenotypingBatchForRecord.mockResolvedValue({
      id: 'batch-1',
      projectId,
      batchNumber: 'PCR-20260725-01',
      genotypeDefinitionId: 'definition-1',
      assessedAt: '2026-07-25T08:00:00.000Z',
      method: 'PCR + 凝胶电泳',
      status: 'committed',
      sourceAttachmentId: 'table-1',
      previewHash: 'preview-hash',
      previewRowCount: 24,
      committedAt: '2026-07-25T08:05:00.000Z',
      revision: 3,
      createdAt: '2026-07-25T08:00:00.000Z',
      updatedAt: '2026-07-25T08:05:00.000Z',
    })
    gatewayMock.listAttachments.mockResolvedValue([{
      id: 'gel-1',
      projectId,
      entityType: 'genotyping_batch',
      entityId: 'batch-1',
      fileName: 'gel-01.png',
      mediaType: 'image/png',
      sizeBytes: 1024,
      sha256: 'b'.repeat(64),
      version: 1,
      revision: 1,
      contentHref: '/attachments/gel-1/content',
      previewSupported: true,
      createdAt: '2026-07-25T08:01:00.000Z',
    }])
    const router = testRouter(`/animals?animal=${animal.id}&project_id=${projectId}`)
    await router.isReady()

    const wrapper = mount(AnimalsView, {
      global: {
        plugins: [router],
        stubs: {
          PageHeader: true,
          NAlert: { template: '<div><slot /><slot name="action" /></div>' },
          NButton: { template: '<button><slot /></button>' },
          NDataTable: true,
          NDrawer: { template: '<div><slot /></div>' },
          NDrawerContent: { template: '<div><slot name="header" /><slot /></div>' },
          NEmpty: true,
          NInput: true,
          NSelect: true,
          NSpin: { template: '<div><slot /></div>' },
          NTabPane: { template: '<div><slot /></div>' },
          NTabs: { template: '<div><slot /></div>' },
          NTag: { template: '<span><slot /></span>' },
          NTimeline: { template: '<div><slot /></div>' },
          NTimelineItem: { template: '<div><slot /></div>' },
        },
      },
    })
    await flushPromises()

    expect(gatewayMock.getGenotypingBatchForRecord).toHaveBeenCalledWith('record-1', projectId)
    expect(gatewayMock.listAttachments).toHaveBeenCalledWith({
      entityType: 'genotyping_batch',
      entityId: 'batch-1',
      projectId,
    })
    expect(wrapper.text()).toContain('来源批次 PCR-20260725-01')
    expect(wrapper.text()).toContain('24 条记录')
    expect(wrapper.text()).toContain('gel-01.png')
  })
})
