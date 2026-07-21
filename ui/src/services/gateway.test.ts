import { describe, expect, it, vi } from 'vitest'
import { currentAuthSession, DemoGateway, LocalTauriGateway, RemoteHttpGateway, createGateway } from './gateway'
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

    expect(calls).toContainEqual(['list_gene_loci', { projectId: 'project-1' }])
    expect(calls).toContainEqual(['create_gene_locus', {
      input: { projectId: 'project-1', symbol: 'GeneA' },
    }])
    expect(calls).toContainEqual(['list_alleles', { locusId: 'locus-1', projectId: 'project-1' }])
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
        citations: [{ entity_type: 'animal', entity_id: 'animal-1', revision: 3 }],
        toolRuns: [{
          tool_run_id: 'run-1', provider_call_id: 'call-1', tool: 'animal_search',
          arguments: { display_id: 'M-001' }, outcome: 'read',
          citations: [{ entity_type: 'animal', entity_id: 'animal-1', revision: 3 }],
        }],
        drafts: [],
        trace: {
          providerId: 'local-provider', model: 'test-model',
          usage: { provider_calls: 1, tool_calls: 1, input_tokens: 10, output_tokens: 5, total_tokens: 15 },
        },
      } as T
    }
    const local = new LocalTauriGateway(invokeCommand)

    const response = await local.aiTurn({ projectId: 'project-1', message: '查找 M-001' })
    await local.listAiDrafts('project-1', 'pending_approval')

    expect(calls[0]).toEqual(['ai_turn', { input: { projectId: 'project-1', message: '查找 M-001' } }])
    expect(calls[1]).toEqual(['list_ai_drafts', { projectId: 'project-1', status: 'pending_approval' }])
    expect(response.citations[0]).toEqual(expect.objectContaining({
      entityType: 'animal', entityId: 'animal-1', revision: 3, route: '/animals?animal=animal-1',
    }))
    expect(response.toolRuns[0]).toEqual(expect.objectContaining({ toolRunId: 'run-1', outcome: 'read' }))
    expect(response.trace.usage.totalTokens).toBe(15)
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
          id: 'message-1', sequence: 1, role: 'assistant', content: '已恢复',
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
    expect(detail.messages[0].response?.citations[0]).toEqual(expect.objectContaining({
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
        : url.endsWith('/cages')
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
      enabled: true, providerKind: 'open_ai_compatible', model: 'gpt-test',
      baseUrl: 'https://api.example/v1', apiKey: 'write-only-key',
    })
    const turn = await remote.aiTurn({ projectId: 'project-1', message: '总结进度' })
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
          id: 'job-failed', kind: 'import', status: 'failed', idempotency_key: 'failed-import',
          progress_current: 2, progress_total: 5, meta: { created_at: '2026-07-19T09:00:00Z' },
        },
        {
          id: 'job-cancelled', kind: 'export', status: 'cancelled', idempotency_key: 'cancelled-export',
          progress_current: 1, progress_total: 4, meta: { created_at: '2026-07-19T09:05:00Z' },
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
      model: 'qwen-local',
      baseUrl: 'http://127.0.0.1:11434/v1',
      apiKey: 'do-not-retain-this',
    })
    const loaded = await gateway.getAiSettings()

    expect(saved.hasKey).toBe(true)
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
})
