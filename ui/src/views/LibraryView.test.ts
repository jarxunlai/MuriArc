import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { LibraryRecord } from '@/services/gateway'
import { currentProjectId } from '@/services/projectContext'
import LibraryView from './LibraryView.vue'

const gatewayMock = vi.hoisted(() => ({
  listProjects: vi.fn(),
  listLibrary: vi.fn(),
  deleteAttachment: vi.fn(),
}))

vi.mock('@/services/gateway', () => ({ gateway: gatewayMock }))
vi.mock('naive-ui', async (importOriginal) => {
  const actual = await importOriginal<typeof import('naive-ui')>()
  return { ...actual, useMessage: () => ({ error: vi.fn(), success: vi.fn(), warning: vi.fn() }) }
})

const libraryEntry: LibraryRecord = {
  attachment: {
    id: 'attachment-1',
    projectId: 'project-1',
    entityType: 'project',
    entityId: 'project-1',
    fileName: 'gel.png',
    mediaType: 'image/png',
    sizeBytes: 1024,
    sha256: 'a'.repeat(64),
    version: 1,
    revision: 4,
    contentHref: '/attachments/attachment-1/content',
    previewHref: '/attachments/attachment-1/preview',
    previewSupported: true,
    createdAt: '2026-07-21T00:00:00Z',
  },
  links: [],
  derivatives: [],
  status: 'ready',
}

const stubs = {
  PageHeader: { template: '<header><slot name="actions" /></header>' },
  NButton: { template: '<button><slot name="icon" /><slot /></button>' },
  NSelect: true,
  NTag: { template: '<span><slot /></span>' },
  NPopconfirm: { template: '<span><slot name="trigger" /><slot /></span>' },
  NEmpty: true,
}

describe('LibraryView', () => {
  beforeEach(() => {
    currentProjectId.value = 'project-1'
    gatewayMock.listProjects.mockReset().mockResolvedValue([
      { id: 'project-1', name: 'Project one' },
      { id: 'project-2', name: 'Project two' },
    ])
    gatewayMock.listLibrary.mockReset().mockResolvedValue([libraryEntry])
    gatewayMock.deleteAttachment.mockReset().mockResolvedValue(libraryEntry.attachment)
  })

  it('reloads project scoped records and clears upload status when switching projects', async () => {
    const wrapper = mount(LibraryView, { global: { stubs } })
    await flushPromises()

    expect(gatewayMock.listLibrary).toHaveBeenCalledWith('project-1')
    ;(wrapper.vm as unknown as { uploads: Array<{ name: string; status: string }> }).uploads = [
      { name: 'old.pdf', status: '完成' },
    ]
    await (wrapper.vm as unknown as { changeProject: (value: string) => Promise<void> })
      .changeProject('project-2')

    expect(gatewayMock.listLibrary).toHaveBeenLastCalledWith('project-2')
    expect((wrapper.vm as unknown as { uploads: unknown[] }).uploads).toEqual([])
  })

  it('deletes a library attachment with revision protection', async () => {
    const wrapper = mount(LibraryView, { global: { stubs } })
    await flushPromises()

    await (wrapper.vm as unknown as { deleteEntry: (entry: LibraryRecord) => Promise<void> })
      .deleteEntry(libraryEntry)

    expect(gatewayMock.deleteAttachment).toHaveBeenCalledWith({
      id: 'attachment-1',
      expectedRevision: 4,
      reason: 'project library deletion',
    })
    expect(gatewayMock.listLibrary).toHaveBeenLastCalledWith('project-1')
  })
})
