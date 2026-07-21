import { afterEach, describe, expect, it, vi } from 'vitest'
import type { MuriArcGateway } from './gateway'
import {
  DemoDataGateway,
  LocalDataGateway,
  MAX_IMPORT_FILE_BYTES,
  RemoteDataGateway,
  type DataArtifact,
  type ImportPreview,
} from './dataGateway'

const PREVIEW_HASH = 'a'.repeat(64)

const validPreview: ImportPreview = {
  importKind: 'animal',
  experimentId: null,
  jobId: 'job-1',
  fileName: 'animals.csv',
  sheetName: 'csv',
  headers: ['display_id'],
  mapping: { columns: { display_id: 'display_id' } },
  previewHash: PREVIEW_HASH,
  totalRows: 1,
  acceptedRows: 1,
  previewRows: [{ display_id: 'M001' }],
  issues: [],
  canConfirm: true,
}

function apiResponse(data: unknown, status = 200): Response {
  return new Response(JSON.stringify({ data, request_id: 'req-1' }), {
    status,
    headers: { 'Content-Type': 'application/json' },
  })
}

function domainGatewayWithAnimals(
  animals: Array<{ id: string; code: string; sex: 'male' | 'female' | 'unknown' }>,
): MuriArcGateway {
  return {
    listAnimals: vi.fn().mockResolvedValue(animals),
  } as unknown as MuriArcGateway
}

afterEach(() => {
  vi.restoreAllMocks()
})

