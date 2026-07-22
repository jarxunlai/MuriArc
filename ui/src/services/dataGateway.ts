import { invoke } from '@tauri-apps/api/core'
import type { MuriArcGateway } from './gateway'
import { DemoGateway, LocalTauriGateway, RemoteHttpGateway } from './gateway'
import { activeProjectId } from './projectContext'

export const MAX_IMPORT_FILE_BYTES = 32 * 1024 * 1024
export type ImportKind = 'animal' | 'measurement'
export type AnimalImportTemplateFormat = 'csv' | 'xlsx'
export type AnimalImportTemplateVariant = 'blank' | 'example'

export interface ImportSelection {
  importKind: ImportKind
  experimentId?: string
}

export interface ImportFieldMapping {
  columns: Record<string, string>
}

export interface ImportIssue {
  row?: number | null
  field?: string | null
  severity: 'warning' | 'error'
  code: string
  message: string
}

export interface ImportPreview {
  importKind: ImportKind
  experimentId?: string | null
  jobId: string
  fileName: string
  sheetName: string
  headers: string[]
  mapping: ImportFieldMapping
  previewHash: string
  totalRows: number
  acceptedRows: number
  previewRows: Array<Record<string, string>>
  issues: ImportIssue[]
  canConfirm: boolean
}

export interface ImportReceipt {
  jobId: string
  previewHash: string
  committedAt: string
  replayed: boolean
  counts: {
    animals: number
    animalEvents: number
    genotypes: number
    pedigrees: number
    measurements: number
  }
}

export interface AnimalImportFieldSpec {
  key: string
  label: string
  data_type: 'string' | 'enum' | 'date' | 'reference' | 'canonical_genotype'
  required: boolean
  legal_values: string[]
  description: string
  example: string
}

export interface AnimalImportSchema {
  version: number
  fields: AnimalImportFieldSpec[]
  genotype_syntax: string
  examples: Array<Record<string, string>>
}

export interface DataArtifact {
  jobId: string
  kind: 'export' | 'snapshot'
  fileName: string
  mediaType: string
  sizeBytes: number
  sha256: string
  downloadUrl?: string
  bytes?: number[]
}

export type AnimalExportField =
  | 'identifier_scope'
  | 'project_name'
  | 'display_id'
  | 'sex'
  | 'birth_date'
  | 'registered_at'
  | 'strain'
  | 'status'
  | 'cage_location'
  | 'cage_section'
  | 'cage_display_id'
  | 'current_genotype_summary'

export interface AnimalExportOptions {
  filter: {
    sexes: Array<'male' | 'female' | 'unknown'>
    cage_locations: string[]
    cage_sections: string[]
    cage_display_ids: string[]
    strains: string[]
    statuses: Array<'planned' | 'alive' | 'in_experiment' | 'sampled' | 'deceased' | 'euthanized' | 'lost' | 'archived'>
    genotype_definitions: string[]
    genotyping_states: Array<'unknown' | 'expected' | 'confirmed' | 'rejected'>
    gene_loci: string[]
    alleles: string[]
    birth_date_from?: string
    birth_date_to?: string
    registered_at_from?: string
    registered_at_to?: string
    assessed_at_from?: string
    assessed_at_to?: string
  }
  fields: AnimalExportField[]
  include_genotype_details: boolean
}

export function defaultAnimalExportOptions(): AnimalExportOptions {
  return {
    filter: {
      sexes: [],
      cage_locations: [],
      cage_sections: [],
      cage_display_ids: [],
      strains: [],
      statuses: [],
      genotype_definitions: [],
      genotyping_states: [],
      gene_loci: [],
      alleles: [],
    },
    fields: [
      'identifier_scope',
      'project_name',
      'display_id',
      'sex',
      'birth_date',
      'registered_at',
      'strain',
      'status',
      'cage_location',
      'cage_section',
      'cage_display_id',
      'current_genotype_summary',
    ],
    include_genotype_details: true,
  }
}

