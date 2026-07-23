import { invoke } from '@tauri-apps/api/core'
import type {
  AiAutonomyMode,
  AiAutonomyUpdateInput,
  AiAutonomyView,
  AiConversationDetail,
  AiConversationCreateInput,
  AiConversationListInput,
  AiConversationSummary,
  AiConversationUpdateInput,
  AiDraftDecisionInput,
  AiDraftDecisionResponse,
  AiDraftStatus,
  AiEntityType,
  AiProviderKind,
  AiProviderPreset,
  AiSettings,
  AiSource,
  AiSourceArchiveInput,
  AiSourceListInput,
  AiSourceUploadInput,
  AiTurnInput,
  AiTurnResponse,
  AiWriteDraft,
  Animal,
  AnimalDraft,
  AnimalDetail,
  AnimalGenotype,
  AnimalSample,
  AuthSession,
  AuthUser,
  BreedingLine,
  BreedingPair,
  Cage,
  Cohort,
  Colony,
  CreatedLitter,
  DataJob,
  Experiment,
  ExperimentEvent,
  ExperimentTemplateSummary,
  GatewayMode,
  GeneAllele,
  GeneLocus,
  GenotypeComponentMode,
  GenotypeDefinition,
  GenotypingRecord,
  GenotypingState,
  Litter,
  LocusPrediction,
  LoginInput,
  LabRole,
  ManagedUser,
  ManagedUserStatus,
  MatingEvent,
  Observation,
  ObservationDefinition,
  ObservationPolicy,
  ObservationSubjectType,
  ObservationValueData,
  ObservationValueRecord,
  ObservationValueType,
  Participation,
  PedigreeRelation,
  Procedure,
  ProjectRole,
  ProjectAnimalAssignment,
  ProjectSummary,
  RecordedObservation,
  RegisteredAnimalDraft,
  SaveAiSettingsInput,
  TemplateFieldValueType,
  TimelineEvent,
  WorkspaceSettings,
} from '@/domain/models'
import { seedAnimals, seedCages, seedDataJobs, seedExperiments } from './mock-data'
import { builtinAiProviderPresets } from './aiProviderPresets'
import {
  activeProjectId,
  clearProjectContext,
  currentAuthSession,
  hasLabRegistryAccess,
} from './projectContext'

export { currentAuthSession } from './projectContext'

export interface CreateCageInput {
  code: string
  room: string
  rack: string
  capacity: number
}

export interface CreateAnimalInput {
  displayId: string
  identifierScope: 'lab' | 'project'
  projectId?: string
  cageId?: string
  sex: 'male' | 'female' | 'unknown'
  strain: string
  birthDate?: string
  initialGenotypingRecords?: Array<{
    genotypeDefinitionId: string
    state: GenotypingState
    assessedAt?: string
    method?: string
    notes?: string
  }>
}

export interface CreateProjectInput {
  name: string
  description: string
}

export interface CreatePublishedTemplateInput {
  name: string
  description: string
  fieldKey: string
  fieldLabel: string
  fieldValueType: TemplateFieldValueType
  fieldUnit: string
}

export interface CreateExperimentInput {
  projectId: string
  templateVersionId: string
  name: string
  description: string
  startDate?: string
}

export interface CreateCohortInput {
  experimentId: string
  name: string
  description: string
}

export interface EnrollAnimalInput {
  experimentId: string
  animalId: string
  cohortId?: string
}

export interface CreateProcedureInput {
  experimentId: string
  animalId?: string
  name: string
  scheduledAt?: string
  performedAt?: string
  status: Procedure['status']
  details?: Record<string, unknown>
}

export interface AnimalAccessContext {
  projectId?: string
}

export interface CreateAnimalSampleInput {
  animalId: string
  projectId: string
  experimentId?: string
  sampleType: string
  quantity?: number
  unit?: string
  location?: string
  collectedAt?: string
}

export interface CreatePedigreeInput {
  projectId?: string
  animalId: string
  parentId: string
  parentType: 'father' | 'mother' | 'unknown'
}

export interface CreateGeneLocusInput {
  projectId?: string
  symbol: string
  description?: string
}

export interface CreateAlleleInput {
  projectId?: string
  locusId: string
  symbol: string
  description?: string
  isWildType: boolean
}

export interface CreateGenotypeInput {
  projectId?: string
  animalId: string
  locusId: string
  allele1Id?: string
  allele2Id?: string
  assessedAt?: string
}

export interface CreateGenotypeDefinitionInput {
  projectId?: string
  name: string
  description?: string
  components: Array<{
    locusId: string
    allele1Id: string
    allele2Id?: string
    mode: GenotypeComponentMode
    displayOrder: number
  }>
}

export interface CreateGenotypingRecordInput {
  projectId?: string
  animalId: string
  genotypeDefinitionId: string
  state: GenotypingState
  assessedAt?: string
  method?: string
  notes?: string
}

export interface GeneticsArchiveInput {
  id: string
  expectedRevision: number
  projectId?: string
}

export interface GeneticsReferenceCounts {
  activeGenotypeDefinitions: number
  genotypeDefinitions: number
  genotypingRecords: number
  breedingLines: number
}

export interface VoidGenotypingRecordInput {
  recordId: string
  expectedRevision: number
  reason: string
  projectId?: string
}

export interface CorrectGenotypingRecordInput {
  recordId: string
  expectedRevision: number
  reason: string
  genotypeDefinitionId: string
  state: GenotypingState
  assessedAt?: string
  method?: string
  notes?: string
  projectId?: string
}

export interface CorrectGenotypingRecordResult {
  voided: GenotypingRecord
  replacement: GenotypingRecord
}

export interface CreateBreedingLineInput {
  name: string
  description?: string
  genotypeDefinitionIds: string[]
}

export interface CreateColonyInput {
  breedingLineId: string
  name: string
  description?: string
}

export interface CreateBreedingPairInput {
  projectId?: string
  colonyId: string
  name: string
  maleAnimalId: string
  femaleAnimalIds: string[]
  startedAt?: string
}

export interface RetireBreedingPairInput {
  id: string
  expectedRevision: number
  endedAt?: string
}

export interface CreateMatingEventInput {
  projectId?: string
  breedingPairId: string
  maleAnimalId: string
  femaleAnimalId: string
  occurredAt?: string
  notes?: string
}

export interface CreateLitterInput {
  matingEventId: string
  bornOn: string
  sizeTotal: number
  drafts: Array<{ temporaryLabel: string; sex: Animal['sex'] }>
  notes?: string
}

export interface RegisterAnimalDraftInput {
  draftId: string
  expectedRevision: number
  identifierScope: 'lab' | 'project'
  projectId?: string
  displayId: string
  strain?: string
  initialCageId?: string
}

export interface BreedingPredictionInput {
  maleGenotypeDefinitionId: string
  femaleGenotypeDefinitionId: string
}

export interface CreateExperimentEventInput {
  experimentId: string
  eventKey: string
  label: string
  occurredAt?: string
  details?: Record<string, unknown>
}

export interface CreateObservationDefinitionInput {
  experimentId: string
  key: string
  label: string
  valueType: ObservationValueType
  unit?: string
  categories?: string[]
  policy: ObservationPolicy
}

export interface ObservationFilter {
  experimentId: string
  experimentEventId?: string
  subjectType?: ObservationSubjectType
  subjectId?: string
}

export interface RecordObservationInput {
  experimentId: string
  experimentEventId: string
  definitionId: string
  subjectType: ObservationSubjectType
  subjectId: string
  context?: Record<string, unknown>
  value: ObservationValueData
  recordedAt?: string
  notes?: string
}

export interface ReviseObservationInput {
  observationId: string
  expectedRevision: number
  value: ObservationValueData
  recordedAt?: string
  notes?: string
}

export type AttachmentTarget = 'project' | 'animal' | 'experiment' | 'measurement' | 'sample'

export interface AttachmentMetadata {
  id: string
  projectId?: string
  entityType: AttachmentTarget
  entityId: string
  fileName: string
  mediaType?: string
  sizeBytes: number
  sha256: string
  version: number
  revision: number
  contentHref: string
  previewSupported: boolean
  previewHref?: string
  previewReason?: string
  createdAt: string
}

export interface AttachmentScope {
  entityType: AttachmentTarget
  entityId: string
  projectId?: string
}

export interface UploadAttachmentInput extends AttachmentScope {
  fileName: string
  mediaType?: string
  content: Blob
}

export interface DeleteAttachmentInput {
  id: string
  expectedRevision: number
  reason?: string
}

export interface OperationRecord {
  id: string
  operationCode: string
  operationVersion: number
  title: string
  summary: string
  entityType: string
  entityId: string
  entityNameSnapshot?: string
  entityRevision?: number
  projectId?: string
  actor: { actor_type: string; user_id?: string; display_name: string }
  source: string
  requestId?: string
  reason?: string
  operationParams: Record<string, unknown>
  before?: unknown
  after?: unknown
  occurredAt: string
  batchCount: number
}
export interface AttachmentLinkRecord {
  id: string
  attachment_id: string
  target_type: string
  target_id: string
  project_id: string
}
export interface LibraryRecord {
  attachment: AttachmentMetadata
  links: AttachmentLinkRecord[]
  derivatives: Array<Record<string, unknown>>
  status: string
}
export interface PrivateImageRecord {
  image: { id: string; project_id?: string; status: string; expires_at: string; meta: { revision: number } }
  fileName: string
  mediaType?: string
  sizeBytes: number
  sha256: string
  contentHref: string
  previewHref: string
  retentionDays: number
}
export interface AiExtractionRecord {
  id: string
  project_id: string
  experiment_id: string
  experiment_event_id: string
  private_image_id: string
  status: string
  items: Array<{ confidence: number; selected: boolean; source_label?: string; observation: { definition_id: string; subject_type: string; subject_id: string }; value: { value: unknown } }>
  meta: { revision: number }
}
export interface AiDiagnostics {
  runtimeConfigured: boolean
  labEnabled: boolean
  userEnabled: boolean
  providerPresetsAvailable: boolean
  status: string
  providerConfigured: boolean
  providerEnabled: boolean
  credentialConfigured: boolean
  supportsVision: boolean
  textModelConfigured: boolean
  visionModelConfigured: boolean
  localEndpointCount: number
  cloudEndpointCount: number
}
export interface AiLabSettings {
  enabled: boolean
  customUrlApprovalRequired: boolean
  configuredUserCount: number
  enabledUserCount: number
  visionUserCount: number
  revision: number
  maxAutonomyMode: AiAutonomyMode
}
export interface TechnicalLogPolicy {
  maxRows: number
  minRetentionDays: number
  revision: number
}
export interface TechnicalLogCleanupPreview {
  totalRows: number
  eligibleRows: number
  cutoff: string
  policyRevision: number
}

export interface AiProviderEndpoint {
  id: string
  providerKind: AiProviderKind
  label: string
  baseUrl: string
  enabled: boolean
  builtin: boolean
  revision: number
}

export interface SaveAiProviderEndpointInput {
  providerKind: AiProviderKind
  label: string
  baseUrl: string
  enabled: boolean
}

export interface ChangePasswordInput {
  currentPassword: string
  newPassword: string
}

export interface UpdateProfileInput {
  displayName: string
}

export interface CreateManagedUserInput {
  email: string
  displayName: string
  temporaryPassword: string
  currentPassword: string
  labRole?: LabRole
  projectRoles: Array<{ projectId: string; role: ProjectRole }>
}

export interface UpdateManagedUserProfileInput {
  expectedRevision: number
  email: string
  displayName: string
  currentPassword: string
}

export interface ResetManagedUserPasswordInput {
  expectedCredentialRevision: number
  temporaryPassword: string
  currentPassword: string
}

export interface SetManagedUserStatusInput {
  expectedRevision: number
  status: ManagedUserStatus
  currentPassword: string
}

export interface GrantLabRoleInput {
  expectedUserRevision: number
  role: LabRole
  currentPassword: string
}

export interface GrantProjectRoleInput {
  expectedUserRevision: number
  projectId: string
  role: ProjectRole
  currentPassword: string
}

export interface UpdateLabRoleInput {
  expectedRevision: number
  role: LabRole
  currentPassword: string
}

export interface UpdateProjectRoleInput {
  expectedRevision: number
  role: ProjectRole
  currentPassword: string
}

export interface RevokeMembershipInput {
  expectedRevision: number
  currentPassword: string
}

export interface MuriArcGateway {
  readonly mode: GatewayMode
  readonly displayName: string
  readonly currentSession?: AuthSession
  readonly requiresLocalWelcome?: boolean
  listCages(context?: AnimalAccessContext): Promise<Cage[]>
  createCage(input: CreateCageInput): Promise<Cage>
  createAnimal(input: CreateAnimalInput): Promise<Animal>
  listAnimals(context?: AnimalAccessContext): Promise<Animal[]>
  getAnimal(id: string, context?: AnimalAccessContext): Promise<Animal | undefined>
  getAnimalDetail(id: string, context?: AnimalAccessContext): Promise<AnimalDetail>
  createAnimalSample(input: CreateAnimalSampleInput): Promise<AnimalSample>
  createPedigree(input: CreatePedigreeInput): Promise<PedigreeRelation>
  listGeneLoci(projectId?: string, includeArchived?: boolean): Promise<GeneLocus[]>
  geneLocusReferences(id: string, projectId?: string): Promise<GeneticsReferenceCounts>
  archiveGeneLocus(input: GeneticsArchiveInput): Promise<GeneLocus>
  restoreGeneLocus(input: GeneticsArchiveInput): Promise<GeneLocus>
  createGeneLocus(input: CreateGeneLocusInput): Promise<GeneLocus>
  listAlleles(locusId: string, projectId?: string, includeArchived?: boolean): Promise<GeneAllele[]>
  alleleReferences(id: string, projectId?: string): Promise<GeneticsReferenceCounts>
  archiveAllele(input: GeneticsArchiveInput): Promise<GeneAllele>
  restoreAllele(input: GeneticsArchiveInput): Promise<GeneAllele>
  createAllele(input: CreateAlleleInput): Promise<GeneAllele>
  listGenotypes(animalId: string, projectId?: string): Promise<AnimalGenotype[]>
  createGenotype(input: CreateGenotypeInput): Promise<AnimalGenotype>
  listGenotypeDefinitions(projectId?: string, includeArchived?: boolean): Promise<GenotypeDefinition[]>
  genotypeDefinitionReferences(id: string, projectId?: string): Promise<GeneticsReferenceCounts>
  archiveGenotypeDefinition(input: GeneticsArchiveInput): Promise<GenotypeDefinition>
  restoreGenotypeDefinition(input: GeneticsArchiveInput): Promise<GenotypeDefinition>
  createGenotypeDefinition(input: CreateGenotypeDefinitionInput): Promise<GenotypeDefinition>
  listGenotypingRecords(animalId: string, projectId?: string): Promise<GenotypingRecord[]>
  createGenotypingRecord(input: CreateGenotypingRecordInput): Promise<GenotypingRecord>
  voidGenotypingRecord(input: VoidGenotypingRecordInput): Promise<GenotypingRecord>
  correctGenotypingRecord(input: CorrectGenotypingRecordInput): Promise<CorrectGenotypingRecordResult>
  listBreedingLines(): Promise<BreedingLine[]>
  createBreedingLine(input: CreateBreedingLineInput): Promise<BreedingLine>
  listColonies(breedingLineId?: string): Promise<Colony[]>
  createColony(input: CreateColonyInput): Promise<Colony>
  listBreedingPairs(colonyId?: string): Promise<BreedingPair[]>
  createBreedingPair(input: CreateBreedingPairInput): Promise<BreedingPair>
  retireBreedingPair(input: RetireBreedingPairInput): Promise<BreedingPair>
  listMatingEvents(breedingPairId: string): Promise<MatingEvent[]>
  createMatingEvent(input: CreateMatingEventInput): Promise<MatingEvent>
  listLitters(breedingPairId: string): Promise<Litter[]>
  createLitter(input: CreateLitterInput): Promise<CreatedLitter>
  listAnimalDrafts(litterId: string): Promise<AnimalDraft[]>
  registerAnimalDraft(input: RegisterAnimalDraftInput): Promise<RegisteredAnimalDraft>
  predictBreeding(input: BreedingPredictionInput): Promise<LocusPrediction[]>
  moveAnimals(animalIds: string[], targetCageId: string): Promise<void>
  createProject(input: CreateProjectInput): Promise<ProjectSummary>
  listProjects(): Promise<ProjectSummary[]>
  listProjectAnimalAssignments?(projectId: string): Promise<ProjectAnimalAssignment[]>
  assignAnimalsToProject?(projectId: string, animalIds: string[], reason?: string): Promise<ProjectAnimalAssignment[]>
  removeAnimalsFromProject?(projectId: string, assignments: Array<{ assignmentId: string; expectedRevision: number }>): Promise<ProjectAnimalAssignment[]>
  listPublishedTemplates(): Promise<ExperimentTemplateSummary[]>
  createPublishedTemplate(input: CreatePublishedTemplateInput): Promise<ExperimentTemplateSummary>
  createExperiment(input: CreateExperimentInput): Promise<Experiment>
  completeExperiment(id: string, expectedRevision: number): Promise<Experiment>
  cancelExperiment(id: string, expectedRevision: number): Promise<Experiment>
  listExperiments(): Promise<Experiment[]>
  listCohorts(experimentId: string): Promise<Cohort[]>
  createCohort(input: CreateCohortInput): Promise<Cohort>
  listParticipations(projectId: string, experimentId: string): Promise<Participation[]>
  enrollAnimal(input: EnrollAnimalInput): Promise<Participation>
  completeParticipation(id: string, expectedRevision: number): Promise<Participation>
  withdrawParticipation(id: string, expectedRevision: number): Promise<Participation>
  listProcedures(experimentId: string): Promise<Procedure[]>
  createProcedure(input: CreateProcedureInput): Promise<Procedure>
  listExperimentEvents(experimentId: string): Promise<ExperimentEvent[]>
  createExperimentEvent(input: CreateExperimentEventInput): Promise<ExperimentEvent>
  listObservationDefinitions(experimentId: string): Promise<ObservationDefinition[]>
  createObservationDefinition(input: CreateObservationDefinitionInput): Promise<ObservationDefinition>
  listObservations(filter: ObservationFilter): Promise<Observation[]>
  recordObservation(input: RecordObservationInput): Promise<RecordedObservation>
  listObservationValues(observationId: string): Promise<ObservationValueRecord[]>
  reviseObservation(input: ReviseObservationInput): Promise<RecordedObservation>
  listDataJobs(): Promise<DataJob[]>
  listAttachments?(scope: AttachmentScope): Promise<AttachmentMetadata[]>
  uploadAttachment?(input: UploadAttachmentInput): Promise<AttachmentMetadata>
  downloadAttachment?(id: string): Promise<Blob>
  deleteAttachment?(input: DeleteAttachmentInput): Promise<AttachmentMetadata>
  getWorkspaceSettings?(): Promise<WorkspaceSettings>
  saveWorkspaceSettings?(input: WorkspaceSettings): Promise<WorkspaceSettings>
  getAiSettings?(): Promise<AiSettings>
  saveAiSettings?(input: SaveAiSettingsInput): Promise<AiSettings>
  clearAiApiKey?(): Promise<AiSettings>
  testAiSettings?(): Promise<{ ok: boolean; latencyMs: number; errorCode?: string }>
  getAiDiagnostics?(): Promise<AiDiagnostics>
  listAiProviderPresets?(): Promise<AiProviderPreset[]>
  getAiLabSettings?(): Promise<AiLabSettings>
  saveAiLabSettings?(input: { enabled: boolean; customUrlApprovalRequired: boolean; maxAutonomyMode: AiAutonomyMode }): Promise<AiLabSettings>
  listAiProviderEndpoints?(): Promise<AiProviderEndpoint[]>
  saveAiProviderEndpoint?(input: SaveAiProviderEndpointInput, id?: string): Promise<AiProviderEndpoint>
  disableAiProviderEndpoint?(id: string): Promise<AiProviderEndpoint>
  listOperations?(query?: URLSearchParams): Promise<OperationRecord[]>
  getTechnicalLogPolicy?(): Promise<TechnicalLogPolicy>
  saveTechnicalLogPolicy?(input: { maxRows: number; minRetentionDays: number; expectedRevision: number }): Promise<TechnicalLogPolicy>
  previewTechnicalLogCleanup?(): Promise<TechnicalLogCleanupPreview>
  cleanupTechnicalLogs?(input: { expectedPolicyRevision: number; expectedEligibleRows: number }): Promise<TechnicalLogCleanupPreview>
  listLibrary?(projectId: string, experimentId?: string): Promise<LibraryRecord[]>
  listPrivateImages?(): Promise<PrivateImageRecord[]>
  uploadPrivateImage?(file: File, conversationId?: string): Promise<PrivateImageRecord>
  archivePrivateImage?(id: string, projectId: string, expectedRevision: number): Promise<PrivateImageRecord>
  listAiExtractions?(projectId?: string): Promise<AiExtractionRecord[]>
  createAiExtraction?(input: { private_image_id: string; project_id: string; experiment_id: string; experiment_event_id: string }): Promise<AiExtractionRecord>
  approveAiExtraction?(id: string, expectedRevision: number, selectedIndexes: number[]): Promise<AiExtractionRecord>
  aiTurn(input: AiTurnInput): Promise<AiTurnResponse>
  createAiConversation(input: AiConversationCreateInput): Promise<AiConversationSummary>
  listAiConversations(projectId?: string, limit?: number): Promise<AiConversationSummary[]>
  queryAiConversations?(input?: AiConversationListInput): Promise<AiConversationSummary[]>
  getAiConversation(conversationId: string, limit?: number): Promise<AiConversationDetail>
  updateAiConversation?(
    conversationId: string,
    input: AiConversationUpdateInput,
  ): Promise<AiConversationSummary>
  uploadAiSource?(input: AiSourceUploadInput): Promise<AiSource>
  listAiSources?(input: AiSourceListInput): Promise<AiSource[]>
  archiveAiSource?(sourceId: string, input: AiSourceArchiveInput): Promise<AiSource>
  deleteAiSource?(sourceId: string): Promise<void>
  getAiAutonomy?(conversationId: string): Promise<AiAutonomyView>
  setAiAutonomy?(conversationId: string, input: AiAutonomyUpdateInput): Promise<AiAutonomyView>
  listAiDrafts(projectId?: string, status?: AiDraftStatus): Promise<AiWriteDraft[]>
  getAiDraft(draftId: string): Promise<AiWriteDraft>
  decideAiDraft(draftId: string, input: AiDraftDecisionInput): Promise<AiDraftDecisionResponse>
  restoreSession?(): Promise<AuthSession>
  login?(input: LoginInput): Promise<AuthSession>
  logout?(): Promise<void>
  changePassword?(input: ChangePasswordInput): Promise<AuthSession>
  updateProfile?(input: UpdateProfileInput): Promise<AuthSession>
  listManagedUsers?(projectId?: string): Promise<ManagedUser[]>
  createManagedUser?(input: CreateManagedUserInput): Promise<ManagedUser>
  updateManagedUserProfile?(userId: string, input: UpdateManagedUserProfileInput): Promise<ManagedUser>
  resetManagedUserPassword?(userId: string, input: ResetManagedUserPasswordInput): Promise<ManagedUser>
  setManagedUserStatus?(userId: string, input: SetManagedUserStatusInput): Promise<ManagedUser>
  grantLabRole?(userId: string, input: GrantLabRoleInput): Promise<ManagedUser>
  grantProjectRole?(userId: string, input: GrantProjectRoleInput): Promise<ManagedUser>
  updateLabRole?(membershipId: string, input: UpdateLabRoleInput): Promise<ManagedUser>
  updateProjectRole?(membershipId: string, input: UpdateProjectRoleInput): Promise<ManagedUser>
  revokeMembership?(membershipId: string, input: RevokeMembershipInput): Promise<ManagedUser>
}

