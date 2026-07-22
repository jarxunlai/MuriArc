<script setup lang="ts">
import { computed, onMounted, reactive, ref } from 'vue'
import { useMessage } from 'naive-ui'
import { useRoute, useRouter } from 'vue-router'
import { Archive, BookOpen, CheckCircle2, Download, FileSpreadsheet, Plus, RefreshCw, UploadCloud, XCircle } from '@lucide/vue'
import type { DataJob, Experiment } from '@/domain/models'
import { gateway } from '@/services/gateway'
import {
  createDataGateway,
  defaultAnimalExportOptions,
  type AnimalExportField,
  type AnimalImportSchema,
  type DataArtifact,
  type ImportKind,
  type ImportPreview,
  type ImportReceipt,
} from '@/services/dataGateway'
import {
  canCreateSnapshot,
  canExportData,
  canImportData,
  currentProjectId,
  hasLabRegistryAccess,
} from '@/services/projectContext'
import {
  assignImportTarget,
  canonicalMappingFromEditable,
  editableMappingFromCanonical,
  sameCanonicalMapping,
  type EditableImportMapping,
} from '@/services/importMapping'
import PageHeader from '@/components/PageHeader.vue'

const message = useMessage()
const route = useRoute()
const router = useRouter()
const dataGateway = createDataGateway(gateway)
const jobs = ref<DataJob[]>([])
const experiments = ref<Experiment[]>([])
const loading = ref(true)
const busy = ref(false)
const step = ref(1)
const selectedFile = ref<File>()
const preview = ref<ImportPreview>()
const editableMapping = ref<EditableImportMapping>({})
const receipt = ref<ImportReceipt>()
const input = ref<HTMLInputElement | null>(null)
const animalImportSchema = ref<AnimalImportSchema>()
const showExport = ref(false)
const exportFormat = ref<'csv' | 'xlsx'>('xlsx')
const exportOptions = reactive(defaultAnimalExportOptions())
const birthDateRange = ref<[number, number] | null>(null)
const registeredAtRange = ref<[number, number] | null>(null)
const assessedAtRange = ref<[number, number] | null>(null)
const genotypeDefinitions = ref<Array<{ label: string; value: string }>>([])
const animalDataMode = computed(() => route.meta.animalData === true)
const animalImportAllowed = computed(() => gateway.mode === 'local' || hasLabRegistryAccess())
const importAllowed = computed(() => (gateway.mode === 'local' || canImportData())
  && (!animalDataMode.value || animalImportAllowed.value))
const exportAllowed = computed(() => gateway.mode === 'local' || canExportData())
const snapshotAllowed = computed(() => gateway.mode === 'local' || canCreateSnapshot())
const importKind = ref<ImportKind>(animalImportAllowed.value ? 'animal' : 'measurement')
const selectedExperimentId = ref<string>()
const stepLabels = ['选择文件', '识别字段', '检查与预览', '事务写入']
const supportsXlsxTemplates = computed(() => dataGateway.animalImportTemplateFormats.includes('xlsx'))
const pageTitle = computed(() => animalDataMode.value ? '动物数据' : '数据中心')
const pageDescription = computed(() => animalDataMode.value
  ? '批量登记动物、查看生产 schema 与模板，并按条件生成不含 UUID 的业务导出。'
  : '导入实验测量、查看任务历史并创建完整业务归档快照。')
const jobStatusMeta: Record<DataJob['status'], { label: string; type: 'default' | 'info' | 'warning' | 'success' | 'error' }> = {
  queued: { label: '排队中', type: 'default' },
  running: { label: '处理中', type: 'info' },
  'needs-review': { label: '需要复核', type: 'warning' },
  completed: { label: '已完成', type: 'success' },
  failed: { label: '失败', type: 'error' },
  cancelled: { label: '已取消', type: 'default' },
}

