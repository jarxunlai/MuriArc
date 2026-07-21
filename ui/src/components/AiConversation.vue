<script setup lang="ts">
import { computed, nextTick, reactive, ref, watch } from 'vue'
import { useMessage } from 'naive-ui'
import {
  Bot,
  Check,
  ChevronDown,
  CornerDownLeft,
  Database,
  FileSignature,
  ShieldCheck,
  UserRound,
  X,
} from '@lucide/vue'
import type { AiAutonomyMode, AiWriteDraft } from '@/domain/models'
import { useAiAssistant } from '@/composables/useAiAssistant'

const props = withDefaults(defineProps<{ compact?: boolean }>(), { compact: false })
const ai = useAiAssistant()
const toast = useMessage()
const prompt = ref('')
const scrollArea = ref<HTMLElement | null>(null)
const statements = reactive<Record<string, string>>({})
const signed = reactive<Record<string, boolean>>({})
const reinforcedConfirmed = reactive<Record<string, boolean>>({})
const currentPasswords = reactive<Record<string, string>>({})
const autonomyModalOpen = ref(false)
const autonomyPassword = ref('')
const autonomyDeclared = ref(false)
const modeOptions = computed(() => {
  const rank: Record<AiAutonomyMode, number> = { ask: 0, auto: 1, full: 2 }
  const max = ai.autonomy.value.maxMode
  return [
    { label: 'Ask', value: 'ask' as const, disabled: rank.ask > rank[max] },
    { label: 'Auto', value: 'auto' as const, disabled: rank.auto > rank[max] },
    { label: 'Full', value: 'full' as const, disabled: rank.full > rank[max] },
  ]
})
const autonomyDescription = computed(() => ({
  ask: '查询自动执行；创建导出等产物前需要你确认。',
  auto: `查询和普通产物可自动执行；普通批量上限 ${ai.autonomy.value.batchLimit} 条。`,
  full: `当前会话内扩大普通操作授权，批量上限 ${ai.autonomy.value.batchLimit} 条；30 分钟无活动自动降级。`,
})[ai.autonomy.value.effectiveMode])
const suggestions = computed(() => props.compact
  ? ['这个页面有哪些异常？', '哪些数据还没记录？']
  : ['总结进行中的实验', '找出待确认的基因型', '哪些动物缺少近期体重？'])
const visibleDrafts = computed(() => {
  const drafts = new Map<string, AiWriteDraft>()
  for (const message of ai.messages.value) {
    for (const draft of message.drafts ?? []) drafts.set(draft.id, draft)
  }
  for (const draft of ai.pendingDrafts.value) drafts.set(draft.id, draft)
  return [...drafts.values()]
})

const requirementLabels: Record<AiWriteDraft['requirement'], string> = {
  preview_confirmation: '预览确认',
  researcher_signature: '研究者签署',
  reinforced_confirmation: '加强确认',
}
const statusLabels: Record<AiWriteDraft['status'], string> = {
  pending_approval: '待审批',
  approved: '已批准',
  rejected: '已拒绝',
  applied: '已写入草稿',
  cancelled: '已取消',
  expired: '已过期',
}

async function send(value = prompt.value) {
  prompt.value = ''
  await ai.send(value)
}

function onKeydown(event: KeyboardEvent) {
  if (event.key === 'Enter' && !event.shiftKey) {
    event.preventDefault()
    void send()
  }
}

function formatValue(value: unknown): string {
  if (value === null || value === undefined) return '—'
  if (typeof value === 'string') return value
  if (typeof value === 'number' || typeof value === 'boolean') return String(value)
  return JSON.stringify(value)
}

function canApprove(draft: AiWriteDraft): boolean {
  if (draft.status !== 'pending_approval') return false
  if (draft.requirement === 'reinforced_confirmation') {
    return Boolean(
      reinforcedConfirmed[draft.id]
      && statements[draft.id]?.trim()
      && (!ai.reinforcedPasswordRequired.value || currentPasswords[draft.id]),
    )
  }
  if (draft.requirement !== 'researcher_signature') return true
  return Boolean(signed[draft.id] && statements[draft.id]?.trim())
}

