export type GatewayMode = 'local' | 'remote'
export type AnimalSex = 'male' | 'female' | 'unknown'
export type AnimalStatus = 'active' | 'breeding' | 'experiment' | 'archived'
export type CageStatus = 'normal' | 'attention' | 'empty'

export interface Cage {
  id: string
  code: string
  room: string
  rack: string
  capacity: number
  animalIds: string[]
  status: CageStatus
  summary: string
  note?: string
}

export interface TimelineEvent {
  id: string
  at: string
  type: 'birth' | 'transfer' | 'genotype' | 'experiment' | 'measurement' | 'sampling' | 'note'
  title: string
  detail: string
  operator: string
}

export interface Animal {
  id: string
  code: string
  legacyCode?: string
  sex: AnimalSex
  strain: string
  genotype: string
  birthDate: string
  status: AnimalStatus
  cageId: string | null
  projectNames: string[]
  projectRefs?: ProjectSummary[]
  weight?: number
  timeline: TimelineEvent[]
}

export interface ProjectSummary {
  id: string
  name: string
}

export interface ProjectAnimalAssignment {
  id: string
  projectId: string
  animalId: string
  assignedBy?: string
  reason?: string
  assignedAt: string
  revision: number
}

export type TemplateFieldValueType = 'number' | 'text' | 'boolean' | 'date' | 'category'

export interface ExperimentTemplateSummary {
  id: string
  name: string
  version: number
}

export interface Cohort {
  id: string
  experimentId: string
  name: string
  description?: string
}

export interface GenotypeSnapshotEntry {
  genotypingRecordId: string
  genotypeDefinitionId: string
  state: GenotypingState
  assessedAt?: string
}

export interface Participation {
  id: string
  experimentId: string
  animalId: string
  cohortId?: string
  status: 'enrolled' | 'completed' | 'withdrawn'
  enrolledAt: string
  exitedAt?: string
  genotypeSnapshot: GenotypeSnapshotEntry[]
  revision: number
}

export interface Procedure {
  id: string
  experimentId: string
  animalId?: string
  name: string
  scheduledAt?: string
  performedAt?: string
  status: 'planned' | 'completed' | 'skipped' | 'cancelled'
  details: Record<string, unknown>
}

export interface Experiment {
  id: string
  projectId: string
  code: string
  name: string
  project: string
  status: 'draft' | 'active' | 'completed' | 'cancelled'
  startDate: string
  animalCount: number
  completedSteps: number
  totalSteps: number
  groups: Array<{ name: string; count: number; color: string }>
  nextAction?: string
  revision: number
}

export type MeasurementDataValue =
  | { type: 'number'; value: number }
  | { type: 'text'; value: string }
  | { type: 'boolean'; value: boolean }
  | { type: 'date'; value: string }
  | { type: 'category'; value: string }

export interface AnimalExperimentRecord {
  projectId: string
  projectName: string
  experimentId: string
  experimentName: string
  experimentStatus: 'draft' | 'active' | 'completed' | 'cancelled' | 'archived'
  cohortId?: string
  cohortName?: string
  participationId: string
  participationStatus: 'enrolled' | 'completed' | 'withdrawn'
  enrolledAt: string
  exitedAt?: string
  revision: number
}

export interface AnimalMeasurement {
  id: string
  projectId: string
  experimentId?: string
  key: string
  label: string
  value: MeasurementDataValue
  unit?: string
  measuredAt: string
  status: 'draft' | 'signed'
  revision: number
}

export interface AnimalSample {
  id: string
  projectId: string
  experimentId?: string
  sampleType: string
  quantity?: number
  unit?: string
  location?: string
  collectedAt: string
  revision: number
}

export interface RelatedAnimal {
  id: string
  code: string
  sex: AnimalSex
  strain?: string
  status: AnimalStatus
}

export interface PedigreeRelation {
  id: string
  direction: 'parent' | 'offspring'
  parentType: 'father' | 'mother' | 'unknown'
  relatedAnimal: RelatedAnimal
  revision: number
}

export interface AnimalAttachment {
  id: string
  projectId?: string
  entityType: 'project' | 'animal' | 'experiment' | 'measurement' | 'sample'
  entityId: string
  fileName: string
  mediaType?: string
  sizeBytes: number
  sha256: string
  version: number
  contentHref: string
  createdAt: string
}

