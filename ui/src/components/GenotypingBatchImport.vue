<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue'
import { CheckCircle2, Download, FileImage, FileSpreadsheet, Link2, RefreshCw, Trash2, UploadCloud } from '@lucide/vue'
import { useMessage } from 'naive-ui'
import type {
  GenotypeDefinition,
  GenotypingBatch,
  GenotypingBatchReceipt,
  GenotypingImportPreview,
  GenotypingState,
} from '@/domain/models'
import {
  gateway,
  type AttachmentMetadata,
} from '@/services/gateway'

const props = defineProps<{ projectId?: string }>()
const emit = defineEmits<{ committed: [receipt: GenotypingBatchReceipt] }>()
const message = useMessage()

const definitions = ref<GenotypeDefinition[]>([])
const recentBatches = ref<GenotypingBatch[]>([])
const batch = ref<GenotypingBatch>()
const resultFile = ref<File>()
const gelFiles = ref<File[]>([])
const resultAttachment = ref<AttachmentMetadata>()
const attachments = ref<AttachmentMetadata[]>([])
const preview = ref<GenotypingImportPreview>()
const receipt = ref<GenotypingBatchReceipt>()
const busy = ref(false)
const loadingRecent = ref(false)
const resultInput = ref<HTMLInputElement>()
const gelInput = ref<HTMLInputElement>()
const imageUrls = reactive(new Map<string, string>())
const pendingImageUrls = reactive(new Map<string, string>())

const form = reactive({
  batchNumber: createBatchNumber(),
  genotypeDefinitionId: null as string | null,
  assessedAt: Date.now(),
  method: 'PCR + 凝胶电泳',
  notes: '',
})

const supported = computed(() => Boolean(
  gateway.createGenotypingBatch
  && gateway.previewGenotypingBatch
  && gateway.commitGenotypingBatch
  && gateway.listGenotypingBatches
  && gateway.listAttachments
  && gateway.uploadAttachment
  && gateway.deleteAttachment,
))
const definitionOptions = computed(() => definitions.value.map((definition) => ({
  label: definition.name,
  value: definition.id,
})))
const gelAttachments = computed(() => attachments.value.filter((item) => item.mediaType?.startsWith('image/')))
const tableAttachments = computed(() => attachments.value.filter((item) => !item.mediaType?.startsWith('image/')))
const previewErrors = computed(() => preview.value?.issues.filter((issue) => issue.severity === 'error') ?? [])
const previewWarnings = computed(() => preview.value?.issues.filter((issue) => issue.severity === 'warning') ?? [])
const canConfirm = computed(() => Boolean(
  batch.value?.status === 'draft'
  && preview.value
  && preview.value.acceptedRows.length
  && !previewErrors.value.length
  && gelAttachments.value.length,
))
const statusMeta = {
  draft: { label: '草稿', type: 'warning' as const },
  committed: { label: '已提交', type: 'success' as const },
  cancelled: { label: '已取消', type: 'default' as const },
}
const stateLabels: Record<GenotypingState, string> = {
  unknown: '未知',
  expected: '预期',
  confirmed: '已确认',
  rejected: '已排除',
}

function createBatchNumber() {
  const now = new Date()
  const ymd = `${now.getFullYear()}${String(now.getMonth() + 1).padStart(2, '0')}${String(now.getDate()).padStart(2, '0')}`
  const hm = `${String(now.getHours()).padStart(2, '0')}${String(now.getMinutes()).padStart(2, '0')}`
  return `PCR-${ymd}-${hm}`
}

function inferredMediaType(file: File): string {
  if (file.type) return file.type
  const extension = file.name.split('.').pop()?.toLowerCase()
  const known: Record<string, string> = {
    csv: 'text/csv',
    xlsx: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
    jpg: 'image/jpeg',
    jpeg: 'image/jpeg',
    png: 'image/png',
    tif: 'image/tiff',
    tiff: 'image/tiff',
    webp: 'image/webp',
    gif: 'image/gif',
  }
  return extension ? known[extension] ?? 'application/octet-stream' : 'application/octet-stream'
}