describe('LocalDataGateway', () => {
  it('uses byte-only camelCase DTOs for every Tauri data command', async () => {
    const calls: Array<[string, Record<string, unknown> | undefined]> = []
    const artifact: DataArtifact = {
      jobId: 'artifact-1', kind: 'export', fileName: 'animals.csv', mediaType: 'text/csv',
      sizeBytes: 1, sha256: 'b'.repeat(64), bytes: [65],
    }
    const invokeCommand = async <T>(command: string, args?: Record<string, unknown>): Promise<T> => {
      calls.push([command, args])
      if (command === 'preview_data_import') return validPreview as T
      if (command === 'confirm_data_import') {
        return {
          jobId: 'job-1', previewHash: PREVIEW_HASH, committedAt: '2026-07-19T00:00:00Z',
          replayed: false,
          counts: { animals: 1, animalEvents: 1, genotypes: 0, pedigrees: 0, measurements: 0 },
        } as T
      }
      if (command === 'create_data_export' || command === 'create_data_snapshot') return artifact as T
      return undefined as T
    }
    const gateway = new LocalDataGateway(invokeCommand)
    const file = new File(['display_id\nM001\n'], 'animals.csv', { type: 'text/csv' })

    const preview = await gateway.previewImport(file)
    await gateway.confirmImport(preview.jobId, preview.previewHash)
    await gateway.cancelImport(preview.jobId)
    await gateway.createExport('csv')
    await gateway.createSnapshot()

    const previewInput = calls[0][1]?.input as Record<string, unknown>
    expect(calls[0][0]).toBe('preview_data_import')
    expect(previewInput).toEqual({
      fileName: 'animals.csv',
      bytes: Array.from(new TextEncoder().encode('display_id\nM001\n')),
      idempotencyKey: expect.stringMatching(/^import-[0-9a-f-]+$/),
      importKind: 'animal',
      experimentId: undefined,
    })
    expect(previewInput).not.toHaveProperty('path')
    expect(previewInput).not.toHaveProperty('filePath')
    expect(calls[1]).toEqual(['confirm_data_import', { input: { jobId: 'job-1', previewHash: PREVIEW_HASH } }])
    expect(calls[2]).toEqual(['cancel_data_import', { input: { jobId: 'job-1' } }])
    expect(calls[3]).toEqual([
      'create_data_export',
      { input: expect.objectContaining({
        format: 'csv',
        projectId: undefined,
        idempotencyKey: expect.stringMatching(/^export-[0-9a-f-]+$/),
        options: expect.objectContaining({ include_genotype_details: true }),
      }) },
    ])
    expect(calls[4]).toEqual([
      'create_data_snapshot',
      { input: { idempotencyKey: expect.stringMatching(/^snapshot-[0-9a-f-]+$/) } },
    ])
  })

  it('passes an explicit experiment scope for measurement imports', async () => {
    const invokeCommand = vi.fn().mockResolvedValue({
      ...validPreview,
      importKind: 'measurement',
      experimentId: 'experiment-1',
      mapping: { columns: { display_id: 'display_id', measurement_key: 'measurement_key' } },
    })
    const gateway = new LocalDataGateway(invokeCommand)
    const file = new File([
      'display_id,measurement_key,value_type,value,unit,measured_at\nM001,body_weight,number,22.1,g,2026-07-19\n',
    ], 'measurements.csv')

    await gateway.previewImport(file, { importKind: 'measurement', experimentId: 'experiment-1' })

    expect(invokeCommand).toHaveBeenCalledWith('preview_data_import', {
      input: expect.objectContaining({
        importKind: 'measurement',
        experimentId: 'experiment-1',
      }),
    })
  })

  it('sends a canonical mapping to a dedicated Tauri remap command', async () => {
    const invokeCommand = vi.fn().mockResolvedValue({ ...validPreview, jobId: 'job-2' })
    const gateway = new LocalDataGateway(invokeCommand)

    const preview = await gateway.remapImport('job-1', {
      columns: { display_id: 'custom_code', sex: 'gender' },
    })

    expect(preview.jobId).toBe('job-2')
    expect(invokeCommand).toHaveBeenCalledWith('remap_data_import', {
      input: {
        jobId: 'job-1',
        mapping: { columns: { display_id: 'custom_code', sex: 'gender' } },
        idempotencyKey: expect.stringMatching(/^remap-[0-9a-f-]+$/),
      },
    })
  })

  it('loads an artifact by job id when Tauri did not inline its bytes', async () => {
    const artifact: DataArtifact = {
      jobId: 'artifact-1', kind: 'export', fileName: 'animals.csv', mediaType: 'text/csv',
      sizeBytes: 1, sha256: 'b'.repeat(64),
    }
    const invokeCommand = vi.fn().mockResolvedValue({ ...artifact, bytes: [65] })
    const click = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => undefined)
    const createObjectURL = vi.fn().mockReturnValue('blob:muriarc-test')
    const revokeObjectURL = vi.fn()
    Object.defineProperty(URL, 'createObjectURL', { configurable: true, value: createObjectURL })
    Object.defineProperty(URL, 'revokeObjectURL', { configurable: true, value: revokeObjectURL })

    await new LocalDataGateway(invokeCommand).downloadArtifact(artifact)

    expect(invokeCommand).toHaveBeenCalledWith('read_data_artifact', { jobId: 'artifact-1' })
    expect(createObjectURL).toHaveBeenCalledOnce()
    expect(click).toHaveBeenCalledOnce()
    expect(revokeObjectURL).toHaveBeenCalledWith('blob:muriarc-test')
  })

  it('normalizes non-Error failures returned by the Tauri bridge', async () => {
    const gateway = new LocalDataGateway(vi.fn().mockRejectedValue({ message: '本地数据库忙' }))
    await expect(gateway.createSnapshot()).rejects.toThrow('本地数据库忙')
  })
})