async function decide(draft: AiWriteDraft, decision: 'approve' | 'reject') {
  let completed = false
  try {
    const needsPassword = decision === 'approve'
      && draft.requirement === 'reinforced_confirmation'
      && ai.reinforcedPasswordRequired.value
    const result = needsPassword
      ? await ai.decideDraft(draft, decision, statements[draft.id], currentPasswords[draft.id])
      : await ai.decideDraft(draft, decision, statements[draft.id])
    completed = true
    if (decision === 'reject') {
      toast.success('已拒绝该 AI 写入草稿')
    } else if (result.measurementId) {
      toast.success('已创建未签署的科研测量草稿，请由研究者继续签署')
    } else {
      toast.success('草稿已批准并应用')
    }
  } catch (error) {
    toast.error(error instanceof Error ? error.message : '审批失败，请刷新后重试')
  } finally {
    currentPasswords[draft.id] = ''
    if (completed) {
      statements[draft.id] = ''
      signed[draft.id] = false
      reinforcedConfirmed[draft.id] = false
    }
  }
}

async function selectAutonomy(mode: AiAutonomyMode) {
  if (mode === ai.autonomy.value.mode) return
  if (mode === 'full') {
    autonomyPassword.value = ''
    autonomyDeclared.value = false
    autonomyModalOpen.value = true
    return
  }
  try {
    await ai.updateAutonomy(mode)
    toast.success(`当前会话已切换到 ${mode === 'ask' ? 'Ask' : 'Auto'} 模式`)
  } catch (error) {
    toast.error(error instanceof Error ? error.message : '无法更新 AI 授权')
  }
}

async function applyFullAutonomy() {
  if (!autonomyDeclared.value) return
  try {
    await ai.updateAutonomy('full', {
      currentPassword: autonomyPassword.value || undefined,
      declared: autonomyDeclared.value,
    })
    autonomyModalOpen.value = false
    autonomyPassword.value = ''
    toast.success('当前会话已启用 Full 模式，30 分钟无活动后自动降级')
  } catch (error) {
    toast.error(error instanceof Error ? error.message : '无法启用 Full 模式')
  }
}

watch(() => ai.messages.value.length, async () => {
  await nextTick()
  scrollArea.value?.scrollTo({ top: scrollArea.value.scrollHeight, behavior: 'smooth' })
})
</script>