function demoAnimalImportSchema(): AnimalImportSchema {
  const fields: AnimalImportFieldSpec[] = [
    { key: 'display_id', label: '动物显示编号', data_type: 'string', required: true, legal_values: [], description: '在当前编号 scope 内唯一；不能为空。', example: 'M-26001' },
    { key: 'sex', label: '性别', data_type: 'enum', required: false, legal_values: ['male', 'female', 'unknown'], description: '模板推荐使用标准英文值。', example: 'male' },
    { key: 'birth_date', label: '出生日期', data_type: 'date', required: false, legal_values: ['YYYY-MM-DD'], description: '推荐 ISO 日期格式。', example: '2026-07-01' },
    { key: 'strain', label: '品系', data_type: 'string', required: false, legal_values: [], description: '动物品系名称。', example: 'C57BL/6J' },
    { key: 'cage', label: '笼位', data_type: 'reference', required: false, legal_values: ['display_id', 'section/display_id'], description: '笼位必须已存在。', example: 'A/A03' },
    { key: 'genotype', label: '基因型', data_type: 'canonical_genotype', required: false, legal_values: ['{Locus}[allele_1]/[allele_2]&{AnotherLocus}[allele_1]/[allele_2]'], description: '位点、allele 和完全匹配的 Genetics v2 定义必须已存在。', example: '{Trp53}[+]/[flox]&{Cre}[Cre]/[+]' },
    { key: 'father', label: '父本', data_type: 'reference', required: false, legal_values: [], description: '父本显示编号。', example: 'M-25010' },
    { key: 'mother', label: '母本', data_type: 'reference', required: false, legal_values: [], description: '母本显示编号。', example: 'F-25011' },
  ]
  return {
    version: 1,
    fields,
    genotype_syntax: '{Locus}[allele_1]/[allele_2]&{AnotherLocus}[allele_1]/[allele_2]',
    examples: [
      {
        display_id: 'EXAMPLE-SIRE-001', sex: 'male', birth_date: '2025-10-01',
        strain: 'C57BL/6J', cage: '', genotype: '', father: '', mother: '',
      },
      {
        display_id: 'EXAMPLE-DAM-001', sex: 'female', birth_date: '2025-10-03',
        strain: 'C57BL/6J', cage: '', genotype: '', father: '', mother: '',
      },
      {
        display_id: 'EXAMPLE-PUP-001', sex: 'male', birth_date: '2026-01-05',
        strain: 'C57BL/6J', cage: '', genotype: '',
        father: 'EXAMPLE-SIRE-001', mother: 'EXAMPLE-DAM-001',
      },
      {
        display_id: 'EXAMPLE-PUP-002', sex: 'female', birth_date: '2026-01-05',
        strain: 'C57BL/6J', cage: '', genotype: '',
        father: 'EXAMPLE-SIRE-001', mother: 'EXAMPLE-DAM-001',
      },
    ],
  }
}

export interface MuriArcDataGateway {
  readonly animalImportTemplateFormats: readonly AnimalImportTemplateFormat[]
  getAnimalImportSchema(): Promise<AnimalImportSchema>
  downloadAnimalImportTemplate(
    format: AnimalImportTemplateFormat,
    variant?: AnimalImportTemplateVariant,
  ): Promise<void>
  previewImport(file: File, selection?: ImportSelection): Promise<ImportPreview>
  remapImport(previousJobId: string, mapping: ImportFieldMapping): Promise<ImportPreview>
  confirmImport(jobId: string, previewHash: string): Promise<ImportReceipt>
  cancelImport(jobId: string): Promise<void>
  createExport(format?: 'csv' | 'xlsx', projectId?: string, options?: AnimalExportOptions): Promise<DataArtifact>
  createSnapshot(): Promise<DataArtifact>
  downloadArtifact(artifact: DataArtifact): Promise<void>
}

type Invoke = <T>(command: string, args?: Record<string, unknown>) => Promise<T>

function idempotencyKey(prefix: string): string {
  return `${prefix}-${crypto.randomUUID()}`
}

function remapIdempotencyKey(
  cache: Map<string, string>,
  previousJobId: string,
  mapping: ImportFieldMapping,
): string {
  const fingerprint = `${previousJobId}:${stableMappingFingerprint(mapping)}`
  const existing = cache.get(fingerprint)
  if (existing) return existing
  const created = idempotencyKey('remap')
  cache.set(fingerprint, created)
  return created
}

function gatewayError(error: unknown): Error {
  if (error instanceof Error) return error
  if (typeof error === 'string') return new Error(error)
  if (error && typeof error === 'object') {
    const message = (error as Record<string, unknown>).message
    if (typeof message === 'string') return new Error(message)
  }
  return new Error('数据操作失败')
}

export class LocalDataGateway implements MuriArcDataGateway {
  readonly animalImportTemplateFormats = ['csv', 'xlsx'] as const
  private readonly remapIdempotency = new Map<string, string>()

  constructor(private readonly invokeCommand: Invoke = invoke) {}