const animalTargets = ['display_id', 'sex', 'birth_date', 'strain', 'cage', 'genotype', 'father', 'mother']
const measurementTargets = ['animal_uuid', 'display_id', 'measurement_key', 'value_type', 'value', 'unit', 'measured_at']
const requiredTargets = new Set(['display_id', 'measurement_key', 'value_type', 'value', 'unit', 'measured_at'])
const targetLabels: Record<string, string> = {
  display_id: '动物编号',
  sex: '性别',
  birth_date: '出生日期',
  strain: '品系',
  cage: '笼位',
  genotype: '基因型',
  father: '父本',
  mother: '母本',
  animal_uuid: '动物 UUID',
  measurement_key: '测量指标',
  value_type: '值类型',
  value: '测量值',
  unit: '单位',
  measured_at: '测量时间',
}
const exportFieldOptions: Array<{ label: string; value: AnimalExportField; disabled?: boolean }> = [
  { label: '编号 scope（固定）', value: 'identifier_scope', disabled: true },
  { label: '项目名称（固定）', value: 'project_name', disabled: true },
  { label: '动物显示编号（固定）', value: 'display_id', disabled: true },
  { label: '性别', value: 'sex' },
  { label: '出生日期', value: 'birth_date' },
  { label: '登记时间', value: 'registered_at' },
  { label: '品系', value: 'strain' },
  { label: '当前状态', value: 'status' },
  { label: '区域/location', value: 'cage_location' },
  { label: 'section', value: 'cage_section' },
  { label: '笼位编号', value: 'cage_display_id' },
  { label: '当前基因型摘要', value: 'current_genotype_summary' },
]
const exportStatusOptions = [
  ['planned', '计划中'], ['alive', '在养'], ['in_experiment', '实验中'], ['sampled', '已采样'],
  ['deceased', '死亡'], ['euthanized', '已安乐死'], ['lost', '失联'], ['archived', '已归档'],
].map(([value, label]) => ({ value, label }))
const exportGenotypingStateOptions = [
  { value: 'unknown', label: '未知' },
  { value: 'expected', label: '预期' },
  { value: 'confirmed', label: '已确认' },
  { value: 'rejected', label: '已排除' },
]

const experimentOptions = computed(() => experiments.value.map((experiment) => ({
  label: `${experiment.name} · ${experiment.project}`,
  value: experiment.id,
})))
const canChooseFile = computed(() => importKind.value === 'animal' || Boolean(selectedExperimentId.value))
const importTitle = computed(() => importKind.value === 'measurement' ? '导入实验测量数据' : '导入动物登记数据')

const targetOptions = computed(() => (
  preview.value?.importKind === 'measurement' ? measurementTargets : animalTargets
).map((target) => ({
  label: `${targetLabels[target] ?? target}${requiredTargets.has(target) ? ' *' : ''}`,
  value: target,
})))

const currentCanonicalMapping = computed(() => canonicalMappingFromEditable(editableMapping.value))
const mappingDirty = computed(() => Boolean(
  preview.value
  && !sameCanonicalMapping(currentCanonicalMapping.value, preview.value.mapping),
))
const mappingRows = computed(() => (preview.value?.headers ?? []).map((source, index) => ({
  key: `${index}-${source}`,
  source,
  target: editableMapping.value[source] ?? null,
  state: editableMapping.value[source] ? (mappingDirty.value ? '待校验' : '已匹配') : '未映射',
})))

const blockingCount = computed(() => preview.value?.issues.filter((issue) => issue.severity === 'error').length ?? 0)
const warningCount = computed(() => preview.value?.issues.filter((issue) => issue.severity === 'warning').length ?? 0)

async function refreshJobs() {
  loading.value = true
  try {
    jobs.value = await gateway.listDataJobs()
  } finally {
    loading.value = false
  }
}

async function selectFile(event: Event) {
  const file = (event.target as HTMLInputElement).files?.[0]
  if (!file) return
  selectedFile.value = file
  preview.value = undefined
  editableMapping.value = {}
  receipt.value = undefined
  step.value = 2
  busy.value = true
  try {
    applyPreview(await dataGateway.previewImport(file, {
      importKind: importKind.value,
      experimentId: importKind.value === 'measurement' ? selectedExperimentId.value : undefined,
    }))
    step.value = 3
    await refreshJobs()
  } catch (error) {
    message.error(error instanceof Error ? error.message : '文件解析失败')
    await resetImport(false)
  } finally {
    busy.value = false
  }
}

function applyPreview(next: ImportPreview) {
  preview.value = next
  editableMapping.value = editableMappingFromCanonical(next.headers, next.mapping)
}

function updateMapping(source: string, target: string | null) {
  editableMapping.value = assignImportTarget(editableMapping.value, source, target)
}

async function revalidateMapping() {
  const current = preview.value
  if (!current || !mappingDirty.value) return
  busy.value = true
  try {
    applyPreview(await dataGateway.remapImport(current.jobId, currentCanonicalMapping.value))
    message.success('已按新映射重新解析并生成可复核预览')
    await refreshJobs()
  } catch (error) {
    message.error(error instanceof Error ? error.message : '重新校验映射失败；原预览仍可用')
  } finally {
    busy.value = false
  }
}

