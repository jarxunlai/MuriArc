<script setup lang="ts">
import { computed, nextTick, onUnmounted, reactive, ref, watch } from 'vue'
import { useMessage } from 'naive-ui'
import {
  AlertTriangle,
  Bot,
  Check,
  ChevronDown,
  CornerDownLeft,
  Database,
  FileSignature,
  ImagePlus,
  Layers3,
  ScanSearch,
  ShieldCheck,
  Trash2,
  UserRound,
  X,
} from '@lucide/vue'
import type { AiAutonomyMode, AiWriteDraft } from '@/domain/models'
import { useAiAssistant } from '@/composables/useAiAssistant'

const props = withDefaults(defineProps<{ compact?: boolean }>(), { compact: false })
const ai = useAiAssistant()
const toast = useMessage()
const scrollArea = ref<HTMLElement | null>(null)
const imageInput = ref<HTMLInputElement | null>(null)
const releaseImageComposer = ai.retainImageComposer()
const statements = reactive<Record<string, string>>({})
const signed = reactive<Record<string, boolean>>({})
const reinforcedConfirmed = reactive<Record<string, boolean>>({})
const currentPasswords = reactive<Record<string, string>>({})
const autonomyModalOpen = ref(false)
const autonomyPassword = ref('')
const autonomyDeclared = ref(false)
const autonomyModalPurpose = ref<'start' | 'update'>('update')
const pendingSendValue = ref('')
const modelSwitchModalOpen = ref(false)
const pendingModelProfileId = ref('')
const modeOptions = [
  { label: 'Ask', value: 'ask' as const },
  { label: 'Auto', value: 'auto' as const },
  { label: 'Full', value: 'full' as const },
]
const autonomyDescription = computed(() => ({
  ask: '查询自动执行；创建导出等产物前需要你确认。',
  auto: `查询和普通产物可自动执行；普通批量上限 ${ai.autonomy.value.batchLimit} 条。`,
  full: `当前会话内扩大普通操作授权，批量上限 ${ai.autonomy.value.batchLimit} 条；30 分钟无活动自动降级。`,
})[ai.conversationId.value ? ai.autonomy.value.effectiveMode : ai.requestedMode.value])
const modeLabels: Record<AiAutonomyMode, string> = {
  ask: 'Ask',
  auto: 'Auto',
  full: 'Full',
}
const requestedModeLabel = computed(() =>
  `${modeLabels[ai.requestedMode.value]}${ai.fullActivationRequired.value ? '（待启用）' : ''}`)
const effectiveModeLabel = computed(() =>
  ai.conversationId.value ? modeLabels[ai.autonomy.value.effectiveMode] : '尚未开始')
const suggestions = computed(() => props.compact
  ? ['这个页面有哪些异常？', '哪些数据还没记录？']
  : ['总结进行中的实验', '找出待确认的基因型', '哪些动物缺少近期体重？'])
const visibleDrafts = computed(() => {
  const drafts = new Map<string, AiWriteDraft>()
  for (const message of ai.messages.value) {
    for (const draft of message.drafts ?? []) drafts.set(draft.id, draft)
  }
  for (const draft of ai.conversationDrafts.value) drafts.set(draft.id, draft)
  return [...drafts.values()]
})
const canSend = computed(() =>
  Boolean(ai.composerDraft.value.trim() || ai.stagedImages.value.length))

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

function openFullModal(purpose: 'start' | 'update', value = '') {
  autonomyModalPurpose.value = purpose
  pendingSendValue.value = value
  autonomyPassword.value = ''
  autonomyDeclared.value = false
  autonomyModalOpen.value = true
}

function closeFullModal() {
  autonomyModalOpen.value = false
  autonomyPassword.value = ''
  autonomyDeclared.value = false
  pendingSendValue.value = ''
}

async function send(value = ai.composerDraft.value) {
  const normalized = value.trim()
  if ((!normalized && !ai.stagedImages.value.length) || ai.busy.value) return
  if (ai.composerDisabledReason.value) {
    toast.error(ai.composerDisabledReason.value)
    return
  }
  if (ai.fullActivationRequired.value) {
    openFullModal('start', normalized)
    return
  }
  try {
    await ai.send(normalized)
  } catch (error) {
    toast.error(error instanceof Error ? error.message : '无法开始 AI 会话')
  }
}

function openImagePicker() {
  imageInput.value?.click()
}