describe('RemoteDataGateway', () => {
  it('streams the original File body and reuses an in-memory cookie CSRF token', async () => {
    const requests: Array<{ url: string; init?: RequestInit }> = []
    const fetchRequest = vi.fn(async (input: string | URL | Request, init?: RequestInit) => {
      const url = String(input)
      requests.push({ url, init })
      if (url.endsWith('/auth/csrf')) {
        return apiResponse({ csrf_token: 'csrf-memory', expires_at: '2026-07-19T00:00:00Z' })
      }
      if (url.includes('/confirm')) {
        return apiResponse({
          jobId: 'job-1', previewHash: PREVIEW_HASH, committedAt: '2026-07-19T00:00:00Z',
          replayed: false,
          counts: { animals: 1, animalEvents: 1, genotypes: 0, pedigrees: 0, measurements: 0 },
        })
      }
      return apiResponse(validPreview)
    }) as unknown as typeof fetch
    const gateway = new RemoteDataGateway({ baseUrl: 'https://lab.example/api/v1/', fetch: fetchRequest })
    const file = new File(['display_id\nM001\n'], 'animals #1.csv', { type: 'text/csv' })

    const preview = await gateway.previewImport(file)
    await gateway.confirmImport(preview.jobId, preview.previewHash)

    expect(requests.filter(({ url }) => url.endsWith('/auth/csrf'))).toHaveLength(1)
    const upload = requests.find(({ url }) => url.includes('/data/imports?'))
    expect(upload).toBeDefined()
    const uploadUrl = new URL(upload!.url)
    expect(uploadUrl.pathname).toBe('/api/v1/data/imports')
    expect(uploadUrl.searchParams.get('file_name')).toBe('animals #1.csv')
    expect(uploadUrl.searchParams.get('idempotency_key')).toMatch(/^import-[0-9a-f-]+$/)
    expect(uploadUrl.searchParams.get('import_kind')).toBe('animal')
    expect(upload!.init?.method).toBe('POST')
    expect(upload!.init?.credentials).toBe('include')
    expect(upload!.init?.body).toBe(file)
    expect(new Headers(upload!.init?.headers).get('Content-Type')).toBe('application/octet-stream')
    expect(new Headers(upload!.init?.headers).get('X-CSRF-Token')).toBe('csrf-memory')
    expect(new Headers(upload!.init?.headers).has('Authorization')).toBe(false)

    const confirm = requests.find(({ url }) => url.endsWith('/data/imports/job-1/confirm'))
    expect(confirm?.init?.body).toBe(JSON.stringify({ preview_hash: PREVIEW_HASH }))
    expect(new Headers(confirm?.init?.headers).get('Content-Type')).toBe('application/json')
    expect(new Headers(confirm?.init?.headers).get('X-CSRF-Token')).toBe('csrf-memory')
  })

  it('includes measurement kind and experiment in the bounded upload query', async () => {
    const requests: string[] = []
    const fetchRequest = vi.fn(async (input: string | URL | Request) => {
      const url = String(input)
      requests.push(url)
      if (url.endsWith('/auth/csrf')) return apiResponse({ csrf_token: 'csrf', expires_at: '2026-07-19T00:00:00Z' })
      return apiResponse({ ...validPreview, importKind: 'measurement', experimentId: 'experiment-1' })
    }) as unknown as typeof fetch
    const gateway = new RemoteDataGateway({ baseUrl: '/api/v1', fetch: fetchRequest })

    await gateway.previewImport(new File(['measurement'], 'measurements.csv'), {
      importKind: 'measurement',
      experimentId: 'experiment-1',
    })

    const upload = new URL(requests.find((url) => url.includes('/data/imports?'))!, 'https://lab.example')
    expect(upload.searchParams.get('import_kind')).toBe('measurement')
    expect(upload.searchParams.get('experiment_id')).toBe('experiment-1')
  })

  it('posts remaps to the previous job and includes a retry-safe idempotency key', async () => {
    const requests: Array<{ url: string; init?: RequestInit }> = []
    const fetchRequest = vi.fn(async (input: string | URL | Request, init?: RequestInit) => {
      const url = String(input)
      requests.push({ url, init })
      if (url.endsWith('/auth/csrf')) return apiResponse({ csrf_token: 'csrf', expires_at: '2026-07-19T00:00:00Z' })
      return apiResponse({ ...validPreview, jobId: 'job-2' }, 201)
    }) as unknown as typeof fetch
    const gateway = new RemoteDataGateway({ baseUrl: 'https://lab.example/api/v1', fetch: fetchRequest })

    const mapping = { columns: { display_id: 'custom_code' } }
    await gateway.remapImport('job/1', mapping)
    await gateway.remapImport('job/1', mapping)

    const remaps = requests.filter(({ url }) => url.endsWith('/data/imports/job%2F1/remap'))
    expect(remaps).toHaveLength(2)
    const firstPayload = JSON.parse(String(remaps[0].init?.body))
    const retryPayload = JSON.parse(String(remaps[1].init?.body))
    expect(remaps[0].init?.method).toBe('POST')
    expect(firstPayload).toEqual({
      mapping,
      idempotency_key: expect.stringMatching(/^remap-[0-9a-f-]+$/),
    })
    expect(retryPayload.idempotency_key).toBe(firstPayload.idempotency_key)
    expect(new Headers(remaps[0].init?.headers).get('X-CSRF-Token')).toBe('csrf')
  })

  it('binds an export artifact job to the selected project', async () => {
    const requests: Array<{ url: string; init?: RequestInit }> = []
    const fetchRequest = vi.fn(async (input: string | URL | Request, init?: RequestInit) => {
      const url = String(input)
      requests.push({ url, init })
      if (url.endsWith('/auth/csrf')) {
        return apiResponse({ csrf_token: 'csrf', expires_at: '2026-07-19T00:00:00Z' })
      }
      return apiResponse({
        jobId: 'export-1', kind: 'export', fileName: 'animals.csv', mediaType: 'text/csv',
        sizeBytes: 1, sha256: 'b'.repeat(64), downloadUrl: '/data/artifacts/export-1',
      })
    }) as unknown as typeof fetch
    const gateway = new RemoteDataGateway({ baseUrl: '/api/v1', fetch: fetchRequest })

    await gateway.createExport('csv', 'project-1')

    const exportRequest = requests.find(({ url }) => url.endsWith('/data/exports'))
    expect(JSON.parse(String(exportRequest?.init?.body))).toMatchObject({
      format: 'csv',
      project_id: 'project-1',
      idempotency_key: expect.stringMatching(/^export-[0-9a-f-]+$/),
    })
  })

  it('surfaces structured Server errors and reacquires CSRF after a forbidden response', async () => {
    let csrfRequests = 0
    let mutations = 0
    const requestUrls: string[] = []
    const fetchRequest = vi.fn(async (input: string | URL | Request) => {
      const url = String(input)
      requestUrls.push(url)
      if (url.endsWith('/auth/csrf')) {
        csrfRequests += 1
        return apiResponse({ csrf_token: `csrf-${csrfRequests}`, expires_at: '2026-07-19T00:00:00Z' })
      }
      mutations += 1
      if (mutations === 1) {
        return new Response(JSON.stringify({ error: { code: 'forbidden', message: '没有导入权限' } }), {
          status: 403,
          headers: { 'Content-Type': 'application/json' },
        })
      }
      return apiResponse({
        jobId: 'export-1', kind: 'export', fileName: 'animals.csv', mediaType: 'text/csv',
        sizeBytes: 1, sha256: 'b'.repeat(64), downloadUrl: '/data/artifacts/export-1',
      })
    }) as unknown as typeof fetch
    const gateway = new RemoteDataGateway({ baseUrl: 'https://lab.example/api/v1', fetch: fetchRequest })

    await expect(gateway.confirmImport('job/unsafe', PREVIEW_HASH)).rejects.toThrow('没有导入权限')
    await expect(gateway.createExport('csv')).resolves.toMatchObject({ jobId: 'export-1' })

    expect(csrfRequests).toBe(2)
    expect(requestUrls.some((url) => url.includes('/job%2Funsafe/confirm'))).toBe(true)
  })

  it('reports invalid CSRF responses and failed artifact downloads', async () => {
    const invalidCsrf = new RemoteDataGateway({
      baseUrl: '/api/v1',
      fetch: vi.fn().mockResolvedValue(new Response('not-json', { status: 502 })) as unknown as typeof fetch,
    })
    await expect(invalidCsrf.createSnapshot()).rejects.toThrow('Server 返回了无效响应')

    const failedDownload = new RemoteDataGateway({
      baseUrl: 'https://lab.example/api/v1',
      fetch: vi.fn().mockResolvedValue(new Response('', { status: 503 })) as unknown as typeof fetch,
    })
    await expect(failedDownload.downloadArtifact({
      jobId: 'artifact-1', kind: 'snapshot', fileName: 'snapshot.zip', mediaType: 'application/zip',
      sizeBytes: 0, sha256: 'c'.repeat(64),
    })).rejects.toThrow('无法下载结果文件（503）')
  })
})

