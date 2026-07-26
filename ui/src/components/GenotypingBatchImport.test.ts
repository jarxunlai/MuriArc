import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import GenotypingBatchImport from './GenotypingBatchImport.vue'

const gatewayMock = vi.hoisted(() => ({
  createGenotypingBatch: vi.fn(),
  previewGenotypingBatch: vi.fn(),
  commitGenotypingBatch: vi.fn(),
  cancelGenotypingBatch: vi.fn(),
  listGenotypingBatches: vi.fn(),
  getGenotypingBatch: vi.fn(),
  listGenotypeDefinitions: vi.fn(),
  listAttachments: vi.fn(),
  uploadAttachment: vi.fn(),
  deleteAttachment: vi.fn(),
}))

const messageMock = vi.hoisted(() => ({
  success: vi.fn(),
  warning: vi.fn(),
  error: vi.fn(),
}))

vi.mock('@/services/gateway', () => ({ gateway: gatewayMock }))
vi.mock('naive-ui', async (importOriginal) => {
  const actual = await importOriginal<typeof import('naive-ui')>()
  return { ...actual, useMessage: () => messageMock }
})

const draft = {
  id: 'batch-1',
  projectId: 'project-1',
  batchNumber: 'PCR-20260725-01',
  genotypeDefinitionId: 'definition-1',
  assessedAt: '2026-07-25T08:00:00.000Z',
  method: 'PCR + 凝胶电泳',
  status: 'draft',
  revision: 1,
  createdAt: '2026-07-25T08:00:00.000Z',
  updatedAt: '2026-07-25T08:00:00.000Z',
} as const

const previewedDraft = {
  ...draft,
  sourceAttachmentId: 'table-1',
  previewHash: 'preview-hash',
  previewRowCount: 1,
  revision: 2,
}

const tableAttachment = {
  id: 'table-1',
  projectId: 'project-1',
  entityType: 'genotyping_batch',
  entityId: 'batch-1',
  fileName: 'results.csv',
  mediaType: 'text/csv',
  sizeBytes: 43,
  sha256: 'a'.repeat(64),
  version: 1,
  revision: 1,
  contentHref: '/attachments/table-1/content',
  previewSupported: false,
  createdAt: '2026-07-25T08:00:00.000Z',
}

const gelAttachment = {
  ...tableAttachment,
  id: 'gel-1',
  fileName: 'gel-01.png',
  mediaType: 'image/png',
  sizeBytes: 3,
  sha256: 'b'.repeat(64),
  contentHref: '/attachments/gel-1/content',
}

const acceptedRow = {
  sourceRow: 2,
  animalId: 'animal-1',
  displayId: 'M-001',
  state: 'confirmed',
  notes: '条带清晰',
} as const

const stubs = {
  NAlert: { template: '<div><slot /><slot name="action" /></div>' },
  NButton: {
    props: ['disabled', 'loading'],
    emits: ['click'],
    template: '<button :disabled="disabled" @click="$emit(\'click\')"><slot name="icon" /><slot /></button>',
  },
  NDatePicker: { template: '<input type="datetime-local">' },
  NEmpty: { template: '<div><slot /></div>' },
  NForm: { template: '<form><slot /></form>' },
  NFormItem: { template: '<label><slot /></label>' },
  NInput: { template: '<input>' },
  NSelect: {
    props: ['value', 'options'],
    emits: ['update:value'],
    template: `
      <select data-testid="definition" :value="value ?? ''"
        @change="$emit('update:value', $event.target.value)">
        <option value=""></option>
        <option v-for="option in options" :key="option.value" :value="option.value">{{ option.label }}</option>
      </select>
    `,
  },
  NTag: { template: '<span><slot /></span>' },
}

function mountComponent() {
  return mount(GenotypingBatchImport, {
    props: { projectId: 'project-1' },
    global: { stubs },
  })
}

async function chooseFiles(wrapper: ReturnType<typeof mountComponent>) {
  await wrapper.get('[data-testid="definition"]').setValue('definition-1')
  const [resultInput, gelInput] = wrapper.findAll<HTMLInputElement>('input[type="file"]')
  const result = new File(
    ['animal_code,state,notes\nM-001,confirmed,条带清晰'],
    'results.csv',
    { type: 'text/csv' },
  )
  const gel = new File(['gel'], 'gel-01.png', { type: 'image/png' })
  Object.defineProperty(resultInput.element, 'files', { configurable: true, value: [result] })
  Object.defineProperty(gelInput.element, 'files', { configurable: true, value: [gel] })
  await resultInput.trigger('change')
  await gelInput.trigger('change')
}

function button(wrapper: ReturnType<typeof mountComponent>, label: string) {
  const match = wrapper.findAll('button').find((item) => item.text().trim() === label)
  if (!match) throw new Error(`button not found: ${label}`)
  return match
}

