// In-memory demo transport for `pnpm dev` and the layout-only Playwright suite.
// It mirrors server rules for UI feedback but is not evidence of backend behavior.
import type {
  AiAutonomyUpdateInput,
  AiAutonomyView,
  AiConversationDetail,
  AiConversationListInput,
  AiConversationSummary,
  AiConversationUpdateInput,
  AiDraftDecisionInput,
  AiDraftDecisionResponse,
  AiSettings,
  AiSource,
  AiSourceArchiveInput,
  AiSourceListInput,
  AiSourceUploadInput,
  AiTurnInput,
  AiTurnResponse,
  AiWriteDraft,
  Animal,
  AnimalDetail,
  AnimalDraft,
  AnimalGenotype,
  AnimalSample,
  BreedingLine,
  BreedingPair,
  Cage,
  Cohort,
  Colony,
  CreatedLitter,
  Experiment,
  ExperimentEvent,
  ExperimentTemplateSummary,
  GeneAllele,
  GeneLocus,
  GenotypeDefinition,
  GenotypingRecord,
  Litter,
  LocusPrediction,
  MatingEvent,
  Observation,
  ObservationDefinition,
  ObservationValueData,
  ObservationValueRecord,
  Participation,
  PedigreeRelation,
  Procedure,
  ProjectSummary,
  RecordedObservation,
  RegisteredAnimalDraft,
  SaveAiSettingsInput,
  StartAiConversationInput,
  StartAiConversationResponse,
  TimelineEvent,
  WorkspaceSettings,
} from '@/domain/models'
import { builtinAiProviderPresets } from './aiProviderPresets'
// Only used inside method bodies, so the gateway.ts <-> demoGateway.ts cycle is safe at module init.
import { defaultAiAutonomy, valueRecord } from './gateway'
import type {
  AiExtractionRecord,
  AiModelDefaultsView,
  AiModelProfileView,
  AiProviderEndpoint,
  AnimalAccessContext,
  AppliedAiExtraction,
  ApproveAiExtractionInput,
  BreedingPredictionInput,
  CorrectGenotypingRecordInput,
  CreateAiExtractionInput,
  CreateAlleleInput,
  CreateAnimalInput,
  CreateAnimalSampleInput,
  CreateBreedingLineInput,
  CreateBreedingPairInput,
  CreateCageInput,
  CreateCohortInput,
  CreateColonyInput,
  CreateExperimentEventInput,
  CreateExperimentInput,
  CreateGeneLocusInput,
  CreateGenotypeDefinitionInput,
  CreateGenotypeInput,
  CreateGenotypingRecordInput,
  CreateLitterInput,
  CreateMatingEventInput,
  CreateObservationDefinitionInput,
  CreatePedigreeInput,
  CreateProcedureInput,
  CreateProjectInput,
  CreatePublishedTemplateInput,
  EnrollAnimalInput,
  GeneticsArchiveInput,
  MuriArcGateway,
  ObservationFilter,
  PrivateImageRecord,
  RecordObservationInput,
  RegisterAnimalDraftInput,
  RejectAiExtractionInput,
  RetireBreedingPairInput,
  ReviseObservationInput,
  SaveAiModelDefaultsInput,
  SaveAiModelProfileInput,
  SaveAiProviderEndpointInput,
  ValidateAiModelProfileInput,
  VoidGenotypingRecordInput,
} from './gateway'
import { seedAnimals, seedCages, seedDataJobs, seedExperiments } from './mock-data'

const clone = <T>(value: T): T => structuredClone(value)
const DEMO_AI_SOURCE_IMAGE_MEDIA_TYPES = new Set([
  'image/jpeg',
  'image/png',
  'image/tiff',
])

function isDemoAiSourceImage(source: AiSource): boolean {
  const mediaType = source.mediaType.split(';', 1)[0]?.trim().toLowerCase() ?? ''
  if (DEMO_AI_SOURCE_IMAGE_MEDIA_TYPES.has(mediaType)) return true
  if (mediaType && mediaType !== 'application/octet-stream') return false
  const extension = source.fileName.split('.').at(-1)?.toLowerCase()
  return extension === 'png'
    || extension === 'jpg'
    || extension === 'jpeg'
    || extension === 'tif'
    || extension === 'tiff'
}

class DemoDomainStore {
  cages = clone(seedCages)
  animals = clone(seedAnimals)
  projects: ProjectSummary[] = [
    { id: 'demo-project-1', name: 'DEMO-GeneA' },
    { id: 'demo-project-2', name: 'GeneA 繁育' },
  ]
  templates: ExperimentTemplateSummary[] = [
    { id: 'demo-template-1', name: '常规动物观察', version: 1 },
  ]
  experiments = clone(seedExperiments)
  cohorts: Cohort[] = []
  participations: Participation[] = []
  procedures: Procedure[] = []
  samples: AnimalSample[] = []
  pedigree: PedigreeRelation[] = []
  geneLoci: GeneLocus[] = []
  alleles: GeneAllele[] = []
  genotypes: AnimalGenotype[] = []
  genotypeDefinitions: GenotypeDefinition[] = []
  genotypingRecords: GenotypingRecord[] = []
  breedingLines: BreedingLine[] = []
  colonies: Colony[] = []
  breedingPairs: BreedingPair[] = []
  matingEvents: MatingEvent[] = []
  litters: Litter[] = []
  animalDrafts: AnimalDraft[] = []
  experimentEvents: ExperimentEvent[] = []
  observationDefinitions: ObservationDefinition[] = []
  observations: Observation[] = []
  observationValues: ObservationValueRecord[] = []
  workspaceSettings: WorkspaceSettings = {
    labName: '我的动物实验室',
    operatorName: '本地操作者',
  }
  aiSettings: AiSettings = {
    enabled: true,
    providerKind: 'open_ai_compatible',
    providerPresetId: 'deepseek',
    model: 'deepseek-chat',
    baseUrl: 'https://api.deepseek.com',
    hasKey: false,
    supportsVision: false,
    contextWindowTokens: 131072,
    maxInputTokens: 65536,
    maxOutputTokens: 4096,
    historyTokenBudget: 32768,
    historyTurns: 20,
    temperature: 0,
    timeoutMs: 120000,
    revision: 0,
  }
  aiModelProfiles: AiModelProfileView[] = [
    {
      id: '00000000-0000-4000-8000-000000000101',
      name: 'DeepSeek 对话',
      currentVersion: 1,
      revision: 1,
      protocol: 'openai_chat_completions',
      transport: 'open_ai_compatible',
      baseUrl: 'https://api.deepseek.com',
      modelId: 'deepseek-chat',
      supportsVision: false,
      contextWindowTokens: 131072,
      maxInputTokens: 65536,
      maxOutputTokens: 4096,
      historyTokenBudget: 32768,
      historyTurns: 20,
      temperature: 0,
      timeoutMs: 120000,
      hasKey: false,
      isDefaultConversation: true,
      isDefaultVision: false,
    },
    {
      id: '00000000-0000-4000-8000-000000000102',
      name: '本地视觉模型',
      currentVersion: 1,
      revision: 1,
      protocol: 'openai_chat_completions',
      transport: 'local_http',
      baseUrl: 'http://127.0.0.1:11434/v1',
      modelId: 'local-vision',
      supportsVision: true,
      contextWindowTokens: 32768,
      maxInputTokens: 24576,
      maxOutputTokens: 4096,
      historyTokenBudget: 12288,
      historyTurns: 12,
      temperature: 0,
      timeoutMs: 120000,
      hasKey: false,
      isDefaultConversation: false,
      isDefaultVision: true,
    },
  ]
  aiModelDefaults: AiModelDefaultsView = {
    defaultConversationProfileId: '00000000-0000-4000-8000-000000000101',
    defaultVisionProfileId: '00000000-0000-4000-8000-000000000102',
    revision: 1,
  }
  aiProviderEndpoints: AiProviderEndpoint[] = builtinAiProviderPresets
    .filter((preset) => preset.recommendedBaseUrl)
    .map((preset, index) => ({
      id: `00000000-0000-0000-0000-${String(index + 1).padStart(12, '0')}`,
      providerKind: preset.providerKind,
      protocol: 'openai_chat_completions',
      label: `${preset.displayName} API`,
      baseUrl: preset.recommendedBaseUrl,
      enabled: true,
      builtin: true,
      revision: 1,
    }))

  createCage(input: CreateCageInput) {
    if (this.cages.some((cage) => cage.room === input.room && cage.code.toLowerCase() === input.code.toLowerCase())) {
      throw new Error(`笼位 ${input.code} 已存在`)
    }
    const cage: Cage = {
      id: crypto.randomUUID(),
      ...input,
      animalIds: [],
      status: 'empty',
      summary: '空笼',
    }
    this.cages.push(cage)
    return clone(cage)
  }

  createAnimal(input: CreateAnimalInput) {
    const project = input.identifierScope === 'project'
      ? this.projects.find((item) => item.id === input.projectId)
      : undefined
    if (input.identifierScope === 'project' && !project) throw new Error('请选择有效项目')
    if (this.animals.some((animal) => animal.code.toLowerCase() === input.displayId.toLowerCase())) {
      throw new Error(`小鼠编号 ${input.displayId} 已存在`)
    }
    if (input.cageId) {
      const cage = this.cages.find((item) => item.id === input.cageId)
      if (!cage) throw new Error('笼位不存在')
      if (cage.animalIds.length >= cage.capacity) throw new Error('笼位容量不足')
    }
    const initialInputs = input.initialGenotypingRecords ?? []
    const definitionIds = new Set<string>()
    const initialDefinitions = initialInputs.map((record) => {
      if (definitionIds.has(record.genotypeDefinitionId)) {
        throw new Error('同一基因型定义不能在登记时重复选择')
      }
      definitionIds.add(record.genotypeDefinitionId)
      const definition = this.genotypeDefinitions.find((candidate) =>
        candidate.id === record.genotypeDefinitionId && !candidate.archivedAt)
      if (!definition) throw new Error('初始基因型定义不存在或已归档')
      if ((record.state === 'confirmed' || record.state === 'rejected') && !record.assessedAt) {
        throw new Error('确认或排除结果必须填写检测时间')
      }
      return definition
    })
    const now = new Date().toISOString()
    const animalId = crypto.randomUUID()
    const initialRecords = initialInputs.map<GenotypingRecord>((record) => ({
      id: crypto.randomUUID(),
      projectId: project?.id,
      animalId,
      genotypeDefinitionId: record.genotypeDefinitionId,
      state: record.state,
      assessedAt: record.assessedAt,
      method: record.method?.trim() || undefined,
      notes: record.notes?.trim() || undefined,
      revision: 1,
      createdAt: now,
      updatedAt: now,
    }))
    const animal: Animal = {
      id: animalId,
      code: input.displayId,
      sex: input.sex,
      strain: input.strain || '未设置',
      genotype: initialDefinitions.map((definition, index) =>
        `${definition.name} · ${initialInputs[index]?.state ?? 'expected'}`).join(' · ') || '待确认',
      birthDate: input.birthDate ?? '',
      status: 'active',
      cageId: input.cageId ?? null,
      projectNames: project ? [project.name] : [],
      timeline: [{
        id: crypto.randomUUID(),
        at: now,
        type: input.birthDate ? 'birth' : 'note',
        title: input.birthDate ? '出生登记' : '登记动物',
        detail: input.birthDate ? `出生日期 ${input.birthDate}` : '创建动物档案',
        operator: '演示操作员',
      }],
    }
    for (const [index, record] of initialRecords.entries()) {
      const definition = initialDefinitions[index]
      animal.timeline.unshift({
        id: crypto.randomUUID(),
        at: record.assessedAt ?? now,
        type: 'genotype',
        title: '初始基因型',
        detail: `${definition?.name ?? record.genotypeDefinitionId} · ${record.state}`,
        operator: '演示操作员',
      })
    }
    this.animals.push(animal)
    this.genotypingRecords.push(...initialRecords)
    const cage = this.cages.find((item) => item.id === input.cageId)
    if (cage) cage.animalIds.push(animal.id)
    return clone(animal)
  }

