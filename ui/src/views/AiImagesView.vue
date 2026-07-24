<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { ImagePlus, Images, ScanSearch, ShieldCheck } from '@lucide/vue'
import { useMessage } from 'naive-ui'
import PageHeader from '@/components/PageHeader.vue'
import {
  gateway,
  type AiExtractionRecord,
  type PrivateImageRecord,
} from '@/services/gateway'

const toast = useMessage()
const images = ref<PrivateImageRecord[]>([])
const drafts = ref<AiExtractionRecord[]>([])
const projects = ref<Array<{ id: string; name: string }>>([])
const projectId = ref<string | null>(null)
const busy = ref(false)
const errorMessage = ref('')
const fileInput = ref<HTMLInputElement | null>(null)
const localPreviewUrls = new Map<string, string>()

const projectOptions = computed(() => projects.value.map((project) => ({
  label: project.name,
  value: project.id,
})))

function releaseLocalPreviews() {
  for (const previewUrl of localPreviewUrls.values()) URL.revokeObjectURL(previewUrl)
  localPreviewUrls.clear()
}

async function previewFor(image: PrivateImageRecord) {
  if (image.previewHref) return image.previewHref
  if (!gateway.readPrivateImage) return ''
  const content = await gateway.readPrivateImage(image.image.id)
  const previewUrl = URL.createObjectURL(content)
  localPreviewUrls.set(image.image.id, previewUrl)
  return previewUrl
}

async function load() {
  if (!gateway.listPrivateImages) return
  errorMessage.value = ''
  releaseLocalPreviews()
  const [loadedImages, loadedDrafts] = await Promise.all([
    gateway.listPrivateImages(undefined, projectId.value ?? undefined),
    gateway.listAiExtractions?.(projectId.value ?? undefined) ?? [],
  ])
  images.value = await Promise.all(loadedImages.map(async (image) => ({
    ...image,
    previewHref: await previewFor(image).catch(() => ''),
  })))
  drafts.value = loadedDrafts
}

function chooseImages() {
  fileInput.value?.click()
}

async function upload(event: Event) {
  const input = event.target as HTMLInputElement
  const files = Array.from(input.files ?? [])
  input.value = ''
  if (!gateway.uploadPrivateImage || !files.length) return
  if (files.length > 8) {
    errorMessage.value = '每批最多上传 8 张图片'
    return
  }
  const allowed = new Set(['image/jpeg', 'image/png', 'image/webp', 'image/gif'])
  const invalid = files.find((file) =>
    !allowed.has(file.type.toLowerCase()) || !file.size || file.size > 100 * 1024 * 1024)
  if (invalid) {
    errorMessage.value = `${invalid.name} 必须是小于 100 MiB 的 JPEG、PNG、WebP 或 GIF`
    return
  }
  busy.value = true
  errorMessage.value = ''
  try {
    for (const file of files) await gateway.uploadPrivateImage(file)
    toast.success(`已上传 ${files.length} 张私人图片`)
    await load()
  } catch (error) {
    errorMessage.value = error instanceof Error ? error.message : '上传失败'
  } finally {
    busy.value = false
  }
}

function formatValue(value: unknown) {
  if (value === null || value === undefined) return '—'
  if (typeof value === 'string' || typeof value === 'number' || typeof value === 'boolean') {
    return String(value)
  }
  return JSON.stringify(value)
}

function statusLabel(status: string) {
  return {
    active: '可用',
    processing: '处理中',
    pending_approval: '待审批',
    archived: '已归档',
    failed: '失败',
    expired: '已过期',
    approved: '已批准',
    rejected: '已拒绝',
  }[status] ?? status
}

onMounted(async () => {
  projects.value = await gateway.listProjects()
  await load()
})
onUnmounted(releaseLocalPreviews)
</script>