export interface AuditSummary {
  id: string
  action: 'create' | 'update' | 'soft_delete' | 'publish' | 'sign' | 'import'
  actor: string
  source: 'desktop' | 'web' | 'api' | 'mcp' | 'ai' | 'migration'
  reason?: string
  occurredAt: string
  revision?: number
}

export interface ProvenanceSummary {
  id: string
  source: 'human' | 'import' | 'ai' | 'migration'
  actor?: string
  recordedAt: string
  requestId?: string
}

export interface GeneLocus {
  id: string
  symbol: string
  description?: string
  archivedAt?: string
  revision: number
}

export interface GeneAllele {
  id: string
  locusId: string
  symbol: string
  description?: string
  isWildType: boolean
  archivedAt?: string
  revision: number
}

export interface AnimalGenotype {
  id: string
  animalId: string
  locusId: string
  allele1Id?: string
  allele2Id?: string
  assessedAt?: string
  revision: number
}

export type GenotypeComponentMode =
  | 'diploid'
  | 'hemizygous'
  | 'transgene_presence'
  | 'conditional'

export type GenotypingState = 'unknown' | 'expected' | 'confirmed' | 'rejected'

export interface GenotypeComponent {
  id: string
  genotypeDefinitionId: string
  locusId: string
  allele1Id: string
  allele2Id?: string
  mode: GenotypeComponentMode
  displayOrder: number
  revision: number
}

export interface GenotypeDefinition {
  id: string
  name: string
  description?: string
  components: GenotypeComponent[]
  revision: number
  createdAt: string
  updatedAt: string
  archivedAt?: string
}

export interface GenotypingRecord {
  id: string
  projectId?: string
  animalId: string
  genotypeDefinitionId: string
  state: GenotypingState
  assessedAt?: string
  method?: string
  notes?: string
  supersedesRecordId?: string
  voidedAt?: string
  voidReason?: string
  revision: number
  createdAt: string
  updatedAt: string
}

export interface BreedingLine {
  id: string
  name: string
  description?: string
  genotypeDefinitionIds: string[]
  revision: number
  createdAt: string
}

export interface Colony {
  id: string
  breedingLineId: string
  name: string
  description?: string
  revision: number
  createdAt: string
}

export type BreedingPairStatus = 'active' | 'retired'
export type BreedingMemberRole = 'male' | 'female'

export interface BreedingPairMember {
  id: string
  breedingPairId: string
  animalId: string
  role: BreedingMemberRole
  joinedAt: string
  leftAt?: string
  revision: number
}

export interface BreedingPair {
  id: string
  colonyId: string
  name: string
  status: BreedingPairStatus
  startedAt: string
  endedAt?: string
  members: BreedingPairMember[]
  revision: number
  createdAt: string
}

export interface MatingEvent {
  id: string
  breedingPairId: string
  maleAnimalId: string
  femaleAnimalId: string
  occurredAt: string
  notes?: string
  revision: number
}

export interface Litter {
  id: string
  matingEventId: string
  bornOn: string
  sizeTotal: number
  sizeAlive: number
  notes?: string
  revision: number
}

export type AnimalDraftStatus = 'pending' | 'registered' | 'discarded'

export interface AnimalDraft {
  id: string
  litterId: string
  temporaryLabel: string
  sex: AnimalSex
  birthDate: string
  status: AnimalDraftStatus
  registeredAnimalId?: string
  revision: number
}

export interface CreatedLitter {
  litter: Litter
  drafts: AnimalDraft[]
}

export interface RegisteredAnimalDraft {
  draft: AnimalDraft
  animal: Animal
}

export interface MendelianOutcome {
  paternalAlleleId?: string
  maternalAlleleId?: string
  probability: number
}

export interface LocusPrediction {
  locusId: string
  outcomes: MendelianOutcome[]
}

export interface ExperimentEvent {
  id: string
  projectId: string
  experimentId: string
  eventKey: string
  label: string
  occurredAt: string
  details: Record<string, unknown>
  revision: number
}

export type ObservationValueType = 'number' | 'text' | 'boolean' | 'date' | 'category' | 'json'
export type ObservationPolicy = 'immutable' | 'mutable' | 'versioned'
export type ObservationSubjectType = 'experiment' | 'animal' | 'sample' | 'artifact'

export type ObservationValueData =
  | { type: 'number'; value: number }
  | { type: 'text'; value: string }
  | { type: 'boolean'; value: boolean }
  | { type: 'date'; value: string }
  | { type: 'category'; value: string }
  | { type: 'json'; value: unknown }