<template>
  <section class="ai-conversation" :class="{ compact }">
    <div ref="scrollArea" class="message-list" aria-live="polite">
      <div class="context-strip">
        <Database :size="14" />
        <span>当前上下文：<strong>{{ ai.contextTitle.value }}</strong></span>
        <span class="scope-divider">·</span>
        <span>{{ ai.selectedProject.value?.name ?? '跨项目只读' }}</span>
        <span class="scope-divider">·</span>
        <n-select
          class="autonomy-select"
          size="tiny"
          :value="ai.autonomy.value.mode"
          :options="modeOptions"
          :loading="ai.autonomyBusy.value"
          :disabled="!ai.conversationId.value"
          aria-label="AI 会话授权模式"
          @update:value="selectAutonomy"
        />
      </div>
      <article v-for="entry in ai.messages.value" :key="entry.id" class="message" :class="entry.role">
        <div class="avatar">
          <Bot v-if="entry.role === 'assistant'" :size="17" />
          <UserRound v-else :size="17" />
        </div>
        <div class="bubble" :class="{ pending: entry.pending, error: entry.error }">
          <p>{{ entry.content }}</p>
          <div v-if="entry.citations?.length" class="citations" aria-label="数据引用">
            <template v-for="citation in entry.citations" :key="`${citation.entityType}-${citation.entityId}`">
              <router-link v-if="citation.route" :to="citation.route">{{ citation.label }}</router-link>
              <span v-else>{{ citation.label }}</span>
            </template>
          </div>
          <details v-if="entry.toolRuns?.length" class="tool-trace">
            <summary>
              <ChevronDown :size="14" />
              已调用 {{ entry.toolRuns.length }} 个安全领域工具
              <small v-if="entry.trace">{{ entry.trace.model }}</small>
            </summary>
            <div class="tool-list">
              <span v-for="run in entry.toolRuns" :key="run.toolRunId">
                {{ run.tool }} · {{ run.outcome === 'read' ? '只读' : '写入草稿' }}
              </span>
            </div>
          </details>
        </div>
      </article>

      <section v-if="visibleDrafts.length" class="draft-section" aria-label="AI 写入草稿">
        <div class="draft-section-title"><FileSignature :size="17" />写入草稿</div>
        <article v-for="draft in visibleDrafts" :key="draft.id" class="draft-card">
          <header>
            <div>
              <strong>{{ draft.kind === 'measurement_result' ? '科研测量草稿' : '数据变更草稿' }}</strong>
              <span>{{ requirementLabels[draft.requirement] }} · revision {{ draft.revision }}</span>
            </div>
            <n-tag size="small" :type="draft.status === 'applied' ? 'success' : draft.status === 'rejected' ? 'error' : 'warning'">
              {{ statusLabels[draft.status] }}
            </n-tag>
          </header>
          <div class="diff-list">
            <div v-for="change in draft.changes" :key="change.path" class="diff-row">
              <code>{{ change.path }}</code>
              <span class="before">{{ formatValue(change.before) }}</span>
              <span class="arrow">→</span>
              <span class="after">{{ formatValue(change.after) }}</span>
            </div>
          </div>
          <template v-if="draft.status === 'pending_approval'">
            <div v-if="draft.requirement === 'reinforced_confirmation'" class="signature-box reinforced-box">
              <n-alert type="warning" :bordered="false">
                <template v-if="ai.reinforcedPasswordRequired.value">
                  此操作会批量或高风险地修改共享数据。请完整核对 diff，并使用当前登录账号的密码完成加强确认。
                </template>
                <template v-else>
                  此操作会批量或高风险地修改本地数据。请完整核对 diff，填写声明并勾选确认；本地原生边界会再次校验该声明。
                </template>
              </n-alert>
              <n-input
                v-model:value="statements[draft.id]"
                type="textarea"
                :rows="2"
                maxlength="500"
                show-count
                :data-testid="`reinforced-statement-${draft.id}`"
                placeholder="填写确认声明，例如：我已核对导入预览、冲突和目标项目"
              />
              <n-checkbox
                v-model:checked="reinforcedConfirmed[draft.id]"
                :data-testid="`reinforced-checkbox-${draft.id}`"
              >我已核对上述 diff，并确认执行此高风险操作</n-checkbox>
              <n-input
                v-if="ai.reinforcedPasswordRequired.value"
                v-model:value="currentPasswords[draft.id]"
                type="password"
                show-password-on="click"
                autocomplete="current-password"
                maxlength="1024"
                :data-testid="`reinforced-password-${draft.id}`"
                placeholder="当前密码"
              />
            </div>
            <div v-else-if="draft.requirement === 'researcher_signature'" class="signature-box">
              <n-input
                v-model:value="statements[draft.id]"
                type="textarea"
                :rows="2"
                maxlength="500"
                show-count
                placeholder="填写研究者签署声明，例如：我已核对动物、指标、数值与单位"
              />
              <n-checkbox v-model:checked="signed[draft.id]">我已核对上述 diff，并以研究者身份签署</n-checkbox>
            </div>
            <div class="draft-actions">
              <n-button
                size="small"
                type="primary"
                :loading="ai.draftBusy(draft.id)"
                :disabled="!canApprove(draft)"
                :data-testid="`approve-draft-${draft.id}`"
                @click="decide(draft, 'approve')"
              ><template #icon><Check :size="15" /></template>批准</n-button>
              <n-button
                size="small"
                type="error"
                secondary
                :loading="ai.draftBusy(draft.id)"
                @click="decide(draft, 'reject')"
              ><template #icon><X :size="15" /></template>拒绝</n-button>
            </div>
          </template>
        </article>
      </section>
    </div>

    <div class="composer">
      <div class="suggestions">
        <button v-for="item in suggestions" :key="item" type="button" @click="send(item)">{{ item }}</button>
      </div>
      <div class="input-wrap">
        <textarea v-model="prompt" rows="2" placeholder="询问动物、实验或数据…" :disabled="ai.busy.value" @keydown="onKeydown" />
        <n-button type="primary" circle :disabled="!prompt.trim() || ai.busy.value" aria-label="发送" @click="send()">
          <template #icon><CornerDownLeft :size="17" /></template>
        </n-button>
      </div>
      <div class="safety-note"><ShieldCheck :size="13" /> {{ autonomyDescription }} 科研签署和高风险操作始终由人工确认。</div>
    </div>

    <n-modal v-model:show="autonomyModalOpen" preset="card" title="启用当前会话的 Full 模式" class="autonomy-modal">
      <n-alert type="warning" :bordered="false">
        Full 不是新角色，也不会扩大你的项目权限。动物转移/死亡、删除与批量导入、科研签署、繁育事实、账号权限和日志清理仍无法自动执行。
      </n-alert>
      <div class="autonomy-boundary-list">
        <span>仅当前会话</span><span>30 分钟无活动降级</span><span>普通批量最多 100 条</span>
      </div>
      <n-input
        v-if="ai.reinforcedPasswordRequired.value"
        v-model:value="autonomyPassword"
        type="password"
        show-password-on="click"
        autocomplete="current-password"
        maxlength="1024"
        placeholder="输入当前登录密码完成身份确认"
      />
      <n-checkbox v-model:checked="autonomyDeclared">
        我理解 Full 仅是当前会话的受限委托，不会绕过人工审批和签署
      </n-checkbox>
      <template #footer>
        <div class="modal-actions">
          <n-button @click="autonomyModalOpen = false">取消</n-button>
          <n-button
            type="primary"
            :loading="ai.autonomyBusy.value"
            :disabled="!autonomyDeclared || (ai.reinforcedPasswordRequired.value && !autonomyPassword)"
            @click="applyFullAutonomy"
          >确认启用</n-button>
        </div>
      </template>
    </n-modal>
  </section>