type Invoke = <T>(command: string, args?: Record<string, unknown>) => Promise<T>
type GatewaySelection = GatewayMode | 'demo'

export class GatewayError extends Error {
  constructor(message: string, readonly code?: string) {
    super(message)
    this.name = 'GatewayError'
  }
}

function gatewayError(error: unknown): Error {
  if (error instanceof Error) return error
  if (typeof error === 'string') return new GatewayError(error)
  if (error && typeof error === 'object') {
    const record = error as Record<string, unknown>
    if (typeof record.message === 'string') {
      return new GatewayError(
        record.message,
        typeof record.code === 'string' ? record.code : undefined,
      )
    }
    if (record.error && typeof record.error === 'object') {
      const nested = record.error as Record<string, unknown>
      if (typeof nested.message === 'string') {
        return new GatewayError(
          nested.message,
          typeof nested.code === 'string' ? nested.code : undefined,
        )
      }
    }
  }
  return new GatewayError('MuriArc 操作失败')
}

export class LocalTauriGateway implements MuriArcGateway {
  readonly mode = 'local' as const
  readonly displayName = '个人本地库'
  readonly requiresLocalWelcome = true

  constructor(private readonly invokeCommand: Invoke = invoke) {}

  private async call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
    try {
      return await this.invokeCommand<T>(command, args)
    } catch (error) {
      throw gatewayError(error)
    }
  }

  listCages(_context?: AnimalAccessContext) { return this.call<Cage[]>('list_cages') }
  createCage(input: CreateCageInput) { return this.call<Cage>('create_cage', { input }) }
  createAnimal(input: CreateAnimalInput) { return this.call<Animal>('create_animal', { input }) }
  listAnimals(_context?: AnimalAccessContext) { return this.call<Animal[]>('list_animals') }
  getAnimal(id: string, _context?: AnimalAccessContext) {
    return this.call<Animal | undefined>('get_animal', { id })
  }
  getAnimalDetail(id: string, context?: AnimalAccessContext) {
    return this.call<AnimalDetail>('get_animal_detail', {
      animalId: id,
      projectId: context?.projectId,
    })
  }
  createAnimalSample(input: CreateAnimalSampleInput) {
    return this.call<AnimalSample>('create_animal_sample', { input })
  }
  createPedigree(input: CreatePedigreeInput) {
    return this.call<PedigreeRelation>('create_pedigree_relation', { input })
  }
  listGeneLoci(projectId?: string, includeArchived = false) {
    return this.call<GeneLocus[]>('list_gene_loci', { projectId, includeArchived })
  }
  geneLocusReferences(id: string, _projectId?: string) {
    return this.call<RawGeneticsReferenceCounts>('gene_locus_references', { id })
      .then(mapGeneticsReferenceCounts)
  }
  archiveGeneLocus(input: GeneticsArchiveInput) {
    const { projectId: _projectId, ...localInput } = input
    return this.call<GeneLocus>('archive_gene_locus', { input: localInput })
  }
  restoreGeneLocus(input: GeneticsArchiveInput) {
    const { projectId: _projectId, ...localInput } = input
    return this.call<GeneLocus>('restore_gene_locus', { input: localInput })
  }
  createGeneLocus(input: CreateGeneLocusInput) {
    return this.call<GeneLocus>('create_gene_locus', { input })
  }
  listAlleles(locusId: string, projectId?: string, includeArchived = false) {
    return this.call<GeneAllele[]>('list_alleles', { locusId, projectId, includeArchived })
  }
  alleleReferences(id: string, _projectId?: string) {
    return this.call<RawGeneticsReferenceCounts>('allele_references', { id })
      .then(mapGeneticsReferenceCounts)
  }
  archiveAllele(input: GeneticsArchiveInput) {
    const { projectId: _projectId, ...localInput } = input
    return this.call<GeneAllele>('archive_allele', { input: localInput })
  }
  restoreAllele(input: GeneticsArchiveInput) {
    const { projectId: _projectId, ...localInput } = input
    return this.call<GeneAllele>('restore_allele', { input: localInput })
  }
  createAllele(input: CreateAlleleInput) {
    return this.call<GeneAllele>('create_allele', { input })
  }
  listGenotypes(animalId: string, projectId?: string) {
    return this.call<AnimalGenotype[]>('list_genotypes', { animalId, projectId })
  }
  createGenotype(input: CreateGenotypeInput) {
    return this.call<AnimalGenotype>('create_genotype', { input })
  }
  listGenotypeDefinitions(_projectId?: string, includeArchived = false) {
    return this.call<RawGenotypeDefinition[]>('list_genotype_definitions', { includeArchived })
      .then((items) => items.map(mapGenotypeDefinition))
  }
  genotypeDefinitionReferences(id: string, _projectId?: string) {
    return this.call<RawGeneticsReferenceCounts>('genotype_definition_references', { id })
      .then(mapGeneticsReferenceCounts)
  }
  archiveGenotypeDefinition(input: GeneticsArchiveInput) {
    const { projectId: _projectId, ...localInput } = input
    return this.call<RawGenotypeDefinition>('archive_genotype_definition', { input: localInput })
      .then(mapGenotypeDefinition)
  }
  restoreGenotypeDefinition(input: GeneticsArchiveInput) {
    const { projectId: _projectId, ...localInput } = input
    return this.call<RawGenotypeDefinition>('restore_genotype_definition', { input: localInput })
      .then(mapGenotypeDefinition)
  }
  createGenotypeDefinition(input: CreateGenotypeDefinitionInput) {
    const { projectId: _projectId, ...localInput } = input
    return this.call<RawGenotypeDefinition>('create_genotype_definition', { input: localInput })
      .then(mapGenotypeDefinition)
  }
  listGenotypingRecords(animalId: string, _projectId?: string) {
    return this.call<RawGenotypingRecord[]>('list_genotyping_records', { animalId })
      .then((items) => items.map(mapGenotypingRecord))
  }
  createGenotypingRecord(input: CreateGenotypingRecordInput) {
    return this.call<RawGenotypingRecord>('create_genotyping_record', { input })
      .then(mapGenotypingRecord)
  }
  voidGenotypingRecord(input: VoidGenotypingRecordInput) {
    const { projectId: _projectId, ...localInput } = input
    return this.call<RawGenotypingRecord>('void_genotyping_record', { input: localInput })
      .then(mapGenotypingRecord)
  }
  correctGenotypingRecord(input: CorrectGenotypingRecordInput) {
    const { projectId: _projectId, ...localInput } = input
    return this.call<RawCorrectGenotypingRecordResult>('correct_genotyping_record', {
      input: localInput,
    }).then((result) => ({
      voided: mapGenotypingRecord(result.voided),
      replacement: mapGenotypingRecord(result.replacement),
    }))
  }
  listBreedingLines() {
    return this.call<RawBreedingLine[]>('list_breeding_lines')
      .then((items) => items.map(mapBreedingLine))
  }
  createBreedingLine(input: CreateBreedingLineInput) {
    return this.call<RawBreedingLine>('create_breeding_line', { input }).then(mapBreedingLine)
  }
  listColonies(breedingLineId?: string) {
    return this.call<RawColony[]>('list_colonies_v2', { breedingLineId })
      .then((items) => items.map(mapColony))
  }
  createColony(input: CreateColonyInput) {
    return this.call<RawColony>('create_colony_v2', { input }).then(mapColony)
  }
  listBreedingPairs(colonyId?: string) {
    return this.call<RawBreedingPair[]>('list_breeding_pairs', { colonyId })
      .then((items) => items.map(mapBreedingPair))
  }
  createBreedingPair(input: CreateBreedingPairInput) {
    const { projectId: _projectId, ...localInput } = input
    return this.call<RawBreedingPair>('create_breeding_pair', { input: localInput })
      .then(mapBreedingPair)
  }
  retireBreedingPair(input: RetireBreedingPairInput) {
    return this.call<RawBreedingPair>('retire_breeding_pair', { input }).then(mapBreedingPair)
  }
  listMatingEvents(breedingPairId: string) {
    return this.call<RawMatingEvent[]>('list_mating_events', { breedingPairId })
      .then((items) => items.map(mapMatingEvent))
  }
  createMatingEvent(input: CreateMatingEventInput) {
    const { projectId: _projectId, ...localInput } = input
    return this.call<RawMatingEvent>('create_mating_event', { input: localInput })
      .then(mapMatingEvent)
  }
  listLitters(breedingPairId: string) {
    return this.call<RawLitter[]>('list_litters', { breedingPairId })
      .then((items) => items.map(mapLitter))
  }
  async createLitter(input: CreateLitterInput): Promise<CreatedLitter> {
    const created = await this.call<RawCreatedLitter>('create_litter', { input })
    return { litter: mapLitter(created.litter), drafts: created.drafts.map(mapAnimalDraft) }
  }
  listAnimalDrafts(litterId: string) {
    return this.call<RawAnimalDraft[]>('list_animal_drafts', { litterId })
      .then((items) => items.map(mapAnimalDraft))
  }
  async registerAnimalDraft(input: RegisterAnimalDraftInput): Promise<RegisteredAnimalDraft> {
    const registered = await this.call<RawRegisteredAnimalDraft>('register_animal_draft', { input })
    return { draft: mapAnimalDraft(registered.draft), animal: mapAnimal(registered.animal) }
  }
  predictBreeding(input: BreedingPredictionInput) {
    return this.call<RawLocusPrediction[]>('predict_breeding', { input })
      .then((items) => items.map(mapLocusPrediction))
  }
  moveAnimals(animalIds: string[], targetCageId: string) {
    return this.call<void>('move_animals', { input: { animalIds, targetCageId } })
  }
  createProject(input: CreateProjectInput) {
    return this.call<ProjectSummary>('create_project', { input })
  }
  listProjects() { return this.call<ProjectSummary[]>('list_projects') }
  listPublishedTemplates() {
    return this.call<ExperimentTemplateSummary[]>('list_published_templates')
  }
  createPublishedTemplate(input: CreatePublishedTemplateInput) {
    return this.call<ExperimentTemplateSummary>('create_published_template', { input })
  }
  createExperiment(input: CreateExperimentInput) {
    return this.call<Experiment>('create_experiment', { input })
  }
  completeExperiment(id: string, expectedRevision: number) {
    return this.call<Experiment>('complete_experiment', { input: { id, expectedRevision } })
  }
  cancelExperiment(id: string, expectedRevision: number) {
    return this.call<Experiment>('cancel_experiment', { input: { id, expectedRevision } })
  }
  listExperiments() { return this.call<Experiment[]>('list_experiments') }
  listCohorts(experimentId: string) {
    return this.call<Cohort[]>('list_cohorts', { experimentId })
  }
  createCohort(input: CreateCohortInput) {
    return this.call<Cohort>('create_cohort', { input })
  }
  listParticipations(projectId: string, experimentId: string) {
    return this.call<Participation[]>('list_participations', { projectId, experimentId })
  }
  enrollAnimal(input: EnrollAnimalInput) {
    return this.call<Participation>('enroll_animal', { input })
  }
  completeParticipation(id: string, expectedRevision: number) {
    return this.call<Participation>('complete_participation', {
      input: { id, expectedRevision },
    })
  }
  withdrawParticipation(id: string, expectedRevision: number) {
    return this.call<Participation>('withdraw_participation', {
      input: { id, expectedRevision },
    })
  }
  listProcedures(experimentId: string) {
    return this.call<Procedure[]>('list_procedures', { experimentId })
  }
  createProcedure(input: CreateProcedureInput) {
    return this.call<Procedure>('create_procedure', { input })
  }
  listExperimentEvents(experimentId: string) {
    return this.call<RawExperimentEvent[]>('list_experiment_events', { experimentId })
      .then((items) => items.map(mapExperimentEvent))
  }
  createExperimentEvent(input: CreateExperimentEventInput) {
    return this.call<RawExperimentEvent>('create_experiment_event', { input })
      .then(mapExperimentEvent)
  }
  listObservationDefinitions(experimentId: string) {
    return this.call<RawObservationDefinition[]>('list_observation_definitions', { experimentId })
      .then((items) => items.map(mapObservationDefinition))
  }
  createObservationDefinition(input: CreateObservationDefinitionInput) {
    return this.call<RawObservationDefinition>('create_observation_definition', { input })
      .then(mapObservationDefinition)
  }
  listObservations(filter: ObservationFilter) {
    return this.call<RawObservation[]>('list_observations', { ...filter })
      .then((items) => items.map(mapObservation))
  }
  async recordObservation(input: RecordObservationInput): Promise<RecordedObservation> {
    return mapRecordedObservation(
      await this.call<RawRecordedObservation>('record_observation', { input }),
    )
  }
  listObservationValues(observationId: string) {
    return this.call<RawObservationValueRecord[]>('list_observation_values', { observationId })
      .then((items) => items.map(mapObservationValueRecord))
  }
  async reviseObservation(input: ReviseObservationInput): Promise<RecordedObservation> {
    return mapRecordedObservation(
      await this.call<RawRecordedObservation>('revise_observation', { input }),
    )
  }
  listDataJobs() { return this.call<DataJob[]>('list_data_jobs') }
  listAttachments(scope: AttachmentScope) {
    return this.call<AttachmentMetadata[]>('list_attachments', { input: scope })
  }
  async uploadAttachment(input: UploadAttachmentInput): Promise<AttachmentMetadata> {
    const bytes = Array.from(new Uint8Array(await input.content.arrayBuffer()))
    return this.call<AttachmentMetadata>('upload_attachment', {
      input: {
        entityType: input.entityType,
        entityId: input.entityId,
        projectId: input.projectId,
        fileName: input.fileName,
        mediaType: input.mediaType ?? (input.content.type || undefined),
        bytes,
      },
    })
  }
  async downloadAttachment(id: string): Promise<Blob> {
    const result = await this.call<{
      metadata: AttachmentMetadata
      bytes: number[]
    }>('download_attachment', { id })
    const bytes = Uint8Array.from(result.bytes)
    return new Blob([bytes.buffer], {
      type: result.metadata.mediaType ?? 'application/octet-stream',
    })
  }
  deleteAttachment(input: DeleteAttachmentInput): Promise<AttachmentMetadata> {
    return this.call<AttachmentMetadata>('delete_attachment', { input })
  }
  getWorkspaceSettings() { return this.call<WorkspaceSettings>('get_workspace_settings') }
  saveWorkspaceSettings(input: WorkspaceSettings) {
    return this.call<WorkspaceSettings>('save_workspace_settings', { input })
  }
  getAiSettings() { return this.call<AiSettings>('get_ai_settings') }
  saveAiSettings(input: SaveAiSettingsInput) {
    return this.call<AiSettings>('save_ai_settings', { input })
  }
  clearAiApiKey() { return this.call<AiSettings>('clear_ai_api_key') }
  async listAiProviderPresets() { return structuredClone(builtinAiProviderPresets) }
  async aiTurn(input: AiTurnInput) {
    return mapAiTurn(await this.call<RawAiTurnResponse>('ai_turn', { input }))
  }
  createAiConversation(input: AiConversationCreateInput) {
    return this.call<RawAiConversationSummary>('create_ai_conversation', { input })
      .then(mapAiConversationSummary)
  }
  listAiConversations(projectId?: string, limit = 50) {
    return this.call<RawAiConversationSummary[]>('list_ai_conversations', { projectId, limit })
      .then((items) => items.map(mapAiConversationSummary))
  }
  queryAiConversations(input: AiConversationListInput = {}) {
    return this.call<RawAiConversationSummary[]>('list_ai_conversations', {
      projectId: input.projectId,
      titleQuery: input.titleQuery,
      archive: input.archive ?? 'active',
      limit: input.limit ?? 100,
    }).then((items) => items.map(mapAiConversationSummary))
  }
  async getAiConversation(conversationId: string, limit = 200) {
    const detail = await this.call<RawAiConversationDetail>('get_ai_conversation', {
      conversationId,
      limit,
    })
    return mapAiConversationDetail(detail)
  }
  updateAiConversation(conversationId: string, input: AiConversationUpdateInput) {
    return this.call<RawAiConversationSummary>('update_ai_conversation', {
      conversationId,
      input,
    }).then(mapAiConversationSummary)
  }
  async uploadAiSource(input: AiSourceUploadInput): Promise<AiSource> {
    const bytes = Array.from(new Uint8Array(await input.file.arrayBuffer()))
    const source = await this.call<RawAiSource>('upload_ai_source', {
      input: {
        fileName: input.file.name,
        mediaType: input.file.type || 'application/octet-stream',
        conversationId: input.conversationId,
        projectId: input.projectId,
        bytes,
      },
    })
    return mapAiSource(source)
  }
  async listAiSources(input: AiSourceListInput): Promise<AiSource[]> {
    const sources = await this.call<RawAiSource[]>('list_ai_sources', {
      input: {
        conversationId: input.conversationId,
        projectId: input.projectId,
        status: input.status,
      },
    })
    return sources.map(mapAiSource)
  }
  async archiveAiSource(sourceId: string, input: AiSourceArchiveInput): Promise<AiSource> {
    const source = await this.call<RawAiSource>('archive_ai_source', {
      sourceId,
      input: {
        projectId: input.projectId,
        expectedRevision: input.expectedRevision,
      },
    })
    return mapAiSource(source)
  }
  async deleteAiSource(sourceId: string): Promise<void> {
    await this.call<unknown>('delete_ai_source', { sourceId })
  }
  getAiAutonomy(conversationId: string) {
    return this.call<AiAutonomyView>('get_ai_autonomy', { conversationId })
  }
  setAiAutonomy(conversationId: string, input: AiAutonomyUpdateInput) {
    return this.call<AiAutonomyView>('set_ai_autonomy', {
      conversationId,
      input: {
        mode: input.mode,
        expectedRevision: input.expectedRevision,
        declared: Boolean(input.declared),
      },
    })
  }
  listAiDrafts(projectId?: string, status?: AiDraftStatus) {
    return this.call<AiWriteDraft[]>('list_ai_drafts', { projectId, status })
  }
  getAiDraft(draftId: string) {
    return this.call<AiWriteDraft>('get_ai_draft', { draftId })
  }
  decideAiDraft(draftId: string, input: AiDraftDecisionInput) {
    const localInput: AiDraftDecisionInput = {
      expectedRevision: input.expectedRevision,
      decision: input.decision,
      ...(input.statement === undefined ? {} : { statement: input.statement }),
    }
    return this.call<AiDraftDecisionResponse>('decide_ai_draft', { draftId, input: localInput })
  }
}

