<script setup lang="ts">
import { computed, h, onMounted, reactive, ref, watch } from 'vue'
import { Download, Filter, Plus, Search, Upload } from '@lucide/vue'
import { NButton, NTag, type DataTableColumns, useMessage } from 'naive-ui'
import { useRoute, useRouter } from 'vue-router'
import type {
  Animal,
  AnimalDetail,
  AnimalGenotype,
  AnimalMeasurement,
  AnimalStatus,
  GeneAllele,
  GeneLocus,
  PedigreeRelation,
  ProjectSummary,
} from '@/domain/models'
import { gateway } from '@/services/gateway'
import {
  canManageBreeding,
  canWriteAnimal,
  canWriteProjectData,
  currentProjectId,
  hasLabRegistryAccess,
} from '@/services/projectContext'
import PageHeader from '@/components/PageHeader.vue'

const route = useRoute()
const router = useRouter()
const message = useMessage()

const animals = ref<Animal[]>([])
const cages = ref(new Map<string, string>())
const cageOptions = ref<Array<{ label: string; value: string }>>([])
const projects = ref<ProjectSummary[]>([])
const loading = ref(true)
const busy = ref(false)
const search = ref('')
const status = ref<AnimalStatus | null>(null)
const selected = ref<Animal | null>(null)
const detail = ref<AnimalDetail | null>(null)
const detailLoading = ref(false)
const detailError = ref('')
const detailTab = ref('timeline')
const showCreate = ref(false)
const showSampleCreate = ref(false)
const sampleSaving = ref(false)
const showGenotypeCreate = ref(false)
const genotypeSaving = ref(false)
const geneticsLoading = ref(false)
const geneLoci = ref<GeneLocus[]>([])
const geneAlleles = ref<GeneAllele[]>([])
const genotypes = ref<AnimalGenotype[]>([])
const attachmentUploading = ref(false)
const attachmentDownloadingId = ref<string | null>(null)
const attachmentProjectId = ref<string | null>(null)
const fileInput = ref<HTMLInputElement | null>(null)

const newAnimal = reactive({
  displayId: '',
  identifierScope: 'lab' as 'lab' | 'project',
  projectId: null as string | null,
  cageId: null as string | null,
  sex: 'unknown' as Animal['sex'],
  strain: '',
  birthDate: null as number | null,
})
const newSample = reactive({
  projectId: null as string | null,
  experimentId: null as string | null,
  sampleType: '',
  quantity: null as number | null,
  unit: '',
  location: '',
  collectedAt: null as number | null,
})
const newGenotype = reactive({
  locusValue: '',
  allele1Value: '',
  allele2Value: '',
  allele1WildType: false,
  allele2WildType: false,
  assessedAt: null as number | null,
})

const routeProjectId = computed(() => typeof route.query.project_id === 'string'
  ? route.query.project_id
  : undefined)
const projectId = computed(() => gateway.mode === 'remote'
  ? currentProjectId.value ?? routeProjectId.value
  : routeProjectId.value)
const accessContext = computed(() => projectId.value ? { projectId: projectId.value } : undefined)
const projectOnly = computed(() => gateway.mode === 'remote' && !hasLabRegistryAccess())
const animalWriteAllowed = computed(() => gateway.mode === 'local' || canWriteAnimal())
const projectDataWriteAllowed = computed(() => gateway.mode === 'local' || canWriteProjectData())
const genotypeWriteAllowed = computed(() => gateway.mode === 'local' || canManageBreeding())

const statusMeta: Record<AnimalStatus, { label: string; type: 'default' | 'success' | 'info' | 'warning' }> = {
  active: { label: '在养', type: 'success' },
  breeding: { label: '繁育', type: 'info' },
  experiment: { label: '实验中', type: 'warning' },
  archived: { label: '已归档', type: 'default' },
}
const statusOptions = Object.entries(statusMeta).map(([value, meta]) => ({ value, label: meta.label }))
const projectOptions = computed(() => projects.value.map((project) => ({ label: project.name, value: project.id })))
const selectedProjectRefs = computed<ProjectSummary[]>(() => {
  if (!selected.value) return []
  const refs = selected.value.projectRefs?.length
    ? selected.value.projectRefs
    : projects.value.filter((project) => selected.value?.projectNames.includes(project.name))
  const unique = new Map(refs.map((project) => [project.id, project]))
  if (projectId.value) {
    const routeProject = projects.value.find((project) => project.id === projectId.value)
    if (routeProject) unique.set(routeProject.id, routeProject)
  }
  return [...unique.values()]
})
const selectedProjectOptions = computed(() => selectedProjectRefs.value.map((project) => ({
  label: project.name,
  value: project.id,
})))
const sampleExperimentOptions = computed(() => detail.value?.experiments
  .filter((record) => !newSample.projectId || record.projectId === newSample.projectId)
  .filter((record, index, records) => records.findIndex((item) => item.experimentId === record.experimentId) === index)
  .map((record) => ({ label: record.experimentName, value: record.experimentId })) ?? [])
const locusOptions = computed(() => geneLoci.value.map((locus) => ({
  label: locus.symbol,
  value: locus.id,
})))
const selectedLocus = computed(() => {
  const value = newGenotype.locusValue.trim().toLowerCase()
  return geneLoci.value.find((locus) => locus.id === newGenotype.locusValue
    || locus.symbol.toLowerCase() === value)
})
const alleleOptions = computed(() => selectedLocus.value
  ? geneAlleles.value
      .filter((allele) => allele.locusId === selectedLocus.value?.id)
      .map((allele) => ({
        label: `${allele.symbol}${allele.isWildType ? '（野生型）' : ''}`,
        value: allele.id,
      }))
  : [])