describe('GenotypingBatchImport', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    vi.stubGlobal('URL', {
      ...URL,
      createObjectURL: vi.fn(() => 'blob:gel-preview'),
      revokeObjectURL: vi.fn(),
    })
    gatewayMock.listGenotypeDefinitions.mockResolvedValue([{
      id: 'definition-1',
      name: 'Cre/loxP',
      components: [],
      revision: 1,
      createdAt: '2026-07-25T08:00:00.000Z',
      updatedAt: '2026-07-25T08:00:00.000Z',
    }])
    gatewayMock.listGenotypingBatches.mockResolvedValue([])
    gatewayMock.getGenotypingBatch.mockResolvedValue({ batch: draft, records: [] })
    gatewayMock.createGenotypingBatch.mockResolvedValue(draft)
    gatewayMock.uploadAttachment.mockImplementation(async (input: { mediaType?: string }) =>
      input.mediaType?.startsWith('image/') ? gelAttachment : tableAttachment)
    gatewayMock.listAttachments.mockResolvedValue([tableAttachment, gelAttachment])
    gatewayMock.previewGenotypingBatch.mockResolvedValue({
      batch: previewedDraft,
      preview: {
        totalRows: 1,
        acceptedRows: [acceptedRow],
        issues: [],
        previewHash: 'preview-hash',
      },
    })
    gatewayMock.commitGenotypingBatch.mockResolvedValue({
      batch: {
        ...previewedDraft,
        status: 'committed',
        committedAt: '2026-07-25T08:05:00.000Z',
        revision: 3,
      },
      records: [{
        id: 'record-1',
        projectId: 'project-1',
        animalId: 'animal-1',
        genotypeDefinitionId: 'definition-1',
        state: 'confirmed',
        assessedAt: draft.assessedAt,
        revision: 1,
        createdAt: draft.createdAt,
        updatedAt: draft.updatedAt,
      }],
    })
  })

  it('binds a result table and multiple evidence files before an atomic commit', async () => {
    const wrapper = mountComponent()
    await flushPromises()
    await chooseFiles(wrapper)

    await button(wrapper, '创建草稿、上传并生成预览').trigger('click')
    await flushPromises()

    expect(gatewayMock.createGenotypingBatch).toHaveBeenCalledWith(expect.objectContaining({
      projectId: 'project-1',
      genotypeDefinitionId: 'definition-1',
    }))
    expect(gatewayMock.uploadAttachment).toHaveBeenCalledTimes(2)
    expect(gatewayMock.previewGenotypingBatch).toHaveBeenCalledWith({
      batchId: 'batch-1',
      projectId: 'project-1',
      expectedRevision: 1,
      sourceAttachmentId: 'table-1',
    })
    expect(wrapper.text()).toContain('M-001')
    expect(wrapper.text()).toContain('条带清晰')
    expect(wrapper.text()).toContain('1 张')

    const confirm = button(wrapper, '确认并原子写入')
    expect(confirm.attributes('disabled')).toBeUndefined()
    await confirm.trigger('click')
    await flushPromises()

    expect(gatewayMock.commitGenotypingBatch).toHaveBeenCalledWith({
      batchId: 'batch-1',
      projectId: 'project-1',
      expectedRevision: 2,
      previewHash: 'preview-hash',
    })
    expect(wrapper.emitted('committed')).toHaveLength(1)
    expect(wrapper.text()).toContain('批次已提交并可追溯')
    expect(wrapper.text()).not.toContain('确认并原子写入')
  })

  it('keeps the commit action disabled when preview validation has a blocking error', async () => {
    gatewayMock.previewGenotypingBatch.mockResolvedValueOnce({
      batch: previewedDraft,
      preview: {
        totalRows: 1,
        acceptedRows: [acceptedRow],
        issues: [{
          row: 2,
          field: 'animal_code',
          severity: 'error',
          code: 'animal_not_found',
          message: '找不到动物 M-001',
        }],
        previewHash: 'blocked-preview',
      },
    })
    const wrapper = mountComponent()
    await flushPromises()
    await chooseFiles(wrapper)

    await button(wrapper, '创建草稿、上传并生成预览').trigger('click')
    await flushPromises()

    expect(wrapper.text()).toContain('1 个阻断错误')
    expect(button(wrapper, '确认并原子写入').attributes('disabled')).toBeDefined()
    expect(gatewayMock.commitGenotypingBatch).not.toHaveBeenCalled()
  })

  it('restores an unfinished draft with its stored table and gel evidence', async () => {
    gatewayMock.listGenotypingBatches.mockResolvedValueOnce([draft])
    const wrapper = mountComponent()
    await flushPromises()

    const resume = button(wrapper, '继续')
    expect(resume.attributes('disabled')).toBeUndefined()
    expect((wrapper.vm as unknown as { batch?: unknown }).batch).toBeUndefined()
    await resume.trigger('click')
    await flushPromises()

    expect(gatewayMock.getGenotypingBatch).toHaveBeenCalledWith('batch-1', 'project-1')
    expect(gatewayMock.listAttachments).toHaveBeenCalledWith({
      entityType: 'genotyping_batch',
      entityId: 'batch-1',
      projectId: 'project-1',
    })
    expect(gatewayMock.previewGenotypingBatch).toHaveBeenCalledWith({
      batchId: 'batch-1',
      projectId: 'project-1',
      expectedRevision: 1,
      sourceAttachmentId: 'table-1',
    })
    expect(wrapper.text()).toContain('M-001')
    expect(wrapper.text()).toContain('gel-01.png')
    expect(messageMock.success).toHaveBeenCalledWith('已恢复批次草稿并按当前结果表重新校验')
  })
})