interface ApiCollection<T> { data: T[]; count: number; request_id: string }
interface ApiItem<T> { data: T; request_id: string }

interface RawCage {
  id: string
  section: string
  display_id: string
  location?: string | null
  capacity: number
}

interface RawProjectAnimalAssignment {
  id: string
  project_id: string
  animal_id: string
  assigned_by?: string | null
  reason?: string | null
  meta: {
    created_at: string
    revision: number
  }
}

interface RawAnimal {
  id: string
  display_id: string
  legacy_id?: string | null
  strain?: string | null
  sex: 'male' | 'female' | 'unknown'
  birth_date?: string | null
  current_cage_id?: string | null
  current_status: 'planned' | 'alive' | 'in_experiment' | 'sampled' | 'deceased' | 'euthanized' | 'lost' | 'archived'
}

interface RawAnimalOverview {
  animal: RawAnimal
  genotype: string
  projects: RawProject[]
  latest_weight?: { value: number; unit?: string | null; measured_at: string } | null
}

interface RawDetailExperiment {
  project: RawProject
  experiment: {
    id: string
    name: string
    status: 'draft' | 'active' | 'completed' | 'cancelled' | 'archived'
    starts_at?: string | null
    ends_at?: string | null
    revision: number
  }
  participation: {
    id: string
    status: 'enrolled' | 'completed' | 'withdrawn'
    enrolled_at: string
    exited_at?: string | null
    revision: number
  }
  cohort?: { id: string; name: string } | null
}

interface RawDetailMeasurement {
  id: string
  project_id: string
  experiment_id?: string | null
  key: string
  label: string
  value: AnimalDetail['measurements'][number]['value']
  unit?: string | null
  measured_at: string
  status: 'draft' | 'signed'
  revision: number
}

interface RawDetailSample {
  id: string
  project_id: string
  experiment_id?: string | null
  sample_type: string
  quantity?: number | null
  unit?: string | null
  location?: string | null
  collected_at: string
  revision: number
}

interface RawDetailPedigree {
  id: string
  direction: 'parent' | 'offspring'
  parent_type: 'father' | 'mother' | 'unknown'
  related_animal: {
    id: string
    display_id: string
    sex: Animal['sex']
    strain?: string | null
    current_status: RawAnimal['current_status']
  }
  revision: number
}

interface RawDetailAttachment extends Omit<RawAttachment, 'meta'> {
  created_at: string
  revision: number
}

interface RawAuditSummary {
  id: string
  action: AnimalDetail['audits'][number]['action']
  actor: string
  source: AnimalDetail['audits'][number]['source']
  reason?: string | null
  occurred_at: string
  revision?: number | null
}

interface RawProvenanceSummary {
  id: string
  source: AnimalDetail['provenance'][number]['source']
  actor?: string | null
  recorded_at: string
  request_id?: string | null
}

interface RawAnimalDetail {
  events: RawAnimalEvent[]
  experiments: RawDetailExperiment[]
  measurements: RawDetailMeasurement[]
  pedigree: RawDetailPedigree[]
  samples: RawDetailSample[]
  attachments: RawDetailAttachment[]
  audit_visible: boolean
  audits: RawAuditSummary[]
  provenance: RawProvenanceSummary[]
}

interface RawAnimalEvent {
  id: string
  kind: { type: string; [key: string]: unknown }
  occurred_at: string
  recorded_by?: string | null
  notes?: string | null
}

interface RawGeneLocus {
  id: string
  symbol: string
  description?: string | null
  meta: RawRecordMeta
}

interface RawAllele {
  id: string
  locus_id: string
  symbol: string
  description?: string | null
  is_wild_type: boolean
  meta: RawRecordMeta
}

interface RawGenotype {
  id: string
  animal_id: string
  locus_id: string
  allele_1_id?: string | null
  allele_2_id?: string | null
  assessed_at?: string | null
  meta: { revision: number }
}

interface RawRecordMeta {
  created_at: string
  updated_at: string
  deleted_at?: string | null
  revision: number
}

interface RawGenotypeComponent {
  id: string
  genotype_definition_id: string
  locus_id: string
  allele_1_id: string
  allele_2_id?: string | null
  mode: GenotypeComponentMode
  display_order: number
  meta: RawRecordMeta
}

interface RawGenotypeDefinition {
  id: string
  lab_id: string
  name: string
  description?: string | null
  components: RawGenotypeComponent[]
  meta: RawRecordMeta
}

interface RawGenotypingRecord {
  id: string
  lab_id: string
  project_id?: string | null
  animal_id: string
  genotype_definition_id: string
  state: GenotypingState
  assessed_at?: string | null
  method?: string | null
  notes?: string | null
  supersedes_record_id?: string | null
  voided_at?: string | null
  void_reason?: string | null
  meta: RawRecordMeta
}

interface RawGeneticsReferenceCounts {
  active_genotype_definitions: number
  genotype_definitions: number
  genotyping_records: number
  breeding_lines: number
}

interface RawCorrectGenotypingRecordResult {
  voided: RawGenotypingRecord
  replacement: RawGenotypingRecord
}

interface RawBreedingLine {
  id: string
  lab_id: string
  name: string
  description?: string | null
  genotype_definition_ids: string[]
  meta: RawRecordMeta
}

interface RawColony {
  id: string
  lab_id: string
  breeding_line_id: string
  name: string
  description?: string | null
  meta: RawRecordMeta
}

interface RawBreedingPairMember {
  id: string
  breeding_pair_id: string
  animal_id: string
  role: 'male' | 'female'
  joined_at: string
  left_at?: string | null
  meta: RawRecordMeta
}

interface RawBreedingPair {
  id: string
  lab_id: string
  colony_id: string
  name: string
  status: 'active' | 'retired'
  started_at: string
  ended_at?: string | null
  members: RawBreedingPairMember[]
  meta: RawRecordMeta
}

interface RawMatingEvent {
  id: string
  lab_id: string
  breeding_pair_id: string
  male_animal_id: string
  female_animal_id: string
  occurred_at: string
  notes?: string | null
  meta: RawRecordMeta
}

interface RawLitter {
  id: string
  lab_id: string
  mating_event_id: string
  born_on: string
  size_total: number
  size_alive: number
  notes?: string | null
  meta: RawRecordMeta
}

interface RawAnimalDraft {
  id: string
  lab_id: string
  litter_id: string
  temporary_label: string
  sex: Animal['sex']
  birth_date: string
  status: AnimalDraft['status']
  registered_animal_id?: string | null
  meta: RawRecordMeta
}

interface RawCreatedLitter {
  litter: RawLitter
  drafts: RawAnimalDraft[]
}

interface RawRegisteredAnimalDraft {
  draft: RawAnimalDraft
  animal: RawAnimal
}

interface RawMendelianOutcome {
  paternal_allele_id?: string | null
  maternal_allele_id?: string | null
  probability: number
}

interface RawLocusPrediction {
  locus_id: string
  outcomes: RawMendelianOutcome[]
}

interface RawExperimentEvent {
  id: string
  lab_id: string
  project_id: string
  experiment_id: string
  event_key: string
  label: string
  occurred_at: string
  details: Record<string, unknown>
  meta: RawRecordMeta
}

interface RawObservationDefinition {
  id: string
  lab_id: string
  project_id: string
  experiment_id: string
  key: string
  label: string
  value_type: ObservationValueType
  unit?: string | null
  categories: string[]
  policy: ObservationPolicy
  meta: RawRecordMeta
}

interface RawObservation {
  id: string
  lab_id: string
  project_id: string
  experiment_id: string
  experiment_event_id: string
  definition_id: string
  subject_type: ObservationSubjectType
  subject_id: string
  context: Record<string, unknown>
  current_value_version: number
  meta: RawRecordMeta
}

interface RawObservationValueRecord {
  id: string
  observation_id: string
  version: number
  value: ObservationValueData
  recorded_at: string
  recorded_by?: string | null
  notes?: string | null
  meta: RawRecordMeta
}

interface RawRecordedObservation {
  observation: RawObservation
  value: RawObservationValueRecord
}

interface RawProject { id: string; name: string }
interface RawTemplate {
  id: string
  name: string
  version: number
  status: 'draft' | 'published' | 'retired'
  meta: { revision: number }
}
interface RawCohort {
  id: string
  experiment_id: string
  name: string
  description?: string | null
}
interface RawParticipation {
  id: string
  experiment_id: string
  animal_id: string
  cohort_id?: string | null
  status: Participation['status']
  enrolled_at: string
  exited_at?: string | null
  genotype_snapshot?: Array<{
    genotyping_record_id: string
    genotype_definition_id: string
    state: GenotypingState
    assessed_at?: string | null
  }>
  meta: { revision: number }
}
interface RawProcedure {
  id: string
  experiment_id: string
  animal_id?: string | null
  name: string
  scheduled_at?: string | null
  performed_at?: string | null
  status: Procedure['status']
  details: Record<string, unknown>
}
interface RawExperiment {
  id: string
  project_id: string
  name: string
  status: 'draft' | 'active' | 'completed' | 'cancelled' | 'archived'
  starts_at?: string | null
  meta: { revision: number }
}
interface RawJob {
  id: string
  project_id?: string | null
  kind: 'import' | 'export' | 'snapshot' | 'bulk_operation'
  status: 'queued' | 'parsing' | 'validating' | 'awaiting_confirmation' | 'writing' | 'completed' | 'failed' | 'cancelled'
  progress_current: number
  progress_total?: number | null
  result_available: boolean
  error_report_available: boolean
  cancellation_requested: boolean
  revision: number
  created_at: string
  updated_at: string
}

interface RawAttachment {
  id: string
  project_id?: string | null
  entity_type: AttachmentTarget
  entity_id: string
  file_name: string
  media_type?: string | null
  size_bytes: number
  sha256: string
  version: number
  content_href: string
  preview_supported: boolean
  preview_href?: string | null
  preview_reason?: string | null
  meta: { created_at: string; revision?: number }
}

interface RawAuthUser {
  id: string
  lab_id: string
  email?: string | null
  display_name: string
  lab_roles: AuthUser['labRoles']
  project_roles: Array<{ project_id: string; role: AuthUser['projectRoles'][number]['role'] }>
  ai_scopes?: AuthUser['aiScopes']
  authentication: AuthUser['authentication']
  must_change_password?: boolean
  is_environment_root?: boolean
}

interface RawLoginResponse {
  user: RawAuthUser
  csrf_token: string
  expires_at: string
}

interface RawCsrfResponse {
  csrf_token: string
  expires_at: string
}

interface RawAiCitation {
  entity_type: string
  entity_id: string
  revision?: number | null
}

interface RawAiToolRun {
  tool_run_id: string
  provider_call_id: string
  tool: string
  arguments: unknown
  outcome: 'read' | 'write_draft'
  citations: RawAiCitation[]
  draft_id?: string | null
}

interface RawAiTurnResponse {
  conversationId: string
  content: string
  citations: RawAiCitation[]
  toolRuns: RawAiToolRun[]
  drafts: AiWriteDraft[]
  trace: {
    providerId: string
    model: string
    usage: {
      provider_calls: number
      tool_calls: number
      input_tokens: number
      output_tokens: number
      total_tokens: number
    }
    context?: {
      estimatedInputTokens: number
      inputTokenCountIsEstimate: boolean
      contextTrimmed: boolean
      trimmedHistoryTurns: number
      trimReasons?: string[]
    }
  }
  incompleteReason?:
    | 'iteration_limit_exceeded'
    | 'tool_call_limit_exceeded'
    | 'total_timeout_exceeded'
    | 'provider_failure'
    | 'tool_execution_failure'
    | null
  autonomy?: AiAutonomyView
}

interface RawAiConversationSourceRef {
  sourceId?: string
  source_id?: string
  sourceRevision?: number
  source_revision?: number
  fileName?: string
  file_name?: string
  mediaType?: string | null
  media_type?: string | null
  sizeBytes?: number
  size_bytes?: number
}

interface RawAiConversationMessage {
  id: string
  sequence: number
  role: 'user' | 'assistant'
  content: string
  sourceRefs?: RawAiConversationSourceRef[]
  source_refs?: RawAiConversationSourceRef[]
  response?: RawAiTurnResponse | null
  createdAt: string
}