const genotypeRows = computed(() => genotypes.value.map((genotype) => {
  const locus = geneLoci.value.find((candidate) => candidate.id === genotype.locusId)
  const first = geneAlleles.value.find((candidate) => candidate.id === genotype.allele1Id)
  const second = geneAlleles.value.find((candidate) => candidate.id === genotype.allele2Id)
  return {
    ...genotype,
    locusLabel: locus?.symbol ?? '未知位点',
    alleleLabel: `${first?.symbol ?? '?'} / ${second?.symbol ?? '?'}`,
  }
}))
const filtered = computed(() => {
  const query = search.value.trim().toLowerCase()
  return animals.value.filter((animal) => (!status.value || animal.status === status.value) && (!query
    || animal.code.toLowerCase().includes(query)
    || animal.genotype.toLowerCase().includes(query)
    || animal.strain.toLowerCase().includes(query)
    || animal.projectNames.some((project) => project.toLowerCase().includes(query))))
})

function sexLabel(sex: Animal['sex']) {
  return sex === 'male' ? '雄' : sex === 'female' ? '雌' : '未知'
}

function formatDate(value: number | null) {
  if (!value) return undefined
  const date = new Date(value)
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, '0')}-${String(date.getDate()).padStart(2, '0')}`
}

function formatDateTime(value?: string) {
  if (!value) return '未记录'
  const date = new Date(value)
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString('zh-CN')
}

function formatBytes(value: number) {
  if (value < 1024) return `${value} B`
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KiB`
  if (value < 1024 * 1024 * 1024) return `${(value / 1024 / 1024).toFixed(1)} MiB`
  return `${(value / 1024 / 1024 / 1024).toFixed(1)} GiB`
}

function formatMeasurement(measurement: AnimalMeasurement) {
  const value = measurement.value
  if (value.type === 'boolean') return value.value ? '是' : '否'
  return `${String(value.value)}${measurement.unit ? ` ${measurement.unit}` : ''}`
}

function pedigreeLabel(relation: PedigreeRelation) {
  if (relation.direction === 'offspring') return '后代'
  return relation.parentType === 'father' ? '父本' : relation.parentType === 'mother' ? '母本' : '父母'
}

async function loadGenetics(animalId: string) {
  geneticsLoading.value = true
  try {
    const [loci, rows] = await Promise.all([
      gateway.listGeneLoci(projectId.value),
      gateway.listGenotypes(animalId, projectId.value),
    ])
    const alleleGroups = await Promise.all(loci.map((locus) => gateway.listAlleles(
      locus.id,
      projectId.value,
    )))
    geneLoci.value = loci
    geneAlleles.value = alleleGroups.flat()
    genotypes.value = rows
  } finally {
    geneticsLoading.value = false
  }
}

async function hydrateSelected(animal: Animal, resetTab = true) {
  selected.value = animal
  detail.value = null
  geneLoci.value = []
  geneAlleles.value = []
  genotypes.value = []
  detailError.value = ''
  if (resetTab) detailTab.value = 'timeline'
  detailLoading.value = true
  try {
    const [summary, result] = await Promise.all([
      gateway.getAnimal(animal.id, accessContext.value),
      gateway.getAnimalDetail(animal.id, accessContext.value),
      loadGenetics(animal.id),
    ])
    if (selected.value?.id !== animal.id) return
    if (summary) {
      selected.value = summary
      const index = animals.value.findIndex((candidate) => candidate.id === summary.id)
      if (index >= 0) animals.value[index] = summary
    }
    detail.value = result
    attachmentProjectId.value = projectId.value
      ?? selectedProjectRefs.value[0]?.id
      ?? null
  } catch (error) {
    if (selected.value?.id === animal.id) {
      detailError.value = error instanceof Error ? error.message : '读取小鼠详情失败'
      message.error(detailError.value)
    }
  } finally {
    if (selected.value?.id === animal.id) detailLoading.value = false
  }
}

function openAnimal(animal: Animal) {
  void hydrateSelected(animal)
  void router.replace({ query: { ...route.query, animal: animal.id } })
}

function closeAnimal() {
  selected.value = null
  detail.value = null
  detailError.value = ''
  const query = { ...route.query }
  delete query.animal
  void router.replace({ query })
}

const columns: DataTableColumns<Animal> = [
  { title: '编号', key: 'code', width: 130, render: (row) => h('button', { class: 'table-link', onClick: () => openAnimal(row) }, row.code) },
  { title: '性别', key: 'sex', width: 70, render: (row) => sexLabel(row.sex) },
  { title: '品系', key: 'strain', minWidth: 130 },
  { title: '基因型', key: 'genotype', minWidth: 130, render: (row) => h(NTag, { size: 'small', bordered: false, type: row.genotype === '待确认' ? 'warning' : 'default' }, { default: () => row.genotype }) },
  { title: '当前笼位', key: 'cageId', width: 110, render: (row) => row.cageId ? (cages.value.get(row.cageId) ?? '未知') : '未分配' },
  { title: '状态', key: 'status', width: 100, render: (row) => h(NTag, { size: 'small', bordered: false, round: true, type: statusMeta[row.status].type }, { default: () => statusMeta[row.status].label }) },
  { title: '关联项目', key: 'projectNames', minWidth: 150, render: (row) => row.projectNames.join('、') || '未关联' },
  { title: '', key: 'actions', width: 70, render: (row) => h(NButton, { size: 'small', quaternary: true, onClick: () => openAnimal(row) }, { default: () => '查看' }) },
]

async function load() {
  loading.value = true
  try {
    const [animalRows, cageRows, projectRows] = await Promise.all([
      gateway.listAnimals(accessContext.value),
      gateway.mode === 'local' || hasLabRegistryAccess()
        ? gateway.listCages()
        : Promise.resolve([]),
      gateway.listProjects(),
    ])
    animals.value = animalRows
    projects.value = projectRows
    cages.value = new Map(cageRows.map((cage) => [cage.id, cage.code]))
    cageOptions.value = cageRows.map((cage) => ({
      label: `${cage.code} · ${cage.animalIds.length}/${cage.capacity}`,
      value: cage.id,
    }))
    openFromQuery(route.query.animal)
  } catch (error) {
    message.error(error instanceof Error ? error.message : '读取动物列表失败')
  } finally {
    loading.value = false
  }
}

