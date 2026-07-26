import { describe, expect, it, vi } from 'vitest'
import {
  currentAuthSession,
  DemoGateway,
  GatewayError,
  LocalTauriGateway,
  RemoteHttpGateway,
  createGateway,
} from './gateway'
import { currentProjectId } from './projectContext'

describe('MuriArc gateway selection', () => {
  it('uses real adapters for explicit local and remote modes', () => {
    expect(createGateway('local')).toBeInstanceOf(LocalTauriGateway)
    expect(createGateway('remote')).toBeInstanceOf(RemoteHttpGateway)
    expect(createGateway('demo')).toBeInstanceOf(DemoGateway)
  })

  it('passes structured command payloads to Tauri invoke', async () => {
    const calls: Array<[string, Record<string, unknown> | undefined]> = []
    const invokeCommand = async <T>(command: string, args?: Record<string, unknown>): Promise<T> => {
      calls.push([command, args])
      return undefined as T
    }
    const gateway = new LocalTauriGateway(invokeCommand)

    await gateway.moveAnimals(['animal-1'], 'cage-2')

    expect(calls).toEqual([[
      'move_animals',
      { input: { animalIds: ['animal-1'], targetCageId: 'cage-2' } },
    ]])
  })

  it('sends only the one-time selection token when scheduling a local storage migration', async () => {
    const calls: Array<[string, Record<string, unknown> | undefined]> = []
    const invokeCommand = async <T>(command: string, args?: Record<string, unknown>): Promise<T> => {
      calls.push([command, args])
      if (command === 'choose_local_storage_directory') {
        return {
          selectionToken: 'selection-token-1',
          targetDataRoot: 'D:\\MuriArcData',
        } as T
      }
      return {
        scheduled: true,
        requiresRestart: true,
        targetDataRoot: 'D:\\MuriArcData',
      } as T
    }
    const gateway = new LocalTauriGateway(invokeCommand)

    const selection = await gateway.chooseLocalStorageDirectory()
    await gateway.requestLocalStorageMigration(selection!.selectionToken)

    expect(calls).toEqual([
      ['choose_local_storage_directory', undefined],
      [
        'request_local_storage_migration',
        { input: { selectionToken: 'selection-token-1' } },
      ],
    ])
    expect(JSON.stringify(calls[1])).not.toContain('MuriArcData')
  })

  it('preserves structured Tauri error codes for UI recovery decisions', async () => {
    const gateway = new LocalTauriGateway(async () => {
      throw { code: 'conflict', message: 'AI 会话 revision 已变化' }
    })

    const operation = gateway.getAiConversation('conversation-1')
    await expect(operation).rejects.toBeInstanceOf(GatewayError)
    await expect(operation).rejects.toMatchObject({
      code: 'conflict',
      message: 'AI 会话 revision 已变化',
    })
  })

  it('never forwards a Server password or client step-up claim to the local Tauri decision DTO', async () => {
    const calls: Array<[string, Record<string, unknown> | undefined]> = []
    const invokeCommand = async <T>(command: string, args?: Record<string, unknown>): Promise<T> => {
      calls.push([command, args])
      return {} as T
    }
    const local = new LocalTauriGateway(invokeCommand)

    await local.decideAiDraft('draft-1', {
      expectedRevision: 2,
      decision: 'approve',
      statement: '我已核对完整导入预览',
      currentPassword: 'must-not-cross-local-ipc',
    })

    expect(calls).toEqual([[
      'decide_ai_draft',
      {
        draftId: 'draft-1',
        input: {
          expectedRevision: 2,
          decision: 'approve',
          statement: '我已核对完整导入预览',
        },
      },
    ]])
    expect(JSON.stringify(calls)).not.toContain('must-not-cross-local-ipc')
    expect(JSON.stringify(calls)).not.toContain('stepUpVerified')
  })

  it('uses scoped Tauri commands for animal detail, samples, and pedigree writes', async () => {
    const calls: Array<[string, Record<string, unknown> | undefined]> = []
    const invokeCommand = async <T>(command: string, args?: Record<string, unknown>): Promise<T> => {
      calls.push([command, args])
      return {} as T
    }
    const gateway = new LocalTauriGateway(invokeCommand)

    await gateway.getAnimalDetail('animal-1', { projectId: 'project-1' })
    await gateway.createAnimalSample({
      animalId: 'animal-1', projectId: 'project-1', experimentId: 'experiment-1',
      sampleType: 'lung tissue', quantity: 12.5, unit: 'mg', location: '-80C/A3',
      collectedAt: '2026-07-19T08:30:00Z',
    })
    await gateway.createPedigree({
      projectId: 'project-1', animalId: 'animal-1', parentId: 'animal-2', parentType: 'father',
    })

    expect(calls).toEqual([
      ['get_animal_detail', { animalId: 'animal-1', projectId: 'project-1' }],
      ['create_animal_sample', { input: {
        animalId: 'animal-1', projectId: 'project-1', experimentId: 'experiment-1',
        sampleType: 'lung tissue', quantity: 12.5, unit: 'mg', location: '-80C/A3',
        collectedAt: '2026-07-19T08:30:00Z',
      } }],
      ['create_pedigree_relation', { input: {
        projectId: 'project-1', animalId: 'animal-1', parentId: 'animal-2', parentType: 'father',
      } }],
    ])
  })

  it('sends stable DTOs for the local animal and experiment write chain', async () => {
    const calls: Array<[string, Record<string, unknown> | undefined]> = []
    const invokeCommand = async <T>(command: string, args?: Record<string, unknown>): Promise<T> => {
      calls.push([command, args])
      return { id: `${command}-id`, name: command } as T
    }
    const gateway = new LocalTauriGateway(invokeCommand)

    await gateway.createAnimal({
      displayId: 'M-001', identifierScope: 'project', projectId: 'project-1',
      cageId: 'cage-1', sex: 'female', strain: 'C57BL/6J', birthDate: '2026-06-01',
    })
    await gateway.createPublishedTemplate({
      name: '体重观察', description: '', fieldKey: 'body_weight',
      fieldLabel: '体重', fieldValueType: 'number', fieldUnit: 'g',
    })
    await gateway.createExperiment({
      projectId: 'project-1', templateVersionId: 'template-1',
      name: 'DEMO-001', description: '', startDate: '2026-07-19',
    })
    await gateway.enrollAnimal({
      experimentId: 'experiment-1', animalId: 'animal-1', cohortId: 'cohort-1',
    })
    await gateway.createProcedure({
      experimentId: 'experiment-1', animalId: 'animal-1', name: '给药',
      status: 'completed', performedAt: '2026-07-19T08:00:00Z', details: { dose: 'vehicle' },
    })

    expect(calls).toContainEqual(['create_animal', {
      input: expect.objectContaining({
        displayId: 'M-001', identifierScope: 'project', projectId: 'project-1', cageId: 'cage-1',
      }),
    }])
    expect(calls).toContainEqual(['create_published_template', {
      input: expect.objectContaining({ fieldKey: 'body_weight', fieldValueType: 'number', fieldUnit: 'g' }),
    }])
    expect(calls).toContainEqual(['create_experiment', {
      input: expect.objectContaining({ projectId: 'project-1', templateVersionId: 'template-1' }),
    }])
    expect(calls.at(-2)).toEqual(['enroll_animal', {
      input: { experimentId: 'experiment-1', animalId: 'animal-1', cohortId: 'cohort-1' },
    }])
    expect(calls.at(-1)).toEqual(['create_procedure', {
      input: expect.objectContaining({ status: 'completed', performedAt: '2026-07-19T08:00:00Z' }),
    }])
  })

  it('uses stable Tauri DTOs for genetics and lifecycle commands', async () => {
    const calls: Array<[string, Record<string, unknown> | undefined]> = []
    const invokeCommand = async <T>(command: string, args?: Record<string, unknown>): Promise<T> => {
      calls.push([command, args])
      return (command.startsWith('list_') ? [] : { id: `${command}-id` }) as T
    }
    const gateway = new LocalTauriGateway(invokeCommand)

    await gateway.listGeneLoci('project-1')
    await gateway.createGeneLocus({ projectId: 'project-1', symbol: 'GeneA' })
    await gateway.listAlleles('locus-1', 'project-1')
    await gateway.createAllele({
      projectId: 'project-1', locusId: 'locus-1', symbol: 'flox', isWildType: false,
    })
    await gateway.listGenotypes('animal-1', 'project-1')
    await gateway.createGenotype({
      projectId: 'project-1', animalId: 'animal-1', locusId: 'locus-1',
      allele1Id: 'allele-1', allele2Id: 'allele-2', assessedAt: '2026-07-19T09:00:00Z',
    })
    await gateway.completeExperiment('experiment-1', 3)
    await gateway.cancelExperiment('experiment-2', 4)
    await gateway.completeParticipation('participation-1', 5)
    await gateway.withdrawParticipation('participation-2', 6)

    expect(calls).toContainEqual(['list_gene_loci', {
      projectId: 'project-1', includeArchived: false,
    }])
    expect(calls).toContainEqual(['create_gene_locus', {
      input: { projectId: 'project-1', symbol: 'GeneA' },
    }])
    expect(calls).toContainEqual(['list_alleles', {
      locusId: 'locus-1', projectId: 'project-1', includeArchived: false,
    }])
    expect(calls).toContainEqual(['create_genotype', { input: expect.objectContaining({
      animalId: 'animal-1', locusId: 'locus-1', allele1Id: 'allele-1', allele2Id: 'allele-2',
    }) }])
    expect(calls.slice(-4)).toEqual([
      ['complete_experiment', { input: { id: 'experiment-1', expectedRevision: 3 } }],
      ['cancel_experiment', { input: { id: 'experiment-2', expectedRevision: 4 } }],
      ['complete_participation', { input: { id: 'participation-1', expectedRevision: 5 } }],
      ['withdraw_participation', { input: { id: 'participation-2', expectedRevision: 6 } }],
    ])
  })

  it('uses typed settings commands without returning stored API keys', async () => {
    const calls: Array<[string, Record<string, unknown> | undefined]> = []
    const invokeCommand = async <T>(command: string, args?: Record<string, unknown>): Promise<T> => {
      calls.push([command, args])
      if (command === 'get_workspace_settings' || command === 'save_workspace_settings') {
        return { labName: '转化医学实验室', operatorName: '林研究员' } as T
      }
      return {
        enabled: true,
        providerKind: 'open_ai_compatible',
        model: 'gpt-4.1-mini',
        baseUrl: 'https://api.openai.com/v1',
        hasKey: true,
      } as T
    }
    const gateway = new LocalTauriGateway(invokeCommand)

    const workspace = await gateway.saveWorkspaceSettings({
      labName: '转化医学实验室',
      operatorName: '林研究员',
    })
    const saved = await gateway.saveAiSettings({
      enabled: true,
      providerKind: 'open_ai_compatible',
      model: 'gpt-4.1-mini',
      baseUrl: 'https://api.openai.com/v1',
      providerPresetId: 'openai',
      contextWindowTokens: 400000,
      maxInputTokens: 65536,
      maxOutputTokens: 4096,
      historyTokenBudget: 32768,
      historyTurns: 20,
      temperature: 0,
      timeoutMs: 120000,
      apiKey: 'write-only-secret',
    })
    const loaded = await gateway.getAiSettings()
    await gateway.clearAiApiKey()

    expect(workspace.operatorName).toBe('林研究员')
    expect(calls).toContainEqual([
      'save_workspace_settings',
      { input: { labName: '转化医学实验室', operatorName: '林研究员' } },
    ])
    expect(calls).toContainEqual([
      'save_ai_settings',
      { input: expect.objectContaining({ apiKey: 'write-only-secret' }) },
    ])
    expect(calls.at(-2)).toEqual(['get_ai_settings', undefined])
    expect(calls.at(-1)).toEqual(['clear_ai_api_key', undefined])
    expect(saved).not.toHaveProperty('apiKey')
    expect(loaded).toEqual(expect.objectContaining({ hasKey: true }))
    expect(loaded).not.toHaveProperty('apiKey')
  })

  it('maps Tauri AI responses and sends only stable command DTOs', async () => {
    const calls: Array<[string, Record<string, unknown> | undefined]> = []
    const invokeCommand = async <T>(command: string, args?: Record<string, unknown>): Promise<T> => {
      calls.push([command, args])
      if (command !== 'ai_turn') return [] as T
      return {
        conversationId: 'conversation-1',
        content: '已找到 1 只动物',
        citations: [
          { entity_type: 'animal', entity_id: 'animal-1', revision: 3 },
          {
            entity_type: 'project_animal_assignment',
            entity_id: 'assignment-1',
            revision: 7,
          },
          {
            entity_type: 'ai_conversation_source',
            entity_id: 'source-1',
            revision: 2,
          },
          { entity_type: 'future_entity', entity_id: 'future-1', revision: 1 },
        ],
        toolRuns: [{
          tool_run_id: 'run-1', provider_call_id: 'call-1', tool: 'animal_search',
          arguments: { display_id: 'M-001' }, outcome: 'read',
          citations: [{ entity_type: 'animal', entity_id: 'animal-1', revision: 3 }],
        }],
        drafts: [],
        incompleteReason: 'tool_call_limit_exceeded',
        trace: {
          providerId: 'local-provider', model: 'test-model',
          usage: { provider_calls: 1, tool_calls: 1, input_tokens: 10, output_tokens: 5, total_tokens: 15 },
        },
      } as T
    }
    const local = new LocalTauriGateway(invokeCommand)

    const response = await local.aiTurn({
      conversationId: 'conversation-1',
      projectId: 'project-1',
      message: '查找 M-001',
    })
    await local.listAiDrafts('project-1', 'pending_approval')

    expect(calls[0]).toEqual(['ai_turn', {
      input: {
        conversationId: 'conversation-1',
        projectId: 'project-1',
        message: '查找 M-001',
      },
    }])
    expect(calls[1]).toEqual(['list_ai_drafts', { projectId: 'project-1', status: 'pending_approval' }])
    expect(response.citations[0]).toEqual(expect.objectContaining({
      entityType: 'animal', entityId: 'animal-1', revision: 3, route: '/animals?animal=animal-1',
    }))
    expect(response.citations[1]).toEqual(expect.objectContaining({
      entityType: 'project_animal_assignment',
      entityId: 'assignment-1',
      revision: 7,
      label: expect.stringContaining('项目动物关系'),
      route: undefined,
    }))
    expect(response.citations[2]).toEqual(expect.objectContaining({
      entityType: 'ai_conversation_source',
      entityId: 'source-1',
      revision: 2,
      label: expect.stringContaining('AI 会话来源'),
      route: undefined,
    }))
    expect(response.citations[3]).toEqual(expect.objectContaining({
      entityType: 'future_entity',
      entityId: 'future-1',
      label: expect.stringContaining('未知实体（future_entity）'),
      route: undefined,
    }))
    expect(response.citations.map((citation) => citation.label)).not.toContain(
      expect.stringContaining('undefined'),
    )
    expect(response.toolRuns[0]).toEqual(expect.objectContaining({ toolRunId: 'run-1', outcome: 'read' }))
    expect(response.trace.usage.totalTokens).toBe(15)
    expect(response.incompleteReason).toBe('tool_call_limit_exceeded')
  })

  it('loads and maps persisted AI conversations through Tauri commands', async () => {
    const calls: Array<[string, Record<string, unknown> | undefined]> = []
    const invokeCommand = async <T>(command: string, args?: Record<string, unknown>): Promise<T> => {
      calls.push([command, args])
      if (command === 'list_ai_conversations') {
        return [{
          id: 'conversation-1', projectId: null, title: '实验进度', revision: 2,
          createdAt: '2026-07-19T01:00:00Z', updatedAt: '2026-07-19T02:00:00Z',
        }] as T
      }
      return {
        conversation: {
          id: 'conversation-1', projectId: null, title: '实验进度', revision: 2,
          createdAt: '2026-07-19T01:00:00Z', updatedAt: '2026-07-19T02:00:00Z',
        },
        messages: [{
          id: 'message-1', sequence: 1, role: 'user', content: '检查文件',
          sourceRefs: [{
            sourceId: 'source-1', sourceRevision: 2,
            fileName: 'weights.csv', mediaType: 'text/csv', sizeBytes: 2048,
          }],
          createdAt: '2026-07-19T01:59:59Z',
        }, {
          id: 'message-2', sequence: 2, role: 'assistant', content: '已恢复',
          createdAt: '2026-07-19T02:00:00Z',
          response: {
            conversationId: 'conversation-1', content: '已恢复',
            citations: [{ entity_type: 'animal', entity_id: 'animal-1', revision: 4 }],
            toolRuns: [], drafts: [],
            trace: { providerId: 'local', model: 'test', usage: {
              provider_calls: 1, tool_calls: 0, input_tokens: 1, output_tokens: 1, total_tokens: 2,
            } },
          },
        }],
      } as T
    }
    const local = new LocalTauriGateway(invokeCommand)

    const conversations = await local.listAiConversations(undefined, 25)
    const detail = await local.getAiConversation('conversation-1', 40)

    expect(calls).toEqual([
      ['list_ai_conversations', { projectId: undefined, limit: 25 }],
      ['get_ai_conversation', { conversationId: 'conversation-1', limit: 40 }],
    ])
    expect(conversations[0].projectId).toBeUndefined()
    expect(detail.messages[0].sourceRefs).toEqual([{
      sourceId: 'source-1',
      sourceRevision: 2,
      fileName: 'weights.csv',
      mediaType: 'text/csv',
      sizeBytes: 2048,
    }])
    expect(detail.messages[1].response?.citations[0]).toEqual(expect.objectContaining({
      entityId: 'animal-1', revision: 4,
    }))
  })

  it('uses bounded project-scoped Server conversation reads without CSRF', async () => {
    const requests: Array<{ url: string; init?: RequestInit }> = []
    const fetchRequest = vi.fn(async (input: string | URL | Request, init?: RequestInit) => {
      const url = String(input)
      requests.push({ url, init })
      if (url.includes('/ai/conversations/conversation-1')) {
        return new Response(JSON.stringify({ data: {
          conversation: {
            id: 'conversation-1', projectId: 'project-1', title: '实验进度', revision: 1,
            createdAt: '2026-07-19T01:00:00Z', updatedAt: '2026-07-19T01:00:00Z',
          },
          messages: [],
        }, request_id: 'req-detail' }), { status: 200, headers: { 'Content-Type': 'application/json' } })
      }
      return new Response(JSON.stringify({ data: [{
        id: 'conversation-1', projectId: 'project-1', title: '实验进度', revision: 1,
        createdAt: '2026-07-19T01:00:00Z', updatedAt: '2026-07-19T01:00:00Z',
      }], count: 1, request_id: 'req-list' }), { status: 200, headers: { 'Content-Type': 'application/json' } })
    }) as unknown as typeof fetch
    const remote = new RemoteHttpGateway({ baseUrl: 'https://lab.example/api/v1', fetch: fetchRequest })

    await remote.listAiConversations('project-1', 30)
    await remote.getAiConversation('conversation-1', 60)

    expect(requests.map((request) => request.url)).toEqual([
      'https://lab.example/api/v1/ai/conversations?limit=30&project_id=project-1',
      'https://lab.example/api/v1/ai/conversations/conversation-1?limit=60',
    ])
    expect(requests.every((request) => new Headers(request.init?.headers).get('X-CSRF-Token') === null)).toBe(true)
  })

  it('uses revisioned conversation actions and opaque staged source APIs on Server', async () => {
    const requests: Array<{ url: string; init?: RequestInit }> = []
    const summary = {
      id: 'conversation-1', projectId: 'project-1', title: '实验进度',
      pinnedAt: '2026-07-23T09:00:00Z', archivedAt: null, revision: 3,
      createdAt: '2026-07-23T08:00:00Z', updatedAt: '2026-07-23T09:00:00Z',
    }
    const source = {
      id: 'source-1', conversationId: 'conversation-1', projectId: 'project-1',
      fileName: 'measurements.xlsx',
      mediaType: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
      sizeBytes: 4, status: 'ready', revision: 1,
      createdAt: '2026-07-23T09:01:00Z', expiresAt: '2026-08-22T09:01:00Z',
    }
    const listedSources = [
      source,
      { ...source, id: 'source-staged', status: 'staged' },
      { ...source, id: 'source-archived', status: 'archived', revision: 2 },
      { ...source, id: 'source-failed', status: 'failed' },
      { ...source, id: 'source-expired', status: 'expired' },
    ]
    const archivedSource = { ...source, status: 'archived', revision: 2 }
    const fetchRequest = vi.fn(async (input: string | URL | Request, init?: RequestInit) => {
      const url = String(input)
      requests.push({ url, init })
      if (url.endsWith('/auth/login')) {
        return new Response(JSON.stringify({ data: {
          user: { id: 'user-1', lab_id: 'lab-1', display_name: '研究者', lab_roles: [], project_roles: [], authentication: 'session' },
          csrf_token: 'csrf-ai-workbench', expires_at: '2026-07-23T12:00:00Z',
        }, request_id: 'req-login' }), { status: 200, headers: { 'Content-Type': 'application/json' } })
      }
      if (url.endsWith('/ai/sources/source-1/archive') && init?.method === 'POST') {
        return new Response(JSON.stringify({ data: archivedSource, request_id: 'req-archive' }), {
          status: 200, headers: { 'Content-Type': 'application/json' },
        })
      }
      if (url.includes('/ai/sources/source-1') && init?.method === 'DELETE') {
        return new Response(null, { status: 204 })
      }
      if (url.includes('/ai/sources?')) {
        const status = new URL(url).searchParams.get('status')
        const data = status
          ? listedSources.filter((candidate) => candidate.status === status)
          : listedSources
        return new Response(JSON.stringify({
          data, count: data.length, request_id: 'req-source-list',
        }), { status: 200, headers: { 'Content-Type': 'application/json' } })
      }
      if (url.includes('/ai/sources/upload')) {
        return new Response(JSON.stringify({ data: source, request_id: 'req-source' }), {
          status: 201, headers: { 'Content-Type': 'application/json' },
        })
      }
      if (url.endsWith('/ai/conversations') && init?.method === 'POST') {
        return new Response(JSON.stringify({
          data: {
            conversation: summary,
            autonomy: {
              mode: 'ask',
              effectiveMode: 'ask',
              maxMode: 'full',
              batchLimit: 1,
              revision: 1,
              requiresHumanApproval: [],
            },
          },
          request_id: 'req-create',
        }), {
          status: 201, headers: { 'Content-Type': 'application/json' },
        })
      }
      if (url.endsWith('/ai/conversations/conversation-1') && init?.method === 'PATCH') {
        return new Response(JSON.stringify({ data: summary, request_id: 'req-update' }), {
          status: 200, headers: { 'Content-Type': 'application/json' },
        })
      }
      return new Response(JSON.stringify({ data: [summary], count: 1, request_id: 'req-list' }), {
        status: 200, headers: { 'Content-Type': 'application/json' },
      })
    }) as unknown as typeof fetch
    const remote = new RemoteHttpGateway({
      baseUrl: 'https://lab.example/api/v1',
      fetch: fetchRequest,
    })
    await remote.login({ email: 'r@example.org', password: 'not-retained' })

    await remote.startAiConversation({
      projectId: 'project-1',
      title: '新对话',
      modelProfileId: 'profile-1',
      requestedMode: 'ask',
    })
    const conversations = await remote.queryAiConversations({
      projectId: 'project-1',
      titleQuery: '实验',
      archive: 'active',
      limit: 80,
    })
    await remote.updateAiConversation('conversation-1', {
      action: 'rename',
      title: '新的实验标题',
      expectedRevision: 2,
    })
    const file = new File(['xlsx'], 'measurements.xlsx', {
      type: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
    })
    const uploaded = await remote.uploadAiSource({
      file,
      conversationId: 'conversation-1',
      projectId: 'project-1',
    })
    const sources = await remote.listAiSources({
      conversationId: 'conversation-1',
      projectId: 'project-1',
    })
    const archivedSources = await remote.listAiSources({
      conversationId: 'conversation-1',
      projectId: 'project-1',
      status: 'archived',
    })
    const archived = await remote.archiveAiSource(uploaded.id, {
      projectId: 'project-1',
      expectedRevision: uploaded.revision,
    })
    await remote.deleteAiSource(uploaded.id)

    expect(conversations[0]).toEqual(expect.objectContaining({
      id: 'conversation-1',
      pinnedAt: '2026-07-23T09:00:00Z',
      archivedAt: undefined,
    }))
    expect(sources.map((candidate) => candidate.status)).toEqual([
      'ready', 'staged', 'archived', 'failed', 'expired',
    ])
    expect(archivedSources).toEqual([
      expect.objectContaining({ id: 'source-archived', status: 'archived', revision: 2 }),
    ])
    expect(archived).toEqual(expect.objectContaining({
      id: 'source-1',
      status: 'archived',
      revision: 2,
    }))
    const create = requests.find((request) =>
      request.url.endsWith('/ai/conversations') && request.init?.method === 'POST')!
    expect(JSON.parse(String(create.init?.body))).toEqual({
      projectId: 'project-1',
      title: '新对话',
      modelProfileId: 'profile-1',
      requestedMode: 'ask',
    })
    expect(requests.some((request) => request.url.endsWith(
      '/ai/conversations?archive=active&limit=80&project_id=project-1&q=%E5%AE%9E%E9%AA%8C',
    ))).toBe(true)
    const update = requests.find((request) =>
      request.url.endsWith('/ai/conversations/conversation-1') && request.init?.method === 'PATCH')!
    expect(JSON.parse(String(update.init?.body))).toEqual({
      action: 'rename',
      expected_revision: 2,
      title: '新的实验标题',
    })
    const upload = requests.find((request) => request.url.includes('/ai/sources/upload?'))!
    expect(new URL(upload.url).searchParams.get('conversation_id')).toBe('conversation-1')
    expect(upload.init?.body).toBe(file)
    expect(requests.some((request) => request.url.endsWith(
      '/ai/sources?conversation_id=conversation-1&project_id=project-1',
    ))).toBe(true)
    expect(requests.some((request) => request.url.endsWith(
      '/ai/sources?conversation_id=conversation-1&project_id=project-1&status=archived',
    ))).toBe(true)
    const archive = requests.find((request) =>
      request.url.endsWith('/ai/sources/source-1/archive') && request.init?.method === 'POST')!
    expect(JSON.parse(String(archive.init?.body))).toEqual({
      project_id: 'project-1',
      expected_revision: 1,
    })
    for (const request of requests.filter((request) =>
      !request.url.endsWith('/auth/login')
      && ['PATCH', 'POST', 'DELETE'].includes(request.init?.method ?? ''))) {
      expect(new Headers(request.init?.headers).get('X-CSRF-Token')).toBe('csrf-ai-workbench')
    }
  })

  it('sends source bytes and conversation actions through typed Tauri commands', async () => {
    const calls: Array<[string, Record<string, unknown> | undefined]> = []
    const summary = {
      id: 'conversation-1', projectId: null, title: '已置顶', pinnedAt: '2026-07-23T09:00:00Z',
      archivedAt: null, createdAt: '2026-07-23T08:00:00Z',
      updatedAt: '2026-07-23T09:00:00Z', revision: 3,
    }
    const source = {
      id: 'source-1', conversationId: 'conversation-1', projectId: 'project-1',
      fileName: 'notes.txt', mediaType: 'text/plain', sizeBytes: 3,
      status: 'ready', revision: 1, createdAt: '2026-07-23T09:00:00Z',
      expiresAt: '2026-08-22T09:00:00Z',
    }
    const invokeCommand = async <T>(command: string, args?: Record<string, unknown>): Promise<T> => {
      calls.push([command, args])
      if (command === 'list_ai_conversations') return [summary] as T
      if (command === 'start_ai_conversation') {
        return {
          conversation: summary,
          autonomy: {
            mode: 'ask',
            effectiveMode: 'ask',
            maxMode: 'full',
            batchLimit: 1,
            revision: 1,
            requiresHumanApproval: [],
          },
        } as T
      }
      if (command === 'upload_ai_source') return source as T
      if (command === 'list_ai_sources') {
        return [
          source,
          { ...source, id: 'source-staged', status: 'staged' },
          { ...source, id: 'source-archived', status: 'archived', revision: 2 },
          { ...source, id: 'source-failed', status: 'failed' },
          { ...source, id: 'source-expired', status: 'expired' },
        ] as T
      }
      if (command === 'archive_ai_source') {
        return { ...source, status: 'archived', revision: 2 } as T
      }
      return summary as T
    }
    const local = new LocalTauriGateway(invokeCommand)

    await local.startAiConversation({
      projectId: 'project-1',
      title: '新对话',
      modelProfileId: 'profile-1',
      requestedMode: 'ask',
    })
    await local.queryAiConversations({ titleQuery: '置顶', archive: 'all', limit: 40 })
    await local.updateAiConversation('conversation-1', {
      action: 'pin',
      expectedRevision: 2,
    })
    const file = {
      name: 'notes.txt',
      type: 'text/plain',
      size: 3,
      arrayBuffer: async () => Uint8Array.from([97, 98, 99]).buffer,
    } as File
    await local.uploadAiSource({
      file,
      conversationId: 'conversation-1',
      projectId: 'project-1',
    })
    const sources = await local.listAiSources({
      conversationId: 'conversation-1',
      projectId: 'project-1',
    })
    const archived = await local.archiveAiSource('source-1', {
      projectId: 'project-1',
      expectedRevision: 1,
    })
    await local.deleteAiSource('source-1')

    expect(sources.map((candidate) => candidate.status)).toEqual([
      'ready', 'staged', 'archived', 'failed', 'expired',
    ])
    expect(archived).toEqual(expect.objectContaining({
      id: 'source-1',
      status: 'archived',
      revision: 2,
    }))
    expect(calls).toEqual([
      ['start_ai_conversation', {
        input: {
          projectId: 'project-1',
          title: '新对话',
          modelProfileId: 'profile-1',
          requestedMode: 'ask',
        },
      }],
      ['list_ai_conversations', {
        projectId: undefined, titleQuery: '置顶', archive: 'all', limit: 40,
      }],
      ['update_ai_conversation', {
        conversationId: 'conversation-1',
        input: { action: 'pin', expectedRevision: 2 },
      }],
      ['upload_ai_source', { input: {
        fileName: 'notes.txt', mediaType: 'text/plain',
        conversationId: 'conversation-1', projectId: 'project-1', bytes: [97, 98, 99],
      } }],
      ['list_ai_sources', { input: {
        conversationId: 'conversation-1', projectId: 'project-1', status: undefined,
      } }],
      ['archive_ai_source', {
        sourceId: 'source-1',
        input: { projectId: 'project-1', expectedRevision: 1 },
      }],
      ['delete_ai_source', { sourceId: 'source-1' }],
    ])
  })

  it('rejects unknown AI source states instead of silently treating them as ready', async () => {
    const local = new LocalTauriGateway(async <T>(command: string): Promise<T> => {
      if (command !== 'list_ai_sources') return undefined as T
      return [{
        id: 'source-unknown',
        conversationId: 'conversation-1',
        projectId: 'project-1',
        fileName: 'unknown.csv',
        mediaType: 'text/csv',
        sizeBytes: 12,
        status: 'mystery',
        revision: 1,
        createdAt: '2026-07-23T09:00:00Z',
        expiresAt: '2026-08-22T09:00:00Z',
      }] as T
    })

    const operation = local.listAiSources({
      conversationId: 'conversation-1',
      projectId: 'project-1',
    })
    await expect(operation).rejects.toBeInstanceOf(GatewayError)
    await expect(operation).rejects.toMatchObject({ code: 'invalid_ai_source_status' })
  })

  it('keeps workspace settings local while exposing per-user Server AI settings', () => {
    const remote = new RemoteHttpGateway({
      baseUrl: 'https://lab.example/api/v1',
      fetch: vi.fn() as unknown as typeof fetch,
    })

    expect('getWorkspaceSettings' in remote).toBe(false)
    expect(typeof remote.getAiSettings).toBe('function')
    expect(typeof remote.saveAiSettings).toBe('function')
    expect(typeof remote.clearAiApiKey).toBe('function')
  })

  it('uses an HttpOnly cookie session and attaches the in-memory CSRF token to mutations', async () => {
    const requests: Array<{ url: string; init?: RequestInit }> = []
    const fetchRequest = vi.fn(async (input: string | URL | Request, init?: RequestInit) => {
      const url = String(input)
      requests.push({ url, init })
      if (url.endsWith('/auth/login')) {
        return new Response(JSON.stringify({
          data: {
            user: {
              id: 'user-1',
              lab_id: 'lab-1',
              email: 'researcher@example.org',
              display_name: '林研究员',
              lab_roles: ['lab_admin'],
              project_roles: [{ project_id: 'project-1', role: 'editor' }],
              authentication: 'session',
            },
            csrf_token: 'mac-memory-only',
            expires_at: '2026-07-19T10:00:00Z',
          },
          request_id: 'req-login',
        }), { status: 200, headers: { 'Content-Type': 'application/json' } })
      }
      if (url.endsWith('/auth/logout')) return new Response(null, { status: 204 })
      return new Response(JSON.stringify({
        data: { id: 'cage-1', section: 'SPF-A', display_id: 'A01', location: 'R1', capacity: 5 },
        request_id: 'req-cage',
      }), { status: 201, headers: { 'Content-Type': 'application/json' } })
    }) as unknown as typeof fetch
    const gateway = new RemoteHttpGateway({ baseUrl: 'https://lab.example/api/v1', fetch: fetchRequest })

    const session = await gateway.login({
      email: 'researcher@example.org',
      password: 'correct horse battery staple',
    })
    await gateway.createCage({ code: 'A01', room: 'SPF-A', rack: 'R1', capacity: 5 })
    await gateway.logout()

    expect(session).toEqual(expect.objectContaining({
      csrfAvailable: true,
      user: expect.objectContaining({ displayName: '林研究员', labId: 'lab-1' }),
    }))
    expect(JSON.stringify(session)).not.toContain('correct horse battery staple')
    const mutation = requests.find((request) => request.url.endsWith('/cages'))
    expect(new Headers(mutation?.init?.headers).get('X-CSRF-Token')).toBe('mac-memory-only')
    expect(new Headers(mutation?.init?.headers).has('Authorization')).toBe(false)
    expect(mutation?.init?.credentials).toBe('include')
    const logout = requests.find((request) => request.url.endsWith('/auth/logout'))
    expect(new Headers(logout?.init?.headers).get('X-CSRF-Token')).toBe('mac-memory-only')
    expect(logout?.init?.method).toBe('POST')
  })

  it('updates the current profile and password through CSRF-protected account routes', async () => {
    const requests: Array<{ url: string; init?: RequestInit }> = []
    const baseUser = {
      id: 'user-1', lab_id: 'lab-1', email: 'researcher@example.org',
      display_name: 'Researcher', lab_roles: ['animal_manager'], project_roles: [],
      authentication: 'session', must_change_password: false, is_environment_root: false,
    }
    const fetchRequest = vi.fn(async (input: string | URL | Request, init?: RequestInit) => {
      const url = String(input)
      requests.push({ url, init })
      if (url.endsWith('/auth/login')) {
        return new Response(JSON.stringify({
          data: { user: baseUser, csrf_token: 'csrf-account', expires_at: '2026-07-19T10:00:00Z' },
          request_id: 'req-login',
        }), { status: 200, headers: { 'Content-Type': 'application/json' } })
      }
      const data = url.endsWith('/auth/profile')
        ? { ...baseUser, display_name: 'Updated Researcher' }
        : { ...baseUser, display_name: 'Updated Researcher', must_change_password: false }
      return new Response(JSON.stringify({ data, request_id: 'req-account' }), {
        status: 200, headers: { 'Content-Type': 'application/json' },
      })
    }) as unknown as typeof fetch
    const remote = new RemoteHttpGateway({
      baseUrl: 'https://lab.example/api/v1',
      fetch: fetchRequest,
    })

    await remote.login({ email: 'researcher@example.org', password: 'login-password' })
    const profiled = await remote.updateProfile({ displayName: 'Updated Researcher' })
    const changed = await remote.changePassword({
      currentPassword: 'login-password',
      newPassword: 'replacement-password',
    })

    expect(profiled.user.displayName).toBe('Updated Researcher')
    expect(changed.user.mustChangePassword).toBe(false)
    expect(remote.currentSession).toBe(changed)
    expect(JSON.stringify(changed)).not.toContain('login-password')
    expect(JSON.stringify(changed)).not.toContain('replacement-password')
    const profile = requests.find((request) => request.url.endsWith('/auth/profile'))!
    const password = requests.find((request) => request.url.endsWith('/auth/password/change'))!
    expect(profile.init?.method).toBe('PATCH')
    expect(password.init?.method).toBe('POST')
    expect(new Headers(profile.init?.headers).get('X-CSRF-Token')).toBe('csrf-account')
    expect(new Headers(password.init?.headers).get('X-CSRF-Token')).toBe('csrf-account')
    expect(JSON.parse(String(profile.init?.body))).toEqual({ displayName: 'Updated Researcher' })
    expect(JSON.parse(String(password.init?.body))).toEqual({
      currentPassword: 'login-password',
      newPassword: 'replacement-password',
    })
  })

  it('restores the cookie user and obtains a fresh in-memory CSRF token', async () => {
    const fetchMock = vi.fn(async (input: string | URL | Request, _init?: RequestInit) => {
      const url = String(input)
      const data = url.endsWith('/auth/csrf')
        ? {
            csrf_token: 'mac-restored',
            expires_at: '2026-07-19T11:00:00Z',
          }
        : {
            id: 'user-1',
            lab_id: 'lab-1',
            email: null,
            display_name: '研究员',
            lab_roles: ['animal_manager'],
            project_roles: [],
            authentication: 'session',
          }
      return new Response(JSON.stringify({
        data,
        request_id: url.endsWith('/auth/csrf') ? 'req-csrf' : 'req-session',
      }), { status: 200, headers: { 'Content-Type': 'application/json' } })
    })
    const fetchRequest = fetchMock as unknown as typeof fetch
    const gateway = new RemoteHttpGateway({ baseUrl: 'https://lab.example/api/v1', fetch: fetchRequest })

    const session = await gateway.restoreSession()

    expect(session.user.displayName).toBe('研究员')
    expect(session.csrfAvailable).toBe(true)
    expect(session.expiresAt).toBe('2026-07-19T11:00:00Z')
    expect(fetchRequest).toHaveBeenCalledTimes(2)
    expect(await gateway.restoreSession()).toBe(session)
    expect(fetchRequest).toHaveBeenCalledTimes(2)

    await gateway.createCage({ code: 'A02', room: 'SPF-A', rack: 'R1', capacity: 5 })
    const mutation = fetchMock.mock.calls.find(([input]) => String(input).endsWith('/cages'))
    expect(new Headers(mutation?.[1]?.headers).get('X-CSRF-Token')).toBe('mac-restored')
  })

  it('clears the remote session boundary and invokes the login redirect on 401', async () => {
    const onUnauthorized = vi.fn()
    const fetchRequest = vi.fn(async () => new Response(JSON.stringify({
      error: { code: 'unauthorized', message: '请先登录' },
    }), { status: 401, headers: { 'Content-Type': 'application/json' } })) as unknown as typeof fetch
    const gateway = new RemoteHttpGateway({
      baseUrl: 'https://lab.example/api/v1',
      fetch: fetchRequest,
      onUnauthorized,
    })

    await expect(gateway.restoreSession()).rejects.toMatchObject({ status: 401 })

    expect(onUnauthorized).toHaveBeenCalledTimes(1)
  })

  it('unwraps and maps Server collection envelopes', async () => {
    const fetchRequest = vi.fn(async (input: string | URL | Request) => {
      const url = String(input)
      const data = url.endsWith('/cages')
        ? [{ id: 'cage-1', section: 'SPF-A', display_id: 'A01', location: 'R1', capacity: 5 }]
        : [{
            id: 'animal-1', display_id: 'M-001', strain: 'C57BL/6J', sex: 'male',
            current_cage_id: 'cage-1', current_status: 'alive',
          }]
      return new Response(JSON.stringify({ data, count: data.length, request_id: 'req-1' }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      })
    }) as unknown as typeof fetch
    const gateway = new RemoteHttpGateway({ baseUrl: 'https://lab.example/api/v1', fetch: fetchRequest })

    const cages = await gateway.listCages()

    expect(cages).toEqual([expect.objectContaining({
      code: 'A01', room: 'SPF-A', rack: 'R1', capacity: 5, animalIds: ['animal-1'], status: 'normal',
    })])
    expect(fetchRequest).toHaveBeenCalledTimes(2)
  })

  it('loads and reverses the Server animal timeline for detail views', async () => {
    const urls: string[] = []
    const fetchRequest = vi.fn(async (input: string | URL | Request) => {
      const url = String(input)
      urls.push(url)
      const data = url.includes('/events')
        ? [{
            id: 'event-1', kind: { type: 'transferred', from_cage_id: null, to_cage_id: 'cage-1' },
            occurred_at: '2026-07-18T01:00:00Z', recorded_by: 'user-1', notes: null,
          }]
        : url.includes('/animal-overviews')
          ? [{
              animal: {
                id: 'animal-1', display_id: 'M-001', strain: 'C57BL/6J', sex: 'male',
                current_cage_id: 'cage-1', current_status: 'alive',
              },
              genotype: '待确认', projects: [], latest_weight: null,
            }]
        : url.includes('/cages')
          ? [{ id: 'cage-1', section: 'SPF-A', display_id: 'A01', location: 'R1', capacity: 5 }]
          : {
              id: 'animal-1', display_id: 'M-001', strain: 'C57BL/6J', sex: 'male',
              current_cage_id: 'cage-1', current_status: 'alive',
            }
      const envelope = Array.isArray(data)
        ? { data, count: data.length, request_id: 'req-1' }
        : { data, request_id: 'req-1' }
      return new Response(JSON.stringify(envelope), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      })
    }) as unknown as typeof fetch
    const gateway = new RemoteHttpGateway({ baseUrl: 'https://lab.example/api/v1', fetch: fetchRequest })

    const animal = await gateway.getAnimal('animal-1', {
      projectId: 'ed8f6474-a192-4f1e-bb5c-51032ca94c80',
    })

    expect(animal?.timeline).toEqual([expect.objectContaining({
      type: 'transfer', detail: '未分配 → A01', operator: '实验室用户',
    })])
    expect(urls).toContain('https://lab.example/api/v1/animals/animal-1?project_id=ed8f6474-a192-4f1e-bb5c-51032ca94c80')
    expect(urls).toContain('https://lab.example/api/v1/animals/animal-1/events?project_id=ed8f6474-a192-4f1e-bb5c-51032ca94c80')
  })

  it('uses CSRF-protected Server AI settings, turns and approval routes', async () => {
    const requests: Array<{ url: string; init?: RequestInit }> = []
    const fetchRequest = vi.fn(async (input: string | URL | Request, init?: RequestInit) => {
      const url = String(input)
      requests.push({ url, init })
      if (url.endsWith('/auth/login')) {
        return new Response(JSON.stringify({ data: {
          user: { id: 'user-1', lab_id: 'lab-1', display_name: '研究者', lab_roles: [], project_roles: [], authentication: 'session' },
          csrf_token: 'csrf-ai', expires_at: '2026-07-19T10:00:00Z',
        }, request_id: 'req-login' }), { status: 200, headers: { 'Content-Type': 'application/json' } })
      }
      if (url.endsWith('/ai/settings')) {
        return new Response(JSON.stringify({ data: {
          enabled: true, providerKind: 'open_ai_compatible', model: 'gpt-test',
          baseUrl: 'https://api.example/v1', hasKey: true, revision: 2,
        }, request_id: 'req-settings' }), { status: 200, headers: { 'Content-Type': 'application/json' } })
      }
      if (url.endsWith('/ai/turns')) {
        return new Response(JSON.stringify({ data: {
          conversationId: 'conversation-1', content: '查询完成', citations: [], toolRuns: [], drafts: [],
          trace: { providerId: 'server-provider', model: 'gpt-test', usage: {
            provider_calls: 1, tool_calls: 0, input_tokens: 4, output_tokens: 2, total_tokens: 6,
          } },
        }, request_id: 'req-turn' }), { status: 200, headers: { 'Content-Type': 'application/json' } })
      }
      return new Response(JSON.stringify({ data: [], count: 0, request_id: 'req-list' }), {
        status: 200, headers: { 'Content-Type': 'application/json' },
      })
    }) as unknown as typeof fetch
    const remote = new RemoteHttpGateway({ baseUrl: 'https://lab.example/api/v1', fetch: fetchRequest })
    await remote.login({ email: 'r@example.org', password: 'not-retained' })

    const settings = await remote.saveAiSettings({
      enabled: true, providerKind: 'open_ai_compatible', providerPresetId: 'custom-openai-compatible', model: 'gpt-test',
      baseUrl: 'https://api.example/v1', contextWindowTokens: 131072, maxInputTokens: 65536,
      maxOutputTokens: 4096, historyTokenBudget: 32768, historyTurns: 20,
      temperature: 0, timeoutMs: 120000, apiKey: 'write-only-key',
    })
    const turn = await remote.aiTurn({
      conversationId: 'conversation-1',
      projectId: 'project-1',
      message: '总结进度',
    })
    await remote.listAiDrafts('project-1', 'pending_approval')
    await remote.decideAiDraft('draft-1', {
      expectedRevision: 2,
      decision: 'approve',
      statement: '我已核对完整导入预览',
      currentPassword: 'one-request-password',
    })

    expect(settings.hasKey).toBe(true)
    expect(settings).not.toHaveProperty('apiKey')
    expect(turn.trace.usage.totalTokens).toBe(6)
    const mutations = requests.filter((request) => ['/ai/settings', '/ai/turns'].some((path) => request.url.endsWith(path)))
    expect(mutations.every((request) => new Headers(request.init?.headers).get('X-CSRF-Token') === 'csrf-ai')).toBe(true)
    expect(requests.some((request) => request.url.endsWith('/ai/approvals?project_id=project-1&status=pending_approval'))).toBe(true)
    const decision = requests.find((request) => request.url.endsWith('/ai/approvals/draft-1/decision'))
    expect(new Headers(decision?.init?.headers).get('X-CSRF-Token')).toBe('csrf-ai')
    expect(JSON.parse(String(decision?.init?.body))).toEqual({
      expectedRevision: 2,
      decision: 'approve',
      statement: '我已核对完整导入预览',
      currentPassword: 'one-request-password',
    })
    expect(String(decision?.init?.body)).not.toContain('stepUpVerified')
  })

  it('keeps an explicit lab-wide AI scope independent from the top-bar project', async () => {
    const requests: Array<{ url: string; init?: RequestInit }> = []
    currentProjectId.value = 'top-bar-project'
    const fetchRequest = vi.fn(async (input: string | URL | Request, init?: RequestInit) => {
      const url = String(input)
      requests.push({ url, init })
      if (url.endsWith('/auth/login')) {
        return new Response(JSON.stringify({
          data: {
            user: {
              id: 'lab-reader',
              lab_id: 'lab-1',
              display_name: 'Lab reader',
              lab_roles: [],
              project_roles: [],
              authentication: 'session',
            },
            csrf_token: 'csrf-lab-scope',
            expires_at: '2026-07-24T12:00:00Z',
          },
          request_id: 'req-lab-login',
        }), { status: 200, headers: { 'Content-Type': 'application/json' } })
      }
      if (url.endsWith('/ai/turns')) {
        return new Response(JSON.stringify({
          data: {
            conversationId: 'lab-conversation',
            content: '只读查询完成',
            citations: [],
            toolRuns: [],
            drafts: [],
            trace: {
              providerId: 'server-provider',
              model: 'gpt-test',
              usage: {
                provider_calls: 1,
                tool_calls: 0,
                input_tokens: 2,
                output_tokens: 2,
                total_tokens: 4,
              },
            },
          },
          request_id: 'req-lab-turn',
        }), { status: 200, headers: { 'Content-Type': 'application/json' } })
      }
      return new Response(JSON.stringify({
        data: [],
        count: 0,
        request_id: 'req-lab-drafts',
      }), { status: 200, headers: { 'Content-Type': 'application/json' } })
    }) as unknown as typeof fetch
    const remote = new RemoteHttpGateway({
      baseUrl: 'https://lab.example/api/v1',
      fetch: fetchRequest,
    })
    await remote.login({ email: 'reader@example.org', password: 'not-retained' })

    try {
      await remote.aiTurn({
        conversationId: 'lab-conversation',
        message: '跨项目只读汇总',
      })
      await remote.listAiDrafts(undefined, 'pending_approval')
    } finally {
      currentProjectId.value = undefined
    }

    const turn = requests.find((request) => request.url.endsWith('/ai/turns'))
    expect(JSON.parse(String(turn?.init?.body))).toEqual({
      conversationId: 'lab-conversation',
      message: '跨项目只读汇总',
    })
    expect(requests.some((request) =>
      request.url.endsWith('/ai/approvals?status=pending_approval'))).toBe(true)
    expect(JSON.stringify(requests)).not.toContain('top-bar-project')
  })

  it('uses session-only CSRF-protected admin routes without retaining passwords', async () => {
    const requests: Array<{ url: string; init?: RequestInit }> = []
    const managedUser = {
      id: 'user-2', email: 'student@example.org', displayName: 'Student', status: 'active',
      revision: 1, credentialRevision: 1, mustChangePassword: true,
      isEnvironmentRoot: false, labRole: undefined, labMembershipId: undefined,
      labMembershipRevision: undefined, projectMemberships: [],
      createdAt: '2026-07-19T01:00:00Z', updatedAt: '2026-07-19T01:00:00Z',
    }
    const fetchRequest = vi.fn(async (input: string | URL | Request, init?: RequestInit) => {
      const url = String(input)
      requests.push({ url, init })
      if (url.endsWith('/auth/login')) {
        return new Response(JSON.stringify({
          data: {
            user: {
              id: 'admin-1', lab_id: 'lab-1', email: 'admin@example.org',
              display_name: 'Lab Admin', lab_roles: ['lab_admin'], project_roles: [],
              authentication: 'session',
            },
            csrf_token: 'csrf-admin', expires_at: '2026-07-19T10:00:00Z',
          },
          request_id: 'login-admin',
        }), { status: 200, headers: { 'Content-Type': 'application/json' } })
      }
      return new Response(JSON.stringify({
        data: init?.method ? managedUser : [managedUser],
        count: init?.method ? undefined : 1,
        request_id: 'admin-request',
      }), { status: init?.method === 'POST' ? 201 : 200, headers: { 'Content-Type': 'application/json' } })
    }) as unknown as typeof fetch
    const remote = new RemoteHttpGateway({ baseUrl: 'https://lab.example/api/v1', fetch: fetchRequest })
    const session = await remote.login({ email: 'admin@example.org', password: 'login-password' })
    expect(remote.currentSession).toBe(session)
    expect(currentAuthSession.value).toBe(session)

    const users = await remote.listManagedUsers()
    const created = await remote.createManagedUser({
      email: 'student@example.org', displayName: 'Student',
      temporaryPassword: 'temporary-password', currentPassword: 'admin-password',
      projectRoles: [{ projectId: 'project-1', role: 'viewer' }],
    })
    await remote.updateManagedUserProfile('user-2', {
      expectedRevision: 1, email: 'updated-student@example.org',
      displayName: 'Updated Student', currentPassword: 'admin-password',
    })
    await remote.resetManagedUserPassword('user-2', {
      expectedCredentialRevision: 1, temporaryPassword: 'replacement-temporary-password',
      currentPassword: 'admin-password',
    })
    await remote.setManagedUserStatus('user-2', {
      expectedRevision: 1, status: 'suspended', currentPassword: 'admin-password',
    })
    await remote.grantLabRole('user-2', {
      expectedUserRevision: 1, role: 'animal_manager', currentPassword: 'admin-password',
    })
    await remote.updateLabRole('membership-lab', {
      expectedRevision: 1, role: 'lab_admin', currentPassword: 'admin-password',
    })
    await remote.grantProjectRole('user-2', {
      expectedUserRevision: 1, projectId: 'project-1', role: 'viewer',
      currentPassword: 'admin-password',
    })
    await remote.updateProjectRole('membership-project', {
      expectedRevision: 1, role: 'editor', currentPassword: 'admin-password',
    })
    await remote.revokeMembership('membership-project', {
      expectedRevision: 2, currentPassword: 'admin-password',
    })

    expect(users).toHaveLength(1)
    expect(created).not.toHaveProperty('temporaryPassword')
    expect(created).not.toHaveProperty('currentPassword')
    expect(JSON.stringify(created)).not.toContain('admin-password')
    const adminRequests = requests.filter((request) => request.url.includes('/admin/'))
    expect(adminRequests).toHaveLength(10)
    expect(adminRequests[0]?.init?.method).toBeUndefined()
    expect(adminRequests.slice(1).every((request) =>
      new Headers(request.init?.headers).get('X-CSRF-Token') === 'csrf-admin')).toBe(true)
    expect(adminRequests.every((request) =>
      new Headers(request.init?.headers).has('Authorization') === false)).toBe(true)
    expect(adminRequests.some((request) => request.init?.method === 'DELETE')).toBe(true)
    const createBody = JSON.parse(String(adminRequests[1]?.init?.body))
    expect(createBody).toEqual(expect.objectContaining({
      temporaryPassword: 'temporary-password', currentPassword: 'admin-password',
    }))
    expect(createBody).not.toHaveProperty('labRole')
    expect(createBody).not.toHaveProperty('stepUpVerified')
    const profile = adminRequests.find((request) => request.url.endsWith('/users/user-2/profile'))!
    expect(profile.init?.method).toBe('PATCH')
    expect(JSON.parse(String(profile.init?.body))).toEqual({
      expectedRevision: 1,
      email: 'updated-student@example.org',
      displayName: 'Updated Student',
      currentPassword: 'admin-password',
    })
    const reset = adminRequests.find((request) => request.url.endsWith('/users/user-2/password-reset'))!
    expect(reset.init?.method).toBe('POST')
    expect(JSON.parse(String(reset.init?.body))).toEqual({
      expectedCredentialRevision: 1,
      temporaryPassword: 'replacement-temporary-password',
      currentPassword: 'admin-password',
    })
  })

  it('streams attachment bytes without accepting client paths, hashes, or sizes', async () => {
    const requests: Array<{ url: string; init?: RequestInit }> = []
    const attachment = {
      id: 'attachment-1', project_id: 'project-1', entity_type: 'animal', entity_id: 'animal-1',
      file_name: 'weight.csv', media_type: 'text/csv', size_bytes: 7, sha256: 'a'.repeat(64),
      version: 1, content_href: '/api/v1/attachments/attachment-1/content',
      meta: { created_at: '2026-07-19T01:00:00Z', revision: 3 },
    }
    const fetchRequest = vi.fn(async (input: string | URL | Request, init?: RequestInit) => {
      const url = String(input)
      requests.push({ url, init })
      if (url.endsWith('/auth/login')) {
        return new Response(JSON.stringify({ data: {
          user: { id: 'user-1', lab_id: 'lab-1', display_name: 'Researcher', lab_roles: [], project_roles: [], authentication: 'session' },
          csrf_token: 'csrf-attachment', expires_at: '2026-07-19T10:00:00Z',
        }, request_id: 'req-login' }), { status: 200, headers: { 'Content-Type': 'application/json' } })
      }
      if (url.includes('/attachments/upload?')) {
        return new Response(JSON.stringify({ data: attachment, request_id: 'req-upload' }), {
          status: 201, headers: { 'Content-Type': 'application/json' },
        })
      }
      if (url.endsWith('/attachments/attachment-1/content')) {
        return new Response('a,b\n1,2', { status: 200, headers: { 'Content-Type': 'text/csv' } })
      }
      if (url.endsWith('/attachments/attachment-1') && init?.method === 'DELETE') {
        return new Response(JSON.stringify({ data: attachment, request_id: 'req-delete' }), {
          status: 200, headers: { 'Content-Type': 'application/json' },
        })
      }
      return new Response(JSON.stringify({ data: [attachment], count: 1, request_id: 'req-list' }), {
        status: 200, headers: { 'Content-Type': 'application/json' },
      })
    }) as unknown as typeof fetch
    const gateway = new RemoteHttpGateway({ baseUrl: 'https://lab.example/api/v1', fetch: fetchRequest })
    await gateway.login({ email: 'r@example.org', password: 'not-retained' })
    const content = new Blob(['a,b\n1,2'], { type: 'text/csv' })

    const uploaded = await gateway.uploadAttachment({
      entityType: 'animal', entityId: 'animal-1', projectId: 'project-1',
      fileName: 'weight.csv', mediaType: 'text/csv', content,
    })
    const listed = await gateway.listAttachments({
      entityType: 'animal', entityId: 'animal-1', projectId: 'project-1',
    })
    const downloaded = await gateway.downloadAttachment('attachment-1')
    await gateway.deleteAttachment({
      id: 'attachment-1',
      expectedRevision: uploaded.revision,
      reason: 'remove duplicate',
    })

    expect(uploaded).toEqual(expect.objectContaining({ id: 'attachment-1', sizeBytes: 7, version: 1 }))
    expect(uploaded.revision).toBe(3)
    expect(listed).toHaveLength(1)
    expect(downloaded).toEqual(expect.objectContaining({ size: 7, type: 'text/csv' }))
    const upload = requests.find((request) => request.url.includes('/attachments/upload?'))!
    const uploadUrl = new URL(upload.url)
    expect(uploadUrl.searchParams.get('file_name')).toBe('weight.csv')
    expect(uploadUrl.searchParams.has('relative_path')).toBe(false)
    expect(uploadUrl.searchParams.has('sha256')).toBe(false)
    expect(uploadUrl.searchParams.has('size_bytes')).toBe(false)
    expect(upload.init?.body).toBe(content)
    const uploadHeaders = new Headers(upload.init?.headers)
    expect(uploadHeaders.get('Content-Type')).toBe('text/csv')
    expect(uploadHeaders.get('X-CSRF-Token')).toBe('csrf-attachment')
    const deleted = requests.find((request) => request.url.endsWith('/attachments/attachment-1') && request.init?.method === 'DELETE')!
    expect(JSON.parse(String(deleted.init?.body))).toEqual({
      expected_revision: 3,
      reason: 'remove duplicate',
    })
    expect(new Headers(deleted.init?.headers).get('X-CSRF-Token')).toBe('csrf-attachment')
  })

  it('maps the scoped animal detail resource without exposing audit snapshots', async () => {
    const urls: string[] = []
    const rawDetail = {
      events: [{
        id: 'event-1', kind: { type: 'sample_collected', sample_id: 'sample-1' },
        occurred_at: '2026-07-19T08:30:00Z', recorded_by: 'user-1', notes: null,
      }],
      experiments: [{
        project: { id: 'project-1', name: 'DEMO' },
        experiment: { id: 'experiment-1', name: 'DEMO-001', status: 'active', revision: 2 },
        participation: {
          id: 'participation-1', status: 'enrolled',
          enrolled_at: '2026-07-01T00:00:00Z', revision: 3,
        },
        cohort: { id: 'cohort-1', name: 'Control' },
      }],
      measurements: [{
        id: 'measurement-1', project_id: 'project-1', experiment_id: 'experiment-1',
        key: 'body_weight', label: '体重', value: { type: 'number', value: 23.4 }, unit: 'g',
        measured_at: '2026-07-18T08:00:00Z', status: 'signed', revision: 4,
      }],
      pedigree: [{
        id: 'pedigree-1', direction: 'parent', parent_type: 'father', revision: 1,
        related_animal: {
          id: 'animal-2', display_id: 'M-002', sex: 'male', strain: 'C57BL/6J',
          current_status: 'alive',
        },
      }],
      samples: [{
        id: 'sample-1', project_id: 'project-1', experiment_id: 'experiment-1',
        sample_type: 'lung tissue', quantity: 12.5, unit: 'mg', location: '-80C/A3',
        collected_at: '2026-07-19T08:30:00Z', revision: 1,
      }],
      attachments: [{
        id: 'attachment-1', project_id: 'project-1', entity_type: 'animal',
        entity_id: 'animal-1', file_name: 'observation.txt', media_type: 'text/plain',
        size_bytes: 7, sha256: 'a'.repeat(64), version: 1,
        content_href: '/api/v1/attachments/attachment-1/content',
        created_at: '2026-07-19T09:00:00Z', revision: 1,
      }],
      audit_visible: true,
      audits: [{
        id: 'audit-1', action: 'create', actor: 'Researcher', source: 'web',
        occurred_at: '2026-07-19T09:00:00Z', revision: 1,
        before: { secret: 'must not map' }, after: { secret: 'must not map' },
      }],
      provenance: [{
        id: 'provenance-1', source: 'human', actor: 'Researcher',
        recorded_at: '2026-07-19T09:00:00Z', request_id: 'request-1',
      }],
    }
    const fetchRequest = vi.fn(async (input: string | URL | Request) => {
      const url = String(input)
      urls.push(url)
      if (url.endsWith('/cages')) {
        return new Response(JSON.stringify({ data: [], count: 0, request_id: 'req-cages' }), {
          status: 200, headers: { 'Content-Type': 'application/json' },
        })
      }
      return new Response(JSON.stringify({ data: rawDetail, request_id: 'req-detail' }), {
        status: 200, headers: { 'Content-Type': 'application/json' },
      })
    }) as unknown as typeof fetch
    const gateway = new RemoteHttpGateway({ baseUrl: 'https://lab.example/api/v1', fetch: fetchRequest })

    const detail = await gateway.getAnimalDetail('animal-1', { projectId: 'project-1' })

    expect(urls).toContain('https://lab.example/api/v1/animals/animal-1/detail?limit=500&project_id=project-1')
    expect(detail.experiments[0]).toEqual(expect.objectContaining({
      projectName: 'DEMO', experimentName: 'DEMO-001', cohortName: 'Control', revision: 3,
    }))
    expect(detail.measurements[0]).toEqual(expect.objectContaining({
      label: '体重', value: { type: 'number', value: 23.4 }, unit: 'g', status: 'signed',
    }))
    expect(detail.pedigree[0]).toEqual(expect.objectContaining({
      direction: 'parent', parentType: 'father', relatedAnimal: expect.objectContaining({ code: 'M-002' }),
    }))
    expect(detail.samples[0]).toEqual(expect.objectContaining({ sampleType: 'lung tissue', quantity: 12.5 }))
    expect(detail.attachments[0]).toEqual(expect.objectContaining({ fileName: 'observation.txt', sizeBytes: 7 }))
    expect(detail.audits[0]).toEqual(expect.objectContaining({ actor: 'Researcher', revision: 1 }))
    expect(detail.audits[0]).not.toHaveProperty('before')
    expect(detail.audits[0]).not.toHaveProperty('after')
  })

  it('keeps unscoped animal detail requests backward compatible', async () => {
    const urls: string[] = []
    const fetchRequest = vi.fn(async (input: string | URL | Request) => {
      const url = String(input)
      urls.push(url)
      const data = url.includes('/events')
        ? []
        : url.includes('/animal-overviews')
          ? [{
              animal: {
                id: 'animal-1', display_id: 'M-001', strain: 'C57BL/6J', sex: 'male',
                current_cage_id: null, current_status: 'alive',
              },
              genotype: '待确认', projects: [], latest_weight: null,
            }]
        : url.endsWith('/cages')
          ? []
          : {
              id: 'animal-1', display_id: 'M-001', strain: 'C57BL/6J', sex: 'male',
              current_cage_id: null, current_status: 'alive',
            }
      const envelope = Array.isArray(data)
        ? { data, count: data.length, request_id: 'req-1' }
        : { data, request_id: 'req-1' }
      return new Response(JSON.stringify(envelope), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      })
    }) as unknown as typeof fetch
    const gateway = new RemoteHttpGateway({ baseUrl: 'https://lab.example/api/v1', fetch: fetchRequest })

    await gateway.getAnimal('animal-1')

    expect(urls).toContain('https://lab.example/api/v1/animals/animal-1')
    expect(urls).toContain('https://lab.example/api/v1/animals/animal-1/events')
    expect(urls.some((url) => url.includes('project_id='))).toBe(false)
  })

  it('preserves failed and cancelled Job terminal states', async () => {
    const fetchRequest = vi.fn(async () => new Response(JSON.stringify({
      data: [
        {
          id: 'job-failed', kind: 'import', status: 'failed',
          progress_current: 2, progress_total: 5,
          result_available: false, error_report_available: true,
          cancellation_requested: false, revision: 2,
          created_at: '2026-07-19T09:00:00Z', updated_at: '2026-07-19T09:01:00Z',
        },
        {
          id: 'job-cancelled', kind: 'export', status: 'cancelled',
          progress_current: 1, progress_total: 4,
          result_available: false, error_report_available: false,
          cancellation_requested: true, revision: 3,
          created_at: '2026-07-19T09:05:00Z', updated_at: '2026-07-19T09:06:00Z',
        },
      ],
      count: 2,
      request_id: 'req-jobs',
    }), { status: 200, headers: { 'Content-Type': 'application/json' } })) as unknown as typeof fetch
    const gateway = new RemoteHttpGateway({
      baseUrl: 'https://lab.example/api/v1',
      fetch: fetchRequest,
    })

    const jobs = await gateway.listDataJobs()

    expect(jobs.map((job) => job.status)).toEqual(['failed', 'cancelled'])
    expect(jobs[0].detail).toContain('失败')
    expect(jobs[1].detail).toContain('取消')
  })

  it('automatically scopes Project Viewer animal, experiment, and template lists', async () => {
    const urls: string[] = []
    currentAuthSession.value = {
      user: {
        id: 'viewer-1', labId: 'lab-1', displayName: 'Viewer', labRoles: [],
        projectRoles: [{ projectId: 'project-1', role: 'viewer' }],
        authentication: 'session',
        mustChangePassword: false,
        isEnvironmentRoot: false,
      },
      csrfAvailable: true,
    }
    currentProjectId.value = 'project-1'
    const fetchRequest = vi.fn(async (input: string | URL | Request) => {
      const url = String(input)
      urls.push(url)
      const data = url.endsWith('/projects')
        ? [{ id: 'project-1', name: 'Project one' }]
        : []
      return new Response(JSON.stringify({ data, count: data.length, request_id: 'req-1' }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      })
    }) as unknown as typeof fetch
    const gateway = new RemoteHttpGateway({
      baseUrl: 'https://lab.example/api/v1',
      fetch: fetchRequest,
    })

    try {
      await Promise.all([
        gateway.listAnimals(),
        gateway.listPublishedTemplates(),
        gateway.listExperiments(),
      ])
    } finally {
      currentProjectId.value = undefined
      currentAuthSession.value = undefined
    }

    expect(urls.some((url) => url.includes('/animal-overviews?') && url.includes('project_id=project-1'))).toBe(true)
    expect(urls).toContain('https://lab.example/api/v1/experiment-template-versions?project_id=project-1')
    expect(urls).toContain('https://lab.example/api/v1/experiments?project_id=project-1')
    expect(urls.some((url) => url.endsWith('/cages'))).toBe(false)
  })

  it('maps new Tauri research entities and strips Server-only project scope fields', async () => {
    const calls: Array<[string, Record<string, unknown> | undefined]> = []
    const meta = {
      created_at: '2026-07-19T01:00:00Z', updated_at: '2026-07-19T01:00:00Z',
      deleted_at: null, revision: 1,
    }
    const invokeCommand = async <T>(command: string, args?: Record<string, unknown>): Promise<T> => {
      calls.push([command, args])
      if (command === 'create_genotype_definition') {
        return {
          id: 'definition-1', lab_id: 'lab-1', name: 'GeneA fl/fl', description: null,
          components: [{
            id: 'component-1', genotype_definition_id: 'definition-1', locus_id: 'locus-1',
            allele_1_id: 'allele-1', allele_2_id: 'allele-1', mode: 'diploid',
            display_order: 0, meta,
          }],
          meta,
        } as T
      }
      if (command === 'create_breeding_pair') {
        return {
          id: 'pair-1', lab_id: 'lab-1', colony_id: 'colony-1', name: 'Pair A',
          status: 'active', started_at: '2026-07-19T01:00:00Z', ended_at: null,
          members: [{
            id: 'member-1', breeding_pair_id: 'pair-1', animal_id: 'animal-male',
            role: 'male', joined_at: '2026-07-19T01:00:00Z', left_at: null, meta,
          }, {
            id: 'member-2', breeding_pair_id: 'pair-1', animal_id: 'animal-female',
            role: 'female', joined_at: '2026-07-19T01:00:00Z', left_at: null, meta,
          }],
          meta,
        } as T
      }
      return {
        observation: {
          id: 'observation-1', lab_id: 'lab-1', project_id: 'project-1',
          experiment_id: 'experiment-1', experiment_event_id: 'event-1',
          definition_id: 'observation-definition-1', subject_type: 'animal',
          subject_id: 'animal-1', context: {}, current_value_version: 1, meta,
        },
        value: {
          id: 'value-1', observation_id: 'observation-1', version: 1,
          value: { type: 'number', value: 24.2 }, recorded_at: '2026-07-19T02:00:00Z',
          recorded_by: 'user-1', notes: null, meta,
        },
      } as T
    }
    const local = new LocalTauriGateway(invokeCommand)

    const definition = await local.createGenotypeDefinition({
      projectId: 'server-only-project', name: 'GeneA fl/fl', components: [{
        locusId: 'locus-1', allele1Id: 'allele-1', allele2Id: 'allele-1',
        mode: 'diploid', displayOrder: 0,
      }],
    })
    const pair = await local.createBreedingPair({
      projectId: 'server-only-project', colonyId: 'colony-1', name: 'Pair A',
      maleAnimalId: 'animal-male', femaleAnimalIds: ['animal-female'],
    })
    const recorded = await local.recordObservation({
      experimentId: 'experiment-1', experimentEventId: 'event-1',
      definitionId: 'observation-definition-1', subjectType: 'animal', subjectId: 'animal-1',
      value: { type: 'number', value: 24.2 },
    })

    expect(definition.components[0]).toEqual(expect.objectContaining({
      locusId: 'locus-1', allele2Id: 'allele-1', displayOrder: 0,
    }))
    expect(pair.members.map((member) => member.role)).toEqual(['male', 'female'])
    expect(recorded.value).toEqual(expect.objectContaining({
      version: 1, value: { type: 'number', value: 24.2 }, recordedBy: 'user-1',
    }))
    expect(calls[0]?.[1]).toEqual({ input: {
      name: 'GeneA fl/fl', components: [{
        locusId: 'locus-1', allele1Id: 'allele-1', allele2Id: 'allele-1',
        mode: 'diploid', displayOrder: 0,
      }],
    } })
    expect(calls[1]?.[1]).toEqual({ input: {
      colonyId: 'colony-1', name: 'Pair A', maleAnimalId: 'animal-male',
      femaleAnimalIds: ['animal-female'],
    } })
    expect(JSON.stringify(calls.slice(0, 2))).not.toContain('server-only-project')
  })
})

