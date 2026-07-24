<script setup lang="ts">
import { computed, onMounted, onUnmounted, reactive, ref, watch } from 'vue'
import {
  CalendarClock,
  CheckCircle2,
  Download,
  FlaskConical,
  ImagePlus,
  Plus,
  ScanSearch,
  ShieldCheck,
  Trash2,
  Upload,
  UsersRound,
} from '@lucide/vue'
import { useMessage } from 'naive-ui'
import type {
  Animal,
  Cohort,
  Experiment,
  ExperimentEvent,
  ExperimentTemplateSummary,
  Observation,
  ObservationDefinition,
  ObservationPolicy,
  ObservationSubjectType,
  ObservationValueData,
  ObservationValueRecord,
  ObservationValueType,
  Participation,
  Procedure,
  ProjectSummary,
  TemplateFieldValueType,
} from '@/domain/models'
import {
  gateway,
  type AiExtractionRecord,
  type AiModelDefaultsView,
  type AiModelProfileView,
  type AttachmentMetadata,
  type PrivateImageRecord,
} from '@/services/gateway'
import {
  canCreateProject,
  canPublishTemplate,
  canWriteExperiment,
  currentProjectId,
} from '@/services/projectContext'
import PageHeader from '@/components/PageHeader.vue'
import { router } from '@/router'

const message = useMessage()
const currentRoute = router.currentRoute
const detailMode = computed(() => typeof currentRoute.value.params.experimentId === 'string')
const detailSection = computed(() => {
  const value = String(currentRoute.value.params.section ?? 'overview')
  return ['overview', 'design', 'animals', 'execution', 'data', 'traceability'].includes(value)
    ? value
    : 'overview'
})
const detailSections = [
  { key: 'overview', label: '概览' },
  { key: 'design', label: '实验设计' },
  { key: 'animals', label: '参与动物' },
  { key: 'execution', label: '实验执行' },
  { key: 'data', label: '数据工作表' },
  { key: 'traceability', label: '追溯' },
] as const
const experiments = ref<Experiment[]>([])
const projects = ref<ProjectSummary[]>([])
const templates = ref<ExperimentTemplateSummary[]>([])
const animals = ref<Animal[]>([])
const loading = ref(true)
const busy = ref(false)
const filter = ref<'all' | Experiment['status']>('all')
const selected = ref<Experiment | null>(null)
const cohorts = ref<Cohort[]>([])
const participations = ref<Participation[]>([])
const procedures = ref<Procedure[]>([])
const experimentEvents = ref<ExperimentEvent[]>([])
const observationDefinitions = ref<ObservationDefinition[]>([])
const observations = ref<Observation[]>([])
const observationValues = ref(new Map<string, ObservationValueRecord[]>())
const experimentAttachments = ref<AttachmentMetadata[]>([])
const historyObservation = ref<Observation | null>(null)
const revisionObservation = ref<Observation | null>(null)
const showCreate = ref(false)
const showProject = ref(false)
const showTemplate = ref(false)
const showCohort = ref(false)
const showEnroll = ref(false)
const showProcedure = ref(false)
const showExperimentEvent = ref(false)
const showObservationDefinition = ref(false)
const showObservation = ref(false)
const showObservationRevision = ref(false)
const showObservationHistory = ref(false)
const experimentFileInput = ref<HTMLInputElement | null>(null)
const dataEntryFileInput = ref<HTMLInputElement | null>(null)
const attachmentUploading = ref(false)
const attachmentDownloadingId = ref<string | null>(null)
interface DataEntryImage {
  localId: string
  file: File
  previewUrl: string
  status: 'staged' | 'uploading' | 'ready' | 'error'
  uploaded?: PrivateImageRecord
  error?: string
}
const MAX_PORTABLE_AI_IMAGE_BYTES = 10 * 1024 * 1024
const dataEntryMode = ref<'manual' | 'ai'>('manual')
const dataEntryImages = ref<DataEntryImage[]>([])
const dataEntryImageError = ref('')
const dataEntryAiBusy = ref(false)
let dataEntryGeneration = 0
const visionProfiles = ref<AiModelProfileView[]>([])
const visionDefaults = ref<AiModelDefaultsView>({ revision: 0 })
const selectedVisionProfileId = ref<string | null>(null)
const extractionDraft = ref<AiExtractionRecord | null>(null)
const aiCandidateNotes = ref('')
const aiApprovalConfirmed = ref(false)
const writeAllowed = computed(() => gateway.mode === 'local' || canWriteExperiment())
const projectCreationAllowed = computed(() => gateway.mode === 'local' || canCreateProject())
const templatePublishAllowed = computed(() => gateway.mode === 'local' || canPublishTemplate())

const newProject = reactive({ name: '', description: '' })
const newTemplate = reactive({
  name: '', description: '', fieldKey: 'observation', fieldLabel: '观察记录',
  fieldValueType: 'text' as TemplateFieldValueType, fieldUnit: '',
})
const newExperiment = reactive({
  projectId: null as string | null,
  templateVersionId: null as string | null,
  name: '',
  description: '',
  startDate: null as number | null,
})
const newCohort = reactive({ name: '', description: '' })
const enrollment = reactive({ animalId: null as string | null, cohortId: null as string | null })
const newProcedure = reactive({
  animalId: null as string | null,
  name: '',
  status: 'planned' as Procedure['status'],
  at: null as number | null,
})
const newExperimentEvent = reactive({
  eventKey: '',
  label: '',
  occurredAt: null as number | null,
  notes: '',
})
const newObservationDefinition = reactive({
  key: '',
  label: '',
  valueType: 'text' as ObservationValueType,
  unit: '',
  categoriesText: '',
  policy: 'versioned' as ObservationPolicy,
})
const newObservation = reactive({
  experimentEventId: null as string | null,
  definitionId: null as string | null,
  subjectType: 'experiment' as ObservationSubjectType,
  subjectId: null as string | null,
  recordedAt: null as number | null,
  notes: '',
  contextJson: '{}',
})
const observationValueForm = reactive({
  numberValue: null as number | null,
  textValue: '',
  booleanValue: false,
  dateValue: null as number | null,
  categoryValue: null as string | null,
  jsonValue: '{}',
})

const filtered = computed(() => filter.value === 'all' ? experiments.value : experiments.value.filter((item) => item.status === filter.value))
const projectOptions = computed(() => projects.value
  .filter((item) => !currentProjectId.value || item.id === currentProjectId.value)
  .map((item) => ({ label: item.name, value: item.id })))
const templateOptions = computed(() => templates.value.map((item) => ({ label: `${item.name} · v${item.version}`, value: item.id })))
const cohortOptions = computed(() => cohorts.value.map((item) => ({ label: item.name, value: item.id })))
const animalOptions = computed(() => {
  const enrolled = new Set(participations.value.map((item) => item.animalId))
  return animals.value
    .filter((animal) => !enrolled.has(animal.id))
    .map((animal) => ({ label: `${animal.code} · ${animal.strain}`, value: animal.id }))
})
const animalLabels = computed(() => new Map(animals.value.map((animal) => [animal.id, animal.code])))
const cohortLabels = computed(() => new Map(cohorts.value.map((cohort) => [cohort.id, cohort.name])))
const eventLabels = computed(() => new Map(
  experimentEvents.value.map((event) => [event.id, event.label]),
))
const definitionLabels = computed(() => new Map(
  observationDefinitions.value.map((definition) => [definition.id, definition.label]),
))
const genotypeDefinitionLabels = ref(new Map<string, string>())
const experimentEventOptions = computed(() => experimentEvents.value.map((event) => ({
  label: `${event.label} · ${new Date(event.occurredAt).toLocaleString('zh-CN')}`,
  value: event.id,
})))
const observationDefinitionOptions = computed(() => observationDefinitions.value.map((definition) => ({
  label: `${definition.label} · ${observationTypeLabel(definition.valueType)}`,
  value: definition.id,
})))
const selectedObservationDefinition = computed(() => observationDefinitions.value.find(
  (definition) => definition.id === newObservation.definitionId,
) ?? null)
const visionProfileOptions = computed(() => visionProfiles.value.map((profile) => ({
  label: `${profile.name} · ${profile.modelId} · v${profile.currentVersion}${profile.isDefaultVision ? '（默认）' : ''}`,
  value: profile.id,
})))
const selectedVisionProfile = computed(() => visionProfiles.value.find((profile) =>
  profile.id === selectedVisionProfileId.value) ?? null)
const extractionCandidate = computed(() => extractionDraft.value?.candidates[0] ?? null)
const extractionCellLocked = computed(() =>
  dataEntryAiBusy.value || Boolean(extractionDraft.value))
const currentDataCellReady = computed(() => Boolean(
  selected.value
  && newObservation.experimentEventId
  && selectedObservationDefinition.value
  && newObservation.subjectId,
))
const revisionDefinition = computed(() => observationDefinitions.value.find(
  (definition) => definition.id === revisionObservation.value?.definitionId,
) ?? null)
const observationSubjectOptions = computed(() => participations.value.map((participation) => ({
  label: animalLabels.value.get(participation.animalId) ?? participation.animalId,
  value: participation.animalId,
})))
const statusMeta: Record<Experiment['status'], { label: string; type: 'default' | 'info' | 'success' | 'error' }> = {
  active: { label: '进行中', type: 'info' },
  draft: { label: '草稿', type: 'default' },
  completed: { label: '已完成', type: 'success' },
  cancelled: { label: '已取消', type: 'error' },
}
const participationStatusMeta: Record<Participation['status'], { label: string; type: 'info' | 'success' | 'warning' }> = {
  enrolled: { label: '进行中', type: 'info' },
  completed: { label: '已完成', type: 'success' },
  withdrawn: { label: '已退出', type: 'warning' },
}
const selectedIsOpen = computed(() => selected.value?.status === 'active' || selected.value?.status === 'draft')