export interface ObservationDefinition {
  id: string
  projectId: string
  experimentId: string
  key: string
  label: string
  valueType: ObservationValueType
  unit?: string
  categories: string[]
  policy: ObservationPolicy
  revision: number
}

export interface Observation {
  id: string
  projectId: string
  experimentId: string
  experimentEventId: string
  definitionId: string
  subjectType: ObservationSubjectType
  subjectId: string
  context: Record<string, unknown>
  currentValueVersion: number
  revision: number
}

export interface ObservationValueRecord {
  id: string
  observationId: string
  version: number
  value: ObservationValueData
  recordedAt: string
  recordedBy?: string
  notes?: string
  revision: number
}

export interface RecordedObservation {
  observation: Observation
  value: ObservationValueRecord
}

export interface AnimalDetail {
  timeline: TimelineEvent[]
  experiments: AnimalExperimentRecord[]
  measurements: AnimalMeasurement[]
  pedigree: PedigreeRelation[]
  samples: AnimalSample[]
  attachments: AnimalAttachment[]
  auditVisible: boolean
  audits: AuditSummary[]
  provenance: ProvenanceSummary[]
}

export interface DataJob {
  id: string
  name: string
  kind: 'import' | 'export' | 'snapshot'
  status: 'queued' | 'running' | 'completed' | 'needs-review' | 'failed' | 'cancelled'
  progress: number
  createdAt: string
  detail: string
}

export interface WorkspaceSettings {
  labName: string
  operatorName: string
}

export type AiProviderKind = 'open_ai_compatible' | 'local_http'

export interface AiProviderModelPreset {
  id: string
  displayName: string
  contextWindowTokens: number
  maxOutputTokens: number
  supportsVision: boolean
}

export interface AiProviderPreset {
  id: string
  displayName: string
  providerKind: AiProviderKind
  recommendedBaseUrl: string
  models: AiProviderModelPreset[]
  supportsVision: boolean
  documentationUrl: string
  builtin: boolean
  enabled: boolean
  defaultPreset: boolean
}

export interface AiSettings {
  enabled: boolean
  providerKind: AiProviderKind
  providerPresetId: string
  model: string
  baseUrl: string
  hasKey: boolean
  supportsVision: boolean
  visionModel?: string
  contextWindowTokens: number
  maxInputTokens: number
  maxOutputTokens: number
  historyTokenBudget: number
  historyTurns: number
  temperature: number
  timeoutMs: number
  revision: number
}

export interface SaveAiSettingsInput {
  enabled: boolean
  providerKind: AiProviderKind
  providerPresetId: string
  model: string
  baseUrl: string
  supportsVision?: boolean
  visionModel?: string
  contextWindowTokens: number
  maxInputTokens: number
  maxOutputTokens: number
  historyTokenBudget: number
  historyTurns: number
  temperature: number
  timeoutMs: number
  /** Omit to keep the existing protected secret. Never populated from a read response. */
  apiKey?: string
}

export type LabRole = 'lab_admin' | 'animal_manager'
export type ProjectRole = 'project_admin' | 'editor' | 'viewer'
export type AiScope = 'read' | 'write-draft' | 'import' | 'export' | 'template-draft'

export interface AuthUser {
  id: string
  labId: string
  email?: string
  displayName: string
  labRoles: LabRole[]
  projectRoles: Array<{ projectId: string; role: ProjectRole }>
  aiScopes?: AiScope[]
  authentication: 'session' | 'bearer'
  mustChangePassword: boolean
  isEnvironmentRoot: boolean
}

export interface LoginInput {
  email: string
  password: string
}

export interface AuthSession {
  user: AuthUser
  /** Present after login or CSRF recovery; held in memory only. */
  csrfAvailable: boolean
  expiresAt?: string
}

export type ManagedUserStatus = 'active' | 'suspended'

export interface ManagedProjectMembership {
  membershipId: string
  projectId: string
  projectName: string
  role: ProjectRole
  revision: number
}

export interface ManagedUser {
  id: string
  email: string
  displayName: string
  status: ManagedUserStatus
  revision: number
  credentialRevision: number
  mustChangePassword: boolean
  isEnvironmentRoot: boolean
  labMembershipId?: string
  labRole?: LabRole
  labMembershipRevision?: number
  projectMemberships: ManagedProjectMembership[]
  createdAt: string
  updatedAt: string
}

export interface AiCitation {
  /**
   * Providers may return a newly-added entity type before this UI is upgraded.
   * Keep the runtime value so it can be rendered as plain text without
   * manufacturing a navigation target.
   */
  entityType: string
  entityId: string
  revision?: number
  label: string
  route?: string
}