describe('multi-model gateway contracts', () => {
  const modelInput = {
    name: '自由模型',
    protocol: 'openai_responses' as const,
    transport: 'open_ai_compatible' as const,
    baseUrl: 'https://provider.example/v1',
    modelId: 'organization/model:latest',
    supportsVision: true,
    contextWindowTokens: 131072,
    maxInputTokens: 65536,
    maxOutputTokens: 4096,
    historyTokenBudget: 32768,
    historyTurns: 20,
    temperature: 0,
    timeoutMs: 120000,
    apiKey: 'write-only-secret',
  }
  const validationInput = {
    protocol: modelInput.protocol,
    transport: modelInput.transport,
    baseUrl: modelInput.baseUrl,
    modelId: modelInput.modelId,
    supportsVision: modelInput.supportsVision,
    contextWindowTokens: modelInput.contextWindowTokens,
    maxInputTokens: modelInput.maxInputTokens,
    maxOutputTokens: modelInput.maxOutputTokens,
    historyTokenBudget: modelInput.historyTokenBudget,
    historyTurns: modelInput.historyTurns,
    temperature: modelInput.temperature,
    timeoutMs: modelInput.timeoutMs,
    apiKey: modelInput.apiKey,
  }

  it('uses stable, semantic Tauri commands for every model operation', async () => {
    const calls: Array<[string, Record<string, unknown> | undefined]> = []
    const invokeCommand = async <T>(command: string, args?: Record<string, unknown>): Promise<T> => {
      calls.push([command, args])
      return (command === 'list_ai_model_profiles' ? [] : {}) as T
    }
    const local = new LocalTauriGateway(invokeCommand)

    await local.listAiModelProfiles()
    await local.getAiModelProfile('profile-1')
    await local.createAiModelProfile(modelInput)
    await local.updateAiModelProfile('profile-1', { ...modelInput, expectedRevision: 3 })
    await local.validateAiModelProfile({
      ...validationInput,
      profileId: 'profile-1',
      currentVersion: 2,
    })
    await local.clearAiModelProfileKey('profile-1')
    await local.archiveAiModelProfile('profile-1', 4)
    await local.getAiModelDefaults()
    await local.saveAiModelDefaults({
      defaultConversationProfileId: 'profile-1',
      defaultVisionProfileId: null,
      expectedRevision: 2,
    })

    expect(calls).toEqual([
      ['list_ai_model_profiles', undefined],
      ['get_ai_model_profile', { id: 'profile-1' }],
      ['create_ai_model_profile', { input: modelInput }],
      ['update_ai_model_profile', {
        id: 'profile-1',
        input: { ...modelInput, expectedRevision: 3 },
      }],
      ['validate_ai_model_profile', {
        input: { ...validationInput, profileId: 'profile-1', currentVersion: 2 },
      }],
      ['clear_ai_model_profile_key', { id: 'profile-1' }],
      ['archive_ai_model_profile', { id: 'profile-1', expectedRevision: 4 }],
      ['get_ai_model_defaults', undefined],
      ['save_ai_model_defaults', {
        input: {
          defaultConversationProfileId: 'profile-1',
          defaultVisionProfileId: null,
          expectedRevision: 2,
        },
      }],
    ])
  })

  it('uses versioned Server model routes and keeps write operations CSRF protected', async () => {
    const requests: Array<{ url: string; init?: RequestInit }> = []
    const profile = {
      id: 'profile-1',
      ...modelInput,
      apiKey: undefined,
      currentVersion: 1,
      revision: 1,
      hasKey: true,
      isDefaultConversation: false,
      isDefaultVision: false,
    }
    const fetchRequest = vi.fn(async (input: string | URL | Request, init?: RequestInit) => {
      const url = String(input)
      requests.push({ url, init })
      if (url.endsWith('/auth/login')) {
        return new Response(JSON.stringify({ data: {
          user: {
            id: 'user-1',
            lab_id: 'lab-1',
            display_name: '研究者',
            lab_roles: [],
            project_roles: [],
            authentication: 'session',
          },
          csrf_token: 'csrf-models',
          expires_at: '2026-07-23T10:00:00Z',
        } }), { status: 200, headers: { 'Content-Type': 'application/json' } })
      }
      if (url.endsWith('/ai/models/defaults')) {
        return new Response(JSON.stringify({ data: {
          defaultConversationProfileId: 'profile-1',
          revision: 2,
        } }), { status: 200, headers: { 'Content-Type': 'application/json' } })
      }
      if (url.endsWith('/ai/models/validate')) {
        return new Response(JSON.stringify({ data: { ok: true, latencyMs: 18 } }), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        })
      }
      if (url.endsWith('/ai/models') && !init?.method) {
        return new Response(JSON.stringify({ data: [profile], count: 1 }), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        })
      }
      return new Response(JSON.stringify({ data: profile }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      })
    }) as unknown as typeof fetch
    const remote = new RemoteHttpGateway({
      baseUrl: 'https://lab.example/api/v1',
      fetch: fetchRequest,
    })
    await remote.login({ email: 'researcher@example.test', password: 'not-retained' })

    await remote.listAiModelProfiles()
    await remote.getAiModelProfile('profile-1')
    await remote.createAiModelProfile(modelInput)
    await remote.updateAiModelProfile('profile-1', { ...modelInput, expectedRevision: 1 })
    await remote.validateAiModelProfile({
      ...validationInput,
      profileId: 'profile-1',
      currentVersion: 1,
    })
    await remote.clearAiModelProfileKey('profile-1')
    await remote.archiveAiModelProfile('profile-1', 2)
    await remote.getAiModelDefaults()
    await remote.saveAiModelDefaults({
      defaultConversationProfileId: 'profile-1',
      defaultVisionProfileId: null,
      expectedRevision: 2,
    })

    expect(requests.map(({ url, init }) => [
      url.replace('https://lab.example/api/v1', ''),
      init?.method ?? 'GET',
    ])).toEqual([
      ['/auth/login', 'POST'],
      ['/ai/models', 'GET'],
      ['/ai/models/profile-1', 'GET'],
      ['/ai/models', 'POST'],
      ['/ai/models/profile-1', 'PUT'],
      ['/ai/models/validate', 'POST'],
      ['/ai/models/profile-1/key', 'DELETE'],
      ['/ai/models/profile-1/archive', 'POST'],
      ['/ai/models/defaults', 'GET'],
      ['/ai/models/defaults', 'PUT'],
    ])
    const mutations = requests.slice(3)
      .filter(({ init }) => (init?.method ?? 'GET') !== 'GET')
    expect(mutations.every(({ init }) =>
      new Headers(init?.headers).get('X-CSRF-Token') === 'csrf-models')).toBe(true)
    const validationRequest = requests.find(({ url }) => url.endsWith('/ai/models/validate'))
    expect(JSON.parse(String(validationRequest?.init?.body))).toEqual({
      ...validationInput,
      profileId: 'profile-1',
      currentVersion: 1,
    })
    expect(JSON.parse(String(validationRequest?.init?.body))).not.toHaveProperty('name')
    expect(JSON.parse(String(requests.at(-1)?.init?.body))).toEqual({
      defaultConversationProfileId: 'profile-1',
      defaultVisionProfileId: null,
      expectedRevision: 2,
    })
  })

  it('implements credential isolation, validation, defaults, and archive in demo mode', async () => {
    const demo = new DemoGateway()
    const before = await demo.listAiModelProfiles()

    await expect(demo.createAiModelProfile({
      ...modelInput,
      apiKey: undefined,
    })).rejects.toThrow('首次保存必须填写 API Key')

    const created = await demo.createAiModelProfile(modelInput)
    expect(created).toEqual(expect.objectContaining({
      modelId: 'organization/model:latest',
      hasKey: true,
    }))
    expect(created).not.toHaveProperty('apiKey')

    const validation = await demo.validateAiModelProfile({
      ...validationInput,
      apiKey: undefined,
      profileId: created.id,
      currentVersion: created.currentVersion,
    })
    expect(validation.ok).toBe(true)
    await expect(demo.validateAiModelProfile({
      ...validationInput,
      profileId: created.id,
    })).rejects.toThrow('必须同时提供')

    const rotated = await demo.updateAiModelProfile(created.id, {
      ...modelInput,
      apiKey: 'rotated-write-only-secret',
      expectedRevision: created.revision,
    })
    expect(rotated.currentVersion).toBe(created.currentVersion)
    expect(rotated.revision).toBe(created.revision)

    const emptyKeyUpdate = await demo.updateAiModelProfile(created.id, {
      ...modelInput,
      name: '自由模型（重命名）',
      apiKey: '',
      expectedRevision: rotated.revision,
    })
    expect(emptyKeyUpdate.hasKey).toBe(true)

    await expect(demo.updateAiModelProfile(created.id, {
      ...modelInput,
      protocol: 'anthropic_messages',
      apiKey: undefined,
      expectedRevision: emptyKeyUpdate.revision,
    })).rejects.toThrow('必须重新输入 API Key')

    const updated = await demo.updateAiModelProfile(created.id, {
      ...modelInput,
      protocol: 'anthropic_messages',
      expectedRevision: emptyKeyUpdate.revision,
    })
    const defaults = await demo.getAiModelDefaults()
    await demo.saveAiModelDefaults({
      defaultConversationProfileId: updated.id,
      defaultVisionProfileId: updated.id,
      expectedRevision: defaults.revision,
    })
    await expect(demo.updateAiModelProfile(updated.id, {
      ...modelInput,
      protocol: 'anthropic_messages',
      supportsVision: false,
      expectedRevision: updated.revision,
    })).rejects.toThrow('取消默认视觉模型')
    await demo.archiveAiModelProfile(updated.id, updated.revision)

    const after = await demo.listAiModelProfiles()
    const clearedDefaults = await demo.getAiModelDefaults()
    expect(after).toHaveLength(before.length)
    expect(clearedDefaults.defaultConversationProfileId).toBeUndefined()
    expect(clearedDefaults.defaultVisionProfileId).toBeUndefined()
  })
})