async function confirmImport() {
  const current = preview.value
  if (!current || !current.canConfirm || mappingDirty.value) return
  busy.value = true
  try {
    receipt.value = await dataGateway.confirmImport(current.jobId, current.previewHash)
    step.value = 4
    message.success(`已事务写入 ${receipt.value.counts.animals + receipt.value.counts.measurements} 条核心记录`)
    await refreshJobs()
  } catch (error) {
    message.error(error instanceof Error ? error.message : '确认导入失败')
  } finally {
    busy.value = false
  }
}

async function resetImport(cancelPending = true) {
  const pendingJobId = preview.value?.jobId
  if (cancelPending && pendingJobId && !receipt.value) {
    try {
      await dataGateway.cancelImport(pendingJobId)
      await refreshJobs()
    } catch (error) {
      message.warning(error instanceof Error ? error.message : '未能取消导入任务')
    }
  }
  step.value = 1
  selectedFile.value = undefined
  preview.value = undefined
  editableMapping.value = {}
  receipt.value = undefined
  if (input.value) input.value.value = ''
}

function downloadIssueReport() {
  const current = preview.value
  if (!current) return
  const blob = new Blob([JSON.stringify({
    fileName: current.fileName,
    previewHash: current.previewHash,
    issues: current.issues,
  }, null, 2)], { type: 'application/json' })
  const url = URL.createObjectURL(blob)
  const link = document.createElement('a')
  link.href = url
  link.download = `${current.fileName}.issues.json`
  link.click()
  URL.revokeObjectURL(url)
}

function isoDate(value: number) {
  const date = new Date(value)
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, '0')}-${String(date.getDate()).padStart(2, '0')}`
}

async function downloadAnimalTemplate(
  format: 'csv' | 'xlsx',
  variant: 'blank' | 'example' = 'example',
) {
  if (format === 'xlsx' && !supportsXlsxTemplates.value) return
  busy.value = true
  try {
    await dataGateway.downloadAnimalImportTemplate(format, variant)
  } catch (error) {
    message.error(error instanceof Error ? error.message : '模板下载失败')
  } finally {
    busy.value = false
  }
}

async function createExport() {
  exportOptions.filter.birth_date_from = birthDateRange.value ? isoDate(birthDateRange.value[0]) : undefined
  exportOptions.filter.birth_date_to = birthDateRange.value ? isoDate(birthDateRange.value[1]) : undefined
  exportOptions.filter.registered_at_from = registeredAtRange.value ? new Date(registeredAtRange.value[0]).toISOString() : undefined
  exportOptions.filter.registered_at_to = registeredAtRange.value ? new Date(registeredAtRange.value[1]).toISOString() : undefined
  exportOptions.filter.assessed_at_from = assessedAtRange.value ? new Date(assessedAtRange.value[0]).toISOString() : undefined
  exportOptions.filter.assessed_at_to = assessedAtRange.value ? new Date(assessedAtRange.value[1]).toISOString() : undefined
  const created = await createAndDownload(
    () => dataGateway.createExport(exportFormat.value, currentProjectId.value, {
      filter: { ...exportOptions.filter },
      fields: [...exportOptions.fields],
      include_genotype_details: exportFormat.value === 'xlsx' && exportOptions.include_genotype_details,
    }),
    currentProjectId.value ? '项目动物业务导出已生成' : '动物业务导出已生成',
  )
  if (created) showExport.value = false
}

async function createSnapshot() {
  await createAndDownload(() => dataGateway.createSnapshot(), '完整业务归档快照已生成（当前仅供校验与留存）')
}

async function createAndDownload(factory: () => Promise<DataArtifact>, success: string): Promise<boolean> {
  busy.value = true
  try {
    const artifact = await factory()
    await dataGateway.downloadArtifact(artifact)
    message.success(`${success}（SHA-256 ${artifact.sha256.slice(0, 12)}…）`)
    await refreshJobs()
    return true
  } catch (error) {
    message.error(error instanceof Error ? error.message : '数据任务失败')
    return false
  } finally {
    busy.value = false
  }
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`
  return `${(bytes / 1024 / 1024).toFixed(1)} MiB`
}

onMounted(async () => {
  if (animalDataMode.value) importKind.value = 'animal'
  await Promise.all([
    refreshJobs(),
    animalDataMode.value
      ? Promise.resolve()
      : gateway.listExperiments().then((items) => { experiments.value = items }),
    animalImportAllowed.value && importAllowed.value
      ? dataGateway.getAnimalImportSchema().then((schema) => { animalImportSchema.value = schema })
      : Promise.resolve(),
    exportAllowed.value
      ? gateway.listGenotypeDefinitions(currentProjectId.value).then((items) => {
        genotypeDefinitions.value = items.map((definition) => ({
          label: definition.name,
          value: definition.name,
        }))
      }).catch(() => { genotypeDefinitions.value = [] })
      : Promise.resolve(),
  ])
})
</script>