function openCreate() {
  if (projectOnly.value) {
    newAnimal.identifierScope = 'project'
    newAnimal.projectId = projectId.value ?? null
  }
  showCreate.value = true
}

async function createAnimal() {
  if (!newAnimal.displayId.trim()) {
    message.warning('请输入小鼠编号')
    return
  }
  if (newAnimal.identifierScope === 'project' && !newAnimal.projectId) {
    message.warning('项目编号命名空间必须选择项目')
    return
  }
  busy.value = true
  try {
    const created = await gateway.createAnimal({
      displayId: newAnimal.displayId.trim(),
      identifierScope: newAnimal.identifierScope,
      projectId: newAnimal.identifierScope === 'project' ? newAnimal.projectId ?? undefined : undefined,
      cageId: gateway.mode === 'local' ? newAnimal.cageId ?? undefined : undefined,
      sex: newAnimal.sex,
      strain: newAnimal.strain.trim(),
      birthDate: formatDate(newAnimal.birthDate),
    })
    showCreate.value = false
    Object.assign(newAnimal, {
      displayId: '',
      identifierScope: projectOnly.value ? 'project' : 'lab',
      projectId: projectOnly.value ? projectId.value ?? null : null,
      cageId: null,
      sex: 'unknown', strain: '', birthDate: null,
    })
    await load()
    message.success(`已登记小鼠 ${created.code}`)
  } catch (error) {
    message.error(error instanceof Error ? error.message : '登记失败')
  } finally {
    busy.value = false
  }
}

function openGenotypeCreate() {
  if (!selected.value) return
  Object.assign(newGenotype, {
    locusValue: geneLoci.value[0]?.id ?? '',
    allele1Value: '',
    allele2Value: '',
    allele1WildType: false,
    allele2WildType: false,
    assessedAt: Date.now(),
  })
  showGenotypeCreate.value = true
}

function normalizedGeneticValue(value: string) {
  return value.trim().toLowerCase()
}

async function resolveLocus(value: string): Promise<GeneLocus> {
  const normalized = normalizedGeneticValue(value)
  const existing = geneLoci.value.find((locus) => locus.id === value
    || locus.symbol.toLowerCase() === normalized)
  if (existing) return existing
  const created = await gateway.createGeneLocus({
    projectId: projectId.value,
    symbol: value.trim(),
  })
  geneLoci.value.push(created)
  return created
}

async function resolveAllele(
  value: string,
  locus: GeneLocus,
  isWildType: boolean,
): Promise<GeneAllele> {
  const normalized = normalizedGeneticValue(value)
  const existing = geneAlleles.value.find((allele) => allele.locusId === locus.id
    && (allele.id === value || allele.symbol.toLowerCase() === normalized))
  if (existing) return existing
  const created = await gateway.createAllele({
    projectId: projectId.value,
    locusId: locus.id,
    symbol: value.trim(),
    isWildType,
  })
  geneAlleles.value.push(created)
  return created
}

function isNewAllele(value: string) {
  const normalized = normalizedGeneticValue(value)
  return !!value.trim() && !geneAlleles.value.some((allele) => allele.id === value
    || (allele.locusId === selectedLocus.value?.id && allele.symbol.toLowerCase() === normalized))
}

async function recordGenotype() {
  if (!selected.value || !newGenotype.locusValue.trim()
    || !newGenotype.allele1Value.trim() || !newGenotype.allele2Value.trim()) {
    message.warning('请选择或新建基因位点，并填写两个等位基因')
    return
  }
  genotypeSaving.value = true
  try {
    const locus = await resolveLocus(newGenotype.locusValue)
    const allele1 = await resolveAllele(
      newGenotype.allele1Value,
      locus,
      newGenotype.allele1WildType,
    )
    const allele2 = await resolveAllele(
      newGenotype.allele2Value,
      locus,
      newGenotype.allele2WildType,
    )
    await gateway.createGenotype({
      projectId: projectId.value,
      animalId: selected.value.id,
      locusId: locus.id,
      allele1Id: allele1.id,
      allele2Id: allele2.id,
      assessedAt: newGenotype.assessedAt
        ? new Date(newGenotype.assessedAt).toISOString()
        : undefined,
    })
    showGenotypeCreate.value = false
    await hydrateSelected(selected.value, false)
    detailTab.value = 'genotypes'
    message.success('基因型已记录')
  } catch (error) {
    message.error(error instanceof Error ? error.message : '记录基因型失败')
  } finally {
    genotypeSaving.value = false
  }
}

function openSampleCreate() {
  if (!selected.value) return
  Object.assign(newSample, {
    projectId: projectId.value ?? selectedProjectRefs.value[0]?.id ?? null,
    experimentId: null,
    sampleType: '',
    quantity: null,
    unit: '',
    location: '',
    collectedAt: Date.now(),
  })
  showSampleCreate.value = true
}

async function createSample() {
  if (!selected.value || !newSample.projectId || !newSample.sampleType.trim()) {
    message.warning('请选择项目并填写样本类型')
    return
  }
  if (newSample.quantity != null && !newSample.unit.trim()) {
    message.warning('填写数量时必须填写单位')
    return
  }
  sampleSaving.value = true
  try {
    await gateway.createAnimalSample({
      animalId: selected.value.id,
      projectId: newSample.projectId,
      experimentId: newSample.experimentId ?? undefined,
      sampleType: newSample.sampleType.trim(),
      quantity: newSample.quantity ?? undefined,
      unit: newSample.unit.trim() || undefined,
      location: newSample.location.trim() || undefined,
      collectedAt: newSample.collectedAt ? new Date(newSample.collectedAt).toISOString() : undefined,
    })
    showSampleCreate.value = false
    await hydrateSelected(selected.value, false)
    detailTab.value = 'samples'
    message.success('样本已登记')
  } catch (error) {
    message.error(error instanceof Error ? error.message : '登记样本失败')
  } finally {
    sampleSaving.value = false
  }
}