  private async call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
    try {
      return await this.invokeCommand<T>(command, args)
    } catch (error) {
      throw gatewayError(error)
    }
  }

  getAnimalImportSchema() {
    return this.call<AnimalImportSchema>('get_animal_import_schema')
  }

  async downloadAnimalImportTemplate(
    format: AnimalImportTemplateFormat,
    variant: AnimalImportTemplateVariant = 'example',
  ) {
    const template = await this.call<{ fileName: string; mediaType: string; bytes: number[] }>(
      'get_animal_import_template',
      { input: { format, variant } },
    )
    saveBlob(template.fileName, template.mediaType, new Uint8Array(template.bytes))
  }

  async previewImport(file: File, selection: ImportSelection = { importKind: 'animal' }): Promise<ImportPreview> {
    validateFile(file)
    validateImportSelection(selection)
    const bytes = Array.from(await readFileBytes(file))
    return this.call<ImportPreview>('preview_data_import', {
      input: {
        fileName: file.name,
        bytes,
        idempotencyKey: idempotencyKey('import'),
        importKind: selection.importKind,
        experimentId: selection.experimentId,
      },
    })
  }

  remapImport(previousJobId: string, mapping: ImportFieldMapping) {
    return this.call<ImportPreview>('remap_data_import', {
      input: {
        jobId: previousJobId,
        mapping,
        idempotencyKey: remapIdempotencyKey(this.remapIdempotency, previousJobId, mapping),
      },
    })
  }

  confirmImport(jobId: string, previewHash: string) {
    return this.call<ImportReceipt>('confirm_data_import', { input: { jobId, previewHash } })
  }

  cancelImport(jobId: string) {
    return this.call<void>('cancel_data_import', { input: { jobId } })
  }

  createExport(format: 'csv' | 'xlsx' = 'xlsx', projectId?: string, options?: AnimalExportOptions) {
    return this.call<DataArtifact>('create_data_export', {
      input: {
        format,
        idempotencyKey: idempotencyKey('export'),
        projectId: activeProjectId(projectId),
        options: options ?? defaultAnimalExportOptions(),
      },
    })
  }

  createSnapshot() {
    return this.call<DataArtifact>('create_data_snapshot', {
      input: { idempotencyKey: idempotencyKey('snapshot') },
    })
  }

  async downloadArtifact(artifact: DataArtifact) {
    const resolved = artifact.bytes
      ? artifact
      : await this.call<DataArtifact>('read_data_artifact', { jobId: artifact.jobId })
    if (!resolved.bytes) throw new Error('本地结果文件不可用')
    saveBlob(resolved.fileName, resolved.mediaType, new Uint8Array(resolved.bytes))
  }
}

interface ApiItem<T> { data: T; request_id: string }
interface RawCsrf { csrf_token: string; expires_at: string }

export interface RemoteDataGatewayOptions {
  baseUrl?: string
  fetch?: typeof globalThis.fetch
}

export class RemoteDataGateway implements MuriArcDataGateway {
  readonly animalImportTemplateFormats = ['csv', 'xlsx'] as const
  private readonly baseUrl: string
  private readonly fetchRequest: typeof globalThis.fetch
  private csrfToken?: string
  private readonly remapIdempotency = new Map<string, string>()

  constructor(options: RemoteDataGatewayOptions = {}) {
    this.baseUrl = (options.baseUrl ?? import.meta.env.VITE_MURIARC_API_BASE ?? '/api/v1').replace(/\/$/, '')
    this.fetchRequest = options.fetch ?? globalThis.fetch.bind(globalThis)
  }

  private async ensureCsrf(): Promise<string> {
    if (this.csrfToken) return this.csrfToken
    const response = await this.fetchRequest(`${this.baseUrl}/auth/csrf`, {
      credentials: 'include',
      headers: { Accept: 'application/json' },
    })
    const payload = await response.json().catch(() => undefined) as ApiItem<RawCsrf> | undefined
    if (!response.ok || !payload?.data?.csrf_token) {
      throw new Error(payload ? '无法恢复数据操作的 CSRF 凭据' : 'Server 返回了无效响应')
    }
    this.csrfToken = payload.data.csrf_token
    return this.csrfToken
  }

  private async json<T>(path: string, init: RequestInit = {}, csrf = true): Promise<T> {
    const headers = new Headers(init.headers)
    headers.set('Accept', 'application/json')
    if (init.body && !(init.body instanceof Blob)) headers.set('Content-Type', 'application/json')
    if (csrf) headers.set('X-CSRF-Token', await this.ensureCsrf())
    const response = await this.fetchRequest(`${this.baseUrl}${path}`, {
      ...init,
      credentials: 'include',
      headers,
    })
    const payload = await response.json().catch(() => undefined) as
      | ApiItem<T>
      | { error?: { message?: string } }
      | undefined
    if (!response.ok) {
      if (response.status === 401 || response.status === 403) this.csrfToken = undefined
      const message = payload && 'error' in payload ? payload.error?.message : undefined
      throw new Error(message ?? `Server 数据请求失败（${response.status}）`)
    }
    return (payload as ApiItem<T>).data
  }