<template>
  <div class="page">
    <PageHeader :title="pageTitle" :description="pageDescription">
      <template #actions>
        <n-button v-if="exportAllowed" secondary :loading="busy" @click="showExport = true">
          <template #icon><Download :size="17" /></template>配置动物导出
        </n-button>
        <n-button v-if="importAllowed" type="primary" :disabled="busy" @click="resetImport()">
          <template #icon><Plus :size="17" /></template>新建导入
        </n-button>
      </template>
    </PageHeader>

    <section class="data-layout" :class="{ compact: !importAllowed }">
      <div v-if="importAllowed" class="import-panel surface">
        <header>
          <div><UploadCloud :size="19" /><div><strong>{{ importTitle }}</strong><span>CSV / XLSX · 最大 32 MiB</span></div></div>
          <button v-if="step > 1" type="button" :disabled="busy" @click="resetImport()">取消并重新选择</button>
        </header>
        <ol class="steps" aria-label="导入进度">
          <li
            v-for="(label, index) in stepLabels"
            :key="label"
            :class="{ active: step === index + 1, done: step > index + 1 }"
            :aria-current="step === index + 1 ? 'step' : undefined"
            :aria-label="`第 ${index + 1} 步：${label}`"
          >
            <i aria-hidden="true">{{ step > index + 1 ? '✓' : index + 1 }}</i><span>{{ label }}</span>
          </li>
        </ol>
        <div class="mobile-step-label" aria-live="polite">第 {{ step }}/{{ stepLabels.length }} 步 · {{ stepLabels[step - 1] }}</div>

        <div v-if="step === 1 && !animalDataMode" class="import-selection">
          <div>
            <span>导入类型</span>
            <n-radio-group v-model:value="importKind" size="small">
              <n-radio-button v-if="animalImportAllowed" value="animal">动物登记</n-radio-button>
              <n-radio-button value="measurement">实验测量</n-radio-button>
            </n-radio-group>
          </div>
          <div v-if="importKind === 'measurement'">
            <span>所属实验</span>
            <n-select v-model:value="selectedExperimentId" :options="experimentOptions" filterable placeholder="选择已配置测量模板的实验" />
            <small v-if="!experimentOptions.length">尚无可选实验，请先在实验管理中创建实验并关联已发布模板。</small>
          </div>
        </div>
        <section v-if="step === 1 && importKind === 'animal'" class="import-resources" aria-labelledby="import-resources-title">
          <div>
            <BookOpen :size="18" />
            <span>
              <strong id="import-resources-title">导入指南与合成示例</strong>
              <small>查看字段、合法值和风险提示，或下载 4 行示例后调整。</small>
            </span>
          </div>
          <div class="resource-actions">
            <n-button size="small" secondary @click="router.push({ name: 'animal-import-guide' })">查看指南</n-button>
            <n-button size="small" secondary :loading="busy" @click="downloadAnimalTemplate('csv', 'example')">CSV 示例</n-button>
            <n-button
              size="small"
              secondary
              :loading="busy"
              :disabled="!supportsXlsxTemplates"
              @click="downloadAnimalTemplate('xlsx', 'example')"
            >XLSX 示例</n-button>
          </div>
          <small v-if="!supportsXlsxTemplates" class="resource-note">当前运行环境仅提供 CSV；指南页会说明 XLSX 降级范围。</small>
        </section>
        <div v-if="step === 1" class="drop-zone" :class="{ disabled: !canChooseFile }" @click="canChooseFile && input?.click()">
          <input ref="input" type="file" accept=".xlsx,.csv" hidden @change="selectFile" />
          <FileSpreadsheet :size="31" /><strong>选择一个表格文件</strong>
          <span>{{ importKind === 'measurement' && !selectedExperimentId ? '请先选择所属实验' : '文件先解析、校验并形成预览，不会直接写入数据库' }}</span>
          <n-button type="primary" secondary :disabled="!canChooseFile">浏览文件</n-button>
        </div>
        <div v-else-if="step === 2" class="processing">
          <n-spin size="small" /><strong>正在识别 {{ selectedFile?.name }}</strong>
          <span>流式接收后检查表头、字段类型、关系和冲突…</span>
        </div>
        <div v-else-if="step === 3 && preview" class="mapping-preview">
          <div class="file-summary">
            <FileSpreadsheet :size="19" />
            <div><strong>{{ preview.fileName }}</strong><span>{{ preview.importKind === 'measurement' ? '实验测量' : '动物登记' }} · {{ preview.sheetName }} · {{ preview.totalRows }} 行 · 可导入 {{ preview.acceptedRows }} 行 · {{ selectedFile ? formatBytes(selectedFile.size) : '' }}</span></div>
          </div>
          <div class="mapping-table">
            <div class="mapping-head"><span>源字段</span><span>MuriArc 字段</span><span>状态</span></div>
            <div v-for="row in mappingRows" :key="row.key">
              <code>{{ row.source }}</code>
              <n-select
                :value="row.target"
                :options="targetOptions"
                clearable
                size="small"
                placeholder="未映射"
                @update:value="(value: string | null) => updateMapping(row.source, value)"
              />
              <n-tag :type="row.state === '已匹配' ? 'success' : row.state === '待校验' ? 'warning' : 'default'" size="small" :bordered="false">{{ row.state }}</n-tag>
            </div>
          </div>
          <div v-if="preview.previewRows.length" class="row-preview">
            <strong>数据预览（最多 20 行）</strong>
            <div class="row-preview-scroll">
              <table>
                <thead><tr><th v-for="field in animalImportSchema?.fields ?? []" :key="field.key">{{ field.key }}</th></tr></thead>
                <tbody><tr v-for="(row, index) in preview.previewRows" :key="index"><td v-for="field in animalImportSchema?.fields ?? []" :key="field.key">{{ row[field.key] || '—' }}</td></tr></tbody>
              </table>
            </div>
          </div>
          <div v-if="mappingDirty" class="validation-note mapping-changed">
            <strong>字段映射已修改</strong>
            <span>旧预览已锁定，必须由后端重新解析同一文件后才能确认写入。</span>
          </div>
          <div v-else-if="preview.issues.length" class="validation-note" :class="{ blocking: blockingCount > 0 }">
            <strong>{{ blockingCount ? `${blockingCount} 个阻断错误` : `${warningCount} 个提示` }}</strong>
            <span v-for="issue in preview.issues.slice(0, 4)" :key="`${issue.row}-${issue.code}`">{{ issue.row ? `第 ${issue.row} 行：` : '' }}{{ issue.message }}</span>
            <span v-if="preview.issues.length > 4">另有 {{ preview.issues.length - 4 }} 项，请下载完整报告。</span>
          </div>
          <div v-else class="validation-ok"><CheckCircle2 :size="17" /><span>校验通过，可以事务写入。</span></div>
          <div class="import-actions">
            <n-button :disabled="!preview.issues.length || mappingDirty" @click="downloadIssueReport">下载校验报告</n-button>
            <n-button secondary :loading="busy" :disabled="!mappingDirty" @click="revalidateMapping">按此映射重新校验</n-button>
            <n-button type="primary" :loading="busy" :disabled="!preview.canConfirm || mappingDirty" @click="confirmImport">确认预览并事务写入</n-button>
          </div>
        </div>
        <div v-else class="created">
          <CheckCircle2 :size="38" /><strong>导入已完成</strong>
          <span v-if="receipt">动物 {{ receipt.counts.animals }} · 事件 {{ receipt.counts.animalEvents }} · 测量 {{ receipt.counts.measurements }}{{ receipt.replayed ? ' · 幂等重放' : '' }}</span>
          <n-button type="primary" secondary @click="resetImport(false)">继续导入</n-button>
        </div>
      </div>

      <aside class="principles surface">
        <h3>支持范围与数据安全</h3>
        <ul>
          <li><CheckCircle2 :size="15" />解析和校验先于写入</li>
          <li><CheckCircle2 :size="15" />确认时校验预览哈希</li>
          <li><XCircle :size="15" />冲突不覆盖、不自动合并</li>
          <li><CheckCircle2 :size="15" />任务、操作者与来源留痕</li>
          <li><CheckCircle2 :size="15" />普通导入：{{ animalDataMode ? '动物登记' : '动物登记、实验测量' }}</li>
          <li><CheckCircle2 :size="15" />业务导出固定不包含 animal UUID</li>
          <li v-if="!animalDataMode"><CheckCircle2 :size="15" />快照：完整业务归档；当前不可 restore/apply</li>
        </ul>
        <n-button v-if="snapshotAllowed && !animalDataMode" block secondary :loading="busy" @click="createSnapshot">
          <template #icon><Archive :size="16" /></template>创建完整归档快照
        </n-button>
      </aside>
    </section>

    <n-modal v-model:show="showExport" preset="card" title="配置动物业务导出" class="export-dialog" :bordered="false">
      <n-alert type="info" :show-icon="false">普通业务导出固定不包含 animal UUID；编号 scope、项目名称和显示编号作为可理解的复合身份固定输出。</n-alert>
      <n-form label-placement="top">
        <div class="export-grid">
          <n-form-item label="格式"><n-radio-group v-model:value="exportFormat"><n-radio-button value="xlsx">XLSX（多 sheet）</n-radio-button><n-radio-button value="csv">CSV（动物摘要）</n-radio-button></n-radio-group></n-form-item>
          <n-form-item label="性别"><n-select v-model:value="exportOptions.filter.sexes" multiple clearable :options="[{label:'雄',value:'male'},{label:'雌',value:'female'},{label:'未知',value:'unknown'}]" placeholder="全部" /></n-form-item>
          <n-form-item label="当前状态"><n-select v-model:value="exportOptions.filter.statuses" multiple clearable :options="exportStatusOptions" placeholder="全部" /></n-form-item>
          <n-form-item label="检测状态"><n-select v-model:value="exportOptions.filter.genotyping_states" multiple clearable :options="exportGenotypingStateOptions" placeholder="全部" /></n-form-item>
          <n-form-item label="基因型定义"><n-select v-model:value="exportOptions.filter.genotype_definitions" multiple clearable filterable :options="genotypeDefinitions" placeholder="全部" /></n-form-item>
          <n-form-item label="出生日期"><n-date-picker v-model:value="birthDateRange" type="daterange" clearable /></n-form-item>
          <n-form-item label="登记时间"><n-date-picker v-model:value="registeredAtRange" type="datetimerange" clearable /></n-form-item>
          <n-form-item label="鉴定时间"><n-date-picker v-model:value="assessedAtRange" type="datetimerange" clearable /></n-form-item>
        </div>
        <div class="tag-filter-grid">
          <n-form-item label="品系"><n-dynamic-tags v-model:value="exportOptions.filter.strains" /></n-form-item>
          <n-form-item label="区域/location"><n-dynamic-tags v-model:value="exportOptions.filter.cage_locations" /></n-form-item>
          <n-form-item label="section"><n-dynamic-tags v-model:value="exportOptions.filter.cage_sections" /></n-form-item>
          <n-form-item label="笼位编号"><n-dynamic-tags v-model:value="exportOptions.filter.cage_display_ids" /></n-form-item>
          <n-form-item label="位点"><n-dynamic-tags v-model:value="exportOptions.filter.gene_loci" /></n-form-item>
          <n-form-item label="allele"><n-dynamic-tags v-model:value="exportOptions.filter.alleles" /></n-form-item>
        </div>
        <n-form-item label="animals sheet 字段">
          <n-checkbox-group v-model:value="exportOptions.fields" class="field-grid">
            <n-checkbox v-for="field in exportFieldOptions" :key="field.value" :value="field.value" :disabled="field.disabled">{{ field.label }}</n-checkbox>
          </n-checkbox-group>
        </n-form-item>
        <n-checkbox v-model:checked="exportOptions.include_genotype_details" :disabled="exportFormat !== 'xlsx'">XLSX 包含 genotypes 明细 sheet（每个当前记录组件一行）</n-checkbox>
      </n-form>
      <template #footer><div class="export-actions"><n-button @click="showExport = false">取消</n-button><n-button type="primary" :loading="busy" @click="createExport">生成并下载</n-button></div></template>
    </n-modal>

    <section class="jobs-section">
      <header><div><h2>最近任务</h2><span>导入、导出与快照</span></div><n-button quaternary circle :loading="loading" @click="refreshJobs"><template #icon><RefreshCw :size="16" /></template></n-button></header>
      <n-spin :show="loading">
        <div class="job-list surface">
          <article v-for="job in jobs" :key="job.id">
            <div class="job-icon"><UploadCloud v-if="job.kind === 'import'" :size="18" /><Download v-else-if="job.kind === 'export'" :size="18" /><Archive v-else :size="18" /></div>
            <div><strong>{{ job.name }}</strong><span>{{ job.detail }} · {{ job.createdAt }}</span></div>
            <n-tag :type="jobStatusMeta[job.status].type" size="small" round :bordered="false">{{ jobStatusMeta[job.status].label }}</n-tag>
          </article>
          <div v-if="!jobs.length && !loading" class="empty-jobs">尚无数据任务</div>
        </div>
      </n-spin>
    </section>
  </div>