export type AiEntityType =
  | 'lab'
  | 'user'
  | 'project'
  | 'membership'
  | 'cage'
  | 'animal'
  | 'animal_event'
  | 'gene_locus'
  | 'allele'
  | 'genotype'
  | 'genotype_definition'
  | 'genotyping_record'
  | 'breeding_line'
  | 'colony'
  | 'breeding_pair'
  | 'breeding_pair_member'
  | 'mating_event'
  | 'litter'
  | 'animal_draft'
  | 'pedigree'
  | 'experiment_event'
  | 'observation_definition'
  | 'observation'
  | 'observation_value'
  | 'experiment_template_version'
  | 'experiment'
  | 'cohort'
  | 'participation'
  | 'procedure'
  | 'measurement'
  | 'sample'
  | 'attachment'
  | 'project_animal_assignment'
  | 'ai_conversation'
  | 'ai_conversation_source'
  | 'tool_run'
  | 'approval'
  | 'job'

export type AiDraftKind =
  | 'ordinary_write'
  | 'measurement_result'
  | 'research_plan'
  | 'bulk_import'
  | 'bulk_measurement'
  | 'soft_delete'
  | 'permission_change'
  | 'migration'

export type AiApprovalRequirement =
  | 'preview_confirmation'
  | 'researcher_signature'
  | 'reinforced_confirmation'

export type AiDraftStatus =
  | 'pending_approval'
  | 'approved'
  | 'rejected'
  | 'applied'
  | 'cancelled'
  | 'expired'

export interface AiFieldChange {
  path: string
  before: unknown | null
  after: unknown | null
}

export interface AiImportPreviewRow {
  rowNumber: number
  animalId: string
  animalDisplayId: string
  measurementKey: string
  value: string
  unit?: string
  measuredAt: string
}

export interface AiImportPreviewIssue {
  row?: number
  field?: string
  severity: 'warning' | 'error'
  code: string
  message: string
}

export interface AiImportPreview {
  importKind: string
  projectId: string
  experimentId: string
  fileName: string
  sheetName: string
  totalRows: number
  acceptedRows: number
  issueCount: number
  issuesTruncated: boolean
  canConfirm: boolean
  previewRows: AiImportPreviewRow[]
  previewRowsTruncated: boolean
  issues: AiImportPreviewIssue[]
}

export interface AiWriteDraft {
  id: string
  kind: AiDraftKind
  projectId?: string
  changes: AiFieldChange[]
  importPreview?: AiImportPreview
  requirement: AiApprovalRequirement
  status: AiDraftStatus
  revision: number
  createdAt: string
  expiresAt: string
}

export interface AiToolRun {
  toolRunId: string
  providerCallId: string
  tool: string
  arguments: unknown
  outcome: 'read' | 'write_draft'
  citations: AiCitation[]
  draftId?: string
}

export interface AiAssistantTrace {
  providerId: string
  model: string
  usage: {
    providerCalls: number
    toolCalls: number
    inputTokens: number
    outputTokens: number
    totalTokens: number
  }
  context: {
    estimatedInputTokens: number
    inputTokenCountIsEstimate: boolean
    contextTrimmed: boolean
    trimmedHistoryTurns: number
    trimReasons: string[]
  }
  stages?: AiModelStageTrace[]
  imageEvidence?: AiImageEvidence[]
}

export interface AiModelStageTrace {
  profileId: string
  profileVersion: number
  purpose: 'vision_and_final' | 'vision_observation' | 'final_answer'
  modelId?: string
  inputTokens: number
  outputTokens: number
  totalTokens: number
  providerRequestId?: string
}

export interface AiImageEvidence {
  imageId: string
  sha256: string
  displayOrder: number
}

export interface AiTurnInput {
  conversationId: string
  projectId?: string
  message: string
  sourceRefs?: string[]
  imageIds?: string[]
  visionModelProfileId?: string
}

export interface StartAiConversationInput {
  projectId?: string
  title: string
  modelProfileId?: string
  requestedMode: AiAutonomyMode
  /** Server sessions only. LocalTauriGateway strips this before invoking Rust. */
  currentPassword?: string
}