interface RawAiConversationSummary {
  id: string
  projectId?: string | null
  project_id?: string | null
  title: string
  pinnedAt?: string | null
  pinned_at?: string | null
  archivedAt?: string | null
  archived_at?: string | null
  createdAt: string
  created_at?: string
  updatedAt: string
  updated_at?: string
  revision: number
}

interface RawAiConversationDetail {
  conversation: RawAiConversationSummary
  messages: RawAiConversationMessage[]
}

interface RawAiSource {
  id: string
  conversationId?: string | null
  conversation_id?: string | null
  projectId?: string | null
  project_id?: string | null
  fileName: string
  file_name?: string
  mediaType: string
  media_type?: string
  sizeBytes: number
  size_bytes?: number
  status: string
  revision: number
  createdAt: string
  created_at?: string
  expiresAt?: string
  expires_at?: string
}

export class HttpGatewayError extends GatewayError {
  constructor(readonly status: number, message: string, code?: string) {
    super(message, code)
    this.name = 'HttpGatewayError'
  }
}

export interface RemoteHttpGatewayOptions {
  baseUrl?: string
  fetch?: typeof globalThis.fetch
  onUnauthorized?: () => void
}

interface RemoteRequestOptions {
  csrf?: boolean
  accept?: string
  contentType?: string | null
}

export class RemoteHttpGateway implements MuriArcGateway {
  readonly mode = 'remote' as const
  readonly displayName = '共享实验室'
  private readonly baseUrl: string
  private readonly fetchRequest: typeof globalThis.fetch
  private readonly onUnauthorized: () => void
  private csrfToken?: string
  private session?: AuthSession
  private restoringSession?: Promise<AuthSession>

  get currentSession(): AuthSession | undefined {
    return this.session
  }

  constructor(options: RemoteHttpGatewayOptions = {}) {
    this.baseUrl = (options.baseUrl ?? import.meta.env.VITE_MURIARC_API_BASE ?? '/api/v1').replace(/\/$/, '')
    this.fetchRequest = options.fetch ?? globalThis.fetch.bind(globalThis)
    this.onUnauthorized = options.onUnauthorized ?? redirectToRemoteLogin
  }

  private async send(
    path: string,
    init: RequestInit = {},
    options: RemoteRequestOptions = {},
  ): Promise<Response> {
    const headers = new Headers(init.headers)
    headers.set('Accept', options.accept ?? 'application/json')
    if (init.body && options.contentType !== null) {
      headers.set('Content-Type', options.contentType ?? 'application/json')
    }
    const method = (init.method ?? 'GET').toUpperCase()
    const requiresCsrf = options.csrf ?? !['GET', 'HEAD', 'OPTIONS'].includes(method)
    if (requiresCsrf) {
      if (!this.csrfToken) {
        throw new HttpGatewayError(
          403,
          '当前会话缺少 CSRF 凭据，请刷新会话后重试',
          'csrf_unavailable',
        )
      }
      headers.set('X-CSRF-Token', this.csrfToken)
    }

    let response: Response
    try {
      response = await this.fetchRequest(`${this.baseUrl}${path}`, {
        ...init,
        credentials: 'include',
        headers,
      })
    } catch (error) {
      throw new Error(`无法连接 MuriArc Server：${gatewayError(error).message}`)
    }

    if (!response.ok) {
      const payload = await response.json().catch(() => undefined) as { error?: { code?: string; message?: string } } | undefined
      if (response.status === 401) {
        this.clearSession()
        this.onUnauthorized()
      } else if (payload?.error?.code === 'password_change_required') {
        if (this.session) {
          this.session = {
            ...this.session,
            user: { ...this.session.user, mustChangePassword: true },
          }
          currentAuthSession.value = this.session
        }
        redirectToPasswordChange()
      }
      throw new HttpGatewayError(response.status, payload?.error?.message ?? `Server 请求失败（${response.status}）`, payload?.error?.code)
    }
    return response
  }

  private async request<T>(
    path: string,
    init: RequestInit = {},
    options: RemoteRequestOptions = {},
  ): Promise<T> {
    const response = await this.send(path, init, options)
    return await response.json().catch(() => undefined) as T
  }

  private clearSession() {
    this.session = undefined
    clearProjectContext()
    this.csrfToken = undefined
    this.restoringSession = undefined
  }

  private updateSessionUser(raw: RawAuthUser): AuthSession {
    if (!this.session) {
      throw new HttpGatewayError(401, '当前登录会话已失效', 'session_unavailable')
    }
    this.session = { ...this.session, user: mapAuthUser(raw) }
    currentAuthSession.value = this.session
    return this.session
  }

  async restoreSession(): Promise<AuthSession> {
    if (this.session) return this.session
    if (this.restoringSession) return this.restoringSession
    this.restoringSession = this.request<ApiItem<RawAuthUser>>('/auth/session')
      .then(async (response) => {
        const csrf = await this.request<ApiItem<RawCsrfResponse>>('/auth/csrf')
        this.csrfToken = csrf.data.csrf_token
        this.session = {
          user: mapAuthUser(response.data),
          csrfAvailable: true,
          expiresAt: csrf.data.expires_at,
        }
        currentAuthSession.value = this.session
        return this.session
      })
      .finally(() => {
        this.restoringSession = undefined
      })
    return this.restoringSession
  }

  async login(input: LoginInput): Promise<AuthSession> {
    this.clearSession()
    const response = await this.request<ApiItem<RawLoginResponse>>('/auth/login', {
      method: 'POST',
      body: JSON.stringify({ email: input.email, password: input.password }),
    }, { csrf: false })
    this.csrfToken = response.data.csrf_token
    this.session = {
      user: mapAuthUser(response.data.user),
      csrfAvailable: true,
      expiresAt: response.data.expires_at,
    }
    currentAuthSession.value = this.session
    return this.session
  }

  async logout(): Promise<void> {
    await this.request<void>('/auth/logout', { method: 'POST' })
    this.clearSession()
  }

  async changePassword(input: ChangePasswordInput): Promise<AuthSession> {
    const response = await this.request<ApiItem<RawAuthUser>>('/auth/password/change', {
      method: 'POST',
      body: JSON.stringify(input),
    })
    return this.updateSessionUser(response.data)
  }

  async updateProfile(input: UpdateProfileInput): Promise<AuthSession> {
    const response = await this.request<ApiItem<RawAuthUser>>('/auth/profile', {
      method: 'PATCH',
      body: JSON.stringify(input),
    })
    return this.updateSessionUser(response.data)
  }

  async listManagedUsers(projectId?: string): Promise<ManagedUser[]> {
    const suffix = projectId ? `?project_id=${encodeURIComponent(projectId)}` : ''
    const response = await this.request<ApiCollection<ManagedUser>>(`/admin/users${suffix}`)
    return response.data
  }

  async getTechnicalLogPolicy(): Promise<TechnicalLogPolicy> {
    return (await this.request<ApiItem<TechnicalLogPolicy>>('/admin/technical-logs/policy')).data
  }

  async saveTechnicalLogPolicy(
    input: { maxRows: number; minRetentionDays: number; expectedRevision: number },
  ): Promise<TechnicalLogPolicy> {
    return (await this.request<ApiItem<TechnicalLogPolicy>>('/admin/technical-logs/policy', {
      method: 'PUT', body: JSON.stringify(input),
    })).data
  }

  async previewTechnicalLogCleanup(): Promise<TechnicalLogCleanupPreview> {
    return (await this.request<ApiItem<TechnicalLogCleanupPreview>>(
      '/admin/technical-logs/cleanup/preview',
    )).data
  }

  async cleanupTechnicalLogs(
    input: { expectedPolicyRevision: number; expectedEligibleRows: number },
  ): Promise<TechnicalLogCleanupPreview> {
    return (await this.request<ApiItem<TechnicalLogCleanupPreview>>(
      '/admin/technical-logs/cleanup',
      { method: 'POST', body: JSON.stringify(input) },
    )).data
  }

  async createManagedUser(input: CreateManagedUserInput): Promise<ManagedUser> {
    const response = await this.request<ApiItem<ManagedUser>>('/admin/users', {
      method: 'POST',
      body: JSON.stringify(input),
    })
    return response.data
  }

  async updateManagedUserProfile(
    userId: string,
    input: UpdateManagedUserProfileInput,
  ): Promise<ManagedUser> {
    const response = await this.request<ApiItem<ManagedUser>>(
      `/admin/users/${encodeURIComponent(userId)}/profile`,
      { method: 'PATCH', body: JSON.stringify(input) },
    )
    return response.data
  }

  async resetManagedUserPassword(
    userId: string,
    input: ResetManagedUserPasswordInput,
  ): Promise<ManagedUser> {
    const response = await this.request<ApiItem<ManagedUser>>(
      `/admin/users/${encodeURIComponent(userId)}/password-reset`,
      { method: 'POST', body: JSON.stringify(input) },
    )
    return response.data
  }

  async setManagedUserStatus(
    userId: string,
    input: SetManagedUserStatusInput,
  ): Promise<ManagedUser> {
    const response = await this.request<ApiItem<ManagedUser>>(
      `/admin/users/${encodeURIComponent(userId)}/status`,
      { method: 'PATCH', body: JSON.stringify(input) },
    )
    return response.data
  }

  async grantLabRole(userId: string, input: GrantLabRoleInput): Promise<ManagedUser> {
    const response = await this.request<ApiItem<ManagedUser>>(
      `/admin/users/${encodeURIComponent(userId)}/lab-membership`,
      { method: 'POST', body: JSON.stringify(input) },
    )
    return response.data
  }

  async grantProjectRole(userId: string, input: GrantProjectRoleInput): Promise<ManagedUser> {
    const response = await this.request<ApiItem<ManagedUser>>(
      `/admin/users/${encodeURIComponent(userId)}/project-memberships`,
      { method: 'POST', body: JSON.stringify(input) },
    )
    return response.data
  }

  async updateLabRole(
    membershipId: string,
    input: UpdateLabRoleInput,
  ): Promise<ManagedUser> {
    const response = await this.request<ApiItem<ManagedUser>>(
      `/admin/memberships/${encodeURIComponent(membershipId)}/lab-role`,
      { method: 'PATCH', body: JSON.stringify(input) },
    )
    return response.data
  }

  async updateProjectRole(
    membershipId: string,
    input: UpdateProjectRoleInput,
  ): Promise<ManagedUser> {
    const response = await this.request<ApiItem<ManagedUser>>(
      `/admin/memberships/${encodeURIComponent(membershipId)}/project-role`,
      { method: 'PATCH', body: JSON.stringify(input) },
    )
    return response.data
  }

  async revokeMembership(
    membershipId: string,
    input: RevokeMembershipInput,
  ): Promise<ManagedUser> {
    const response = await this.request<ApiItem<ManagedUser>>(
      `/admin/memberships/${encodeURIComponent(membershipId)}`,
      { method: 'DELETE', body: JSON.stringify(input) },
    )
    return response.data
  }

  async listCages(context?: AnimalAccessContext): Promise<Cage[]> {
    const projectId = activeProjectId(context?.projectId)
    const query = projectId
      ? `?project_id=${encodeURIComponent(projectId)}`
      : ''
    const [cages, animals] = await Promise.all([
      this.request<ApiCollection<RawCage>>(`/cages${query}`),
      this.request<ApiCollection<RawAnimal>>(`/animals${query}`),
    ])
    return mapCages(cages.data, animals.data)
  }

  async listProjectAnimalAssignments(projectId: string): Promise<ProjectAnimalAssignment[]> {
    const response = await this.request<ApiCollection<RawProjectAnimalAssignment>>(
      `/projects/${encodeURIComponent(projectId)}/animal-assignments`,
    )
    return response.data.map(mapProjectAnimalAssignment)
  }

  async assignAnimalsToProject(
    projectId: string,
    animalIds: string[],
    reason?: string,
  ): Promise<ProjectAnimalAssignment[]> {
    const response = await this.request<ApiItem<RawProjectAnimalAssignment[]>>(
      `/projects/${encodeURIComponent(projectId)}/animal-assignments`,
      {
        method: 'POST',
        body: JSON.stringify({ animal_ids: animalIds, reason: reason || null }),
      },
    )
    return response.data.map(mapProjectAnimalAssignment)
  }

  async removeAnimalsFromProject(
    projectId: string,
    assignments: Array<{ assignmentId: string; expectedRevision: number }>,
  ): Promise<ProjectAnimalAssignment[]> {
    const response = await this.request<ApiItem<RawProjectAnimalAssignment[]>>(
      `/projects/${encodeURIComponent(projectId)}/animal-assignments`,
      {
        method: 'DELETE',
        body: JSON.stringify({
          assignments: assignments.map((assignment) => ({
            assignment_id: assignment.assignmentId,
            expected_revision: assignment.expectedRevision,
          })),
        }),
      },
    )
    return response.data.map(mapProjectAnimalAssignment)
  }

  async createCage(input: CreateCageInput): Promise<Cage> {
    const response = await this.request<ApiItem<RawCage>>('/cages', {
      method: 'POST',
      body: JSON.stringify({
        section: input.room,
        display_id: input.code,
        location: input.rack,
        capacity: input.capacity,
      }),
    })
    return mapCages([response.data], [])[0]
  }

  async createAnimal(input: CreateAnimalInput): Promise<Animal> {
    if (input.cageId) {
      throw new Error('共享版请先登记动物，再通过转笼操作分配笼位')
    }
    const response = await this.request<ApiItem<RawAnimal>>('/animals', {
      method: 'POST',
      body: JSON.stringify({
        display_id: input.displayId,
        sex: input.sex,
        project_id: input.identifierScope === 'project'
          ? activeProjectId(input.projectId)
          : null,
        strain: input.strain || null,
        birth_date: input.birthDate || null,
        legacy_id: null,
        initial_genotyping_records: (input.initialGenotypingRecords ?? []).map((record) => ({
          genotype_definition_id: record.genotypeDefinitionId,
          state: record.state,
          assessed_at: record.assessedAt ?? null,
          method: record.method ?? null,
          notes: record.notes ?? null,
        })),
      }),
    })
    return mapAnimal(response.data)
  }

  async listAnimals(context?: AnimalAccessContext): Promise<Animal[]> {
    const rows: Animal[] = []
    const pageSize = 500
    for (let page = 0; page < 20; page += 1) {
      const query = new URLSearchParams({
        limit: String(pageSize),
        offset: String(page * pageSize),
      })
      const projectId = activeProjectId(context?.projectId)
      if (projectId) query.set('project_id', projectId)
      const response = await this.request<ApiCollection<RawAnimalOverview>>(
        `/animal-overviews?${query}`,
      )
      rows.push(...response.data.map(mapAnimalOverview))
      if (response.data.length < pageSize) return rows
    }
    throw new Error('动物列表超过 10000 条，请按项目或条件缩小范围')
  }

  async getAnimal(id: string, context?: AnimalAccessContext): Promise<Animal | undefined> {
    try {
      const pathId = encodeURIComponent(id)
      const projectId = activeProjectId(context?.projectId)
      const query = projectId
        ? `?project_id=${encodeURIComponent(projectId)}`
        : ''
      const cagesRequest = !currentAuthSession.value || hasLabRegistryAccess() || projectId
        ? this.request<ApiCollection<RawCage>>(`/cages${query}`)
        : Promise.resolve({ data: [], count: 0, request_id: '' })
      const [response, events, cages] = await Promise.all([
        this.request<ApiItem<RawAnimal>>(`/animals/${pathId}${query}`),
        this.request<ApiCollection<RawAnimalEvent>>(`/animals/${pathId}/events${query}`),
        cagesRequest,
      ])
      const overviewQuery = new URLSearchParams({ q: response.data.display_id, limit: '500' })
      if (projectId) overviewQuery.set('project_id', projectId)
      const overviews = await this.request<ApiCollection<RawAnimalOverview>>(
        `/animal-overviews?${overviewQuery}`,
      )
      const overview = overviews.data.find((candidate) => candidate.animal.id === id)
      const animal = overview ? mapAnimalOverview(overview) : mapAnimal(response.data)
      const cageNames = new Map(cages.data.map((cage) => [cage.id, cage.display_id]))
      animal.timeline = events.data.slice().reverse().map((event) => mapTimelineEvent(event, cageNames))
      return animal
    } catch (error) {
      if (error instanceof HttpGatewayError && error.status === 404) return undefined
      throw error
    }
  }

  async getAnimalDetail(id: string, context?: AnimalAccessContext): Promise<AnimalDetail> {
    const query = new URLSearchParams({ limit: '500' })
    const projectId = activeProjectId(context?.projectId)
    if (projectId) query.set('project_id', projectId)
    const cagesRequest = !currentAuthSession.value || hasLabRegistryAccess()
      ? this.request<ApiCollection<RawCage>>('/cages')
      : Promise.resolve({ data: [], count: 0, request_id: '' })
    const [response, cages] = await Promise.all([
      this.request<ApiItem<RawAnimalDetail>>(
        `/animals/${encodeURIComponent(id)}/detail?${query}`,
      ),
      cagesRequest,
    ])
    return mapAnimalDetail(
      response.data,
      new Map(cages.data.map((cage) => [cage.id, cage.display_id])),
    )
  }

  async listGeneLoci(projectId?: string, includeArchived = false): Promise<GeneLocus[]> {
    const query = new URLSearchParams({ limit: '500' })
    const scope = activeProjectId(projectId)
    if (scope) query.set('project_id', scope)
    if (includeArchived) query.set('include_archived', 'true')
    const response = await this.request<ApiCollection<RawGeneLocus>>(`/gene-loci?${query}`)
    return response.data.map(mapGeneLocus)
  }

  async geneLocusReferences(id: string, projectId?: string): Promise<GeneticsReferenceCounts> {
    const query = new URLSearchParams()
    const scope = activeProjectId(projectId)
    if (scope) query.set('project_id', scope)
    const response = await this.request<ApiItem<RawGeneticsReferenceCounts>>(
      `/gene-loci/${encodeURIComponent(id)}/references?${query}`,
    )
    return mapGeneticsReferenceCounts(response.data)
  }

  async archiveGeneLocus(input: GeneticsArchiveInput): Promise<GeneLocus> {
    return this.setGeneLocusArchived(input, 'archive')
  }