  getAnimalImportSchema() {
    return this.json<AnimalImportSchema>('/data/animal-import/schema', {}, false)
  }

  async downloadAnimalImportTemplate(
    format: AnimalImportTemplateFormat,
    variant: AnimalImportTemplateVariant = 'example',
  ) {
    const query = new URLSearchParams({ format, variant })
    const response = await this.fetchRequest(
      `${this.baseUrl}/data/animal-import/template?${query}`,
      { credentials: 'include' },
    )
    if (!response.ok) throw new Error(`无法下载动物导入模板（${response.status}）`)
    saveBlob(
      animalImportTemplateFileName(format, variant),
      format === 'csv'
        ? 'text/csv;charset=utf-8'
        : 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
      await response.blob(),
    )
  }

  async previewImport(file: File, selection: ImportSelection = { importKind: 'animal' }): Promise<ImportPreview> {
    validateFile(file)
    validateImportSelection(selection)
    const query = new URLSearchParams({
      file_name: file.name,
      idempotency_key: idempotencyKey('import'),
      import_kind: selection.importKind,
    })
    if (selection.experimentId) query.set('experiment_id', selection.experimentId)
    return this.json<ImportPreview>(`/data/imports?${query}`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/octet-stream' },
      body: file,
    })
  }

  remapImport(previousJobId: string, mapping: ImportFieldMapping) {
    return this.json<ImportPreview>(`/data/imports/${encodeURIComponent(previousJobId)}/remap`, {
      method: 'POST',
      body: JSON.stringify({
        mapping,
        idempotency_key: remapIdempotencyKey(this.remapIdempotency, previousJobId, mapping),
      }),
    })
  }

  confirmImport(jobId: string, previewHash: string) {
    return this.json<ImportReceipt>(`/data/imports/${encodeURIComponent(jobId)}/confirm`, {
      method: 'POST',
      body: JSON.stringify({ preview_hash: previewHash }),
    })
  }

  cancelImport(jobId: string) {
    return this.json<void>(`/data/imports/${encodeURIComponent(jobId)}/cancel`, { method: 'POST' })
  }

  createExport(format: 'csv' | 'xlsx' = 'xlsx', projectId?: string, options?: AnimalExportOptions) {
    return this.json<DataArtifact>('/data/exports', {
      method: 'POST',
      body: JSON.stringify({
        format,
        idempotency_key: idempotencyKey('export'),
        project_id: activeProjectId(projectId) ?? null,
        options: options ?? defaultAnimalExportOptions(),
      }),
    })
  }

  createSnapshot() {
    return this.json<DataArtifact>('/data/snapshots', {
      method: 'POST',
      body: JSON.stringify({ idempotency_key: idempotencyKey('snapshot') }),
    })
  }

  async downloadArtifact(artifact: DataArtifact) {
    const path = artifact.downloadUrl ?? `/data/artifacts/${encodeURIComponent(artifact.jobId)}`
    const response = await this.fetchRequest(`${this.baseUrl}${path}`, {
      credentials: 'include',
      headers: { Accept: artifact.mediaType },
    })
    if (!response.ok) throw new Error(`无法下载结果文件（${response.status}）`)
    saveBlob(artifact.fileName, artifact.mediaType, await response.blob())
  }
}

export class DemoDataGateway implements MuriArcDataGateway {
  readonly animalImportTemplateFormats = ['csv'] as const
  private pending = new Map<string, DemoPendingImport>()

  constructor(private readonly domainGateway: MuriArcGateway) {}

  async getAnimalImportSchema() {
    return demoAnimalImportSchema()
  }

  async downloadAnimalImportTemplate(
    format: AnimalImportTemplateFormat,
    variant: AnimalImportTemplateVariant = 'example',
  ) {
    if (format === 'xlsx') throw new Error('浏览器演示模式仅提供 CSV 模板')
    const schema = demoAnimalImportSchema()
    const headers = schema.fields.map((field) => field.key)
    const rows = variant === 'example'
      ? schema.examples.map((example) => headers.map((header) => example[header] ?? ''))
      : []
    const encode = (value: string) => `"${value.replaceAll('"', '""')}"`
    const bytes = new TextEncoder().encode([
      headers.join(','),
      ...rows.map((row) => row.map(encode).join(',')),
    ].join('\n'))
    saveBlob(animalImportTemplateFileName(format, variant), 'text/csv;charset=utf-8', bytes)
  }

