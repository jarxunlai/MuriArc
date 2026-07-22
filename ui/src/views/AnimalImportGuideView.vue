<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { ArrowLeft, Download, FileSpreadsheet, ShieldAlert } from '@lucide/vue'
import { useMessage } from 'naive-ui'
import { useRouter } from 'vue-router'
import PageHeader from '@/components/PageHeader.vue'
import { gateway } from '@/services/gateway'
import {
  createDataGateway,
  type AnimalImportFieldSpec,
  type AnimalImportSchema,
} from '@/services/dataGateway'

type TemplateFormat = 'csv' | 'xlsx'
type TemplateVariant = 'blank' | 'example'

const router = useRouter()
const message = useMessage()
const dataGateway = createDataGateway(gateway)
const schema = ref<AnimalImportSchema>()
const loading = ref(true)
const loadError = ref('')
const downloading = ref('')

const downloads: Array<{
  key: string
  format: TemplateFormat
  variant: TemplateVariant
  title: string
  description: string
}> = [
  {
    key: 'csv-blank',
    format: 'csv',
    variant: 'blank',
    title: '空白 CSV',
    description: '仅包含标准表头，适合从零填写。',
  },
  {
    key: 'csv-example',
    format: 'csv',
    variant: 'example',
    title: '示例 CSV',
    description: '包含 4 行合成数据，下载后替换再导入。',
  },
  {
    key: 'xlsx-blank',
    format: 'xlsx',
    variant: 'blank',
    title: '空白 XLSX',
    description: '工作簿格式，适合在表格软件中填写。',
  },
  {
    key: 'xlsx-example',
    format: 'xlsx',
    variant: 'example',
    title: '示例 XLSX',
    description: '包含 4 行合成数据，适合在表格软件中调整。',
  },
]

const supportsXlsx = computed(() => dataGateway.animalImportTemplateFormats.includes('xlsx'))
const exampleRows = computed(() => schema.value?.examples.slice(0, 4) ?? [])
const contractSourceDescription = computed(() => supportsXlsx.value
  ? '字段规范、4 行合成示例和模板均来自当前生产导入契约。'
  : '浏览器演示使用内置契约副本；正式环境模板由生产 parser 生成。')
const fieldContractDescription = computed(() => supportsXlsx.value
  ? '必填性、合法值和示例均与当前生产 parser 使用同一契约。'
  : '此处展示浏览器演示的内置副本；正式导入以生产 parser 返回的契约为准。')

const dataTypeLabels: Record<AnimalImportFieldSpec['data_type'], string> = {
  string: '文本',
  enum: '枚举',
  date: '日期',
  reference: '引用',
  canonical_genotype: '结构化基因型',
}

function legalValues(field: AnimalImportFieldSpec): string {
  return field.legal_values.length ? field.legal_values.join(' / ') : '自由文本'
}

async function downloadTemplate(format: TemplateFormat, variant: TemplateVariant) {
  if (format === 'xlsx' && !supportsXlsx.value) return
  downloading.value = `${format}-${variant}`
  try {
    await dataGateway.downloadAnimalImportTemplate(format, variant)
  } catch (error) {
    message.error(error instanceof Error ? error.message : '模板下载失败')
  } finally {
    downloading.value = ''
  }
}

onMounted(async () => {
  try {
    schema.value = await dataGateway.getAnimalImportSchema()
  } catch (error) {
    loadError.value = error instanceof Error ? error.message : '无法读取动物导入规范'
    message.error(loadError.value)
  } finally {
    loading.value = false
  }
})
</script>