  async restoreGeneLocus(input: GeneticsArchiveInput): Promise<GeneLocus> {
    return this.setGeneLocusArchived(input, 'restore')
  }

  private async setGeneLocusArchived(
    input: GeneticsArchiveInput,
    action: 'archive' | 'restore',
  ): Promise<GeneLocus> {
    const response = await this.request<ApiItem<RawGeneLocus>>(
      `/gene-loci/${encodeURIComponent(input.id)}/${action}`,
      {
        method: 'POST',
        body: JSON.stringify({
          project_id: activeProjectId(input.projectId) ?? null,
          expected_revision: input.expectedRevision,
        }),
      },
    )
    return mapGeneLocus(response.data)
  }

  async createGeneLocus(input: CreateGeneLocusInput): Promise<GeneLocus> {
    const response = await this.request<ApiItem<RawGeneLocus>>('/gene-loci', {
      method: 'POST',
      body: JSON.stringify({
        project_id: activeProjectId(input.projectId) ?? null,
        symbol: input.symbol,
        description: input.description ?? null,
      }),
    })
    return mapGeneLocus(response.data)
  }

  async listAlleles(
    locusId: string,
    projectId?: string,
    includeArchived = false,
  ): Promise<GeneAllele[]> {
    const query = new URLSearchParams({ locus_id: locusId, limit: '500' })
    const scope = activeProjectId(projectId)
    if (scope) query.set('project_id', scope)
    if (includeArchived) query.set('include_archived', 'true')
    const response = await this.request<ApiCollection<RawAllele>>(`/alleles?${query}`)
    return response.data.map(mapAllele)
  }

  async alleleReferences(id: string, projectId?: string): Promise<GeneticsReferenceCounts> {
    const query = new URLSearchParams()
    const scope = activeProjectId(projectId)
    if (scope) query.set('project_id', scope)
    const response = await this.request<ApiItem<RawGeneticsReferenceCounts>>(
      `/alleles/${encodeURIComponent(id)}/references?${query}`,
    )
    return mapGeneticsReferenceCounts(response.data)
  }

  async archiveAllele(input: GeneticsArchiveInput): Promise<GeneAllele> {
    return this.setAlleleArchived(input, 'archive')
  }

  async restoreAllele(input: GeneticsArchiveInput): Promise<GeneAllele> {
    return this.setAlleleArchived(input, 'restore')
  }

  private async setAlleleArchived(
    input: GeneticsArchiveInput,
    action: 'archive' | 'restore',
  ): Promise<GeneAllele> {
    const response = await this.request<ApiItem<RawAllele>>(
      `/alleles/${encodeURIComponent(input.id)}/${action}`,
      {
        method: 'POST',
        body: JSON.stringify({
          project_id: activeProjectId(input.projectId) ?? null,
          expected_revision: input.expectedRevision,
        }),
      },
    )
    return mapAllele(response.data)
  }

  async createAllele(input: CreateAlleleInput): Promise<GeneAllele> {
    const response = await this.request<ApiItem<RawAllele>>('/alleles', {
      method: 'POST',
      body: JSON.stringify({
        project_id: activeProjectId(input.projectId) ?? null,
        locus_id: input.locusId,
        symbol: input.symbol,
        description: input.description ?? null,
        is_wild_type: input.isWildType,
      }),
    })
    return mapAllele(response.data)
  }

  async listGenotypes(animalId: string, projectId?: string): Promise<AnimalGenotype[]> {
    const query = new URLSearchParams({ animal_id: animalId, limit: '500' })
    const scope = activeProjectId(projectId)
    if (scope) query.set('project_id', scope)
    const response = await this.request<ApiCollection<RawGenotype>>(`/genotypes?${query}`)
    return response.data.map(mapGenotype)
  }

  async createGenotype(input: CreateGenotypeInput): Promise<AnimalGenotype> {
    const response = await this.request<ApiItem<RawGenotype>>('/genotypes', {
      method: 'POST',
      body: JSON.stringify({
        project_id: activeProjectId(input.projectId) ?? null,
        animal_id: input.animalId,
        locus_id: input.locusId,
        allele_1_id: input.allele1Id ?? null,
        allele_2_id: input.allele2Id ?? null,
        assessed_at: input.assessedAt ?? null,
      }),
    })
    return mapGenotype(response.data)
  }

  async listGenotypeDefinitions(
    projectId?: string,
    includeArchived = false,
  ): Promise<GenotypeDefinition[]> {
    const query = new URLSearchParams({ limit: '500' })
    const scope = activeProjectId(projectId)
    if (scope) query.set('project_id', scope)
    if (includeArchived) query.set('include_archived', 'true')
    const response = await this.request<ApiCollection<RawGenotypeDefinition>>(
      `/genotype-definitions?${query}`,
    )
    return response.data.map(mapGenotypeDefinition)
  }

  async genotypeDefinitionReferences(
    id: string,
    projectId?: string,
  ): Promise<GeneticsReferenceCounts> {
    const query = new URLSearchParams()
    const scope = activeProjectId(projectId)
    if (scope) query.set('project_id', scope)
    const response = await this.request<ApiItem<RawGeneticsReferenceCounts>>(
      `/genotype-definitions/${encodeURIComponent(id)}/references?${query}`,
    )
    return mapGeneticsReferenceCounts(response.data)
  }

  async archiveGenotypeDefinition(input: GeneticsArchiveInput): Promise<GenotypeDefinition> {
    return this.setGenotypeDefinitionArchived(input, 'archive')
  }

  async restoreGenotypeDefinition(input: GeneticsArchiveInput): Promise<GenotypeDefinition> {
    return this.setGenotypeDefinitionArchived(input, 'restore')
  }

  private async setGenotypeDefinitionArchived(
    input: GeneticsArchiveInput,
    action: 'archive' | 'restore',
  ): Promise<GenotypeDefinition> {
    const response = await this.request<ApiItem<RawGenotypeDefinition>>(
      `/genotype-definitions/${encodeURIComponent(input.id)}/${action}`,
      {
        method: 'POST',
        body: JSON.stringify({
          project_id: activeProjectId(input.projectId) ?? null,
          expected_revision: input.expectedRevision,
        }),
      },
    )
    return mapGenotypeDefinition(response.data)
  }

  async createGenotypeDefinition(
    input: CreateGenotypeDefinitionInput,
  ): Promise<GenotypeDefinition> {
    const response = await this.request<ApiItem<RawGenotypeDefinition>>('/genotype-definitions', {
      method: 'POST',
      body: JSON.stringify({
        project_id: activeProjectId(input.projectId) ?? null,
        name: input.name,
        description: input.description ?? null,
        components: input.components.map((component) => ({
          locus_id: component.locusId,
          allele_1_id: component.allele1Id,
          allele_2_id: component.allele2Id ?? null,
          mode: component.mode,
          display_order: component.displayOrder,
        })),
      }),
    })
    return mapGenotypeDefinition(response.data)
  }

  async listGenotypingRecords(
    animalId: string,
    projectId?: string,
  ): Promise<GenotypingRecord[]> {
    const query = new URLSearchParams({ animal_id: animalId, limit: '500' })
    const scope = activeProjectId(projectId)
    if (scope) query.set('project_id', scope)
    const response = await this.request<ApiCollection<RawGenotypingRecord>>(
      `/genotyping-records?${query}`,
    )
    return response.data.map(mapGenotypingRecord)
  }

  async createGenotypingRecord(input: CreateGenotypingRecordInput): Promise<GenotypingRecord> {
    const response = await this.request<ApiItem<RawGenotypingRecord>>('/genotyping-records', {
      method: 'POST',
      body: JSON.stringify({
        project_id: activeProjectId(input.projectId) ?? null,
        animal_id: input.animalId,
        genotype_definition_id: input.genotypeDefinitionId,
        state: input.state,
        assessed_at: input.assessedAt ?? null,
        method: input.method ?? null,
        notes: input.notes ?? null,
      }),
    })
    return mapGenotypingRecord(response.data)
  }

  async voidGenotypingRecord(input: VoidGenotypingRecordInput): Promise<GenotypingRecord> {
    const response = await this.request<ApiItem<RawGenotypingRecord>>(
      `/genotyping-records/${encodeURIComponent(input.recordId)}/void`,
      {
        method: 'POST',
        body: JSON.stringify({
          project_id: activeProjectId(input.projectId) ?? null,
          expected_revision: input.expectedRevision,
          reason: input.reason,
        }),
      },
    )
    return mapGenotypingRecord(response.data)
  }

  async correctGenotypingRecord(
    input: CorrectGenotypingRecordInput,
  ): Promise<CorrectGenotypingRecordResult> {
    const response = await this.request<ApiItem<RawCorrectGenotypingRecordResult>>(
      `/genotyping-records/${encodeURIComponent(input.recordId)}/correct`,
      {
        method: 'POST',
        body: JSON.stringify({
          project_id: activeProjectId(input.projectId) ?? null,
          expected_revision: input.expectedRevision,
          reason: input.reason,
          genotype_definition_id: input.genotypeDefinitionId,
          state: input.state,
          assessed_at: input.assessedAt ?? null,
          method: input.method ?? null,
          notes: input.notes ?? null,
        }),
      },
    )
    return {
      voided: mapGenotypingRecord(response.data.voided),
      replacement: mapGenotypingRecord(response.data.replacement),
    }
  }

  async listBreedingLines(): Promise<BreedingLine[]> {
    const response = await this.request<ApiCollection<RawBreedingLine>>('/breeding-lines?limit=500')
    return response.data.map(mapBreedingLine)
  }

  async createBreedingLine(input: CreateBreedingLineInput): Promise<BreedingLine> {
    const response = await this.request<ApiItem<RawBreedingLine>>('/breeding-lines', {
      method: 'POST',
      body: JSON.stringify({
        name: input.name,
        description: input.description ?? null,
        genotype_definition_ids: input.genotypeDefinitionIds,
      }),
    })
    return mapBreedingLine(response.data)
  }

  async listColonies(breedingLineId?: string): Promise<Colony[]> {
    const query = new URLSearchParams({ limit: '500' })
    if (breedingLineId) query.set('breeding_line_id', breedingLineId)
    const response = await this.request<ApiCollection<RawColony>>(`/colonies?${query}`)
    return response.data.map(mapColony)
  }

  async createColony(input: CreateColonyInput): Promise<Colony> {
    const response = await this.request<ApiItem<RawColony>>('/colonies', {
      method: 'POST',
      body: JSON.stringify({
        breeding_line_id: input.breedingLineId,
        name: input.name,
        description: input.description ?? null,
      }),
    })
    return mapColony(response.data)
  }

  async listBreedingPairs(colonyId?: string): Promise<BreedingPair[]> {
    const query = new URLSearchParams({ limit: '500' })
    if (colonyId) query.set('colony_id', colonyId)
    const response = await this.request<ApiCollection<RawBreedingPair>>(`/breeding-pairs?${query}`)
    return response.data.map(mapBreedingPair)
  }

  async createBreedingPair(input: CreateBreedingPairInput): Promise<BreedingPair> {
    const response = await this.request<ApiItem<RawBreedingPair>>('/breeding-pairs', {
      method: 'POST',
      body: JSON.stringify({
        project_id: activeProjectId(input.projectId) ?? null,
        colony_id: input.colonyId,
        name: input.name,
        male_animal_id: input.maleAnimalId,
        female_animal_ids: input.femaleAnimalIds,
        started_at: input.startedAt ?? null,
      }),
    })
    return mapBreedingPair(response.data)
  }

  async retireBreedingPair(input: RetireBreedingPairInput): Promise<BreedingPair> {
    const response = await this.request<ApiItem<RawBreedingPair>>(
      `/breeding-pairs/${encodeURIComponent(input.id)}/retire`,
      {
        method: 'POST',
        body: JSON.stringify({
          expected_revision: input.expectedRevision,
          ended_at: input.endedAt ?? null,
        }),
      },
    )
    return mapBreedingPair(response.data)
  }

  async listMatingEvents(breedingPairId: string): Promise<MatingEvent[]> {
    const query = new URLSearchParams({ breeding_pair_id: breedingPairId, limit: '500' })
    const response = await this.request<ApiCollection<RawMatingEvent>>(`/mating-events?${query}`)
    return response.data.map(mapMatingEvent)
  }

  async createMatingEvent(input: CreateMatingEventInput): Promise<MatingEvent> {
    const response = await this.request<ApiItem<RawMatingEvent>>('/mating-events', {
      method: 'POST',
      body: JSON.stringify({
        project_id: activeProjectId(input.projectId) ?? null,
        breeding_pair_id: input.breedingPairId,
        male_animal_id: input.maleAnimalId,
        female_animal_id: input.femaleAnimalId,
        occurred_at: input.occurredAt ?? null,
        notes: input.notes ?? null,
      }),
    })
    return mapMatingEvent(response.data)
  }

  async listLitters(breedingPairId: string): Promise<Litter[]> {
    const query = new URLSearchParams({ breeding_pair_id: breedingPairId, limit: '500' })
    const response = await this.request<ApiCollection<RawLitter>>(`/litters?${query}`)
    return response.data.map(mapLitter)
  }

  async createLitter(input: CreateLitterInput): Promise<CreatedLitter> {
    const response = await this.request<ApiItem<RawCreatedLitter>>('/litters', {
      method: 'POST',
      body: JSON.stringify({
        mating_event_id: input.matingEventId,
        born_on: input.bornOn,
        size_total: input.sizeTotal,
        drafts: input.drafts.map((draft) => ({
          temporary_label: draft.temporaryLabel,
          sex: draft.sex,
        })),
        notes: input.notes ?? null,
      }),
    })
    return {
      litter: mapLitter(response.data.litter),
      drafts: response.data.drafts.map(mapAnimalDraft),
    }
  }

  async listAnimalDrafts(litterId: string): Promise<AnimalDraft[]> {
    const query = new URLSearchParams({ litter_id: litterId, limit: '500' })
    const response = await this.request<ApiCollection<RawAnimalDraft>>(`/animal-drafts?${query}`)
    return response.data.map(mapAnimalDraft)
  }

  async registerAnimalDraft(input: RegisterAnimalDraftInput): Promise<RegisteredAnimalDraft> {
    const response = await this.request<ApiItem<RawRegisteredAnimalDraft>>(
      `/animal-drafts/${encodeURIComponent(input.draftId)}/register`,
      {
        method: 'POST',
        body: JSON.stringify({
          expected_revision: input.expectedRevision,
          identifier_scope: input.identifierScope,
          project_id: activeProjectId(input.projectId) ?? null,
          display_id: input.displayId,
          strain: input.strain ?? null,
          initial_cage_id: input.initialCageId ?? null,
        }),
      },
    )
    return {
      draft: mapAnimalDraft(response.data.draft),
      animal: mapAnimal(response.data.animal),
    }
  }

  async predictBreeding(input: BreedingPredictionInput): Promise<LocusPrediction[]> {
    const response = await this.request<ApiItem<RawLocusPrediction[]>>('/breeding-predictions', {
      method: 'POST',
      body: JSON.stringify({
        male_genotype_definition_id: input.maleGenotypeDefinitionId,
        female_genotype_definition_id: input.femaleGenotypeDefinitionId,
      }),
    })
    return response.data.map(mapLocusPrediction)
  }

  async createAnimalSample(input: CreateAnimalSampleInput): Promise<AnimalSample> {
    const response = await this.request<ApiItem<RawDetailSample & { meta: { revision: number } }>>(
      '/samples',
      {
        method: 'POST',
        body: JSON.stringify({
          project_id: activeProjectId(input.projectId),
          experiment_id: input.experimentId ?? null,
          animal_id: input.animalId,
          sample_type: input.sampleType,
          quantity: input.quantity ?? null,
          unit: input.unit ?? null,
          location: input.location ?? null,
          collected_at: input.collectedAt ?? null,
        }),
      },
    )
    return mapDetailSample({ ...response.data, revision: response.data.meta.revision })
  }

  async createPedigree(input: CreatePedigreeInput): Promise<PedigreeRelation> {
    const projectId = activeProjectId(input.projectId)
    const response = await this.request<ApiItem<{
      id: string
      animal_id: string
      parent_id: string
      parent_type: CreatePedigreeInput['parentType']
      meta: { revision: number }
    }>>('/pedigrees', {
      method: 'POST',
      body: JSON.stringify({
        project_id: projectId ?? null,
        animal_id: input.animalId,
        parent_id: input.parentId,
        parent_type: input.parentType,
      }),
    })
    const detail = await this.getAnimalDetail(
      input.animalId,
      projectId ? { projectId } : undefined,
    )
    const relation = detail.pedigree.find((item) => item.id === response.data.id)
    if (!relation) throw new Error('谱系关系已写入，但刷新详情失败')
    return relation
  }

  async moveAnimals(animalIds: string[], targetCageId: string): Promise<void> {
    await this.request<ApiItem<unknown>>('/animals/transfer', {
      method: 'POST',
      body: JSON.stringify({ animal_ids: animalIds, target_cage_id: targetCageId }),
    })
  }

  async createProject(input: CreateProjectInput): Promise<ProjectSummary> {
    const response = await this.request<ApiItem<RawProject>>('/projects', {
      method: 'POST',
      body: JSON.stringify({ name: input.name, description: input.description || null }),
    })
    return { id: response.data.id, name: response.data.name }
  }

  async listProjects(): Promise<ProjectSummary[]> {
    const response = await this.request<ApiCollection<RawProject>>('/projects')
    return response.data.map(({ id, name }) => ({ id, name }))
  }

  async listPublishedTemplates(): Promise<ExperimentTemplateSummary[]> {
    const projectId = activeProjectId()
    const suffix = projectId ? `?project_id=${encodeURIComponent(projectId)}` : ''
    const response = await this.request<ApiCollection<RawTemplate>>(
      `/experiment-template-versions${suffix}`,
    )
    return response.data
      .filter((template) => template.status === 'published')
      .map(mapTemplate)
  }

