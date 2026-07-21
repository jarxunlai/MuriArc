<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { RefreshCw, Search, ShieldCheck, Trash2 } from '@lucide/vue'
import { useMessage } from 'naive-ui'
import PageHeader from '@/components/PageHeader.vue'
import {
  currentAuthSession,
  gateway,
  type OperationRecord,
  type TechnicalLogCleanupPreview,
  type TechnicalLogPolicy,
} from '@/services/gateway'
import { currentProjectId } from '@/services/projectContext'

const message = useMessage()
const loading = ref(false)
const items = ref<OperationRecord[]>([])
const scope = ref<string | null>(null)
const source = ref<string | null>(null)
const search = ref('')
const rootLogAdmin = computed(() => currentAuthSession.value?.user.isEnvironmentRoot === true
  && Boolean(gateway.getTechnicalLogPolicy))
const logPolicy = ref<TechnicalLogPolicy>()
const maxRows = ref(20_000)
const minRetentionDays = ref(30)
const cleanupPreview = ref<TechnicalLogCleanupPreview>()
const logBusy = ref(false)

const scopes = [
  { label: '全部活动', value: null },
  { label: '动物', value: 'animal' },
  { label: '实验', value: 'experiment' },
  { label: '项目', value: 'project' },
  { label: 'AI', value: 'ai' },
]
const sources = [
  { label: '全部来源', value: null },
  { label: 'Web', value: 'web' },
  { label: '桌面端', value: 'desktop' },
  { label: 'API', value: 'api' },
  { label: 'AI', value: 'ai' },
  { label: 'MCP', value: 'mcp' },
  { label: '迁移', value: 'migration' },
]
const filtered = computed(() => {
  const term = search.value.trim().toLowerCase()
  return term
    ? items.value.filter((item) => `${item.title} ${item.summary} ${item.actor.display_name}`.toLowerCase().includes(term))
    : items.value
})

async function load() {
  if (!gateway.listOperations) return
  loading.value = true
  try {
    const query = new URLSearchParams()
    if (scope.value) query.set('scope', scope.value)
    if (source.value) query.set('source', source.value)
    items.value = await gateway.listOperations(query)
  } catch (error) {
    message.error(error instanceof Error ? error.message : '无法读取活动记录')
  } finally {
    loading.value = false
  }
}

async function loadLogPolicy() {
  if (!rootLogAdmin.value || !gateway.getTechnicalLogPolicy) return
  logBusy.value = true
  try {
    logPolicy.value = await gateway.getTechnicalLogPolicy()
    maxRows.value = logPolicy.value.maxRows
    minRetentionDays.value = logPolicy.value.minRetentionDays
  } catch (error) {
    message.error(error instanceof Error ? error.message : '无法读取技术日志策略')
  } finally {
    logBusy.value = false
  }
}

async function saveLogPolicy() {
  if (!gateway.saveTechnicalLogPolicy || !logPolicy.value) return
  logBusy.value = true
  try {
    logPolicy.value = await gateway.saveTechnicalLogPolicy({
      maxRows: maxRows.value,
      minRetentionDays: minRetentionDays.value,
      expectedRevision: logPolicy.value.revision,
    })
    cleanupPreview.value = undefined
    message.success('技术日志自动清理策略已保存')
  } catch (error) {
    message.error(error instanceof Error ? error.message : '无法保存技术日志策略')
  } finally {
    logBusy.value = false
  }
}

async function previewLogCleanup() {
  if (!gateway.previewTechnicalLogCleanup) return
  logBusy.value = true
  try {
    cleanupPreview.value = await gateway.previewTechnicalLogCleanup()
  } catch (error) {
    message.error(error instanceof Error ? error.message : '无法预览技术日志清理')
  } finally {
    logBusy.value = false
  }
}

async function cleanTechnicalLogs() {
  if (!gateway.cleanupTechnicalLogs || !cleanupPreview.value) return
  logBusy.value = true
  try {
    cleanupPreview.value = await gateway.cleanupTechnicalLogs({
      expectedPolicyRevision: cleanupPreview.value.policyRevision,
      expectedEligibleRows: cleanupPreview.value.eligibleRows,
    })
    message.success('符合保留策略的技术日志已删除，正式 Audit 与 Provenance 未受影响')
  } catch (error) {
    message.error(error instanceof Error ? error.message : '技术日志清理失败')
  } finally {
    logBusy.value = false
  }
}

function date(value: string) {
  return new Intl.DateTimeFormat('zh-CN', {
    month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', second: '2-digit',
  }).format(new Date(value))
}

watch([scope, source], load)
watch(currentProjectId, () => {
  if (gateway.mode === 'remote') void load()
})
onMounted(() => {
  void load()
  void loadLogPolicy()
})
</script>