function chooseAttachment() {
  if (!gateway.uploadAttachment) {
    message.warning('当前运行模式未提供附件上传')
    return
  }
  fileInput.value?.click()
}

async function uploadAttachment(event: Event) {
  const input = event.target as HTMLInputElement
  const file = input.files?.[0]
  if (!file || !selected.value || !gateway.uploadAttachment) return
  attachmentUploading.value = true
  try {
    await gateway.uploadAttachment({
      entityType: 'animal',
      entityId: selected.value.id,
      projectId: attachmentProjectId.value ?? undefined,
      fileName: file.name,
      mediaType: file.type || undefined,
      content: file,
    })
    await hydrateSelected(selected.value, false)
    detailTab.value = 'attachments'
    message.success(`已上传 ${file.name}`)
  } catch (error) {
    message.error(error instanceof Error ? error.message : '附件上传失败')
  } finally {
    attachmentUploading.value = false
    input.value = ''
  }
}

async function downloadAttachment(id: string, fileName: string) {
  if (!gateway.downloadAttachment) {
    message.warning('当前运行模式未提供附件下载')
    return
  }
  attachmentDownloadingId.value = id
  try {
    const blob = await gateway.downloadAttachment(id)
    const url = URL.createObjectURL(blob)
    const anchor = document.createElement('a')
    anchor.href = url
    anchor.download = fileName
    anchor.click()
    window.setTimeout(() => URL.revokeObjectURL(url), 0)
  } catch (error) {
    message.error(error instanceof Error ? error.message : '附件下载失败')
  } finally {
    attachmentDownloadingId.value = null
  }
}

function openBreeding() {
  if (!selected.value) return
  void router.push({ name: 'breeding', query: { ...route.query, animal: selected.value.id } })
}

function openRelatedAnimal(relation: PedigreeRelation) {
  const animal = animals.value.find((item) => item.id === relation.relatedAnimal.id)
  if (animal) openAnimal(animal)
}

function openFromQuery(value: unknown) {
  if (typeof value !== 'string' || selected.value?.id === value) return
  const animal = animals.value.find((item) => item.id === value)
  if (animal) void hydrateSelected(animal)
}

watch(() => route.query.animal, openFromQuery)
watch(() => route.query.project_id, () => void load())
watch(currentProjectId, () => {
  if (gateway.mode === 'remote') void load()
})
watch(() => newAnimal.identifierScope, (scope) => {
  if (scope === 'lab') newAnimal.projectId = null
})
watch(() => newSample.projectId, () => { newSample.experimentId = null })
watch(() => newGenotype.locusValue, () => {
  newGenotype.allele1Value = ''
  newGenotype.allele2Value = ''
  newGenotype.allele1WildType = false
  newGenotype.allele2WildType = false
})
onMounted(load)
</script>