function dateValue(value: number | null) {
  if (!value) return undefined
  const date = new Date(value)
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, '0')}-${String(date.getDate()).padStart(2, '0')}`
}
function instantValue(value: number | null) {
  return value ? new Date(value).toISOString() : undefined
}

function observationTypeLabel(valueType: ObservationValueType) {
  return {
    number: '数值', text: '文本', boolean: '布尔', date: '日期',
    category: '分类', json: 'JSON',
  }[valueType]
}

function observationPolicyLabel(policy: ObservationPolicy) {
  return { immutable: '不可变', mutable: '可修订', versioned: '版本化' }[policy]
}

function genotypeStateLabel(state: Participation['genotypeSnapshot'][number]['state']) {
  return { unknown: '未知', expected: '预期', confirmed: '确认', rejected: '排除' }[state]
}

function parseObject(value: string, field: string): Record<string, unknown> {
  let parsed: unknown
  try {
    parsed = JSON.parse(value || '{}')
  } catch {
    throw new Error(`${field} 必须是合法 JSON`)
  }
  if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) {
    throw new Error(`${field} 必须是 JSON 对象`)
  }
  return parsed as Record<string, unknown>
}

function buildObservationValue(definition: ObservationDefinition): ObservationValueData {
  switch (definition.valueType) {
    case 'number':
      if (observationValueForm.numberValue === null
        || !Number.isFinite(observationValueForm.numberValue)) {
        throw new Error('请输入有效数值')
      }
      return { type: 'number', value: observationValueForm.numberValue }
    case 'text':
      return { type: 'text', value: observationValueForm.textValue }
    case 'boolean':
      return { type: 'boolean', value: observationValueForm.booleanValue }
    case 'date': {
      const value = dateValue(observationValueForm.dateValue)
      if (!value) throw new Error('请选择日期')
      return { type: 'date', value }
    }
    case 'category':
      if (!observationValueForm.categoryValue) throw new Error('请选择分类值')
      return { type: 'category', value: observationValueForm.categoryValue }
    case 'json':
      return { type: 'json', value: JSON.parse(observationValueForm.jsonValue || 'null') }
  }
}

function resetObservationValue(value?: ObservationValueData) {
  Object.assign(observationValueForm, {
    numberValue: value?.type === 'number' ? value.value : null,
    textValue: value?.type === 'text' ? value.value : '',
    booleanValue: value?.type === 'boolean' ? value.value : false,
    dateValue: value?.type === 'date' ? new Date(`${value.value}T00:00:00`).getTime() : null,
    categoryValue: value?.type === 'category' ? value.value : null,
    jsonValue: value?.type === 'json' ? JSON.stringify(value.value, null, 2) : '{}',
  })
}

function latestObservationValue(observation: Observation) {
  return observationValues.value.get(observation.id)?.at(-1)
}

function formatObservationValue(value?: ObservationValueData) {
  if (!value) return '尚无值'
  if (value.type === 'boolean') return value.value ? '是' : '否'
  if (value.type === 'json') return JSON.stringify(value.value)
  return String(value.value)
}

function observationSubjectLabel(observation: Observation) {
  if (observation.subjectType === 'experiment') return selected.value?.name ?? '实验'
  if (observation.subjectType === 'animal') {
    return animalLabels.value.get(observation.subjectId) ?? observation.subjectId
  }
  return `${observation.subjectType} · ${observation.subjectId}`
}

const animalsById = computed(() => new Map(animals.value.map((animal) => [animal.id, animal])))
const experimentLevelObservations = computed(() => observations.value.filter(
  (observation) => observation.subjectType === 'experiment',
))
const currentProcedure = computed(() => {
  const planned = procedures.value.find((procedure) => procedure.status === 'planned')
  return planned ?? procedures.value.at(-1) ?? null
})
const currentProcedureEvent = computed(() => currentProcedure.value
  ? procedureEvent(currentProcedure.value)
  : undefined)

function procedureStatusLabel(status: Procedure['status']) {
  return { planned: '已计划', completed: '已完成', skipped: '已跳过', cancelled: '已取消' }[status]
}

function formatInstant(value?: string) {
  return value ? new Date(value).toLocaleString('zh-CN') : '未设置时间'
}

function eventNotes(event: ExperimentEvent) {
  const notes = event.details.notes
  return typeof notes === 'string' && notes.trim() ? notes : ''
}

function procedureEvent(procedure: Procedure) {
  return experimentEvents.value.find((event) => event.details.procedure_id === procedure.id)
}

function procedureNodeStatus(procedure: Procedure) {
  return procedureEvent(procedure) ? '已生成采集节点' : '未生成采集节点'
}

function procedureNodeTime(procedure: Procedure) {
  return procedure.performedAt ?? procedure.scheduledAt
}

function cellObservation(animalId: string, eventId: string, definitionId: string) {
  return observations.value.find((observation) => observation.subjectType === 'animal'
    && observation.subjectId === animalId
    && observation.experimentEventId === eventId
    && observation.definitionId === definitionId)
}

function cellDisplayValue(animalId: string, eventId: string, definitionId: string) {
  const observation = cellObservation(animalId, eventId, definitionId)
  return formatObservationValue(observation ? latestObservationValue(observation)?.value : undefined)
}

function revokeDataEntryPreview(previewUrl: string) {
  if (previewUrl && typeof URL.revokeObjectURL === 'function') URL.revokeObjectURL(previewUrl)
}

function clearDataEntryImages() {
  for (const image of dataEntryImages.value) revokeDataEntryPreview(image.previewUrl)
  dataEntryImages.value = []
}

function resetDataEntryAiState() {
  dataEntryGeneration += 1
  clearDataEntryImages()
  dataEntryMode.value = 'manual'
  dataEntryImageError.value = ''
  dataEntryAiBusy.value = false
  extractionDraft.value = null
  aiCandidateNotes.value = ''
  aiApprovalConfirmed.value = false
}

class DataEntryOperationCancelled extends Error {}

function isCurrentDataEntryGeneration(generation: number) {
  return generation === dataEntryGeneration
}

function assertCurrentDataEntryGeneration(generation: number) {
  if (!isCurrentDataEntryGeneration(generation) || !showObservation.value) {
    throw new DataEntryOperationCancelled()
  }
}

function beginReviewingExtraction(draft: AiExtractionRecord) {
  extractionDraft.value = draft
  resetObservationValue(draft.candidates[0].value)
  aiCandidateNotes.value = draft.candidates[0].notes ?? ''
  aiApprovalConfirmed.value = false
  dataEntryMode.value = 'ai'
}

async function loadVisionProfiles(generation: number) {
  if (!gateway.listAiModelProfiles || !gateway.getAiModelDefaults) {
    if (!isCurrentDataEntryGeneration(generation)) return
    visionProfiles.value = []
    selectedVisionProfileId.value = null
    return
  }
  const [profiles, defaults] = await Promise.all([
    gateway.listAiModelProfiles(false),
    gateway.getAiModelDefaults(),
  ])
  if (!isCurrentDataEntryGeneration(generation) || !showObservation.value) return
  visionProfiles.value = profiles.filter((profile) =>
    profile.supportsVision && !profile.archivedAt)
  visionDefaults.value = defaults
  const selectedAvailable = visionProfiles.value.some((profile) =>
    profile.id === selectedVisionProfileId.value)
  if (!selectedAvailable) {
    const defaultId = defaults.defaultVisionProfileId ?? null
    selectedVisionProfileId.value = visionProfiles.value.some((profile) =>
      profile.id === defaultId)
      ? defaultId
      : null
  }
}

async function restorePendingExtraction(generation: number) {
  const experiment = selected.value
  const definition = selectedObservationDefinition.value
  if (!gateway.listAiExtractions || !experiment || !definition
    || !newObservation.experimentEventId || !newObservation.subjectId) return
  const drafts = await gateway.listAiExtractions(experiment.projectId)
  if (!isCurrentDataEntryGeneration(generation) || !showObservation.value) return
  const draft = drafts.find((entry) =>
    entry.status === 'pending_approval'
    && entry.projectId === experiment.projectId
    && entry.experimentId === experiment.id
    && entry.experimentEventId === newObservation.experimentEventId
    && entry.currentDataCell?.definitionId === definition.id
    && entry.currentDataCell.subjectType === newObservation.subjectType
    && entry.currentDataCell.subjectId === newObservation.subjectId
    && entry.candidates.length === 1)
  if (draft) beginReviewingExtraction(draft)
}

function openDataEntryModal() {
  resetDataEntryAiState()
  showObservation.value = true
  const generation = dataEntryGeneration
  void Promise.all([
    loadVisionProfiles(generation),
    restorePendingExtraction(generation),
  ]).catch((error) => {
    if (!isCurrentDataEntryGeneration(generation) || !showObservation.value) return
    dataEntryImageError.value = error instanceof Error
      ? error.message
      : '无法读取视觉模型'
  })
}

function chooseDataEntryImages() {
  dataEntryFileInput.value?.click()
}

function stageDataEntryImages(event: Event) {
  const input = event.target as HTMLInputElement
  const files = Array.from(input.files ?? [])
  input.value = ''
  dataEntryImageError.value = ''
  const available = 8 - dataEntryImages.value.length
  if (available <= 0) {
    dataEntryImageError.value = '当前数据单元最多使用 8 张图片'
    return
  }
  if (files.length > available) {
    dataEntryImageError.value = `当前数据单元最多使用 8 张图片，已保留前 ${available} 张`
  }
  const allowed = new Set(['image/jpeg', 'image/png', 'image/webp', 'image/gif'])
  for (const file of files.slice(0, available)) {
    if (!allowed.has(file.type.toLowerCase())) {
      dataEntryImageError.value = `${file.name} 不是支持的 JPEG、PNG、WebP 或 GIF`
      continue
    }
    if (!file.size || file.size > MAX_PORTABLE_AI_IMAGE_BYTES) {
      dataEntryImageError.value = `${file.name} 必须不超过 10 MiB`
      continue
    }
    dataEntryImages.value.push({
      localId: crypto.randomUUID(),
      file,
      previewUrl: typeof URL.createObjectURL === 'function' ? URL.createObjectURL(file) : '',
      status: 'staged',
    })
  }
}

function removeDataEntryImage(localId: string) {
  const image = dataEntryImages.value.find((entry) => entry.localId === localId)
  if (!image) return
  revokeDataEntryPreview(image.previewUrl)
  dataEntryImages.value = dataEntryImages.value.filter((entry) => entry.localId !== localId)
  if (!dataEntryImages.value.length) dataEntryImageError.value = ''
}

async function uploadDataEntryImages(generation: number) {
  if (!gateway.uploadPrivateImage) throw new Error('当前运行模式不支持私人图片上传')
  const uploaded: PrivateImageRecord[] = []
  const stagedImages = [...dataEntryImages.value]
  for (const image of stagedImages) {
    assertCurrentDataEntryGeneration(generation)
    if (image.uploaded) {
      uploaded.push(image.uploaded)
      continue
    }
    image.status = 'uploading'
    image.error = undefined
    try {
      const uploadedImage = await gateway.uploadPrivateImage(image.file)
      assertCurrentDataEntryGeneration(generation)
      image.uploaded = uploadedImage
      image.status = 'ready'
      uploaded.push(uploadedImage)
    } catch (error) {
      if (error instanceof DataEntryOperationCancelled) throw error
      image.status = 'error'
      image.error = error instanceof Error ? error.message : '上传失败'
      dataEntryImageError.value = `${image.file.name}：${image.error}`
      throw error
    }
  }
  return uploaded
}

async function generateExtractionCandidate() {
  const definition = selectedObservationDefinition.value
  if (!selected.value || !newObservation.experimentEventId || !definition
    || !newObservation.subjectId) {
    dataEntryImageError.value = '请先完整选择当前数据单元'
    return
  }
  if (!dataEntryImages.value.length || dataEntryImages.value.length > 8) {
    dataEntryImageError.value = '请选择 1–8 张当前数据单元的图片'
    return
  }
  if (!selectedVisionProfile.value) {
    dataEntryImageError.value = '请明确选择一个可用的视觉模型'
    return
  }
  if (!gateway.createAiExtraction) {
    dataEntryImageError.value = '当前运行模式不支持视觉数据提取'
    return
  }
  const currentDataCell = {
    definitionId: definition.id,
    subjectType: newObservation.subjectType,
    subjectId: newObservation.subjectId,
  }
  const extractionScope = {
    projectId: selected.value.projectId,
    experimentId: selected.value.id,
    experimentEventId: newObservation.experimentEventId,
    visionModelProfileId: selectedVisionProfile.value.id,
  }
  const generation = dataEntryGeneration + 1
  dataEntryGeneration = generation
  dataEntryAiBusy.value = true
  dataEntryImageError.value = ''
  try {
    const uploaded = await uploadDataEntryImages(generation)
    // Closing/resetting after uploads must not start an expensive Provider
    // request that would leave a detached pending draft.
    assertCurrentDataEntryGeneration(generation)
    const draft = await gateway.createAiExtraction({
      imageIds: uploaded.map((image) => image.image.id),
      ...extractionScope,
      currentDataCell,
    })
    assertCurrentDataEntryGeneration(generation)
    if (!draft.currentDataCell
      || draft.currentDataCell.definitionId !== currentDataCell.definitionId
      || draft.currentDataCell.subjectType !== currentDataCell.subjectType
      || draft.currentDataCell.subjectId !== currentDataCell.subjectId
      || draft.candidates.length !== 1) {
      throw new Error('AI 返回的候选没有严格绑定当前数据单元')
    }
    beginReviewingExtraction(draft)
    message.success('已生成当前数据单元的候选，请编辑并人工批准')
  } catch (error) {
    if (error instanceof DataEntryOperationCancelled) return
    if (!isCurrentDataEntryGeneration(generation)) return
    dataEntryImageError.value = error instanceof Error ? error.message : '生成候选失败'
  } finally {
    if (isCurrentDataEntryGeneration(generation)) dataEntryAiBusy.value = false
  }
}

async function rejectExtractionCandidate() {
  const draft = extractionDraft.value
  if (!draft || !gateway.rejectAiExtraction) {
    dataEntryImageError.value = '当前运行模式不支持放弃 AI 候选'
    return
  }
  const generation = dataEntryGeneration + 1
  dataEntryGeneration = generation
  dataEntryAiBusy.value = true
  dataEntryImageError.value = ''
  try {
    const rejected = await gateway.rejectAiExtraction(draft.id, {
      expectedRevision: draft.revision,
    })
    assertCurrentDataEntryGeneration(generation)
    if (rejected.status !== 'rejected') {
      throw new Error('AI 候选未被正确放弃')
    }
    extractionDraft.value = null
    aiCandidateNotes.value = ''
    aiApprovalConfirmed.value = false
    for (const image of dataEntryImages.value) {
      if (image.uploaded) image.status = 'ready'
    }
    message.success('已放弃候选并释放全部私人暂存图片')
  } catch (error) {
    if (error instanceof DataEntryOperationCancelled) return
    if (!isCurrentDataEntryGeneration(generation)) return
    dataEntryImageError.value = error instanceof Error ? error.message : '放弃候选失败'
  } finally {
    if (isCurrentDataEntryGeneration(generation)) dataEntryAiBusy.value = false
  }
}

async function approveExtractionCandidate() {
  const draft = extractionDraft.value
  const definition = selectedObservationDefinition.value
  if (!draft || !definition || !gateway.approveAiExtraction) return
  if (!aiApprovalConfirmed.value) {
    dataEntryImageError.value = '批准前请确认已核对当前数据单元、候选值和图片证据'
    return
  }
  const generation = dataEntryGeneration + 1
  dataEntryGeneration = generation
  dataEntryAiBusy.value = true
  dataEntryImageError.value = ''
  try {
    const applied = await gateway.approveAiExtraction(draft.id, {
      expectedRevision: draft.revision,
      selections: [{
        itemIndex: draft.candidates[0].itemIndex,
        value: buildObservationValue(definition),
        notes: aiCandidateNotes.value.trim() || undefined,
      }],
    })
    assertCurrentDataEntryGeneration(generation)
    for (const observation of applied.observations) {
      const index = observations.value.findIndex((entry) => entry.id === observation.id)
      if (index >= 0) observations.value[index] = observation
      else observations.value.push(observation)
    }
    let refreshFailed = false
    try {
      const loadedValues = await Promise.all(applied.observations.map(async (observation) => [
        observation.id,
        await gateway.listObservationValues(observation.id),
      ] as const))
      assertCurrentDataEntryGeneration(generation)
      const next = new Map(observationValues.value)
      for (const [observationId, values] of loadedValues) next.set(observationId, values)
      observationValues.value = next
    } catch (error) {
      if (error instanceof DataEntryOperationCancelled) return
      refreshFailed = true
    }
    // The atomic approval has already succeeded. Never leave a retryable
    // "approval failed" state merely because the follow-up refresh failed.
    dataEntryAiBusy.value = false
    showObservation.value = false
    message.success('已由人工批准并原子写入 Observation、附件、Audit 与 Provenance')
    if (refreshFailed) {
      message.warning('数据已正式写入，但最新观察值刷新失败；重新打开实验即可同步')
    }
  } catch (error) {
    if (error instanceof DataEntryOperationCancelled) return
    if (!isCurrentDataEntryGeneration(generation)) return
    dataEntryImageError.value = error instanceof Error ? error.message : '批准候选失败'
  } finally {
    if (isCurrentDataEntryGeneration(generation)) dataEntryAiBusy.value = false
  }
}

function editDataCell(
  participation: Participation,
  event: ExperimentEvent,
  definition: ObservationDefinition,
) {
  const existing = cellObservation(participation.animalId, event.id, definition.id)
  if (existing) {
    if (definition.policy === 'immutable') openObservationHistory(existing)
    else openObservationRevision(existing)
    return
  }
  Object.assign(newObservation, {
    experimentEventId: event.id,
    definitionId: definition.id,
    subjectType: 'animal',
    subjectId: participation.animalId,
    recordedAt: Date.now(),
    notes: '',
    contextJson: '{}',
  })
  resetObservationValue()
  openDataEntryModal()
}

async function loadRoute() {
  await load()
  if (!detailMode.value) return
  const experimentId = String(currentRoute.value.params.experimentId)
  const experiment = experiments.value.find((item) => item.id === experimentId)
  if (!experiment) {
    message.error('未找到该实验，或当前项目无权访问')
    await router.replace({ name: 'experiments', query: currentRoute.value.query })
    return
  }
  await loadExperimentDetail(experiment)
}

async function load() {
  loading.value = true
  try {
    const [loadedExperiments, loadedProjects, loadedTemplates, loadedAnimals, genotypeDefinitions] = await Promise.all([
      gateway.listExperiments(),
      gateway.listProjects(),
      gateway.listPublishedTemplates(),
      gateway.listAnimals(),
      gateway.listGenotypeDefinitions(),
    ])
    experiments.value = loadedExperiments
    projects.value = loadedProjects
    templates.value = loadedTemplates
    animals.value = loadedAnimals
    genotypeDefinitionLabels.value = new Map(
      genotypeDefinitions.map((definition) => [definition.id, definition.name]),
    )
    if (currentProjectId.value) newExperiment.projectId = currentProjectId.value
  } catch (error) {
    message.error(error instanceof Error ? error.message : '读取实验数据失败')
  } finally {
    loading.value = false
  }
}

function openExperiment(experiment: Experiment) {
  void router.push({
    name: 'experiment-detail',
    params: { experimentId: experiment.id, section: 'overview' },
    query: currentRoute.value.query,
  })
}

async function loadExperimentDetail(experiment: Experiment) {
  selected.value = experiment
  try {
    const [loadedCohorts, loadedParticipations, loadedProcedures, loadedEvents, loadedDefinitions, loadedObservations] = await Promise.all([
      gateway.listCohorts(experiment.id),
      gateway.listParticipations(experiment.projectId, experiment.id),
      gateway.listProcedures(experiment.id),
      gateway.listExperimentEvents(experiment.id),
      gateway.listObservationDefinitions(experiment.id),
      gateway.listObservations({ experimentId: experiment.id }),
    ])
    if (selected.value?.id !== experiment.id) return
    cohorts.value = loadedCohorts
    participations.value = loadedParticipations
    procedures.value = loadedProcedures
    experimentEvents.value = loadedEvents
    observationDefinitions.value = loadedDefinitions
    observations.value = loadedObservations
    if (gateway.listAttachments) {
      experimentAttachments.value = await gateway.listAttachments({
        entityType: 'experiment',
        entityId: experiment.id,
        projectId: experiment.projectId,
      })
    } else {
      experimentAttachments.value = []
    }
    const values = await Promise.all(loadedObservations.map(async (observation) => [
      observation.id,
      await gateway.listObservationValues(observation.id),
    ] as const))
    if (selected.value?.id === experiment.id) observationValues.value = new Map(values)
  } catch (error) {
    message.error(error instanceof Error ? error.message : '读取实验详情失败')
  }
}

async function createProject() {
  if (!newProject.name.trim()) return message.warning('请输入项目名称')
  busy.value = true
  try {
    const project = await gateway.createProject({ ...newProject, name: newProject.name.trim() })
    projects.value.push(project)
    newExperiment.projectId = project.id
    showProject.value = false
    Object.assign(newProject, { name: '', description: '' })
    message.success('科研项目已创建')
  } catch (error) {
    message.error(error instanceof Error ? error.message : '创建项目失败')
  } finally { busy.value = false }
}

async function createTemplate() {
  if (!newTemplate.name.trim() || !newTemplate.fieldKey.trim() || !newTemplate.fieldLabel.trim()) {
    return message.warning('请完整填写模板和字段')
  }
  busy.value = true
  try {
    const template = await gateway.createPublishedTemplate({ ...newTemplate })
    templates.value.push(template)
    newExperiment.templateVersionId = template.id
    showTemplate.value = false
    Object.assign(newTemplate, {
      name: '', description: '', fieldKey: 'observation', fieldLabel: '观察记录',
      fieldValueType: 'text', fieldUnit: '',
    })
    message.success('模板草稿已配置字段并发布')
  } catch (error) {
    message.error(error instanceof Error ? error.message : '创建模板失败')
  } finally { busy.value = false }
}

async function createExperiment() {
  if (!newExperiment.projectId || !newExperiment.templateVersionId || !newExperiment.name.trim()) {
    return message.warning('请选择项目和已发布模板，并填写实验名称')
  }
  busy.value = true
  try {
    const experiment = await gateway.createExperiment({
      projectId: newExperiment.projectId,
      templateVersionId: newExperiment.templateVersionId,
      name: newExperiment.name.trim(),
      description: newExperiment.description.trim(),
      startDate: dateValue(newExperiment.startDate),
    })
    experiments.value.unshift(experiment)
    showCreate.value = false
    Object.assign(newExperiment, {
      projectId: currentProjectId.value ?? null,
      templateVersionId: null,
      name: '',
      description: '',
      startDate: null,
    })
    message.success('实验已创建')
    await openExperiment(experiment)
  } catch (error) {
    message.error(error instanceof Error ? error.message : '创建实验失败')
  } finally { busy.value = false }
}

async function createCohort() {
  if (!selected.value || !newCohort.name.trim()) return
  busy.value = true
  try {
    const cohort = await gateway.createCohort({
      experimentId: selected.value.id,
      name: newCohort.name.trim(),
      description: newCohort.description.trim(),
    })
    cohorts.value.push(cohort)
    showCohort.value = false
    Object.assign(newCohort, { name: '', description: '' })
    await load()
    message.success('实验组已创建')
  } catch (error) {
    message.error(error instanceof Error ? error.message : '创建实验组失败')
  } finally { busy.value = false }
}

async function enrollAnimal() {
  if (!selected.value || !enrollment.animalId) return
  busy.value = true
  try {
    const participation = await gateway.enrollAnimal({
      experimentId: selected.value.id,
      animalId: enrollment.animalId,
      cohortId: enrollment.cohortId ?? undefined,
    })
    participations.value.push(participation)
    showEnroll.value = false
    Object.assign(enrollment, { animalId: null, cohortId: null })
    await load()
    message.success('动物已纳入实验')
  } catch (error) {
    message.error(error instanceof Error ? error.message : '纳入动物失败')
  } finally { busy.value = false }
}

async function createProcedure() {
  if (!selected.value || !newProcedure.name.trim()) return
  if (!newProcedure.at) return message.warning('请选择计划或执行时间')
  busy.value = true
  try {
    const at = instantValue(newProcedure.at)
    const completed = newProcedure.status === 'completed'
    const procedure = await gateway.createProcedure({
      experimentId: selected.value.id,
      animalId: newProcedure.animalId ?? undefined,
      name: newProcedure.name.trim(),
      status: newProcedure.status,
      scheduledAt: completed ? undefined : at,
      performedAt: completed ? at : undefined,
      details: {},
    })
    procedures.value.push(procedure)
    showProcedure.value = false
    Object.assign(newProcedure, { animalId: null, name: '', status: 'planned', at: null })
    await load()
    if (selected.value) await loadExperimentDetail(selected.value)
    message.success(completed ? '执行记录已保存' : '实验步骤已安排')
  } catch (error) {
    message.error(error instanceof Error ? error.message : '记录步骤失败')
  } finally { busy.value = false }
}

async function syncProcedureEvent(procedure: Procedure) {
  if (!selected.value) return
  if (procedureEvent(procedure)) {
    await router.push({
      name: 'experiment-detail',
      params: { experimentId: selected.value.id, section: 'data' },
      query: currentRoute.value.query,
    })
    return
  }
  busy.value = true
  try {
    const event = await gateway.createExperimentEvent({
      experimentId: selected.value.id,
      eventKey: `procedure_${procedure.id}`,
      label: procedure.name,
      occurredAt: procedureNodeTime(procedure),
      details: {
        source: 'procedure',
        procedure_id: procedure.id,
        procedure_status: procedure.status,
      },
    })
    experimentEvents.value.push(event)
    await router.push({
      name: 'experiment-detail',
      params: { experimentId: selected.value.id, section: 'data' },
      query: currentRoute.value.query,
    })
    message.success('采集节点已生成')
  } catch (error) {
    message.error(error instanceof Error ? error.message : '生成采集节点失败')
  } finally {
    busy.value = false
  }
}

function chooseExperimentAttachment() {
  if (!gateway.uploadAttachment) {
    message.warning('当前运行模式未提供附件上传')
    return
  }
  experimentFileInput.value?.click()
}

async function uploadExperimentAttachment(event: Event) {
  const input = event.target as HTMLInputElement
  const file = input.files?.[0]
  if (!file || !selected.value || !gateway.uploadAttachment) return
  attachmentUploading.value = true
  try {
    await gateway.uploadAttachment({
      entityType: 'experiment',
      entityId: selected.value.id,
      projectId: selected.value.projectId,
      fileName: file.name,
      mediaType: file.type || undefined,
      content: file,
    })
    await loadExperimentDetail(selected.value)
    message.success(`已上传 ${file.name}`)
  } catch (error) {
    message.error(error instanceof Error ? error.message : '附件上传失败')
  } finally {
    attachmentUploading.value = false
    input.value = ''
  }
}

async function downloadExperimentAttachment(attachment: AttachmentMetadata) {
  if (!gateway.downloadAttachment) {
    message.warning('当前运行模式未提供附件下载')
    return
  }
  attachmentDownloadingId.value = attachment.id
  try {
    const blob = await gateway.downloadAttachment(attachment.id)
    const url = URL.createObjectURL(blob)
    const anchor = document.createElement('a')
    anchor.href = url
    anchor.download = attachment.fileName
    anchor.click()
    window.setTimeout(() => URL.revokeObjectURL(url), 0)
  } catch (error) {
    message.error(error instanceof Error ? error.message : '附件下载失败')
  } finally {
    attachmentDownloadingId.value = null
  }
}

function openExperimentEvent() {
  Object.assign(newExperimentEvent, {
    eventKey: `event_${Date.now().toString(36)}`,
    label: '',
    occurredAt: Date.now(),
    notes: '',
  })
  showExperimentEvent.value = true
}

async function createExperimentEvent() {
  if (!selected.value || !newExperimentEvent.label.trim()) {
    return message.warning('请填写事件名称')
  }
  busy.value = true
  try {
    const event = await gateway.createExperimentEvent({
      experimentId: selected.value.id,
      eventKey: newExperimentEvent.eventKey.trim(),
      label: newExperimentEvent.label.trim(),
      occurredAt: instantValue(newExperimentEvent.occurredAt),
      details: newExperimentEvent.notes.trim()
        ? { notes: newExperimentEvent.notes.trim() }
        : {},
    })
    experimentEvents.value.push(event)
    newObservation.experimentEventId = event.id
    showExperimentEvent.value = false
    message.success('实验事件已创建')
  } catch (error) {
    message.error(error instanceof Error ? error.message : '创建实验事件失败')
  } finally { busy.value = false }
}

function openObservationDefinition() {
  Object.assign(newObservationDefinition, {
    key: '',
    label: '',
    valueType: 'text',
    unit: '',
    categoriesText: '',
    policy: 'versioned',
  })
  showObservationDefinition.value = true
}

async function createObservationDefinition() {
  if (!selected.value || !newObservationDefinition.key.trim()
    || !newObservationDefinition.label.trim()) {
    return message.warning('请填写观察定义键和标签')
  }
  const categories = newObservationDefinition.categoriesText
    .split(/[,，\n]/)
    .map((item) => item.trim())
    .filter(Boolean)
  if (newObservationDefinition.valueType === 'number' && !newObservationDefinition.unit.trim()) {
    return message.warning('数值观察定义必须填写单位')
  }
  if (newObservationDefinition.valueType === 'category' && !categories.length) {
    return message.warning('分类观察定义必须填写至少一个类别')
  }
  busy.value = true
  try {
    const definition = await gateway.createObservationDefinition({
      experimentId: selected.value.id,
      key: newObservationDefinition.key.trim(),
      label: newObservationDefinition.label.trim(),
      valueType: newObservationDefinition.valueType,
      unit: newObservationDefinition.valueType === 'number'
        ? newObservationDefinition.unit.trim()
        : undefined,
      categories: newObservationDefinition.valueType === 'category' ? categories : [],
      policy: newObservationDefinition.policy,
    })
    observationDefinitions.value.push(definition)
    newObservation.definitionId = definition.id
    showObservationDefinition.value = false
    message.success('观察定义已创建')
  } catch (error) {
    message.error(error instanceof Error ? error.message : '创建观察定义失败')
  } finally { busy.value = false }
}

function openObservation() {
  if (!selected.value) return
  Object.assign(newObservation, {
    experimentEventId: experimentEvents.value[0]?.id ?? null,
    definitionId: observationDefinitions.value[0]?.id ?? null,
    subjectType: 'experiment',
    subjectId: selected.value.id,
    recordedAt: Date.now(),
    notes: '',
    contextJson: '{}',
  })
  resetObservationValue()
  openDataEntryModal()
}

function normalizeObservationSubject() {
  if (!selected.value) return
  newObservation.subjectId = newObservation.subjectType === 'experiment'
    ? selected.value.id
    : newObservation.subjectType === 'animal'
      ? observationSubjectOptions.value[0]?.value ?? null
      : null
}

async function createObservation() {
  const definition = selectedObservationDefinition.value
  if (!selected.value || !newObservation.experimentEventId || !definition
    || !newObservation.subjectId) {
    return message.warning('请选择实验事件、观察定义和观察对象')
  }
  busy.value = true
  try {
    const recorded = await gateway.recordObservation({
      experimentId: selected.value.id,
      experimentEventId: newObservation.experimentEventId,
      definitionId: definition.id,
      subjectType: newObservation.subjectType,
      subjectId: newObservation.subjectId,
      context: parseObject(newObservation.contextJson, '观察上下文'),
      value: buildObservationValue(definition),
      recordedAt: instantValue(newObservation.recordedAt),
      notes: newObservation.notes.trim() || undefined,
    })
    observations.value.push(recorded.observation)
    const next = new Map(observationValues.value)
    next.set(recorded.observation.id, [recorded.value])
    observationValues.value = next
    showObservation.value = false
    message.success('科学观察已记录')
  } catch (error) {
    message.error(error instanceof Error ? error.message : '记录观察失败')
  } finally { busy.value = false }
}

function openObservationHistory(observation: Observation) {
  historyObservation.value = observation
  showObservationHistory.value = true
}

function openObservationRevision(observation: Observation) {
  revisionObservation.value = observation
  resetObservationValue(latestObservationValue(observation)?.value)
  newObservation.recordedAt = Date.now()
  newObservation.notes = ''
  showObservationRevision.value = true
}

async function reviseObservation() {
  const observation = revisionObservation.value
  const definition = revisionDefinition.value
  if (!observation || !definition) return
  busy.value = true
  try {
    const revised = await gateway.reviseObservation({
      observationId: observation.id,
      expectedRevision: observation.revision,
      value: buildObservationValue(definition),
      recordedAt: instantValue(newObservation.recordedAt),
      notes: newObservation.notes.trim() || undefined,
    })
    const index = observations.value.findIndex((item) => item.id === revised.observation.id)
    if (index >= 0) observations.value[index] = revised.observation
    const next = new Map(observationValues.value)
    next.set(revised.observation.id, [
      ...(next.get(revised.observation.id) ?? []),
      revised.value,
    ])
    observationValues.value = next
    revisionObservation.value = revised.observation
    showObservationRevision.value = false
    message.success('观察值新版本已追加')
  } catch (error) {
    message.error(error instanceof Error ? error.message : '修订观察失败')
  } finally { busy.value = false }
}

async function transitionExperiment(target: 'completed' | 'cancelled') {
  const current = selected.value
  if (!current) return
  busy.value = true
  try {
    const updated = target === 'completed'
      ? await gateway.completeExperiment(current.id, current.revision)
      : await gateway.cancelExperiment(current.id, current.revision)
    selected.value = updated
    await load()
    const refreshed = experiments.value.find((experiment) => experiment.id === updated.id) ?? updated
    await openExperiment(refreshed)
    message.success(target === 'completed' ? '实验已完成，进行中的动物参与已一并结束' : '实验已取消，进行中的动物参与已退出')
  } catch (error) {
    message.error(error instanceof Error ? error.message : '更新实验状态失败')
  } finally {
    busy.value = false
  }
}

async function transitionParticipation(
  participation: Participation,
  target: 'completed' | 'withdrawn',
) {
  busy.value = true
  try {
    const updated = target === 'completed'
      ? await gateway.completeParticipation(participation.id, participation.revision)
      : await gateway.withdrawParticipation(participation.id, participation.revision)
    const index = participations.value.findIndex((item) => item.id === updated.id)
    if (index >= 0) participations.value[index] = updated
    await load()
    message.success(target === 'completed' ? '动物实验参与已完成' : '动物已退出实验')
  } catch (error) {
    message.error(error instanceof Error ? error.message : '更新动物参与状态失败')
  } finally {
    busy.value = false
  }
}

watch(() => newTemplate.fieldValueType, (value) => {
  if (value !== 'number') newTemplate.fieldUnit = ''
})
watch(() => newObservationDefinition.valueType, (value) => {
  if (value !== 'number') newObservationDefinition.unit = ''
  if (value !== 'category') newObservationDefinition.categoriesText = ''
})
watch(() => newObservation.definitionId, () => resetObservationValue())
watch(() => newObservation.subjectType, normalizeObservationSubject)
watch(showObservation, (visible) => {
  if (!visible && dataEntryAiBusy.value) {
    showObservation.value = true
    return
  }
  if (!visible) resetDataEntryAiState()
})
onUnmounted(() => {
  dataEntryGeneration += 1
  clearDataEntryImages()
})
onMounted(loadRoute)
</script>

<template>
  <div class="page">
    <input
      ref="experimentFileInput"
      class="visually-hidden"
      type="file"
      accept="image/*,.pdf,.tif,.tiff,.heic,.heif,.csv,.xlsx"
      @change="uploadExperimentAttachment"
    >
    <template v-if="!detailMode">
    <PageHeader title="实验管理" description="将参与动物、实验步骤与测量数据组织在同一个可追溯流程中。">
      <template #actions><n-button v-if="writeAllowed" type="primary" @click="showCreate = true"><template #icon><Plus :size="17" /></template>创建实验</n-button></template>
    </PageHeader>
    <section class="metrics">
      <div class="surface"><FlaskConical :size="19" /><span>进行中的实验</span><strong>{{ experiments.filter((e) => e.status === 'active').length }}</strong></div>
      <div class="surface"><UsersRound :size="19" /><span>已纳入动物</span><strong>{{ experiments.reduce((n, e) => n + e.animalCount, 0) }}</strong></div>
      <div class="surface"><CalendarClock :size="19" /><span>待执行步骤</span><strong>{{ experiments.reduce((n, e) => n + Math.max(0, e.totalSteps - e.completedSteps), 0) }}</strong></div>
      <div class="surface"><CheckCircle2 :size="19" /><span>已完成实验</span><strong>{{ experiments.filter((e) => e.status === 'completed').length }}</strong></div>
    </section>
    <div class="filter-row"><n-radio-group v-model:value="filter" size="small"><n-radio-button value="all">全部</n-radio-button><n-radio-button value="active">进行中</n-radio-button><n-radio-button value="draft">草稿</n-radio-button><n-radio-button value="completed">已完成</n-radio-button><n-radio-button value="cancelled">已取消</n-radio-button></n-radio-group></div>
    <n-spin :show="loading">
      <section class="experiment-list">
        <article v-for="experiment in filtered" :key="experiment.id" class="experiment-card surface">
          <div class="experiment-main">
            <div class="eyebrow"><span>{{ experiment.code }}</span><n-tag :type="statusMeta[experiment.status].type" size="small" round :bordered="false">{{ statusMeta[experiment.status].label }}</n-tag></div>
            <h2>{{ experiment.name }}</h2>
            <p>{{ experiment.project }} · {{ experiment.startDate || '未设置开始日期' }} · {{ experiment.animalCount }} 只动物</p>
            <div class="groups"><span v-for="group in experiment.groups" :key="group.name"><i :style="{ background: group.color }" />{{ group.name }} <b>{{ group.count }}</b></span><span v-if="!experiment.groups.length">尚未分组</span></div>
          </div>
          <div class="progress-panel">
            <span>流程进度</span><strong>{{ experiment.completedSteps }} / {{ experiment.totalSteps }}</strong>
            <n-progress type="line" :percentage="experiment.totalSteps ? experiment.completedSteps / experiment.totalSteps * 100 : 0" :show-indicator="false" :height="6" />
            <p v-if="experiment.status === 'cancelled'">实验已取消</p>
            <p v-else-if="experiment.nextAction"><CalendarClock :size="14" />{{ experiment.nextAction }}</p>
            <p v-else-if="experiment.status === 'completed' || experiment.completedSteps >= experiment.totalSteps" class="completed"><CheckCircle2 :size="14" />流程已完成</p>
            <p v-else><CalendarClock :size="14" />等待记录步骤</p>
            <n-button secondary size="small" @click="openExperiment(experiment)">打开实验</n-button>
          </div>
        </article>
        <n-empty v-if="!loading && !filtered.length" :description="writeAllowed ? '暂无实验，先创建科研项目与已发布模板' : '当前项目暂无可查看的实验'" />
      </section>
    </n-spin>

    </template>
    <n-modal v-model:show="showCreate" preset="card" title="创建实验" class="dialog-card" :bordered="false">
      <n-space vertical>
        <n-alert v-if="!projects.length" type="warning" :show-icon="false">请先创建科研项目。<n-button v-if="projectCreationAllowed" text type="primary" @click="showProject = true">创建项目</n-button></n-alert>
        <n-alert v-if="!templates.length" type="warning" :show-icon="false">实验必须绑定已发布模板。<n-button v-if="templatePublishAllowed" text type="primary" @click="showTemplate = true">配置并发布模板</n-button><span v-else>请联系项目管理员发布模板。</span></n-alert>
      </n-space>
      <n-form label-placement="top">
        <div class="form-grid">
          <n-form-item label="科研项目" required><n-select v-model:value="newExperiment.projectId" :disabled="!!currentProjectId" :options="projectOptions" filterable /><n-button v-if="projectCreationAllowed" text type="primary" @click="showProject = true">新建项目</n-button></n-form-item>
          <n-form-item label="已发布模板" required><n-select v-model:value="newExperiment.templateVersionId" :options="templateOptions" filterable /><n-button v-if="templatePublishAllowed" text type="primary" @click="showTemplate = true">新建模板</n-button></n-form-item>
        </div>
        <n-form-item label="实验名称" required><n-input v-model:value="newExperiment.name" /></n-form-item>
        <n-form-item label="说明"><n-input v-model:value="newExperiment.description" type="textarea" :rows="2" /></n-form-item>
        <n-form-item label="开始日期"><n-date-picker v-model:value="newExperiment.startDate" type="date" clearable /></n-form-item>
      </n-form>
      <template #footer><div class="dialog-actions"><n-button @click="showCreate = false">取消</n-button><n-button type="primary" :loading="busy" :disabled="!projects.length || !templates.length" @click="createExperiment">创建实验</n-button></div></template>
    </n-modal>

    <n-modal v-model:show="showProject" preset="card" title="新建科研项目" class="small-dialog" :bordered="false">
      <n-form label-placement="top"><n-form-item label="项目名称" required><n-input v-model:value="newProject.name" /></n-form-item><n-form-item label="说明"><n-input v-model:value="newProject.description" type="textarea" /></n-form-item></n-form>
      <template #footer><div class="dialog-actions"><n-button @click="showProject = false">取消</n-button><n-button type="primary" :loading="busy" @click="createProject">创建项目</n-button></div></template>
    </n-modal>

    <n-modal v-model:show="showTemplate" preset="card" title="配置并发布实验模板" class="dialog-card" :bordered="false">
      <n-alert type="info" :show-icon="false">此操作先创建模板草稿、写入字段配置，再按 revision 发布；发布后不可直接修改。</n-alert>
      <n-form label-placement="top">
        <n-form-item label="模板名称" required><n-input v-model:value="newTemplate.name" /></n-form-item>
        <n-form-item label="说明"><n-input v-model:value="newTemplate.description" /></n-form-item>
        <div class="form-grid"><n-form-item label="字段键" required><n-input v-model:value="newTemplate.fieldKey" placeholder="body_weight" /></n-form-item><n-form-item label="字段标签" required><n-input v-model:value="newTemplate.fieldLabel" placeholder="体重" /></n-form-item></div>
        <div class="form-grid"><n-form-item label="字段类型"><n-select v-model:value="newTemplate.fieldValueType" :options="[{label:'数值',value:'number'},{label:'文本',value:'text'},{label:'布尔',value:'boolean'},{label:'日期',value:'date'}]" /></n-form-item><n-form-item label="单位"><n-input v-model:value="newTemplate.fieldUnit" :disabled="newTemplate.fieldValueType !== 'number'" placeholder="例如 g" /></n-form-item></div>
      </n-form>
      <template #footer><div class="dialog-actions"><n-button @click="showTemplate = false">取消</n-button><n-button type="primary" :loading="busy" @click="createTemplate">创建并发布</n-button></div></template>
    </n-modal>

    <n-spin v-if="detailMode" :show="loading || !selected">
      <section v-if="selected" class="experiment-workspace">
        <header class="workspace-header surface">
          <div class="workspace-heading">
            <button class="back-link" type="button" @click="router.push({ name: 'experiments', query: currentRoute.query })">← 返回实验列表</button>
            <div class="workspace-title-row">
              <div><div class="eyebrow"><span>{{ selected.code }}</span><n-tag :type="statusMeta[selected.status].type" size="small" round :bordered="false">{{ statusMeta[selected.status].label }}</n-tag></div><h1>{{ selected.name }}</h1><p>{{ selected.project }} · {{ selected.startDate || '未设置开始日期' }} · revision {{ selected.revision }}</p></div>
              <div v-if="writeAllowed && selectedIsOpen" class="lifecycle-actions">
                <n-popconfirm positive-text="确认完成" negative-text="返回" @positive-click="transitionExperiment('completed')"><template #trigger><n-button type="success" secondary :loading="busy">完成实验</n-button></template>完成后将锁定日常录入，并结束所有仍进行中的动物参与记录。</n-popconfirm>
                <n-popconfirm positive-text="确认取消" negative-text="返回" @positive-click="transitionExperiment('cancelled')"><template #trigger><n-button quaternary type="error" :loading="busy">取消实验</n-button></template>取消后，所有仍进行中的动物参与记录将标记为已退出。</n-popconfirm>
              </div>
            </div>
          </div>
          <nav class="workspace-nav" aria-label="实验工作区导航"><router-link v-for="item in detailSections" :key="item.key" :to="{ name: 'experiment-detail', params: { experimentId: selected.id, section: item.key }, query: currentRoute.query }" :class="{ active: detailSection === item.key }">{{ item.label }}</router-link></nav>
        </header>

        <section v-if="detailSection === 'overview'" class="workspace-section">
          <div class="workspace-metrics">
            <article class="surface"><span>参与动物</span><strong>{{ participations.length }}</strong><small>{{ participations.filter((item) => item.status === 'enrolled').length }} 只进行中</small></article>
            <article class="surface"><span>实验组</span><strong>{{ cohorts.length }}</strong><small>{{ participations.filter((item) => !item.cohortId).length }} 只未分组</small></article>
            <article class="surface"><span>操作步骤</span><strong>{{ procedures.length }}</strong><small>{{ procedures.filter((item) => item.status === 'planned').length }} 项待执行</small></article>
            <article class="surface"><span>实验数据</span><strong>{{ observations.length }}</strong><small>{{ observationDefinitions.length }} 个数据列</small></article>
          </div>
          <div class="overview-grid">
            <article class="surface overview-card"><div class="section-title"><div><span>下一步实验执行</span><small>计划时间与实际执行时间分别保留</small></div><n-button v-if="writeAllowed && selectedIsOpen" size="small" type="primary" @click="showProcedure = true">安排操作</n-button></div><div v-for="procedure in procedures.slice(0, 5)" :key="procedure.id" class="compact-row"><div><strong>{{ procedure.name }}</strong><small>{{ procedure.animalId ? animalLabels.get(procedure.animalId) : '全实验' }}</small></div><div class="compact-meta"><n-tag size="small" :bordered="false">{{ procedureStatusLabel(procedure.status) }}</n-tag><small>{{ formatInstant(procedure.performedAt ?? procedure.scheduledAt) }}</small></div></div><n-empty v-if="!procedures.length" description="尚未安排实验操作" /></article>
            <article class="surface overview-card"><div class="section-title"><div><span>数据工作表</span><small>以 Observation 作为权威实验数据源</small></div><n-button size="small" @click="router.push({ name: 'experiment-detail', params: { experimentId: selected.id, section: 'data' }, query: currentRoute.query })">打开工作表</n-button></div><div class="readiness-list"><span><b>{{ experimentEvents.length }}</b> 个采集节点</span><span><b>{{ observationDefinitions.length }}</b> 个数据列</span><span><b>{{ observations.length }}</b> 条结构化数据</span></div><n-alert v-if="!experimentEvents.length || !observationDefinitions.length" type="info" :show-icon="false">先在“实验设计”中准备分组和数据列，再开始批量录入。</n-alert></article>
          </div>
        </section>

        <section v-else-if="detailSection === 'design'" class="workspace-section">
          <div class="section-toolbar"><div><h2>实验设计</h2><p>集中管理分组、时间结构和数据列，不再暴露面向开发者的 JSON 配置。</p></div><div v-if="writeAllowed && selectedIsOpen"><n-button @click="showCohort = true">添加实验组</n-button><n-button type="primary" secondary @click="openObservationDefinition">添加数据列</n-button></div></div>
          <div class="design-grid">
            <article class="surface design-panel"><div class="section-title"><div><span>实验组</span><small>组别属于动物在本实验中的参与关系</small></div><b>{{ cohorts.length }}</b></div><div v-for="cohort in cohorts" :key="cohort.id" class="design-item"><div><strong>{{ cohort.name }}</strong><small>{{ cohort.description || '无说明' }}</small></div><n-tag size="small" :bordered="false">{{ participations.filter((item) => item.cohortId === cohort.id).length }} 只</n-tag></div><n-empty v-if="!cohorts.length" description="尚未创建实验组" /></article>
            <article class="surface design-panel"><div class="section-title"><div><span>数据列</span><small>实验人员在工作表中看到的表头</small></div><b>{{ observationDefinitions.length }}</b></div><div v-for="definition in observationDefinitions" :key="definition.id" class="design-item"><div><strong>{{ definition.label }}<template v-if="definition.unit">（{{ definition.unit }}）</template></strong><small>{{ observationTypeLabel(definition.valueType) }} · {{ observationPolicyLabel(definition.policy) }}</small></div><code>{{ definition.key }}</code></div><n-empty v-if="!observationDefinitions.length" description="尚未添加数据列" /></article>
            <article class="surface design-panel design-wide"><div class="section-title"><div><span>时间结构</span><small>计划时间点与实际事件将分开管理；现有记录作为采集节点兼容显示</small></div><b>{{ experimentEvents.length }}</b></div><div class="timeline-chips"><span v-for="event in experimentEvents" :key="event.id"><strong>{{ event.label }}</strong><small>{{ formatInstant(event.occurredAt) }}</small></span></div><n-empty v-if="!experimentEvents.length" description="尚无采集节点；实际事件可在“追溯”中记录" /></article>
          </div>
        </section>

        <section v-else-if="detailSection === 'animals'" class="workspace-section">
          <div class="section-toolbar"><div><h2>参与动物</h2><p>动物主档案保持独立；这里保存本实验的组别、参与状态和入组快照。</p></div><n-button v-if="writeAllowed && selectedIsOpen" type="primary" @click="showEnroll = true">纳入动物</n-button></div>
          <div class="surface participant-table-wrap"><table class="participant-table"><thead><tr><th>动物编号</th><th>性别/品系</th><th>实验组</th><th>入组时间</th><th>基因检测快照</th><th>状态</th><th></th></tr></thead><tbody><tr v-for="item in participations" :key="item.id">
            <td><strong>{{ animalLabels.get(item.animalId) ?? item.animalId }}</strong></td><td>{{ animalsById.get(item.animalId)?.sex === 'male' ? '雄性' : animalsById.get(item.animalId)?.sex === 'female' ? '雌性' : '未知' }} · {{ animalsById.get(item.animalId)?.strain || '未知品系' }}</td><td>{{ item.cohortId ? cohortLabels.get(item.cohortId) : '未分组' }}</td><td>{{ formatInstant(item.enrolledAt) }}</td>
            <td><div class="snapshot-tags"><n-tag v-for="snapshot in item.genotypeSnapshot" :key="snapshot.genotypingRecordId" size="tiny" :bordered="false">{{ genotypeDefinitionLabels.get(snapshot.genotypeDefinitionId) ?? snapshot.genotypeDefinitionId.slice(0, 8) }} · {{ genotypeStateLabel(snapshot.state) }}</n-tag><span v-if="!item.genotypeSnapshot.length">无快照</span></div></td><td><n-tag :type="participationStatusMeta[item.status].type" size="small" :bordered="false">{{ participationStatusMeta[item.status].label }}</n-tag></td>
            <td><div v-if="writeAllowed && selectedIsOpen && item.status === 'enrolled'" class="participation-actions"><n-popconfirm positive-text="确认完成" negative-text="返回" @positive-click="transitionParticipation(item, 'completed')"><template #trigger><n-button text type="primary" size="tiny" :disabled="busy">完成</n-button></template>将该动物的实验参与标记为已完成？</n-popconfirm><n-popconfirm positive-text="确认退出" negative-text="返回" @positive-click="transitionParticipation(item, 'withdrawn')"><template #trigger><n-button text type="warning" size="tiny" :disabled="busy">退出</n-button></template>将该动物退出当前实验？</n-popconfirm></div></td>
          </tr></tbody></table><n-empty v-if="!participations.length" description="尚未纳入动物" /></div>
        </section>

        <section v-else-if="detailSection === 'execution'" class="workspace-section">
          <div class="section-toolbar">
            <div><h2>实验执行</h2><p>步骤名称和状态保持突出，精确时间作为次级信息显示。</p></div>
            <div v-if="writeAllowed && selectedIsOpen">
              <n-button secondary :loading="attachmentUploading" :disabled="!gateway.uploadAttachment" @click="chooseExperimentAttachment"><template #icon><Upload :size="15" /></template>上传附件</n-button>
              <n-button type="primary" @click="showProcedure = true">安排或记录操作</n-button>
            </div>
          </div>
          <article v-if="currentProcedure" class="surface current-node">
            <div>
              <span>当前采集节点</span>
              <strong>{{ currentProcedureEvent?.label ?? currentProcedure.name }}</strong>
              <small>{{ procedureNodeStatus(currentProcedure) }} · {{ formatInstant(procedureNodeTime(currentProcedure)) }}</small>
            </div>
            <n-button size="small" type="primary" secondary :loading="busy" @click="syncProcedureEvent(currentProcedure)">
              {{ currentProcedureEvent ? '打开工作表' : '生成并打开工作表' }}
            </n-button>
          </article>
          <div class="procedure-list">
            <article v-for="procedure in procedures" :key="procedure.id" class="surface procedure-card">
              <div class="procedure-status"><span :class="['status-dot', procedure.status]" /><n-tag size="small" :bordered="false">{{ procedureStatusLabel(procedure.status) }}</n-tag></div>
              <div><h3>{{ procedure.name }}</h3><p>{{ procedure.animalId ? animalLabels.get(procedure.animalId) : '全实验' }} · {{ procedureNodeStatus(procedure) }}</p></div>
              <div class="procedure-time"><small>{{ procedure.status === 'completed' ? '实际执行' : '计划执行' }}</small><strong>{{ formatInstant(procedureNodeTime(procedure)) }}</strong><n-button v-if="writeAllowed && selectedIsOpen" text type="primary" size="tiny" :loading="busy" @click="syncProcedureEvent(procedure)">{{ procedureEvent(procedure) ? '工作表' : '生成节点' }}</n-button></div>
            </article>
            <n-empty v-if="!procedures.length" description="尚未安排实验操作" />
          </div>
          <article class="surface experiment-attachments">
            <div class="section-title">
              <div><span>实验附件</span><small>保存本实验相关图片、PDF 和数据文件。</small></div>
              <n-button v-if="writeAllowed && selectedIsOpen" size="small" type="primary" secondary :loading="attachmentUploading" :disabled="!gateway.uploadAttachment" @click="chooseExperimentAttachment"><template #icon><Upload :size="15" /></template>上传附件</n-button>
            </div>
            <div v-if="experimentAttachments.length" class="attachment-list">
              <div v-for="attachment in experimentAttachments" :key="attachment.id" class="attachment-row">
                <div><strong>{{ attachment.fileName }}</strong><span>{{ attachment.mediaType || 'application/octet-stream' }} · {{ (attachment.sizeBytes / 1024).toFixed(1) }} KiB · v{{ attachment.version }}</span></div>
                <div>
                  <n-button size="small" secondary :loading="attachmentDownloadingId === attachment.id" :disabled="!gateway.downloadAttachment" @click="downloadExperimentAttachment(attachment)"><template #icon><Download :size="14" /></template>下载</n-button>
                </div>
              </div>
            </div>
            <n-empty v-else description="暂无实验附件" />
          </article>
        </section>

        <section v-else-if="detailSection === 'data'" class="workspace-section data-section">
          <div class="section-toolbar"><div><h2>数据工作表</h2><p>行对应参与动物，表头由采集节点和数据列组成；点击单元格即可录入或修订。</p></div><div v-if="writeAllowed && selectedIsOpen"><n-button @click="openObservationDefinition">添加数据列</n-button><n-button type="primary" :disabled="!experimentEvents.length || !observationDefinitions.length" @click="openObservation">录入实验级数据</n-button></div></div>
          <n-tabs type="card" animated>
            <n-tab-pane name="animal" tab="动物纵向数据"><div v-if="participations.length && experimentEvents.length && observationDefinitions.length" class="sheet surface"><table class="data-grid"><thead><tr><th rowspan="2" class="frozen frozen-id">动物编号</th><th rowspan="2" class="frozen frozen-group">实验组</th><th v-for="event in experimentEvents" :key="event.id" :colspan="observationDefinitions.length" class="event-heading"><strong>{{ event.label }}</strong><small>{{ formatInstant(event.occurredAt) }}</small></th></tr><tr><template v-for="event in experimentEvents" :key="event.id"><th v-for="definition in observationDefinitions" :key="event.id + '-' + definition.id"><strong>{{ definition.label }}</strong><small v-if="definition.unit">{{ definition.unit }}</small></th></template></tr></thead><tbody><tr v-for="participation in participations" :key="participation.id"><td class="frozen frozen-id"><strong>{{ animalLabels.get(participation.animalId) ?? participation.animalId }}</strong></td><td class="frozen frozen-group">{{ participation.cohortId ? cohortLabels.get(participation.cohortId) : '未分组' }}</td><template v-for="event in experimentEvents" :key="event.id"><td v-for="definition in observationDefinitions" :key="participation.id + '-' + event.id + '-' + definition.id"><button type="button" class="data-cell" :class="{ filled: !!cellObservation(participation.animalId, event.id, definition.id) }" :disabled="!writeAllowed || !selectedIsOpen" @click="editDataCell(participation, event, definition)">{{ cellDisplayValue(participation.animalId, event.id, definition.id) }}</button></td></template></tr></tbody></table></div><n-empty v-else description="请先纳入动物，并准备采集节点和数据列" /></n-tab-pane>
            <n-tab-pane name="experiment" tab="实验级记录"><div class="surface experiment-records"><div v-for="observation in experimentLevelObservations" :key="observation.id" class="compact-row"><div><strong>{{ definitionLabels.get(observation.definitionId) ?? observation.definitionId }}</strong><small>{{ eventLabels.get(observation.experimentEventId) ?? observation.experimentEventId }} · v{{ observation.currentValueVersion }}</small></div><div class="observation-actions"><b>{{ formatObservationValue(latestObservationValue(observation)?.value) }}</b><n-button text size="tiny" @click="openObservationHistory(observation)">历史</n-button><n-button v-if="writeAllowed && selectedIsOpen && observationDefinitions.find((definition) => definition.id === observation.definitionId)?.policy !== 'immutable'" text type="primary" size="tiny" @click="openObservationRevision(observation)">修订</n-button></div></div><n-empty v-if="!experimentLevelObservations.length" description="尚无实验级记录" /></div></n-tab-pane>
          </n-tabs>
          <article class="surface experiment-attachments data-attachments">
            <div class="section-title">
              <div><span>数据图片与附件</span><small>当前先关联到实验；单元格级证据将复用资料库关联模型。</small></div>
              <n-button v-if="writeAllowed && selectedIsOpen" size="small" type="primary" secondary :loading="attachmentUploading" :disabled="!gateway.uploadAttachment" @click="chooseExperimentAttachment"><template #icon><Upload :size="15" /></template>上传附件</n-button>
            </div>
            <div v-if="experimentAttachments.length" class="attachment-list compact-attachments">
              <div v-for="attachment in experimentAttachments" :key="attachment.id" class="attachment-row">
                <div><strong>{{ attachment.fileName }}</strong><span>{{ attachment.mediaType || 'application/octet-stream' }} · v{{ attachment.version }}</span></div>
                <n-button size="small" secondary :loading="attachmentDownloadingId === attachment.id" :disabled="!gateway.downloadAttachment" @click="downloadExperimentAttachment(attachment)"><template #icon><Download :size="14" /></template>下载</n-button>
              </div>
            </div>
            <n-empty v-else description="暂无数据图片或附件" />
          </article>
        </section>

        <section v-else class="workspace-section">
          <div class="section-toolbar"><div><h2>追溯</h2><p>集中查看实际事件与数据版本；精确时间不再挤占日常操作列表。</p></div><n-button v-if="writeAllowed && selectedIsOpen" type="primary" @click="openExperimentEvent">记录实验事件</n-button></div>
          <div class="trace-grid"><article class="surface trace-panel"><div class="section-title"><div><span>实验事件</span><small>里程碑、异常和实际发生事项</small></div><b>{{ experimentEvents.length }}</b></div><div v-for="event in experimentEvents" :key="event.id" class="trace-item"><span class="trace-marker" /><div><strong>{{ event.label }}</strong><p v-if="eventNotes(event)">{{ eventNotes(event) }}</p><small>{{ formatInstant(event.occurredAt) }} · revision {{ event.revision }}</small></div></div><n-empty v-if="!experimentEvents.length" description="尚未记录实验事件" /></article><article class="surface trace-panel"><div class="section-title"><div><span>数据修订</span><small>当前值均可回看历史版本</small></div><b>{{ observations.length }}</b></div><div v-for="observation in observations.slice(0, 12)" :key="observation.id" class="compact-row"><div><strong>{{ definitionLabels.get(observation.definitionId) ?? observation.definitionId }}</strong><small>{{ observationSubjectLabel(observation) }} · v{{ observation.currentValueVersion }}</small></div><n-button text size="tiny" @click="openObservationHistory(observation)">查看历史</n-button></div><n-empty v-if="!observations.length" description="尚无数据修订记录" /></article></div>
        </section>
      </section>
    </n-spin>

    <n-modal v-model:show="showCohort" preset="card" title="新增实验组" class="small-dialog"><n-form label-placement="top"><n-form-item label="组名" required><n-input v-model:value="newCohort.name" /></n-form-item><n-form-item label="说明"><n-input v-model:value="newCohort.description" /></n-form-item></n-form><template #footer><div class="dialog-actions"><n-button @click="showCohort = false">取消</n-button><n-button type="primary" :loading="busy" @click="createCohort">创建</n-button></div></template></n-modal>
    <n-modal v-model:show="showEnroll" preset="card" title="纳入动物" class="small-dialog"><n-form label-placement="top"><n-form-item label="动物" required><n-select v-model:value="enrollment.animalId" filterable :options="animalOptions" /></n-form-item><n-form-item label="实验组"><n-select v-model:value="enrollment.cohortId" clearable :options="cohortOptions" /></n-form-item></n-form><template #footer><div class="dialog-actions"><n-button @click="showEnroll = false">取消</n-button><n-button type="primary" :loading="busy" :disabled="!enrollment.animalId" @click="enrollAnimal">确认纳入</n-button></div></template></n-modal>
    <n-modal v-model:show="showProcedure" preset="card" title="记录实验步骤" class="small-dialog"><n-form label-placement="top"><n-form-item label="步骤名称" required><n-input v-model:value="newProcedure.name" /></n-form-item><n-form-item label="关联动物"><n-select v-model:value="newProcedure.animalId" clearable :options="participations.map((p) => ({label: animalLabels.get(p.animalId) ?? p.animalId, value: p.animalId}))" /></n-form-item><div class="form-grid"><n-form-item label="状态"><n-select v-model:value="newProcedure.status" :options="[{label:'计划',value:'planned'},{label:'已完成',value:'completed'}]" /></n-form-item><n-form-item :label="newProcedure.status === 'completed' ? '执行时间' : '计划时间'" required><n-date-picker v-model:value="newProcedure.at" type="datetime" clearable /></n-form-item></div></n-form><template #footer><div class="dialog-actions"><n-button @click="showProcedure = false">取消</n-button><n-button type="primary" :loading="busy" @click="createProcedure">保存记录</n-button></div></template></n-modal>

    <n-modal v-model:show="showExperimentEvent" preset="card" title="记录实验事件" class="small-dialog">
      <n-form label-placement="top"><n-form-item label="事件名称" required><n-input v-model:value="newExperimentEvent.label" placeholder="例如：设备异常、提前退出或阶段完成" /></n-form-item><n-form-item label="实际发生时间"><n-date-picker v-model:value="newExperimentEvent.occurredAt" type="datetime" clearable /></n-form-item><n-form-item label="说明"><n-input v-model:value="newExperimentEvent.notes" type="textarea" :rows="3" placeholder="记录原因、影响和后续处理" /></n-form-item></n-form>
      <template #footer><div class="dialog-actions"><n-button @click="showExperimentEvent = false">取消</n-button><n-button type="primary" :loading="busy" @click="createExperimentEvent">保存事件</n-button></div></template>
    </n-modal>

    <n-modal v-model:show="showObservationDefinition" preset="card" title="添加数据列" class="dialog-card">
      <n-alert type="info" :show-icon="false">数值列需要设置单位，分类列需要配置可选项；完成实验后日常录入将被锁定。</n-alert>
      <n-form label-placement="top"><div class="form-grid"><n-form-item label="内部标识" required><n-input v-model:value="newObservationDefinition.key" placeholder="body_weight" /></n-form-item><n-form-item label="列标题" required><n-input v-model:value="newObservationDefinition.label" placeholder="体重" /></n-form-item></div><div class="form-grid"><n-form-item label="数据类型" required><n-select v-model:value="newObservationDefinition.valueType" :options="[{label:'数值',value:'number'},{label:'文本',value:'text'},{label:'是/否',value:'boolean'},{label:'日期',value:'date'},{label:'分类',value:'category'}]" /></n-form-item><n-form-item label="修改规则" required><n-select v-model:value="newObservationDefinition.policy" :options="[{label:'保留每次修改',value:'versioned'},{label:'允许修订',value:'mutable'},{label:'录入后不可修改',value:'immutable'}]" /></n-form-item></div><n-form-item v-if="newObservationDefinition.valueType === 'number'" label="单位" required><n-input v-model:value="newObservationDefinition.unit" placeholder="例如 g" /></n-form-item><n-form-item v-if="newObservationDefinition.valueType === 'category'" label="可选项（逗号或换行分隔）" required><n-input v-model:value="newObservationDefinition.categoriesText" type="textarea" placeholder="正常, 轻度, 重度" /></n-form-item></n-form>
      <template #footer><div class="dialog-actions"><n-button @click="showObservationDefinition = false">取消</n-button><n-button type="primary" :loading="busy" @click="createObservationDefinition">添加数据列</n-button></div></template>
    </n-modal>

    <n-modal
      v-model:show="showObservation"
      preset="card"
      title="录入实验数据"
      class="data-entry-dialog"
      :closable="!dataEntryAiBusy"
      :mask-closable="!dataEntryAiBusy"
      :close-on-esc="!dataEntryAiBusy"
    >
      <n-alert type="info" :show-icon="false">
        当前数据单元由采集节点、数据列与数据对象共同确定。AI 只能为这个单元生成候选，不能修改绑定，也不会直接写入正式数据。
      </n-alert>
      <n-form label-placement="top" class="data-cell-form">
        <div class="form-grid">
          <n-form-item label="采集节点" required>
            <n-select
              v-model:value="newObservation.experimentEventId"
              :options="experimentEventOptions"
              :disabled="extractionCellLocked"
              filterable
            />
          </n-form-item>
          <n-form-item label="数据列" required>
            <n-select
              v-model:value="newObservation.definitionId"
              :options="observationDefinitionOptions"
              :disabled="extractionCellLocked"
              filterable
            />
          </n-form-item>
        </div>
        <div class="form-grid">
          <n-form-item label="数据对象类型" required>
            <n-select
              v-model:value="newObservation.subjectType"
              :disabled="extractionCellLocked"
              :options="[
                {label:'整个实验',value:'experiment'},
                {label:'动物',value:'animal'},
                {label:'样本',value:'sample'},
                {label:'研究产物',value:'artifact'},
              ]"
            />
          </n-form-item>
          <n-form-item label="数据对象" required>
            <n-input
              v-if="newObservation.subjectType === 'experiment'"
              :value="selected?.name"
              disabled
            />
            <n-select
              v-else-if="newObservation.subjectType === 'animal'"
              v-model:value="newObservation.subjectId"
              :options="observationSubjectOptions"
              :disabled="extractionCellLocked"
              filterable
            />
            <n-input
              v-else
              v-model:value="newObservation.subjectId"
              :disabled="extractionCellLocked"
              placeholder="输入当前实验内对象的 UUID"
            />
          </n-form-item>
        </div>
      </n-form>

      <n-tabs v-model:value="dataEntryMode" type="segment" animated>
        <n-tab-pane name="manual" tab="手工录入">
          <n-form v-if="selectedObservationDefinition" label-placement="top">
            <n-form-item
              v-if="selectedObservationDefinition.valueType === 'number'"
              :label="'数值（' + selectedObservationDefinition.unit + '）'"
              required
            ><n-input-number v-model:value="observationValueForm.numberValue" /></n-form-item>
            <n-form-item
              v-else-if="selectedObservationDefinition.valueType === 'text'"
              label="文本"
              required
            ><n-input v-model:value="observationValueForm.textValue" type="textarea" /></n-form-item>
            <n-form-item
              v-else-if="selectedObservationDefinition.valueType === 'boolean'"
              label="是/否"
            ><n-switch v-model:value="observationValueForm.booleanValue" /></n-form-item>
            <n-form-item
              v-else-if="selectedObservationDefinition.valueType === 'date'"
              label="日期"
              required
            ><n-date-picker v-model:value="observationValueForm.dateValue" type="date" /></n-form-item>
            <n-form-item
              v-else-if="selectedObservationDefinition.valueType === 'category'"
              label="分类"
              required
            >
              <n-select
                v-model:value="observationValueForm.categoryValue"
                :options="selectedObservationDefinition.categories.map((value) => ({ label: value, value }))"
              />
            </n-form-item>
            <n-form-item v-else label="结构化值" required>
              <n-input v-model:value="observationValueForm.jsonValue" type="textarea" :rows="3" />
            </n-form-item>
            <n-form-item label="实际记录时间">
              <n-date-picker v-model:value="newObservation.recordedAt" type="datetime" clearable />
            </n-form-item>
            <n-form-item label="备注">
              <n-input v-model:value="newObservation.notes" type="textarea" />
            </n-form-item>
          </n-form>
        </n-tab-pane>

        <n-tab-pane name="ai" tab="图片识别候选">
          <div class="ai-entry-boundary">
            <ShieldCheck :size="16" />
            <span>图片先进入私人暂存区；生成结果只是候选，批准时才事务性创建正式 Observation、附件关系、Audit 与 Provenance。</span>
          </div>
          <input
            ref="dataEntryFileInput"
            class="visually-hidden"
            type="file"
            multiple
            accept="image/jpeg,image/png,image/webp,image/gif"
            aria-label="选择当前数据单元的图片"
            @change="stageDataEntryImages"
          >
          <div class="ai-entry-toolbar">
            <n-button
              secondary
              :disabled="extractionCellLocked || dataEntryImages.length >= 8"
              @click="chooseDataEntryImages"
            >
              <template #icon><ImagePlus :size="16" /></template>
              添加图片
            </n-button>
            <span>{{ dataEntryImages.length }}/8 张</span>
          </div>
          <div v-if="dataEntryImages.length" class="data-entry-images">
            <article v-for="image in dataEntryImages" :key="image.localId">
              <img :src="image.previewUrl" :alt="`当前数据单元图片：${image.file.name}`">
              <div>
                <strong>{{ image.file.name }}</strong>
                <small v-if="image.status === 'uploading'">正在上传…</small>
                <small v-else-if="image.status === 'ready'">已安全暂存</small>
                <small v-else-if="image.status === 'error'" class="image-error">{{ image.error }}</small>
                <small v-else>{{ (image.file.size / 1048576).toFixed(1) }} MiB</small>
              </div>
              <button
                type="button"
                :aria-label="`移除图片 ${image.file.name}`"
                :disabled="extractionCellLocked"
                @click="removeDataEntryImage(image.localId)"
              ><Trash2 :size="15" /></button>
            </article>
          </div>
          <n-empty v-else description="添加 1–8 张只属于当前数据单元的图片" />

          <n-form label-placement="top" class="vision-model-field">
            <n-form-item label="视觉模型" required>
              <n-select
                v-model:value="selectedVisionProfileId"
                :options="visionProfileOptions"
                :disabled="extractionCellLocked"
                clearable
                filterable
                placeholder="没有可用默认值时必须明确选择"
              />
            </n-form-item>
          </n-form>

          <article v-if="extractionDraft && extractionCandidate" class="candidate-review">
            <header>
              <div>
                <span><ScanSearch :size="16" />AI 候选（可编辑）</span>
                <small>
                  置信度 {{ Math.round(extractionCandidate.confidence * 100) }}%
                  · v{{ extractionDraft.modelTrace?.profileVersion ?? '—' }}
                  · {{ extractionDraft.evidence.length }} 张证据
                </small>
              </div>
              <n-tag type="warning" size="small">尚未写入</n-tag>
            </header>
            <n-progress
              type="line"
              :percentage="Math.round(extractionCandidate.confidence * 100)"
              :show-indicator="false"
            />
            <n-form v-if="selectedObservationDefinition" label-placement="top">
              <n-form-item
                v-if="selectedObservationDefinition.valueType === 'number'"
                :label="'候选数值（' + selectedObservationDefinition.unit + '）'"
                required
              ><n-input-number v-model:value="observationValueForm.numberValue" /></n-form-item>
              <n-form-item
                v-else-if="selectedObservationDefinition.valueType === 'text'"
                label="候选文本"
                required
              ><n-input v-model:value="observationValueForm.textValue" type="textarea" /></n-form-item>
              <n-form-item
                v-else-if="selectedObservationDefinition.valueType === 'boolean'"
                label="候选是/否"
              ><n-switch v-model:value="observationValueForm.booleanValue" /></n-form-item>
              <n-form-item
                v-else-if="selectedObservationDefinition.valueType === 'date'"
                label="候选日期"
                required
              ><n-date-picker v-model:value="observationValueForm.dateValue" type="date" /></n-form-item>
              <n-form-item
                v-else-if="selectedObservationDefinition.valueType === 'category'"
                label="候选分类"
                required
              >
                <n-select
                  v-model:value="observationValueForm.categoryValue"
                  :options="selectedObservationDefinition.categories.map((value) => ({ label: value, value }))"
                />
              </n-form-item>
              <n-form-item v-else label="候选结构化值" required>
                <n-input v-model:value="observationValueForm.jsonValue" type="textarea" :rows="3" />
              </n-form-item>
              <n-form-item label="人工备注">
                <n-input
                  v-model:value="aiCandidateNotes"
                  type="textarea"
                  :rows="2"
                  maxlength="1024"
                  show-count
                />
              </n-form-item>
            </n-form>
            <n-checkbox v-model:checked="aiApprovalConfirmed">
              我已核对当前数据单元、候选值和全部图片证据，并批准写入正式数据
            </n-checkbox>
          </article>

          <p
            v-if="dataEntryImageError"
            class="data-entry-error"
            role="alert"
            aria-live="assertive"
          >{{ dataEntryImageError }}</p>
        </n-tab-pane>
      </n-tabs>

      <template #footer>
        <div class="dialog-actions">
          <n-button
            :disabled="busy || dataEntryAiBusy"
            @click="showObservation = false"
          >取消</n-button>
          <n-button
            v-if="dataEntryMode === 'ai' && extractionDraft"
            type="error"
            secondary
            :loading="dataEntryAiBusy"
            @click="rejectExtractionCandidate"
          >放弃候选并释放图片</n-button>
          <n-button
            v-if="dataEntryMode === 'manual'"
            type="primary"
            :loading="busy"
            :disabled="!currentDataCellReady"
            @click="createObservation"
          >保存数据</n-button>
          <n-button
            v-else-if="!extractionDraft"
            type="primary"
            :loading="dataEntryAiBusy"
            :disabled="!currentDataCellReady || !dataEntryImages.length || !selectedVisionProfile"
            @click="generateExtractionCandidate"
          >生成候选</n-button>
          <n-button
            v-else
            type="primary"
            :loading="dataEntryAiBusy"
            :disabled="!aiApprovalConfirmed"
            @click="approveExtractionCandidate"
          >批准并正式写入</n-button>
        </div>
      </template>
    </n-modal>

    <n-modal v-model:show="showObservationRevision" preset="card" title="修订观察值" class="small-dialog">
      <n-alert type="warning" :show-icon="false">修订不会覆盖历史值，而是追加新版本并提升 Observation revision。</n-alert>
      <n-form v-if="revisionDefinition" label-placement="top">
        <n-form-item v-if="revisionDefinition.valueType === 'number'" :label="`数值（${revisionDefinition.unit}）`" required><n-input-number v-model:value="observationValueForm.numberValue" /></n-form-item>
        <n-form-item v-else-if="revisionDefinition.valueType === 'text'" label="文本值" required><n-input v-model:value="observationValueForm.textValue" type="textarea" /></n-form-item>
        <n-form-item v-else-if="revisionDefinition.valueType === 'boolean'" label="布尔值"><n-switch v-model:value="observationValueForm.booleanValue" /></n-form-item>
        <n-form-item v-else-if="revisionDefinition.valueType === 'date'" label="日期值" required><n-date-picker v-model:value="observationValueForm.dateValue" type="date" /></n-form-item>
        <n-form-item v-else-if="revisionDefinition.valueType === 'category'" label="分类值" required><n-select v-model:value="observationValueForm.categoryValue" :options="revisionDefinition.categories.map((value) => ({ label: value, value }))" /></n-form-item>
        <n-form-item v-else label="JSON 值" required><n-input v-model:value="observationValueForm.jsonValue" type="textarea" :rows="3" /></n-form-item>
        <n-form-item label="记录时间"><n-date-picker v-model:value="newObservation.recordedAt" type="datetime" clearable /></n-form-item>
        <n-form-item label="修订说明"><n-input v-model:value="newObservation.notes" type="textarea" /></n-form-item>
      </n-form>
      <template #footer><div class="dialog-actions"><n-button @click="showObservationRevision = false">取消</n-button><n-button type="primary" :loading="busy" @click="reviseObservation">追加版本</n-button></div></template>
    </n-modal>

    <n-modal v-model:show="showObservationHistory" preset="card" title="观察值版本历史" class="dialog-card">
      <n-list v-if="historyObservation" bordered>
        <n-list-item v-for="value in observationValues.get(historyObservation.id) ?? []" :key="value.id">
          <div>
            <strong>v{{ value.version }} · {{ formatObservationValue(value.value) }}</strong>
            <small>{{ new Date(value.recordedAt).toLocaleString('zh-CN') }} · {{ value.notes || '无修订说明' }}</small>
          </div>
          <template #suffix><n-tag size="small" :bordered="false">revision {{ value.revision }}</n-tag></template>
        </n-list-item>
      </n-list>
    </n-modal>
  </div>
</template>

<style scoped>
.metrics { display: grid; grid-template-columns: repeat(4, 1fr); gap: 10px; margin-bottom: 16px; }
.metrics > div { display: grid; grid-template-columns: 28px 1fr auto; align-items: center; gap: 4px; padding: 13px; }
.metrics svg { color: var(--muri-primary); }
.metrics span { color: var(--muri-text-secondary); }
.metrics strong { font-size: 20px; }
.filter-row { display: flex; justify-content: flex-end; margin-bottom: 11px; }
.experiment-list { display: flex; flex-direction: column; gap: 10px; }
.experiment-card { display: grid; grid-template-columns: 1fr 300px; overflow: hidden; }
.experiment-main { padding: 17px; }
.eyebrow { display: flex; align-items: center; gap: 8px; color: var(--muri-primary); font-size: 12px; font-weight: 600; }
h2 { margin: 7px 0 4px; font-size: 17px; }
.experiment-main > p { margin: 0; color: var(--muri-text-secondary); font-size: 12px; }
.groups { display: flex; flex-wrap: wrap; gap: 7px; margin-top: 14px; }
.groups span { padding: 5px 8px; border: 1px solid var(--muri-border); border-radius: 999px; font-size: 11px; }
.groups i { display: inline-block; width: 7px; height: 7px; margin-right: 5px; border-radius: 50%; }
.groups b { margin-left: 3px; }
.progress-panel { display: grid; grid-template-columns: 1fr auto; align-content: center; gap: 7px 12px; padding: 17px; border-left: 1px solid var(--muri-border); background: var(--muri-surface-muted); }
.progress-panel > span { color: var(--muri-text-secondary); font-size: 12px; }
.progress-panel > :deep(.n-progress), .progress-panel > p, .progress-panel > button { grid-column: 1 / -1; }
.progress-panel p { display: flex; align-items: center; gap: 5px; margin: 2px 0; color: #8a5c1e; font-size: 11px; }
.progress-panel p.completed { color: var(--muri-success); }
.progress-panel button { justify-self: end; }
.dialog-card { width: min(620px, calc(100vw - 28px)); }
.data-entry-dialog { width: min(760px, calc(100vw - 28px)); }
.data-cell-form { margin-top: 14px; }
.ai-entry-boundary { display: flex; align-items: flex-start; gap: 7px; margin: 10px 0 12px; padding: 9px 11px; border: 1px solid #c8deef; border-radius: 8px; color: var(--muri-text-secondary); background: var(--muri-primary-soft); font-size: 11px; line-height: 1.55; }.ai-entry-boundary svg { flex: 0 0 auto; margin-top: 1px; color: var(--muri-primary); }
.ai-entry-toolbar { display: flex; align-items: center; justify-content: space-between; gap: 10px; margin-bottom: 9px; }.ai-entry-toolbar > span { color: var(--muri-text-tertiary); font-size: 11px; }
.data-entry-images { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 8px; margin-bottom: 12px; }.data-entry-images article { display: grid; min-width: 0; grid-template-columns: 58px minmax(0, 1fr) 32px; align-items: center; gap: 8px; padding: 7px; border: 1px solid var(--muri-border); border-radius: 8px; background: var(--muri-surface-muted); }.data-entry-images img { width: 58px; height: 58px; border-radius: 6px; object-fit: cover; }.data-entry-images article > div { display: flex; min-width: 0; flex-direction: column; }.data-entry-images strong,.data-entry-images small { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }.data-entry-images strong { font-size: 11px; }.data-entry-images small { color: var(--muri-text-tertiary); font-size: 10px; }.data-entry-images small.image-error { color: var(--muri-danger); }.data-entry-images button { display: grid; width: 32px; height: 32px; padding: 0; place-items: center; border: 0; border-radius: 6px; color: var(--muri-text-tertiary); background: transparent; cursor: pointer; transition: color var(--muri-transition-fast), background var(--muri-transition-fast); }.data-entry-images button:hover { color: var(--muri-danger); background: #fff1f1; }.data-entry-images button:focus-visible { outline: 3px solid rgba(15, 95, 170, .22); outline-offset: 1px; }.data-entry-images button:disabled { cursor: wait; opacity: .55; }
.vision-model-field { margin-top: 12px; }.candidate-review { display: grid; gap: 10px; margin-top: 4px; padding: 13px; border: 1px solid #e1cfaa; border-radius: 9px; background: #fffaf2; }.candidate-review header { display: flex; align-items: flex-start; justify-content: space-between; gap: 12px; }.candidate-review header > div { display: flex; min-width: 0; flex-direction: column; }.candidate-review header span { display: flex; align-items: center; gap: 6px; color: var(--muri-text); font-weight: 650; }.candidate-review header span svg { color: var(--muri-primary); }.candidate-review header small { color: var(--muri-text-tertiary); font-size: 10px; }.candidate-review :deep(.n-form-item:last-child) { margin-bottom: 0; }.data-entry-error { margin: 10px 0 0; color: var(--muri-danger); font-size: 11px; line-height: 1.5; }
.small-dialog { width: min(480px, calc(100vw - 28px)); }
.form-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; }
.dialog-actions,.detail-actions { display: flex; justify-content: flex-end; gap: 9px; }
.detail-actions { flex-wrap: wrap; margin-bottom: 14px; }
.participation-actions { display: flex; align-items: center; gap: 7px; }
.snapshot-tags { display: flex; flex-wrap: wrap; gap: 4px; margin-top: 6px; }
.snapshot-tags > span { color: var(--muri-text-tertiary); font-size: 10px; }
.observation-definition-strip { display: flex; flex-wrap: wrap; gap: 6px; margin-bottom: 10px; }
.observation-definition-strip > span { color: var(--muri-text-tertiary); font-size: 11px; }
.observation-value { display: block; max-width: 420px; margin-top: 5px; overflow: hidden; color: var(--muri-primary); font-size: 12px; font-weight: 600; text-overflow: ellipsis; white-space: nowrap; }
.observation-actions { display: flex; align-items: center; gap: 6px; }
:deep(.n-list-item small) { display: block; color: var(--muri-text-tertiary); font-weight: 400; }
@media (max-width: 900px) { .metrics { grid-template-columns: 1fr 1fr; }.experiment-card { grid-template-columns: 1fr; }.progress-panel { border-top: 1px solid var(--muri-border); border-left: 0; } }
@media (max-width: 540px) { .metrics { grid-template-columns: 1fr 1fr; }.metrics > div { grid-template-columns: 24px 1fr; }.metrics strong { grid-column: 2; }.filter-row { overflow-x: auto; justify-content: flex-start; }.form-grid { grid-template-columns: 1fr; gap: 0; }.detail-actions { flex-wrap: wrap; }.detail-actions button { flex: 1; }.data-entry-images { grid-template-columns: minmax(0, 1fr); }.data-entry-dialog :deep(.n-card__content) { padding-inline: 14px; }.data-entry-dialog :deep(.n-tabs-tab) { min-width: 0; }.candidate-review { padding: 10px; } }

.experiment-workspace { display: flex; flex-direction: column; gap: 16px; }
.workspace-header { overflow: hidden; }
.workspace-heading { padding: 18px 20px 0; }
.back-link { padding: 0; border: 0; color: var(--muri-primary); background: transparent; cursor: pointer; font-size: 12px; }
.workspace-title-row { display: flex; align-items: flex-start; justify-content: space-between; gap: 20px; margin-top: 12px; }
.workspace-title-row h1 { margin: 7px 0 4px; font-size: 24px; letter-spacing: -0.02em; }
.workspace-title-row p { margin: 0; color: var(--muri-text-secondary); font-size: 12px; }
.lifecycle-actions { display: flex; align-items: center; gap: 8px; }
.workspace-nav { display: flex; gap: 4px; margin-top: 18px; padding: 0 20px; overflow-x: auto; border-top: 1px solid var(--muri-border); }
.workspace-nav a { position: relative; padding: 13px 12px 11px; color: var(--muri-text-secondary); white-space: nowrap; }
.workspace-nav a::after { position: absolute; inset: auto 10px 0; height: 2px; border-radius: 2px; background: transparent; content: ''; }
.workspace-nav a.active { color: var(--muri-primary); font-weight: 600; }
.workspace-nav a.active::after { background: var(--muri-primary); }
.workspace-section { min-width: 0; }
.workspace-metrics { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 10px; }
.workspace-metrics article { display: flex; min-height: 106px; padding: 16px; flex-direction: column; }
.workspace-metrics span { color: var(--muri-text-secondary); font-size: 12px; }
.workspace-metrics strong { margin: 8px 0 3px; font-size: 24px; }
.workspace-metrics small { color: var(--muri-text-tertiary); }
.overview-grid,.design-grid,.trace-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 12px; margin-top: 12px; }
.overview-card,.design-panel,.trace-panel,.experiment-records { padding: 16px; }
.section-title,.section-toolbar { display: flex; align-items: center; justify-content: space-between; gap: 14px; }
.section-title > div,.section-toolbar > div:first-child { display: flex; flex-direction: column; }
.section-title span { font-weight: 700; }
.section-title small,.section-toolbar p { color: var(--muri-text-tertiary); font-size: 11px; }
.section-toolbar { margin-bottom: 12px; }
.section-toolbar h2 { margin: 0 0 3px; font-size: 20px; }
.section-toolbar p { margin: 0; }
.section-toolbar > div:last-child { display: flex; gap: 8px; }
.compact-row,.design-item { display: flex; min-height: 54px; align-items: center; justify-content: space-between; gap: 12px; padding: 10px 0; border-bottom: 1px solid var(--muri-border); }
.compact-row:last-child,.design-item:last-child { border-bottom: 0; }
.compact-row > div:first-child,.design-item > div { display: flex; min-width: 0; flex-direction: column; }
.compact-row small,.design-item small { color: var(--muri-text-tertiary); }
.compact-meta { align-items: flex-end; }
.readiness-list { display: grid; grid-template-columns: repeat(3, 1fr); gap: 8px; margin: 18px 0; }
.readiness-list span { padding: 12px; border-radius: 7px; color: var(--muri-text-secondary); background: var(--muri-surface-muted); text-align: center; }
.readiness-list b { display: block; color: var(--muri-text-primary); font-size: 20px; }
.design-panel { min-height: 220px; }
.design-wide { grid-column: 1 / -1; min-height: auto; }
.design-item code { color: var(--muri-text-tertiary); font-size: 11px; }
.timeline-chips { display: flex; flex-wrap: wrap; gap: 8px; margin-top: 16px; }
.timeline-chips > span { display: flex; min-width: 150px; padding: 10px 12px; border: 1px solid var(--muri-border); border-radius: 8px; flex-direction: column; }
.timeline-chips small { color: var(--muri-text-tertiary); }
.participant-table-wrap,.sheet { overflow: auto; }
.participant-table,.data-grid { width: 100%; border-collapse: separate; border-spacing: 0; }
.participant-table th,.participant-table td { padding: 12px; border-bottom: 1px solid var(--muri-border); text-align: left; white-space: nowrap; }
.participant-table th { color: var(--muri-text-tertiary); background: var(--muri-surface-muted); font-size: 11px; }
.procedure-list { display: flex; flex-direction: column; gap: 9px; }
.current-node { display: flex; align-items: center; justify-content: space-between; gap: 12px; padding: 14px 16px; margin-bottom: 10px; }
.current-node > div { display: flex; min-width: 0; flex-direction: column; }
.current-node span,.current-node small { color: var(--muri-text-tertiary); font-size: 11px; }
.current-node strong { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.procedure-card { display: grid; grid-template-columns: 110px minmax(0, 1fr) minmax(190px, auto); align-items: center; gap: 14px; padding: 15px 17px; }
.procedure-card h3 { margin: 0 0 3px; font-size: 15px; }
.procedure-card p { margin: 0; color: var(--muri-text-tertiary); font-size: 11px; }
.procedure-status { display: flex; align-items: center; gap: 7px; }
.status-dot { width: 8px; height: 8px; border-radius: 50%; background: var(--muri-warning); }
.status-dot.completed { background: var(--muri-success); }
.status-dot.skipped,.status-dot.cancelled { background: var(--muri-text-tertiary); }
.procedure-time { display: flex; align-items: flex-end; flex-direction: column; white-space: nowrap; }
.procedure-time small { color: var(--muri-text-tertiary); }
.experiment-attachments { padding: 16px; margin-top: 10px; }
.data-attachments { margin-top: 12px; }
.attachment-list { display: flex; flex-direction: column; gap: 8px; margin-top: 12px; }
.attachment-row { display: flex; min-height: 50px; align-items: center; justify-content: space-between; gap: 12px; padding: 10px 0; border-bottom: 1px solid var(--muri-border); }
.attachment-row:last-child { border-bottom: 0; }
.attachment-row > div:first-child { display: flex; min-width: 0; flex-direction: column; }
.attachment-row strong { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.attachment-row span { color: var(--muri-text-tertiary); font-size: 11px; }
.attachment-row > div:last-child { display: flex; gap: 7px; }
.data-section :deep(.n-tabs-nav) { margin-bottom: 10px; }
.sheet { max-height: calc(100vh - 310px); }
.data-grid { min-width: max-content; }
.data-grid th,.data-grid td { min-width: 120px; height: 46px; padding: 0; border-right: 1px solid var(--muri-border); border-bottom: 1px solid var(--muri-border); background: #fff; text-align: center; }
.data-grid th { padding: 8px 10px; background: var(--muri-surface-muted); color: var(--muri-text-secondary); font-size: 11px; }
.data-grid th strong,.data-grid th small { display: block; }
.data-grid th small { color: var(--muri-text-tertiary); font-weight: 400; }
.data-grid .event-heading { color: var(--muri-primary); background: #edf5fc; }
.data-grid .frozen { position: sticky; z-index: 2; text-align: left; }
.data-grid th.frozen { z-index: 4; }
.data-grid .frozen-id { left: 0; min-width: 128px; padding: 0 12px; }
.data-grid .frozen-group { left: 128px; min-width: 110px; padding: 0 12px; box-shadow: 5px 0 10px rgba(27,50,73,.05); }
.data-cell { width: 100%; height: 45px; padding: 0 9px; overflow: hidden; border: 0; color: var(--muri-text-tertiary); background: transparent; cursor: pointer; text-overflow: ellipsis; white-space: nowrap; }
.data-cell:hover:not(:disabled),.data-cell:focus-visible { outline: 2px solid var(--muri-primary); outline-offset: -2px; background: var(--muri-primary-soft); }
.data-cell.filled { color: var(--muri-text-primary); font-weight: 600; }
.data-cell:disabled { cursor: default; }
.trace-item { position: relative; display: grid; grid-template-columns: 14px 1fr; gap: 8px; padding: 12px 0; border-bottom: 1px solid var(--muri-border); }
.trace-marker { width: 8px; height: 8px; margin-top: 5px; border-radius: 50%; background: var(--muri-primary); box-shadow: 0 0 0 4px var(--muri-primary-soft); }
.trace-item p { margin: 3px 0; color: var(--muri-text-secondary); }
.trace-item small { color: var(--muri-text-tertiary); }
.experiment-records { min-height: 180px; }
@media (max-width: 1050px) { .workspace-metrics { grid-template-columns: 1fr 1fr; }.overview-grid,.design-grid,.trace-grid { grid-template-columns: 1fr; }.design-wide { grid-column: auto; }.procedure-card { grid-template-columns: 100px 1fr; }.procedure-time { grid-column: 2; align-items: flex-start; } }
@media (max-width: 620px) { .workspace-heading { padding: 14px 14px 0; }.workspace-title-row { flex-direction: column; }.workspace-title-row h1 { font-size: 20px; }.workspace-nav { padding: 0 8px; }.workspace-metrics { grid-template-columns: 1fr 1fr; }.section-toolbar,.current-node,.attachment-row { align-items: flex-start; flex-direction: column; }.procedure-card { grid-template-columns: 1fr; }.procedure-time { grid-column: auto; align-items: flex-start; }.readiness-list { grid-template-columns: 1fr; } }

</style>