function formatDateTime(value?: string) {
  return value ? new Date(value).toLocaleString('zh-CN', { hour12: false }) : '未记录'
}

function formatBytes(bytes: number) {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`
  return `${(bytes / 1024 / 1024).toFixed(1)} MiB`
}

function pickResult(event: Event) {
  const file = (event.target as HTMLInputElement).files?.[0]
  if (!file) return
  const extension = file.name.split('.').pop()?.toLowerCase()
  if (extension !== 'csv' && extension !== 'xlsx') {
    message.error('鉴定结果必须是 CSV 或 XLSX 文件')
    return
  }
  resultFile.value = file
  preview.value = undefined
}

function pickGels(event: Event) {
  const files = Array.from((event.target as HTMLInputElement).files ?? [])
  const images = files.filter((file) => inferredMediaType(file).startsWith('image/'))
  if (images.length !== files.length) message.warning('已忽略不是图片的文件')
  for (const file of images) {
    if (gelFiles.value.some((candidate) => candidate.name === file.name && candidate.size === file.size)) continue
    gelFiles.value.push(file)
    pendingImageUrls.set(fileKey(file), URL.createObjectURL(file))
  }
  if (gelInput.value) gelInput.value.value = ''
}

function fileKey(file: File) {
  return `${file.name}:${file.size}:${file.lastModified}`
}

function removePendingGel(file: File) {
  gelFiles.value = gelFiles.value.filter((candidate) => candidate !== file)
  const key = fileKey(file)
  const url = pendingImageUrls.get(key)
  if (url) URL.revokeObjectURL(url)
  pendingImageUrls.delete(key)
}

async function createDraftAndUpload() {
  if (!supported.value || !gateway.createGenotypingBatch || !gateway.uploadAttachment) return
  if (!form.batchNumber.trim() || !form.genotypeDefinitionId || !resultFile.value || !gelFiles.value.length) {
    message.warning('请填写批次信息，并选择一份结果表和至少一张胶图')
    return
  }
  busy.value = true
  try {
    if (!batch.value) {
      batch.value = await gateway.createGenotypingBatch({
        projectId: props.projectId,
        batchNumber: form.batchNumber.trim(),
        genotypeDefinitionId: form.genotypeDefinitionId,
        assessedAt: new Date(form.assessedAt).toISOString(),
        method: form.method.trim() || undefined,
        notes: form.notes.trim() || undefined,
      })
    }
    if (!resultAttachment.value) {
      const file = resultFile.value
      resultAttachment.value = await gateway.uploadAttachment({
        entityType: 'genotyping_batch',
        entityId: batch.value.id,
        projectId: batch.value.projectId,
        fileName: file.name,
        mediaType: inferredMediaType(file),
        content: file,
      })
    }
    for (const file of [...gelFiles.value]) {
      await gateway.uploadAttachment({
        entityType: 'genotyping_batch',
        entityId: batch.value.id,
        projectId: batch.value.projectId,
        fileName: file.name,
        mediaType: inferredMediaType(file),
        content: file,
      })
      removePendingGel(file)
    }
    await loadAttachments()
    message.success('结果表和胶图已绑定到批次草稿，请复核解析结果')
    await runPreview()
  } catch (error) {
    message.error(error instanceof Error ? error.message : '批次草稿或附件上传失败')
    if (batch.value) await loadAttachments().catch(() => undefined)
  } finally {
    busy.value = false
  }
}

async function uploadReplacementResult() {
  if (!batch.value || !resultFile.value || !gateway.uploadAttachment) return
  busy.value = true
  try {
    const file = resultFile.value
    resultAttachment.value = await gateway.uploadAttachment({
      entityType: 'genotyping_batch',
      entityId: batch.value.id,
      projectId: batch.value.projectId,
      fileName: file.name,
      mediaType: inferredMediaType(file),
      content: file,
    })
    preview.value = undefined
    await loadAttachments()
    await runPreview()
  } catch (error) {
    message.error(error instanceof Error ? error.message : '替换结果表失败')
  } finally {
    busy.value = false
  }
}

async function uploadMoreGels() {
  if (!batch.value || !gelFiles.value.length || !gateway.uploadAttachment) return
  busy.value = true
  try {
    for (const file of [...gelFiles.value]) {
      await gateway.uploadAttachment({
        entityType: 'genotyping_batch',
        entityId: batch.value.id,
        projectId: batch.value.projectId,
        fileName: file.name,
        mediaType: inferredMediaType(file),
        content: file,
      })
      removePendingGel(file)
    }
    await loadAttachments()
    message.success('胶图已继续绑定到当前批次')
  } catch (error) {
    message.error(error instanceof Error ? error.message : '胶图上传失败')
  } finally {
    busy.value = false
  }
}

async function runPreview() {
  if (!batch.value || !resultAttachment.value || !gateway.previewGenotypingBatch) return
  const result = await gateway.previewGenotypingBatch({
    batchId: batch.value.id,
    projectId: batch.value.projectId,
    expectedRevision: batch.value.revision,
    sourceAttachmentId: resultAttachment.value.id,
  })
  batch.value = result.batch
  preview.value = result.preview
  if (previewErrors.value.length) message.warning('预览存在阻断错误；正式数据尚未写入')
}

async function commitBatch() {
  if (!batch.value || !preview.value || !canConfirm.value || !gateway.commitGenotypingBatch) return
  busy.value = true
  try {
    receipt.value = await gateway.commitGenotypingBatch({
      batchId: batch.value.id,
      projectId: batch.value.projectId,
      expectedRevision: batch.value.revision,
      previewHash: preview.value.previewHash,
    })
    batch.value = receipt.value.batch
    message.success(`批次已原子提交，共写入 ${receipt.value.records.length} 条鉴定记录`)
    emit('committed', receipt.value)
    await loadRecent()
  } catch (error) {
    message.error(error instanceof Error ? error.message : '批次提交失败；未产生部分正式记录')
    await loadAttachments().catch(() => undefined)
  } finally {
    busy.value = false
  }
}

async function cancelDraft() {
  if (!batch.value || batch.value.status !== 'draft' || !gateway.cancelGenotypingBatch) {
    resetForm()
    return
  }
  busy.value = true
  try {
    batch.value = await gateway.cancelGenotypingBatch({
      batchId: batch.value.id,
      projectId: batch.value.projectId,
      expectedRevision: batch.value.revision,
      reason: '用户取消批量基因鉴定录入',
    })
    message.success('批次草稿已取消，附件与审计记录保留用于追溯')
    await loadRecent()
    resetForm()
  } catch (error) {
    message.error(error instanceof Error ? error.message : '取消批次失败')
  } finally {
    busy.value = false
  }
}

async function loadAttachments() {
  if (!batch.value || !gateway.listAttachments) return
  attachments.value = await gateway.listAttachments({
    entityType: 'genotyping_batch',
    entityId: batch.value.id,
    projectId: batch.value.projectId,
  })
  const current = attachments.value.find((item) => item.id === resultAttachment.value?.id)
  const previewSource = attachments.value.find((item) => item.id === batch.value?.sourceAttachmentId)
  const latestTable = [...tableAttachments.value].sort((left, right) =>
    right.version - left.version || right.createdAt.localeCompare(left.createdAt))[0]
  resultAttachment.value = current ?? previewSource ?? latestTable
  for (const attachment of gelAttachments.value) await hydrateImageUrl(attachment)
}

async function hydrateImageUrl(attachment: AttachmentMetadata) {
  if (imageUrls.has(attachment.id) || !gateway.downloadAttachment) return
  try {
    const blob = await gateway.downloadAttachment(attachment.id)
    imageUrls.set(attachment.id, URL.createObjectURL(blob))
  } catch {
    // Metadata and download action remain available when a thumbnail cannot be created.
  }
}

async function deleteAttachment(attachment: AttachmentMetadata) {
  if (!gateway.deleteAttachment || batch.value?.status !== 'draft') return
  busy.value = true
  try {
    await gateway.deleteAttachment({
      id: attachment.id,
      expectedRevision: attachment.revision,
      reason: '批次提交前移除或替换附件',
    })
    if (resultAttachment.value?.id === attachment.id) {
      resultAttachment.value = undefined
      preview.value = undefined
    }
    const url = imageUrls.get(attachment.id)
    if (url) URL.revokeObjectURL(url)
    imageUrls.delete(attachment.id)
    await loadAttachments()
  } catch (error) {
    message.error(error instanceof Error ? error.message : '移除附件失败')
  } finally {
    busy.value = false
  }
}

async function downloadStoredAttachment(attachment: AttachmentMetadata) {
  if (!gateway.downloadAttachment) return
  try {
    downloadBlob(await gateway.downloadAttachment(attachment.id), attachment.fileName)
  } catch (error) {
    message.error(error instanceof Error ? error.message : '附件下载失败')
  }
}

async function downloadTemplate() {
  if (!gateway.downloadGenotypingBatchTemplate) return
  try {
    downloadBlob(await gateway.downloadGenotypingBatchTemplate(), 'muriarc-genotyping-batch-template.csv')
  } catch (error) {
    message.error(error instanceof Error ? error.message : '模板下载失败')
  }
}

function downloadBlob(blob: Blob, fileName: string) {
  const url = URL.createObjectURL(blob)
  const link = document.createElement('a')
  link.href = url
  link.download = fileName
  link.click()
  URL.revokeObjectURL(url)
}

async function loadRecent() {
  if (!gateway.listGenotypingBatches) return
  loadingRecent.value = true
  try {
    recentBatches.value = await gateway.listGenotypingBatches(props.projectId)
  } catch (error) {
    message.error(error instanceof Error ? error.message : '最近批次读取失败')
  } finally {
    loadingRecent.value = false
  }
}

async function resumeDraft(item: GenotypingBatch) {
  if (batch.value || item.status !== 'draft') return
  busy.value = true
  try {
    batch.value = gateway.getGenotypingBatch
      ? (await gateway.getGenotypingBatch(item.id, item.projectId)).batch
      : item
    resultFile.value = undefined
    resultAttachment.value = undefined
    preview.value = undefined
    receipt.value = undefined
    await loadAttachments()
    if (resultAttachment.value) await runPreview()
    message.success(resultAttachment.value
      ? '已恢复批次草稿并按当前结果表重新校验'
      : '已恢复批次草稿，请补充结果表和胶图')
  } catch (error) {
    resetForm()
    message.error(error instanceof Error ? error.message : '恢复批次草稿失败')
  } finally {
    busy.value = false
  }
}

function resetForm() {
  batch.value = undefined
  resultFile.value = undefined
  resultAttachment.value = undefined
  attachments.value = []
  preview.value = undefined
  receipt.value = undefined
  form.batchNumber = createBatchNumber()
  form.genotypeDefinitionId = null
  form.assessedAt = Date.now()
  form.method = 'PCR + 凝胶电泳'
  form.notes = ''
  for (const url of imageUrls.values()) URL.revokeObjectURL(url)
  imageUrls.clear()
  for (const file of [...gelFiles.value]) removePendingGel(file)
  if (resultInput.value) resultInput.value.value = ''
}

onMounted(async () => {
  if (!supported.value) return
  await Promise.all([
    gateway.listGenotypeDefinitions(props.projectId).then((items) => { definitions.value = items.filter((item) => !item.archivedAt) }),
    loadRecent(),
  ])
})

watch(() => props.projectId, async () => {
  if (!supported.value || batch.value) return
  definitions.value = (await gateway.listGenotypeDefinitions(props.projectId)).filter((item) => !item.archivedAt)
  await loadRecent()
})

onBeforeUnmount(() => {
  for (const url of imageUrls.values()) URL.revokeObjectURL(url)
  for (const url of pendingImageUrls.values()) URL.revokeObjectURL(url)
})
</script>

<template>
  <section class="batch-import surface">
    <header class="batch-header">
      <div>
        <Link2 :size="20" />
        <span><strong>批量基因鉴定与胶图归档</strong><small>结果表、胶图、操作者和动物事件使用同一个批次关系绑定</small></span>
      </div>
      <n-button size="small" secondary :disabled="busy || !!batch" @click="downloadTemplate">
        <template #icon><Download :size="15" /></template>CSV 模板
      </n-button>
    </header>

    <n-alert v-if="!supported" type="warning" :show-icon="false">当前运行环境尚未提供批量基因鉴定接口。</n-alert>

    <template v-else>
      <div v-if="!batch" class="setup-grid">
        <n-form label-placement="top">
          <div class="form-grid">
            <n-form-item label="鉴定批次编号" required><n-input v-model:value="form.batchNumber" placeholder="例如 PCR-20260725-01" /></n-form-item>
            <n-form-item label="基因型定义" required><n-select v-model:value="form.genotypeDefinitionId" filterable :options="definitionOptions" placeholder="本批次统一使用的定义" /></n-form-item>
          </div>
          <div class="form-grid">
            <n-form-item label="鉴定时间" required><n-date-picker v-model:value="form.assessedAt" type="datetime" /></n-form-item>
            <n-form-item label="鉴定方法"><n-input v-model:value="form.method" /></n-form-item>
          </div>
          <n-form-item label="批次备注"><n-input v-model:value="form.notes" type="textarea" :rows="2" placeholder="PCR 条件、胶号、复核说明等" /></n-form-item>
        </n-form>

        <div class="evidence-grid">
          <button type="button" class="file-picker" @click="resultInput?.click()">
            <input ref="resultInput" type="file" accept=".csv,.xlsx" hidden @change="pickResult" />
            <FileSpreadsheet :size="27" />
            <strong>鉴定结果表</strong>
            <span>{{ resultFile ? `${resultFile.name} · ${formatBytes(resultFile.size)}` : '选择一份 CSV / XLSX' }}</span>
          </button>
          <button type="button" class="file-picker" @click="gelInput?.click()">
            <input ref="gelInput" type="file" accept="image/*,.tif,.tiff" multiple hidden @change="pickGels" />
            <FileImage :size="27" />
            <strong>胶图证据（可多选）</strong>
            <span>{{ gelFiles.length ? `已选择 ${gelFiles.length} 张` : '至少一张；可继续追加' }}</span>
          </button>
        </div>
        <div v-if="gelFiles.length" class="pending-images">
          <article v-for="file in gelFiles" :key="fileKey(file)">
            <img :src="pendingImageUrls.get(fileKey(file))" :alt="file.name" />
            <span><strong>{{ file.name }}</strong><small>{{ formatBytes(file.size) }}</small></span>
            <n-button quaternary circle size="small" @click="removePendingGel(file)"><template #icon><Trash2 :size="15" /></template></n-button>
          </article>
        </div>
        <div class="batch-actions">
          <n-button type="primary" :loading="busy" @click="createDraftAndUpload">
            <template #icon><UploadCloud :size="16" /></template>创建草稿、上传并生成预览
          </n-button>
        </div>
      </div>

      <div v-else class="draft-workspace">
        <div class="draft-summary">
          <div><strong>{{ batch.batchNumber }}</strong><span>{{ formatDateTime(batch.assessedAt) }} · revision {{ batch.revision }}</span></div>
          <n-tag :type="statusMeta[batch.status].type" :bordered="false">{{ statusMeta[batch.status].label }}</n-tag>
        </div>

        <section class="attachment-section">
          <header><div><h4>结果表</h4><span>确认时会重新读取并核对同一个附件与预览哈希</span></div></header>
          <article v-if="resultAttachment" class="table-file">
            <FileSpreadsheet :size="20" /><span><strong>{{ resultAttachment.fileName }}</strong><small>{{ formatBytes(resultAttachment.sizeBytes) }} · SHA-256 {{ resultAttachment.sha256.slice(0, 12) }}…</small></span>
            <n-button v-if="batch.status === 'draft'" quaternary circle @click="deleteAttachment(resultAttachment)"><template #icon><Trash2 :size="16" /></template></n-button>
          </article>
          <div v-else class="replacement-row">
            <input ref="resultInput" type="file" accept=".csv,.xlsx" @change="pickResult" />
            <n-button :disabled="!resultFile" :loading="busy" @click="uploadReplacementResult">上传结果表并重新预览</n-button>
          </div>
          <details v-if="tableAttachments.length > 1"><summary>查看其他历史结果附件（{{ tableAttachments.length - 1 }}）</summary><ul><li v-for="item in tableAttachments.filter((candidate) => candidate.id !== resultAttachment?.id)" :key="item.id">{{ item.fileName }}</li></ul></details>
        </section>

        <section class="attachment-section">
          <header><div><h4>胶图证据</h4><span>提交时至少保留一张 image/*；提交后附件锁定</span></div><n-tag size="small" :type="gelAttachments.length ? 'success' : 'error'">{{ gelAttachments.length }} 张</n-tag></header>
          <div v-if="gelAttachments.length" class="gel-gallery">
            <article v-for="item in gelAttachments" :key="item.id">
              <img v-if="imageUrls.get(item.id)" :src="imageUrls.get(item.id)" :alt="item.fileName" />
              <div v-else class="image-placeholder"><FileImage :size="24" /></div>
              <span><strong>{{ item.fileName }}</strong><small>{{ formatBytes(item.sizeBytes) }}</small></span>
              <div>
                <n-button quaternary circle size="small" @click="downloadStoredAttachment(item)"><template #icon><Download :size="14" /></template></n-button>
                <n-button v-if="batch.status === 'draft'" quaternary circle size="small" @click="deleteAttachment(item)"><template #icon><Trash2 :size="14" /></template></n-button>
              </div>
            </article>
          </div>
          <div v-if="batch.status === 'draft'" class="append-row">
            <input ref="gelInput" type="file" accept="image/*,.tif,.tiff" multiple @change="pickGels" />
            <n-button :disabled="!gelFiles.length" :loading="busy" @click="uploadMoreGels">追加 {{ gelFiles.length || '' }} 张胶图</n-button>
          </div>
        </section>

        <section v-if="preview" class="preview-section">
          <header>
            <div><h4>可复核预览</h4><span>源文件 {{ preview.totalRows }} 行 · 可接收 {{ preview.acceptedRows.length }} 行</span></div>
            <n-tag :type="previewErrors.length ? 'error' : 'success'" :bordered="false">{{ previewErrors.length ? `${previewErrors.length} 个阻断错误` : '校验通过' }}</n-tag>
          </header>
          <div v-if="preview.issues.length" class="issue-list" :class="{ blocking: previewErrors.length }">
            <span v-for="issue in preview.issues.slice(0, 8)" :key="`${issue.row}-${issue.code}`">{{ issue.row ? `第 ${issue.row} 行：` : '' }}{{ issue.message }}</span>
            <small v-if="previewWarnings.length">另有 {{ previewWarnings.length }} 个提示</small>
          </div>
          <div class="preview-table-wrap">
            <table>
              <thead><tr><th>源行</th><th>动物编号</th><th>鉴定结果</th><th>备注</th></tr></thead>
              <tbody><tr v-for="row in preview.acceptedRows.slice(0, 50)" :key="row.animalId"><td>{{ row.sourceRow }}</td><td>{{ row.displayId }}</td><td>{{ stateLabels[row.state] }}</td><td>{{ row.notes || '—' }}</td></tr></tbody>
            </table>
          </div>
          <small v-if="preview.acceptedRows.length > 50">仅显示前 50 行；确认会处理全部 {{ preview.acceptedRows.length }} 行。</small>
        </section>

        <div v-if="receipt" class="receipt">
          <CheckCircle2 :size="34" /><span><strong>批次已提交并可追溯</strong><small>{{ receipt.records.length }} 条鉴定记录已与 {{ gelAttachments.length }} 张胶图绑定；操作者、Audit、Provenance 和 AnimalEvent 已同步记录。</small></span>
        </div>

        <div class="batch-actions">
          <n-button v-if="batch.status === 'draft'" :disabled="busy" @click="cancelDraft">取消草稿</n-button>
          <n-button v-if="batch.status === 'draft'" secondary :loading="busy" :disabled="!resultAttachment" @click="runPreview"><template #icon><RefreshCw :size="15" /></template>重新校验</n-button>
          <n-button v-if="batch.status === 'draft'" type="primary" :loading="busy" :disabled="!canConfirm" @click="commitBatch">确认并原子写入</n-button>
          <n-button v-else type="primary" secondary @click="resetForm">录入下一批</n-button>
        </div>
      </div>

      <section class="recent-section">
        <header><div><h3>最近鉴定批次</h3><span>草稿、已提交与取消记录均保留</span></div><n-button quaternary circle :loading="loadingRecent" @click="loadRecent"><template #icon><RefreshCw :size="16" /></template></n-button></header>
        <div class="recent-list">
          <article v-for="item in recentBatches.slice(0, 12)" :key="item.id">
            <span><strong>{{ item.batchNumber }}</strong><small>{{ formatDateTime(item.assessedAt) }} · {{ item.previewRowCount ?? 0 }} 条</small></span>
            <div>
              <n-tag size="small" :type="statusMeta[item.status].type" :bordered="false">{{ statusMeta[item.status].label }}</n-tag>
              <n-button v-if="item.status === 'draft'" size="tiny" secondary :disabled="busy || !!batch" @click="resumeDraft(item)">继续</n-button>
            </div>
          </article>
          <n-empty v-if="!recentBatches.length && !loadingRecent" size="small" description="暂无鉴定批次" />
        </div>
      </section>
    </template>
  </section>
</template>

<style scoped>
.batch-import { overflow: hidden; }
.batch-header,.draft-summary,.attachment-section > header,.preview-section > header,.recent-section > header { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
.batch-header { padding: 15px 17px; border-bottom: 1px solid var(--muri-border); }
.batch-header > div { display: flex; align-items: center; gap: 9px; }.batch-header svg { color: var(--muri-primary); }.batch-header span,.draft-summary > div,.attachment-section header div,.preview-section header div,.recent-section header div { display: flex; flex-direction: column; }.batch-header small,.draft-summary span,.attachment-section header span,.preview-section header span,.recent-section header span { color: var(--muri-text-tertiary); font-size: 11px; }
.setup-grid,.draft-workspace { padding: 17px; }.form-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 0 12px; }
.evidence-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; }.file-picker { display: flex; min-height: 135px; align-items: center; justify-content: center; flex-direction: column; gap: 5px; border: 1px dashed var(--muri-border-strong); border-radius: 9px; color: var(--muri-text-secondary); background: var(--muri-surface-muted); cursor: pointer; }.file-picker svg { color: var(--muri-primary); }.file-picker span { max-width: 90%; overflow: hidden; color: var(--muri-text-tertiary); font-size: 11px; text-overflow: ellipsis; white-space: nowrap; }
.pending-images,.gel-gallery { display: grid; grid-template-columns: repeat(auto-fill, minmax(150px, 1fr)); gap: 9px; margin-top: 11px; }.pending-images article,.gel-gallery article { position: relative; min-width: 0; overflow: hidden; border: 1px solid var(--muri-border); border-radius: 8px; background: white; }.pending-images img,.gel-gallery img,.image-placeholder { width: 100%; height: 100px; object-fit: cover; background: var(--muri-surface-muted); }.image-placeholder { display: grid; place-items: center; color: var(--muri-text-tertiary); }.pending-images article > span,.gel-gallery article > span { display: flex; min-width: 0; padding: 7px 8px; flex-direction: column; }.pending-images strong,.gel-gallery strong { overflow: hidden; font-size: 11px; text-overflow: ellipsis; white-space: nowrap; }.pending-images small,.gel-gallery small { color: var(--muri-text-tertiary); font-size: 10px; }.pending-images article > button { position: absolute; top: 4px; right: 4px; background: rgb(255 255 255 / 88%); }.gel-gallery article > div:last-child { position: absolute; top: 4px; right: 4px; display: flex; border-radius: 6px; background: rgb(255 255 255 / 90%); }
.batch-actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 15px; }.draft-summary { padding: 11px 13px; border: 1px solid var(--muri-border); border-radius: 8px; background: var(--muri-surface-muted); }.attachment-section,.preview-section { margin-top: 13px; padding: 13px; border: 1px solid var(--muri-border); border-radius: 8px; }.attachment-section h4,.preview-section h4 { margin: 0; font-size: 13px; }.table-file { display: grid; grid-template-columns: auto 1fr auto; align-items: center; gap: 9px; margin-top: 10px; padding: 10px; border-radius: 7px; background: var(--muri-surface-muted); }.table-file > span { display: flex; min-width: 0; flex-direction: column; }.table-file small { color: var(--muri-text-tertiary); font-size: 10px; }.replacement-row,.append-row { display: flex; align-items: center; justify-content: space-between; gap: 9px; margin-top: 10px; }.replacement-row input,.append-row input { min-width: 0; font-size: 11px; }.attachment-section details { margin-top: 8px; color: var(--muri-text-tertiary); font-size: 11px; }
.issue-list { display: flex; margin-top: 10px; padding: 9px 11px; border-left: 3px solid var(--muri-warning); background: #fff9ee; flex-direction: column; color: #785b2e; font-size: 11px; }.issue-list.blocking { border-color: var(--muri-danger); background: #fff5f5; color: #8b3434; }.preview-table-wrap { overflow: auto; margin-top: 10px; border: 1px solid var(--muri-border); border-radius: 7px; }.preview-table-wrap table { width: 100%; min-width: 560px; border-collapse: collapse; font-size: 11px; }.preview-table-wrap th,.preview-table-wrap td { padding: 7px 9px; border-right: 1px solid var(--muri-border); border-bottom: 1px solid var(--muri-border); text-align: left; }.preview-table-wrap th { color: var(--muri-text-tertiary); background: var(--muri-surface-muted); }.receipt { display: flex; align-items: center; gap: 10px; margin-top: 13px; padding: 13px; border: 1px solid #a9dec7; border-radius: 8px; color: var(--muri-success); background: #f1fbf6; }.receipt span { display: flex; flex-direction: column; }.receipt small { color: var(--muri-text-secondary); }
.recent-section { margin: 0 17px 17px; padding-top: 14px; border-top: 1px solid var(--muri-border); }.recent-section h3 { margin: 0; font-size: 14px; }.recent-list { margin-top: 8px; border: 1px solid var(--muri-border); border-radius: 8px; }.recent-list article { display: flex; min-height: 52px; align-items: center; justify-content: space-between; gap: 9px; padding: 8px 11px; border-bottom: 1px solid var(--muri-border); }.recent-list article:last-child { border-bottom: 0; }.recent-list article > span { display: flex; min-width: 0; flex-direction: column; }.recent-list article > div { display: flex; align-items: center; gap: 6px; }.recent-list small { color: var(--muri-text-tertiary); font-size: 10px; }
@media (max-width: 700px) { .form-grid,.evidence-grid { grid-template-columns: 1fr; }.replacement-row,.append-row { align-items: stretch; flex-direction: column; }.replacement-row input,.append-row input { width: 100%; } }
</style>