function selectImages(event: Event) {
  const input = event.target as HTMLInputElement
  const files = Array.from(input.files ?? [])
  input.value = ''
  if (!files.length) return
  try {
    ai.stageImages(files)
  } catch (error) {
    toast.error(error instanceof Error ? error.message : '无法暂存图片')
  }
}

function selectVisionModel(profileId: string | null) {
  try {
    ai.selectVisionModel(profileId ?? undefined)
  } catch (error) {
    toast.error(error instanceof Error ? error.message : '无法选择视觉模型')
  }
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
  if (mode === ai.requestedMode.value) return
  if (!ai.conversationId.value) {
    await ai.requestMode(mode)
    return
  }
  if (mode === 'full') {
    openFullModal('update')
    return
  }
  try {
    await ai.requestMode(mode)
    toast.success(`当前会话已切换到 ${mode === 'ask' ? 'Ask' : 'Auto'} 模式`)
  } catch (error) {
    toast.error(error instanceof Error ? error.message : '无法更新 AI 授权')
  }
}

async function applyFullAutonomy() {
  if (!autonomyDeclared.value) return
  try {
    if (autonomyModalPurpose.value === 'start') {
      await ai.send(pendingSendValue.value, {
        fullConfirmed: true,
        currentPassword: autonomyPassword.value || undefined,
      })
      closeFullModal()
      toast.success('新会话已按 Full 请求启动；实际模式以会话状态为准')
    } else {
      await ai.updateAutonomy('full', {
        currentPassword: autonomyPassword.value || undefined,
      })
      closeFullModal()
      toast.success('已请求 Full 模式；实际执行模式以会话状态为准')
    }
  } catch (error) {
    toast.error(error instanceof Error ? error.message : '无法启用 Full 模式')
  } finally {
    autonomyPassword.value = ''
  }
}

function selectModel(profileId: string) {
  if (profileId === ai.selectedModelProfileId.value) return
  if (ai.modelSwitchNeedsConfirmation(profileId)) {
    pendingModelProfileId.value = profileId
    modelSwitchModalOpen.value = true
    return
  }
  try {
    ai.selectModel(profileId)
  } catch (error) {
    toast.error(error instanceof Error ? error.message : '无法切换模型')
  }
}

function cancelModelSwitch() {
  pendingModelProfileId.value = ''
  modelSwitchModalOpen.value = false
}

function confirmModelSwitch() {
  try {
    ai.selectModel(pendingModelProfileId.value, true)
    cancelModelSwitch()
    toast.success('已创建新的空会话，项目范围和未发送输入已保留')
  } catch (error) {
    toast.error(error instanceof Error ? error.message : '无法切换模型')
  }
}

watch(() => ai.messages.value.length, async () => {
  await nextTick()
  scrollArea.value?.scrollTo({ top: scrollArea.value.scrollHeight, behavior: 'smooth' })
})

onUnmounted(releaseImageComposer)
</script>