</template>

<style scoped>
.ai-conversation { display: grid; grid-template-rows: minmax(0, 1fr) auto; min-height: 0; height: 100%; background: var(--muri-surface); }
.message-list { overflow: auto; padding: 20px; }
.context-strip { display: flex; align-items: center; gap: 6px; width: fit-content; margin: 0 auto 20px; padding: 6px 10px; border: 1px solid var(--muri-border); border-radius: 999px; color: var(--muri-text-secondary); background: var(--muri-surface-muted); font-size: 12px; }.scope-divider { color: var(--muri-border-strong); }.autonomy-select { width: 82px; }
.message { display: flex; align-items: flex-start; gap: 10px; max-width: 760px; margin: 0 auto 16px; }.message.user { flex-direction: row-reverse; }
.avatar { display: grid; flex: 0 0 30px; width: 30px; height: 30px; place-items: center; border: 1px solid var(--muri-border); border-radius: 50%; color: var(--muri-primary); background: white; }.user .avatar { color: var(--muri-text-secondary); }
.bubble { max-width: min(82%, 640px); padding: 10px 13px; border: 1px solid var(--muri-border); border-radius: 4px 12px 12px; background: var(--muri-surface-muted); line-height: 1.65; }.user .bubble { border-color: #c8deef; border-radius: 12px 4px 12px 12px; background: var(--muri-primary-soft); }.bubble.error { border-color: #efd0d0; color: var(--muri-danger); background: #fff7f7; }.bubble.pending { color: var(--muri-text-secondary); animation: soft-pulse 1.3s ease-in-out infinite; }.bubble p { margin: 0; white-space: pre-wrap; }
.citations { display: flex; flex-wrap: wrap; gap: 6px; margin-top: 8px; }.citations a, .citations span { padding: 3px 8px; border-radius: 999px; color: var(--muri-primary); background: white; font-size: 12px; }
.tool-trace { margin-top: 9px; border-top: 1px solid var(--muri-border); padding-top: 7px; }.tool-trace summary { display: flex; align-items: center; gap: 5px; color: var(--muri-text-tertiary); cursor: pointer; font-size: 11px; list-style: none; }.tool-trace summary small { margin-left: auto; }.tool-trace[open] summary svg { transform: rotate(180deg); }.tool-list { display: flex; flex-wrap: wrap; gap: 5px; margin-top: 7px; }.tool-list span { padding: 3px 6px; border-radius: 4px; background: white; color: var(--muri-text-secondary); font-family: ui-monospace, monospace; font-size: 10px; }
.draft-section { max-width: 760px; margin: 22px auto 0; }.draft-section-title { display: flex; align-items: center; gap: 6px; margin-bottom: 8px; color: var(--muri-text); font-weight: 650; }.draft-section-title svg { color: var(--muri-primary); }
.draft-card { margin-bottom: 10px; padding: 13px; border: 1px solid #c8deef; border-radius: 9px; background: #fbfdff; }.draft-card header { display: flex; align-items: flex-start; justify-content: space-between; gap: 12px; }.draft-card header > div { display: flex; flex-direction: column; }.draft-card header span { color: var(--muri-text-tertiary); font-size: 11px; }.diff-list { margin: 11px 0; border: 1px solid var(--muri-border); border-radius: 6px; overflow: hidden; }.diff-row { display: grid; grid-template-columns: minmax(100px, 1fr) minmax(80px, 1fr) 18px minmax(80px, 1fr); gap: 6px; align-items: center; padding: 7px 9px; border-bottom: 1px solid var(--muri-border); font-size: 11px; }.diff-row:last-child { border-bottom: 0; }.diff-row code { overflow: hidden; color: var(--muri-text-secondary); text-overflow: ellipsis; }.diff-row .before { color: var(--muri-danger); text-decoration: line-through; }.diff-row .after { color: var(--muri-success); }.arrow { color: var(--muri-text-tertiary); text-align: center; }
.signature-box { display: flex; flex-direction: column; gap: 8px; margin-top: 10px; }.draft-actions { display: flex; gap: 8px; margin-top: 10px; }
.composer { padding: 12px 18px 16px; border-top: 1px solid var(--muri-border); background: white; }.suggestions { display: flex; gap: 7px; max-width: 760px; margin: 0 auto 8px; overflow-x: auto; scrollbar-width: none; }.suggestions button { flex: 0 0 auto; padding: 5px 9px; border: 1px solid var(--muri-border); border-radius: 999px; color: var(--muri-text-secondary); background: white; cursor: pointer; transition: border-color var(--muri-transition-fast), color var(--muri-transition-fast); }.suggestions button:hover { border-color: var(--muri-primary); color: var(--muri-primary); }
.input-wrap { display: flex; align-items: flex-end; gap: 8px; max-width: 760px; margin: 0 auto; padding: 8px; border: 1px solid var(--muri-border-strong); border-radius: 10px; transition: border-color var(--muri-transition-fast), box-shadow var(--muri-transition-fast); }.input-wrap:focus-within { border-color: var(--muri-primary); box-shadow: 0 0 0 3px rgba(15, 95, 170, 0.1); }textarea { flex: 1; min-height: 42px; max-height: 130px; padding: 3px 5px; resize: none; border: 0; outline: 0; color: var(--muri-text); background: transparent; line-height: 1.5; }.safety-note { display: flex; align-items: center; justify-content: center; gap: 5px; margin-top: 7px; color: var(--muri-text-tertiary); font-size: 11px; }
.compact .message-list { padding: 16px 14px; }.compact .composer { padding: 10px 12px 12px; }.compact .suggestions { max-width: 100%; }
.autonomy-modal { width: min(520px, calc(100vw - 28px)); }.autonomy-boundary-list { display: flex; flex-wrap: wrap; gap: 6px; margin: 14px 0; }.autonomy-boundary-list span { padding: 4px 8px; border-radius: 999px; color: var(--muri-primary); background: var(--muri-primary-soft); font-size: 12px; }.autonomy-modal :deep(.n-checkbox) { margin-top: 14px; }.modal-actions { display: flex; justify-content: flex-end; gap: 8px; }
@keyframes soft-pulse { 50% { opacity: 0.55; } }
@media (max-width: 620px) { .context-strip { max-width: 100%; flex-wrap: wrap; justify-content: center; }.diff-row { grid-template-columns: 1fr 18px 1fr; }.diff-row code { grid-column: 1 / -1; }.draft-card { padding: 11px; } }
@media (prefers-reduced-motion: reduce) { .bubble.pending { animation: none; }.tool-trace summary svg { transition: none; } }
</style>