  moveAnimals(animalIds: string[], targetCageId: string) {
    const target = this.cages.find((cage) => cage.id === targetCageId)
    if (!target) throw new Error('目标笼位不存在')
    const uniqueIds = [...new Set(animalIds)]
    const newResidents = uniqueIds.filter((id) => !target.animalIds.includes(id))
    if (target.animalIds.length + newResidents.length > target.capacity) {
      throw new Error(`目标笼位容量不足（${target.animalIds.length}/${target.capacity}）`)
    }
    if (uniqueIds.some((id) => !this.animals.some((animal) => animal.id === id))) {
      throw new Error('包含不存在的动物')
    }

    for (const cage of this.cages) {
      cage.animalIds = cage.animalIds.filter((id) => !uniqueIds.includes(id))
      cage.status = cage.animalIds.length ? (cage.status === 'attention' ? 'attention' : 'normal') : 'empty'
    }
    for (const animalId of uniqueIds) {
      const animal = this.animals.find((item) => item.id === animalId)!
      animal.cageId = targetCageId
      const event: TimelineEvent = {
        id: crypto.randomUUID(),
        at: new Date().toISOString(),
        type: 'transfer',
        title: '转入笼位',
        detail: `转入 ${target.code}`,
        operator: '演示操作员',
      }
      animal.timeline.unshift(event)
    }
    target.animalIds.push(...newResidents)
    target.status = 'normal'
    target.summary = `${target.animalIds.length} 只动物`
  }
}

const pause = (duration: number) => new Promise((resolve) => globalThis.setTimeout(resolve, duration))

function compareAiConversations(left: AiConversationSummary, right: AiConversationSummary): number {
  const pinOrder = Number(Boolean(right.pinnedAt)) - Number(Boolean(left.pinnedAt))
  return pinOrder || right.updatedAt.localeCompare(left.updatedAt)
}

interface DemoConversationModelBinding {
  profileId: string
  profileVersion: number
  profileName: string
  modelId: string
  supportsVision: boolean
}

function assertDemoObservationValue(
  definition: ObservationDefinition,
  value: ObservationValueData,
) {
  if (definition.valueType !== value.type
    || (value.type === 'number' && !Number.isFinite(value.value))
    || (value.type === 'category' && !definition.categories.includes(value.value))) {
    throw new Error('观察值与定义的数据类型或类别约束不匹配')
  }
}

/** Browser-only demo adapter. It is never selected for a packaged build unless explicitly configured. */
export class DemoGateway implements MuriArcGateway {
  readonly mode = 'local' as const
  readonly displayName = '浏览器演示数据'
  private readonly store = new DemoDomainStore()
  private readonly aiConversations = new Map<string, AiConversationDetail>()
  private readonly aiSources = new Map<string, AiSource>()
  private readonly aiConversationAutonomy = new Map<string, AiAutonomyView>()
  private readonly aiConversationModels = new Map<string, DemoConversationModelBinding>()
  private readonly privateImages: PrivateImageRecord[] = []
  private readonly privateImageFiles = new Map<string, Blob>()
  private readonly aiExtractions: AiExtractionRecord[] = []

  private setDemoArchive<T extends {
    id: string
    revision: number
    archivedAt?: string
    updatedAt?: string
  }>(items: T[], input: GeneticsArchiveInput, archived: boolean): T {
    const item = items.find((candidate) => candidate.id === input.id)
    if (!item || item.revision !== input.expectedRevision || Boolean(item.archivedAt) === archived) {
      throw new Error('目录记录已变化，请刷新后重试')
    }
    const now = new Date().toISOString()
    item.archivedAt = archived ? now : undefined
    if ('updatedAt' in item) item.updatedAt = now
    item.revision += 1
    return clone(item)
  }