<template>
  <div class="page animals-page">
    <PageHeader title="小鼠档案" section="动物管理" description="查看每只小鼠的身份、当前位置、实验数据与完整可追溯记录。">
      <template #actions>
        <n-button v-if="animalWriteAllowed" type="primary" @click="openCreate"><template #icon><Plus :size="17" /></template>新增小鼠</n-button>
      </template>
    </PageHeader>

    <section class="summary-row">
      <div class="surface"><span>在养动物</span><strong>{{ animals.filter((animal) => animal.status !== 'archived').length }}</strong></div>
      <div class="surface"><span>实验中</span><strong>{{ animals.filter((animal) => animal.status === 'experiment').length }}</strong></div>
      <div class="surface attention"><span>待确认基因型</span><strong>{{ animals.filter((animal) => animal.genotype === '待确认').length }}</strong></div>
    </section>

    <section class="toolbar surface">
      <n-input v-model:value="search" clearable placeholder="搜索编号、品系、基因型或项目"><template #prefix><Search :size="16" /></template></n-input>
      <n-select v-model:value="status" clearable :options="statusOptions" placeholder="全部状态"><template #arrow><Filter :size="15" /></template></n-select>
      <span>{{ filtered.length }} 条记录</span>
    </section>

    <section class="surface desktop-only table-wrap">
      <n-data-table :columns="columns" :data="filtered" :loading="loading" :row-key="(row: Animal) => row.id" :bordered="false" :single-line="false" :pagination="{ pageSize: 12 }" />
    </section>

    <section class="mobile-list mobile-only" aria-label="小鼠卡片列表">
      <button v-for="animal in filtered" :key="animal.id" type="button" class="animal-card surface" :aria-label="`查看小鼠 ${animal.code}`" @click="openAnimal(animal)">
        <div class="card-title"><strong>{{ animal.code }}</strong><n-tag :type="statusMeta[animal.status].type" size="small" round :bordered="false">{{ statusMeta[animal.status].label }}</n-tag></div>
        <div class="card-grid"><span>性别</span><b>{{ sexLabel(animal.sex) }}</b><span>基因型</span><b>{{ animal.genotype }}</b><span>笼位</span><b>{{ animal.cageId ? cages.get(animal.cageId) : '未分配' }}</b></div>
        <small>{{ animal.strain }} · {{ animal.projectNames.join('、') || '未关联项目' }}</small>
      </button>
      <n-empty v-if="!loading && !filtered.length" description="没有匹配的小鼠" />
    </section>

    <n-modal v-model:show="showCreate" preset="card" title="登记小鼠" class="dialog-card" :bordered="false">
      <n-form label-placement="top">
        <div class="form-grid">
          <n-form-item label="显示编号" required><n-input v-model:value="newAnimal.displayId" placeholder="例如 M-26001" /></n-form-item>
          <n-form-item label="性别"><n-select v-model:value="newAnimal.sex" :options="[{label:'雄',value:'male'},{label:'雌',value:'female'},{label:'未知',value:'unknown'}]" /></n-form-item>
        </div>
        <div class="form-grid">
          <n-form-item label="编号命名空间" required><n-select v-model:value="newAnimal.identifierScope" :disabled="projectOnly" :options="[{label:'实验室内唯一',value:'lab'},{label:'项目内唯一',value:'project'}]" /></n-form-item>
          <n-form-item v-if="newAnimal.identifierScope === 'project'" label="科研项目" required><n-select v-model:value="newAnimal.projectId" :disabled="projectOnly" filterable :options="projectOptions" placeholder="选择项目" /></n-form-item>
          <n-form-item v-else label="科研项目"><n-input value="可在实验纳入时关联" disabled /></n-form-item>
        </div>
        <div class="form-grid">
          <n-form-item label="品系"><n-input v-model:value="newAnimal.strain" placeholder="例如 C57BL/6J" /></n-form-item>
          <n-form-item label="出生日期"><n-date-picker v-model:value="newAnimal.birthDate" type="date" clearable /></n-form-item>
        </div>
        <n-form-item v-if="gateway.mode === 'local'" label="初始笼位"><n-select v-model:value="newAnimal.cageId" clearable filterable :options="cageOptions" placeholder="可稍后转笼" /></n-form-item>
        <n-alert v-else type="info" :show-icon="false">共享版登记后通过受审计的转笼操作分配笼位。</n-alert>
      </n-form>
      <template #footer><div class="dialog-actions"><n-button @click="showCreate = false">取消</n-button><n-button type="primary" :loading="busy" @click="createAnimal">登记小鼠</n-button></div></template>
    </n-modal>

    <n-modal v-model:show="showGenotypeCreate" preset="card" title="记录基因型" class="dialog-card" :bordered="false">
      <n-form label-placement="top">
        <n-form-item label="基因位点" required>
          <n-select v-model:value="newGenotype.locusValue" filterable tag :options="locusOptions" placeholder="选择位点，或输入符号后回车新建" />
        </n-form-item>
        <div class="form-grid">
          <n-form-item label="等位基因 1" required>
            <n-select v-model:value="newGenotype.allele1Value" filterable tag :options="alleleOptions" placeholder="选择或输入后回车新建" />
            <n-checkbox v-if="isNewAllele(newGenotype.allele1Value)" v-model:checked="newGenotype.allele1WildType">新建为野生型</n-checkbox>
          </n-form-item>
          <n-form-item label="等位基因 2" required>
            <n-select v-model:value="newGenotype.allele2Value" filterable tag :options="alleleOptions" placeholder="选择或输入后回车新建" />
            <n-checkbox v-if="isNewAllele(newGenotype.allele2Value)" v-model:checked="newGenotype.allele2WildType">新建为野生型</n-checkbox>
          </n-form-item>
        </div>
        <n-form-item label="鉴定时间"><n-date-picker v-model:value="newGenotype.assessedAt" type="datetime" clearable /></n-form-item>
        <n-alert type="info" :show-icon="false">新输入的位点或等位基因会先创建，再与本次鉴定记录一同写入审计。</n-alert>
      </n-form>
      <template #footer><div class="dialog-actions"><n-button @click="showGenotypeCreate = false">取消</n-button><n-button type="primary" :loading="genotypeSaving" @click="recordGenotype">确认记录</n-button></div></template>
    </n-modal>

    <n-modal v-model:show="showSampleCreate" preset="card" title="登记样本" class="dialog-card" :bordered="false">
      <n-alert v-if="!selectedProjectOptions.length" type="warning" :show-icon="false">动物需先参与科研项目和实验，才能登记项目样本。</n-alert>
      <n-form label-placement="top">
        <div class="form-grid">
          <n-form-item label="科研项目" required><n-select v-model:value="newSample.projectId" :options="selectedProjectOptions" placeholder="选择动物已参与的项目" /></n-form-item>
          <n-form-item label="关联实验"><n-select v-model:value="newSample.experimentId" clearable :options="sampleExperimentOptions" placeholder="可选" /></n-form-item>
        </div>
        <div class="form-grid">
          <n-form-item label="样本类型" required><n-input v-model:value="newSample.sampleType" placeholder="例如 lung tissue" /></n-form-item>
          <n-form-item label="采集时间"><n-date-picker v-model:value="newSample.collectedAt" type="datetime" clearable /></n-form-item>
        </div>
        <div class="form-grid quantity-grid">
          <n-form-item label="数量"><n-input-number v-model:value="newSample.quantity" :min="0" clearable /></n-form-item>
          <n-form-item label="单位"><n-input v-model:value="newSample.unit" placeholder="例如 mg、μL" /></n-form-item>
        </div>
        <n-form-item label="保存位置"><n-input v-model:value="newSample.location" placeholder="例如 -80℃ A / Box 3 / A2" /></n-form-item>
      </n-form>
      <template #footer><div class="dialog-actions"><n-button @click="showSampleCreate = false">取消</n-button><n-button type="primary" :loading="sampleSaving" :disabled="!selectedProjectOptions.length" @click="createSample">确认登记</n-button></div></template>
    </n-modal>

    <input ref="fileInput" class="visually-hidden" type="file" @change="uploadAttachment">

    <n-drawer :show="!!selected" :width="680" placement="right" @update:show="(show: boolean) => !show && closeAnimal()">
      <n-drawer-content v-if="selected" closable :native-scrollbar="false">
        <template #header>
          <div class="detail-title">
            <div><strong>{{ selected.code }}</strong><span>{{ selected.strain }} · {{ selected.genotype }}</span></div>
            <n-tag :type="statusMeta[selected.status].type" round :bordered="false">{{ statusMeta[selected.status].label }}</n-tag>
          </div>
        </template>

        <div class="detail-summary">
          <div><span>性别</span><strong>{{ sexLabel(selected.sex) }}</strong></div>
          <div><span>出生日期</span><strong>{{ selected.birthDate || '未记录' }}</strong></div>
          <div><span>当前笼位</span><strong>{{ selected.cageId ? (cages.get(selected.cageId) ?? '未知') : '未分配' }}</strong></div>
          <div><span>最近体重</span><strong>{{ selected.weight == null ? '未记录' : `${selected.weight.toFixed(1)} g` }}</strong></div>
        </div>

        <n-alert v-if="detailError" type="error" :show-icon="false" class="detail-error">
          {{ detailError }}
          <template #action><n-button size="small" @click="hydrateSelected(selected, false)">重试</n-button></template>
        </n-alert>

        <n-spin :show="detailLoading">
          <n-tabs v-model:value="detailTab" type="line" animated>
            <n-tab-pane name="timeline" tab="时间线">
              <n-timeline v-if="detail?.timeline.length">
                <n-timeline-item v-for="event in detail.timeline" :key="event.id" :title="event.title" :time="formatDateTime(event.at)">
                  <p>{{ event.detail }}</p><small>{{ event.operator }}</small>
                </n-timeline-item>
              </n-timeline>
              <n-empty v-else-if="!detailLoading" description="暂无事件记录" />
            </n-tab-pane>

            <n-tab-pane name="genotypes" tab="基因型">
              <div class="tab-actions"><span>按位点记录双等位基因与鉴定时间。</span><n-button v-if="genotypeWriteAllowed" size="small" type="primary" @click="openGenotypeCreate"><template #icon><Plus :size="15" /></template>记录基因型</n-button></div>
              <n-spin :show="geneticsLoading">
                <div v-if="genotypeRows.length" class="record-list">
                  <article v-for="genotype in genotypeRows" :key="genotype.id" class="record-card genotype-card">
                    <header><div><strong>{{ genotype.locusLabel }}</strong><span>{{ formatDateTime(genotype.assessedAt) }}</span></div><n-tag size="small" :bordered="false">{{ genotype.alleleLabel }}</n-tag></header>
                    <small>revision {{ genotype.revision }}</small>
                  </article>
                </div>
                <n-empty v-else-if="!geneticsLoading" description="尚未记录基因型" />
              </n-spin>
            </n-tab-pane>

            <n-tab-pane name="experiments" tab="实验">
              <div v-if="detail?.experiments.length" class="record-list">
                <article v-for="record in detail.experiments" :key="record.participationId" class="record-card">
                  <header><div><strong>{{ record.experimentName }}</strong><span>{{ record.projectName }}</span></div><n-tag size="small" :bordered="false">{{ record.participationStatus }}</n-tag></header>
                  <dl><dt>实验状态</dt><dd>{{ record.experimentStatus }}</dd><dt>分组</dt><dd>{{ record.cohortName || '未分组' }}</dd><dt>纳入时间</dt><dd>{{ formatDateTime(record.enrolledAt) }}</dd><dt>退出时间</dt><dd>{{ formatDateTime(record.exitedAt) }}</dd></dl>
                  <small>revision {{ record.revision }}</small>
                </article>
              </div>
              <n-empty v-else-if="!detailLoading" description="尚未参与实验" />
            </n-tab-pane>

            <n-tab-pane name="measurements" tab="测量">
              <div v-if="detail?.measurements.length" class="measurement-list">
                <article v-for="measurement in detail.measurements" :key="measurement.id" class="measurement-row">
                  <div><strong>{{ measurement.label }}</strong><span>{{ measurement.key }}</span></div>
                  <b>{{ formatMeasurement(measurement) }}</b>
                  <div class="measurement-meta"><span>{{ formatDateTime(measurement.measuredAt) }}</span><n-tag size="tiny" :type="measurement.status === 'signed' ? 'success' : 'warning'" :bordered="false">{{ measurement.status === 'signed' ? '已签署' : '草稿' }}</n-tag><small>revision {{ measurement.revision }}</small></div>
                </article>
              </div>
              <n-empty v-else-if="!detailLoading" description="暂无测量记录" />
            </n-tab-pane>

            <n-tab-pane name="breeding" tab="繁育">
              <div class="tab-actions"><span>谱系来自双向父母/后代关系查询。</span><n-button v-if="!projectOnly" size="small" secondary @click="openBreeding">管理谱系</n-button></div>
              <div v-if="detail?.pedigree.length" class="relation-list">
                <button v-for="relation in detail.pedigree" :key="relation.id" type="button" class="relation-row" @click="openRelatedAnimal(relation)">
                  <n-tag size="small" :type="relation.direction === 'parent' ? 'info' : 'success'" :bordered="false">{{ pedigreeLabel(relation) }}</n-tag>
                  <span><strong>{{ relation.relatedAnimal.code }}</strong><small>{{ sexLabel(relation.relatedAnimal.sex) }} · {{ relation.relatedAnimal.strain || '未记录品系' }}</small></span>
                  <b>revision {{ relation.revision }}</b>
                </button>
              </div>
              <n-empty v-else-if="!detailLoading" description="尚未登记父母或后代关系" />
            </n-tab-pane>

            <n-tab-pane name="samples" tab="样本">
              <div class="tab-actions"><span>记录来源动物、项目、数量和位置。</span><n-button v-if="projectDataWriteAllowed" size="small" type="primary" :disabled="!selectedProjectOptions.length" @click="openSampleCreate"><template #icon><Plus :size="15" /></template>登记样本</n-button></div>
              <div v-if="detail?.samples.length" class="record-list">
                <article v-for="sample in detail.samples" :key="sample.id" class="record-card sample-card">
                  <header><div><strong>{{ sample.sampleType }}</strong><span>{{ projects.find((project) => project.id === sample.projectId)?.name || sample.projectId }}</span></div><n-tag size="small" :bordered="false">{{ sample.quantity == null ? '未记录数量' : `${sample.quantity} ${sample.unit || ''}` }}</n-tag></header>
                  <dl><dt>采集时间</dt><dd>{{ formatDateTime(sample.collectedAt) }}</dd><dt>保存位置</dt><dd>{{ sample.location || '未记录' }}</dd><dt>关联实验</dt><dd>{{ detail.experiments.find((record) => record.experimentId === sample.experimentId)?.experimentName || sample.experimentId || '未关联' }}</dd></dl>
                  <small>revision {{ sample.revision }}</small>
                </article>
              </div>
              <n-empty v-else-if="!detailLoading" description="暂无样本记录" />
            </n-tab-pane>

            <n-tab-pane name="attachments" tab="附件">
              <div class="tab-actions attachment-actions">
                <n-select v-if="selectedProjectOptions.length" v-model:value="attachmentProjectId" clearable size="small" :options="selectedProjectOptions" placeholder="实验室范围" />
                <span v-else>附件将关联到当前动物。</span>
                <n-button v-if="projectDataWriteAllowed" size="small" type="primary" :loading="attachmentUploading" :disabled="!gateway.uploadAttachment" @click="chooseAttachment"><template #icon><Upload :size="15" /></template>上传附件</n-button>
              </div>
              <div v-if="detail?.attachments.length" class="attachment-list">
                <article v-for="attachment in detail.attachments" :key="attachment.id" class="attachment-row">
                  <div><strong>{{ attachment.fileName }}</strong><span>{{ attachment.mediaType || 'application/octet-stream' }} · {{ formatBytes(attachment.sizeBytes) }}</span><small :title="attachment.sha256">SHA-256 {{ attachment.sha256.slice(0, 16) }}… · v{{ attachment.version }} · {{ formatDateTime(attachment.createdAt) }}</small></div>
                  <n-button size="small" secondary :loading="attachmentDownloadingId === attachment.id" :disabled="!gateway.downloadAttachment" @click="downloadAttachment(attachment.id, attachment.fileName)"><template #icon><Download :size="15" /></template>下载</n-button>
                </article>
              </div>
              <n-empty v-else-if="!detailLoading" description="暂无关联附件" />
            </n-tab-pane>

            <n-tab-pane name="audit" tab="审计">
              <n-alert v-if="detail && !detail.auditVisible" type="info" :show-icon="false">当前账号无权查看审计摘要。</n-alert>
              <template v-else>
                <section class="audit-section">
                  <h3>写入审计</h3>
                  <div v-if="detail?.audits.length" class="audit-list">
                    <article v-for="audit in detail.audits" :key="audit.id"><div><strong>{{ audit.action }}</strong><span>{{ audit.actor }} · {{ audit.source }}</span></div><div><span>{{ formatDateTime(audit.occurredAt) }}</span><small>{{ audit.revision == null ? 'revision 未提供' : `revision ${audit.revision}` }}</small></div><p v-if="audit.reason">{{ audit.reason }}</p></article>
                  </div>
                  <n-empty v-else-if="!detailLoading" size="small" description="暂无可见审计记录" />
                </section>
                <section class="audit-section provenance-section">
                  <h3>数据来源</h3>
                  <div v-if="detail?.provenance.length" class="provenance-list">
                    <article v-for="entry in detail.provenance" :key="entry.id"><n-tag size="small" :bordered="false">{{ entry.source }}</n-tag><div><strong>{{ entry.actor || '系统记录' }}</strong><span>{{ formatDateTime(entry.recordedAt) }}</span></div><small v-if="entry.requestId">request {{ entry.requestId }}</small></article>
                  </div>
                  <n-empty v-else-if="!detailLoading" size="small" description="暂无可见来源记录" />
                </section>
              </template>
            </n-tab-pane>
          </n-tabs>
        </n-spin>
      </n-drawer-content>
    </n-drawer>
  </div>