<template>
  <div class="page guide-page">
    <PageHeader
      title="动物导入指南"
      section="动物管理"
      :description="contractSourceDescription"
    >
      <template #actions>
        <n-button secondary @click="router.push({ name: 'animal-data' })">
          <template #icon><ArrowLeft :size="17" /></template>
          返回动物数据
        </n-button>
      </template>
    </PageHeader>

    <n-spin :show="loading">
      <n-alert v-if="loadError" type="error" role="alert">
        {{ loadError }}
      </n-alert>

      <template v-else-if="schema">
        <section class="surface download-section" aria-labelledby="template-download-title">
          <header>
            <div>
              <FileSpreadsheet :size="20" />
              <span>
                <strong id="template-download-title">下载可编辑模板</strong>
                <small>空白模板用于正式填写；示例模板中的动物均为合成数据。</small>
              </span>
            </div>
            <n-tag size="small" :bordered="false">schema v{{ schema.version }}</n-tag>
          </header>

          <n-alert v-if="!supportsXlsx" type="info" :show-icon="false" class="format-note">
            当前运行环境仅提供 CSV 模板；XLSX 请在 Desktop 或 Server 环境中下载。
          </n-alert>

          <div class="download-grid">
            <article v-for="item in downloads" :key="item.key">
              <div>
                <strong>{{ item.title }}</strong>
                <span>{{ item.description }}</span>
              </div>
              <n-button
                secondary
                size="small"
                :data-testid="`download-${item.key}`"
                :aria-label="`下载 ${item.title}`"
                :disabled="item.format === 'xlsx' && !supportsXlsx"
                :loading="downloading === item.key"
                @click="downloadTemplate(item.format, item.variant)"
              >
                <template #icon><Download :size="15" /></template>
                下载
              </n-button>
            </article>
          </div>
        </section>

        <section class="risk-section" aria-labelledby="risk-title">
          <header>
            <ShieldAlert :size="19" />
            <div><strong id="risk-title">导入前必须核对</strong><span>示例可编辑，但不能当作真实实验室数据直接提交。</span></div>
          </header>
          <ul>
            <li><strong>合成示例：</strong>示例编号、笼位、父母和基因型仅用于解释格式，导入前必须替换或清空。</li>
            <li><strong>关系引用：</strong>未知或歧义的笼位、父母、位点、allele 和基因型定义会在预览阶段阻断。</li>
            <li><strong>编号唯一：</strong><code>display_id</code> 必须在对应编号范围内唯一，系统不会覆盖或自动合并冲突。</li>
            <li><strong>先预览再写入：</strong>选择文件只会解析和校验，必须人工确认预览后才会事务写入。</li>
          </ul>
        </section>

        <section class="surface guide-section" aria-labelledby="field-guide-title">
          <header>
            <div><strong id="field-guide-title">字段说明</strong><span>{{ fieldContractDescription }}</span></div>
          </header>
          <div class="field-grid">
            <article v-for="field in schema.fields" :key="field.key" class="field-card">
              <header>
                <code>{{ field.key }}</code>
                <n-tag size="small" :type="field.required ? 'warning' : 'default'" :bordered="false">
                  {{ field.required ? '必填' : '可选' }}
                </n-tag>
              </header>
              <dl>
                <div><dt>类型</dt><dd>{{ dataTypeLabels[field.data_type] }}</dd></div>
                <div><dt>合法值</dt><dd>{{ legalValues(field) }}</dd></div>
                <div><dt>示例</dt><dd><code>{{ field.example }}</code></dd></div>
              </dl>
              <p>{{ field.description }}</p>
            </article>
          </div>
          <n-alert type="info" :show-icon="false" class="genotype-syntax">
            基因型 canonical 语法：<code>{{ schema.genotype_syntax }}</code>
          </n-alert>
        </section>

        <section class="surface guide-section example-section" aria-labelledby="example-title">
          <header>
            <div><strong id="example-title">4 行合成示例</strong><span>用于理解列之间的关系；下载示例模板后再按实验室数据调整。</span></div>
            <n-tag size="small" type="warning" :bordered="false">非真实动物数据</n-tag>
          </header>

          <div class="example-table-scroll">
            <table aria-label="动物导入合成示例">
              <thead><tr><th v-for="field in schema.fields" :key="field.key" scope="col">{{ field.key }}</th></tr></thead>
              <tbody>
                <tr v-for="(row, index) in exampleRows" :key="index">
                  <td v-for="field in schema.fields" :key="field.key">{{ row[field.key] || '—' }}</td>
                </tr>
              </tbody>
            </table>
          </div>

          <div class="example-cards" aria-label="动物导入合成示例（移动端）">
            <article v-for="(row, index) in exampleRows" :key="index">
              <header><strong>示例 {{ index + 1 }}</strong><code>{{ row.display_id || '未填写编号' }}</code></header>
              <dl>
                <div v-for="field in schema.fields" :key="field.key">
                  <dt>{{ field.key }}</dt><dd>{{ row[field.key] || '—' }}</dd>
                </div>
              </dl>
            </article>
          </div>
        </section>
      </template>
    </n-spin>
  </div>
</template>