describe('DemoDataGateway', () => {
  it('previews conflicts, maps aliases, and blocks confirmation without writing', async () => {
    const gateway = new DemoDataGateway(domainGatewayWithAnimals([
      { id: 'animal-1', code: 'M-001', sex: 'male' },
    ]))
    const preview = await gateway.previewImport(new File([
      '\uFEFFMouse ID,Sex,Birth Date,Strain\nM-001,male,2026-01-01,C57BL/6J\nNEW-001,female,2026-01-02,BALB/c\n',
    ], 'animals.csv'))

    expect(preview).toMatchObject({
      totalRows: 2,
      acceptedRows: 1,
      canConfirm: false,
      mapping: { columns: { display_id: 'Mouse ID', sex: 'Sex', birth_date: 'Birth Date', strain: 'Strain' } },
    })
    expect(preview.issues).toContainEqual(expect.objectContaining({
      row: 2, severity: 'error', code: 'existing_display_id',
    }))
    await expect(gateway.confirmImport(preview.jobId, preview.previewHash)).rejects.toThrow('仍有阻断错误')
  })

  it('reparses the original CSV with a manual mapping and invalidates the old job', async () => {
    const gateway = new DemoDataGateway(domainGatewayWithAnimals([]))
    const original = await gateway.previewImport(new File([
      'custom_code,gender\nNEW-REMAP-1,female\n',
    ], 'animals.csv'))
    expect(original.canConfirm).toBe(false)

    const remapped = await gateway.remapImport(original.jobId, {
      columns: { display_id: 'custom_code', sex: 'gender' },
    })

    expect(remapped).toMatchObject({ acceptedRows: 1, canConfirm: true })
    expect(remapped.jobId).not.toBe(original.jobId)
    expect(remapped.previewHash).not.toBe(original.previewHash)
    await expect(gateway.confirmImport(original.jobId, original.previewHash)).rejects.toThrow('预览已失效')
    await expect(gateway.confirmImport(remapped.jobId, remapped.previewHash)).resolves.toMatchObject({
      counts: { animals: 1 },
    })
  })

  it('keeps the old demo preview when reparsing the replacement fails', async () => {
    const listAnimals = vi.fn()
      .mockResolvedValueOnce([])
      .mockRejectedValueOnce(new Error('目录暂时不可用'))
    const gateway = new DemoDataGateway({ listAnimals } as unknown as MuriArcGateway)
    const original = await gateway.previewImport(new File(['display_id\nM-SAFE\n'], 'animals.csv'))

    await expect(gateway.remapImport(original.jobId, original.mapping)).rejects.toThrow('目录暂时不可用')
    await expect(gateway.confirmImport(original.jobId, original.previewHash)).resolves.toMatchObject({
      counts: { animals: 1 },
    })
  })

  it('rejects reused, unknown, and duplicate source columns during demo remap', async () => {
    const gateway = new DemoDataGateway(domainGatewayWithAnimals([]))
    const original = await gateway.previewImport(new File(['code,code\nM-1,M-2\n'], 'animals.csv'))
    const remapped = await gateway.remapImport(original.jobId, {
      columns: { display_id: 'code', sex: 'code', unsupported: 'missing' },
    })
    const codes = remapped.issues.map((issue) => issue.code)
    expect(codes).toContain('duplicate_source_column')
    expect(codes).toContain('duplicate_source_mapping')
    expect(codes).toContain('unknown_mapping_target')
    expect(remapped.canConfirm).toBe(false)
  })

  it('confirms a clean CSV preview once and returns transaction counts', async () => {
    const gateway = new DemoDataGateway(domainGatewayWithAnimals([]))
    const preview = await gateway.previewImport(new File([
      '小鼠编号,性别,笼位,基因型\nNEW-001,male,A01,WT\nNEW-002,female,A02,fl/fl\n',
    ], 'animals.csv'))

    expect(preview).toMatchObject({ totalRows: 2, acceptedRows: 2, issues: [], canConfirm: true })
    expect(preview.previewHash).toMatch(/^[0-9a-f]{64}$/)
    const receipt = await gateway.confirmImport(preview.jobId, preview.previewHash)
    expect(receipt).toMatchObject({
      jobId: preview.jobId,
      previewHash: preview.previewHash,
      replayed: false,
      counts: { animals: 2, animalEvents: 2, genotypes: 0, pedigrees: 0, measurements: 0 },
    })
    await expect(gateway.confirmImport(preview.jobId, preview.previewHash)).rejects.toThrow('预览已失效')
  })

  it('marks every row unaccepted when the required animal-id mapping is absent', async () => {
    const gateway = new DemoDataGateway(domainGatewayWithAnimals([]))
    const preview = await gateway.previewImport(new File(['Weight,Date\n23.1,2026-07-19\n'], 'measurements.csv'))

    expect(preview).toMatchObject({ totalRows: 1, acceptedRows: 0, canConfirm: false })
    expect(preview.issues).toContainEqual(expect.objectContaining({ code: 'missing_required_mapping' }))
  })

  it('previews and confirms measurement CSVs only with explicit experiment scope', async () => {
    const gateway = new DemoDataGateway(domainGatewayWithAnimals([
      { id: 'animal-1', code: 'M-001', sex: 'female' },
    ]))
    const file = new File([
      'display_id,measurement_key,value_type,value,unit,measured_at\nM-001,body_weight,number,22.5,g,2026-07-19\n',
    ], 'measurements.csv')

    await expect(gateway.previewImport(file, { importKind: 'measurement' })).rejects.toThrow('必须选择所属实验')
    const preview = await gateway.previewImport(file, {
      importKind: 'measurement',
      experimentId: 'experiment-1',
    })
    expect(preview).toMatchObject({
      importKind: 'measurement',
      experimentId: 'experiment-1',
      totalRows: 1,
      acceptedRows: 1,
      canConfirm: true,
    })
    const receipt = await gateway.confirmImport(preview.jobId, preview.previewHash)
    expect(receipt.counts).toEqual({ animals: 0, animalEvents: 0, genotypes: 0, pedigrees: 0, measurements: 1 })
  })

  it('repairs custom measurement headers through the same remap operation', async () => {
    const gateway = new DemoDataGateway(domainGatewayWithAnimals([
      { id: 'animal-1', code: 'M-001', sex: 'female' },
    ]))
    const original = await gateway.previewImport(new File([
      'mouse,metric,kind,result,result_unit,when\nM-001,body_weight,number,22.5,g,2026-07-19\n',
    ], 'measurements.csv'), { importKind: 'measurement', experimentId: 'experiment-1' })
    expect(original.canConfirm).toBe(false)

    const remapped = await gateway.remapImport(original.jobId, { columns: {
      display_id: 'mouse', measurement_key: 'metric', value_type: 'kind', value: 'result',
      unit: 'result_unit', measured_at: 'when',
    } })
    expect(remapped).toMatchObject({ importKind: 'measurement', acceptedRows: 1, canConfirm: true })
  })

  it('creates deterministic downloadable CSV and JSON artifacts from domain animals', async () => {
    const animals = [
      { id: 'animal-1', code: 'M-001', sex: 'male' as const },
      { id: 'animal-2', code: 'M-002', sex: 'female' as const },
    ]
    const gateway = new DemoDataGateway(domainGatewayWithAnimals(animals))

    const exported = await gateway.createExport('csv')
    const snapshot = await gateway.createSnapshot()
    const exportText = new TextDecoder().decode(Uint8Array.from(exported.bytes!))
    const snapshotJson = JSON.parse(new TextDecoder().decode(Uint8Array.from(snapshot.bytes!)))

    expect(exported).toMatchObject({ kind: 'export', fileName: 'animals.csv', mediaType: 'text/csv;charset=utf-8' })
    expect(exported.sizeBytes).toBe(exported.bytes?.length)
    expect(exported.sha256).toMatch(/^[0-9a-f]{64}$/)
    expect(exportText).not.toContain('animal_uuid')
    expect(exportText).not.toContain('animal-1')
    expect(exportText).toContain('identifier_scope,project_name,display_id')
    expect(exportText).toContain('"M-001"')
    expect(snapshot).toMatchObject({ kind: 'snapshot', fileName: 'muriarc-demo-snapshot.json' })
    expect(snapshotJson).toEqual({ product: 'MuriArc', demo: true, animals })
  })
})

