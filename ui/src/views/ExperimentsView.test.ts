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
  listAiModelProfiles: vi.fn(),
  getAiModelDefaults: vi.fn(),
  listAiExtractions: vi.fn(),
  uploadPrivateImage: vi.fn(),
  createAiExtraction: vi.fn(),
  approveAiExtraction: vi.fn(),
  rejectAiExtraction: vi.fn(),
}))

const messageMock = vi.hoisted(() => ({
  error: vi.fn(),
  success: vi.fn(),
  warning: vi.fn(),
}))

vi.mock('@/services/gateway', () => ({ gateway: gatewayMock }))
vi.mock('naive-ui', async (importOriginal) => {
  const actual = await importOriginal<typeof import('naive-ui')>()
  return { ...actual, useMessage: () => messageMock }
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

function dataCellFixture() {
  const participation = {
    id: 'participation-1',
    projectId: 'project-1',
    experimentId: 'experiment-1',
    animalId: 'animal-1',
    status: 'enrolled' as const,
    enrolledAt: '2026-07-18T01:00:00Z',
    genotypeSnapshot: [],
    revision: 1,
  }
  const event = {
    id: 'event-1',
    labId: 'lab-1',
    projectId: 'project-1',
    experimentId: 'experiment-1',
    eventKey: 'day_1',
    label: '第 1 天',
    occurredAt: '2026-07-18T01:00:00Z',
    details: {},
    revision: 1,
  }
  const definition = {
    id: 'definition-1',
    labId: 'lab-1',
    projectId: 'project-1',
    experimentId: 'experiment-1',
    key: 'body_weight',
    label: '体重',
    valueType: 'number' as const,
    unit: 'g',
    categories: [],
    policy: 'versioned' as const,
    revision: 1,
  }
  const profile = {
    id: 'vision-1',
    name: '视觉模型',
    currentVersion: 3,
    revision: 1,
    protocol: 'openai_responses' as const,
    transport: 'open_ai_compatible' as const,
    baseUrl: 'https://provider.example/v1',
    modelId: 'vision-model',
    supportsVision: true,
    contextWindowTokens: 131072,
    maxInputTokens: 65536,
    maxOutputTokens: 4096,
    historyTokenBudget: 32768,
    historyTurns: 20,
    temperature: 0,
    timeoutMs: 120000,
    hasKey: true,
    isDefaultConversation: false,
    isDefaultVision: true,
  }
  const uploaded = {
    image: {
      id: 'image-1',
      project_id: 'project-1',
      status: 'active' as const,
      expires_at: '2026-08-18T01:00:00Z',
      meta: { revision: 1 },
    },
    fileName: 'scale.png',
    mediaType: 'image/png',
    sizeBytes: 4,
    sha256: 'a'.repeat(64),
    retentionDays: 30,
  }
  const draft = {
    id: 'extraction-1',
    projectId: 'project-1',
    experimentId: 'experiment-1',
    experimentEventId: 'event-1',
    currentDataCell: {
      definitionId: 'definition-1',
      subjectType: 'animal' as const,
      subjectId: 'animal-1',
    },
    status: 'pending_approval',
    candidates: [{
      itemIndex: 0,
      confidence: 0.91,
      selected: false,
      sourceLabel: '秤显示',
      value: { type: 'number' as const, value: 23.8 },
    }],
    evidence: [{
      displayOrder: 0,
      privateImageId: 'image-1',
      privateAttachmentId: 'private-attachment-1',
      originalSha256: 'a'.repeat(64),
      sanitizedSha256: 'b'.repeat(64),
    }],
    modelTrace: {
      profileId: 'vision-1',
      profileVersion: 3,
      purpose: 'vision',
      inputTokens: 10,
      outputTokens: 4,
      totalTokens: 14,
    },
    revision: 1,
  }
  return { participation, event, definition, profile, uploaded, draft }
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
  NModal: {
    name: 'NModal',
    props: ['show', 'closable', 'maskClosable', 'closeOnEsc'],
    template: '<div v-if="show"><slot /><slot name="footer" /></div>',
  },
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
    gatewayMock.listAiModelProfiles.mockReset().mockResolvedValue([])
    gatewayMock.getAiModelDefaults.mockReset().mockResolvedValue({ revision: 1 })
    gatewayMock.listAiExtractions.mockReset().mockResolvedValue([])
    gatewayMock.uploadPrivateImage.mockReset()
    gatewayMock.createAiExtraction.mockReset()
    gatewayMock.approveAiExtraction.mockReset()
    gatewayMock.rejectAiExtraction.mockReset()
    messageMock.error.mockReset()
    messageMock.success.mockReset()
    messageMock.warning.mockReset()
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

  it('keeps image recognition bound to one current data cell until human approval', async () => {
    currentAuthSession.value = session('editor')
    routerMock.currentRoute.value = {
      params: { experimentId: 'experiment-1', section: 'data' },
      query: {},
    }
    const participation = {
      id: 'participation-1',
      projectId: 'project-1',
      experimentId: 'experiment-1',
      animalId: 'animal-1',
      status: 'enrolled',
      enrolledAt: '2026-07-18T01:00:00Z',
      genotypeSnapshot: [],
      revision: 1,
    }
    const event = {
      id: 'event-1',
      labId: 'lab-1',
      projectId: 'project-1',
      experimentId: 'experiment-1',
      eventKey: 'day_1',
      label: '第 1 天',
      occurredAt: '2026-07-18T01:00:00Z',
      details: {},
      revision: 1,
    }
    const definition = {
      id: 'definition-1',
      labId: 'lab-1',
      projectId: 'project-1',
      experimentId: 'experiment-1',
      key: 'body_weight',
      label: '体重',
      valueType: 'number',
      unit: 'g',
      categories: [],
      policy: 'versioned',
      revision: 1,
    }
    gatewayMock.listAnimals.mockResolvedValueOnce([{
      id: 'animal-1',
      code: 'M-001',
      strain: 'C57BL/6J',
      sex: 'female',
      status: 'experiment',
    }])
    gatewayMock.listParticipations.mockResolvedValueOnce([participation])
    gatewayMock.listExperimentEvents.mockResolvedValueOnce([event])
    gatewayMock.listObservationDefinitions.mockResolvedValueOnce([definition])
    gatewayMock.listAiModelProfiles.mockResolvedValueOnce([{
      id: 'vision-1',
      name: '视觉模型',
      currentVersion: 3,
      revision: 1,
      protocol: 'openai_responses',
      transport: 'open_ai_compatible',
      baseUrl: 'https://provider.example/v1',
      modelId: 'vision-model',
      supportsVision: true,
      contextWindowTokens: 131072,
      maxInputTokens: 65536,
      maxOutputTokens: 4096,
      historyTokenBudget: 32768,
      historyTurns: 20,
      temperature: 0,
      timeoutMs: 120000,
      hasKey: true,
      isDefaultConversation: false,
      isDefaultVision: true,
    }])
    gatewayMock.getAiModelDefaults.mockResolvedValueOnce({
      defaultVisionProfileId: 'vision-1',
      revision: 2,
    })
    gatewayMock.uploadPrivateImage.mockResolvedValueOnce({
      image: {
        id: 'image-1',
        project_id: 'project-1',
        status: 'active',
        expires_at: '2026-08-18T01:00:00Z',
        meta: { revision: 1 },
      },
      fileName: 'scale.png',
      mediaType: 'image/png',
      sizeBytes: 4,
      sha256: 'a'.repeat(64),
      retentionDays: 30,
    })
    gatewayMock.createAiExtraction.mockResolvedValueOnce({
      id: 'extraction-1',
      projectId: 'project-1',
      experimentId: 'experiment-1',
      experimentEventId: 'event-1',
      currentDataCell: {
        definitionId: 'definition-1',
        subjectType: 'animal',
        subjectId: 'animal-1',
      },
      status: 'pending_approval',
      candidates: [{
        itemIndex: 0,
        confidence: 0.91,
        selected: false,
        sourceLabel: '秤显示',
        value: { type: 'number', value: 23.8 },
      }],
      evidence: [{
        displayOrder: 0,
        privateImageId: 'image-1',
        privateAttachmentId: 'private-attachment-1',
        originalSha256: 'a'.repeat(64),
        sanitizedSha256: 'b'.repeat(64),
      }],
      modelTrace: {
        profileId: 'vision-1',
        profileVersion: 3,
        purpose: 'vision',
        inputTokens: 10,
        outputTokens: 4,
        totalTokens: 14,
      },
      revision: 1,
    })
    gatewayMock.approveAiExtraction.mockResolvedValueOnce({
      draft: { id: 'extraction-1', status: 'approved' },
      observations: [{
        id: 'observation-1',
        projectId: 'project-1',
        experimentId: 'experiment-1',
        experimentEventId: 'event-1',
        definitionId: 'definition-1',
        subjectType: 'animal',
        subjectId: 'animal-1',
        currentValueVersion: 1,
        revision: 1,
      }],
      attachments: [],
      links: [],
    })
    const createPreview = vi.spyOn(URL, 'createObjectURL').mockReturnValue('blob:data-entry')
    const revokePreview = vi.spyOn(URL, 'revokeObjectURL').mockImplementation(() => undefined)
    const wrapper = mount(ExperimentsView, { global: { stubs } })
    await flushPromises()
    const state = (wrapper.vm as any).$?.setupState as Record<string, any>

    state.editDataCell(participation, event, definition)
    await flushPromises()
    state.stageDataEntryImages({
      target: {
        files: [new File(['data'], 'scale.png', { type: 'image/png' })],
        value: 'selected',
      },
    })
    await state.generateExtractionCandidate()

    expect(gatewayMock.createAiExtraction).toHaveBeenCalledWith({
      imageIds: ['image-1'],
      projectId: 'project-1',
      experimentId: 'experiment-1',
      experimentEventId: 'event-1',
      currentDataCell: {
        definitionId: 'definition-1',
        subjectType: 'animal',
        subjectId: 'animal-1',
      },
      visionModelProfileId: 'vision-1',
    })
    expect(gatewayMock.approveAiExtraction).not.toHaveBeenCalled()

    state.observationValueForm.numberValue = 24.1
    state.aiCandidateNotes = '人工核对秤显示'
    state.aiApprovalConfirmed = true
    gatewayMock.listObservationValues.mockRejectedValueOnce(new Error('refresh unavailable'))
    await state.approveExtractionCandidate()

    expect(gatewayMock.approveAiExtraction).toHaveBeenCalledWith('extraction-1', {
      expectedRevision: 1,
      selections: [{
        itemIndex: 0,
        value: { type: 'number', value: 24.1 },
        notes: '人工核对秤显示',
      }],
    })
    expect(gatewayMock.approveAiExtraction.mock.calls[0][1]).not.toHaveProperty('projectId')
    expect(gatewayMock.approveAiExtraction.mock.calls[0][1]).not.toHaveProperty('currentDataCell')
    expect(state.showObservation).toBe(false)
    expect(messageMock.success).toHaveBeenCalledWith(
      '已由人工批准并原子写入 Observation、附件、Audit 与 Provenance',
    )
    expect(messageMock.warning).toHaveBeenCalledWith(
      '数据已正式写入，但最新观察值刷新失败；重新打开实验即可同步',
    )
    expect(state.dataEntryImageError).toBe('')
    wrapper.unmount()
    expect(revokePreview).toHaveBeenCalledWith('blob:data-entry')
    createPreview.mockRestore()
    revokePreview.mockRestore()
  })

  it('rejects data-entry images above 10 MiB before upload or Provider work', async () => {
    currentAuthSession.value = session('editor')
    const { participation, event, definition, profile } = dataCellFixture()
    routerMock.currentRoute.value = {
      params: { experimentId: experiment.id, section: 'data' },
      query: {},
    }
    gatewayMock.listAnimals.mockResolvedValueOnce([{
      id: 'animal-1',
      code: 'M-001',
      strain: 'C57BL/6J',
      sex: 'female',
      status: 'experiment',
    }])
    gatewayMock.listParticipations.mockResolvedValueOnce([participation])
    gatewayMock.listExperimentEvents.mockResolvedValueOnce([event])
    gatewayMock.listObservationDefinitions.mockResolvedValueOnce([definition])
    gatewayMock.listAiModelProfiles.mockResolvedValueOnce([profile])
    gatewayMock.getAiModelDefaults.mockResolvedValueOnce({
      defaultVisionProfileId: profile.id,
      revision: 2,
    })
    const wrapper = mount(ExperimentsView, { global: { stubs } })
    await flushPromises()
    const state = (wrapper.vm as any).$?.setupState as Record<string, any>
    state.editDataCell(participation, event, definition)
    await flushPromises()
    const oversized = new File(['x'], 'oversized.png', { type: 'image/png' })
    Object.defineProperty(oversized, 'size', { value: 10 * 1024 * 1024 + 1 })

    state.stageDataEntryImages({
      target: { files: [oversized], value: 'selected' },
    })

    expect(state.dataEntryImages).toHaveLength(0)
    expect(state.dataEntryImageError).toContain('10 MiB')
    expect(gatewayMock.uploadPrivateImage).not.toHaveBeenCalled()
    expect(gatewayMock.createAiExtraction).not.toHaveBeenCalled()
    wrapper.unmount()
  })

  it('blocks modal closure while busy and cancels stale upload writeback before draft creation', async () => {
    currentAuthSession.value = session('editor')
    const { participation, event, definition, profile, uploaded } = dataCellFixture()
    routerMock.currentRoute.value = {
      params: { experimentId: experiment.id, section: 'data' },
      query: {},
    }
    gatewayMock.listAnimals.mockResolvedValueOnce([{
      id: 'animal-1',
      code: 'M-001',
      strain: 'C57BL/6J',
      sex: 'female',
      status: 'experiment',
    }])
    gatewayMock.listParticipations.mockResolvedValueOnce([participation])
    gatewayMock.listExperimentEvents.mockResolvedValueOnce([event])
    gatewayMock.listObservationDefinitions.mockResolvedValueOnce([definition])
    gatewayMock.listAiModelProfiles.mockResolvedValueOnce([profile])
    gatewayMock.getAiModelDefaults.mockResolvedValueOnce({
      defaultVisionProfileId: profile.id,
      revision: 2,
    })
    let finishUpload!: (value: typeof uploaded) => void
    gatewayMock.uploadPrivateImage.mockReturnValueOnce(new Promise((resolve) => {
      finishUpload = resolve
    }))
    const createPreview = vi.spyOn(URL, 'createObjectURL').mockReturnValue('blob:busy-entry')
    const revokePreview = vi.spyOn(URL, 'revokeObjectURL').mockImplementation(() => undefined)
    const wrapper = mount(ExperimentsView, { global: { stubs } })
    await flushPromises()
    const state = (wrapper.vm as any).$?.setupState as Record<string, any>
    state.editDataCell(participation, event, definition)
    await flushPromises()
    state.stageDataEntryImages({
      target: {
        files: [new File(['data'], 'scale.png', { type: 'image/png' })],
        value: 'selected',
      },
    })

    const pending = state.generateExtractionCandidate()
    await Promise.resolve()
    expect(state.dataEntryAiBusy).toBe(true)
    const dataEntryModal = wrapper.findAllComponents({ name: 'NModal' })
      .find((modal) => modal.text().includes('当前数据单元由采集节点'))
    expect(dataEntryModal?.props('closable')).toBe(false)

    state.showObservation = false
    await flushPromises()
    expect(state.showObservation).toBe(true)
    expect(state.dataEntryImages).toHaveLength(1)

    state.resetDataEntryAiState()
    finishUpload(uploaded)
    await pending
    expect(gatewayMock.createAiExtraction).not.toHaveBeenCalled()
    expect(state.extractionDraft).toBeNull()
    expect(state.dataEntryImageError).toBe('')
    expect(revokePreview).toHaveBeenCalledWith('blob:busy-entry')
    wrapper.unmount()
    createPreview.mockRestore()
    revokePreview.mockRestore()
  })

  it('restores and explicitly rejects an exact-cell pending draft', async () => {
    currentAuthSession.value = session('editor')
    const { participation, event, definition, profile, draft } = dataCellFixture()
    routerMock.currentRoute.value = {
      params: { experimentId: experiment.id, section: 'data' },
      query: {},
    }
    gatewayMock.listAnimals.mockResolvedValueOnce([{
      id: 'animal-1',
      code: 'M-001',
      strain: 'C57BL/6J',
      sex: 'female',
      status: 'experiment',
    }])
    gatewayMock.listParticipations.mockResolvedValueOnce([participation])
    gatewayMock.listExperimentEvents.mockResolvedValueOnce([event])
    gatewayMock.listObservationDefinitions.mockResolvedValueOnce([definition])
    gatewayMock.listAiModelProfiles.mockResolvedValueOnce([profile])
    gatewayMock.getAiModelDefaults.mockResolvedValueOnce({
      defaultVisionProfileId: profile.id,
      revision: 2,
    })
    gatewayMock.listAiExtractions.mockResolvedValueOnce([draft])
    gatewayMock.rejectAiExtraction.mockResolvedValueOnce({
      ...draft,
      status: 'rejected',
      revision: 2,
    })
    const wrapper = mount(ExperimentsView, { global: { stubs } })
    await flushPromises()
    const state = (wrapper.vm as any).$?.setupState as Record<string, any>
    state.editDataCell(participation, event, definition)
    await flushPromises()

    expect(state.extractionDraft?.id).toBe(draft.id)
    expect(state.dataEntryMode).toBe('ai')
    expect(state.observationValueForm.numberValue).toBe(23.8)
    await state.rejectExtractionCandidate()

    expect(gatewayMock.rejectAiExtraction).toHaveBeenCalledWith(draft.id, {
      expectedRevision: 1,
    })
    expect(state.extractionDraft).toBeNull()
    expect(messageMock.success).toHaveBeenCalledWith(
      '已放弃候选并释放全部私人暂存图片',
    )
    wrapper.unmount()
  })


})