<template>
  <div class="page">
    <PageHeader
      title="私人 AI 图片"
      description="图片按用户与会话隔离；只有经人工批准的数据录入才会提升为项目正式附件。"
    >
      <template #actions>
        <n-button type="primary" :loading="busy" @click="chooseImages">
          <template #icon><ImagePlus :size="17" /></template>
          上传图片
        </n-button>
      </template>
    </PageHeader>

    <input
      ref="fileInput"
      class="visually-hidden"
      type="file"
      multiple
      accept="image/jpeg,image/png,image/webp,image/gif"
      aria-label="上传私人 AI 图片"
      @change="upload"
    >

    <section class="image-boundary surface">
      <div>
        <ShieldCheck :size="17" />
        <span>
          未归档图片从最后活动起保留 30 天；处理中与待审批图片不会清理。AI 不能直接把私人图片写入项目资料。
        </span>
      </div>
      <n-select
        v-model:value="projectId"
        :options="projectOptions"
        clearable
        filterable
        placeholder="全部私人图片"
        aria-label="按科研项目筛选私人图片"
        @update:value="load"
      />
    </section>

    <p v-if="errorMessage" class="page-error" role="alert" aria-live="assertive">
      {{ errorMessage }}
    </p>

    <section class="image-grid" aria-label="私人图片列表">
      <article v-for="entry in images" :key="entry.image.id" class="surface image-card">
        <img
          v-if="entry.previewHref"
          :src="entry.previewHref"
          :alt="`私人图片：${entry.fileName}`"
        >
        <div v-else class="preview-unavailable"><Images :size="28" />无法预览</div>
        <div class="image-copy">
          <strong>{{ entry.fileName }}</strong>
          <small>{{ (entry.sizeBytes / 1048576).toFixed(1) }} MiB · SHA {{ entry.sha256.slice(0, 12) }}</small>
          <small>{{ entry.retentionDays }} 天后到期</small>
        </div>
        <n-tag size="small">{{ statusLabel(entry.image.status) }}</n-tag>
      </article>
      <n-empty v-if="!images.length" description="尚未上传私人图片" />
    </section>

    <section class="extraction-history">
      <header>
        <div><ScanSearch :size="18" /><h2>数据单元识别记录</h2></div>
        <span>{{ drafts.length }} 项</span>
      </header>
      <div class="draft-list">
        <article v-for="draft in drafts" :key="draft.id" class="surface draft-card">
          <header>
            <div>
              <strong>{{ statusLabel(draft.status) }}</strong>
              <small>
                {{ draft.evidence.length }} 张证据
                · 模型 v{{ draft.modelTrace?.profileVersion ?? '—' }}
                · revision {{ draft.revision }}
              </small>
            </div>
            <n-tag
              size="small"
              :type="draft.status === 'approved' ? 'success' : 'warning'"
            >{{ statusLabel(draft.status) }}</n-tag>
          </header>
          <div v-for="candidate in draft.candidates" :key="candidate.itemIndex" class="candidate-row">
            <span>{{ candidate.sourceLabel ?? draft.currentDataCell?.definitionId }}</span>
            <code>{{ formatValue(candidate.value.value) }}</code>
            <n-progress
              type="line"
              :percentage="Math.round(candidate.confidence * 100)"
              :show-indicator="true"
            />
          </div>
          <p v-if="draft.status === 'pending_approval'">
            请回到对应实验的“录入实验数据”窗口编辑并批准；此页面不会绕过当前数据单元确认。
          </p>
        </article>
        <n-empty v-if="!drafts.length" description="尚无图片识别候选" />
      </div>
    </section>
  </div>
</template>

<style scoped>
.visually-hidden { position: absolute; width: 1px; height: 1px; padding: 0; overflow: hidden; clip: rect(0, 0, 0, 0); white-space: nowrap; border: 0; }
.image-boundary { display: grid; grid-template-columns: minmax(0, 1fr) minmax(180px, 260px); align-items: center; gap: 14px; margin-bottom: 12px; padding: 12px; }.image-boundary > div { display: flex; align-items: flex-start; gap: 7px; color: var(--muri-text-secondary); font-size: 11px; line-height: 1.55; }.image-boundary svg { flex: 0 0 auto; margin-top: 1px; color: var(--muri-primary); }
.page-error { margin: 0 0 12px; padding: 8px 10px; border: 1px solid #efd0d0; border-radius: 7px; color: var(--muri-danger); background: #fff7f7; font-size: 11px; }
.image-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(205px, 1fr)); gap: 10px; }.image-card { position: relative; display: flex; min-width: 0; padding: 9px; flex-direction: column; gap: 7px; }.image-card > img,.preview-unavailable { width: 100%; height: 150px; border-radius: 7px; object-fit: cover; }.preview-unavailable { display: grid; place-content: center; gap: 5px; color: var(--muri-text-tertiary); background: var(--muri-surface-muted); text-align: center; font-size: 11px; }.image-copy { display: flex; min-width: 0; flex-direction: column; }.image-copy strong,.image-copy small { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }.image-copy strong { color: var(--muri-text); font-size: 12px; }.image-copy small { color: var(--muri-text-tertiary); font-size: 10px; }.image-card :deep(.n-tag) { align-self: flex-start; }
.extraction-history { margin-top: 22px; }.extraction-history > header { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin-bottom: 9px; }.extraction-history > header > div { display: flex; align-items: center; gap: 7px; }.extraction-history h2 { margin: 0; font-size: 17px; }.extraction-history svg { color: var(--muri-primary); }.extraction-history > header > span { color: var(--muri-text-tertiary); font-size: 11px; }.draft-list { display: grid; gap: 10px; }.draft-card { padding: 14px; }.draft-card > header { display: flex; align-items: flex-start; justify-content: space-between; gap: 12px; }.draft-card > header > div { display: flex; min-width: 0; flex-direction: column; }.draft-card small { color: var(--muri-text-tertiary); font-size: 10px; }.candidate-row { display: grid; grid-template-columns: minmax(120px, 1fr) minmax(100px, 1fr) minmax(150px, 220px); align-items: center; gap: 10px; padding: 9px 0; border-bottom: 1px solid var(--muri-border); }.candidate-row span { color: var(--muri-text-secondary); font-size: 11px; }.candidate-row code { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }.draft-card > p { margin: 10px 0 0; color: var(--muri-text-secondary); font-size: 11px; }
@media (max-width: 768px) { .image-boundary { grid-template-columns: minmax(0, 1fr); }.candidate-row { grid-template-columns: minmax(0, 1fr); }.image-grid { grid-template-columns: repeat(2, minmax(0, 1fr)); } }
@media (max-width: 430px) { .image-grid { grid-template-columns: minmax(0, 1fr); }.image-card > img,.preview-unavailable { height: 180px; } }
</style>