describe('data file validation', () => {
  it('rejects path-bearing names, unsupported types, empty files, and oversized files before transport', async () => {
    const invokeCommand = vi.fn()
    const gateway = new LocalDataGateway(invokeCommand)
    const fileLike = (name: string, size: number) => ({ name, size }) as File

    await expect(gateway.previewImport(fileLike('../animals.csv', 1))).rejects.toThrow('不能包含路径')
    await expect(gateway.previewImport(fileLike('folder\\animals.csv', 1))).rejects.toThrow('不能包含路径')
    await expect(gateway.previewImport(fileLike('animals.exe', 1))).rejects.toThrow('仅支持 CSV 与 XLSX')
    await expect(gateway.previewImport(fileLike('animals.csv', 0))).rejects.toThrow('文件为空')
    await expect(gateway.previewImport(fileLike('animals.csv', MAX_IMPORT_FILE_BYTES + 1))).rejects.toThrow('32 MiB')
    expect(invokeCommand).not.toHaveBeenCalled()
  })

  it('keeps XLSX unavailable in browser demo mode instead of pretending to parse it', async () => {
    const gateway = new DemoDataGateway(domainGatewayWithAnimals([]))
    await expect(gateway.previewImport(new File(['xlsx'], 'animals.xlsx'))).rejects.toThrow('演示模式仅解析 CSV')
  })
})