<template>
  <section class="ai-conversation" :class="{ compact }">
    <div ref="scrollArea" class="message-list" aria-live="polite">
      <div class="context-strip">
        <div class="context-scope">
          <Database :size="14" />
          <span>当前上下文：<strong>{{ ai.contextTitle.value }}</strong></span>
          <span class="scope-divider">·</span>
          <span>{{ ai.selectedProject.value?.name ?? '跨项目只读' }}</span>
        </div>
        <div class="conversation-controls">
          <label class="control-field model-field">
            <span><Layers3 :size="13" />模型</span>
            <n-select
              data-testid="conversation-model-select"
              size="small"
              :value="ai.selectedModelProfileId.value ?? null"
              :options="ai.modelOptions.value"
              :loading="ai.loadingModels.value"
              :disabled="ai.busy.value"
              placeholder="明确选择模型"
              aria-label="AI 对话模型"
              @update:value="selectModel"
            />
          </label>
          <label class="control-field mode-field">
            <span><ShieldCheck :size="13" />请求模式</span>
            <n-select
              data-testid="conversation-mode-select"
              size="small"
              :value="ai.requestedMode.value"
              :options="modeOptions"
              :loading="ai.autonomyBusy.value"
              :disabled="ai.busy.value || Boolean(ai.conversationReadOnlyReason.value)"
              aria-label="AI 会话请求模式"
              @update:value="selectAutonomy"
            />
          </label>
          <div class="mode-status" data-testid="conversation-mode-status" aria-label="请求模式与实际模式">
            <span>请求 <strong>{{ requestedModeLabel }}</strong></span>
            <span class="mode-arrow" aria-hidden="true">→</span>
            <span>实际 <strong>{{ effectiveModeLabel }}</strong></span>
          </div>
        </div>
      </div>
      <article v-for="entry in ai.messages.value" :key="entry.id" class="message" :class="entry.role">
        <div class="avatar">
          <Bot v-if="entry.role === 'assistant'" :size="17" />
          <UserRound v-else :size="17" />
        </div>
        <div class="bubble" :class="{ pending: entry.pending, error: entry.error }">
          <p>{{ entry.content }}</p>
          <div v-if="entry.images?.length" class="message-images" aria-label="本轮图片证据">
            <img
              v-for="image in entry.images"
              :key="image.id"
              :src="image.previewHref"
              :alt="`本轮图片：${image.fileName}`"
            />
          </div>
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
          <details
            v-if="entry.trace?.stages?.length || entry.trace?.imageEvidence?.length"
            class="tool-trace"
          >
            <summary>
              <ChevronDown :size="14" />
              视觉与最终模型 Trace
              <small>{{ entry.trace.imageEvidence?.length ?? 0 }} 张证据</small>
            </summary>
            <div class="tool-list">
              <span v-for="stage in entry.trace.stages ?? []" :key="`${stage.profileId}-${stage.purpose}`">
                {{ stage.purpose === 'vision_and_final'
                  ? '直接视觉'
                  : stage.purpose === 'vision_observation'
                    ? '视觉观察'
                    : '最终回答' }}
                · v{{ stage.profileVersion }}
                · {{ stage.totalTokens }} tokens
              </span>
              <span v-for="evidence in entry.trace.imageEvidence ?? []" :key="evidence.imageId">
                图片 {{ evidence.displayOrder + 1 }} · SHA {{ evidence.sha256.slice(0, 12) }}
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
      <section
        v-if="ai.stagedImages.value.length"
        class="image-staging"
        aria-label="待发送图片"
      >
        <div class="image-staging-heading">
          <span><ImagePlus :size="15" />暂存图片 {{ ai.stagedImages.value.length }}/8</span>
          <small>仅发送成功后清空</small>
        </div>
        <div class="staged-image-list">
          <article v-for="image in ai.stagedImages.value" :key="image.localId">
            <img :src="image.previewUrl" :alt="`待发送图片：${image.file.name}`" />
            <div>
              <strong>{{ image.file.name }}</strong>
              <small v-if="image.status === 'uploading'">正在上传…</small>
              <small v-else-if="image.status === 'ready'">已安全暂存</small>
              <small v-else-if="image.status === 'error'" class="image-error">
                {{ image.error }}
              </small>
              <small v-else>{{ (image.file.size / 1048576).toFixed(1) }} MiB</small>
            </div>
            <button
              type="button"
              :aria-label="`移除图片 ${image.file.name}`"
              :disabled="ai.busy.value"
              @click="ai.removeStagedImage(image.localId)"
            >
              <Trash2 :size="15" />
            </button>
          </article>
        </div>
        <div class="vision-route">
          <template v-if="ai.visionRoute.value === 'direct'">
            <ScanSearch :size="15" />
            <span>
              当前对话模型支持视觉，将直接读取图片并回答。
            </span>
          </template>
          <template v-else>
            <label for="chat-vision-model">
              <ScanSearch :size="15" />
              <span>视觉中转模型</span>
            </label>
            <n-select
              id="chat-vision-model"
              size="small"
              :value="ai.selectedVisionModelProfileId.value ?? null"
              :options="ai.visionModelOptions.value"
              :disabled="ai.busy.value"
              clearable
              placeholder="必须明确选择"
              aria-label="聊天图片视觉中转模型"
              @update:value="selectVisionModel"
            />
            <small>视觉模型只生成受控观察，最终回答仍由当前对话模型完成。</small>
          </template>
        </div>
      </section>
      <p
        v-if="ai.imageStageError.value"
        class="image-stage-error"
        role="alert"
        aria-live="assertive"
      >
        {{ ai.imageStageError.value }}
      </p>
      <div class="suggestions">
        <button
          v-for="item in suggestions"
          :key="item"
          type="button"
          :disabled="ai.busy.value || Boolean(ai.composerDisabledReason.value)"
          @click="send(item)"
        >{{ item }}</button>
      </div>
      <div
        v-if="ai.composerDisabledReason.value"
        class="composer-blocked"
        role="status"
        data-testid="composer-disabled-reason"
      >
        <AlertTriangle :size="15" />
        <span>{{ ai.composerDisabledReason.value }}</span>
      </div>
      <div class="input-wrap">
        <input
          ref="imageInput"
          class="visually-hidden"
          type="file"
          multiple
          accept="image/jpeg,image/png,image/webp,image/gif"
          aria-label="选择最多八张聊天图片"
          @change="selectImages"
        />
        <n-button
          quaternary
          circle
          :disabled="ai.busy.value || Boolean(ai.conversationReadOnlyReason.value)"
          aria-label="添加聊天图片"
          data-testid="ai-image-picker"
          @click="openImagePicker"
        >
          <template #icon><ImagePlus :size="18" /></template>
        </n-button>
        <textarea
          v-model="ai.composerDraft.value"
          data-testid="ai-composer-input"
          rows="2"
          placeholder="询问动物、实验或数据…"
          :disabled="ai.busy.value || Boolean(ai.composerDisabledReason.value)"
          @keydown="onKeydown"
        />
        <n-button
          type="primary"
          circle
          :disabled="!canSend || ai.busy.value || Boolean(ai.composerDisabledReason.value)"
          aria-label="发送"
          data-testid="ai-composer-send"
          @click="send()"
        >
          <template #icon><CornerDownLeft :size="17" /></template>
        </n-button>
      </div>
      <div class="safety-note"><ShieldCheck :size="13" /> {{ autonomyDescription }} 科研签署和高风险操作始终由人工确认。</div>
    </div>

    <n-modal
      v-model:show="autonomyModalOpen"
      preset="card"
      :title="autonomyModalPurpose === 'start' ? '以 Full 请求开始新会话' : '请求当前会话的 Full 模式'"
      class="autonomy-modal"
      @after-leave="closeFullModal"
    >
      <n-alert type="warning" :bordered="false">
        Full 不是新角色，也不会扩大你的项目权限。动物转移/死亡、删除与批量导入、科研签署、繁育事实、账号权限和日志清理仍无法自动执行。
      </n-alert>
      <div class="autonomy-boundary-list">
        <span>仅当前会话</span><span>30 分钟无活动降级</span><span>普通批量最多 100 条</span>
      </div>
      <p v-if="!ai.reinforcedPasswordRequired.value" class="native-confirmation-note">
        <ShieldCheck :size="14" />
        桌面端还会在原生边界确认本次启动声明；取消或验证失败时不会调用模型。
      </p>
      <n-input
        v-if="ai.reinforcedPasswordRequired.value"
        v-model:value="autonomyPassword"
        type="password"
        show-password-on="click"
        autocomplete="current-password"
        maxlength="1024"
        data-testid="full-start-password"
        placeholder="输入当前登录密码完成身份确认"
      />
      <n-checkbox v-model:checked="autonomyDeclared" data-testid="full-start-declaration">
        我理解 Full 仅是当前会话的受限委托，不会绕过人工审批和签署
      </n-checkbox>
      <template #footer>
        <div class="modal-actions">
          <n-button
            data-testid="cancel-full-start"
            :disabled="ai.autonomyBusy.value || ai.startingConversation.value"
            @click="closeFullModal"
          >取消</n-button>
          <n-button
            type="primary"
            :loading="ai.autonomyBusy.value || ai.startingConversation.value"
            :disabled="!autonomyDeclared || (ai.reinforcedPasswordRequired.value && !autonomyPassword)"
            data-testid="confirm-full-start"
            @click="applyFullAutonomy"
          >确认启用</n-button>
        </div>
      </template>
    </n-modal>

    <n-modal
      v-model:show="modelSwitchModalOpen"
      preset="card"
      title="使用所选模型开始新会话？"
      class="autonomy-modal"
      @after-leave="cancelModelSwitch"
    >
      <n-alert type="warning" :bordered="false">
        当前会话已有持久消息，模型绑定不能修改。确认后会创建新的空会话。
      </n-alert>
      <ul class="switch-boundaries">
        <li>保留当前科研项目范围</li>
        <li>保留尚未发送的输入</li>
        <li>不继承消息、工具结果、当前会话草稿或 Full 授权</li>
      </ul>
      <template #footer>
        <div class="modal-actions">
          <n-button data-testid="cancel-model-switch" @click="cancelModelSwitch">取消</n-button>
          <n-button type="primary" data-testid="confirm-model-switch" @click="confirmModelSwitch">
            开始新会话
          </n-button>
        </div>
      </template>
    </n-modal>
  </section>