export interface AiTurnResponse {
  conversationId: string
  content: string
  citations: AiCitation[]
  toolRuns: AiToolRun[]
  drafts: AiWriteDraft[]
  trace: AiAssistantTrace
  incompleteReason?:
    | 'iteration_limit_exceeded'
    | 'tool_call_limit_exceeded'
    | 'total_timeout_exceeded'
    | 'provider_failure'
    | 'tool_execution_failure'
  autonomy?: AiAutonomyView
}

export type AiAutonomyMode = 'ask' | 'auto' | 'full'

export interface AiAutonomyView {
  mode: AiAutonomyMode
  effectiveMode: AiAutonomyMode
  maxMode: AiAutonomyMode
  batchLimit: number
  revision: number
  expiresAt?: string
  requiresHumanApproval: string[]
}

export interface AiAutonomyUpdateInput {
  mode: AiAutonomyMode
  expectedRevision: number
  currentPassword?: string
  declared?: boolean
}

export interface AiConversationSummary {
  id: string
  projectId?: string
  title: string
  pinnedAt?: string
  archivedAt?: string
  modelProfileId?: string
  modelProfileVersion?: number
  modelProfileName?: string
  modelId?: string
  readOnly: boolean
  readOnlyReason?: 'legacy_model_unknown' | 'model_archived' | 'model_unavailable'
  createdAt: string
  updatedAt: string
  revision: number
}

export type AiConversationArchiveFilter = 'active' | 'archived' | 'all'

export interface AiConversationListInput {
  projectId?: string
  titleQuery?: string
  archive?: AiConversationArchiveFilter
  limit?: number
}

export type AiConversationAction = 'rename' | 'pin' | 'unpin' | 'archive' | 'unarchive'

export interface AiConversationUpdateInput {
  action: AiConversationAction
  expectedRevision: number
  title?: string
}

export type AiSourceStatus = 'staged' | 'ready' | 'archived' | 'failed' | 'expired'

export interface AiSource {
  id: string
  conversationId?: string
  projectId?: string
  fileName: string
  mediaType: string
  sizeBytes: number
  status: AiSourceStatus
  revision: number
  createdAt: string
  expiresAt: string
}

export interface AiSourceUploadInput {
  file: File
  conversationId: string
  projectId?: string
}

export interface AiSourceListInput {
  conversationId: string
  projectId?: string
  status?: AiSourceStatus
}

export interface AiSourceArchiveInput {
  projectId: string
  expectedRevision: number
}

export type AiComposerSourceStatus =
  | 'uploading'
  | 'staged'
  | 'ready'
  | 'archived'
  | 'failed'
  | 'expired'
  | 'error'

export interface AiComposerSource {
  clientId: string
  sourceId?: string
  projectId?: string
  fileName: string
  mediaType: string
  sizeBytes: number
  status: AiComposerSourceStatus
  revision?: number
  expiresAt?: string
  error?: string
  retryable?: boolean
}

export interface AiConversationSourceRef {
  sourceId: string
  sourceRevision: number
  fileName: string
  mediaType?: string
  sizeBytes: number
}

export interface StartAiConversationResponse {
  conversation: AiConversationSummary
  autonomy: AiAutonomyView
}

export interface AiConversationMessage {
  id: string
  sequence: number
  role: 'user' | 'assistant'
  content: string
  sourceRefs?: AiConversationSourceRef[]
  response?: AiTurnResponse
  createdAt: string
}

export interface AiConversationDetail {
  conversation: AiConversationSummary
  messages: AiConversationMessage[]
}

export interface AiDraftDecisionInput {
  expectedRevision: number
  decision: 'approve' | 'reject'
  statement?: string
  /** Server Session reinforced approval only; LocalTauriGateway always strips this field. */
  currentPassword?: string
}

export interface AiDraftDecisionResponse {
  draft: AiWriteDraft
  measurementId?: string
  jobId?: string
}

export interface AiMessage {
  id: string
  role: 'user' | 'assistant'
  content: string
  createdAt: string
  images?: Array<{
    id: string
    fileName: string
    previewHref: string
  }>
  citations?: AiCitation[]
  toolRuns?: AiToolRun[]
  drafts?: AiWriteDraft[]
  trace?: AiAssistantTrace
  incompleteReason?:
    | 'iteration_limit_exceeded'
    | 'tool_call_limit_exceeded'
    | 'total_timeout_exceeded'
    | 'provider_failure'
    | 'tool_execution_failure'
  pending?: boolean
  error?: boolean
  sources?: AiMessageSource[]
}

export interface AiMessageSource {
  sourceId?: string
  fileName: string
  mediaType: string
  sizeBytes: number
  released?: boolean
}