</template>

<style scoped>
.summary-row { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 10px; margin-bottom: 12px; }
.summary-row > div { display: flex; align-items: center; justify-content: space-between; padding: 13px 16px; }
.summary-row span { color: var(--muri-text-secondary); }
.summary-row strong { font-size: 20px; }
.summary-row .attention strong { color: var(--muri-warning); }
.toolbar { display: grid; grid-template-columns: minmax(260px, 420px) 160px 1fr; align-items: center; gap: 10px; margin-bottom: 12px; padding: 10px; }
.toolbar > span { justify-self: end; padding-right: 5px; color: var(--muri-text-tertiary); font-size: 12px; }
.table-wrap { overflow: hidden; }
:deep(.table-link) { padding: 0; border: 0; color: var(--muri-primary); background: transparent; cursor: pointer; font-weight: 600; }
.mobile-list { display: none; flex-direction: column; gap: 9px; }
.animal-card { padding: 13px; text-align: left; background: white; }
.card-title { display: flex; align-items: center; justify-content: space-between; margin-bottom: 9px; }
.card-title strong { font-size: 16px; }
.card-grid { display: grid; grid-template-columns: auto 1fr auto 1fr auto 1fr; gap: 5px; color: var(--muri-text-tertiary); font-size: 12px; }
.card-grid b { color: var(--muri-text); font-weight: 500; }
.animal-card > small { display: block; margin-top: 8px; color: var(--muri-text-secondary); }
.dialog-card { width: min(590px, calc(100vw - 28px)); }
.form-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; }
.quantity-grid { grid-template-columns: 2fr 1fr; }
.dialog-actions { display: flex; justify-content: flex-end; gap: 9px; }
.visually-hidden { position: fixed; width: 1px; height: 1px; opacity: 0; pointer-events: none; }
.detail-title { display: flex; width: 100%; align-items: center; justify-content: space-between; gap: 12px; }
.detail-title > div { display: flex; flex-direction: column; }
.detail-title strong { font-size: 18px; }
.detail-title span { margin-top: 2px; color: var(--muri-text-secondary); font-size: 12px; font-weight: 400; }
.detail-summary { display: grid; grid-template-columns: repeat(4, 1fr); gap: 8px; margin-bottom: 14px; }
.detail-summary > div { display: flex; padding: 10px; flex-direction: column; border: 1px solid var(--muri-border); border-radius: 7px; background: var(--muri-surface-muted); }
.detail-summary span { color: var(--muri-text-tertiary); font-size: 11px; }
.detail-summary strong { margin-top: 2px; overflow: hidden; font-size: 13px; text-overflow: ellipsis; white-space: nowrap; }
.detail-error { margin-bottom: 12px; }
:deep(.n-tabs-nav-scroll-content) { min-width: max-content; }
:deep(.n-tab-pane) { min-height: 220px; padding-top: 13px; }
:deep(.n-timeline-item-content p) { margin: 4px 0; color: var(--muri-text-secondary); }
:deep(.n-timeline-item-content small) { color: var(--muri-text-tertiary); }
:deep(.n-empty) { min-height: 170px; justify-content: center; }
.tab-actions { display: flex; min-height: 34px; align-items: center; justify-content: space-between; gap: 10px; margin-bottom: 10px; }
.tab-actions > span { color: var(--muri-text-tertiary); font-size: 12px; }
.attachment-actions > .n-select { width: min(230px, 55%); }
.record-list, .measurement-list, .relation-list, .attachment-list, .audit-list, .provenance-list { display: flex; flex-direction: column; gap: 8px; }
.record-card { padding: 12px; border: 1px solid var(--muri-border); border-radius: 7px; }
.record-card header { display: flex; align-items: flex-start; justify-content: space-between; gap: 10px; }
.record-card header > div { display: flex; min-width: 0; flex-direction: column; }
.record-card header span { margin-top: 2px; color: var(--muri-text-tertiary); font-size: 11px; }
.record-card dl { display: grid; grid-template-columns: auto 1fr auto 1fr; gap: 4px 9px; margin: 10px 0 0; font-size: 12px; }
.record-card dt { color: var(--muri-text-tertiary); }
.record-card dd { margin: 0; overflow: hidden; text-overflow: ellipsis; }
.record-card > small { display: block; margin-top: 8px; color: var(--muri-text-tertiary); }
.measurement-row { display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: 8px 12px; padding: 11px 12px; border: 1px solid var(--muri-border); border-radius: 7px; }
.measurement-row > div:first-child { display: flex; flex-direction: column; }
.measurement-row > div:first-child span { color: var(--muri-text-tertiary); font-size: 11px; }
.measurement-row > b { align-self: center; color: var(--muri-primary); font-size: 15px; }
.measurement-meta { display: flex; grid-column: 1 / -1; align-items: center; gap: 8px; color: var(--muri-text-tertiary); font-size: 11px; }
.measurement-meta small { margin-left: auto; }
.relation-row { display: grid; grid-template-columns: auto 1fr auto; align-items: center; gap: 10px; padding: 10px; border: 1px solid var(--muri-border); border-radius: 7px; background: white; text-align: left; cursor: pointer; }
.relation-row > span { display: flex; min-width: 0; flex-direction: column; }
.relation-row small { color: var(--muri-text-tertiary); }
.relation-row > b { color: var(--muri-text-tertiary); font-size: 10px; font-weight: 400; }
.attachment-row { display: flex; align-items: center; justify-content: space-between; gap: 12px; padding: 11px 12px; border: 1px solid var(--muri-border); border-radius: 7px; }
.attachment-row > div { display: flex; min-width: 0; flex-direction: column; }
.attachment-row strong, .attachment-row span, .attachment-row small { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.attachment-row span { margin-top: 2px; color: var(--muri-text-secondary); font-size: 11px; }
.attachment-row small { margin-top: 4px; color: var(--muri-text-tertiary); font-size: 10px; }
.audit-section h3 { margin: 0 0 9px; font-size: 13px; }
.audit-section + .audit-section { margin-top: 16px; padding-top: 14px; border-top: 1px solid var(--muri-border); }
.audit-list article { display: grid; grid-template-columns: 1fr auto; gap: 4px 10px; padding: 10px; border-radius: 7px; background: var(--muri-surface-muted); }
.audit-list article > div { display: flex; flex-direction: column; }
.audit-list article > div:nth-child(2) { align-items: flex-end; }
.audit-list span, .audit-list small { color: var(--muri-text-tertiary); font-size: 11px; }
.audit-list p { grid-column: 1 / -1; margin: 3px 0 0; color: var(--muri-text-secondary); font-size: 12px; }
.provenance-list article { display: grid; grid-template-columns: auto 1fr auto; align-items: center; gap: 9px; padding: 9px 10px; border: 1px solid var(--muri-border); border-radius: 7px; }
.provenance-list article > div { display: flex; flex-direction: column; }
.provenance-list span, .provenance-list small { color: var(--muri-text-tertiary); font-size: 11px; }
@media (max-width: 900px) {
  .summary-row { grid-template-columns: repeat(3, 1fr); }
  .summary-row > div { align-items: flex-start; padding: 10px; flex-direction: column; }
  .summary-row span { font-size: 11px; }
  .summary-row strong { margin-top: 2px; font-size: 18px; }
  .toolbar { grid-template-columns: 1fr 130px; }
  .toolbar > span { display: none; }
  :global(.n-drawer) { max-width: 100% !important; }
  .detail-summary { grid-template-columns: 1fr 1fr; }
  .card-grid { grid-template-columns: auto 1fr auto 1fr; }
  .form-grid, .quantity-grid { grid-template-columns: 1fr; gap: 0; }
  .record-card dl { grid-template-columns: auto 1fr; }
  .relation-row { grid-template-columns: auto 1fr; }
  .relation-row > b { grid-column: 2; }
  .attachment-actions { align-items: stretch; flex-direction: column; }
  .attachment-actions > .n-select { width: 100%; }
  .attachment-row { align-items: flex-start; }
  .provenance-list article { grid-template-columns: auto 1fr; }
  .provenance-list article > small { grid-column: 2; }
}
@media (prefers-reduced-motion: reduce) {
  :deep(.n-tabs-pane-wrapper), .relation-row { transition: none; }
}
</style>