  async previewImport(file: File, selection: ImportSelection = { importKind: 'animal' }): Promise<ImportPreview> {
    validateFile(file)
    validateImportSelection(selection)
    if (!file.name.toLowerCase().endsWith('.csv')) throw new Error('浏览器演示模式仅解析 CSV；正式版本支持 XLSX')
    const sourceText = await readFileText(file)
    return this.createPreview(file.name, sourceText, { ...selection })
  }

  async remapImport(previousJobId: string, mapping: ImportFieldMapping): Promise<ImportPreview> {
    const previous = this.pending.get(previousJobId)
    if (!previous) throw new Error('原导入预览已失效，请重新选择文件')
    const replacement = await this.createPreview(
      previous.fileName,
      previous.sourceText,
      { ...previous.selection },
      mapping,
    )
    this.pending.delete(previousJobId)
    return replacement
  }

  async confirmImport(jobId: string, previewHash: string): Promise<ImportReceipt> {
    const pending = this.pending.get(jobId)
    const preview = pending?.preview
    if (!preview || preview.previewHash !== previewHash) throw new Error('预览已失效，请重新选择文件')
    if (!preview.canConfirm) throw new Error('仍有阻断错误，不能确认导入')
    this.pending.delete(jobId)
    return {
      jobId,
      previewHash,
      committedAt: new Date().toISOString(),
      replayed: false,
      counts: preview.importKind === 'measurement'
        ? { animals: 0, animalEvents: 0, genotypes: 0, pedigrees: 0, measurements: preview.acceptedRows }
        : { animals: preview.acceptedRows, animalEvents: preview.acceptedRows, genotypes: 0, pedigrees: 0, measurements: 0 },
    }
  }

  async cancelImport(jobId: string) { this.pending.delete(jobId) }

  async createExport(format: 'csv' | 'xlsx' = 'xlsx', projectId?: string, options?: AnimalExportOptions): Promise<DataArtifact> {
    const scopedProjectId = activeProjectId(projectId)
    const animals = await this.domainGateway.listAnimals(
      scopedProjectId ? { projectId: scopedProjectId } : undefined,
    )
    const selected = options ?? defaultAnimalExportOptions()
    const filtered = animals.filter((animal) =>
      (!selected.filter.sexes.length || selected.filter.sexes.includes(animal.sex))
      && (!selected.filter.strains.length || selected.filter.strains.some((strain) =>
        strain.localeCompare(animal.strain, undefined, { sensitivity: 'accent' }) === 0)))
    const requestedFields = new Set<AnimalExportField>([
      'identifier_scope', 'project_name', 'display_id', ...selected.fields,
    ])
    const orderedFields = defaultAnimalExportOptions().fields.filter((field) => requestedFields.has(field))
    const value = (animal: (typeof animals)[number], field: AnimalExportField) => {
      if (field === 'identifier_scope') return scopedProjectId ? 'project' : 'lab'
      if (field === 'project_name') return animal.projectNames?.join('、') ?? ''
      if (field === 'display_id') return animal.code
      if (field === 'sex') return animal.sex
      if (field === 'birth_date') return animal.birthDate ?? ''
      if (field === 'strain') return animal.strain ?? ''
      if (field === 'status') return animal.status ?? ''
      if (field === 'cage_display_id') return animal.cageId ?? ''
      if (field === 'current_genotype_summary') return animal.genotype ?? ''
      return ''
    }
    const escape = (cell: string) => `"${cell.replaceAll('"', '""')}"`
    const text = [orderedFields.join(','), ...filtered.map((animal) =>
      orderedFields.map((field) => escape(value(animal, field) ?? '')).join(','))].join('\n')
    const bytes = Array.from(new TextEncoder().encode(text))
    return demoArtifact('export', format === 'csv' ? 'animals.csv' : 'animals-demo.csv', 'text/csv;charset=utf-8', bytes)
  }

  async createSnapshot(): Promise<DataArtifact> {
    const animals = await this.domainGateway.listAnimals()
    const bytes = Array.from(new TextEncoder().encode(JSON.stringify({ product: 'MuriArc', demo: true, animals }, null, 2)))
    return demoArtifact('snapshot', 'muriarc-demo-snapshot.json', 'application/json', bytes)
  }

  async downloadArtifact(artifact: DataArtifact) {
    if (!artifact.bytes) throw new Error('演示结果不可用')
    saveBlob(artifact.fileName, artifact.mediaType, new Uint8Array(artifact.bytes))
  }