  async createPublishedTemplate(
    input: CreatePublishedTemplateInput,
  ): Promise<ExperimentTemplateSummary> {
    const key = `web.${crypto.randomUUID()}`
    const projectId = activeProjectId()
    const draft = await this.request<ApiItem<RawTemplate>>('/experiment-template-versions', {
      method: 'POST',
      body: JSON.stringify({
        project_id: projectId ?? null,
        template_key: key,
        version: 1,
        name: input.name,
        description: input.description || null,
        fields: [{
          key: input.fieldKey,
          label: input.fieldLabel,
          value_type: input.fieldValueType,
          unit: input.fieldValueType === 'number' && input.fieldUnit ? input.fieldUnit : null,
          required: false,
          categories: [],
          minimum: null,
          maximum: null,
          display_order: 0,
          ai_writable: false,
        }],
      }),
    })
    const published = await this.request<ApiItem<RawTemplate>>(
      `/experiment-template-versions/${encodeURIComponent(draft.data.id)}/publish`,
      {
        method: 'POST',
        body: JSON.stringify({
          project_id: projectId ?? null,
          expected_revision: draft.data.meta.revision,
        }),
      },
    )
    return mapTemplate(published.data)
  }

  async createExperiment(input: CreateExperimentInput): Promise<Experiment> {
    const [project, response] = await Promise.all([
      this.request<ApiItem<RawProject>>(`/projects/${encodeURIComponent(input.projectId)}`),
      this.request<ApiItem<RawExperiment>>('/experiments', {
        method: 'POST',
        body: JSON.stringify({
          project_id: input.projectId,
          name: input.name,
          description: input.description || null,
          template_version_id: input.templateVersionId,
        }),
      }),
    ])
    return mapExperiment(response.data, project.data.name)
  }

  async completeExperiment(id: string, expectedRevision: number): Promise<Experiment> {
    const current = (await this.request<ApiItem<RawExperiment>>(
      `/experiments/${encodeURIComponent(id)}`,
    )).data
    const project = (await this.request<ApiItem<RawProject>>(
      `/projects/${encodeURIComponent(current.project_id)}`,
    )).data
    const response = await this.request<ApiItem<RawExperiment>>(
      `/experiments/${encodeURIComponent(id)}/complete`,
      { method: 'POST', body: JSON.stringify({ expected_revision: expectedRevision }) },
    )
    return mapExperiment(response.data, project.name)
  }

  async cancelExperiment(id: string, expectedRevision: number): Promise<Experiment> {
    const current = (await this.request<ApiItem<RawExperiment>>(
      `/experiments/${encodeURIComponent(id)}`,
    )).data
    const project = (await this.request<ApiItem<RawProject>>(
      `/projects/${encodeURIComponent(current.project_id)}`,
    )).data
    const response = await this.request<ApiItem<RawExperiment>>(
      `/experiments/${encodeURIComponent(id)}/cancel`,
      { method: 'POST', body: JSON.stringify({ expected_revision: expectedRevision }) },
    )
    return mapExperiment(response.data, project.name)
  }

  async listExperiments(): Promise<Experiment[]> {
    const selectedProjectId = activeProjectId()
    const projects = await this.request<ApiCollection<RawProject>>('/projects')
    const visibleProjects = selectedProjectId
      ? projects.data.filter((project) => project.id === selectedProjectId)
      : projects.data
    const rows = await Promise.all(visibleProjects.map(async (project) => ({
      project,
      experiments: (await this.request<ApiCollection<RawExperiment>>(`/experiments?project_id=${encodeURIComponent(project.id)}`)).data,
    })))
    return rows.flatMap(({ project, experiments }) => experiments.map((experiment) => mapExperiment(experiment, project.name)))
  }

  async listCohorts(experimentId: string): Promise<Cohort[]> {
    const response = await this.request<ApiCollection<RawCohort>>(
      `/cohorts?experiment_id=${encodeURIComponent(experimentId)}`,
    )
    return response.data.map(mapCohort)
  }

  async createCohort(input: CreateCohortInput): Promise<Cohort> {
    const response = await this.request<ApiItem<RawCohort>>('/cohorts', {
      method: 'POST',
      body: JSON.stringify({
        experiment_id: input.experimentId,
        name: input.name,
        description: input.description || null,
      }),
    })
    return mapCohort(response.data)
  }

  async listParticipations(projectId: string, experimentId: string): Promise<Participation[]> {
    const query = new URLSearchParams({ project_id: projectId, experiment_id: experimentId })
    const response = await this.request<ApiCollection<RawParticipation>>(`/participations?${query}`)
    return response.data.map(mapParticipation)
  }

  async enrollAnimal(input: EnrollAnimalInput): Promise<Participation> {
    const response = await this.request<ApiItem<RawParticipation>>('/participations', {
      method: 'POST',
      body: JSON.stringify({
        experiment_id: input.experimentId,
        animal_id: input.animalId,
        cohort_id: input.cohortId || null,
      }),
    })
    return mapParticipation(response.data)
  }

  async completeParticipation(
    id: string,
    expectedRevision: number,
  ): Promise<Participation> {
    const response = await this.request<ApiItem<RawParticipation>>(
      `/participations/${encodeURIComponent(id)}/complete`,
      { method: 'POST', body: JSON.stringify({ expected_revision: expectedRevision }) },
    )
    return mapParticipation(response.data)
  }

  async withdrawParticipation(
    id: string,
    expectedRevision: number,
  ): Promise<Participation> {
    const response = await this.request<ApiItem<RawParticipation>>(
      `/participations/${encodeURIComponent(id)}/withdraw`,
      { method: 'POST', body: JSON.stringify({ expected_revision: expectedRevision }) },
    )
    return mapParticipation(response.data)
  }

  async listProcedures(experimentId: string): Promise<Procedure[]> {
    const response = await this.request<ApiCollection<RawProcedure>>(
      `/procedures?experiment_id=${encodeURIComponent(experimentId)}`,
    )
    return response.data.map(mapProcedure)
  }

  async createProcedure(input: CreateProcedureInput): Promise<Procedure> {
    const response = await this.request<ApiItem<RawProcedure>>('/procedures', {
      method: 'POST',
      body: JSON.stringify({
        experiment_id: input.experimentId,
        animal_id: input.animalId || null,
        name: input.name,
        scheduled_at: input.scheduledAt || null,
        performed_at: input.performedAt || null,
        status: input.status,
        details: input.details ?? {},
      }),
    })
    return mapProcedure(response.data)
  }

  async listExperimentEvents(experimentId: string): Promise<ExperimentEvent[]> {
    const query = new URLSearchParams({ experiment_id: experimentId, limit: '500' })
    const response = await this.request<ApiCollection<RawExperimentEvent>>(
      `/experiment-events?${query}`,
    )
    return response.data.map(mapExperimentEvent)
  }

  async createExperimentEvent(input: CreateExperimentEventInput): Promise<ExperimentEvent> {
    const response = await this.request<ApiItem<RawExperimentEvent>>('/experiment-events', {
      method: 'POST',
      body: JSON.stringify({
        experiment_id: input.experimentId,
        event_key: input.eventKey,
        label: input.label,
        occurred_at: input.occurredAt ?? null,
        details: input.details ?? {},
      }),
    })
    return mapExperimentEvent(response.data)
  }

  async listObservationDefinitions(experimentId: string): Promise<ObservationDefinition[]> {
    const query = new URLSearchParams({ experiment_id: experimentId, limit: '500' })
    const response = await this.request<ApiCollection<RawObservationDefinition>>(
      `/observation-definitions?${query}`,
    )
    return response.data.map(mapObservationDefinition)
  }

  async createObservationDefinition(
    input: CreateObservationDefinitionInput,
  ): Promise<ObservationDefinition> {
    const response = await this.request<ApiItem<RawObservationDefinition>>(
      '/observation-definitions',
      {
        method: 'POST',
        body: JSON.stringify({
          experiment_id: input.experimentId,
          key: input.key,
          label: input.label,
          value_type: input.valueType,
          unit: input.unit ?? null,
          categories: input.categories ?? [],
          policy: input.policy,
        }),
      },
    )
    return mapObservationDefinition(response.data)
  }

  async listObservations(filter: ObservationFilter): Promise<Observation[]> {
    const query = new URLSearchParams({ experiment_id: filter.experimentId, limit: '500' })
    if (filter.experimentEventId) query.set('experiment_event_id', filter.experimentEventId)
    if (filter.subjectType) query.set('subject_type', filter.subjectType)
    if (filter.subjectId) query.set('subject_id', filter.subjectId)
    const response = await this.request<ApiCollection<RawObservation>>(`/observations?${query}`)
    return response.data.map(mapObservation)
  }

  async recordObservation(input: RecordObservationInput): Promise<RecordedObservation> {
    const response = await this.request<ApiItem<RawRecordedObservation>>('/observations', {
      method: 'POST',
      body: JSON.stringify({
        experiment_id: input.experimentId,
        experiment_event_id: input.experimentEventId,
        definition_id: input.definitionId,
        subject_type: input.subjectType,
        subject_id: input.subjectId,
        context: input.context ?? {},
        value: input.value,
        recorded_at: input.recordedAt ?? null,
        notes: input.notes ?? null,
      }),
    })
    return mapRecordedObservation(response.data)
  }

  async listObservationValues(observationId: string): Promise<ObservationValueRecord[]> {
    const response = await this.request<ApiCollection<RawObservationValueRecord>>(
      `/observations/${encodeURIComponent(observationId)}/values`,
    )
    return response.data.map(mapObservationValueRecord)
  }

  async reviseObservation(input: ReviseObservationInput): Promise<RecordedObservation> {
    const response = await this.request<ApiItem<RawRecordedObservation>>(
      `/observations/${encodeURIComponent(input.observationId)}/revisions`,
      {
        method: 'POST',
        body: JSON.stringify({
          expected_revision: input.expectedRevision,
          value: input.value,
          recorded_at: input.recordedAt ?? null,
          notes: input.notes ?? null,
        }),
      },
    )
    return mapRecordedObservation(response.data)
  }

  async listDataJobs(): Promise<DataJob[]> {
    const response = await this.request<ApiCollection<RawJob>>('/jobs')
    return response.data.map(mapJob)
  }

  async listAttachments(scope: AttachmentScope): Promise<AttachmentMetadata[]> {
    const query = attachmentScopeQuery({
      ...scope,
      projectId: activeProjectId(scope.projectId),
    })
    const response = await this.request<ApiCollection<RawAttachment>>(`/attachments?${query}`)
    return response.data.map(mapAttachment)
  }

  async uploadAttachment(input: UploadAttachmentInput): Promise<AttachmentMetadata> {
    const query = attachmentScopeQuery({
      ...input,
      projectId: activeProjectId(input.projectId),
    })
    query.set('file_name', input.fileName)
    if (input.mediaType) query.set('media_type', input.mediaType)
    const response = await this.request<ApiItem<RawAttachment>>(
      `/attachments/upload?${query}`,
      { method: 'POST', body: input.content },
      { contentType: input.mediaType || input.content.type || 'application/octet-stream' },
    )
    return mapAttachment(response.data)
  }

  async downloadAttachment(id: string): Promise<Blob> {
    const response = await this.send(
      `/attachments/${encodeURIComponent(id)}/content`,
      {},
      { accept: 'application/octet-stream' },
    )
    return response.blob()
  }

  async deleteAttachment(input: DeleteAttachmentInput): Promise<AttachmentMetadata> {
    const response = await this.request<ApiItem<RawAttachment>>(
      `/attachments/${encodeURIComponent(input.id)}`,
      {
        method: 'DELETE',
        body: JSON.stringify({
          expected_revision: input.expectedRevision,
          reason: input.reason ?? null,
        }),
      },
    )
    return mapAttachment(response.data)
  }

  async listOperations(query = new URLSearchParams()): Promise<OperationRecord[]> {
    const scopedQuery = new URLSearchParams(query)
    const projectId = activeProjectId()
    if (projectId && !scopedQuery.has('project_id')) scopedQuery.set('project_id', projectId)
    if (!scopedQuery.has('limit')) scopedQuery.set('limit', '200')
    const response = await this.request<ApiCollection<OperationRecord>>('/operations?' + scopedQuery.toString())
    return response.data
  }

  async listLibrary(projectId: string, experimentId?: string): Promise<LibraryRecord[]> {
    const query = new URLSearchParams({ project_id: projectId, limit: '500' })
    if (experimentId) query.set('experiment_id', experimentId)
    const response = await this.request<ApiCollection<{
      attachment: { id: string; project_id?: string | null; entity_type: AttachmentTarget; entity_id: string; file_name: string; media_type?: string | null; size_bytes: number; sha256: string; version: number; meta: { created_at: string; revision?: number } }
      links: AttachmentLinkRecord[]
      derivatives: Array<Record<string, unknown>>
      previewSupported: boolean
      previewHref?: string
      previewReason?: string
      status: string
    }>>('/library?' + query.toString())
    return response.data.map((entry) => ({
      attachment: {
        id: entry.attachment.id, projectId: entry.attachment.project_id ?? undefined,
        entityType: entry.attachment.entity_type, entityId: entry.attachment.entity_id,
        fileName: entry.attachment.file_name, mediaType: entry.attachment.media_type ?? undefined,
        sizeBytes: entry.attachment.size_bytes, sha256: entry.attachment.sha256,
        version: entry.attachment.version, revision: entry.attachment.meta.revision ?? entry.attachment.version,
        contentHref: '/api/v1/attachments/' + entry.attachment.id + '/content',
        previewSupported: entry.previewSupported, previewHref: entry.previewHref,
        previewReason: entry.previewReason, createdAt: entry.attachment.meta.created_at,
      },
      links: entry.links, derivatives: entry.derivatives, status: entry.status,
    }))
  }

  async listPrivateImages(): Promise<PrivateImageRecord[]> {
    return (await this.request<ApiCollection<PrivateImageRecord>>('/ai/images')).data
  }
  async uploadPrivateImage(file: File, conversationId?: string): Promise<PrivateImageRecord> {
    const query = new URLSearchParams({ file_name: file.name, media_type: file.type || 'application/octet-stream' })
    if (conversationId) query.set('conversation_id', conversationId)
    return (await this.request<ApiItem<PrivateImageRecord>>('/ai/images/upload?' + query.toString(),
      { method: 'POST', body: file }, { contentType: file.type || 'application/octet-stream' })).data
  }
  async archivePrivateImage(id: string, projectId: string, expectedRevision: number) {
    return (await this.request<ApiItem<PrivateImageRecord>>('/ai/images/' + encodeURIComponent(id) + '/archive',
      { method: 'POST', body: JSON.stringify({ project_id: projectId, expected_revision: expectedRevision }) })).data
  }
  async listAiExtractions(projectId?: string): Promise<AiExtractionRecord[]> {
    const suffix = projectId ? '?project_id=' + encodeURIComponent(projectId) : ''
    return (await this.request<ApiCollection<AiExtractionRecord>>('/ai/extractions' + suffix)).data
  }
  async createAiExtraction(input: { private_image_id: string; project_id: string; experiment_id: string; experiment_event_id: string }): Promise<AiExtractionRecord> {
    return (await this.request<ApiItem<AiExtractionRecord>>('/ai/extractions',
      { method: 'POST', body: JSON.stringify(input) })).data
  }
  async approveAiExtraction(id: string, expectedRevision: number, selectedIndexes: number[]) {
    const response = await this.request<ApiItem<{ draft: AiExtractionRecord }>>('/ai/extractions/' + encodeURIComponent(id) + '/approve',
      { method: 'POST', body: JSON.stringify({ expected_revision: expectedRevision, selected_indexes: selectedIndexes }) })
    return response.data.draft
  }
  async testAiSettings() {
    return (await this.request<ApiItem<{ ok: boolean; latencyMs: number; errorCode?: string }>>('/ai/settings/test', { method: 'POST' })).data
  }
  async getAiDiagnostics(): Promise<AiDiagnostics> {
    return (await this.request<ApiItem<AiDiagnostics>>('/ai/diagnostics')).data
  }
  async listAiProviderPresets(): Promise<AiProviderPreset[]> {
    return (await this.request<ApiCollection<AiProviderPreset>>('/ai/provider-presets')).data
  }
  async getAiLabSettings(): Promise<AiLabSettings> {
    return (await this.request<ApiItem<AiLabSettings>>('/admin/ai')).data
  }
  async saveAiLabSettings(input: { enabled: boolean; customUrlApprovalRequired: boolean; maxAutonomyMode: AiAutonomyMode }) {
    return (await this.request<ApiItem<AiLabSettings>>('/admin/ai',
      { method: 'PUT', body: JSON.stringify(input) })).data
  }
  async listAiProviderEndpoints(): Promise<AiProviderEndpoint[]> {
    return (await this.request<ApiCollection<AiProviderEndpoint>>('/admin/ai/endpoints')).data
  }
  async saveAiProviderEndpoint(input: SaveAiProviderEndpointInput, id?: string) {
    const path = id ? `/admin/ai/endpoints/${encodeURIComponent(id)}` : '/admin/ai/endpoints'
    return (await this.request<ApiItem<AiProviderEndpoint>>(path,
      { method: id ? 'PUT' : 'POST', body: JSON.stringify(input) })).data
  }
  async disableAiProviderEndpoint(id: string) {
    return (await this.request<ApiItem<AiProviderEndpoint>>(`/admin/ai/endpoints/${encodeURIComponent(id)}/disable`,
      { method: 'POST' })).data
  }

  async getAiSettings(): Promise<AiSettings> {
    const response = await this.request<ApiItem<AiSettings & { revision: number }>>('/ai/settings')
    return response.data
  }

  async saveAiSettings(input: SaveAiSettingsInput): Promise<AiSettings> {
    const response = await this.request<ApiItem<AiSettings & { revision: number }>>('/ai/settings', {
      method: 'PUT',
      body: JSON.stringify(input),
    })
    return response.data
  }

  async clearAiApiKey(): Promise<AiSettings> {
    const response = await this.request<ApiItem<AiSettings & { revision: number }>>('/ai/settings', {
      method: 'DELETE',
    })
    return response.data
  }

  async aiTurn(input: AiTurnInput): Promise<AiTurnResponse> {
    const response = await this.request<ApiItem<RawAiTurnResponse>>('/ai/turns', {
      method: 'POST',
      // AI workspace scope is explicit. An absent projectId means a
      // lab-registry, read-only conversation and must never inherit the
      // unrelated top-bar project context.
      body: JSON.stringify(input),
    })
    return mapAiTurn(response.data)
  }