</template>

<style scoped>
.data-layout { display: grid; grid-template-columns: minmax(0, 1fr) 280px; align-items: start; gap: 13px; }
.data-layout.compact { grid-template-columns: minmax(0, 560px); }
.import-panel { overflow: hidden; }.import-panel > header { display: flex; align-items: center; justify-content: space-between; padding: 14px 16px; border-bottom: 1px solid var(--muri-border); }.import-panel > header > div { display: flex; align-items: center; gap: 10px; }.import-panel > header svg { color: var(--muri-primary); }.import-panel > header div div { display: flex; flex-direction: column; }.import-panel > header span { color: var(--muri-text-tertiary); font-size: 11px; }.import-panel > header button { border: 0; color: var(--muri-primary); background: transparent; cursor: pointer; font-size: 12px; }.import-panel > header button:disabled { opacity: .5; }
.steps { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); margin: 0; padding: 13px 18px 11px; list-style: none; border-bottom: 1px solid var(--muri-border); background: var(--muri-surface-muted); }
.steps li { position: relative; display: grid; min-width: 0; justify-items: center; gap: 5px; color: var(--muri-text-tertiary); font-size: 11px; line-height: 1.25; text-align: center; }
.steps li:not(:last-child)::after { position: absolute; z-index: 0; top: 11px; right: calc(-50% + 16px); left: calc(50% + 16px); height: 1px; background: var(--muri-border-strong); content: ''; }
.steps i { z-index: 1; display: grid; width: 22px; height: 22px; place-items: center; border: 1px solid var(--muri-border-strong); border-radius: 50%; background: white; font-style: normal; }
.steps span { z-index: 1; display: block; max-width: 100%; overflow-wrap: anywhere; }
.steps .active { color: var(--muri-primary); font-weight: 600; }
.steps .active i,.steps .done i { border-color: var(--muri-primary); color: white; background: var(--muri-primary); }
.mobile-step-label { display: none; padding: 9px 18px; border-bottom: 1px solid var(--muri-border); color: var(--muri-primary); background: var(--muri-surface-muted); font-size: 12px; font-weight: 600; text-align: center; }
.import-selection { display: grid; grid-template-columns: minmax(220px, .7fr) minmax(280px, 1.3fr); gap: 13px; padding: 15px 18px 0; }.import-selection > div { display: flex; min-width: 0; flex-direction: column; gap: 6px; }.import-selection span { color: var(--muri-text-secondary); font-size: 11px; font-weight: 600; }.import-selection small { color: var(--muri-warning); line-height: 1.45; }
.import-resources { display: flex; min-width: 0; align-items: center; justify-content: space-between; flex-wrap: wrap; gap: 8px 12px; margin: 13px 18px 0; padding: 11px 12px; border: 1px solid var(--muri-border); border-radius: 8px; background: var(--muri-surface-muted); }
.import-resources > div:first-child { display: flex; min-width: 0; flex: 1; align-items: flex-start; gap: 8px; }
.import-resources > div:first-child svg { flex: none; margin-top: 1px; color: var(--muri-primary); }
.import-resources > div:first-child span { display: flex; min-width: 0; flex-direction: column; }
.import-resources > div:first-child small { margin-top: 2px; color: var(--muri-text-tertiary); font-size: 11px; line-height: 1.4; }
.resource-actions { display: flex; flex: none; flex-wrap: wrap; justify-content: flex-end; gap: 7px; }
.resource-note { flex-basis: 100%; color: var(--muri-text-tertiary); font-size: 10px; text-align: right; }
.drop-zone { display: flex; min-height: 250px; align-items: center; justify-content: center; flex-direction: column; gap: 8px; margin: 15px 18px 18px; border: 1px dashed #aab8c5; border-radius: 9px; color: var(--muri-text-secondary); background: #fbfcfd; cursor: pointer; }.drop-zone.disabled { opacity: .62; cursor: not-allowed; }.drop-zone svg { color: var(--muri-primary); }.drop-zone span { margin-bottom: 7px; color: var(--muri-text-tertiary); font-size: 12px; }.processing,.created { display: flex; min-height: 310px; align-items: center; justify-content: center; flex-direction: column; gap: 8px; }.processing span,.created span { color: var(--muri-text-secondary); }.created svg { color: var(--muri-success); }.created button { margin-top: 5px; }
.mapping-preview { padding: 18px; }.file-summary { display: flex; align-items: center; gap: 9px; margin-bottom: 13px; }.file-summary svg { color: var(--muri-primary); }.file-summary div { display: flex; flex-direction: column; }.file-summary span { color: var(--muri-text-tertiary); font-size: 11px; }.mapping-table { border: 1px solid var(--muri-border); border-radius: 7px; overflow: hidden; }.mapping-table > div { display: grid; grid-template-columns: 1fr 1fr 90px; align-items: center; min-height: 38px; padding: 0 11px; border-bottom: 1px solid var(--muri-border); }.mapping-table > div:last-child { border-bottom: 0; }.mapping-head { color: var(--muri-text-tertiary); background: var(--muri-surface-muted); font-size: 11px; }.mapping-table code { min-width: 0; overflow: hidden; color: var(--muri-primary); text-overflow: ellipsis; white-space: nowrap; }.mapping-changed { border-color: var(--muri-warning); }.validation-note,.validation-ok { display: flex; padding: 10px 12px; gap: 2px; margin-top: 12px; border-left: 3px solid var(--muri-warning); background: #fff9ee; flex-direction: column; }.validation-note.blocking { border-color: var(--muri-danger, #d95656); background: #fff6f6; }.validation-note strong { color: #7b531b; font-size: 12px; }.validation-note.blocking strong { color: #9b3333; }.validation-note span { color: #8a6a3e; font-size: 11px; }.validation-ok { align-items: center; border-color: var(--muri-success); color: var(--muri-success); background: #f2fbf7; flex-direction: row; font-size: 12px; }.import-actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 13px; }
.row-preview { margin-top: 13px; }.row-preview > strong { display: block; margin-bottom: 7px; font-size: 12px; }.row-preview-scroll { overflow: auto; border: 1px solid var(--muri-border); border-radius: 7px; }.row-preview table { width: 100%; min-width: 800px; border-collapse: collapse; font-size: 11px; }.row-preview th, .row-preview td { max-width: 240px; padding: 7px 9px; overflow: hidden; border-right: 1px solid var(--muri-border); border-bottom: 1px solid var(--muri-border); text-align: left; text-overflow: ellipsis; white-space: nowrap; }.row-preview th { color: var(--muri-text-tertiary); background: var(--muri-surface-muted); }.row-preview tr:last-child td { border-bottom: 0; }
.principles { padding: 16px; }.principles h3 { margin: 0 0 12px; font-size: 14px; }.principles ul { display: flex; padding: 0; flex-direction: column; gap: 10px; margin: 0 0 18px; list-style: none; color: var(--muri-text-secondary); font-size: 12px; }.principles li { display: flex; align-items: center; gap: 7px; }.principles li svg { color: var(--muri-success); }.principles li:nth-child(3) svg { color: var(--muri-danger, #d95656); }
.jobs-section { margin-top: 22px; }.jobs-section > header { display: flex; align-items: center; justify-content: space-between; margin-bottom: 8px; }.jobs-section header div { display: flex; align-items: baseline; gap: 8px; }.jobs-section h2 { margin: 0; font-size: 17px; }.jobs-section header span { color: var(--muri-text-tertiary); font-size: 11px; }.job-list article { display: grid; grid-template-columns: 32px 1fr auto; align-items: center; gap: 10px; min-height: 58px; padding: 8px 12px; border-bottom: 1px solid var(--muri-border); }.job-list article:last-child { border-bottom: 0; }.job-icon { display: grid; width: 30px; height: 30px; place-items: center; border-radius: 7px; color: var(--muri-primary); background: var(--muri-primary-soft); }.job-list article > div:nth-child(2) { display: flex; min-width: 0; flex-direction: column; }.job-list article span { color: var(--muri-text-tertiary); font-size: 11px; }.empty-jobs { padding: 28px; color: var(--muri-text-tertiary); text-align: center; font-size: 12px; }
.export-dialog { width: min(820px, calc(100vw - 28px)); }.export-dialog .n-alert { margin-bottom: 14px; }.export-grid, .tag-filter-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 0 12px; }.field-grid { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 8px; }.export-actions { display: flex; justify-content: flex-end; gap: 8px; }
@media (max-width: 1180px) { .data-layout,.data-layout.compact { grid-template-columns: 1fr; } }
@media (max-width: 760px) { .import-resources { align-items: stretch; flex-direction: column; }.resource-actions { justify-content: flex-start; }.resource-note { text-align: left; } }
@media (max-width: 600px) { .steps { padding-bottom: 13px; }.steps span { display: none; }.steps li:not(:last-child)::after { right: calc(-50% + 16px); left: calc(50% + 16px); }.mobile-step-label { display: block; }.import-selection { grid-template-columns: 1fr; }.resource-actions { display: grid; grid-template-columns: 1fr 1fr; }.resource-actions > :first-child { grid-column: 1 / -1; }.drop-zone { min-height: 220px; text-align: center; }.mapping-table > div { grid-template-columns: 1fr 1fr 70px; padding: 0 7px; }.job-list article { grid-template-columns: 32px 1fr auto; }.export-grid, .tag-filter-grid, .field-grid { grid-template-columns: 1fr; } }
</style>