<template>
  <div class="page activity-page">
    <PageHeader title="活动记录" section="管理与工具" description="只展示会影响动物、项目、实验或数据的关键活动；同一批次合并为一条摘要。">
      <template #actions><n-button secondary :loading="loading" @click="load"><template #icon><RefreshCw :size="16" /></template>刷新</n-button></template>
    </PageHeader>

    <n-alert type="info" :show-icon="false" class="audit-note">
      活动记录用于日常查看。正式 Audit 与 Provenance 仍保留逐实体、revision 和前后差异，不参与自动清理。
    </n-alert>

    <section v-if="rootLogAdmin" class="log-policy surface">
      <div class="log-policy-copy">
        <ShieldCheck :size="18" />
        <div><strong>技术日志保留</strong><span>仅 Environment Root 可设置或删除。默认最多 20,000 条且至少保留 30 天；正式审计永久保留。</span></div>
      </div>
      <n-input-number v-model:value="maxRows" :min="1000" :max="1000000" :step="1000" aria-label="技术日志最大条数" />
      <n-input-number v-model:value="minRetentionDays" :min="1" :max="3650" aria-label="技术日志最短保留天数" />
      <n-button secondary :loading="logBusy" @click="saveLogPolicy">保存策略</n-button>
      <n-button secondary :loading="logBusy" @click="previewLogCleanup">预览清理</n-button>
      <div v-if="cleanupPreview" class="cleanup-preview">
        <span>现有 {{ cleanupPreview.totalRows }} 条；按当前策略可删除 {{ cleanupPreview.eligibleRows }} 条。</span>
        <n-popconfirm
          positive-text="确认删除"
          negative-text="取消"
          :disabled="cleanupPreview.eligibleRows === 0"
          @positive-click="cleanTechnicalLogs"
        >
          <template #trigger><n-button type="error" secondary :disabled="cleanupPreview.eligibleRows === 0"><template #icon><Trash2 :size="15" /></template>删除可清理日志</n-button></template>
          只删除已预览且超过数量与最短保留期的技术日志；该动作本身写入永久审计。
        </n-popconfirm>
      </div>
    </section>

    <section class="filters surface">
      <n-input v-model:value="search" clearable placeholder="搜索活动或操作者"><template #prefix><Search :size="16" /></template></n-input>
      <n-select v-model:value="scope" :options="scopes" />
      <n-select v-model:value="source" :options="sources" />
      <span>{{ filtered.length }} 条关键活动</span>
    </section>

    <n-spin :show="loading">
      <section v-if="filtered.length" class="activity-list surface">
        <article v-for="item in filtered" :key="item.id" class="activity-row">
          <time :datetime="item.occurredAt">{{ date(item.occurredAt) }}</time>
          <span class="activity-dot" />
          <div class="activity-content">
            <header>
              <strong>{{ item.title }}</strong>
              <n-tag v-if="item.batchCount > 1" size="small" type="info" :bordered="false">批量 {{ item.batchCount }} 项</n-tag>
              <n-tag size="small" :bordered="false">{{ item.source }}</n-tag>
            </header>
            <p>{{ item.summary }}</p>
            <small>{{ item.actor.display_name }}<template v-if="item.reason"> · {{ item.reason }}</template></small>
          </div>
        </article>
      </section>
      <n-empty v-else-if="!loading" description="没有匹配的关键活动" class="empty-result" />
    </n-spin>
  </div>
</template>

<style scoped>
.audit-note { margin-bottom: 12px; }
.log-policy { display: grid; grid-template-columns: minmax(280px, 1fr) 150px 150px auto auto; align-items: center; gap: 10px; margin-bottom: 12px; padding: 12px; }.log-policy-copy { display: flex; align-items: flex-start; gap: 8px; }.log-policy-copy svg { flex: 0 0 auto; color: var(--muri-primary); }.log-policy-copy div { display: flex; flex-direction: column; }.log-policy-copy span,.cleanup-preview span { color: var(--muri-text-tertiary); font-size: 11px; }.cleanup-preview { display: flex; grid-column: 1 / -1; align-items: center; justify-content: flex-end; gap: 12px; padding-top: 8px; border-top: 1px solid var(--muri-border); }
.filters { display: grid; grid-template-columns: minmax(260px, 1fr) 150px 150px auto; align-items: center; gap: 10px; padding: 11px; margin-bottom: 12px; }
.filters > span { color: var(--muri-text-tertiary); font-size: 12px; }
.activity-list { padding: 4px 16px; }
.activity-row { display: grid; grid-template-columns: 116px 14px minmax(0, 1fr); gap: 12px; padding: 16px 0; border-bottom: 1px solid var(--muri-border); }
.activity-row:last-child { border-bottom: 0; }
.activity-row time { padding-top: 2px; color: var(--muri-text-tertiary); font-size: 12px; }
.activity-dot { width: 9px; height: 9px; margin-top: 5px; border: 2px solid white; border-radius: 50%; background: var(--muri-primary); box-shadow: 0 0 0 2px var(--muri-primary-soft); }
.activity-content header { display: flex; align-items: center; gap: 7px; }
.activity-content header strong { margin-right: auto; font-size: 14px; }
.activity-content p { margin: 5px 0 3px; color: var(--muri-text-secondary); }
.activity-content small { color: var(--muri-text-tertiary); }
.empty-result { padding: 80px 0; }
@media (max-width: 900px) {
  .log-policy { grid-template-columns: 1fr 1fr; }.log-policy-copy,.cleanup-preview { grid-column: 1 / -1; }.cleanup-preview { align-items: stretch; flex-direction: column; }
  .filters { grid-template-columns: 1fr 1fr; }
  .filters > :first-child { grid-column: 1 / -1; }
  .filters > span { display: none; }
  .activity-list { padding: 2px 13px; }
  .activity-row { grid-template-columns: 12px minmax(0, 1fr); gap: 10px; }
  .activity-row time { grid-column: 2; grid-row: 2; padding: 0; }
  .activity-dot { grid-column: 1; grid-row: 1 / span 2; }
  .activity-content { grid-column: 2; grid-row: 1; }
}
</style>