  async createAiConversation(
    input: AiConversationCreateInput,
  ): Promise<AiConversationSummary> {
    const response = await this.request<ApiItem<RawAiConversationSummary>>(
      '/ai/conversations',
      {
        method: 'POST',
        body: JSON.stringify({
          project_id: input.projectId,
          title: input.title,
        }),
      },
    )
    return mapAiConversationSummary(response.data)
  }

  async listAiConversations(projectId?: string, limit = 50): Promise<AiConversationSummary[]> {
    const query = new URLSearchParams({ limit: String(limit) })
    if (projectId) query.set('project_id', projectId)
    const response = await this.request<ApiCollection<RawAiConversationSummary>>(
      `/ai/conversations?${query.toString()}`,
    )
    return response.data.map(mapAiConversationSummary)
  }

  async queryAiConversations(input: AiConversationListInput = {}): Promise<AiConversationSummary[]> {
    const query = new URLSearchParams({
      archive: input.archive ?? 'active',
      limit: String(input.limit ?? 100),
    })
    if (input.projectId) query.set('project_id', input.projectId)
    if (input.titleQuery?.trim()) query.set('q', input.titleQuery.trim())
    const response = await this.request<ApiCollection<RawAiConversationSummary>>(
      `/ai/conversations?${query.toString()}`,
    )
    return response.data.map(mapAiConversationSummary)
  }

  async getAiConversation(conversationId: string, limit = 200): Promise<AiConversationDetail> {
    const query = new URLSearchParams({ limit: String(limit) })
    const response = await this.request<ApiItem<RawAiConversationDetail>>(
      `/ai/conversations/${encodeURIComponent(conversationId)}?${query.toString()}`,
    )
    return mapAiConversationDetail(response.data)
  }

  async updateAiConversation(
    conversationId: string,
    input: AiConversationUpdateInput,
  ): Promise<AiConversationSummary> {
    const response = await this.request<ApiItem<RawAiConversationSummary>>(
      `/ai/conversations/${encodeURIComponent(conversationId)}`,
      {
        method: 'PATCH',
        body: JSON.stringify({
          action: input.action,
          expected_revision: input.expectedRevision,
          ...(input.title === undefined ? {} : { title: input.title }),
        }),
      },
    )
    return mapAiConversationSummary(response.data)
  }

  async uploadAiSource(input: AiSourceUploadInput): Promise<AiSource> {
    const query = new URLSearchParams({
      file_name: input.file.name,
      media_type: input.file.type || 'application/octet-stream',
    })
    query.set('conversation_id', input.conversationId)
    if (input.projectId) query.set('project_id', input.projectId)
    const response = await this.request<ApiItem<RawAiSource>>(
      `/ai/sources/upload?${query.toString()}`,
      { method: 'POST', body: input.file },
      { contentType: input.file.type || 'application/octet-stream' },
    )
    return mapAiSource(response.data)
  }

  async listAiSources(input: AiSourceListInput): Promise<AiSource[]> {
    const query = new URLSearchParams({ conversation_id: input.conversationId })
    if (input.projectId) query.set('project_id', input.projectId)
    if (input.status) query.set('status', input.status)
    const response = await this.request<ApiCollection<RawAiSource>>(
      `/ai/sources?${query.toString()}`,
    )
    return response.data.map(mapAiSource)
  }

  async archiveAiSource(sourceId: string, input: AiSourceArchiveInput): Promise<AiSource> {
    const response = await this.request<ApiItem<RawAiSource>>(
      `/ai/sources/${encodeURIComponent(sourceId)}/archive`,
      {
        method: 'POST',
        body: JSON.stringify({
          project_id: input.projectId,
          expected_revision: input.expectedRevision,
        }),
      },
    )
    return mapAiSource(response.data)
  }

  async deleteAiSource(sourceId: string): Promise<void> {
    await this.request<void>(
      `/ai/sources/${encodeURIComponent(sourceId)}`,
      { method: 'DELETE' },
    )
  }

  async getAiAutonomy(conversationId: string): Promise<AiAutonomyView> {
    const response = await this.request<ApiItem<AiAutonomyView>>(
      `/ai/conversations/${encodeURIComponent(conversationId)}/autonomy`,
    )
    return response.data
  }

  async setAiAutonomy(
    conversationId: string,
    input: AiAutonomyUpdateInput,
  ): Promise<AiAutonomyView> {
    const response = await this.request<ApiItem<AiAutonomyView>>(
      `/ai/conversations/${encodeURIComponent(conversationId)}/autonomy`,
      {
        method: 'PUT',
        body: JSON.stringify({
          mode: input.mode,
          expectedRevision: input.expectedRevision,
          ...(input.currentPassword ? { currentPassword: input.currentPassword } : {}),
        }),
      },
    )
    return response.data
  }

  async listAiDrafts(projectId?: string, status?: AiDraftStatus): Promise<AiWriteDraft[]> {
    const query = new URLSearchParams()
    if (projectId) query.set('project_id', projectId)
    if (status) query.set('status', status)
    const suffix = query.size ? `?${query.toString()}` : ''
    const response = await this.request<ApiCollection<AiWriteDraft>>(`/ai/approvals${suffix}`)
    return response.data
  }

  async getAiDraft(draftId: string): Promise<AiWriteDraft> {
    const response = await this.request<ApiItem<AiWriteDraft>>(`/ai/approvals/${encodeURIComponent(draftId)}`)
    return response.data
  }

  async decideAiDraft(draftId: string, input: AiDraftDecisionInput): Promise<AiDraftDecisionResponse> {
    const response = await this.request<ApiItem<AiDraftDecisionResponse>>(
      `/ai/approvals/${encodeURIComponent(draftId)}/decision`,
      { method: 'POST', body: JSON.stringify(input) },
    )
    return response.data
  }
}

const aiEntityLabels: Record<AiEntityType, string> = {
  lab: '实验室', user: '用户', project: '科研项目', membership: '成员关系', cage: '笼位',
  animal: '动物', animal_event: '动物事件', gene_locus: '基因位点', allele: '等位基因',
  genotype: '旧版基因型', genotype_definition: '基因型定义', genotyping_record: '基因检测',
  breeding_line: '繁育品系', colony: '繁育群体', breeding_pair: '繁育配对',
  breeding_pair_member: '配对成员', mating_event: '交配事件', litter: '窝次',
  animal_draft: '待登记动物', pedigree: '谱系', experiment_event: '实验事件',
  observation_definition: '观察定义', observation: '观察记录',
  observation_value: '观察值', experiment_template_version: '实验模板',
  experiment: '实验', cohort: '实验组', participation: '实验参与', procedure: '实验步骤',
  measurement: '测量', sample: '样本', attachment: '附件',
  project_animal_assignment: '项目动物关系', ai_conversation: 'AI 会话',
  ai_conversation_source: 'AI 会话来源', tool_run: '工具执行', approval: '审批',
  job: '数据任务',
}

function aiEntityRoute(entityType: string, entityId: string): string | undefined {
  const encoded = encodeURIComponent(entityId)
  if (entityType === 'animal') return `/animals?animal=${encoded}`
  if (entityType === 'cage') return `/cages?focus=${encoded}`
  // Only emit routes whose target views consume the exact focus query. A
  // generic section URL would look actionable while silently losing context.
  return undefined
}

function mapAiCitation(raw: RawAiCitation) {
  const entityLabel = aiEntityLabels[raw.entity_type as AiEntityType]
    ?? `未知实体（${raw.entity_type}）`
  return {
    entityType: raw.entity_type,
    entityId: raw.entity_id,
    revision: raw.revision ?? undefined,
    label: `${entityLabel} ${raw.entity_id.slice(0, 8)}`,
    route: aiEntityRoute(raw.entity_type, raw.entity_id),
  }
}

function mapAiTurn(raw: RawAiTurnResponse): AiTurnResponse {
  return {
    conversationId: raw.conversationId,
    content: raw.content,
    citations: raw.citations.map(mapAiCitation),
    toolRuns: raw.toolRuns.map((run) => ({
      toolRunId: run.tool_run_id,
      providerCallId: run.provider_call_id,
      tool: run.tool,
      arguments: run.arguments,
      outcome: run.outcome,
      citations: run.citations.map(mapAiCitation),
      draftId: run.draft_id ?? undefined,
    })),
    drafts: raw.drafts,
    trace: {
      providerId: raw.trace.providerId,
      model: raw.trace.model,
      usage: {
        providerCalls: raw.trace.usage.provider_calls,
        toolCalls: raw.trace.usage.tool_calls,
        inputTokens: raw.trace.usage.input_tokens,
        outputTokens: raw.trace.usage.output_tokens,
        totalTokens: raw.trace.usage.total_tokens,
      },
      context: {
        estimatedInputTokens: raw.trace.context?.estimatedInputTokens ?? 0,
        inputTokenCountIsEstimate: raw.trace.context?.inputTokenCountIsEstimate ?? false,
        contextTrimmed: raw.trace.context?.contextTrimmed ?? false,
        trimmedHistoryTurns: raw.trace.context?.trimmedHistoryTurns ?? 0,
        trimReasons: raw.trace.context?.trimReasons ?? [],
      },
    },
    incompleteReason: raw.incompleteReason ?? undefined,
    autonomy: raw.autonomy ?? defaultAiAutonomy(),
  }
}

function defaultAiAutonomy(): AiAutonomyView {
  return {
    mode: 'ask',
    effectiveMode: 'ask',
    maxMode: 'full',
    batchLimit: 1,
    revision: 0,
    requiresHumanApproval: [
      'research_signature',
      'animal_transfer_or_death',
      'delete_or_bulk_import',
      'permissions_and_accounts',
      'audit_or_log_cleanup',
      'breeding_scientific_facts',
    ],
  }
}

function mapAiConversationDetail(raw: RawAiConversationDetail): AiConversationDetail {
  return {
    conversation: mapAiConversationSummary(raw.conversation),
    messages: raw.messages.map((message) => ({
      id: message.id,
      sequence: message.sequence,
      role: message.role,
      content: message.content,
      sourceRefs: (message.sourceRefs ?? message.source_refs ?? []).map((source) => ({
        sourceId: source.sourceId ?? source.source_id ?? '',
        sourceRevision: source.sourceRevision ?? source.source_revision ?? 0,
        fileName: source.fileName ?? source.file_name ?? '',
        mediaType: source.mediaType ?? source.media_type ?? undefined,
        sizeBytes: source.sizeBytes ?? source.size_bytes ?? 0,
      })),
      response: message.response ? mapAiTurn(message.response) : undefined,
      createdAt: message.createdAt,
    })),
  }
}

function mapAiConversationSummary(raw: RawAiConversationSummary): AiConversationSummary {
  return {
    id: raw.id,
    projectId: raw.projectId ?? raw.project_id ?? undefined,
    title: raw.title,
    pinnedAt: raw.pinnedAt ?? raw.pinned_at ?? undefined,
    archivedAt: raw.archivedAt ?? raw.archived_at ?? undefined,
    createdAt: raw.createdAt ?? raw.created_at ?? '',
    updatedAt: raw.updatedAt ?? raw.updated_at ?? '',
    revision: raw.revision,
  }
}

function mapAiSourceStatus(status: string): AiSource['status'] {
  if (status === 'staged'
    || status === 'ready'
    || status === 'archived'
    || status === 'failed'
    || status === 'expired') {
    return status
  }
  throw new GatewayError('服务端返回了无法识别的 AI 来源状态', 'invalid_ai_source_status')
}

function mapAiSource(raw: RawAiSource): AiSource {
  return {
    id: raw.id,
    conversationId: raw.conversationId ?? raw.conversation_id ?? undefined,
    projectId: raw.projectId ?? raw.project_id ?? undefined,
    fileName: raw.fileName ?? raw.file_name ?? '未命名文件',
    mediaType: raw.mediaType ?? raw.media_type ?? 'application/octet-stream',
    sizeBytes: raw.sizeBytes ?? raw.size_bytes ?? 0,
    status: mapAiSourceStatus(raw.status),
    revision: raw.revision,
    createdAt: raw.createdAt ?? raw.created_at ?? '',
    expiresAt: raw.expiresAt ?? raw.expires_at ?? '',
  }
}

function mapAuthUser(raw: RawAuthUser): AuthUser {
  return {
    id: raw.id,
    labId: raw.lab_id,
    email: raw.email ?? undefined,
    displayName: raw.display_name,
    labRoles: raw.lab_roles,
    projectRoles: raw.project_roles.map((grant) => ({
      projectId: grant.project_id,
      role: grant.role,
    })),
    aiScopes: raw.ai_scopes,
    authentication: raw.authentication,
    mustChangePassword: raw.must_change_password ?? false,
    isEnvironmentRoot: raw.is_environment_root ?? false,
  }
}

function redirectToPasswordChange() {
  if (typeof window === 'undefined') return
  const current = window.location.hash.slice(1) || '/'
  if (current.startsWith('/change-password')) return
  const redirect = current.startsWith('/') && !current.startsWith('//') ? current : '/'
  window.location.hash = `/change-password?redirect=${encodeURIComponent(redirect)}`
}

function redirectToRemoteLogin() {
  if (typeof window === 'undefined') return
  const current = window.location.hash.slice(1) || '/'
  if (current.startsWith('/login')) return
  const redirect = current.startsWith('/') && !current.startsWith('//') ? current : '/'
  window.location.hash = `/login?redirect=${encodeURIComponent(redirect)}`
}

function mapAnimalStatus(status: RawAnimal['current_status']): Animal['status'] {
  if (status === 'in_experiment' || status === 'sampled') return 'experiment'
  if (status === 'deceased' || status === 'euthanized' || status === 'lost' || status === 'archived') return 'archived'
  return 'active'
}

function mapAnimalOverview(raw: RawAnimalOverview): Animal {
  const animal = mapAnimal(raw.animal)
  animal.genotype = raw.genotype
  animal.projectRefs = raw.projects.map(({ id, name }) => ({ id, name }))
  animal.projectNames = animal.projectRefs.map((project) => project.name)
  animal.weight = raw.latest_weight?.value
  return animal
}

function mapDetailSample(raw: RawDetailSample): AnimalSample {
  return {
    id: raw.id,
    projectId: raw.project_id,
    experimentId: raw.experiment_id ?? undefined,
    sampleType: raw.sample_type,
    quantity: raw.quantity ?? undefined,
    unit: raw.unit ?? undefined,
    location: raw.location ?? undefined,
    collectedAt: raw.collected_at,
    revision: raw.revision,
  }
}

function mapAnimalDetail(
  raw: RawAnimalDetail,
  cageNames: Map<string, string>,
): AnimalDetail {
  return {
    timeline: raw.events.map((event) => mapTimelineEvent(event, cageNames)),
    experiments: raw.experiments.map((record) => ({
      projectId: record.project.id,
      projectName: record.project.name,
      experimentId: record.experiment.id,
      experimentName: record.experiment.name,
      experimentStatus: record.experiment.status,
      cohortId: record.cohort?.id,
      cohortName: record.cohort?.name,
      participationId: record.participation.id,
      participationStatus: record.participation.status,
      enrolledAt: record.participation.enrolled_at,
      exitedAt: record.participation.exited_at ?? undefined,
      revision: record.participation.revision,
    })),
    measurements: raw.measurements.map((measurement) => ({
      id: measurement.id,
      projectId: measurement.project_id,
      experimentId: measurement.experiment_id ?? undefined,
      key: measurement.key,
      label: measurement.label,
      value: measurement.value,
      unit: measurement.unit ?? undefined,
      measuredAt: measurement.measured_at,
      status: measurement.status,
      revision: measurement.revision,
    })),
    pedigree: raw.pedigree.map((relation) => ({
      id: relation.id,
      direction: relation.direction,
      parentType: relation.parent_type,
      relatedAnimal: {
        id: relation.related_animal.id,
        code: relation.related_animal.display_id,
        sex: relation.related_animal.sex,
        strain: relation.related_animal.strain ?? undefined,
        status: mapAnimalStatus(relation.related_animal.current_status),
      },
      revision: relation.revision,
    })),
    samples: raw.samples.map(mapDetailSample),
    attachments: raw.attachments.map((attachment) => ({
      id: attachment.id,
      projectId: attachment.project_id ?? undefined,
      entityType: attachment.entity_type,
      entityId: attachment.entity_id,
      fileName: attachment.file_name,
      mediaType: attachment.media_type ?? undefined,
      sizeBytes: attachment.size_bytes,
      sha256: attachment.sha256,
      version: attachment.version,
      contentHref: attachment.content_href,
      createdAt: attachment.created_at,
    })),
    auditVisible: raw.audit_visible,
    audits: raw.audits.map((audit) => ({
      id: audit.id,
      action: audit.action,
      actor: audit.actor,
      source: audit.source,
      reason: audit.reason ?? undefined,
      occurredAt: audit.occurred_at,
      revision: audit.revision ?? undefined,
    })),
    provenance: raw.provenance.map((entry) => ({
      id: entry.id,
      source: entry.source,
      actor: entry.actor ?? undefined,
      recordedAt: entry.recorded_at,
      requestId: entry.request_id ?? undefined,
    })),
  }
}

function mapGeneLocus(raw: RawGeneLocus): GeneLocus {
  return {
    id: raw.id,
    symbol: raw.symbol,
    description: raw.description ?? undefined,
    archivedAt: raw.meta.deleted_at ?? undefined,
    revision: raw.meta.revision,
  }
}

function mapAllele(raw: RawAllele): GeneAllele {
  return {
    id: raw.id,
    locusId: raw.locus_id,
    symbol: raw.symbol,
    description: raw.description ?? undefined,
    isWildType: raw.is_wild_type,
    archivedAt: raw.meta.deleted_at ?? undefined,
    revision: raw.meta.revision,
  }
}

function mapGenotype(raw: RawGenotype): AnimalGenotype {
  return {
    id: raw.id,
    animalId: raw.animal_id,
    locusId: raw.locus_id,
    allele1Id: raw.allele_1_id ?? undefined,
    allele2Id: raw.allele_2_id ?? undefined,
    assessedAt: raw.assessed_at ?? undefined,
    revision: raw.meta.revision,
  }
}