<style scoped>
.guide-page { min-width: 0; }
.download-section,.guide-section { min-width: 0; padding: 17px; }
.download-section > header,.guide-section > header { display: flex; min-width: 0; align-items: flex-start; justify-content: space-between; gap: 12px; margin-bottom: 14px; }
.download-section > header > div { display: flex; min-width: 0; align-items: flex-start; gap: 9px; }
.download-section > header svg { flex: none; margin-top: 1px; color: var(--muri-primary); }
.download-section > header span,.guide-section > header > div { display: flex; min-width: 0; flex-direction: column; }
.download-section > header small,.guide-section > header span { margin-top: 3px; color: var(--muri-text-tertiary); font-size: 11px; font-weight: 400; line-height: 1.45; }
.format-note { margin-bottom: 12px; }
.download-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 9px; }
.download-grid article { display: flex; min-width: 0; min-height: 74px; align-items: center; justify-content: space-between; gap: 12px; padding: 11px 12px; border: 1px solid var(--muri-border); border-radius: 7px; background: var(--muri-surface-muted); }
.download-grid article > div { display: flex; min-width: 0; flex-direction: column; }
.download-grid article span { margin-top: 3px; color: var(--muri-text-secondary); font-size: 11px; line-height: 1.4; }
.risk-section { display: grid; grid-template-columns: minmax(220px, .68fr) minmax(0, 1.32fr); gap: 14px; margin: 12px 0; padding: 15px 17px; border: 1px solid #f0d3a7; border-radius: 8px; background: #fff9ef; }
.risk-section > header { display: flex; align-items: flex-start; gap: 9px; }
.risk-section > header svg { flex: none; color: var(--muri-warning); }
.risk-section > header div { display: flex; flex-direction: column; }
.risk-section > header span { margin-top: 3px; color: #765b35; font-size: 11px; line-height: 1.45; }
.risk-section ul { display: flex; padding-left: 19px; flex-direction: column; gap: 7px; margin: 0; color: #614c2e; font-size: 12px; line-height: 1.5; }
.risk-section code { color: #754b12; }
.guide-section + .guide-section { margin-top: 12px; }
.field-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 9px; }
.field-card { min-width: 0; padding: 12px; border: 1px solid var(--muri-border); border-radius: 7px; }
.field-card > header { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
.field-card > header code { color: var(--muri-primary); font-weight: 600; overflow-wrap: anywhere; }
.field-card dl { display: flex; flex-direction: column; gap: 5px; margin: 10px 0 0; }
.field-card dl div { display: grid; grid-template-columns: 62px minmax(0, 1fr); gap: 7px; font-size: 11px; }
.field-card dt { color: var(--muri-text-tertiary); }
.field-card dd { min-width: 0; margin: 0; color: var(--muri-text-secondary); overflow-wrap: anywhere; }
.field-card dd code { color: var(--muri-primary); }
.field-card p { margin: 9px 0 0; color: var(--muri-text-secondary); font-size: 11px; line-height: 1.5; }
.genotype-syntax { margin-top: 12px; }
.genotype-syntax code { overflow-wrap: anywhere; }
.example-table-scroll { max-width: 100%; overflow-x: auto; border: 1px solid var(--muri-border); border-radius: 7px; }
.example-table-scroll table { width: 100%; min-width: 940px; border-collapse: collapse; font-size: 11px; }
.example-table-scroll th,.example-table-scroll td { max-width: 230px; padding: 8px 9px; overflow: hidden; border-right: 1px solid var(--muri-border); border-bottom: 1px solid var(--muri-border); text-align: left; text-overflow: ellipsis; white-space: nowrap; }
.example-table-scroll th { color: var(--muri-text-tertiary); background: var(--muri-surface-muted); font-weight: 600; }
.example-table-scroll tr:last-child td { border-bottom: 0; }
.example-table-scroll th:last-child,.example-table-scroll td:last-child { border-right: 0; }
.example-cards { display: none; flex-direction: column; gap: 8px; }
.example-cards > article { padding: 11px; border: 1px solid var(--muri-border); border-radius: 7px; }
.example-cards header { display: flex; align-items: center; justify-content: space-between; gap: 8px; padding-bottom: 8px; border-bottom: 1px solid var(--muri-border); }
.example-cards header code { color: var(--muri-primary); }
.example-cards dl { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 7px 12px; margin: 9px 0 0; }
.example-cards dl div { min-width: 0; }
.example-cards dt { color: var(--muri-text-tertiary); font-size: 10px; }
.example-cards dd { margin: 2px 0 0; color: var(--muri-text-secondary); font-size: 11px; overflow-wrap: anywhere; }
@media (max-width: 800px) {
  .download-grid,.field-grid { grid-template-columns: 1fr; }
  .risk-section { grid-template-columns: 1fr; }
  .example-table-scroll { display: none; }
  .example-cards { display: flex; }
}
@media (max-width: 520px) {
  .download-section,.guide-section { padding: 14px; }
  .download-section > header,.guide-section > header { flex-direction: column; }
  .download-grid article { align-items: stretch; flex-direction: column; }
  .download-grid article :deep(.n-button) { width: 100%; }
  .example-cards dl { grid-template-columns: 1fr; }
}
</style>