  async listCages(_context?: AnimalAccessContext) { await pause(20); return clone(this.store.cages) }
  async createCage(input: CreateCageInput) { await pause(20); return this.store.createCage(input) }
  async createAnimal(input: CreateAnimalInput) { await pause(20); return this.store.createAnimal(input) }
  async listAnimals(_context?: AnimalAccessContext) { await pause(20); return clone(this.store.animals) }
  async getAnimal(id: string, _context?: AnimalAccessContext) {
    await pause(20)
    const animal = this.store.animals.find((item) => item.id === id)
    return animal ? clone(animal) : undefined
  }
  async getAnimalDetail(id: string, _context?: AnimalAccessContext): Promise<AnimalDetail> {
    await pause(20)
    const animal = this.store.animals.find((item) => item.id === id)
    if (!animal) throw new Error('动物不存在')
    return {
      timeline: clone(animal.timeline),
      experiments: [],
      measurements: [],
      pedigree: clone(this.store.pedigree.filter((relation) =>
        relation.relatedAnimal.id === id || relation.id.startsWith(`${id}:`))),
      samples: clone(this.store.samples.filter((sample) => sample.projectId && sample.id.startsWith(`${id}:`))),
      attachments: [],
      auditVisible: true,
      audits: [],
      provenance: [],
    }
  }
  async listGeneLoci(_projectId?: string, includeArchived = false) {
    await pause(20)
    return clone(this.store.geneLoci.filter((locus) => includeArchived || !locus.archivedAt))
  }
  async geneLocusReferences(id: string, _projectId?: string) {
    await pause(20)
    const definitionIds = this.store.genotypeDefinitions
      .filter((definition) => definition.components.some((component) => component.locusId === id))
      .map((definition) => definition.id)
    const activeDefinitionIds = new Set(this.store.genotypeDefinitions
      .filter((definition) => !definition.archivedAt && definitionIds.includes(definition.id))
      .map((definition) => definition.id))
    return {
      activeGenotypeDefinitions: activeDefinitionIds.size,
      genotypeDefinitions: definitionIds.length,
      genotypingRecords: this.store.genotypingRecords.filter((record) => definitionIds.includes(record.genotypeDefinitionId)).length,
      breedingLines: this.store.breedingLines.filter((line) => line.genotypeDefinitionIds.some((definitionId) => definitionIds.includes(definitionId))).length,
    }
  }
  async archiveGeneLocus(input: GeneticsArchiveInput) {
    const references = await this.geneLocusReferences(input.id)
    if (references.activeGenotypeDefinitions) throw new Error('该位点仍被活动基因型定义引用')
    return this.setDemoArchive(this.store.geneLoci, input, true)
  }
  async restoreGeneLocus(input: GeneticsArchiveInput) {
    return this.setDemoArchive(this.store.geneLoci, input, false)
  }
  async createGeneLocus(input: CreateGeneLocusInput) {
    await pause(20)
    const symbol = input.symbol.trim()
    if (!symbol) throw new Error('请输入基因位点')
    const existing = this.store.geneLoci.find((locus) => locus.symbol.toLowerCase() === symbol.toLowerCase())
    if (existing) return clone(existing)
    const locus: GeneLocus = { id: crypto.randomUUID(), symbol, description: input.description, revision: 1 }
    this.store.geneLoci.push(locus)
    return clone(locus)
  }
  async listAlleles(locusId: string, _projectId?: string, includeArchived = false) {
    await pause(20)
    return clone(this.store.alleles.filter((allele) =>
      allele.locusId === locusId && (includeArchived || !allele.archivedAt)))
  }
  async alleleReferences(id: string, _projectId?: string) {
    await pause(20)
    const definitionIds = this.store.genotypeDefinitions
      .filter((definition) => definition.components.some((component) =>
        component.allele1Id === id || component.allele2Id === id))
      .map((definition) => definition.id)
    return {
      activeGenotypeDefinitions: this.store.genotypeDefinitions.filter((definition) =>
        !definition.archivedAt && definitionIds.includes(definition.id)).length,
      genotypeDefinitions: definitionIds.length,
      genotypingRecords: this.store.genotypingRecords.filter((record) => definitionIds.includes(record.genotypeDefinitionId)).length,
      breedingLines: this.store.breedingLines.filter((line) => line.genotypeDefinitionIds.some((definitionId) => definitionIds.includes(definitionId))).length,
    }
  }
  async archiveAllele(input: GeneticsArchiveInput) {
    const references = await this.alleleReferences(input.id)
    if (references.activeGenotypeDefinitions) throw new Error('该 allele 仍被活动基因型定义引用')
    return this.setDemoArchive(this.store.alleles, input, true)
  }
  async restoreAllele(input: GeneticsArchiveInput) {
    const allele = this.store.alleles.find((candidate) => candidate.id === input.id)
    if (allele && this.store.geneLoci.find((locus) => locus.id === allele.locusId)?.archivedAt) {
      throw new Error('请先恢复所属位点')
    }
    return this.setDemoArchive(this.store.alleles, input, false)
  }
  async createAllele(input: CreateAlleleInput) {
    await pause(20)
    if (!this.store.geneLoci.some((locus) => locus.id === input.locusId)) throw new Error('基因位点不存在')
    const symbol = input.symbol.trim()
    if (!symbol) throw new Error('请输入等位基因')
    const existing = this.store.alleles.find((allele) => allele.locusId === input.locusId && allele.symbol.toLowerCase() === symbol.toLowerCase())
    if (existing) return clone(existing)
    const allele: GeneAllele = {
      id: crypto.randomUUID(), locusId: input.locusId, symbol,
      description: input.description, isWildType: input.isWildType, revision: 1,
    }
    this.store.alleles.push(allele)
    return clone(allele)
  }
  async listGenotypes(animalId: string, _projectId?: string) {
    await pause(20)
    return clone(this.store.genotypes.filter((genotype) => genotype.animalId === animalId))
  }
  async createGenotype(input: CreateGenotypeInput) {
    await pause(20)
    const animal = this.store.animals.find((candidate) => candidate.id === input.animalId)
    const locus = this.store.geneLoci.find((candidate) => candidate.id === input.locusId)
    if (!animal || !locus) throw new Error('动物或基因位点不存在')
    const genotype: AnimalGenotype = {
      id: crypto.randomUUID(), animalId: input.animalId, locusId: input.locusId,
      allele1Id: input.allele1Id, allele2Id: input.allele2Id,
      assessedAt: input.assessedAt, revision: 1,
    }
    this.store.genotypes.push(genotype)
    const labels = this.store.genotypes.filter((row) => row.animalId === animal.id).map((row) => {
      const rowLocus = this.store.geneLoci.find((candidate) => candidate.id === row.locusId)
      const first = this.store.alleles.find((candidate) => candidate.id === row.allele1Id)?.symbol ?? '?'
      const second = this.store.alleles.find((candidate) => candidate.id === row.allele2Id)?.symbol ?? '?'
      return `${rowLocus?.symbol ?? '未知位点'} ${first}/${second}`
    })
    animal.genotype = labels.join(' · ') || '待确认'
    animal.timeline.unshift({
      id: crypto.randomUUID(), at: input.assessedAt ?? new Date().toISOString(), type: 'genotype',
      title: '基因型记录', detail: animal.genotype, operator: '演示操作员',
    })
    return clone(genotype)
  }
  async listGenotypeDefinitions(_projectId?: string, includeArchived = false) {
    await pause(20)
    return clone(this.store.genotypeDefinitions.filter((definition) =>
      includeArchived || !definition.archivedAt))
  }
  async genotypeDefinitionReferences(id: string, _projectId?: string) {
    await pause(20)
    return {
      activeGenotypeDefinitions: 0,
      genotypeDefinitions: 0,
      genotypingRecords: this.store.genotypingRecords.filter((record) => record.genotypeDefinitionId === id).length,
      breedingLines: this.store.breedingLines.filter((line) => line.genotypeDefinitionIds.includes(id)).length,
    }
  }
  async archiveGenotypeDefinition(input: GeneticsArchiveInput) {
    return this.setDemoArchive(this.store.genotypeDefinitions, input, true)
  }
  async restoreGenotypeDefinition(input: GeneticsArchiveInput) {
    const definition = this.store.genotypeDefinitions.find((candidate) => candidate.id === input.id)
    if (definition?.components.some((component) =>
      this.store.geneLoci.find((locus) => locus.id === component.locusId)?.archivedAt
      || this.store.alleles.find((allele) => allele.id === component.allele1Id)?.archivedAt
      || (component.allele2Id && this.store.alleles.find((allele) => allele.id === component.allele2Id)?.archivedAt))) {
      throw new Error('请先恢复定义引用的位点和 allele')
    }
    return this.setDemoArchive(this.store.genotypeDefinitions, input, false)
  }
  async createGenotypeDefinition(input: CreateGenotypeDefinitionInput) {
    await pause(20)
    const name = input.name.trim()
    if (!name || !input.components.length) throw new Error('基因型定义至少包含一个组件')
    const loci = new Set<string>()
    const displayOrders = new Set<number>()
    for (const component of input.components) {
      const locus = this.store.geneLoci.find((item) => item.id === component.locusId)
      const first = this.store.alleles.find((item) => item.id === component.allele1Id)
      const second = component.allele2Id
        ? this.store.alleles.find((item) => item.id === component.allele2Id)
        : undefined
      const requiresSecond = component.mode === 'diploid' || component.mode === 'conditional'
      if (!locus || !first || first.locusId !== locus.id
        || (requiresSecond && (!second || second.locusId !== locus.id))
        || (!requiresSecond && component.allele2Id)
        || component.displayOrder < 0
        || loci.has(component.locusId)
        || displayOrders.has(component.displayOrder)) {
        throw new Error('基因型组件配置无效或存在重复位点')
      }
      loci.add(component.locusId)
      displayOrders.add(component.displayOrder)
    }
    const now = new Date().toISOString()
    const definitionId = crypto.randomUUID()
    const definition: GenotypeDefinition = {
      id: definitionId,
      name,
      description: input.description?.trim() || undefined,
      components: input.components.map((component) => ({
        id: crypto.randomUUID(),
        genotypeDefinitionId: definitionId,
        locusId: component.locusId,
        allele1Id: component.allele1Id,
        allele2Id: component.allele2Id,
        mode: component.mode,
        displayOrder: component.displayOrder,
        revision: 1,
      })),
      revision: 1,
      createdAt: now,
      updatedAt: now,
    }
    this.store.genotypeDefinitions.push(definition)
    return clone(definition)
  }
  async listGenotypingRecords(animalId: string, _projectId?: string) {
    await pause(20)
    return clone(this.store.genotypingRecords.filter((record) => record.animalId === animalId))
  }
  async createGenotypingRecord(input: CreateGenotypingRecordInput) {
    await pause(20)
    const animal = this.store.animals.find((item) => item.id === input.animalId)
    const definition = this.store.genotypeDefinitions.find(
      (item) => item.id === input.genotypeDefinitionId,
    )
    if (!animal || !definition) throw new Error('动物或基因型定义不存在')
    if ((input.state === 'confirmed' || input.state === 'rejected') && !input.assessedAt) {
      throw new Error('确认或排除结果必须填写检测时间')
    }
    const now = new Date().toISOString()
    const record: GenotypingRecord = {
      id: crypto.randomUUID(),
      projectId: input.projectId,
      animalId: input.animalId,
      genotypeDefinitionId: input.genotypeDefinitionId,
      state: input.state,
      assessedAt: input.assessedAt,
      method: input.method?.trim() || undefined,
      notes: input.notes?.trim() || undefined,
      revision: 1,
      createdAt: now,
      updatedAt: now,
    }
    this.store.genotypingRecords.push(record)
    animal.timeline.unshift({
      id: crypto.randomUUID(), at: input.assessedAt ?? now, type: 'genotype',
      title: '基因检测', detail: `${definition.name} · ${input.state}`,
      operator: '演示操作员',
    })
    return clone(record)
  }
  async voidGenotypingRecord(input: VoidGenotypingRecordInput) {
    await pause(20)
    const record = this.store.genotypingRecords.find((candidate) => candidate.id === input.recordId)
    const reason = input.reason.trim()
    if (!record || record.revision !== input.expectedRevision || record.voidedAt) {
      throw new Error('检测记录已变化，请刷新后重试')
    }
    if (!reason) throw new Error('请填写作废原因')
    const now = new Date().toISOString()
    record.voidedAt = now
    record.voidReason = reason
    record.updatedAt = now
    record.revision += 1
    return clone(record)
  }
  async correctGenotypingRecord(input: CorrectGenotypingRecordInput) {
    await pause(20)
    const original = this.store.genotypingRecords.find((candidate) => candidate.id === input.recordId)
    const definition = this.store.genotypeDefinitions.find((candidate) =>
      candidate.id === input.genotypeDefinitionId && !candidate.archivedAt)
    if (!original || original.revision !== input.expectedRevision || original.voidedAt) {
      throw new Error('检测记录已变化，请刷新后重试')
    }
    if (!definition) throw new Error('替代基因型定义不存在或已归档')
    if (!input.reason.trim()) throw new Error('请填写更正原因')
    if ((input.state === 'confirmed' || input.state === 'rejected') && !input.assessedAt) {
      throw new Error('确认或排除结果必须填写检测时间')
    }
    const now = new Date().toISOString()
    const replacement: GenotypingRecord = {
      id: crypto.randomUUID(),
      projectId: original.projectId,
      animalId: original.animalId,
      genotypeDefinitionId: input.genotypeDefinitionId,
      state: input.state,
      assessedAt: input.assessedAt,
      method: input.method?.trim() || undefined,
      notes: input.notes?.trim() || undefined,
      supersedesRecordId: original.id,
      revision: 1,
      createdAt: now,
      updatedAt: now,
    }
    original.voidedAt = now
    original.voidReason = input.reason.trim()
    original.updatedAt = now
    original.revision += 1
    this.store.genotypingRecords.push(replacement)
    return { voided: clone(original), replacement: clone(replacement) }
  }
  async listBreedingLines() {
    await pause(20)
    return clone(this.store.breedingLines)
  }
  async createBreedingLine(input: CreateBreedingLineInput) {
    await pause(20)
    if (!input.name.trim() || !input.genotypeDefinitionIds.length
      || input.genotypeDefinitionIds.some((id) => !this.store.genotypeDefinitions.some((item) => item.id === id))) {
      throw new Error('繁育品系必须关联至少一个有效基因型定义')
    }
    const line: BreedingLine = {
      id: crypto.randomUUID(), name: input.name.trim(),
      description: input.description?.trim() || undefined,
      genotypeDefinitionIds: [...new Set(input.genotypeDefinitionIds)],
      revision: 1, createdAt: new Date().toISOString(),
    }
    this.store.breedingLines.push(line)
    return clone(line)
  }
  async listColonies(breedingLineId?: string) {
    await pause(20)
    return clone(this.store.colonies.filter(
      (colony) => !breedingLineId || colony.breedingLineId === breedingLineId,
    ))
  }
  async createColony(input: CreateColonyInput) {
    await pause(20)
    if (!input.name.trim() || !this.store.breedingLines.some((line) => line.id === input.breedingLineId)) {
      throw new Error('请选择有效繁育品系并填写 Colony 名称')
    }
    const colony: Colony = {
      id: crypto.randomUUID(), breedingLineId: input.breedingLineId,
      name: input.name.trim(), description: input.description?.trim() || undefined,
      revision: 1, createdAt: new Date().toISOString(),
    }
    this.store.colonies.push(colony)
    return clone(colony)
  }
  async listBreedingPairs(colonyId?: string) {
    await pause(20)
    return clone(this.store.breedingPairs.filter((pair) => !colonyId || pair.colonyId === colonyId))
  }
  async createBreedingPair(input: CreateBreedingPairInput) {
    await pause(20)
    const male = this.store.animals.find((animal) => animal.id === input.maleAnimalId)
    const females = input.femaleAnimalIds.map((id) => this.store.animals.find((animal) => animal.id === id))
    const memberIds = [input.maleAnimalId, ...input.femaleAnimalIds]
    const alreadyActive = this.store.breedingPairs.some((pair) => pair.status === 'active'
      && pair.members.some((member) => memberIds.includes(member.animalId)))
    if (!input.name.trim() || !this.store.colonies.some((colony) => colony.id === input.colonyId)
      || !male || male.sex !== 'male' || !females.length
      || females.some((female) => !female || female.sex !== 'female')
      || new Set(memberIds).size !== memberIds.length || alreadyActive) {
      throw new Error('配对必须包含一只未占用雄鼠和至少一只未占用雌鼠')
    }
    const now = new Date().toISOString()
    const startedAt = input.startedAt ?? now
    const pairId = crypto.randomUUID()
    const pair: BreedingPair = {
      id: pairId, colonyId: input.colonyId, name: input.name.trim(), status: 'active',
      startedAt, members: [
        { id: crypto.randomUUID(), breedingPairId: pairId, animalId: male.id, role: 'male', joinedAt: startedAt, revision: 1 },
        ...input.femaleAnimalIds.map((animalId) => ({
          id: crypto.randomUUID(), breedingPairId: pairId, animalId,
          role: 'female' as const, joinedAt: startedAt, revision: 1,
        })),
      ],
      revision: 1, createdAt: now,
    }
    this.store.breedingPairs.push(pair)
    return clone(pair)
  }
  async retireBreedingPair(input: RetireBreedingPairInput) {
    await pause(20)
    const pair = this.store.breedingPairs.find((item) => item.id === input.id)
    if (!pair || pair.status !== 'active') throw new Error('繁育配对不存在或已退役')
    if (pair.revision !== input.expectedRevision) throw new Error('配对已更新，请刷新后重试')
    const endedAt = input.endedAt ?? new Date().toISOString()
    pair.status = 'retired'; pair.endedAt = endedAt; pair.revision += 1
    pair.members.forEach((member) => { member.leftAt = endedAt; member.revision += 1 })
    return clone(pair)
  }
  async listMatingEvents(breedingPairId: string) {
    await pause(20)
    return clone(this.store.matingEvents.filter((event) => event.breedingPairId === breedingPairId))
  }
  async createMatingEvent(input: CreateMatingEventInput) {
    await pause(20)
    const pair = this.store.breedingPairs.find((item) => item.id === input.breedingPairId)
    if (!pair || pair.status !== 'active'
      || !pair.members.some((member) => member.animalId === input.maleAnimalId && member.role === 'male')
      || !pair.members.some((member) => member.animalId === input.femaleAnimalId && member.role === 'female')) {
      throw new Error('交配事件只能引用活跃配对中的雄鼠和雌鼠')
    }
    const event: MatingEvent = {
      id: crypto.randomUUID(), breedingPairId: input.breedingPairId,
      maleAnimalId: input.maleAnimalId, femaleAnimalId: input.femaleAnimalId,
      occurredAt: input.occurredAt ?? new Date().toISOString(),
      notes: input.notes?.trim() || undefined, revision: 1,
    }
    this.store.matingEvents.push(event)
    return clone(event)
  }
  async listLitters(breedingPairId: string) {
    await pause(20)
    const eventIds = new Set(this.store.matingEvents
      .filter((event) => event.breedingPairId === breedingPairId).map((event) => event.id))
    return clone(this.store.litters.filter((litter) => eventIds.has(litter.matingEventId)))
  }
  async createLitter(input: CreateLitterInput): Promise<CreatedLitter> {
    await pause(20)
    if (!this.store.matingEvents.some((event) => event.id === input.matingEventId)
      || input.sizeTotal < input.drafts.length || input.sizeTotal < 0
      || input.drafts.some((draft) => !draft.temporaryLabel.trim())) {
      throw new Error('窝次总数必须覆盖全部存活 Draft，且临时标签不能为空')
    }
    const litterId = crypto.randomUUID()
    const litter: Litter = {
      id: litterId, matingEventId: input.matingEventId, bornOn: input.bornOn,
      sizeTotal: input.sizeTotal, sizeAlive: input.drafts.length,
      notes: input.notes?.trim() || undefined, revision: 1,
    }
    const drafts: AnimalDraft[] = input.drafts.map((draft) => ({
      id: crypto.randomUUID(), litterId, temporaryLabel: draft.temporaryLabel.trim(),
      sex: draft.sex, birthDate: input.bornOn, status: 'pending', revision: 1,
    }))
    this.store.litters.push(litter)
    this.store.animalDrafts.push(...drafts)
    return clone({ litter, drafts })
  }
  async listAnimalDrafts(litterId: string) {
    await pause(20)
    return clone(this.store.animalDrafts.filter((draft) => draft.litterId === litterId))
  }
  async registerAnimalDraft(input: RegisterAnimalDraftInput): Promise<RegisteredAnimalDraft> {
    await pause(20)
    const draft = this.store.animalDrafts.find((item) => item.id === input.draftId)
    if (!draft || draft.status !== 'pending') throw new Error('Animal Draft 不存在或已登记')
    if (draft.revision !== input.expectedRevision) throw new Error('Draft 已更新，请刷新后重试')
    const animal = this.store.createAnimal({
      displayId: input.displayId,
      identifierScope: input.identifierScope,
      projectId: input.projectId,
      cageId: input.initialCageId,
      sex: draft.sex,
      strain: input.strain ?? '',
      birthDate: draft.birthDate,
    })
    draft.status = 'registered'; draft.registeredAnimalId = animal.id; draft.revision += 1
    return clone({ draft, animal })
  }
  async predictBreeding(input: BreedingPredictionInput): Promise<LocusPrediction[]> {
    await pause(20)
    const male = this.store.genotypeDefinitions.find((item) => item.id === input.maleGenotypeDefinitionId)
    const female = this.store.genotypeDefinitions.find((item) => item.id === input.femaleGenotypeDefinitionId)
    if (!male || !female) throw new Error('请选择有效的父本和母本基因型定义')
    const loci = new Set([...male.components, ...female.components].map((component) => component.locusId))
    const gametes = (component: GenotypeDefinition['components'][number] | undefined) => {
      if (!component) return [{ id: undefined as string | undefined, probability: 1 }]
      const values = component.mode === 'diploid' || component.mode === 'conditional'
        ? [{ id: component.allele1Id, probability: 0.5 }, { id: component.allele2Id, probability: 0.5 }]
        : [{ id: component.allele1Id, probability: 0.5 }, { id: undefined, probability: 0.5 }]
      const merged = new Map<string | undefined, number>()
      values.forEach((item) => merged.set(item.id, (merged.get(item.id) ?? 0) + item.probability))
      return [...merged].map(([id, probability]) => ({ id, probability }))
    }
    return [...loci].sort().map((locusId) => {
      const paternal = gametes(male.components.find((item) => item.locusId === locusId))
      const maternal = gametes(female.components.find((item) => item.locusId === locusId))
      return {
        locusId,
        outcomes: paternal.flatMap((father) => maternal.map((mother) => ({
          paternalAlleleId: father.id,
          maternalAlleleId: mother.id,
          probability: father.probability * mother.probability,
        }))),
      }
    })
  }
  async createAnimalSample(input: CreateAnimalSampleInput): Promise<AnimalSample> {
    await pause(20)
    const sample: AnimalSample = {
      id: `${input.animalId}:${crypto.randomUUID()}`,
      projectId: input.projectId,
      experimentId: input.experimentId,
      sampleType: input.sampleType,
      quantity: input.quantity,
      unit: input.unit,
      location: input.location,
      collectedAt: input.collectedAt ?? new Date().toISOString(),
      revision: 1,
    }
    this.store.samples.push(sample)
    return clone(sample)
  }
  async createPedigree(input: CreatePedigreeInput): Promise<PedigreeRelation> {
    await pause(20)
    const parent = this.store.animals.find((animal) => animal.id === input.parentId)
    if (!parent || !this.store.animals.some((animal) => animal.id === input.animalId)) {
      throw new Error('动物不存在')
    }
    const relation: PedigreeRelation = {
      id: `${input.animalId}:${crypto.randomUUID()}`,
      direction: 'parent',
      parentType: input.parentType,
      relatedAnimal: {
        id: parent.id,
        code: parent.code,
        sex: parent.sex,
        strain: parent.strain,
        status: parent.status,
      },
      revision: 1,
    }
    this.store.pedigree.push(relation)
    return clone(relation)
  }
  async moveAnimals(animalIds: string[], targetCageId: string) { await pause(20); this.store.moveAnimals(animalIds, targetCageId) }
  async createProject(input: CreateProjectInput) {
    await pause(20)
    const project = { id: crypto.randomUUID(), name: input.name.trim() }
    if (!project.name) throw new Error('请输入项目名称')
    this.store.projects.push(project)
    return clone(project)
  }
  async listProjects() { await pause(20); return clone(this.store.projects) }
  async listPublishedTemplates() { await pause(20); return clone(this.store.templates) }
  async createPublishedTemplate(input: CreatePublishedTemplateInput) {
    await pause(20)
    if (!input.name.trim() || !input.fieldKey.trim() || !input.fieldLabel.trim()) {
      throw new Error('请完整填写模板和字段信息')
    }
    const template = { id: crypto.randomUUID(), name: input.name.trim(), version: 1 }
    this.store.templates.push(template)
    return clone(template)
  }
  async createExperiment(input: CreateExperimentInput) {
    await pause(20)
    const project = this.store.projects.find((item) => item.id === input.projectId)
    const template = this.store.templates.find((item) => item.id === input.templateVersionId)
    if (!project || !template) throw new Error('请选择有效项目和已发布模板')
    const experiment: Experiment = {
      id: crypto.randomUUID(),
      projectId: project.id,
      code: `EXP-${String(this.store.experiments.length + 1).padStart(3, '0')}`,
      name: input.name.trim(),
      project: project.name,
      status: 'draft',
      startDate: input.startDate ?? '',
      animalCount: 0,
      completedSteps: 0,
      totalSteps: 1,
      groups: [],
      revision: 1,
    }
    this.store.experiments.unshift(experiment)
    return clone(experiment)
  }
  async completeExperiment(id: string, expectedRevision: number) {
    await pause(20)
    return this.transitionDemoExperiment(id, expectedRevision, 'completed')
  }
  async cancelExperiment(id: string, expectedRevision: number) {
    await pause(20)
    return this.transitionDemoExperiment(id, expectedRevision, 'cancelled')
  }
  private transitionDemoExperiment(
    id: string,
    expectedRevision: number,
    status: 'completed' | 'cancelled',
  ) {
    const experiment = this.store.experiments.find((item) => item.id === id)
    if (!experiment) throw new Error('实验不存在')
    if (experiment.revision !== expectedRevision) throw new Error('实验已被其他操作更新，请刷新')
    if (experiment.status !== 'draft' && experiment.status !== 'active') {
      throw new Error('实验已经结束')
    }
    experiment.status = status
    experiment.revision += 1
    const exitedAt = new Date().toISOString()
    for (const participation of this.store.participations) {
      if (participation.experimentId === id && participation.status === 'enrolled') {
        participation.status = status === 'completed' ? 'completed' : 'withdrawn'
        participation.exitedAt = exitedAt
        participation.revision += 1
      }
    }
    return clone(experiment)
  }
  async listExperiments() { await pause(20); return clone(this.store.experiments) }
  async listCohorts(experimentId: string) {
    await pause(20)
    return clone(this.store.cohorts.filter((item) => item.experimentId === experimentId))
  }
  async createCohort(input: CreateCohortInput) {
    await pause(20)
    const cohort: Cohort = {
      id: crypto.randomUUID(), experimentId: input.experimentId,
      name: input.name.trim(), description: input.description || undefined,
    }
    this.store.cohorts.push(cohort)
    return clone(cohort)
  }
  async listParticipations(projectId: string, experimentId: string) {
    await pause(20)
    if (!this.store.experiments.some((item) => item.id === experimentId && item.projectId === projectId)) return []
    return clone(this.store.participations.filter((item) => item.experimentId === experimentId))
  }
  async enrollAnimal(input: EnrollAnimalInput) {
    await pause(20)
    const latestByDefinition = new Map<string, GenotypingRecord>()
    this.store.genotypingRecords
      .filter((record) => record.animalId === input.animalId)
      .sort((left, right) => (left.assessedAt ?? left.createdAt).localeCompare(
        right.assessedAt ?? right.createdAt,
      ))
      .forEach((record) => latestByDefinition.set(record.genotypeDefinitionId, record))
    const participation: Participation = {
      id: crypto.randomUUID(), experimentId: input.experimentId, animalId: input.animalId,
      cohortId: input.cohortId, status: 'enrolled', enrolledAt: new Date().toISOString(),
      genotypeSnapshot: [...latestByDefinition.values()].map((record) => ({
        genotypingRecordId: record.id,
        genotypeDefinitionId: record.genotypeDefinitionId,
        state: record.state,
        assessedAt: record.assessedAt,
      })),
      revision: 1,
    }
    this.store.participations.push(participation)
    return clone(participation)
  }
  async completeParticipation(id: string, expectedRevision: number) {
    await pause(20)
    return this.transitionDemoParticipation(id, expectedRevision, 'completed')
  }
  async withdrawParticipation(id: string, expectedRevision: number) {
    await pause(20)
    return this.transitionDemoParticipation(id, expectedRevision, 'withdrawn')
  }
  private transitionDemoParticipation(
    id: string,
    expectedRevision: number,
    status: 'completed' | 'withdrawn',
  ) {
    const participation = this.store.participations.find((item) => item.id === id)
    if (!participation) throw new Error('实验参与记录不存在')
    if (participation.revision !== expectedRevision) throw new Error('参与记录已更新，请刷新')
    if (participation.status !== 'enrolled') throw new Error('实验参与已经结束')
    participation.status = status
    participation.exitedAt = new Date().toISOString()
    participation.revision += 1
    return clone(participation)
  }
  async listProcedures(experimentId: string) {
    await pause(20)
    return clone(this.store.procedures.filter((item) => item.experimentId === experimentId))
  }
  async createProcedure(input: CreateProcedureInput) {
    await pause(20)
    const procedure: Procedure = {
      id: crypto.randomUUID(), experimentId: input.experimentId, animalId: input.animalId,
      name: input.name.trim(), scheduledAt: input.scheduledAt, performedAt: input.performedAt,
      status: input.status, details: input.details ?? {},
    }
    this.store.procedures.push(procedure)
    return clone(procedure)
  }
  async listExperimentEvents(experimentId: string) {
    await pause(20)
    return clone(this.store.experimentEvents.filter((event) => event.experimentId === experimentId))
  }
  async createExperimentEvent(input: CreateExperimentEventInput) {
    await pause(20)
    const experiment = this.store.experiments.find((item) => item.id === input.experimentId)
    if (!experiment || !input.eventKey.trim() || !input.label.trim()) {
      throw new Error('请选择实验并填写事件键和标签')
    }
    const event: ExperimentEvent = {
      id: crypto.randomUUID(), projectId: experiment.projectId, experimentId: experiment.id,
      eventKey: input.eventKey.trim(), label: input.label.trim(),
      occurredAt: input.occurredAt ?? new Date().toISOString(), details: input.details ?? {},
      revision: 1,
    }
    this.store.experimentEvents.push(event)
    return clone(event)
  }
  async listObservationDefinitions(experimentId: string) {
    await pause(20)
    return clone(this.store.observationDefinitions.filter(
      (definition) => definition.experimentId === experimentId,
    ))
  }
  async createObservationDefinition(input: CreateObservationDefinitionInput) {
    await pause(20)
    const experiment = this.store.experiments.find((item) => item.id === input.experimentId)
    const categories = input.categories?.map((item) => item.trim()).filter(Boolean) ?? []
    if (!experiment || !input.key.trim() || !input.label.trim()
      || (input.valueType === 'number' && !input.unit?.trim())
      || (input.valueType !== 'number' && input.unit)
      || (input.valueType === 'category' && !categories.length)
      || (input.valueType !== 'category' && categories.length)) {
      throw new Error('观察定义的数据类型、单位或类别配置无效')
    }
    const definition: ObservationDefinition = {
      id: crypto.randomUUID(), projectId: experiment.projectId, experimentId: experiment.id,
      key: input.key.trim(), label: input.label.trim(), valueType: input.valueType,
      unit: input.unit?.trim() || undefined, categories, policy: input.policy, revision: 1,
    }
    this.store.observationDefinitions.push(definition)
    return clone(definition)
  }
  async listObservations(filter: ObservationFilter) {
    await pause(20)
    return clone(this.store.observations.filter((observation) =>
      observation.experimentId === filter.experimentId
      && (!filter.experimentEventId || observation.experimentEventId === filter.experimentEventId)
      && (!filter.subjectType || observation.subjectType === filter.subjectType)
      && (!filter.subjectId || observation.subjectId === filter.subjectId)))
  }
  async recordObservation(input: RecordObservationInput): Promise<RecordedObservation> {
    await pause(20)
    const experiment = this.store.experiments.find((item) => item.id === input.experimentId)
    const event = this.store.experimentEvents.find((item) => item.id === input.experimentEventId)
    const definition = this.store.observationDefinitions.find((item) => item.id === input.definitionId)
    if (!experiment || !event || event.experimentId !== experiment.id
      || !definition || definition.experimentId !== experiment.id) {
      throw new Error('实验事件或观察定义不属于当前实验')
    }
    const subjectValid = input.subjectType === 'experiment'
      ? input.subjectId === experiment.id
      : input.subjectType === 'animal'
        ? this.store.animals.some((animal) => animal.id === input.subjectId)
          && this.store.participations.some((participation) =>
            participation.experimentId === experiment.id && participation.animalId === input.subjectId)
        : input.subjectType === 'sample'
          ? this.store.samples.some((sample) =>
            sample.id === input.subjectId && sample.experimentId === experiment.id)
          : false
    if (!subjectValid) throw new Error('观察对象不存在或不属于当前实验范围')
    assertDemoObservationValue(definition, input.value)
    const now = new Date().toISOString()
    const observation: Observation = {
      id: crypto.randomUUID(), projectId: experiment.projectId, experimentId: experiment.id,
      experimentEventId: event.id, definitionId: definition.id,
      subjectType: input.subjectType, subjectId: input.subjectId,
      context: input.context ?? {}, currentValueVersion: 1, revision: 1,
    }
    const value: ObservationValueRecord = {
      id: crypto.randomUUID(), observationId: observation.id, version: 1,
      value: clone(input.value), recordedAt: input.recordedAt ?? now,
      notes: input.notes?.trim() || undefined, revision: 1,
    }
    this.store.observations.push(observation)
    this.store.observationValues.push(value)
    return clone({ observation, value })
  }
  async listObservationValues(observationId: string) {
    await pause(20)
    return clone(this.store.observationValues
      .filter((value) => value.observationId === observationId)
      .sort((left, right) => left.version - right.version))
  }
  async reviseObservation(input: ReviseObservationInput): Promise<RecordedObservation> {
    await pause(20)
    const observation = this.store.observations.find((item) => item.id === input.observationId)
    const definition = observation
      ? this.store.observationDefinitions.find((item) => item.id === observation.definitionId)
      : undefined
    if (!observation || !definition) throw new Error('观察记录不存在')
    if (definition.policy === 'immutable') throw new Error('不可变观察记录不能修订')
    if (observation.revision !== input.expectedRevision) throw new Error('观察记录已更新，请刷新')
    assertDemoObservationValue(definition, input.value)
    const value: ObservationValueRecord = {
      id: crypto.randomUUID(), observationId: observation.id,
      version: observation.currentValueVersion + 1, value: clone(input.value),
      recordedAt: input.recordedAt ?? new Date().toISOString(),
      notes: input.notes?.trim() || undefined, revision: 1,
    }
    observation.currentValueVersion = value.version
    observation.revision += 1
    this.store.observationValues.push(value)
    return clone({ observation, value })
  }
  async listDataJobs() { await pause(20); return clone(seedDataJobs) }
  async getWorkspaceSettings() { await pause(20); return clone(this.store.workspaceSettings) }
  async saveWorkspaceSettings(input: WorkspaceSettings) {
    await pause(20)
    this.store.workspaceSettings = clone(input)
    return clone(this.store.workspaceSettings)
  }
  async getAiSettings() { await pause(20); return clone(this.store.aiSettings) }
  async saveAiSettings(input: SaveAiSettingsInput) {
    await pause(20)
    const sameCredentialBinding = this.store.aiSettings.providerKind === input.providerKind
      && this.store.aiSettings.providerPresetId === input.providerPresetId
      && this.store.aiSettings.baseUrl.replace(/\/$/, '') === input.baseUrl.replace(/\/$/, '')
    this.store.aiSettings = {
      enabled: input.enabled,
      providerKind: input.providerKind,
      providerPresetId: input.providerPresetId,
      model: input.model,
      baseUrl: input.baseUrl,
      supportsVision: input.supportsVision ?? false,
      visionModel: input.visionModel,
      contextWindowTokens: input.contextWindowTokens,
      maxInputTokens: input.maxInputTokens,
      maxOutputTokens: input.maxOutputTokens,
      historyTokenBudget: input.historyTokenBudget,
      historyTurns: input.historyTurns,
      temperature: input.temperature,
      timeoutMs: input.timeoutMs,
      revision: this.store.aiSettings.revision + 1,
      hasKey: input.apiKey === undefined
        ? (sameCredentialBinding && this.store.aiSettings.hasKey)
        : input.apiKey.length > 0,
    }
    return clone(this.store.aiSettings)
  }
  async clearAiApiKey() {
    await pause(20)
    this.store.aiSettings.hasKey = false
    return clone(this.store.aiSettings)
  }
  async listAiModelProfiles(includeArchived = false) {
    await pause(20)
    return clone(this.store.aiModelProfiles.filter((profile) => includeArchived || !profile.archivedAt))
  }
  async getAiModelProfile(id: string) {
    await pause(20)
    const profile = this.store.aiModelProfiles.find((item) => item.id === id)
    if (!profile) throw new Error('模型配置不存在')
    return clone(profile)
  }
  async createAiModelProfile(input: SaveAiModelProfileInput) {
    await pause(20)
    if (input.transport === 'open_ai_compatible' && !input.apiKey?.trim()) {
      throw new Error('云端模型首次保存必须填写 API Key')
    }
    const { apiKey, expectedRevision: _expectedRevision, ...configuration } = input
    const profile: AiModelProfileView = {
      id: crypto.randomUUID(),
      ...clone(configuration),
      currentVersion: 1,
      revision: 1,
      hasKey: Boolean(apiKey?.trim()),
      isDefaultConversation: false,
      isDefaultVision: false,
    }
    this.store.aiModelProfiles.push(profile)
    return clone(profile)
  }
  async updateAiModelProfile(id: string, input: SaveAiModelProfileInput) {
    await pause(20)
    const index = this.store.aiModelProfiles.findIndex((item) => item.id === id)
    const current = this.store.aiModelProfiles[index]
    if (!current || current.archivedAt) throw new Error('模型配置不存在或已停用')
    if (input.expectedRevision !== current.revision) throw new Error('模型配置已更新，请刷新后重试')
    if (current.isDefaultVision && !input.supportsVision) {
      throw new Error('请先取消默认视觉模型，再关闭视觉能力')
    }
    const identityChanged = current.protocol !== input.protocol
      || current.transport !== input.transport
      || current.baseUrl.replace(/\/$/, '') !== input.baseUrl.replace(/\/$/, '')
    if (identityChanged && !input.apiKey?.trim()) {
      throw new Error('协议、连接方式或 Base URL 变化后必须重新输入 API Key')
    }
    const { apiKey, expectedRevision: _expectedRevision, ...configuration } = input
    const configurationChanged = current.name !== configuration.name
      || current.protocol !== configuration.protocol
      || current.transport !== configuration.transport
      || current.baseUrl !== configuration.baseUrl
      || current.modelId !== configuration.modelId
      || current.supportsVision !== configuration.supportsVision
      || current.contextWindowTokens !== configuration.contextWindowTokens
      || current.maxInputTokens !== configuration.maxInputTokens
      || current.maxOutputTokens !== configuration.maxOutputTokens
      || current.historyTokenBudget !== configuration.historyTokenBudget
      || current.historyTurns !== configuration.historyTurns
      || current.temperature !== configuration.temperature
      || current.timeoutMs !== configuration.timeoutMs
    if (!configurationChanged) {
      if (apiKey?.trim()) current.hasKey = true
      return clone(current)
    }
    const updated: AiModelProfileView = {
      ...current,
      ...clone(configuration),
      currentVersion: current.currentVersion + 1,
      revision: current.revision + 1,
      hasKey: apiKey?.trim() ? true : current.hasKey,
    }
    this.store.aiModelProfiles[index] = updated
    return clone(updated)
  }
  async validateAiModelProfile(input: ValidateAiModelProfileInput) {
    await pause(20)
    if ((input.profileId === undefined) !== (input.currentVersion === undefined)) {
      throw new Error('profileId 与 currentVersion 必须同时提供')
    }
    const stored = input.profileId
      ? this.store.aiModelProfiles.find((item) =>
        item.id === input.profileId && item.currentVersion === input.currentVersion)
      : undefined
    const canReuseKey = stored?.protocol === input.protocol
      && stored.transport === input.transport
      && stored.baseUrl.replace(/\/$/, '') === input.baseUrl.replace(/\/$/, '')
      && stored.hasKey
    if (input.transport === 'open_ai_compatible' && !input.apiKey?.trim() && !canReuseKey) {
      return { ok: false, latencyMs: 20, errorCode: 'missing_credential' }
    }
    return { ok: true, latencyMs: 20 }
  }
  async clearAiModelProfileKey(id: string) {
    await pause(20)
    const profile = this.store.aiModelProfiles.find((item) => item.id === id)
    if (!profile || profile.archivedAt) throw new Error('模型配置不存在或已停用')
    profile.hasKey = false
    return clone(profile)
  }
  async archiveAiModelProfile(id: string, expectedRevision: number) {
    await pause(20)
    const profile = this.store.aiModelProfiles.find((item) => item.id === id)
    if (!profile || profile.archivedAt || profile.revision !== expectedRevision) {
      throw new Error('模型配置已变化，请刷新后重试')
    }
    profile.archivedAt = new Date().toISOString()
    profile.revision += 1
    let defaultsChanged = false
    if (this.store.aiModelDefaults.defaultConversationProfileId === id) {
      this.store.aiModelDefaults.defaultConversationProfileId = undefined
      defaultsChanged = true
    }
    if (this.store.aiModelDefaults.defaultVisionProfileId === id) {
      this.store.aiModelDefaults.defaultVisionProfileId = undefined
      defaultsChanged = true
    }
    if (defaultsChanged) {
      this.store.aiModelDefaults.revision += 1
    }
    profile.isDefaultConversation = false
    profile.isDefaultVision = false
    for (const detail of this.aiConversations.values()) {
      if (detail.conversation.modelProfileId === id) {
        detail.conversation.readOnly = true
        detail.conversation.readOnlyReason = 'model_archived'
        detail.conversation.revision += 1
      }
    }
    return clone(profile)
  }
  async getAiModelDefaults() {
    await pause(20)
    return clone(this.store.aiModelDefaults)
  }
  async saveAiModelDefaults(input: SaveAiModelDefaultsInput) {
    await pause(20)
    if (input.expectedRevision !== this.store.aiModelDefaults.revision) {
      throw new Error('默认模型已更新，请刷新后重试')
    }
    const conversation = input.defaultConversationProfileId
      ? this.store.aiModelProfiles.find((profile) =>
        profile.id === input.defaultConversationProfileId && !profile.archivedAt)
      : undefined
    const vision = input.defaultVisionProfileId
      ? this.store.aiModelProfiles.find((profile) =>
        profile.id === input.defaultVisionProfileId && !profile.archivedAt && profile.supportsVision)
      : undefined
    if (input.defaultConversationProfileId && !conversation) throw new Error('默认对话模型不可用')
    if (input.defaultVisionProfileId && !vision) throw new Error('默认视觉模型不可用或不支持视觉')
    this.store.aiModelDefaults = {
      defaultConversationProfileId: conversation?.id,
      defaultVisionProfileId: vision?.id,
      revision: this.store.aiModelDefaults.revision + 1,
    }
    for (const profile of this.store.aiModelProfiles) {
      profile.isDefaultConversation = profile.id === conversation?.id
      profile.isDefaultVision = profile.id === vision?.id
    }
    return clone(this.store.aiModelDefaults)
  }
  async listAiProviderPresets() {
    await pause(20)
    return clone(builtinAiProviderPresets)
  }
  async listAiProviderEndpoints() {
    await pause(20)
    return clone(this.store.aiProviderEndpoints)
  }
  async saveAiProviderEndpoint(input: SaveAiProviderEndpointInput, id?: string) {
    await pause(20)
    if (id === '00000000-0000-0000-0000-000000000001') throw new Error('内置出口不能修改')
    const existing = id ? this.store.aiProviderEndpoints.find((item) => item.id === id) : undefined
    const endpoint: AiProviderEndpoint = existing ?? {
      id: crypto.randomUUID(),
      providerKind: input.providerKind,
      protocol: input.protocol,
      label: input.label,
      baseUrl: input.baseUrl,
      enabled: input.enabled,
      builtin: false,
      revision: 0,
    }
    endpoint.providerKind = input.providerKind
    endpoint.protocol = input.protocol
    endpoint.label = input.label
    endpoint.baseUrl = input.baseUrl
    endpoint.enabled = input.enabled
    endpoint.revision += 1
    if (!existing) this.store.aiProviderEndpoints.push(endpoint)
    return clone(endpoint)
  }
  async disableAiProviderEndpoint(id: string) {
    await pause(20)
    const endpoint = this.store.aiProviderEndpoints.find((item) => item.id === id && !item.builtin)
    if (!endpoint) throw new Error('Provider 出口不存在或不可停用')
    endpoint.enabled = false
    endpoint.revision += 1
    return clone(endpoint)
  }
  async listPrivateImages(conversationId?: string, projectId?: string) {
    await pause(20)
    return clone(this.privateImages.filter((entry) =>
      (!conversationId || entry.image.conversation_id === conversationId)
      && (!projectId || entry.image.project_id === projectId)))
  }
  async uploadPrivateImage(file: File, conversationId?: string) {
    await pause(20)
    const conversation = conversationId ? this.aiConversations.get(conversationId) : undefined
    if (conversationId && !conversation) throw new Error('AI 会话不存在')
    const id = crypto.randomUUID()
    const bytes = new Uint8Array(await file.arrayBuffer())
    const pseudoHash = [...bytes.slice(0, 32)]
      .map((value) => value.toString(16).padStart(2, '0'))
      .join('')
      .padEnd(64, '0')
    const now = new Date()
    const record: PrivateImageRecord = {
      image: {
        id,
        conversation_id: conversationId,
        project_id: conversation?.conversation.projectId,
        status: 'active',
        expires_at: new Date(now.getTime() + 30 * 86_400_000).toISOString(),
        meta: { revision: 1 },
      },
      fileName: file.name,
      mediaType: file.type || undefined,
      sizeBytes: file.size,
      sha256: pseudoHash,
      contentHref: '',
      previewHref: '',
      retentionDays: 30,
    }
    this.privateImages.push(record)
    this.privateImageFiles.set(id, file.slice(0, file.size, file.type))
    return clone(record)
  }
  async readPrivateImage(id: string) {
    await pause(10)
    const file = this.privateImageFiles.get(id)
    if (!file) throw new Error('私人图片不存在')
    return file.slice(0, file.size, file.type)
  }
  async archivePrivateImage(id: string, projectId: string, expectedRevision: number) {
    await pause(20)
    const entry = this.privateImages.find((candidate) => candidate.image.id === id)
    if (!entry
      || entry.image.project_id !== projectId
      || entry.image.meta.revision !== expectedRevision) {
      throw new Error('私人图片已变化，请刷新后重试')
    }
    entry.image.status = 'archived'
    entry.image.meta.revision += 1
    return clone(entry)
  }
  async listAiExtractions(projectId?: string) {
    await pause(20)
    return clone(this.aiExtractions.filter((entry) => !projectId || entry.projectId === projectId))
  }
  async createAiExtraction(input: CreateAiExtractionInput): Promise<AiExtractionRecord> {
    await pause(40)
    if (input.imageIds.length < 1 || input.imageIds.length > 8
      || new Set(input.imageIds).size !== input.imageIds.length) {
      throw new Error('请选择 1–8 张不重复的图片')
    }
    const images = input.imageIds.map((id) =>
      this.privateImages.find((entry) => entry.image.id === id
        && entry.image.status === 'active'))
    if (images.some((entry) => !entry)) throw new Error('图片证据不存在或当前不可用')
    const definition = this.store.observationDefinitions.find((entry) =>
      entry.id === input.currentDataCell.definitionId
      && entry.experimentId === input.experimentId)
    if (!definition) throw new Error('当前数据单元的数据列不存在')
    if (this.store.observations.some((entry) =>
      entry.experimentId === input.experimentId
      && entry.experimentEventId === input.experimentEventId
      && entry.definitionId === input.currentDataCell.definitionId
      && entry.subjectType === input.currentDataCell.subjectType
      && entry.subjectId === input.currentDataCell.subjectId)) {
      throw new Error('当前数据单元已有正式值，AI 候选不能覆盖')
    }
    const profileId = input.visionModelProfileId
      ?? this.store.aiModelDefaults.defaultVisionProfileId
    const profile = this.store.aiModelProfiles.find((entry) =>
      entry.id === profileId && entry.supportsVision && !entry.archivedAt)
    if (!profile) throw new Error('请明确选择一个可用的视觉模型')
    const value: ObservationValueData = definition.valueType === 'number'
      ? { type: 'number', value: 0 }
      : definition.valueType === 'text'
        ? { type: 'text', value: '演示识别候选' }
        : definition.valueType === 'boolean'
          ? { type: 'boolean', value: false }
          : definition.valueType === 'date'
            ? { type: 'date', value: new Date().toISOString().slice(0, 10) }
            : definition.valueType === 'category'
              ? { type: 'category', value: definition.categories[0] ?? '' }
              : { type: 'json', value: {} }
    const draft: AiExtractionRecord = {
      id: crypto.randomUUID(),
      projectId: input.projectId,
      experimentId: input.experimentId,
      experimentEventId: input.experimentEventId,
      currentDataCell: clone(input.currentDataCell),
      status: 'pending_approval',
      candidates: [{
        itemIndex: 0,
        confidence: 0.82,
        selected: false,
        sourceLabel: definition.label,
        value,
        notes: '浏览器演示候选；尚未写入正式数据',
      }],
      evidence: images.map((entry, displayOrder) => ({
        displayOrder,
        privateImageId: entry!.image.id,
        privateAttachmentId: entry!.image.id,
        originalSha256: entry!.sha256,
        sanitizedSha256: entry!.sha256,
      })),
      modelTrace: {
        profileId: profile.id,
        profileVersion: profile.currentVersion,
        purpose: 'vision',
        inputTokens: 0,
        outputTokens: 0,
        totalTokens: 0,
      },
      revision: 1,
    }
    for (const image of images) {
      image!.image.status = 'pending_approval'
      image!.image.meta.revision += 1
    }
    this.aiExtractions.unshift(draft)
    return clone(draft)
  }
  async approveAiExtraction(
    id: string,
    input: ApproveAiExtractionInput,
  ): Promise<AppliedAiExtraction> {
    await pause(30)
    const draft = this.aiExtractions.find((entry) => entry.id === id)
    if (!draft || draft.revision !== input.expectedRevision
      || draft.status !== 'pending_approval') {
      throw new Error('AI 候选已变化，请刷新后重试')
    }
    if (!draft.currentDataCell || input.selections.length !== 1) {
      throw new Error('当前数据单元必须且只能批准一个候选')
    }
    const selection = input.selections[0]
    const candidate = draft.candidates.find((entry) =>
      entry.itemIndex === selection?.itemIndex)
    const definition = this.store.observationDefinitions.find((entry) =>
      entry.id === draft.currentDataCell?.definitionId)
    if (!candidate || !definition) throw new Error('AI 候选绑定无效')
    assertDemoObservationValue(definition, selection.value)
    const evidenceImages = draft.evidence.map((evidence) => {
      const image = this.privateImages.find((entry) =>
        entry.image.id === evidence.privateImageId)
      if (!image || image.image.status !== 'pending_approval') {
        throw new Error('AI 图片证据已变化，请刷新后重试')
      }
      return image
    })
    const now = new Date().toISOString()
    const observation: Observation = {
      id: crypto.randomUUID(),
      projectId: draft.projectId,
      experimentId: draft.experimentId,
      experimentEventId: draft.experimentEventId,
      definitionId: draft.currentDataCell.definitionId,
      subjectType: draft.currentDataCell.subjectType,
      subjectId: draft.currentDataCell.subjectId,
      context: { source: 'ai_image_extraction' },
      currentValueVersion: 1,
      revision: 1,
    }
    const valueRecord: ObservationValueRecord = {
      id: crypto.randomUUID(),
      observationId: observation.id,
      version: 1,
      value: clone(selection.value),
      recordedAt: now,
      notes: selection.notes?.trim() || undefined,
      revision: 1,
    }
    this.store.observations.push(observation)
    this.store.observationValues.push(valueRecord)
    candidate.selected = true
    candidate.value = clone(selection.value)
    candidate.notes = selection.notes?.trim() || undefined
    const attachments = draft.evidence.map((evidence, index) => {
      const image = evidenceImages[index]!
      const promotedAttachmentId = crypto.randomUUID()
      evidence.promotedAttachmentId = promotedAttachmentId
      image.image.status = 'archived'
      image.image.project_id = draft.projectId
      image.image.meta.revision += 1
      return {
        id: promotedAttachmentId,
        fileName: image.fileName,
        sha256: image.sha256,
        mediaType: image.mediaType,
      }
    })
    const links = attachments.map((attachment) => ({
      id: crypto.randomUUID(),
      attachmentId: attachment.id,
      targetType: draft.currentDataCell!.subjectType,
      targetId: draft.currentDataCell!.subjectId,
      projectId: draft.projectId,
    }))
    draft.status = 'approved'
    draft.revision += 1
    return {
      draft: clone(draft),
      observations: [clone(observation)],
      attachments: clone(attachments),
      links: clone(links),
    }
  }
  async rejectAiExtraction(
    id: string,
    input: RejectAiExtractionInput,
  ): Promise<AiExtractionRecord> {
    await pause(20)
    const draft = this.aiExtractions.find((entry) => entry.id === id)
    if (!draft || draft.revision !== input.expectedRevision
      || draft.status !== 'pending_approval') {
      throw new Error('AI 候选已变化，请刷新后重试')
    }
    for (const evidence of draft.evidence) {
      const image = this.privateImages.find((entry) =>
        entry.image.id === evidence.privateImageId)
      if (!image || image.image.status !== 'pending_approval') {
        throw new Error('AI 图片证据已变化，请刷新后重试')
      }
    }
    for (const evidence of draft.evidence) {
      const image = this.privateImages.find((entry) =>
        entry.image.id === evidence.privateImageId)!
      image.image.status = 'active'
      image.image.meta.revision += 1
    }
    draft.status = 'rejected'
    draft.revision += 1
    return clone(draft)
  }
  async startAiConversation(
    input: StartAiConversationInput,
  ): Promise<StartAiConversationResponse> {
    await pause(20)
    const profileId = input.modelProfileId
      ?? this.store.aiModelDefaults.defaultConversationProfileId
    const profile = this.store.aiModelProfiles.find((item) =>
      item.id === profileId && !item.archivedAt)
    if (!profile) throw new Error('请选择一个可用的对话模型')
    const now = new Date().toISOString()
    const conversation: AiConversationSummary = {
      id: crypto.randomUUID(),
      projectId: input.projectId,
      title: input.title,
      modelProfileId: profile.id,
      modelProfileVersion: profile.currentVersion,
      modelProfileName: profile.name,
      modelId: profile.modelId,
      readOnly: false,
      createdAt: now,
      updatedAt: now,
      revision: 0,
    }
    const autonomy: AiAutonomyView = {
      ...defaultAiAutonomy(),
      mode: input.requestedMode,
      effectiveMode: input.requestedMode,
      revision: 1,
    }
    this.aiConversations.set(conversation.id, { conversation, messages: [] })
    this.aiConversationAutonomy.set(conversation.id, autonomy)
    this.aiConversationModels.set(conversation.id, {
      profileId: profile.id,
      profileVersion: profile.currentVersion,
      profileName: profile.name,
      modelId: profile.modelId,
      supportsVision: profile.supportsVision,
    })
    return clone({ conversation, autonomy })
  }
  async aiTurn(input: AiTurnInput): Promise<AiTurnResponse> {
    await pause(80)
    const governed = this.requireWritableAiConversation(input.conversationId, input.projectId)
    const { detail, model: conversationModel, autonomy } = governed
    const conversationId = detail.conversation.id
    const selectedSources = (input.sourceRefs ?? []).map((sourceId) => {
      const source = this.aiSources.get(sourceId)
      if (!source
        || source.conversationId !== conversationId
        || source.projectId !== detail.conversation.projectId
        || source.status !== 'ready'
        || Date.parse(source.expiresAt) <= Date.now()) {
        throw new Error('所选 AI 文件已失效，请重新上传')
      }
      return source
    })
    const imageIds = input.imageIds ?? []
    if (imageIds.length > 8 || new Set(imageIds).size !== imageIds.length) {
      throw new Error('每轮最多使用 8 张不重复的图片')
    }
    const imageEvidence = imageIds.map((id, displayOrder) => {
      const image = this.privateImages.find((entry) =>
        entry.image.id === id
        && entry.image.conversation_id === conversationId
        && entry.image.status === 'active')
      if (!image) throw new Error('图片证据不存在或不属于当前会话')
      return { imageId: id, sha256: image.sha256, displayOrder }
    })
    const sourceImageCount = selectedSources.filter(isDemoAiSourceImage).length
    const visualInputCount = imageIds.length + sourceImageCount
    if (visualInputCount > 8) {
      throw new Error('每轮最多使用 8 个图片输入')
    }
    if (input.visionModelProfileId
      && (!visualInputCount || conversationModel.supportsVision)) {
      throw new Error('视觉模型只能用于需要中转的图片证据')
    }
    let visionProfile: AiModelProfileView | undefined
    if (visualInputCount && !conversationModel.supportsVision) {
      const profileId = input.visionModelProfileId
        ?? this.store.aiModelDefaults.defaultVisionProfileId
      visionProfile = this.store.aiModelProfiles.find((profile) =>
        profile.id === profileId && profile.supportsVision && !profile.archivedAt)
      if (!visionProfile) throw new Error('请明确选择一个可用的视觉模型')
    }
    const now = new Date().toISOString()
    const citation = {
      entityType: 'animal' as const,
      entityId: 'animal-006',
      label: '动物 M-26006',
      route: '/animals?animal=animal-006',
    }
    const response: AiTurnResponse = {
      conversationId,
      content: input.sourceRefs?.length || imageIds.length
        ? [
            input.sourceRefs?.length ? `已收到 ${input.sourceRefs.length} 个演示文件` : undefined,
            visualInputCount
              ? `已按${visionProfile ? '视觉中转' : '当前模型直接视觉'}路径读取 ${visualInputCount} 张演示图片`
              : undefined,
            '浏览器演示不会调用真实 Provider、解析正式附件或写入正式数据库。',
          ].filter(Boolean).join('；')
        : '演示数据中有 1 只动物的基因型仍待确认。浏览器演示不会读取正式数据库，也不会创建写入草稿。',
      citations: [citation],
      toolRuns: [{
        toolRunId: crypto.randomUUID(), providerCallId: 'demo-call', tool: 'animal_search',
        arguments: { status: 'active' }, outcome: 'read', citations: [citation],
      }],
      drafts: [],
      trace: {
        providerId: 'demo', model: 'deterministic-demo',
        usage: { providerCalls: 0, toolCalls: 1, inputTokens: 0, outputTokens: 0, totalTokens: 0 },
        context: {
          estimatedInputTokens: 0,
          inputTokenCountIsEstimate: true,
          contextTrimmed: false,
          trimmedHistoryTurns: 0,
          trimReasons: [],
        },
        stages: visualInputCount
          ? visionProfile
            ? [
                {
                  profileId: visionProfile.id,
                  profileVersion: visionProfile.currentVersion,
                  purpose: 'vision_observation',
                  modelId: visionProfile.modelId,
                  inputTokens: 0,
                  outputTokens: 0,
                  totalTokens: 0,
                },
                {
                  profileId: conversationModel.profileId,
                  profileVersion: conversationModel.profileVersion,
                  purpose: 'final_answer',
                  modelId: conversationModel.modelId,
                  inputTokens: 0,
                  outputTokens: 0,
                  totalTokens: 0,
                },
              ]
            : [{
                profileId: conversationModel.profileId,
                profileVersion: conversationModel.profileVersion,
                purpose: 'vision_and_final',
                modelId: conversationModel.modelId,
                inputTokens: 0,
                outputTokens: 0,
                totalTokens: 0,
              }]
          : [],
        imageEvidence,
      },
      autonomy: clone(autonomy),
    }
    const conversationDetail = detail
    if (!conversationDetail.messages.length && conversationDetail.conversation.title === '新会话') {
      conversationDetail.conversation.title = input.message.trim().slice(0, 80) || '新会话'
    }
    conversationDetail.messages.push(
      {
        id: crypto.randomUUID(),
        sequence: conversationDetail.messages.length + 1,
        role: 'user',
        content: input.message,
        createdAt: now,
      },
      {
        id: crypto.randomUUID(),
        sequence: conversationDetail.messages.length + 2,
        role: 'assistant',
        content: response.content,
        response: clone(response),
        createdAt: now,
      },
    )
    conversationDetail.conversation.updatedAt = now
    conversationDetail.conversation.revision += 1
    return clone(response)
  }
  async getAiAutonomy(conversationId: string) {
    await pause(20)
    const autonomy = this.aiConversationAutonomy.get(conversationId)
    if (!autonomy) throw new Error('AI 会话不存在')
    return clone(autonomy)
  }
  async setAiAutonomy(conversationId: string, input: AiAutonomyUpdateInput) {
    await pause(20)
    const current = this.aiConversationAutonomy.get(conversationId)
    if (!current) throw new Error('AI 会话不存在')
    if (current.revision !== input.expectedRevision) throw new Error('会话授权已变化，请刷新后重试')
    const updated: AiAutonomyView = {
      ...current,
      mode: input.mode,
      effectiveMode: input.mode,
      revision: current.revision + 1,
    }
    this.aiConversationAutonomy.set(conversationId, updated)
    return clone(updated)
  }
  async listAiConversations(projectId?: string, limit = 50) {
    await pause(20)
    return [...this.aiConversations.values()]
      .map((detail) => detail.conversation)
      .filter((conversation) => !conversation.archivedAt)
      .filter((conversation) => !projectId || conversation.projectId === projectId)
      .sort(compareAiConversations)
      .slice(0, limit)
      .map(clone)
  }
  async queryAiConversations(input: AiConversationListInput = {}) {
    await pause(20)
    const query = input.titleQuery?.trim().toLocaleLowerCase()
    return [...this.aiConversations.values()]
      .map((detail) => detail.conversation)
      .filter((conversation) => !input.projectId || conversation.projectId === input.projectId)
      .filter((conversation) => {
        if ((input.archive ?? 'active') === 'all') return true
        return (input.archive === 'archived') === Boolean(conversation.archivedAt)
      })
      .filter((conversation) => !query || conversation.title.toLocaleLowerCase().includes(query))
      .sort(compareAiConversations)
      .slice(0, input.limit ?? 100)
      .map(clone)
  }
  async getAiConversation(conversationId: string, limit = 200) {
    await pause(20)
    const detail = this.aiConversations.get(conversationId)
    if (!detail) throw new Error('AI 会话不存在')
    return clone({
      conversation: detail.conversation,
      messages: detail.messages.slice(-limit),
    })
  }
  async updateAiConversation(
    conversationId: string,
    input: AiConversationUpdateInput,
  ): Promise<AiConversationSummary> {
    await pause(20)
    const detail = this.aiConversations.get(conversationId)
    if (!detail || detail.conversation.revision !== input.expectedRevision) {
      throw new Error('AI 会话已变化，请刷新后重试')
    }
    const now = new Date().toISOString()
    if (input.action === 'rename') {
      const title = input.title?.trim()
      if (!title) throw new Error('会话标题不能为空')
      detail.conversation.title = title.slice(0, 120)
    } else if (input.action === 'pin') {
      detail.conversation.pinnedAt = now
    } else if (input.action === 'unpin') {
      detail.conversation.pinnedAt = undefined
    } else if (input.action === 'archive') {
      detail.conversation.archivedAt = now
    } else {
      detail.conversation.archivedAt = undefined
    }
    detail.conversation.updatedAt = now
    detail.conversation.revision += 1
    return clone(detail.conversation)
  }
  async uploadAiSource(input: AiSourceUploadInput): Promise<AiSource> {
    await pause(40)
    const { detail } = this.requireWritableAiConversation(
      input.conversationId,
      input.projectId,
    )
    const conversation = detail.conversation
    const now = new Date().toISOString()
    const source: AiSource = {
      id: crypto.randomUUID(),
      conversationId: input.conversationId,
      projectId: conversation.projectId,
      fileName: input.file.name,
      mediaType: input.file.type || 'application/octet-stream',
      sizeBytes: input.file.size,
      status: 'ready',
      revision: 1,
      createdAt: now,
      expiresAt: new Date(Date.now() + 30 * 24 * 60 * 60 * 1000).toISOString(),
    }
    this.aiSources.set(source.id, source)
    return clone(source)
  }
  private requireWritableAiConversation(
    conversationId: string,
    projectId?: string,
  ): {
    detail: AiConversationDetail
    model: DemoConversationModelBinding
    autonomy: AiAutonomyView
  } {
    const detail = this.aiConversations.get(conversationId)
    if (!detail) throw new Error('AI 会话不存在')
    const conversation = detail.conversation
    if (conversation.archivedAt) throw new Error('已归档的 AI 会话为只读')
    if (conversation.readOnly) throw new Error('AI 会话为只读，当前模型不可用')
    if (projectId !== undefined && projectId !== conversation.projectId) {
      throw new Error('AI 会话不存在或文件范围不匹配')
    }

    const model = this.aiConversationModels.get(conversationId)
    const liveProfile = model
      ? this.store.aiModelProfiles.find((profile) => profile.id === model.profileId)
      : undefined
    if (!model
      || conversation.modelProfileId !== model.profileId
      || conversation.modelProfileVersion !== model.profileVersion
      || conversation.modelProfileName !== model.profileName
      || conversation.modelId !== model.modelId
      || !liveProfile
      || liveProfile.currentVersion < model.profileVersion) {
      conversation.readOnly = true
      conversation.readOnlyReason = 'model_unavailable'
      conversation.revision += 1
      throw new Error('会话绑定的模型不可用')
    }
    if (liveProfile.archivedAt) {
      conversation.readOnly = true
      conversation.readOnlyReason = 'model_archived'
      conversation.revision += 1
      throw new Error('会话绑定的模型已归档')
    }

    const autonomy = this.aiConversationAutonomy.get(conversationId)
    if (!autonomy) {
      conversation.readOnly = true
      conversation.readOnlyReason = 'model_unavailable'
      conversation.revision += 1
      throw new Error('AI 会话治理状态不可用')
    }
    return { detail, model, autonomy }
  }
  async listAiSources(input: AiSourceListInput): Promise<AiSource[]> {
    await pause(20)
    return clone([...this.aiSources.values()].filter((source) =>
      source.conversationId === input.conversationId
      && source.projectId === input.projectId
      && (!input.status || source.status === input.status)))
  }
  async archiveAiSource(sourceId: string, input: AiSourceArchiveInput): Promise<AiSource> {
    await pause(20)
    const source = this.aiSources.get(sourceId)
    if (!source
      || source.projectId !== input.projectId
      || source.revision !== input.expectedRevision
      || (source.status !== 'staged' && source.status !== 'ready')) {
      throw new Error('AI 来源已变化或不能归档')
    }
    source.status = 'archived'
    source.revision += 1
    return clone(source)
  }
  async deleteAiSource(sourceId: string): Promise<void> {
    await pause(20)
    const source = this.aiSources.get(sourceId)
    if (source?.status === 'archived' || source?.status === 'expired') {
      throw new Error('已归档或过期的 AI 来源不能作为暂存文件删除')
    }
    this.aiSources.delete(sourceId)
  }
  async listAiDrafts() { return [] }
  async getAiDraft(_draftId: string): Promise<AiWriteDraft> { throw new Error('演示模式没有正式 AI 草稿') }
  async decideAiDraft(_draftId: string, _input: AiDraftDecisionInput): Promise<AiDraftDecisionResponse> {
    throw new Error('演示模式不会执行 AI 写入')
  }
}