function mapGenotypeDefinition(raw: RawGenotypeDefinition): GenotypeDefinition {
  return {
    id: raw.id,
    name: raw.name,
    description: raw.description ?? undefined,
    components: raw.components
      .slice()
      .sort((left, right) => left.display_order - right.display_order)
      .map((component) => ({
        id: component.id,
        genotypeDefinitionId: component.genotype_definition_id,
        locusId: component.locus_id,
        allele1Id: component.allele_1_id,
        allele2Id: component.allele_2_id ?? undefined,
        mode: component.mode,
        displayOrder: component.display_order,
        revision: component.meta.revision,
      })),
    revision: raw.meta.revision,
    createdAt: raw.meta.created_at,
    updatedAt: raw.meta.updated_at,
    archivedAt: raw.meta.deleted_at ?? undefined,
  }
}

function mapGenotypingRecord(raw: RawGenotypingRecord): GenotypingRecord {
  return {
    id: raw.id,
    projectId: raw.project_id ?? undefined,
    animalId: raw.animal_id,
    genotypeDefinitionId: raw.genotype_definition_id,
    state: raw.state,
    assessedAt: raw.assessed_at ?? undefined,
    method: raw.method ?? undefined,
    notes: raw.notes ?? undefined,
    supersedesRecordId: raw.supersedes_record_id ?? undefined,
    voidedAt: raw.voided_at ?? undefined,
    voidReason: raw.void_reason ?? undefined,
    revision: raw.meta.revision,
    createdAt: raw.meta.created_at,
    updatedAt: raw.meta.updated_at,
  }
}

function mapGeneticsReferenceCounts(raw: RawGeneticsReferenceCounts): GeneticsReferenceCounts {
  return {
    activeGenotypeDefinitions: raw.active_genotype_definitions,
    genotypeDefinitions: raw.genotype_definitions,
    genotypingRecords: raw.genotyping_records,
    breedingLines: raw.breeding_lines,
  }
}

function mapBreedingLine(raw: RawBreedingLine): BreedingLine {
  return {
    id: raw.id,
    name: raw.name,
    description: raw.description ?? undefined,
    genotypeDefinitionIds: raw.genotype_definition_ids,
    revision: raw.meta.revision,
    createdAt: raw.meta.created_at,
  }
}

function mapColony(raw: RawColony): Colony {
  return {
    id: raw.id,
    breedingLineId: raw.breeding_line_id,
    name: raw.name,
    description: raw.description ?? undefined,
    revision: raw.meta.revision,
    createdAt: raw.meta.created_at,
  }
}

function mapBreedingPair(raw: RawBreedingPair): BreedingPair {
  return {
    id: raw.id,
    colonyId: raw.colony_id,
    name: raw.name,
    status: raw.status,
    startedAt: raw.started_at,
    endedAt: raw.ended_at ?? undefined,
    members: raw.members.map((member) => ({
      id: member.id,
      breedingPairId: member.breeding_pair_id,
      animalId: member.animal_id,
      role: member.role,
      joinedAt: member.joined_at,
      leftAt: member.left_at ?? undefined,
      revision: member.meta.revision,
    })),
    revision: raw.meta.revision,
    createdAt: raw.meta.created_at,
  }
}

function mapMatingEvent(raw: RawMatingEvent): MatingEvent {
  return {
    id: raw.id,
    breedingPairId: raw.breeding_pair_id,
    maleAnimalId: raw.male_animal_id,
    femaleAnimalId: raw.female_animal_id,
    occurredAt: raw.occurred_at,
    notes: raw.notes ?? undefined,
    revision: raw.meta.revision,
  }
}

function mapLitter(raw: RawLitter): Litter {
  return {
    id: raw.id,
    matingEventId: raw.mating_event_id,
    bornOn: raw.born_on,
    sizeTotal: raw.size_total,
    sizeAlive: raw.size_alive,
    notes: raw.notes ?? undefined,
    revision: raw.meta.revision,
  }
}

function mapAnimalDraft(raw: RawAnimalDraft): AnimalDraft {
  return {
    id: raw.id,
    litterId: raw.litter_id,
    temporaryLabel: raw.temporary_label,
    sex: raw.sex,
    birthDate: raw.birth_date,
    status: raw.status,
    registeredAnimalId: raw.registered_animal_id ?? undefined,
    revision: raw.meta.revision,
  }
}

function mapLocusPrediction(raw: RawLocusPrediction): LocusPrediction {
  return {
    locusId: raw.locus_id,
    outcomes: raw.outcomes.map((outcome) => ({
      paternalAlleleId: outcome.paternal_allele_id ?? undefined,
      maternalAlleleId: outcome.maternal_allele_id ?? undefined,
      probability: outcome.probability,
    })),
  }
}

function mapExperimentEvent(raw: RawExperimentEvent): ExperimentEvent {
  return {
    id: raw.id,
    projectId: raw.project_id,
    experimentId: raw.experiment_id,
    eventKey: raw.event_key,
    label: raw.label,
    occurredAt: raw.occurred_at,
    details: raw.details,
    revision: raw.meta.revision,
  }
}

function mapObservationDefinition(raw: RawObservationDefinition): ObservationDefinition {
  return {
    id: raw.id,
    projectId: raw.project_id,
    experimentId: raw.experiment_id,
    key: raw.key,
    label: raw.label,
    valueType: raw.value_type,
    unit: raw.unit ?? undefined,
    categories: raw.categories,
    policy: raw.policy,
    revision: raw.meta.revision,
  }
}

function mapObservation(raw: RawObservation): Observation {
  return {
    id: raw.id,
    projectId: raw.project_id,
    experimentId: raw.experiment_id,
    experimentEventId: raw.experiment_event_id,
    definitionId: raw.definition_id,
    subjectType: raw.subject_type,
    subjectId: raw.subject_id,
    context: raw.context,
    currentValueVersion: raw.current_value_version,
    revision: raw.meta.revision,
  }
}

function mapObservationValueRecord(raw: RawObservationValueRecord): ObservationValueRecord {
  return {
    id: raw.id,
    observationId: raw.observation_id,
    version: raw.version,
    value: raw.value,
    recordedAt: raw.recorded_at,
    recordedBy: raw.recorded_by ?? undefined,
    notes: raw.notes ?? undefined,
    revision: raw.meta.revision,
  }
}

function mapRecordedObservation(raw: RawRecordedObservation): RecordedObservation {
  return {
    observation: mapObservation(raw.observation),
    value: mapObservationValueRecord(raw.value),
  }
}

function mapAnimal(raw: RawAnimal): Animal {
  return {
    id: raw.id,
    code: raw.display_id,
    legacyCode: raw.legacy_id ?? undefined,
    sex: raw.sex,
    strain: raw.strain ?? '未设置',
    genotype: '待确认',
    birthDate: raw.birth_date ?? '',
    status: mapAnimalStatus(raw.current_status),
    cageId: raw.current_cage_id ?? null,
    projectNames: [],
    timeline: [],
  }
}

function mapTimelineEvent(raw: RawAnimalEvent, cages: Map<string, string>): TimelineEvent {
  const kind = raw.kind
  let type: TimelineEvent['type'] = 'note'
  let title = '记录'
  let detail = '动物记录已更新'
  switch (kind.type) {
    case 'born':
      type = 'birth'; title = '出生登记'; detail = `出生日期 ${String(kind.birth_date ?? '')}`; break
    case 'transferred': {
      type = 'transfer'; title = '转笼'
      const from = typeof kind.from_cage_id === 'string' ? (cages.get(kind.from_cage_id) ?? kind.from_cage_id) : '未分配'
      const to = typeof kind.to_cage_id === 'string' ? (cages.get(kind.to_cage_id) ?? kind.to_cage_id) : '未分配'
      detail = `${from} → ${to}`
      break
    }
    case 'genotyped': type = 'genotype'; title = '基因型记录'; detail = '已更新基因型'; break
    case 'experiment_enrolled':
    case 'procedure_performed': type = 'experiment'; title = '实验记录'; detail = '已关联实验过程'; break
    case 'experiment_participation_ended':
      type = 'experiment'
      title = kind.status === 'completed' ? '实验参与完成' : '退出实验'
      detail = kind.status === 'completed' ? '动物已完成本次实验参与' : '动物已退出本次实验'
      break
    case 'measurement_recorded': type = 'measurement'; title = '记录测量'; detail = '已关联测量数据'; break
    case 'sample_collected': type = 'sampling'; title = '采集样本'; detail = '已关联采样记录'; break
    case 'status_changed': title = '状态变更'; detail = `${String(kind.from ?? '')} → ${String(kind.to ?? '')}`; break
    case 'registered': title = '登记动物'; detail = '创建动物档案'; break
    case 'note': title = '备注'; detail = String(kind.body ?? ''); break
  }
  return {
    id: raw.id,
    at: raw.occurred_at,
    type,
    title,
    detail: raw.notes ?? detail,
    operator: raw.recorded_by ? '实验室用户' : 'MuriArc',
  }
}

function mapCages(cages: RawCage[], animals: RawAnimal[]): Cage[] {
  return cages.map((raw) => {
    const residents = animals.filter((animal) => animal.current_cage_id === raw.id)
    const strains = [...new Set(residents.map((animal) => animal.strain).filter(Boolean))]
    return {
      id: raw.id,
      code: raw.display_id,
      room: raw.section,
      rack: raw.location ?? '未设置',
      capacity: raw.capacity,
      animalIds: residents.map((animal) => animal.id),
      status: residents.length === 0 ? 'empty' : residents.length > raw.capacity ? 'attention' : 'normal',
      summary: residents.length === 0 ? '空笼' : strains.length ? strains.join(' · ') : `${residents.length} 只动物`,
    }
  })
}

function mapProjectAnimalAssignment(
  raw: RawProjectAnimalAssignment,
): ProjectAnimalAssignment {
  return {
    id: raw.id,
    projectId: raw.project_id,
    animalId: raw.animal_id,
    assignedBy: raw.assigned_by ?? undefined,
    reason: raw.reason ?? undefined,
    assignedAt: raw.meta.created_at,
    revision: raw.meta.revision,
  }
}

function mapTemplate(raw: RawTemplate): ExperimentTemplateSummary {
  return { id: raw.id, name: raw.name, version: raw.version }
}

function mapCohort(raw: RawCohort): Cohort {
  return {
    id: raw.id,
    experimentId: raw.experiment_id,
    name: raw.name,
    description: raw.description ?? undefined,
  }
}

function mapParticipation(raw: RawParticipation): Participation {
  return {
    id: raw.id,
    experimentId: raw.experiment_id,
    animalId: raw.animal_id,
    cohortId: raw.cohort_id ?? undefined,
    status: raw.status,
    enrolledAt: raw.enrolled_at,
    exitedAt: raw.exited_at ?? undefined,
    genotypeSnapshot: (raw.genotype_snapshot ?? []).map((entry) => ({
      genotypingRecordId: entry.genotyping_record_id,
      genotypeDefinitionId: entry.genotype_definition_id,
      state: entry.state,
      assessedAt: entry.assessed_at ?? undefined,
    })),
    revision: raw.meta.revision,
  }
}

function mapProcedure(raw: RawProcedure): Procedure {
  return {
    id: raw.id,
    experimentId: raw.experiment_id,
    animalId: raw.animal_id ?? undefined,
    name: raw.name,
    scheduledAt: raw.scheduled_at ?? undefined,
    performedAt: raw.performed_at ?? undefined,
    status: raw.status,
    details: raw.details,
  }
}

function mapExperiment(raw: RawExperiment, project: string): Experiment {
  const status: Experiment['status'] = raw.status === 'active'
    ? 'active'
    : raw.status === 'completed'
      ? 'completed'
      : raw.status === 'cancelled' || raw.status === 'archived'
        ? 'cancelled'
        : 'draft'
  return {
    id: raw.id,
    projectId: raw.project_id,
    code: `EXP-${raw.id.slice(0, 8).toUpperCase()}`,
    name: raw.name,
    project,
    status,
    startDate: raw.starts_at?.slice(0, 10) ?? '',
    animalCount: 0,
    completedSteps: status === 'completed' ? 1 : 0,
    totalSteps: 1,
    groups: [],
    revision: raw.meta.revision,
  }
}

function mapJob(raw: RawJob): DataJob {
  const progress = raw.progress_total && raw.progress_total > 0
    ? Math.round(raw.progress_current / raw.progress_total * 100)
    : raw.status === 'completed' ? 100 : 0
  const kind = raw.kind === 'bulk_operation' ? 'import' : raw.kind
  const kindName = kind === 'import' ? '导入任务' : kind === 'export' ? '导出任务' : '快照任务'
  return {
    id: raw.id,
    name: `${kindName} ${raw.id.slice(0, 8)}`,
    kind,
    status: raw.status === 'completed' ? 'completed'
      : raw.status === 'awaiting_confirmation' ? 'needs-review'
        : raw.status === 'queued' ? 'queued'
          : raw.status === 'failed' ? 'failed'
            : raw.status === 'cancelled' ? 'cancelled' : 'running',
    progress,
    createdAt: new Date(raw.created_at).toLocaleString('zh-CN'),
    detail: raw.status === 'failed' ? '任务执行失败，请查看报告'
      : raw.status === 'cancelled' ? '任务已取消'
        : raw.result_available ? '任务已生成结果' : '任务处理中',
  }
}

function attachmentScopeQuery(scope: AttachmentScope): URLSearchParams {
  const query = new URLSearchParams({
    entity_type: scope.entityType,
    entity_id: scope.entityId,
  })
  if (scope.projectId) query.set('project_id', scope.projectId)
  return query
}

function mapAttachment(raw: RawAttachment): AttachmentMetadata {
  return {
    id: raw.id,
    projectId: raw.project_id ?? undefined,
    entityType: raw.entity_type,
    entityId: raw.entity_id,
    fileName: raw.file_name,
    mediaType: raw.media_type ?? undefined,
    sizeBytes: raw.size_bytes,
    sha256: raw.sha256,
    version: raw.version,
    revision: raw.meta.revision ?? raw.version,
    contentHref: raw.content_href,
    previewSupported: raw.preview_supported,
    previewHref: raw.preview_href ?? undefined,
    previewReason: raw.preview_reason ?? undefined,
    createdAt: raw.meta.created_at,
  }
}

const clone = <T>(value: T): T => structuredClone(value)

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
  aiProviderEndpoints: AiProviderEndpoint[] = builtinAiProviderPresets
    .filter((preset) => preset.recommendedBaseUrl)
    .map((preset, index) => ({
      id: `00000000-0000-0000-0000-${String(index + 1).padStart(12, '0')}`,
      providerKind: preset.providerKind,
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
      label: input.label,
      baseUrl: input.baseUrl,
      enabled: input.enabled,
      builtin: false,
      revision: 0,
    }
    endpoint.providerKind = input.providerKind
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
  async aiTurn(input: AiTurnInput): Promise<AiTurnResponse> {
    await pause(80)
    for (const sourceId of input.sourceRefs ?? []) {
      const source = this.aiSources.get(sourceId)
      if (!source
        || source.status !== 'ready'
        || Date.parse(source.expiresAt) <= Date.now()) {
        throw new Error('所选 AI 文件已失效，请重新上传')
      }
    }
    const conversationId = input.conversationId ?? crypto.randomUUID()
    const now = new Date().toISOString()
    const citation = {
      entityType: 'animal' as const,
      entityId: 'animal-006',
      label: '动物 M-26006',
      route: '/animals?animal=animal-006',
    }
    const response: AiTurnResponse = {
      conversationId,
      content: input.sourceRefs?.length
        ? `已收到 ${input.sourceRefs.length} 个演示文件。浏览器演示不会解析或写入正式数据库，请在 Desktop 或 Server 模式完成预览。`
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
      },
    }
    let detail = this.aiConversations.get(conversationId)
    if (!detail) {
      detail = {
        conversation: {
          id: conversationId,
          projectId: input.projectId,
          title: input.message.trim().slice(0, 80) || '新会话',
          createdAt: now,
          updatedAt: now,
          revision: 0,
        },
        messages: [],
      }
      this.aiConversations.set(conversationId, detail)
    }
    detail.messages.push(
      {
        id: crypto.randomUUID(),
        sequence: detail.messages.length + 1,
        role: 'user',
        content: input.message,
        createdAt: now,
      },
      {
        id: crypto.randomUUID(),
        sequence: detail.messages.length + 2,
        role: 'assistant',
        content: response.content,
        response: clone(response),
        createdAt: now,
      },
    )
    detail.conversation.updatedAt = now
    detail.conversation.revision += 1
    return clone(response)
  }
  async createAiConversation(
    input: AiConversationCreateInput,
  ): Promise<AiConversationSummary> {
    await pause(20)
    const title = input.title.trim()
    if (!title || [...title].length > 256 || /[\u0000-\u001f\u007f]/u.test(title)) {
      throw new Error('AI 会话标题无效')
    }
    const now = new Date().toISOString()
    const conversation: AiConversationSummary = {
      id: crypto.randomUUID(),
      projectId: input.projectId,
      title,
      createdAt: now,
      updatedAt: now,
      revision: 1,
    }
    this.aiConversations.set(conversation.id, {
      conversation,
      messages: [],
    })
    return clone(conversation)
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
    const conversation = this.aiConversations.get(input.conversationId)?.conversation
    if (!conversation || (input.projectId && input.projectId !== conversation.projectId)) {
      throw new Error('AI 会话不存在或文件范围不匹配')
    }
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

function isTauriRuntime(): boolean {
  return typeof window !== 'undefined' && ('__TAURI_INTERNALS__' in window || '__TAURI__' in window)
}

export function createGateway(selection?: GatewaySelection): MuriArcGateway {
  const selected = selection ?? import.meta.env.VITE_MURIARC_GATEWAY
  if (selected === 'demo') return new DemoGateway()
  if (selected === 'remote') return new RemoteHttpGateway()
  if (selected === 'local' || (!selected && isTauriRuntime())) return new LocalTauriGateway()
  if (!selected && import.meta.env.DEV) return new DemoGateway()
  throw new Error('必须为 MuriArc Web 构建显式设置 VITE_MURIARC_GATEWAY=remote')
}

export const gateway = createGateway()
