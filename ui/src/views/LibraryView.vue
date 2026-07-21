<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useMessage } from 'naive-ui'
import { Camera, Download, Eye, FolderUp, RefreshCw, Trash2 } from '@lucide/vue'
import PageHeader from '@/components/PageHeader.vue'
import { gateway, type LibraryRecord } from '@/services/gateway'
import { currentProjectId } from '@/services/projectContext'

const message = useMessage()
const projects = ref<Array<{ id: string; name: string }>>([])
const selected = ref<string>()
const items = ref<LibraryRecord[]>([])
const loading = ref(false)
const deletingId = ref<string>()
const uploads = ref<Array<{ name: string; status: string }>>([])

const options = computed(() => projects.value.map((project) => ({
  label: project.name,
  value: project.id,
})))

async function load() {
  if (!gateway.listLibrary || !selected.value) return
  loading.value = true
  try {
    items.value = await gateway.listLibrary(selected.value)
  } catch (error) {
    message.error(error instanceof Error ? error.message : '无法读取资料库')
  } finally {
    loading.value = false
  }
}

async function changeProject(value: string | null) {
  selected.value = value ?? undefined
  uploads.value = []
  await load()
}

async function choose(event: Event) {
  const input = event.target as HTMLInputElement
  const files = Array.from(input.files ?? [])
  if (!selected.value || !gateway.uploadAttachment) return
  if (files.reduce((total, file) => total + file.size, 0) > 500 * 1024 * 1024) {
    message.error('单批文件总量不能超过 500 MiB')
    return
  }
  uploads.value = files.map((file) => ({ name: file.name, status: '等待' }))
  for (let index = 0; index < files.length; index += 1) {
    const file = files[index]
    uploads.value[index].status = '上传中'
    try {
      await gateway.uploadAttachment({
        entityType: 'project',
        entityId: selected.value,
        projectId: selected.value,
        fileName: file.name,
        mediaType: file.type || undefined,
        content: file,
      })
      uploads.value[index].status = '完成'
    } catch (error) {
      uploads.value[index].status = '失败'
      message.error(`${file.name}：${error instanceof Error ? error.message : '上传失败'}`)
    }
  }
  await load()
  input.value = ''
}

async function deleteEntry(entry: LibraryRecord) {
  if (!gateway.deleteAttachment) {
    message.warning('当前运行模式未提供附件删除')
    return
  }
  deletingId.value = entry.attachment.id
  try {
    await gateway.deleteAttachment({
      id: entry.attachment.id,
      expectedRevision: entry.attachment.revision,
      reason: 'project library deletion',
    })
    message.success('资料已删除')
    await load()
  } catch (error) {
    message.error(error instanceof Error ? error.message : '删除资料失败')
  } finally {
    deletingId.value = undefined
  }
}

function size(value: number) {
  return value > 1048576
    ? `${(value / 1048576).toFixed(1)} MiB`
    : `${(value / 1024).toFixed(1)} KiB`
}

function open(href?: string) {
  if (href) window.open(href, '_blank', 'noopener,noreferrer')
}

onMounted(async () => {
  projects.value = await gateway.listProjects()
  selected.value = currentProjectId.value ?? projects.value[0]?.id
  await load()
})
</script>