</template>

<style scoped>
.ai-conversation { display: grid; grid-template-rows: minmax(0, 1fr) auto; min-height: 0; height: 100%; background: var(--muri-surface); }
.message-list { overflow: auto; padding: 20px; }
.context-strip { display: flex; width: min(100%, 760px); min-width: 0; margin: 0 auto 20px; padding: 10px; flex-direction: column; gap: 9px; border: 1px solid var(--muri-border); border-radius: 10px; color: var(--muri-text-secondary); background: var(--muri-surface-muted); font-size: 12px; }
.context-scope { display: flex; min-width: 0; align-items: center; justify-content: center; flex-wrap: wrap; gap: 6px; }.context-scope > svg { flex: 0 0 auto; color: var(--muri-primary); }.scope-divider { color: var(--muri-border-strong); }
.conversation-controls { display: grid; min-width: 0; grid-template-columns: minmax(180px, 1.5fr) minmax(112px, .65fr) auto; align-items: end; gap: 9px; }
.control-field { display: grid; min-width: 0; gap: 4px; }.control-field > span { display: flex; align-items: center; gap: 4px; color: var(--muri-text-tertiary); font-size: 10px; font-weight: 600; }.control-field > span svg { color: var(--muri-primary); }.control-field :deep(.n-select) { min-width: 0; width: 100%; }.mode-status { display: flex; min-height: 34px; align-items: center; justify-content: center; flex-wrap: wrap; gap: 5px; padding: 5px 8px; border: 1px solid var(--muri-border); border-radius: 7px; background: white; color: var(--muri-text-secondary); font-size: 11px; }.mode-status strong { color: var(--muri-text); }.mode-arrow { color: var(--muri-primary); }
.message { display: flex; align-items: flex-start; gap: 10px; max-width: 760px; margin: 0 auto 16px; }.message.user { flex-direction: row-reverse; }
.avatar { display: grid; flex: 0 0 30px; width: 30px; height: 30px; place-items: center; border: 1px solid var(--muri-border); border-radius: 50%; color: var(--muri-primary); background: white; }.user .avatar { color: var(--muri-text-secondary); }
.bubble { max-width: min(82%, 640px); padding: 10px 13px; border: 1px solid var(--muri-border); border-radius: 4px 12px 12px; background: var(--muri-surface-muted); line-height: 1.65; }.user .bubble { border-color: #c8deef; border-radius: 12px 4px 12px 12px; background: var(--muri-primary-soft); }.bubble.error { border-color: #efd0d0; color: var(--muri-danger); background: #fff7f7; }.bubble.pending { color: var(--muri-text-secondary); animation: soft-pulse 1.3s ease-in-out infinite; }.bubble p { margin: 0; white-space: pre-wrap; }
.message-images { display: grid; grid-template-columns: repeat(auto-fit, minmax(96px, 1fr)); gap: 6px; margin-top: 9px; }.message-images img { width: 100%; height: 112px; border: 1px solid var(--muri-border); border-radius: 7px; object-fit: cover; }
.citations { display: flex; flex-wrap: wrap; gap: 6px; margin-top: 8px; }.citations a, .citations span { padding: 3px 8px; border-radius: 999px; color: var(--muri-primary); background: white; font-size: 12px; }
.tool-trace { margin-top: 9px; border-top: 1px solid var(--muri-border); padding-top: 7px; }.tool-trace summary { display: flex; align-items: center; gap: 5px; color: var(--muri-text-tertiary); cursor: pointer; font-size: 11px; list-style: none; }.tool-trace summary small { margin-left: auto; }.tool-trace[open] summary svg { transform: rotate(180deg); }.tool-list { display: flex; flex-wrap: wrap; gap: 5px; margin-top: 7px; }.tool-list span { padding: 3px 6px; border-radius: 4px; background: white; color: var(--muri-text-secondary); font-family: ui-monospace, monospace; font-size: 10px; }
.draft-section { max-width: 760px; margin: 22px auto 0; }.draft-section-title { display: flex; align-items: center; gap: 6px; margin-bottom: 8px; color: var(--muri-text); font-weight: 650; }.draft-section-title svg { color: var(--muri-primary); }
.draft-card { margin-bottom: 10px; padding: 13px; border: 1px solid #c8deef; border-radius: 9px; background: #fbfdff; }.draft-card header { display: flex; align-items: flex-start; justify-content: space-between; gap: 12px; }.draft-card header > div { display: flex; flex-direction: column; }.draft-card header span { color: var(--muri-text-tertiary); font-size: 11px; }.diff-list { margin: 11px 0; border: 1px solid var(--muri-border); border-radius: 6px; overflow: hidden; }.diff-row { display: grid; grid-template-columns: minmax(100px, 1fr) minmax(80px, 1fr) 18px minmax(80px, 1fr); gap: 6px; align-items: center; padding: 7px 9px; border-bottom: 1px solid var(--muri-border); font-size: 11px; }.diff-row:last-child { border-bottom: 0; }.diff-row code { overflow: hidden; color: var(--muri-text-secondary); text-overflow: ellipsis; }.diff-row .before { color: var(--muri-danger); text-decoration: line-through; }.diff-row .after { color: var(--muri-success); }.arrow { color: var(--muri-text-tertiary); text-align: center; }
.signature-box { display: flex; flex-direction: column; gap: 8px; margin-top: 10px; }.draft-actions { display: flex; gap: 8px; margin-top: 10px; }
.composer { padding: 12px 18px 16px; border-top: 1px solid var(--muri-border); background: white; }.suggestions { display: flex; gap: 7px; max-width: 760px; margin: 0 auto 8px; overflow-x: auto; scrollbar-width: none; }.suggestions button { flex: 0 0 auto; padding: 5px 9px; border: 1px solid var(--muri-border); border-radius: 999px; color: var(--muri-text-secondary); background: white; cursor: pointer; transition: border-color var(--muri-transition-fast), color var(--muri-transition-fast); }.suggestions button:hover { border-color: var(--muri-primary); color: var(--muri-primary); }
.image-staging { display: grid; max-width: 760px; gap: 8px; margin: 0 auto 10px; padding: 10px; border: 1px solid var(--muri-border); border-radius: 9px; background: var(--muri-surface-muted); }.image-staging-heading { display: flex; align-items: center; justify-content: space-between; gap: 8px; color: var(--muri-text-secondary); font-size: 11px; }.image-staging-heading > span { display: flex; align-items: center; gap: 5px; color: var(--muri-text); font-weight: 650; }.image-staging-heading svg { color: var(--muri-primary); }.image-staging-heading small { color: var(--muri-text-tertiary); }
.staged-image-list { display: grid; grid-template-columns: repeat(auto-fit, minmax(150px, 1fr)); gap: 7px; }.staged-image-list article { display: grid; min-width: 0; grid-template-columns: 46px minmax(0, 1fr) 30px; align-items: center; gap: 7px; padding: 6px; border: 1px solid var(--muri-border); border-radius: 7px; background: white; }.staged-image-list img { width: 46px; height: 46px; border-radius: 5px; object-fit: cover; }.staged-image-list article > div { display: flex; min-width: 0; flex-direction: column; }.staged-image-list strong, .staged-image-list small { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }.staged-image-list strong { color: var(--muri-text); font-size: 11px; }.staged-image-list small { color: var(--muri-text-tertiary); font-size: 10px; }.staged-image-list small.image-error { color: var(--muri-danger); }.staged-image-list button { display: grid; width: 30px; height: 30px; padding: 0; place-items: center; border: 0; border-radius: 6px; color: var(--muri-text-tertiary); background: transparent; cursor: pointer; transition: color var(--muri-transition-fast), background var(--muri-transition-fast); }.staged-image-list button:hover { color: var(--muri-danger); background: #fff1f1; }.staged-image-list button:focus-visible { outline: 3px solid rgba(15, 95, 170, .22); outline-offset: 1px; }.staged-image-list button:disabled { cursor: wait; opacity: .55; }
.vision-route { display: grid; grid-template-columns: auto minmax(160px, 260px) minmax(150px, 1fr); align-items: center; gap: 7px; color: var(--muri-text-secondary); font-size: 11px; }.vision-route > svg { color: var(--muri-primary); }.vision-route > label { display: flex; align-items: center; gap: 5px; color: var(--muri-text); font-weight: 600; }.vision-route > label svg { color: var(--muri-primary); }.vision-route > small { color: var(--muri-text-tertiary); line-height: 1.45; }.image-stage-error { max-width: 760px; margin: 0 auto 8px; color: var(--muri-danger); font-size: 11px; }
.suggestions button:focus-visible { outline: 3px solid rgba(15, 95, 170, .2); outline-offset: 1px; }.suggestions button:disabled { color: var(--muri-text-tertiary); background: var(--muri-surface-muted); cursor: not-allowed; opacity: .7; }
.composer-blocked { display: flex; max-width: 760px; min-width: 0; align-items: flex-start; gap: 6px; margin: 0 auto 8px; padding: 7px 9px; border: 1px solid #efd8b7; border-radius: 7px; color: #8a5515; background: #fffaf2; font-size: 11px; line-height: 1.45; }.composer-blocked svg { flex: 0 0 auto; margin-top: 1px; }
.input-wrap { display: flex; align-items: flex-end; gap: 8px; max-width: 760px; margin: 0 auto; padding: 8px; border: 1px solid var(--muri-border-strong); border-radius: 10px; transition: border-color var(--muri-transition-fast), box-shadow var(--muri-transition-fast); }.input-wrap:focus-within { border-color: var(--muri-primary); box-shadow: 0 0 0 3px rgba(15, 95, 170, 0.1); }textarea { flex: 1; min-height: 42px; max-height: 130px; padding: 3px 5px; resize: none; border: 0; outline: 0; color: var(--muri-text); background: transparent; line-height: 1.5; }.safety-note { display: flex; align-items: center; justify-content: center; gap: 5px; margin-top: 7px; color: var(--muri-text-tertiary); font-size: 11px; }
.visually-hidden { position: absolute; width: 1px; height: 1px; padding: 0; overflow: hidden; clip: rect(0, 0, 0, 0); white-space: nowrap; border: 0; }
.compact .message-list { padding: 16px 14px; }.compact .composer { padding: 10px 12px 12px; }.compact .suggestions { max-width: 100%; }
.autonomy-modal { width: min(520px, calc(100vw - 28px)); }.autonomy-boundary-list { display: flex; flex-wrap: wrap; gap: 6px; margin: 14px 0; }.autonomy-boundary-list span { padding: 4px 8px; border-radius: 999px; color: var(--muri-primary); background: var(--muri-primary-soft); font-size: 12px; }.autonomy-modal :deep(.n-checkbox) { margin-top: 14px; }.modal-actions { display: flex; justify-content: flex-end; gap: 8px; }
.native-confirmation-note { display: flex; align-items: flex-start; gap: 6px; margin: 0 0 2px; color: var(--muri-text-secondary); font-size: 12px; line-height: 1.5; }.native-confirmation-note svg { flex: 0 0 auto; margin-top: 2px; color: var(--muri-primary); }
.switch-boundaries { display: grid; gap: 7px; margin: 14px 0 0; padding-left: 22px; color: var(--muri-text-secondary); line-height: 1.55; }
@keyframes soft-pulse { 50% { opacity: 0.55; } }
@media (max-width: 700px) { .conversation-controls { grid-template-columns: minmax(0, 1fr) minmax(104px, .48fr); }.mode-status { grid-column: 1 / -1; }.diff-row { grid-template-columns: 1fr 18px 1fr; }.diff-row code { grid-column: 1 / -1; }.draft-card { padding: 11px; }.vision-route { grid-template-columns: auto minmax(0, 1fr); }.vision-route > small { grid-column: 1 / -1; } }
@media (max-width: 430px) { .message-list { padding: 14px 10px; }.context-strip { padding: 9px; }.conversation-controls { grid-template-columns: minmax(0, 1fr); }.mode-status { grid-column: auto; }.composer { padding-inline: 10px; }.safety-note { align-items: flex-start; text-align: center; }.bubble { max-width: calc(100% - 40px); overflow-wrap: anywhere; }.staged-image-list { grid-template-columns: minmax(0, 1fr); }.image-staging-heading { align-items: flex-start; flex-direction: column; }.message-images { grid-template-columns: repeat(2, minmax(0, 1fr)); } }
@media (prefers-reduced-motion: reduce) { .bubble.pending { animation: none; }.tool-trace summary svg { transition: none; } }
</style>
