import { flushPromises, mount } from '@vue/test-utils'
import { createMemoryHistory, createRouter } from 'vue-router'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import AnimalImportGuideView from './AnimalImportGuideView.vue'

const mocks = vi.hoisted(() => ({
  gateway: { mode: 'local' as const, displayName: '本地数据' },
  dataGateway: {
    animalImportTemplateFormats: ['csv', 'xlsx'] as Array<'csv' | 'xlsx'>,
    getAnimalImportSchema: vi.fn(),
    downloadAnimalImportTemplate: vi.fn(),
  },
  message: { error: vi.fn() },
}))

vi.mock('@/services/gateway', () => ({ gateway: mocks.gateway }))
vi.mock('@/services/dataGateway', () => ({
  createDataGateway: () => mocks.dataGateway,
}))
vi.mock('naive-ui', async (importOriginal) => {
  const actual = await importOriginal<typeof import('naive-ui')>()
  return { ...actual, useMessage: () => mocks.message }
})

const schema = {
  version: 1,
  genotype_syntax: '{Locus}[allele_1]/[allele_2]',
  fields: [
    {
      key: 'display_id', label: '动物显示编号', data_type: 'string' as const, required: true,
      legal_values: [], description: '在当前编号范围内唯一。', example: 'EXAMPLE-SIRE-001',
    },
    {
      key: 'sex', label: '性别', data_type: 'enum' as const, required: false,
      legal_values: ['male', 'female', 'unknown'], description: '使用标准英文值。', example: 'male',
    },
  ],
  examples: [
    { display_id: 'EXAMPLE-SIRE-001', sex: 'male' },
    { display_id: 'EXAMPLE-DAM-001', sex: 'female' },
    { display_id: 'EXAMPLE-PUP-001', sex: 'male' },
    { display_id: 'EXAMPLE-PUP-002', sex: 'female' },
  ],
}

function createTestRouter() {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: '/animal-data', name: 'animal-data', component: { template: '<div />' } },
      { path: '/animal-data/import-guide', name: 'animal-import-guide', component: AnimalImportGuideView },
    ],
  })
  void router.push('/animal-data/import-guide')
  return router
}

const stubs = {
  PageHeader: {
    props: ['title', 'description'],
    template: '<header><h1>{{ title }}</h1><p>{{ description }}</p><slot name="actions" /></header>',
  },
  NAlert: { template: '<div><slot /></div>' },
  NButton: {
    props: ['disabled'],
    emits: ['click'],
    template: '<button :disabled="disabled" @click="$emit(\'click\')"><slot name="icon" /><slot /></button>',
  },
  NSpin: { template: '<div><slot /></div>' },
  NTag: { template: '<span><slot /></span>' },
}

async function mountGuide() {
  const router = createTestRouter()
  await router.isReady()
  const wrapper = mount(AnimalImportGuideView, {
    global: { plugins: [router], stubs },
  })
  await flushPromises()
  return wrapper
}

describe('AnimalImportGuideView', () => {
  beforeEach(() => {
    mocks.dataGateway.animalImportTemplateFormats = ['csv', 'xlsx']
    mocks.dataGateway.getAnimalImportSchema.mockReset().mockResolvedValue(schema)
    mocks.dataGateway.downloadAnimalImportTemplate.mockReset().mockResolvedValue(undefined)
    mocks.message.error.mockReset()
  })

  it('renders the production field contract and exactly four synthetic examples', async () => {
    const wrapper = await mountGuide()

    expect(mocks.dataGateway.getAnimalImportSchema).toHaveBeenCalledOnce()
    expect(wrapper.findAll('.field-card')).toHaveLength(schema.fields.length)
    expect(wrapper.findAll('.example-table-scroll tbody tr')).toHaveLength(4)
    expect(wrapper.text()).toContain('示例可编辑，但不能当作真实实验室数据直接提交')
    expect(wrapper.text()).toContain('EXAMPLE-PUP-002')
    expect(wrapper.text()).toContain('来自当前生产导入契约')
  })

  it('passes an explicit blank/example variant for all four downloads', async () => {
    const wrapper = await mountGuide()

    for (const key of ['csv-blank', 'csv-example', 'xlsx-blank', 'xlsx-example']) {
      await wrapper.get(`[data-testid="download-${key}"]`).trigger('click')
      await flushPromises()
    }

    expect(mocks.dataGateway.downloadAnimalImportTemplate.mock.calls).toEqual([
      ['csv', 'blank'],
      ['csv', 'example'],
      ['xlsx', 'blank'],
      ['xlsx', 'example'],
    ])
  })

  it('uses the gateway capability to disable XLSX and explain the demo fallback', async () => {
    mocks.dataGateway.animalImportTemplateFormats = ['csv']
    const wrapper = await mountGuide()

    expect(wrapper.get('[data-testid="download-xlsx-blank"]').attributes('disabled')).toBeDefined()
    expect(wrapper.get('[data-testid="download-xlsx-example"]').attributes('disabled')).toBeDefined()
    expect(wrapper.text()).toContain('当前运行环境仅提供 CSV 模板')
    expect(wrapper.text()).toContain('浏览器演示使用内置契约副本')

    await wrapper.get('[data-testid="download-xlsx-example"]').trigger('click')
    expect(mocks.dataGateway.downloadAnimalImportTemplate).not.toHaveBeenCalled()
  })
})