  private async createPreview(
    fileName: string,
    sourceText: string,
    selection: ImportSelection,
    explicitMapping?: ImportFieldMapping,
  ): Promise<ImportPreview> {
    // Parse again on every preview, including a remap. Demo mode must not
    // pretend that changing a select box is equivalent to backend validation.
    const table = parseDemoCsv(sourceText)
    const animals = await this.domainGateway.listAnimals()
    const columns = explicitMapping
      ? { ...explicitMapping.columns }
      : inferDemoColumns(selection.importKind, table.headers)
    const mapping = { columns }
    const issues = validateDemoMapping(selection.importKind, table.headers, columns)
    let acceptedRows = 0

    if (!hasGlobalImportError(issues)) {
      if (selection.importKind === 'animal') {
        acceptedRows = previewDemoAnimalRows(table, columns, animals, issues)
      } else {
        acceptedRows = previewDemoMeasurementRows(table, columns, animals, issues)
      }
    }
    const jobId = crypto.randomUUID()
    const preview: ImportPreview = {
      importKind: selection.importKind,
      experimentId: selection.importKind === 'measurement' ? selection.experimentId! : null,
      jobId,
      fileName,
      sheetName: 'csv',
      headers: table.headers,
      mapping,
      previewHash: await sha256Text([
        selection.importKind,
        selection.experimentId ?? '',
        sourceText,
        stableMappingFingerprint(mapping),
      ].join('\n')),
      totalRows: table.rows.length,
      acceptedRows,
      previewRows: selection.importKind === 'animal'
        ? table.rows.slice(0, 20).map((row) => Object.fromEntries(
            Object.entries(columns).map(([target, source]) => [
              target,
              row[table.headers.indexOf(source)] ?? '',
            ]),
          ))
        : [],
      issues,
      canConfirm: !issues.some((issue) => issue.severity === 'error'),
    }
    this.pending.set(jobId, {
      preview,
      fileName,
      sourceText,
      selection: { ...selection },
    })
    return preview
  }
}

interface DemoPendingImport {
  preview: ImportPreview
  fileName: string
  sourceText: string
  selection: ImportSelection
}

interface DemoTable {
  headers: string[]
  rows: string[][]
}

const DEMO_ANIMAL_TARGETS = new Set([
  'display_id', 'sex', 'birth_date', 'strain', 'cage', 'genotype', 'father', 'mother',
])
const DEMO_MEASUREMENT_TARGETS = new Set([
  'animal_uuid', 'display_id', 'measurement_key', 'value_type', 'value', 'unit', 'measured_at',
])
const DEMO_MEASUREMENT_REQUIRED = ['measurement_key', 'value_type', 'value', 'unit', 'measured_at']

function inferDemoColumns(kind: ImportKind, headers: string[]): Record<string, string> {
  const patterns: Record<string, RegExp> = kind === 'animal'
    ? {
        display_id: /^(mouse[ _-]?id|display[ _-]?id|id|小鼠id|小鼠编号|编号)$/i,
        sex: /^(sex|gender|性别)$/i,
        birth_date: /^(birth[ _-]?date|birthday|dob|出生日期)$/i,
        strain: /^(strain|品系)$/i,
        cage: /^(cage|cage[ _-]?id|笼位|笼位id)$/i,
        genotype: /^(genotype|基因型)$/i,
        father: /^(father|sire|父本)$/i,
        mother: /^(mother|dam|母本)$/i,
      }
    : {
        animal_uuid: /^(animal[ _-]?uuid|mouse[ _-]?uuid|动物uuid|小鼠uuid)$/i,
        display_id: /^(display[ _-]?id|animal[ _-]?id|mouse[ _-]?id|id|小鼠id|小鼠编号|动物编号|编号)$/i,
        measurement_key: /^(measurement[ _-]?key|measurement|metric|指标|测量指标)$/i,
        value_type: /^(value[ _-]?type|type|数据类型|值类型)$/i,
        value: /^(value|result|测量值|结果)$/i,
        unit: /^(unit|units|单位)$/i,
        measured_at: /^(measured[ _-]?at|measurement[ _-]?time|datetime|timestamp|测量时间|时间|日期)$/i,
      }
  const columns: Record<string, string> = {}
  for (const [target, pattern] of Object.entries(patterns)) {
    const source = headers.find((header) => pattern.test(header.trim()))
    if (source) columns[target] = source
  }
  return columns
}