describe('conversation start gateway contracts', () => {
  const startedConversation = {
    id: 'conversation-1',
    projectId: 'project-1',
    title: '新会话',
    modelProfileId: 'profile-1',
    modelProfileVersion: 2,
    modelProfileName: '主对话模型',
    modelId: 'model-primary',
    readOnly: false,
    createdAt: '2026-07-23T01:00:00Z',
    updatedAt: '2026-07-23T01:00:00Z',
    revision: 0,
  }
  const startedAutonomy = {
    mode: 'full' as const,
    effectiveMode: 'auto' as const,
    maxMode: 'auto' as const,
    batchLimit: 50,
    revision: 1,
    requiresHumanApproval: [],
  }

  it('declares Full natively and strips renderer-only authority from Tauri start/update payloads', async () => {
    const calls: Array<[string, Record<string, unknown> | undefined]> = []
    const invokeCommand = async <T>(command: string, args?: Record<string, unknown>): Promise<T> => {
      calls.push([command, args])
      if (command === 'start_ai_conversation') {
        return {
          conversation: startedConversation,
          autonomy: startedAutonomy,
        } as T
      }
      if (command === 'set_ai_autonomy') {
        return { ...startedAutonomy, revision: 2 } as T
      }
      return undefined as T
    }
    const local = new LocalTauriGateway(invokeCommand)

    const started = await local.startAiConversation({
      projectId: 'project-1',
      title: '受治理会话',
      modelProfileId: 'profile-1',
      requestedMode: 'full',
      currentPassword: 'must-not-cross-local-ipc',
      declared: true,
      sessionId: 'renderer-forged-session',
    } as Parameters<typeof local.startAiConversation>[0] & {
      declared: boolean
      sessionId: string
    })
    await local.setAiAutonomy('conversation-1', {
      mode: 'full',
      expectedRevision: 1,
    })

    expect(started.conversation).toEqual(expect.objectContaining({
      id: 'conversation-1',
      modelProfileId: 'profile-1',
    }))
    expect(calls).toEqual([
      ['declare_ai_full_startup', undefined],
      ['start_ai_conversation', {
        input: {
          projectId: 'project-1',
          title: '受治理会话',
          modelProfileId: 'profile-1',
          requestedMode: 'full',
        },
      }],
      ['declare_ai_full_startup', undefined],
      ['set_ai_autonomy', {
        conversationId: 'conversation-1',
        input: {
          mode: 'full',
          expectedRevision: 1,
        },
      }],
    ])
    expect(JSON.stringify(calls)).not.toContain('must-not-cross-local-ipc')
    expect(JSON.stringify(calls)).not.toContain('renderer-forged-session')
    expect(JSON.stringify(calls)).not.toContain('declared')
  })

  it('does not start a Desktop conversation when the native startup declaration fails', async () => {
    const calls: string[] = []
    const local = new LocalTauriGateway(async <T>(command: string): Promise<T> => {
      calls.push(command)
      throw new Error('本次启动尚未声明 Full')
    })

    await expect(local.startAiConversation({
      title: '声明失败会话',
      modelProfileId: 'profile-1',
      requestedMode: 'full',
    })).rejects.toThrow('尚未声明')
    expect(calls).toEqual(['declare_ai_full_startup'])
  })

  it('sends the current password only in a Server Full start and keeps both starts CSRF protected', async () => {
    const requests: Array<{ url: string; init?: RequestInit }> = []
    const fetchRequest = vi.fn(async (input: string | URL | Request, init?: RequestInit) => {
      const url = String(input)
      requests.push({ url, init })
      if (url.endsWith('/auth/login')) {
        return new Response(JSON.stringify({ data: {
          user: {
            id: 'user-1',
            lab_id: 'lab-1',
            display_name: '研究者',
            lab_roles: [],
            project_roles: [],
            authentication: 'session',
          },
          csrf_token: 'csrf-start',
          expires_at: '2026-07-23T10:00:00Z',
        } }), { status: 200, headers: { 'Content-Type': 'application/json' } })
      }
      return new Response(JSON.stringify({ data: {
        conversation: startedConversation,
        autonomy: startedAutonomy,
      } }), { status: 200, headers: { 'Content-Type': 'application/json' } })
    }) as unknown as typeof fetch
    const remote = new RemoteHttpGateway({
      baseUrl: 'https://lab.example/api/v1',
      fetch: fetchRequest,
    })
    await remote.login({ email: 'researcher@example.test', password: 'not-retained' })

    await remote.startAiConversation({
      projectId: 'project-1',
      title: 'Full 会话',
      modelProfileId: 'profile-1',
      requestedMode: 'full',
      currentPassword: 'one-request-password',
    })
    await remote.startAiConversation({
      projectId: 'project-1',
      title: 'Ask 会话',
      modelProfileId: 'profile-1',
      requestedMode: 'ask',
      currentPassword: 'must-be-stripped-for-ask',
    })

    const starts = requests.filter(({ url }) => url.endsWith('/ai/conversations'))
    expect(starts).toHaveLength(2)
    expect(JSON.parse(String(starts[0].init?.body))).toEqual({
      projectId: 'project-1',
      title: 'Full 会话',
      modelProfileId: 'profile-1',
      requestedMode: 'full',
      currentPassword: 'one-request-password',
    })
    expect(JSON.parse(String(starts[1].init?.body))).toEqual({
      projectId: 'project-1',
      title: 'Ask 会话',
      modelProfileId: 'profile-1',
      requestedMode: 'ask',
    })
    expect(starts.every(({ init }) =>
      new Headers(init?.headers).get('X-CSRF-Token') === 'csrf-start')).toBe(true)
  })

  it('includes archived profiles only when explicitly requested across adapters', async () => {
    const calls: Array<[string, Record<string, unknown> | undefined]> = []
    const local = new LocalTauriGateway(async <T>(
      command: string,
      args?: Record<string, unknown>,
    ): Promise<T> => {
      calls.push([command, args])
      return [] as T
    })
    await local.listAiModelProfiles(true)
    expect(calls).toEqual([['list_ai_model_profiles', { includeArchived: true }]])

    const fetchMock = vi.fn(async (_input: string | URL | Request, _init?: RequestInit) =>
      new Response(JSON.stringify({ data: [], count: 0 }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      }))
    const remote = new RemoteHttpGateway({
      baseUrl: 'https://lab.example/api/v1',
      fetch: fetchMock as unknown as typeof fetch,
    })
    await remote.listAiModelProfiles(true)
    expect(fetchMock.mock.calls[0][0]).toBe(
      'https://lab.example/api/v1/ai/models?includeArchived=true',
    )
  })

  it('binds Demo conversations to a profile and makes archived history read-only', async () => {
    const demo = new DemoGateway()
    const profiles = await demo.listAiModelProfiles()
    const profile = profiles[0]
    const started = await demo.startAiConversation({
      projectId: 'project-1',
      title: '演示受治理会话',
      modelProfileId: profile.id,
      requestedMode: 'full',
    })

    expect(started.conversation).toEqual(expect.objectContaining({
      modelProfileId: profile.id,
      modelProfileVersion: profile.currentVersion,
      readOnly: false,
    }))
    expect(started.autonomy).toEqual(expect.objectContaining({
      mode: 'full',
      effectiveMode: 'full',
    }))

    await demo.archiveAiModelProfile(profile.id, profile.revision)
    const detail = await demo.getAiConversation(started.conversation.id)
    expect(detail.conversation).toEqual(expect.objectContaining({
      readOnly: true,
      readOnlyReason: 'model_archived',
    }))
    expect((await demo.listAiModelProfiles(true)).some((item) =>
      item.id === profile.id && item.archivedAt)).toBe(true)
    await expect(demo.aiTurn({
      conversationId: started.conversation.id,
      projectId: 'project-1',
      message: '归档模型不能继续执行',
    })).rejects.toThrow('只读')
    await expect(demo.uploadAiSource({
      conversationId: started.conversation.id,
      projectId: 'project-1',
      file: new File(['blocked'], 'blocked.md', { type: 'text/markdown' }),
    })).rejects.toThrow('只读')
  })

  it('requires a governed writable Demo conversation and keeps its exact model snapshot', async () => {
    const demo = new DemoGateway()
    const profiles = await demo.listAiModelProfiles()
    const conversationProfile = profiles.find((profile) => !profile.supportsVision)!
    const visionProfile = profiles.find((profile) => profile.supportsVision)!
    const started = await demo.startAiConversation({
      projectId: 'project-1',
      title: '新会话',
      modelProfileId: conversationProfile.id,
      requestedMode: 'ask',
    })
    const sourceFile = new File(['# governed'], 'governed.md', {
      type: 'text/markdown',
    })

    await expect(demo.aiTurn({
      conversationId: 'missing-conversation',
      projectId: 'project-1',
      message: '不得隐式创建',
    })).rejects.toThrow('会话不存在')
    await expect(demo.uploadAiSource({
      conversationId: 'missing-conversation',
      projectId: 'project-1',
      file: sourceFile,
    })).rejects.toThrow('会话不存在')

    const source = await demo.uploadAiSource({
      conversationId: started.conversation.id,
      projectId: 'project-1',
      file: sourceFile,
    })
    const imageBytes = new TextEncoder().encode('image')
    const imageFile = {
      name: 'evidence.png',
      type: 'image/png',
      size: imageBytes.byteLength,
      arrayBuffer: async () => imageBytes.buffer,
      slice: () => new Blob([imageBytes], { type: 'image/png' }),
    } as unknown as File
    const image = await demo.uploadPrivateImage(imageFile, started.conversation.id)
    const updated = await demo.updateAiModelProfile(conversationProfile.id, {
      name: `${conversationProfile.name}（新版本）`,
      protocol: conversationProfile.protocol,
      transport: conversationProfile.transport,
      baseUrl: conversationProfile.baseUrl,
      modelId: `${conversationProfile.modelId}-new`,
      supportsVision: true,
      contextWindowTokens: conversationProfile.contextWindowTokens,
      maxInputTokens: conversationProfile.maxInputTokens,
      maxOutputTokens: conversationProfile.maxOutputTokens,
      historyTokenBudget: conversationProfile.historyTokenBudget,
      historyTurns: conversationProfile.historyTurns,
      temperature: conversationProfile.temperature,
      timeoutMs: conversationProfile.timeoutMs,
      expectedRevision: conversationProfile.revision,
    })
    expect(updated.currentVersion).toBe(conversationProfile.currentVersion + 1)

    await expect(demo.aiTurn({
      conversationId: started.conversation.id,
      projectId: 'project-1',
      message: '文本来源不能静默携带视觉模型',
      sourceRefs: [source.id],
      visionModelProfileId: visionProfile.id,
    })).rejects.toThrow('只能用于需要中转的图片证据')

    const turn = await demo.aiTurn({
      conversationId: started.conversation.id,
      projectId: 'project-1',
      message: '读取受治理来源和图片',
      sourceRefs: [source.id],
      imageIds: [image.image.id],
      visionModelProfileId: visionProfile.id,
    })
    expect(turn.content).toContain('视觉中转')
    expect(turn.trace.stages).toEqual([
      expect.objectContaining({
        profileId: visionProfile.id,
        profileVersion: visionProfile.currentVersion,
        purpose: 'vision_observation',
      }),
      expect.objectContaining({
        profileId: conversationProfile.id,
        profileVersion: conversationProfile.currentVersion,
        modelId: conversationProfile.modelId,
        purpose: 'final_answer',
      }),
    ])

    const imageSource = await demo.uploadAiSource({
      conversationId: started.conversation.id,
      projectId: 'project-1',
      file: imageFile,
    })
    const sourceTurn = await demo.aiTurn({
      conversationId: started.conversation.id,
      projectId: 'project-1',
      message: '只读取图片来源',
      sourceRefs: [imageSource.id],
      visionModelProfileId: visionProfile.id,
    })
    expect(sourceTurn.content).toContain('视觉中转')
    expect(sourceTurn.trace.stages).toEqual([
      expect.objectContaining({
        profileId: visionProfile.id,
        purpose: 'vision_observation',
      }),
      expect.objectContaining({
        profileId: conversationProfile.id,
        profileVersion: conversationProfile.currentVersion,
        purpose: 'final_answer',
      }),
    ])

    const directStarted = await demo.startAiConversation({
      projectId: 'project-1',
      title: '直接视觉会话',
      modelProfileId: visionProfile.id,
      requestedMode: 'ask',
    })
    const directImage = await demo.uploadPrivateImage(imageFile, directStarted.conversation.id)
    await expect(demo.aiTurn({
      conversationId: directStarted.conversation.id,
      projectId: 'project-1',
      message: '直接视觉不能携带中转模型',
      imageIds: [directImage.image.id],
      visionModelProfileId: visionProfile.id,
    })).rejects.toThrow('只能用于需要中转的图片证据')

    const detail = await demo.getAiConversation(started.conversation.id)
    await demo.updateAiConversation(started.conversation.id, {
      action: 'archive',
      expectedRevision: detail.conversation.revision,
    })
    await expect(demo.aiTurn({
      conversationId: started.conversation.id,
      projectId: 'project-1',
      message: '归档会话不能继续执行',
    })).rejects.toThrow('已归档')
    await expect(demo.uploadAiSource({
      conversationId: started.conversation.id,
      projectId: 'project-1',
      file: sourceFile,
    })).rejects.toThrow('已归档')
  })
})