<template>
  <div class="page">
    <PageHeader title="项目资料库" description="原件只存一份；实验资料页是项目资料的范围视图，可关联动物、工作表、采集节点和数据单元格。">
      <template #actions>
        <n-button :loading="loading" @click="load">
          <template #icon><RefreshCw :size="16" /></template>
          刷新
        </n-button>
      </template>
    </PageHeader>

    <section class="toolbar surface">
      <n-select
        v-model:value="selected"
        :options="options"
        filterable
        placeholder="选择项目"
        @update:value="changeProject"
      />
      <label class="upload">
        <FolderUp :size="17" />
        选择文件
        <input
          type="file"
          multiple
          accept="image/*,.pdf,.tif,.tiff,.heic,.heif,.csv,.xlsx,.h5ad,.fastq,.fq,.bam"
          @change="choose"
        >
      </label>
      <label class="upload mobile-only">
        <Camera :size="17" />
        相机/相册
        <input type="file" multiple accept="image/*" capture="environment" @change="choose">
      </label>
      <span>单文件 100 MiB · 单批 500 MiB</span>
    </section>

    <section v-if="uploads.length" class="upload-status surface">
      <div v-for="upload in uploads" :key="upload.name">
        <span>{{ upload.name }}</span>
        <n-tag
          size="small"
          :type="upload.status === '失败' ? 'error' : upload.status === '完成' ? 'success' : 'info'"
        >
          {{ upload.status }}
        </n-tag>
      </div>
    </section>

    <section class="grid">
      <article v-for="entry in items" :key="entry.attachment.id" class="surface">
        <div class="thumb" @click="open(entry.attachment.previewHref)">
          <img
            v-if="entry.attachment.previewSupported && entry.attachment.mediaType?.startsWith('image/')"
            :src="entry.attachment.previewHref"
            :alt="entry.attachment.fileName"
          >
          <Eye v-else-if="entry.attachment.previewSupported" :size="28" />
          <span v-else>不可在线预览</span>
        </div>
        <div class="meta">
          <strong>{{ entry.attachment.fileName }}</strong>
          <span>{{ size(entry.attachment.sizeBytes) }} · v{{ entry.attachment.version }}</span>
          <small>{{ entry.attachment.previewReason ?? ('已关联 ' + entry.links.length + ' 个对象') }}</small>
        </div>
        <div class="actions">
          <n-button v-if="entry.attachment.previewSupported" size="small" @click="open(entry.attachment.previewHref)">
            <template #icon><Eye :size="14" /></template>
            预览
          </n-button>
          <n-button size="small" @click="open(entry.attachment.contentHref)">
            <template #icon><Download :size="14" /></template>
            下载
          </n-button>
          <n-popconfirm
            positive-text="删除"
            negative-text="取消"
            :disabled="entry.links.length > 0 || !gateway.deleteAttachment"
            @positive-click="deleteEntry(entry)"
          >
            <template #trigger>
              <n-button
                size="small"
                type="error"
                secondary
                :loading="deletingId === entry.attachment.id"
                :disabled="entry.links.length > 0 || !gateway.deleteAttachment"
              >
                <template #icon><Trash2 :size="14" /></template>
                删除
              </n-button>
            </template>
            删除后资料将从项目资料库隐藏，并保留审计记录。
          </n-popconfirm>
        </div>
      </article>
      <n-empty v-if="!loading && !items.length" description="项目收件箱为空" />
    </section>
  </div>
</template>

<style scoped>
.toolbar { display: grid; grid-template-columns: minmax(220px, 320px) auto auto 1fr; gap: 10px; align-items: center; padding: 12px; margin-bottom: 10px; }
.toolbar > span { color: var(--muri-text-tertiary); font-size: 11px; }
.upload { display: flex; min-height: 36px; align-items: center; gap: 7px; padding: 0 13px; border-radius: 7px; color: #fff; background: var(--muri-primary); cursor: pointer; }
.upload input { display: none; }
.upload-status { padding: 10px; margin-bottom: 10px; }
.upload-status div { display: flex; justify-content: space-between; padding: 4px; }
.grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(260px, 1fr)); gap: 10px; }
.grid article { overflow: hidden; }
.thumb { display: grid; height: 150px; place-items: center; background: #eef3f7; color: var(--muri-text-tertiary); cursor: pointer; }
.thumb img { width: 100%; height: 100%; object-fit: cover; }
.meta { display: flex; padding: 12px; flex-direction: column; }
.meta span,.meta small { color: var(--muri-text-tertiary); font-size: 11px; }
.actions { display: flex; flex-wrap: wrap; gap: 7px; padding: 0 12px 12px; }
@media (max-width: 700px) { .toolbar { grid-template-columns: 1fr; }.grid { grid-template-columns: 1fr; }.thumb { height: 190px; } }
</style>