function validateDemoMapping(
  kind: ImportKind,
  headers: string[],
  columns: Record<string, string>,
): ImportIssue[] {
  const issues: ImportIssue[] = []
  const allowed = kind === 'animal' ? DEMO_ANIMAL_TARGETS : DEMO_MEASUREMENT_TARGETS
  const counts = new Map<string, number>()
  for (const header of headers) counts.set(header, (counts.get(header) ?? 0) + 1)
  const targetsBySource = new Map<string, string[]>()
  for (const [target, source] of Object.entries(columns)) {
    if (!allowed.has(target)) {
      issues.push({ severity: 'error', code: 'unknown_mapping_target', field: target, message: `不支持映射到字段 ${target}` })
      continue
    }
    const count = counts.get(source) ?? 0
    if (!count) issues.push({ severity: 'error', code: 'unknown_source_column', field: target, message: `找不到映射列 ${source}` })
    else if (count > 1) issues.push({ severity: 'error', code: 'duplicate_source_column', field: target, message: `源字段 ${source} 在表头中重复，无法安全映射` })
    const targets = targetsBySource.get(source) ?? []
    targets.push(target)
    targetsBySource.set(source, targets)
  }
  for (const [source, targets] of targetsBySource) {
    if (targets.length > 1) {
      issues.push({ severity: 'error', code: 'duplicate_source_mapping', message: `源字段 ${source} 不能同时映射到 ${targets.join('、')}` })
    }
  }
  if (kind === 'animal') {
    if (!columns.display_id) issues.push({ severity: 'error', code: 'missing_required_mapping', field: 'display_id', message: '必须映射小鼠编号字段' })
  } else {
    for (const required of DEMO_MEASUREMENT_REQUIRED) {
      if (!columns[required]) issues.push({ severity: 'error', code: 'missing_required_mapping', field: required, message: `必须映射测量字段 ${required}` })
    }
    if (!columns.animal_uuid && !columns.display_id) {
      issues.push({ severity: 'error', code: 'missing_animal_identity_mapping', field: 'animal_identity', message: '必须映射 animal_uuid 或动物显示编号' })
    }
  }
  return issues
}

function previewDemoAnimalRows(
  table: DemoTable,
  columns: Record<string, string>,
  animals: Awaited<ReturnType<MuriArcGateway['listAnimals']>>,
  issues: ImportIssue[],
): number {
  const displayIndex = table.headers.indexOf(columns.display_id)
  const existing = new Set(animals.map((animal) => animal.code))
  const seen = new Set<string>()
  let accepted = 0
  table.rows.forEach((row, index) => {
    if (row.every((value) => !value.trim())) return
    const sourceRow = index + 2
    const code = row[displayIndex]?.trim()
    if (!code) issues.push({ row: sourceRow, field: 'display_id', severity: 'error', code: 'missing_display_id', message: '小鼠编号不能为空' })
    else if (seen.has(code)) issues.push({ row: sourceRow, field: 'display_id', severity: 'error', code: 'duplicate_in_file', message: '文件内小鼠编号重复' })
    else if (existing.has(code)) issues.push({ row: sourceRow, field: 'display_id', severity: 'error', code: 'existing_display_id', message: `动物编号 ${code} 已存在` })
    else accepted += 1
    if (code) seen.add(code)
  })
  return accepted
}

function previewDemoMeasurementRows(
  table: DemoTable,
  columns: Record<string, string>,
  animals: Awaited<ReturnType<MuriArcGateway['listAnimals']>>,
  issues: ImportIssue[],
): number {
  const indexes = Object.fromEntries(
    Object.entries(columns).map(([target, source]) => [target, table.headers.indexOf(source)]),
  )
  const animalByUuid = new Map(animals.map((animal) => [animal.id, animal]))
  const animalsByDisplayId = new Map<string, typeof animals>()
  for (const animal of animals) {
    const values = animalsByDisplayId.get(animal.code) ?? []
    values.push(animal)
    animalsByDisplayId.set(animal.code, values)
  }
  let accepted = 0
  table.rows.forEach((row, index) => {
    if (row.every((value) => !value.trim())) return
    const sourceRow = index + 2
    const uuid = indexes.animal_uuid == null ? '' : row[indexes.animal_uuid]?.trim()
    const displayId = indexes.display_id == null ? '' : row[indexes.display_id]?.trim()
    if (uuid) {
      if (!animalByUuid.has(uuid)) issues.push({ row: sourceRow, field: 'animal_uuid', severity: 'error', code: 'unknown_animal_uuid', message: '动物 UUID 不属于当前可用动物目录' })
    } else if (!displayId) {
      issues.push({ row: sourceRow, field: 'display_id', severity: 'error', code: 'missing_animal_identity', message: 'animal_uuid 和动物显示编号不能同时为空' })
    } else {
      const matches = animalsByDisplayId.get(displayId) ?? []
      if (!matches.length) issues.push({ row: sourceRow, field: 'display_id', severity: 'error', code: 'unknown_animal', message: '动物编号在当前目录中不存在' })
      else if (matches.length > 1) issues.push({ row: sourceRow, field: 'display_id', severity: 'error', code: 'ambiguous_animal', message: '动物编号对应多个动物，必须提供 animal_uuid' })
    }
    for (const required of DEMO_MEASUREMENT_REQUIRED) {
      const value = row[indexes[required]]?.trim()
      if (!value) issues.push({ row: sourceRow, field: required, severity: 'error', code: `missing_${required}`, message: `测量字段 ${required} 不能为空` })
    }
    if (!issues.some((issue) => issue.row === sourceRow && issue.severity === 'error')) accepted += 1
  })
  return accepted
}