describe('browser demo gateway', () => {
  it('moves animals atomically and updates both projections', async () => {
    const gateway = new DemoGateway()
    await gateway.moveAnimals(['animal-001'], 'cage-a03')
    const [cages, animals] = await Promise.all([gateway.listCages(), gateway.listAnimals()])
    expect(cages.find((cage) => cage.id === 'cage-a03')?.animalIds).toContain('animal-001')
    expect(cages.find((cage) => cage.id === 'cage-a01')?.animalIds).not.toContain('animal-001')
    expect(animals.find((animal) => animal.id === 'animal-001')?.cageId).toBe('cage-a03')
  })

  it('rejects duplicate cage codes within one room', async () => {
    const gateway = new DemoGateway()
    await expect(gateway.createCage({ code: 'A01', room: 'SPF-A', rack: 'R1', capacity: 5 })).rejects.toThrow('已存在')
  })

  it('does not partially move when one animal is unknown', async () => {
    const gateway = new DemoGateway()
    const before = await gateway.listCages()
    await expect(gateway.moveAnimals(['animal-001', 'missing'], 'cage-a03')).rejects.toThrow('不存在')
    expect(await gateway.listCages()).toEqual(before)
  })

  it('models key presence without retaining the demo API key', async () => {
    const gateway = new DemoGateway()

    const saved = await gateway.saveAiSettings({
      enabled: true,
      providerKind: 'local_http',
      providerPresetId: 'custom-openai-compatible',
      model: 'qwen-local',
      baseUrl: 'http://127.0.0.1:11434/v1',
      contextWindowTokens: 65536,
      maxInputTokens: 32768,
      maxOutputTokens: 3072,
      historyTokenBudget: 12000,
      historyTurns: 8,
      temperature: 0.4,
      timeoutMs: 180000,
      apiKey: 'do-not-retain-this',
    })
    const loaded = await gateway.getAiSettings()

    expect(saved.hasKey).toBe(true)
    expect(loaded).toEqual(expect.objectContaining({
      contextWindowTokens: 65536,
      maxInputTokens: 32768,
      maxOutputTokens: 3072,
      historyTokenBudget: 12000,
      historyTurns: 8,
      temperature: 0.4,
      timeoutMs: 180000,
    }))
    expect(JSON.stringify(loaded)).not.toContain('do-not-retain-this')
    expect((await gateway.clearAiApiKey()).hasKey).toBe(false)
  })

  it('runs the structured genetics-to-litter-to-animal workflow in memory', async () => {
    const gateway = new DemoGateway()
    const locus = await gateway.createGeneLocus({ symbol: 'GeneA' })
    const wildType = await gateway.createAllele({
      locusId: locus.id, symbol: '+', isWildType: true,
    })
    const flox = await gateway.createAllele({
      locusId: locus.id, symbol: 'flox', isWildType: false,
    })
    const definition = await gateway.createGenotypeDefinition({
      name: 'GeneA +/flox',
      components: [{
        locusId: locus.id, allele1Id: wildType.id, allele2Id: flox.id,
        mode: 'diploid', displayOrder: 0,
      }],
    })
    const line = await gateway.createBreedingLine({
      name: 'GeneA conditional', genotypeDefinitionIds: [definition.id],
    })
    const colony = await gateway.createColony({
      breedingLineId: line.id, name: 'Founder colony',
    })
    const pair = await gateway.createBreedingPair({
      colonyId: colony.id, name: 'Pair 1', maleAnimalId: 'animal-001',
      femaleAnimalIds: ['animal-005'],
    })
    const mating = await gateway.createMatingEvent({
      breedingPairId: pair.id, maleAnimalId: 'animal-001', femaleAnimalId: 'animal-005',
      occurredAt: '2026-07-01T08:00:00Z',
    })
    const created = await gateway.createLitter({
      matingEventId: mating.id, bornOn: '2026-07-19', sizeTotal: 2,
      drafts: [{ temporaryLabel: 'P1', sex: 'female' }],
    })
    const registered = await gateway.registerAnimalDraft({
      draftId: created.drafts[0].id, expectedRevision: 1,
      identifierScope: 'lab', displayId: 'M-DEMO-DRAFT-1', strain: 'C57BL/6J',
    })

    expect(pair.members.map((member) => member.role)).toEqual(['male', 'female'])
    expect(created.litter).toEqual(expect.objectContaining({ sizeTotal: 2, sizeAlive: 1 }))
    expect(registered.draft).toEqual(expect.objectContaining({
      status: 'registered', registeredAnimalId: registered.animal.id, revision: 2,
    }))
    expect((await gateway.listAnimals()).some((animal) => animal.code === 'M-DEMO-DRAFT-1')).toBe(true)
  })

  it('captures enrollment genotype snapshot and appends observation versions', async () => {
    const gateway = new DemoGateway()
    const locus = await gateway.createGeneLocus({ symbol: 'Rosa26' })
    const allele = await gateway.createAllele({
      locusId: locus.id, symbol: 'tdTomato', isWildType: false,
    })
    const definition = await gateway.createGenotypeDefinition({
      name: 'Rosa26 tdTomato+',
      components: [{
        locusId: locus.id, allele1Id: allele.id,
        mode: 'transgene_presence', displayOrder: 0,
      }],
    })
    const record = await gateway.createGenotypingRecord({
      animalId: 'animal-006', genotypeDefinitionId: definition.id,
      state: 'confirmed', assessedAt: '2026-07-18T08:00:00Z', method: 'PCR',
    })
    const participation = await gateway.enrollAnimal({
      experimentId: 'exp-001', animalId: 'animal-006',
    })
    const event = await gateway.createExperimentEvent({
      experimentId: 'exp-001', eventKey: 'day_7', label: 'Day 7',
    })
    const observationDefinition = await gateway.createObservationDefinition({
      experimentId: 'exp-001', key: 'body_weight', label: '体重',
      valueType: 'number', unit: 'g', policy: 'versioned',
    })
    const first = await gateway.recordObservation({
      experimentId: 'exp-001', experimentEventId: event.id,
      definitionId: observationDefinition.id, subjectType: 'animal', subjectId: 'animal-006',
      value: { type: 'number', value: 23.1 },
    })
    const second = await gateway.reviseObservation({
      observationId: first.observation.id,
      expectedRevision: first.observation.revision,
      value: { type: 'number', value: 23.4 }, notes: '复核电子秤读数',
    })

    expect(participation.genotypeSnapshot).toEqual([expect.objectContaining({
      genotypingRecordId: record.id,
      genotypeDefinitionId: definition.id,
      state: 'confirmed',
    })])
    expect(second.observation.currentValueVersion).toBe(2)
    expect((await gateway.listObservationValues(first.observation.id)).map((value) => value.version))
      .toEqual([1, 2])
  })

  it('keeps demo extraction pending, releases every image on reject, and promotes on approval', async () => {
    const gateway = new DemoGateway()
    const experiment = (await gateway.listExperiments()).find((entry) => entry.id === 'exp-001')!
    const event = await gateway.createExperimentEvent({
      experimentId: experiment.id,
      eventKey: 'vision_contract',
      label: 'Vision contract',
    })
    const definition = await gateway.createObservationDefinition({
      experimentId: experiment.id,
      key: 'vision_contract_value',
      label: '视觉候选',
      valueType: 'number',
      unit: 'g',
      policy: 'versioned',
    })
    const profile = (await gateway.listAiModelProfiles()).find((entry) =>
      entry.supportsVision && !entry.archivedAt)!
    const imageBytes = new TextEncoder().encode('portable-image')
    const imageFile = {
      name: 'vision.png',
      type: 'image/png',
      size: imageBytes.byteLength,
      arrayBuffer: async () => imageBytes.buffer,
      slice: () => new Blob([imageBytes], { type: 'image/png' }),
    } as unknown as File
    const image = await gateway.uploadPrivateImage(imageFile)
    const createInput = {
      imageIds: [image.image.id],
      projectId: experiment.projectId,
      experimentId: experiment.id,
      experimentEventId: event.id,
      currentDataCell: {
        definitionId: definition.id,
        subjectType: 'experiment' as const,
        subjectId: experiment.id,
      },
      visionModelProfileId: profile.id,
    }

    const pending = await gateway.createAiExtraction(createInput)
    expect(pending.candidates[0].selected).toBe(false)
    expect(pending.modelTrace?.purpose).toBe('vision')
    expect((await gateway.listPrivateImages()).find((entry) =>
      entry.image.id === image.image.id)?.image.status).toBe('pending_approval')

    const rejected = await gateway.rejectAiExtraction(pending.id, {
      expectedRevision: pending.revision,
    })
    expect(rejected.status).toBe('rejected')
    expect((await gateway.listPrivateImages()).find((entry) =>
      entry.image.id === image.image.id)?.image.status).toBe('active')

    const second = await gateway.createAiExtraction(createInput)
    const applied = await gateway.approveAiExtraction(second.id, {
      expectedRevision: second.revision,
      selections: [{
        itemIndex: second.candidates[0].itemIndex,
        value: { type: 'number', value: 12.5 },
        notes: 'human checked',
      }],
    })
    expect(applied.attachments).toHaveLength(1)
    expect(applied.links).toHaveLength(1)
    expect(applied.draft.candidates[0].selected).toBe(true)
    expect((await gateway.listPrivateImages()).find((entry) =>
      entry.image.id === image.image.id)?.image.status).toBe('archived')
    await expect(gateway.readPrivateImage(image.image.id)).resolves.toBeInstanceOf(Blob)
  })
})