function hasGlobalImportError(issues: ImportIssue[]): boolean {
  return issues.some((issue) => issue.severity === 'error' && issue.row == null)
}

function stableMappingFingerprint(mapping: ImportFieldMapping): string {
  return JSON.stringify(Object.entries(mapping.columns).sort(([left], [right]) => left.localeCompare(right)))
}

function readFileBytes(file: File): Promise<Uint8Array> {
  if (typeof file.arrayBuffer === 'function') {
    return file.arrayBuffer().then((buffer) => new Uint8Array(buffer))
  }
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onerror = () => reject(reader.error ?? new Error('无法读取文件'))
    reader.onload = () => resolve(new Uint8Array(reader.result as ArrayBuffer))
    reader.readAsArrayBuffer(file)
  })
}

function readFileText(file: File): Promise<string> {
  if (typeof file.text === 'function') return file.text()
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onerror = () => reject(reader.error ?? new Error('无法读取文件'))
    reader.onload = () => resolve(String(reader.result ?? ''))
    reader.readAsText(file)
  })
}

function validateFile(file: File) {
  if (
    !file.name
    || file.name.length > 255
    || /[\\/\u0000-\u001f\u007f]/.test(file.name)
  ) {
    throw new Error('文件名无效，不能包含路径或控制字符')
  }
  const lower = file.name.toLowerCase()
  if (!lower.endsWith('.csv') && !lower.endsWith('.xlsx')) throw new Error('仅支持 CSV 与 XLSX 文件')
  if (file.size === 0) throw new Error('文件为空')
  if (file.size > MAX_IMPORT_FILE_BYTES) throw new Error(`文件超过 ${MAX_IMPORT_FILE_BYTES / 1024 / 1024} MiB 限制`)
}

function validateImportSelection(selection: ImportSelection) {
  if (selection.importKind === 'measurement' && !selection.experimentId?.trim()) {
    throw new Error('测量数据导入必须选择所属实验')
  }
  if (selection.importKind === 'animal' && selection.experimentId) {
    throw new Error('动物登记导入不能指定实验')
  }
}

function animalImportTemplateFileName(
  format: AnimalImportTemplateFormat,
  variant: AnimalImportTemplateVariant,
) {
  return `muriarc-animal-import${variant === 'blank' ? '-blank' : ''}.${format}`
}

function saveBlob(fileName: string, mediaType: string, content: BlobPart) {
  const blob = content instanceof Blob ? content : new Blob([content], { type: mediaType })
  const url = URL.createObjectURL(blob)
  const link = document.createElement('a')
  link.href = url
  link.download = fileName
  link.click()
  URL.revokeObjectURL(url)
}

function parseDemoCsv(text: string): { headers: string[]; rows: string[][] } {
  const lines = text.replace(/^\uFEFF/, '').split(/\r?\n/).filter((line) => line.trim())
  if (!lines.length) throw new Error('CSV 缺少表头')
  const parse = (line: string) => line.split(',').map((value) => value.trim().replace(/^"|"$/g, ''))
  return { headers: parse(lines[0]), rows: lines.slice(1).map(parse) }
}

async function sha256Text(value: string): Promise<string> {
  const bytes = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(value))
  return Array.from(new Uint8Array(bytes), (byte) => byte.toString(16).padStart(2, '0')).join('')
}

async function demoArtifact(kind: DataArtifact['kind'], fileName: string, mediaType: string, bytes: number[]): Promise<DataArtifact> {
  return {
    jobId: crypto.randomUUID(),
    kind,
    fileName,
    mediaType,
    sizeBytes: bytes.length,
    sha256: await sha256Text(new TextDecoder().decode(new Uint8Array(bytes))),
    bytes,
  }
}

export function createDataGateway(domainGateway: MuriArcGateway): MuriArcDataGateway {
  if (domainGateway instanceof LocalTauriGateway) return new LocalDataGateway()
  if (domainGateway instanceof RemoteHttpGateway) return new RemoteDataGateway()
  if (domainGateway instanceof DemoGateway) return new DemoDataGateway(domainGateway)
  throw new Error('无法识别 MuriArc 数据 transport')
}
