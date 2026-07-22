<script setup lang="ts">
import { computed, onMounted, reactive, ref, watch } from 'vue'
import {
  Baby,
  Calculator,
  Dna,
  HeartHandshake,
  Network,
  Plus,
  Search,
  TestTube2,
} from '@lucide/vue'
import { useDialog, useMessage } from 'naive-ui'
import { useRoute, useRouter } from 'vue-router'
import type {
  Animal,
  AnimalDetail,
  AnimalDraft,
  BreedingLine,
  BreedingPair,
  Colony,
  GeneAllele,
  GeneLocus,
  GenotypeComponentMode,
  GenotypeDefinition,
  GenotypingRecord,
  GenotypingState,
  Litter,
  LocusPrediction,
  MatingEvent,
  PedigreeRelation,
} from '@/domain/models'
import { currentGenotypingRecords } from '@/domain/genetics'
import { gateway } from '@/services/gateway'
import { canManageBreeding, canWriteAnimal } from '@/services/projectContext'
import PageHeader from '@/components/PageHeader.vue'

type WorkspaceTab = 'pedigree' | 'genetics' | 'lines' | 'pairs' | 'litters' | 'prediction'

const route = useRoute()
const router = useRouter()
const message = useMessage()
const dialog = useDialog()

const activeTab = ref<WorkspaceTab>('pedigree')
const loading = ref(true)
const detailLoading = ref(false)
const busy = ref(false)
const animals = ref<Animal[]>([])
const detail = ref<AnimalDetail | null>(null)
const selectedId = ref<string | null>(null)
const search = ref('')
const loci = ref<GeneLocus[]>([])
const alleles = ref<GeneAllele[]>([])
const definitions = ref<GenotypeDefinition[]>([])
const records = ref<GenotypingRecord[]>([])
const lines = ref<BreedingLine[]>([])
const colonies = ref<Colony[]>([])
const pairs = ref<BreedingPair[]>([])
const matingEvents = ref<MatingEvent[]>([])
const litters = ref<Litter[]>([])
const drafts = ref<AnimalDraft[]>([])
const predictions = ref<LocusPrediction[]>([])
const genotypingAnimalId = ref<string | null>(null)
const selectedPairId = ref<string | null>(null)
const selectedLitterId = ref<string | null>(null)

const showCreateRelation = ref(false)
const showCreateLocus = ref(false)
const showCreateAllele = ref(false)
const showCreateDefinition = ref(false)
const showCreateRecord = ref(false)
const showVoidRecord = ref(false)
const showCorrectRecord = ref(false)
const showCreateLine = ref(false)
const showCreateColony = ref(false)
const showCreatePair = ref(false)
const showCreateMating = ref(false)
const showCreateLitter = ref(false)
const showRegisterDraft = ref(false)
const showArchivedGenetics = ref(false)
const lifecycleRecord = ref<GenotypingRecord | null>(null)
const voidReason = ref('')

const relationForm = reactive({
  parentId: null as string | null,
  parentType: 'unknown' as 'father' | 'mother' | 'unknown',
})
const locusForm = reactive({ symbol: '', description: '' })
const alleleForm = reactive({
  locusId: null as string | null,
  symbol: '',
  description: '',
  isWildType: false,
})
const definitionForm = reactive({
  name: '',
  description: '',
  components: [] as Array<{
    locusId: string | null
    allele1Id: string | null
    allele2Id: string | null
    mode: GenotypeComponentMode
  }>,
})
const recordForm = reactive({
  genotypeDefinitionId: null as string | null,
  state: 'unknown' as GenotypingState,
  assessedAt: null as number | null,
  method: '',
  notes: '',
})
const correctionForm = reactive({
  genotypeDefinitionId: null as string | null,
  state: 'unknown' as GenotypingState,
  assessedAt: null as number | null,
  method: '',
  notes: '',
  reason: '',
})
const lineForm = reactive({
  name: '',
  description: '',
  genotypeDefinitionIds: [] as string[],
})
const colonyForm = reactive({
  breedingLineId: null as string | null,
  name: '',
  description: '',
})
const pairForm = reactive({
  colonyId: null as string | null,
  name: '',
  maleAnimalId: null as string | null,
  femaleAnimalIds: [] as string[],
  startedAt: null as number | null,
})
const matingForm = reactive({
  femaleAnimalId: null as string | null,
  occurredAt: null as number | null,
  notes: '',
})
const litterForm = reactive({
  matingEventId: null as string | null,
  bornOn: null as number | null,
  sizeTotal: 1,
  drafts: [{ temporaryLabel: 'P1', sex: 'unknown' as Animal['sex'] }],
  notes: '',
})
const registerForm = reactive({
  draftId: null as string | null,
  displayId: '',
  identifierScope: 'lab' as 'lab' | 'project',
  strain: '',
})
const predictionForm = reactive({
  maleGenotypeDefinitionId: null as string | null,
  femaleGenotypeDefinitionId: null as string | null,
})

const projectId = computed(() => typeof route.query.project_id === 'string'
  ? route.query.project_id
  : undefined)
const writeAllowed = computed(() => gateway.mode === 'local' || canManageBreeding())
const animalRegistrationAllowed = computed(
  () => writeAllowed.value && (gateway.mode === 'local' || canWriteAnimal()),
)
const selected = computed(
  () => animals.value.find((animal) => animal.id === selectedId.value) ?? null,
)
const selectedPair = computed(
  () => pairs.value.find((pair) => pair.id === selectedPairId.value) ?? null,
)
const selectedLitter = computed(
  () => litters.value.find((litter) => litter.id === selectedLitterId.value) ?? null,
)
const parents = computed(
  () => detail.value?.pedigree.filter((item) => item.direction === 'parent') ?? [],
)
const offspring = computed(
  () => detail.value?.pedigree.filter((item) => item.direction === 'offspring') ?? [],
)
const filteredAnimals = computed(() => {
  const query = search.value.trim().toLowerCase()
  if (!query) return animals.value
  return animals.value.filter((animal) => [
    animal.code,
    animal.legacyCode,
    animal.strain,
    animal.genotype,
  ].some((value) => value?.toLowerCase().includes(query)))
})
const animalLabels = computed(
  () => new Map(animals.value.map((animal) => [animal.id, animal.code])),
)
const locusLabels = computed(
  () => new Map(loci.value.map((locus) => [locus.id, locus.symbol])),
)
const alleleLabels = computed(
  () => new Map(alleles.value.map((allele) => [allele.id, allele.symbol])),
)
const definitionLabels = computed(
  () => new Map(definitions.value.map((definition) => [definition.id, definition.name])),
)
const activeLoci = computed(() => loci.value.filter((locus) => !locus.archivedAt))
const visibleLoci = computed(() => loci.value.filter(
  (locus) => showArchivedGenetics.value || !locus.archivedAt,
))
const activeDefinitions = computed(() => definitions.value.filter(
  (definition) => !definition.archivedAt,
))
const visibleDefinitions = computed(() => definitions.value.filter(
  (definition) => showArchivedGenetics.value || !definition.archivedAt,
))
const currentRecordIds = computed(() => new Set(
  currentGenotypingRecords(records.value).map((record) => record.id),
))
const lineLabels = computed(() => new Map(lines.value.map((line) => [line.id, line.name])))
const colonyLabels = computed(
  () => new Map(colonies.value.map((colony) => [colony.id, colony.name])),
)
const definitionOptions = computed(() => activeDefinitions.value.map((definition) => ({
  label: definition.name,
  value: definition.id,
})))
const locusOptions = computed(() => activeLoci.value.map((locus) => ({
  label: locus.symbol,
  value: locus.id,
})))
const lineOptions = computed(() => lines.value.map((line) => ({
  label: line.name,
  value: line.id,
})))
const colonyOptions = computed(() => colonies.value.map((colony) => ({
  label: `${colony.name} · ${lineLabels.value.get(colony.breedingLineId) ?? '未知品系'}`,
  value: colony.id,
})))
const animalOptions = computed(() => animals.value.map((animal) => ({
  label: `${animal.code} · ${sexLabel(animal.sex)} · ${animal.strain}`,
  value: animal.id,
})))
const maleOptions = computed(() => animals.value
  .filter((animal) => animal.sex === 'male' && animal.status !== 'archived')
  .map((animal) => ({ label: `${animal.code} · ${animal.strain}`, value: animal.id })))
const femaleOptions = computed(() => animals.value
  .filter((animal) => animal.sex === 'female' && animal.status !== 'archived')
  .map((animal) => ({ label: `${animal.code} · ${animal.strain}`, value: animal.id })))
const pairFemaleOptions = computed(() => selectedPair.value?.members
  .filter((member) => member.role === 'female' && !member.leftAt)
  .map((member) => ({
    value: member.animalId,
    label: animalLabels.value.get(member.animalId) ?? member.animalId,
  })) ?? [])
const matingEventOptions = computed(() => matingEvents.value.map((event) => ({
  value: event.id,
  label: `${formatInstant(event.occurredAt)} · ${animalLabels.value.get(event.femaleAnimalId) ?? event.femaleAnimalId}`,
})))

const componentModeOptions = [
  { label: '二倍体', value: 'diploid' },
  { label: '半合子', value: 'hemizygous' },
  { label: '转基因存在', value: 'transgene_presence' },
  { label: '条件性', value: 'conditional' },
]
const genotypeStateMeta: Record<GenotypingState, { label: string; type: 'default' | 'info' | 'success' | 'error' }> = {
  unknown: { label: '未知', type: 'default' },
  expected: { label: '预期', type: 'info' },
  confirmed: { label: '已确认', type: 'success' },
  rejected: { label: '已排除', type: 'error' },
}

function sexLabel(sex: Animal['sex']) {
  return sex === 'male' ? '雄' : sex === 'female' ? '雌' : '未知'
}

function relationLabel(relation: PedigreeRelation) {
  if (relation.direction === 'offspring') return '后代'
  if (relation.parentType === 'father') return '父本'
  if (relation.parentType === 'mother') return '母本'
  return '父母（未分类）'
}

function formatInstant(value?: string) {
  return value ? new Date(value).toLocaleString('zh-CN') : '未记录'
}

function dateValue(value: number | null) {
  if (!value) return undefined
  const date = new Date(value)
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, '0')}-${String(date.getDate()).padStart(2, '0')}`
}

function instantValue(value: number | null) {
  return value ? new Date(value).toISOString() : undefined
}

function alleleOptionsFor(locusId: string | null) {
  if (!locusId) return []
  return alleles.value
    .filter((allele) => allele.locusId === locusId && !allele.archivedAt)
    .map((allele) => ({
      value: allele.id,
      label: `${allele.symbol}${allele.isWildType ? ' · WT' : ''}`,
    }))
}

function componentNeedsSecondAllele(mode: GenotypeComponentMode) {
  return mode === 'diploid' || mode === 'conditional'
}

function genotypeComponentLabel(definition: GenotypeDefinition) {
  return definition.components.map((component) => {
    const locus = locusLabels.value.get(component.locusId) ?? component.locusId.slice(0, 8)
    const first = alleleLabels.value.get(component.allele1Id) ?? '?'
    const second = component.allele2Id
      ? (alleleLabels.value.get(component.allele2Id) ?? '?')
      : '—'
    return `${locus} ${first}/${second}`
  }).join(' · ')
}

function predictionAllele(id?: string) {
  return id ? (alleleLabels.value.get(id) ?? id.slice(0, 8)) : '无'
}

async function loadDetail(animalId: string) {
  detailLoading.value = true
  detail.value = null
  try {
    const result = await gateway.getAnimalDetail(
      animalId,
      projectId.value ? { projectId: projectId.value } : undefined,
    )
    if (selectedId.value === animalId) detail.value = result
  } catch (error) {
    message.error(error instanceof Error ? error.message : '读取谱系失败')
  } finally {
    if (selectedId.value === animalId) detailLoading.value = false
  }
}

async function loadRecords(animalId?: string | null) {
  if (!animalId) {
    records.value = []
    return
  }
  try {
    records.value = await gateway.listGenotypingRecords(animalId, projectId.value)
  } catch (error) {
    message.error(error instanceof Error ? error.message : '读取基因检测记录失败')
  }
}

async function loadGeneticsCatalogs() {
  ;[loci.value, definitions.value] = await Promise.all([
    gateway.listGeneLoci(projectId.value, true),
    gateway.listGenotypeDefinitions(projectId.value, true),
  ])
  alleles.value = (await Promise.all(
    loci.value.map((locus) => gateway.listAlleles(locus.id, projectId.value, true)),
  )).flat()
}

async function loadPairResources(pairId?: string | null) {
  if (!pairId) {
    matingEvents.value = []
    litters.value = []
    drafts.value = []
    return
  }
  try {
    ;[matingEvents.value, litters.value] = await Promise.all([
      gateway.listMatingEvents(pairId),
      gateway.listLitters(pairId),
    ])
    const keep = litters.value.some((litter) => litter.id === selectedLitterId.value)
    await selectLitter(keep ? selectedLitterId.value : litters.value[0]?.id ?? null)
  } catch (error) {
    message.error(error instanceof Error ? error.message : '读取配对事件失败')
  }
}

async function selectLitter(litterId: string | null) {
  selectedLitterId.value = litterId
  drafts.value = litterId ? await gateway.listAnimalDrafts(litterId) : []
}

function selectAnimal(animalId: string) {
  if (selectedId.value === animalId && detail.value) return
  selectedId.value = animalId
  void router.replace({ query: { ...route.query, animal: animalId } })
  void loadDetail(animalId)
}

function clearSelection() {
  selectedId.value = null
  detail.value = null
  const query = { ...route.query }
  delete query.animal
  void router.replace({ query })
}

async function selectPair(pairId: string | null) {
  selectedPairId.value = pairId
  await loadPairResources(pairId)
}

async function load() {
  loading.value = true
  try {
    const base = await Promise.all([
      gateway.listAnimals(projectId.value ? { projectId: projectId.value } : undefined),
      gateway.listGeneLoci(projectId.value, true),
      gateway.listGenotypeDefinitions(projectId.value, true),
      gateway.listBreedingLines(),
      gateway.listColonies(),
      gateway.listBreedingPairs(),
    ])
    ;[
      animals.value,
      loci.value,
      definitions.value,
      lines.value,
      colonies.value,
      pairs.value,
    ] = base
    alleles.value = (await Promise.all(
      loci.value.map((locus) => gateway.listAlleles(locus.id, projectId.value, true)),
    )).flat()

    const requested = typeof route.query.animal === 'string' ? route.query.animal : undefined
    if (requested && animals.value.some((animal) => animal.id === requested)) {
      selectAnimal(requested)
    } else if (requested) {
      clearSelection()
    }
    if (!genotypingAnimalId.value && animals.value.length) {
      genotypingAnimalId.value = requested ?? animals.value[0].id
    }
    const pairId = pairs.value.some((pair) => pair.id === selectedPairId.value)
      ? selectedPairId.value
      : pairs.value[0]?.id ?? null
    await selectPair(pairId)
    await loadRecords(genotypingAnimalId.value)
  } catch (error) {
    message.error(error instanceof Error ? error.message : '读取繁育工作区失败')
  } finally {
    loading.value = false
  }
}

function openCreateRelation() {
  if (!selected.value) return
  relationForm.parentId = null
  relationForm.parentType = 'unknown'
  showCreateRelation.value = true
}

async function createRelation() {
  if (!selected.value || !relationForm.parentId) {
    message.warning('请选择父本或母本')
    return
  }
  busy.value = true
  try {
    await gateway.createPedigree({
      projectId: projectId.value,
      animalId: selected.value.id,
      parentId: relationForm.parentId,
      parentType: relationForm.parentType,
    })
    showCreateRelation.value = false
    await loadDetail(selected.value.id)
    message.success('谱系关系已登记')
  } catch (error) {
    message.error(error instanceof Error ? error.message : '登记谱系关系失败')
  } finally {
    busy.value = false
  }
}

function openRelated(relation: PedigreeRelation) {
  const related = animals.value.find((animal) => animal.id === relation.relatedAnimal.id)
  if (related) {
    selectAnimal(related.id)
    return
  }
  void router.push({
    name: 'animals',
    query: { ...route.query, animal: relation.relatedAnimal.id },
  })
}

async function createLocus() {
  if (!locusForm.symbol.trim()) return message.warning('请输入基因位点符号')
  busy.value = true
  try {
    const locus = await gateway.createGeneLocus({
      projectId: projectId.value,
      symbol: locusForm.symbol.trim(),
      description: locusForm.description.trim() || undefined,
    })
    if (!loci.value.some((item) => item.id === locus.id)) loci.value.push(locus)
    alleleForm.locusId = locus.id
    Object.assign(locusForm, { symbol: '', description: '' })
    showCreateLocus.value = false
    message.success('基因位点已创建')
  } catch (error) {
    message.error(error instanceof Error ? error.message : '创建基因位点失败')
  } finally { busy.value = false }
}

async function createAllele() {
  if (!alleleForm.locusId || !alleleForm.symbol.trim()) {
    return message.warning('请选择位点并填写等位基因')
  }
  busy.value = true
  try {
    const allele = await gateway.createAllele({
      projectId: projectId.value,
      locusId: alleleForm.locusId,
      symbol: alleleForm.symbol.trim(),
      description: alleleForm.description.trim() || undefined,
      isWildType: alleleForm.isWildType,
    })
    if (!alleles.value.some((item) => item.id === allele.id)) alleles.value.push(allele)
    Object.assign(alleleForm, {
      symbol: '',
      description: '',
      isWildType: false,
    })
    showCreateAllele.value = false
    message.success('等位基因已创建')
  } catch (error) {
    message.error(error instanceof Error ? error.message : '创建等位基因失败')
  } finally { busy.value = false }
}

function addDefinitionComponent() {
  definitionForm.components.push({
    locusId: null,
    allele1Id: null,
    allele2Id: null,
    mode: 'diploid',
  })
}

function openDefinition() {
  Object.assign(definitionForm, { name: '', description: '' })
  definitionForm.components.splice(0)
  addDefinitionComponent()
  showCreateDefinition.value = true
}

function resetDefinitionAlleles(index: number) {
  const component = definitionForm.components[index]
  if (!component) return
  component.allele1Id = null
  component.allele2Id = null
}

function normalizeDefinitionMode(index: number) {
  const component = definitionForm.components[index]
  if (component && !componentNeedsSecondAllele(component.mode)) component.allele2Id = null
}

async function createDefinition() {
  if (!definitionForm.name.trim() || !definitionForm.components.length
    || definitionForm.components.some((component) =>
      !component.locusId || !component.allele1Id
      || (componentNeedsSecondAllele(component.mode) && !component.allele2Id))) {
    return message.warning('请完整填写基因型名称和全部组件')
  }
  busy.value = true
  try {
    const definition = await gateway.createGenotypeDefinition({
      projectId: projectId.value,
      name: definitionForm.name.trim(),
      description: definitionForm.description.trim() || undefined,
      components: definitionForm.components.map((component, index) => ({
        locusId: component.locusId!,
        allele1Id: component.allele1Id!,
        allele2Id: component.allele2Id ?? undefined,
        mode: component.mode,
        displayOrder: index,
      })),
    })
    definitions.value.push(definition)
    recordForm.genotypeDefinitionId = definition.id
    showCreateDefinition.value = false
    message.success('结构化基因型定义已创建')
  } catch (error) {
    message.error(error instanceof Error ? error.message : '创建基因型定义失败')
  } finally { busy.value = false }
}

function openRecord() {
  if (!genotypingAnimalId.value) return
  Object.assign(recordForm, {
    genotypeDefinitionId: definitions.value[0]?.id ?? null,
    state: 'unknown',
    assessedAt: Date.now(),
    method: '',
    notes: '',
  })
  showCreateRecord.value = true
}

async function createRecord() {
  if (!genotypingAnimalId.value || !recordForm.genotypeDefinitionId) {
    return message.warning('请选择动物和基因型定义')
  }
  if ((recordForm.state === 'confirmed' || recordForm.state === 'rejected')
    && !recordForm.assessedAt) {
    return message.warning('确认或排除结果必须填写检测时间')
  }
  busy.value = true
  try {
    await gateway.createGenotypingRecord({
      projectId: projectId.value,
      animalId: genotypingAnimalId.value,
      genotypeDefinitionId: recordForm.genotypeDefinitionId,
      state: recordForm.state,
      assessedAt: instantValue(recordForm.assessedAt),
      method: recordForm.method.trim() || undefined,
      notes: recordForm.notes.trim() || undefined,
    })
    await loadRecords(genotypingAnimalId.value)
    showCreateRecord.value = false
    message.success('基因检测记录已保存')
  } catch (error) {
    message.error(error instanceof Error ? error.message : '保存基因检测记录失败')
  } finally { busy.value = false }
}

function confirmCatalogMutation(
  title: string,
  content: string,
  positiveText: string,
  action: () => Promise<void>,
) {
  dialog.warning({
    title,
    content,
    positiveText,
    negativeText: '取消',
    onPositiveClick: action,
  })
}

async function toggleDefinitionArchive(definition: GenotypeDefinition) {
  try {
    const restoring = Boolean(definition.archivedAt)
    const counts = await gateway.genotypeDefinitionReferences(definition.id, projectId.value)
    confirmCatalogMutation(
      restoring ? '恢复基因型定义' : '归档基因型定义',
      restoring
        ? `恢复后可用于新记录。历史检测 ${counts.genotypingRecords} 条，繁育品系引用 ${counts.breedingLines} 个。`
        : `归档后不再进入新记录选择器，但保留历史。当前有检测 ${counts.genotypingRecords} 条、繁育品系引用 ${counts.breedingLines} 个。`,
      restoring ? '确认恢复' : '确认归档',
      async () => {
        busy.value = true
        try {
          const input = {
            id: definition.id,
            expectedRevision: definition.revision,
            projectId: projectId.value,
          }
          if (restoring) await gateway.restoreGenotypeDefinition(input)
          else await gateway.archiveGenotypeDefinition(input)
          await loadGeneticsCatalogs()
          message.success(restoring ? '基因型定义已恢复' : '基因型定义已归档')
        } catch (error) {
          message.error(error instanceof Error ? error.message : '更新基因型定义失败')
        } finally { busy.value = false }
      },
    )
  } catch (error) {
    message.error(error instanceof Error ? error.message : '读取引用影响失败')
  }
}

async function toggleLocusArchive(locus: GeneLocus) {
  try {
    const restoring = Boolean(locus.archivedAt)
    const counts = await gateway.geneLocusReferences(locus.id, projectId.value)
    if (!restoring && counts.activeGenotypeDefinitions > 0) {
      message.warning(`该位点仍被 ${counts.activeGenotypeDefinitions} 个活动定义引用，请先归档定义`)
      return
    }
    confirmCatalogMutation(
      restoring ? '恢复基因位点' : '归档基因位点',
      `关联定义 ${counts.genotypeDefinitions} 个、检测 ${counts.genotypingRecords} 条、繁育品系 ${counts.breedingLines} 个；历史引用不会被删除。`,
      restoring ? '确认恢复' : '确认归档',
      async () => {
        busy.value = true
        try {
          const input = { id: locus.id, expectedRevision: locus.revision, projectId: projectId.value }
          if (restoring) await gateway.restoreGeneLocus(input)
          else await gateway.archiveGeneLocus(input)
          await loadGeneticsCatalogs()
          message.success(restoring ? '基因位点已恢复' : '基因位点已归档')
        } catch (error) {
          message.error(error instanceof Error ? error.message : '更新基因位点失败')
        } finally { busy.value = false }
      },
    )
  } catch (error) {
    message.error(error instanceof Error ? error.message : '读取引用影响失败')
  }
}

async function toggleAlleleArchive(allele: GeneAllele) {
  try {
    const restoring = Boolean(allele.archivedAt)
    const counts = await gateway.alleleReferences(allele.id, projectId.value)
    if (!restoring && counts.activeGenotypeDefinitions > 0) {
      message.warning(`该 allele 仍被 ${counts.activeGenotypeDefinitions} 个活动定义引用，请先归档定义`)
      return
    }
    confirmCatalogMutation(
      restoring ? '恢复 allele' : '归档 allele',
      `关联定义 ${counts.genotypeDefinitions} 个、检测 ${counts.genotypingRecords} 条、繁育品系 ${counts.breedingLines} 个；历史引用不会被删除。`,
      restoring ? '确认恢复' : '确认归档',
      async () => {
        busy.value = true
        try {
          const input = { id: allele.id, expectedRevision: allele.revision, projectId: projectId.value }
          if (restoring) await gateway.restoreAllele(input)
          else await gateway.archiveAllele(input)
          await loadGeneticsCatalogs()
          message.success(restoring ? 'allele 已恢复' : 'allele 已归档')
        } catch (error) {
          message.error(error instanceof Error ? error.message : '更新 allele 失败')
        } finally { busy.value = false }
      },
    )
  } catch (error) {
    message.error(error instanceof Error ? error.message : '读取引用影响失败')
  }
}

function openVoidRecord(record: GenotypingRecord) {
  lifecycleRecord.value = record
  voidReason.value = ''
  showVoidRecord.value = true
}

async function submitVoidRecord() {
  const record = lifecycleRecord.value
  if (!record || !voidReason.value.trim()) return message.warning('请填写作废原因')
  busy.value = true
  try {
    await gateway.voidGenotypingRecord({
      recordId: record.id,
      expectedRevision: record.revision,
      reason: voidReason.value.trim(),
      projectId: projectId.value,
    })
    showVoidRecord.value = false
    await loadRecords(genotypingAnimalId.value)
    message.success('检测记录已作废，历史仍完整保留')
  } catch (error) {
    message.error(error instanceof Error ? error.message : '作废检测记录失败')
  } finally { busy.value = false }
}

function openCorrectRecord(record: GenotypingRecord) {
  lifecycleRecord.value = record
  Object.assign(correctionForm, {
    genotypeDefinitionId: record.genotypeDefinitionId,
    state: record.state,
    assessedAt: record.assessedAt ? new Date(record.assessedAt).getTime() : null,
    method: record.method ?? '',
    notes: record.notes ?? '',
    reason: '',
  })
  showCorrectRecord.value = true
}

async function submitCorrectRecord() {
  const record = lifecycleRecord.value
  if (!record || !correctionForm.genotypeDefinitionId || !correctionForm.reason.trim()) {
    return message.warning('请选择替代定义并填写更正原因')
  }
  if ((correctionForm.state === 'confirmed' || correctionForm.state === 'rejected')
    && !correctionForm.assessedAt) {
    return message.warning('确认或排除结果必须填写检测时间')
  }
  busy.value = true
  try {
    await gateway.correctGenotypingRecord({
      recordId: record.id,
      expectedRevision: record.revision,
      reason: correctionForm.reason.trim(),
      genotypeDefinitionId: correctionForm.genotypeDefinitionId,
      state: correctionForm.state,
      assessedAt: instantValue(correctionForm.assessedAt),
      method: correctionForm.method.trim() || undefined,
      notes: correctionForm.notes.trim() || undefined,
      projectId: projectId.value,
    })
    showCorrectRecord.value = false
    await loadRecords(genotypingAnimalId.value)
    message.success('已原子作废原记录并创建替代记录')
  } catch (error) {
    message.error(error instanceof Error ? error.message : '更正检测记录失败')
  } finally { busy.value = false }
}

async function createLine() {
  if (!lineForm.name.trim() || !lineForm.genotypeDefinitionIds.length) {
    return message.warning('请填写品系名称并关联基因型定义')
  }
  busy.value = true
  try {
    const line = await gateway.createBreedingLine({
      name: lineForm.name.trim(),
      description: lineForm.description.trim() || undefined,
      genotypeDefinitionIds: lineForm.genotypeDefinitionIds,
    })
    lines.value.push(line)
    colonyForm.breedingLineId = line.id
    Object.assign(lineForm, { name: '', description: '', genotypeDefinitionIds: [] })
    showCreateLine.value = false
    message.success('繁育品系已创建')
  } catch (error) {
    message.error(error instanceof Error ? error.message : '创建繁育品系失败')
  } finally { busy.value = false }
}

async function createColony() {
  if (!colonyForm.breedingLineId || !colonyForm.name.trim()) {
    return message.warning('请选择品系并填写 Colony 名称')
  }
  busy.value = true
  try {
    const colony = await gateway.createColony({
      breedingLineId: colonyForm.breedingLineId,
      name: colonyForm.name.trim(),
      description: colonyForm.description.trim() || undefined,
    })
    colonies.value.push(colony)
    pairForm.colonyId = colony.id
    Object.assign(colonyForm, { name: '', description: '' })
    showCreateColony.value = false
    message.success('Colony 已创建')
  } catch (error) {
    message.error(error instanceof Error ? error.message : '创建 Colony 失败')
  } finally { busy.value = false }
}

async function createPair() {
  if (!pairForm.colonyId || !pairForm.name.trim() || !pairForm.maleAnimalId
    || !pairForm.femaleAnimalIds.length) {
    return message.warning('配对必须选择一只雄鼠和至少一只雌鼠')
  }
  busy.value = true
  try {
    const pair = await gateway.createBreedingPair({
      projectId: projectId.value,
      colonyId: pairForm.colonyId,
      name: pairForm.name.trim(),
      maleAnimalId: pairForm.maleAnimalId,
      femaleAnimalIds: pairForm.femaleAnimalIds,
      startedAt: instantValue(pairForm.startedAt),
    })
    pairs.value.unshift(pair)
    showCreatePair.value = false
    Object.assign(pairForm, {
      name: '',
      maleAnimalId: null,
      femaleAnimalIds: [],
      startedAt: null,
    })
    await selectPair(pair.id)
    message.success('一雄多雌繁育配对已创建')
  } catch (error) {
    message.error(error instanceof Error ? error.message : '创建繁育配对失败')
  } finally { busy.value = false }
}

async function retirePair() {
  if (!selectedPair.value) return
  busy.value = true
  try {
    const updated = await gateway.retireBreedingPair({
      id: selectedPair.value.id,
      expectedRevision: selectedPair.value.revision,
    })
    const index = pairs.value.findIndex((pair) => pair.id === updated.id)
    if (index >= 0) pairs.value[index] = updated
    message.success('繁育配对已退役')
  } catch (error) {
    message.error(error instanceof Error ? error.message : '退役配对失败')
  } finally { busy.value = false }
}

function openMating() {
  matingForm.femaleAnimalId = pairFemaleOptions.value[0]?.value ?? null
  matingForm.occurredAt = Date.now()
  matingForm.notes = ''
  showCreateMating.value = true
}

async function createMating() {
  const pair = selectedPair.value
  const male = pair?.members.find((member) => member.role === 'male' && !member.leftAt)
  if (!pair || !male || !matingForm.femaleAnimalId) {
    return message.warning('当前配对没有可用的雄鼠或雌鼠')
  }
  busy.value = true
  try {
    const event = await gateway.createMatingEvent({
      projectId: projectId.value,
      breedingPairId: pair.id,
      maleAnimalId: male.animalId,
      femaleAnimalId: matingForm.femaleAnimalId,
      occurredAt: instantValue(matingForm.occurredAt),
      notes: matingForm.notes.trim() || undefined,
    })
    matingEvents.value.unshift(event)
    litterForm.matingEventId = event.id
    showCreateMating.value = false
    message.success('交配事件已记录')
  } catch (error) {
    message.error(error instanceof Error ? error.message : '记录交配事件失败')
  } finally { busy.value = false }
}

function addDraft() {
  litterForm.drafts.push({
    temporaryLabel: `P${litterForm.drafts.length + 1}`,
    sex: 'unknown',
  })
  litterForm.sizeTotal = Math.max(litterForm.sizeTotal, litterForm.drafts.length)
}

function removeDraft(index: number) {
  litterForm.drafts.splice(index, 1)
}

function openLitter() {
  Object.assign(litterForm, {
    matingEventId: matingEvents.value[0]?.id ?? null,
    bornOn: Date.now(),
    sizeTotal: 1,
    notes: '',
  })
  litterForm.drafts.splice(0, litterForm.drafts.length, {
    temporaryLabel: 'P1',
    sex: 'unknown',
  })
  showCreateLitter.value = true
}

async function createLitter() {
  const bornOn = dateValue(litterForm.bornOn)
  if (!litterForm.matingEventId || !bornOn
    || litterForm.sizeTotal < litterForm.drafts.length
    || litterForm.drafts.some((draft) => !draft.temporaryLabel.trim())) {
    return message.warning('请完整填写窝次，并确保总数不少于存活 Draft 数')
  }
  busy.value = true
  try {
    const created = await gateway.createLitter({
      matingEventId: litterForm.matingEventId,
      bornOn,
      sizeTotal: litterForm.sizeTotal,
      drafts: litterForm.drafts.map((draft) => ({
        temporaryLabel: draft.temporaryLabel.trim(),
        sex: draft.sex,
      })),
      notes: litterForm.notes.trim() || undefined,
    })
    litters.value.unshift(created.litter)
    showCreateLitter.value = false
    await selectLitter(created.litter.id)
    message.success('窝次与存活动物 Draft 已原子创建')
  } catch (error) {
    message.error(error instanceof Error ? error.message : '创建窝次失败')
  } finally { busy.value = false }
}

function openRegisterDraft(draft: AnimalDraft) {
  Object.assign(registerForm, {
    draftId: draft.id,
    displayId: draft.temporaryLabel,
    identifierScope: projectId.value ? 'project' : 'lab',
    strain: '',
  })
  showRegisterDraft.value = true
}

async function registerDraft() {
  const draft = drafts.value.find((item) => item.id === registerForm.draftId)
  if (!draft || !registerForm.displayId.trim()) return message.warning('请输入正式动物编号')
  busy.value = true
  try {
    const registered = await gateway.registerAnimalDraft({
      draftId: draft.id,
      expectedRevision: draft.revision,
      identifierScope: registerForm.identifierScope,
      projectId: registerForm.identifierScope === 'project' ? projectId.value : undefined,
      displayId: registerForm.displayId.trim(),
      strain: registerForm.strain.trim() || undefined,
    })
    const index = drafts.value.findIndex((item) => item.id === registered.draft.id)
    if (index >= 0) drafts.value[index] = registered.draft
    if (!animals.value.some((animal) => animal.id === registered.animal.id)) {
      animals.value.push(registered.animal)
    }
    showRegisterDraft.value = false
    message.success('Draft 已原子登记为 Animal，并生成双亲谱系')
  } catch (error) {
    message.error(error instanceof Error ? error.message : '登记 Animal Draft 失败')
  } finally { busy.value = false }
}

async function runPrediction() {
  if (!predictionForm.maleGenotypeDefinitionId
    || !predictionForm.femaleGenotypeDefinitionId) {
    return message.warning('请选择父本和母本基因型定义')
  }
  busy.value = true
  try {
    predictions.value = await gateway.predictBreeding({
      maleGenotypeDefinitionId: predictionForm.maleGenotypeDefinitionId,
      femaleGenotypeDefinitionId: predictionForm.femaleGenotypeDefinitionId,
    })
    message.success('已按确定性孟德尔规则计算')
  } catch (error) {
    message.error(error instanceof Error ? error.message : '遗传预测失败')
  } finally { busy.value = false }
}

watch(() => route.query.animal, (value) => {
  if (typeof value === 'string' && value !== selectedId.value
    && animals.value.some((animal) => animal.id === value)) {
    selectAnimal(value)
  }
})
watch(() => route.query.project_id, () => void load())
watch(() => relationForm.parentType, () => { relationForm.parentId = null })
watch(genotypingAnimalId, (value) => void loadRecords(value))
onMounted(load)
</script>

<template>
  <div class="page breeding-page">
    <PageHeader
      title="繁育与遗传"
      section="动物管理"
      description="从结构化基因型、品系、Colony、配对和窝次推进到正式动物登记；AI 只提供建议，不自动创建交配。"
    >
      <template #actions>
        <n-button
          v-if="activeTab === 'pedigree' && writeAllowed"
          type="primary"
          :disabled="!selected"
          @click="openCreateRelation"
        >
          <template #icon><Plus :size="17" /></template>
          登记父母关系
        </n-button>
      </template>
    </PageHeader>

    <n-tabs v-model:value="activeTab" type="line" animated class="workspace-tabs">
      <n-tab-pane name="pedigree" tab="谱系">
        <section class="breeding-layout">
          <aside class="animal-browser surface">
            <n-input v-model:value="search" clearable placeholder="搜索编号、品系或基因型">
              <template #prefix><Search :size="16" /></template>
            </n-input>
            <div class="animal-results" :aria-busy="loading">
              <n-skeleton v-if="loading" v-for="index in 5" :key="index" text :repeat="2" />
              <button
                v-for="animal in filteredAnimals"
                v-else
                :key="animal.id"
                type="button"
                class="animal-option"
                :class="{ selected: animal.id === selectedId }"
                @click="selectAnimal(animal.id)"
              >
                <span>
                  <strong>{{ animal.code }}</strong>
                  <small>{{ animal.strain }} · {{ animal.genotype }}</small>
                </span>
                <n-tag size="small" :bordered="false">{{ sexLabel(animal.sex) }}</n-tag>
              </button>
              <n-empty
                v-if="!loading && !filteredAnimals.length"
                size="small"
                description="没有匹配的动物"
              />
            </div>
          </aside>

          <section class="pedigree-panel surface">
            <n-empty v-if="!selected" description="请先选择一只动物查看谱系" />
            <template v-else>
              <header class="pedigree-heading">
                <div>
                  <span>当前动物</span>
                  <h2>{{ selected.code }}</h2>
                  <p>{{ sexLabel(selected.sex) }} · {{ selected.strain }} · {{ selected.genotype }}</p>
                </div>
                <div class="pedigree-counts">
                  <span>父母<strong>{{ parents.length }}</strong></span>
                  <span>后代<strong>{{ offspring.length }}</strong></span>
                </div>
              </header>

              <n-spin :show="detailLoading">
                <div class="relationship-section">
                  <div class="section-title">
                    <div><strong>父母</strong><span>父本、母本或尚未分类的父母关系</span></div>
                    <b>{{ parents.length }}</b>
                  </div>
                  <div v-if="parents.length" class="relation-grid">
                    <button
                      v-for="relation in parents"
                      :key="relation.id"
                      type="button"
                      class="relation-card"
                      @click="openRelated(relation)"
                    >
                      <n-tag size="small" type="info" :bordered="false">
                        {{ relationLabel(relation) }}
                      </n-tag>
                      <strong>{{ relation.relatedAnimal.code }}</strong>
                      <span>
                        {{ sexLabel(relation.relatedAnimal.sex) }} ·
                        {{ relation.relatedAnimal.strain || '未记录品系' }}
                      </span>
                      <small>revision {{ relation.revision }}</small>
                    </button>
                  </div>
                  <n-empty
                    v-else-if="!detailLoading"
                    size="small"
                    description="尚未登记父母关系"
                  />
                </div>

                <div class="relationship-section">
                  <div class="section-title">
                    <div><strong>后代</strong><span>由已登记谱系反向查询得到</span></div>
                    <b>{{ offspring.length }}</b>
                  </div>
                  <div v-if="offspring.length" class="relation-grid">
                    <button
                      v-for="relation in offspring"
                      :key="relation.id"
                      type="button"
                      class="relation-card"
                      @click="openRelated(relation)"
                    >
                      <n-tag size="small" type="success" :bordered="false">后代</n-tag>
                      <strong>{{ relation.relatedAnimal.code }}</strong>
                      <span>
                        {{ sexLabel(relation.relatedAnimal.sex) }} ·
                        {{ relation.relatedAnimal.strain || '未记录品系' }}
                      </span>
                      <small>
                        作为{{ relation.parentType === 'father'
                          ? '父本'
                          : relation.parentType === 'mother' ? '母本' : '父母' }}
                      </small>
                    </button>
                  </div>
                  <n-empty
                    v-else-if="!detailLoading"
                    size="small"
                    description="尚未查询到后代"
                  />
                </div>
              </n-spin>
            </template>
          </section>
        </section>
      </n-tab-pane>

      <n-tab-pane name="genetics" tab="基因定义与检测">
        <section class="workspace-grid">
          <article class="surface workspace-card">
            <header class="card-heading">
              <div><Dna :size="18" /><span><strong>结构化基因型定义</strong><small>支持多基因、Cre-lox 与条件性组件</small></span></div>
              <n-space size="small" align="center">
                <n-checkbox v-model:checked="showArchivedGenetics">显示已归档</n-checkbox>
                <template v-if="writeAllowed">
                  <n-button size="small" @click="showCreateLocus = true">新建位点</n-button>
                  <n-button size="small" @click="showCreateAllele = true">新建等位基因</n-button>
                  <n-button size="small" type="primary" @click="openDefinition">新建定义</n-button>
                </template>
              </n-space>
            </header>
            <div class="entity-list">
              <div
                v-for="definition in visibleDefinitions"
                :key="definition.id"
                class="entity-row"
                :class="{ archived: definition.archivedAt }"
              >
                <div>
                  <strong>{{ definition.name }}</strong>
                  <span>{{ genotypeComponentLabel(definition) }}</span>
                  <small>{{ definition.description || '无说明' }} · revision {{ definition.revision }}</small>
                </div>
                <div class="row-actions">
                  <n-tag
                    size="small"
                    :type="definition.archivedAt ? 'default' : 'info'"
                    :bordered="false"
                  >
                    {{ definition.archivedAt ? '已归档' : `${definition.components.length} 组件` }}
                  </n-tag>
                  <n-button
                    v-if="writeAllowed"
                    text
                    size="tiny"
                    :type="definition.archivedAt ? 'primary' : 'warning'"
                    @click="toggleDefinitionArchive(definition)"
                  >
                    {{ definition.archivedAt ? '恢复' : '归档' }}
                  </n-button>
                </div>
              </div>
              <n-empty v-if="!visibleDefinitions.length" description="尚未创建基因型定义" size="small" />
            </div>
            <h3 class="subheading">位点与 allele 目录</h3>
            <div class="catalog-list">
              <div
                v-for="locus in visibleLoci"
                :key="locus.id"
                class="catalog-locus"
                :class="{ archived: locus.archivedAt }"
              >
                <header>
                  <span><strong>{{ locus.symbol }}</strong><small>revision {{ locus.revision }}</small></span>
                  <n-space size="small">
                    <n-tag v-if="locus.archivedAt" size="tiny" :bordered="false">已归档</n-tag>
                    <n-button
                      v-if="writeAllowed"
                      text
                      size="tiny"
                      :type="locus.archivedAt ? 'primary' : 'warning'"
                      @click="toggleLocusArchive(locus)"
                    >
                      {{ locus.archivedAt ? '恢复位点' : '归档位点' }}
                    </n-button>
                  </n-space>
                </header>
                <div class="allele-chips">
                  <span
                    v-for="allele in alleles.filter((item) => item.locusId === locus.id
                      && (showArchivedGenetics || !item.archivedAt))"
                    :key="allele.id"
                    :class="{ archived: allele.archivedAt }"
                  >
                    {{ allele.symbol }}{{ allele.isWildType ? ' · WT' : '' }}
                    <n-button
                      v-if="writeAllowed"
                      text
                      size="tiny"
                      :type="allele.archivedAt ? 'primary' : 'warning'"
                      @click="toggleAlleleArchive(allele)"
                    >
                      {{ allele.archivedAt ? '恢复' : '归档' }}
                    </n-button>
                  </span>
                </div>
              </div>
            </div>
          </article>

          <article class="surface workspace-card">
            <header class="card-heading">
              <div><TestTube2 :size="18" /><span><strong>动物基因检测</strong><small>检测事实独立于 Animal 主记录</small></span></div>
              <n-button
                v-if="writeAllowed"
                size="small"
                type="primary"
                :disabled="!genotypingAnimalId || !activeDefinitions.length"
                @click="openRecord"
              >
                记录检测
              </n-button>
            </header>
            <n-form-item label="动物">
              <n-select
                v-model:value="genotypingAnimalId"
                filterable
                :options="animalOptions"
                placeholder="选择动物"
              />
            </n-form-item>
            <div class="entity-list">
              <div
                v-for="record in records"
                :key="record.id"
                class="entity-row"
                :class="{ voided: record.voidedAt }"
              >
                <div>
                  <strong>
                    {{ definitionLabels.get(record.genotypeDefinitionId) ?? record.genotypeDefinitionId }}
                  </strong>
                  <span>{{ record.method || '未记录方法' }} · {{ formatInstant(record.assessedAt) }}</span>
                  <small v-if="record.voidedAt">
                    作废于 {{ formatInstant(record.voidedAt) }} · {{ record.voidReason }} · revision {{ record.revision }}
                  </small>
                  <small v-else>
                    {{ record.notes || '无备注' }} · revision {{ record.revision }}
                    <template v-if="record.supersedesRecordId"> · 更正自 {{ record.supersedesRecordId.slice(0, 8) }}</template>
                  </small>
                </div>
                <div class="row-actions">
                  <n-tag v-if="currentRecordIds.has(record.id)" size="small" type="info" :bordered="false">当前</n-tag>
                  <n-tag
                    size="small"
                    :type="record.voidedAt ? 'default' : genotypeStateMeta[record.state].type"
                    :bordered="false"
                  >
                    {{ record.voidedAt ? '已作废' : genotypeStateMeta[record.state].label }}
                  </n-tag>
                  <n-space v-if="writeAllowed && !record.voidedAt" size="small">
                    <n-button text size="tiny" type="warning" @click="openCorrectRecord(record)">更正</n-button>
                    <n-button text size="tiny" type="error" @click="openVoidRecord(record)">作废</n-button>
                  </n-space>
                </div>
              </div>
              <n-empty v-if="!records.length" description="该动物尚无新式基因检测记录" size="small" />
            </div>
          </article>
        </section>
      </n-tab-pane>

      <n-tab-pane name="lines" tab="品系与 Colony">
        <section class="workspace-grid">
          <article class="surface workspace-card">
            <header class="card-heading">
              <div><Network :size="18" /><span><strong>繁育品系</strong><small>一个品系可关联多个基因型定义</small></span></div>
              <n-button
                v-if="writeAllowed"
                size="small"
                type="primary"
                :disabled="!definitions.length"
                @click="showCreateLine = true"
              >
                新建品系
              </n-button>
            </header>
            <div class="entity-list">
              <div v-for="line in lines" :key="line.id" class="entity-row">
                <div>
                  <strong>{{ line.name }}</strong>
                  <span>
                    {{ line.genotypeDefinitionIds.map((id) => definitionLabels.get(id) ?? id).join(' · ') }}
                  </span>
                  <small>{{ line.description || '无说明' }} · revision {{ line.revision }}</small>
                </div>
              </div>
              <n-empty v-if="!lines.length" description="尚未创建繁育品系" size="small" />
            </div>
          </article>
          <article class="surface workspace-card">
            <header class="card-heading">
              <div><HeartHandshake :size="18" /><span><strong>Colony</strong><small>按品系组织繁育群体</small></span></div>
              <n-button
                v-if="writeAllowed"
                size="small"
                type="primary"
                :disabled="!lines.length"
                @click="showCreateColony = true"
              >
                新建 Colony
              </n-button>
            </header>
            <div class="entity-list">
              <div v-for="colony in colonies" :key="colony.id" class="entity-row">
                <div>
                  <strong>{{ colony.name }}</strong>
                  <span>{{ lineLabels.get(colony.breedingLineId) ?? colony.breedingLineId }}</span>
                  <small>{{ colony.description || '无说明' }} · revision {{ colony.revision }}</small>
                </div>
              </div>
              <n-empty v-if="!colonies.length" description="尚未创建 Colony" size="small" />
            </div>
          </article>
        </section>
      </n-tab-pane>

      <n-tab-pane name="pairs" tab="配对与交配">
        <section class="master-detail">
          <article class="surface workspace-card pair-list">
            <header class="card-heading">
              <div><HeartHandshake :size="18" /><span><strong>繁育配对</strong><small>严格一雄多雌</small></span></div>
              <n-button
                v-if="writeAllowed"
                size="small"
                type="primary"
                :disabled="!colonies.length"
                @click="showCreatePair = true"
              >
                新建配对
              </n-button>
            </header>
            <button
              v-for="pair in pairs"
              :key="pair.id"
              type="button"
              class="select-row"
              :class="{ selected: pair.id === selectedPairId }"
              @click="selectPair(pair.id)"
            >
              <span>
                <strong>{{ pair.name }}</strong>
                <small>{{ colonyLabels.get(pair.colonyId) ?? pair.colonyId }} · {{ pair.members.length }} 只</small>
              </span>
              <n-tag
                size="small"
                :type="pair.status === 'active' ? 'success' : 'default'"
                :bordered="false"
              >
                {{ pair.status === 'active' ? '活跃' : '已退役' }}
              </n-tag>
            </button>
            <n-empty v-if="!pairs.length" description="尚未创建繁育配对" size="small" />
          </article>

          <article class="surface workspace-card">
            <n-empty v-if="!selectedPair" description="请选择一个繁育配对" />
            <template v-else>
              <header class="card-heading">
                <div>
                  <HeartHandshake :size="18" />
                  <span>
                    <strong>{{ selectedPair.name }}</strong>
                    <small>{{ colonyLabels.get(selectedPair.colonyId) }} · revision {{ selectedPair.revision }}</small>
                  </span>
                </div>
                <n-space v-if="writeAllowed && selectedPair.status === 'active'" size="small">
                  <n-button size="small" type="primary" @click="openMating">记录交配</n-button>
                  <n-popconfirm
                    positive-text="确认退役"
                    negative-text="返回"
                    @positive-click="retirePair"
                  >
                    <template #trigger>
                      <n-button size="small" type="warning" secondary :loading="busy">退役</n-button>
                    </template>
                    退役会同时关闭全部活跃成员关系，不能继续记录交配。
                  </n-popconfirm>
                </n-space>
              </header>
              <div class="member-grid">
                <div v-for="member in selectedPair.members" :key="member.id">
                  <n-tag size="small" :type="member.role === 'male' ? 'info' : 'error'" :bordered="false">
                    {{ member.role === 'male' ? '雄' : '雌' }}
                  </n-tag>
                  <strong>{{ animalLabels.get(member.animalId) ?? member.animalId }}</strong>
                  <small>{{ member.leftAt ? `离组 ${formatInstant(member.leftAt)}` : '活跃成员' }}</small>
                </div>
              </div>
              <h3 class="subheading">交配事件</h3>
              <div class="entity-list">
                <div v-for="event in matingEvents" :key="event.id" class="entity-row">
                  <div>
                    <strong>{{ formatInstant(event.occurredAt) }}</strong>
                    <span>
                      {{ animalLabels.get(event.maleAnimalId) }} ×
                      {{ animalLabels.get(event.femaleAnimalId) }}
                    </span>
                    <small>{{ event.notes || '无备注' }}</small>
                  </div>
                </div>
                <n-empty v-if="!matingEvents.length" description="尚未记录交配事件" size="small" />
              </div>
            </template>
          </article>
        </section>
      </n-tab-pane>

      <n-tab-pane name="litters" tab="窝次与 Draft">
        <section class="master-detail">
          <article class="surface workspace-card pair-list">
            <header class="card-heading">
              <div><Baby :size="18" /><span><strong>窝次</strong><small>先选择配对，再管理窝次与 Draft</small></span></div>
              <n-button
                v-if="writeAllowed"
                size="small"
                type="primary"
                :disabled="!selectedPair || !matingEvents.length"
                @click="openLitter"
              >
                新建窝次
              </n-button>
            </header>
            <n-form-item label="繁育配对">
              <n-select
                :value="selectedPairId"
                :options="pairs.map((pair) => ({ label: pair.name, value: pair.id }))"
                @update:value="selectPair"
              />
            </n-form-item>
            <button
              v-for="litter in litters"
              :key="litter.id"
              type="button"
              class="select-row"
              :class="{ selected: litter.id === selectedLitterId }"
              @click="selectLitter(litter.id)"
            >
              <span>
                <strong>{{ litter.bornOn }}</strong>
                <small>总数 {{ litter.sizeTotal }} · 存活 Draft {{ litter.sizeAlive }}</small>
              </span>
              <n-tag size="small" :bordered="false">revision {{ litter.revision }}</n-tag>
            </button>
            <n-empty v-if="!litters.length" description="当前配对尚无窝次" size="small" />
          </article>

          <article class="surface workspace-card">
            <n-empty v-if="!selectedLitter" description="请选择一个窝次" />
            <template v-else>
              <header class="card-heading">
                <div>
                  <Baby :size="18" />
                  <span>
                    <strong>{{ selectedLitter.bornOn }} 窝次</strong>
                    <small>
                      总数 {{ selectedLitter.sizeTotal }} · 存活 {{ selectedLitter.sizeAlive }} ·
                      {{ selectedLitter.notes || '无备注' }}
                    </small>
                  </span>
                </div>
              </header>
              <div class="draft-grid">
                <div v-for="draft in drafts" :key="draft.id" class="draft-card">
                  <div>
                    <strong>{{ draft.temporaryLabel }}</strong>
                    <span>{{ sexLabel(draft.sex) }} · {{ draft.birthDate }}</span>
                    <small>revision {{ draft.revision }}</small>
                  </div>
                  <n-tag
                    size="small"
                    :type="draft.status === 'registered' ? 'success' : 'warning'"
                    :bordered="false"
                  >
                    {{ draft.status === 'registered' ? '已登记' : draft.status === 'pending' ? '待登记' : '已丢弃' }}
                  </n-tag>
                  <n-button
                    v-if="animalRegistrationAllowed && draft.status === 'pending'"
                    size="tiny"
                    type="primary"
                    secondary
                    @click="openRegisterDraft(draft)"
                  >
                    登记为 Animal
                  </n-button>
                  <small v-else-if="draft.registeredAnimalId">
                    Animal {{ animalLabels.get(draft.registeredAnimalId) ?? draft.registeredAnimalId }}
                  </small>
                </div>
              </div>
              <n-empty v-if="!drafts.length" description="当前窝次没有存活动物 Draft" size="small" />
            </template>
          </article>
        </section>
      </n-tab-pane>

      <n-tab-pane name="prediction" tab="遗传预测">
        <article class="surface prediction-card">
          <header class="card-heading">
            <div><Calculator :size="18" /><span><strong>确定性孟德尔预测</strong><small>规则引擎计算，不调用模型，不创建繁育计划</small></span></div>
          </header>
          <n-alert type="info" :show-icon="false">
            预测仅根据结构化父本/母本定义计算概率；AI 可以解释与建议，但不能自动创建交配或修改动物。
          </n-alert>
          <div class="prediction-form">
            <n-form-item label="父本基因型定义" required>
              <n-select
                v-model:value="predictionForm.maleGenotypeDefinitionId"
                :options="definitionOptions"
                filterable
              />
            </n-form-item>
            <n-form-item label="母本基因型定义" required>
              <n-select
                v-model:value="predictionForm.femaleGenotypeDefinitionId"
                :options="definitionOptions"
                filterable
              />
            </n-form-item>
            <n-button type="primary" :loading="busy" @click="runPrediction">计算概率</n-button>
          </div>
          <div v-for="prediction in predictions" :key="prediction.locusId" class="prediction-locus">
            <h3>{{ locusLabels.get(prediction.locusId) ?? prediction.locusId }}</h3>
            <div class="outcome-grid">
              <div v-for="(outcome, index) in prediction.outcomes" :key="index">
                <strong>
                  {{ predictionAllele(outcome.paternalAlleleId) }} /
                  {{ predictionAllele(outcome.maternalAlleleId) }}
                </strong>
                <span>{{ (outcome.probability * 100).toFixed(1) }}%</span>
              </div>
            </div>
          </div>
          <n-empty v-if="!predictions.length" description="选择父本和母本定义后运行预测" />
        </article>
      </n-tab-pane>
    </n-tabs>

    <n-modal
      v-model:show="showCreateRelation"
      preset="card"
      title="登记父母关系"
      class="small-dialog"
      :bordered="false"
    >
      <n-alert type="info" :show-icon="false">
        关系将写入 {{ selected?.code }} 的谱系，并记录操作者、revision 与审计。
      </n-alert>
      <n-form label-placement="top">
        <n-form-item label="关系类型" required>
          <n-select v-model:value="relationForm.parentType" :options="[
            { label: '父本', value: 'father' },
            { label: '母本', value: 'mother' },
            { label: '父母（未分类）', value: 'unknown' },
          ]" />
        </n-form-item>
        <n-form-item label="选择动物" required>
          <n-select
            v-model:value="relationForm.parentId"
            filterable
            :options="animals.filter((animal) => animal.id !== selectedId)
              .filter((animal) => relationForm.parentType === 'father'
                ? animal.sex !== 'female'
                : relationForm.parentType === 'mother' ? animal.sex !== 'male' : true)
              .map((animal) => ({
                value: animal.id,
                label: `${animal.code} · ${sexLabel(animal.sex)} · ${animal.strain}`,
              }))"
            placeholder="按编号搜索"
          />
        </n-form-item>
      </n-form>
      <template #footer>
        <div class="dialog-actions">
          <n-button @click="showCreateRelation = false">取消</n-button>
          <n-button type="primary" :loading="busy" @click="createRelation">确认登记</n-button>
        </div>
      </template>
    </n-modal>

    <n-modal v-model:show="showCreateLocus" preset="card" title="新建基因位点" class="small-dialog">
      <n-form label-placement="top">
        <n-form-item label="位点符号" required><n-input v-model:value="locusForm.symbol" placeholder="例如 GeneA" /></n-form-item>
        <n-form-item label="说明"><n-input v-model:value="locusForm.description" /></n-form-item>
      </n-form>
      <template #footer><div class="dialog-actions"><n-button @click="showCreateLocus = false">取消</n-button><n-button type="primary" :loading="busy" @click="createLocus">创建</n-button></div></template>
    </n-modal>

    <n-modal v-model:show="showCreateAllele" preset="card" title="新建等位基因" class="small-dialog">
      <n-form label-placement="top">
        <n-form-item label="基因位点" required><n-select v-model:value="alleleForm.locusId" :options="locusOptions" filterable /></n-form-item>
        <n-form-item label="等位基因符号" required><n-input v-model:value="alleleForm.symbol" placeholder="例如 flox" /></n-form-item>
        <n-form-item label="说明"><n-input v-model:value="alleleForm.description" /></n-form-item>
        <n-form-item><n-checkbox v-model:checked="alleleForm.isWildType">野生型等位基因</n-checkbox></n-form-item>
      </n-form>
      <template #footer><div class="dialog-actions"><n-button @click="showCreateAllele = false">取消</n-button><n-button type="primary" :loading="busy" @click="createAllele">创建</n-button></div></template>
    </n-modal>

    <n-modal v-model:show="showCreateDefinition" preset="card" title="新建结构化基因型定义" class="wide-dialog">
      <n-form label-placement="top">
        <div class="form-grid">
          <n-form-item label="名称" required><n-input v-model:value="definitionForm.name" /></n-form-item>
          <n-form-item label="说明"><n-input v-model:value="definitionForm.description" /></n-form-item>
        </div>
        <div v-for="(component, index) in definitionForm.components" :key="index" class="component-editor">
          <strong>组件 {{ index + 1 }}</strong>
          <n-button v-if="definitionForm.components.length > 1" text type="error" size="tiny" @click="definitionForm.components.splice(index, 1)">移除</n-button>
          <n-form-item label="位点" required><n-select v-model:value="component.locusId" :options="locusOptions" filterable @update:value="resetDefinitionAlleles(index)" /></n-form-item>
          <n-form-item label="模式" required><n-select v-model:value="component.mode" :options="componentModeOptions" @update:value="normalizeDefinitionMode(index)" /></n-form-item>
          <n-form-item label="等位基因 1" required><n-select v-model:value="component.allele1Id" :options="alleleOptionsFor(component.locusId)" filterable /></n-form-item>
          <n-form-item :label="componentNeedsSecondAllele(component.mode) ? '等位基因 2' : '等位基因 2（不适用）'" :required="componentNeedsSecondAllele(component.mode)"><n-select v-model:value="component.allele2Id" :options="alleleOptionsFor(component.locusId)" :disabled="!componentNeedsSecondAllele(component.mode)" filterable /></n-form-item>
        </div>
        <n-button dashed block @click="addDefinitionComponent">添加基因组件</n-button>
      </n-form>
      <template #footer><div class="dialog-actions"><n-button @click="showCreateDefinition = false">取消</n-button><n-button type="primary" :loading="busy" @click="createDefinition">创建定义</n-button></div></template>
    </n-modal>

    <n-modal v-model:show="showCreateRecord" preset="card" title="记录基因检测" class="small-dialog">
      <n-form label-placement="top">
        <n-form-item label="基因型定义" required><n-select v-model:value="recordForm.genotypeDefinitionId" :options="definitionOptions" filterable /></n-form-item>
        <div class="form-grid">
          <n-form-item label="检测状态" required><n-select v-model:value="recordForm.state" :options="Object.entries(genotypeStateMeta).map(([value, meta]) => ({ value, label: meta.label }))" /></n-form-item>
          <n-form-item label="检测时间" :required="recordForm.state === 'confirmed' || recordForm.state === 'rejected'"><n-date-picker v-model:value="recordForm.assessedAt" type="datetime" clearable /></n-form-item>
        </div>
        <n-form-item label="检测方法"><n-input v-model:value="recordForm.method" placeholder="PCR / sequencing" /></n-form-item>
        <n-form-item label="备注"><n-input v-model:value="recordForm.notes" type="textarea" /></n-form-item>
      </n-form>
      <template #footer><div class="dialog-actions"><n-button @click="showCreateRecord = false">取消</n-button><n-button type="primary" :loading="busy" @click="createRecord">保存</n-button></div></template>
    </n-modal>

    <n-modal v-model:show="showVoidRecord" preset="card" title="作废基因检测记录" class="small-dialog">
      <n-alert type="warning" :show-icon="false">
        作废不会删除历史；该记录将退出当前基因型投影，但既有实验快照保持不变。
      </n-alert>
      <n-form label-placement="top">
        <n-form-item label="作废原因" required>
          <n-input v-model:value="voidReason" type="textarea" placeholder="说明为什么这条检测事实不再有效" />
        </n-form-item>
      </n-form>
      <template #footer>
        <div class="dialog-actions">
          <n-button @click="showVoidRecord = false">取消</n-button>
          <n-popconfirm
            positive-text="确认作废"
            negative-text="返回"
            :positive-button-props="{ type: 'error', loading: busy }"
            @positive-click="submitVoidRecord"
          >
            <template #trigger>
              <n-button type="error" :disabled="!voidReason.trim()">下一步确认</n-button>
            </template>
            确认作废该检测记录？提交后只能通过新记录更正，历史不会被删除。
          </n-popconfirm>
        </div>
      </template>
    </n-modal>

    <n-modal v-model:show="showCorrectRecord" preset="card" title="更正基因检测记录" class="small-dialog">
      <n-alert type="warning" :show-icon="false">
        提交将在同一事务中作废原记录，并创建一条显式指向原记录的替代记录。
      </n-alert>
      <n-form label-placement="top">
        <n-form-item label="更正原因" required>
          <n-input v-model:value="correctionForm.reason" type="textarea" />
        </n-form-item>
        <n-form-item label="替代基因型定义" required>
          <n-select v-model:value="correctionForm.genotypeDefinitionId" :options="definitionOptions" filterable />
        </n-form-item>
        <div class="form-grid">
          <n-form-item label="替代状态" required>
            <n-select v-model:value="correctionForm.state" :options="Object.entries(genotypeStateMeta).map(([value, meta]) => ({ value, label: meta.label }))" />
          </n-form-item>
          <n-form-item label="检测时间" :required="correctionForm.state === 'confirmed' || correctionForm.state === 'rejected'">
            <n-date-picker v-model:value="correctionForm.assessedAt" type="datetime" clearable />
          </n-form-item>
        </div>
        <n-form-item label="检测方法"><n-input v-model:value="correctionForm.method" /></n-form-item>
        <n-form-item label="备注"><n-input v-model:value="correctionForm.notes" type="textarea" /></n-form-item>
      </n-form>
      <template #footer>
        <div class="dialog-actions">
          <n-button @click="showCorrectRecord = false">取消</n-button>
          <n-popconfirm
            positive-text="确认更正"
            negative-text="返回"
            :positive-button-props="{ type: 'warning', loading: busy }"
            @positive-click="submitCorrectRecord"
          >
            <template #trigger>
              <n-button type="warning" :disabled="!correctionForm.reason.trim()">下一步确认</n-button>
            </template>
            确认原子作废原记录并创建替代记录？
          </n-popconfirm>
        </div>
      </template>
    </n-modal>

    <n-modal v-model:show="showCreateLine" preset="card" title="新建繁育品系" class="small-dialog">
      <n-form label-placement="top">
        <n-form-item label="品系名称" required><n-input v-model:value="lineForm.name" /></n-form-item>
        <n-form-item label="关联基因型定义" required><n-select v-model:value="lineForm.genotypeDefinitionIds" multiple filterable :options="definitionOptions" /></n-form-item>
        <n-form-item label="说明"><n-input v-model:value="lineForm.description" type="textarea" /></n-form-item>
      </n-form>
      <template #footer><div class="dialog-actions"><n-button @click="showCreateLine = false">取消</n-button><n-button type="primary" :loading="busy" @click="createLine">创建</n-button></div></template>
    </n-modal>

    <n-modal v-model:show="showCreateColony" preset="card" title="新建 Colony" class="small-dialog">
      <n-form label-placement="top">
        <n-form-item label="繁育品系" required><n-select v-model:value="colonyForm.breedingLineId" :options="lineOptions" /></n-form-item>
        <n-form-item label="Colony 名称" required><n-input v-model:value="colonyForm.name" /></n-form-item>
        <n-form-item label="说明"><n-input v-model:value="colonyForm.description" type="textarea" /></n-form-item>
      </n-form>
      <template #footer><div class="dialog-actions"><n-button @click="showCreateColony = false">取消</n-button><n-button type="primary" :loading="busy" @click="createColony">创建</n-button></div></template>
    </n-modal>

    <n-modal v-model:show="showCreatePair" preset="card" title="新建繁育配对" class="wide-dialog">
      <n-alert type="info" :show-icon="false">同一动物不能同时属于两个活跃配对；性别和成员角色由领域层再次校验。</n-alert>
      <n-form label-placement="top">
        <div class="form-grid">
          <n-form-item label="Colony" required><n-select v-model:value="pairForm.colonyId" :options="colonyOptions" filterable /></n-form-item>
          <n-form-item label="配对名称" required><n-input v-model:value="pairForm.name" /></n-form-item>
        </div>
        <div class="form-grid">
          <n-form-item label="雄鼠（恰好一只）" required><n-select v-model:value="pairForm.maleAnimalId" :options="maleOptions" filterable /></n-form-item>
          <n-form-item label="雌鼠（一只或多只）" required><n-select v-model:value="pairForm.femaleAnimalIds" multiple :options="femaleOptions" filterable /></n-form-item>
        </div>
        <n-form-item label="开始时间"><n-date-picker v-model:value="pairForm.startedAt" type="datetime" clearable /></n-form-item>
      </n-form>
      <template #footer><div class="dialog-actions"><n-button @click="showCreatePair = false">取消</n-button><n-button type="primary" :loading="busy" @click="createPair">创建配对</n-button></div></template>
    </n-modal>

    <n-modal v-model:show="showCreateMating" preset="card" title="记录交配事件" class="small-dialog">
      <n-form label-placement="top">
        <n-form-item label="雌鼠" required><n-select v-model:value="matingForm.femaleAnimalId" :options="pairFemaleOptions" /></n-form-item>
        <n-form-item label="发生时间"><n-date-picker v-model:value="matingForm.occurredAt" type="datetime" clearable /></n-form-item>
        <n-form-item label="备注"><n-input v-model:value="matingForm.notes" type="textarea" /></n-form-item>
      </n-form>
      <template #footer><div class="dialog-actions"><n-button @click="showCreateMating = false">取消</n-button><n-button type="primary" :loading="busy" @click="createMating">记录事件</n-button></div></template>
    </n-modal>

    <n-modal v-model:show="showCreateLitter" preset="card" title="创建窝次与 Animal Draft" class="wide-dialog">
      <n-form label-placement="top">
        <div class="form-grid">
          <n-form-item label="交配事件" required><n-select v-model:value="litterForm.matingEventId" :options="matingEventOptions" /></n-form-item>
          <n-form-item label="出生日期" required><n-date-picker v-model:value="litterForm.bornOn" type="date" /></n-form-item>
        </div>
        <n-form-item label="总仔数" required><n-input-number v-model:value="litterForm.sizeTotal" :min="0" /></n-form-item>
        <div class="draft-editor-heading"><strong>存活动物 Draft</strong><n-button size="tiny" @click="addDraft">添加 Draft</n-button></div>
        <div v-for="(draft, index) in litterForm.drafts" :key="index" class="draft-editor-row">
          <n-input v-model:value="draft.temporaryLabel" placeholder="临时标签" />
          <n-select v-model:value="draft.sex" :options="[{label:'雄',value:'male'},{label:'雌',value:'female'},{label:'未知',value:'unknown'}]" />
          <n-button text type="error" :disabled="litterForm.drafts.length === 1" @click="removeDraft(index)">移除</n-button>
        </div>
        <n-form-item label="备注"><n-input v-model:value="litterForm.notes" type="textarea" /></n-form-item>
      </n-form>
      <template #footer><div class="dialog-actions"><n-button @click="showCreateLitter = false">取消</n-button><n-button type="primary" :loading="busy" @click="createLitter">原子创建</n-button></div></template>
    </n-modal>

    <n-modal v-model:show="showRegisterDraft" preset="card" title="登记为正式 Animal" class="small-dialog">
      <n-alert type="warning" :show-icon="false">提交后将原子创建 Animal、父母双向谱系、生命周期事件、Audit 与 Provenance。</n-alert>
      <n-form label-placement="top">
        <n-form-item label="正式动物编号" required><n-input v-model:value="registerForm.displayId" /></n-form-item>
        <n-form-item label="编号范围"><n-radio-group v-model:value="registerForm.identifierScope"><n-radio value="lab">实验室</n-radio><n-radio value="project" :disabled="!projectId">当前项目</n-radio></n-radio-group></n-form-item>
        <n-form-item label="品系"><n-input v-model:value="registerForm.strain" /></n-form-item>
      </n-form>
      <template #footer><div class="dialog-actions"><n-button @click="showRegisterDraft = false">取消</n-button><n-button type="primary" :loading="busy" @click="registerDraft">确认登记</n-button></div></template>
    </n-modal>
  </div>
</template>

<style scoped>
.workspace-tabs { min-height: 430px; }
.breeding-layout { display: grid; grid-template-columns: minmax(240px, 300px) minmax(0, 1fr); gap: 12px; align-items: start; }
.animal-browser { position: sticky; top: 12px; padding: 11px; }
.animal-results { display: flex; min-height: 180px; max-height: calc(100vh - 290px); margin-top: 10px; overflow: auto; flex-direction: column; gap: 5px; }
.animal-option,.select-row { display: flex; width: 100%; align-items: center; justify-content: space-between; gap: 8px; padding: 9px 10px; border: 1px solid transparent; border-radius: 7px; background: transparent; text-align: left; cursor: pointer; transition: background-color 140ms ease, border-color 140ms ease; }
.animal-option:hover,.select-row:hover { background: var(--muri-surface-muted); }
.animal-option.selected,.select-row.selected { border-color: color-mix(in srgb, var(--muri-primary) 35%, white); background: color-mix(in srgb, var(--muri-primary) 8%, white); }
.animal-option > span,.select-row > span { display: flex; min-width: 0; flex-direction: column; }
.animal-option strong,.select-row strong { font-size: 13px; }
.animal-option small,.select-row small { overflow: hidden; margin-top: 2px; color: var(--muri-text-tertiary); font-size: 11px; text-overflow: ellipsis; white-space: nowrap; }
.pedigree-panel { min-height: 390px; padding: 16px; }
.pedigree-panel > :deep(.n-empty) { min-height: 355px; justify-content: center; }
.pedigree-heading { display: flex; align-items: flex-end; justify-content: space-between; gap: 14px; padding-bottom: 14px; border-bottom: 1px solid var(--muri-border); }
.pedigree-heading > div:first-child > span { color: var(--muri-text-tertiary); font-size: 11px; }
.pedigree-heading h2 { margin: 2px 0 0; font-size: 20px; }
.pedigree-heading p { margin: 3px 0 0; color: var(--muri-text-secondary); font-size: 12px; }
.pedigree-counts { display: flex; gap: 8px; }
.pedigree-counts span { display: flex; min-width: 62px; padding: 7px 9px; flex-direction: column; border-radius: 7px; background: var(--muri-surface-muted); color: var(--muri-text-tertiary); font-size: 11px; }
.pedigree-counts strong { color: var(--muri-text); font-size: 17px; }
.relationship-section { padding: 15px 0 5px; }
.relationship-section + .relationship-section { border-top: 1px solid var(--muri-border); }
.section-title { display: flex; align-items: center; justify-content: space-between; margin-bottom: 10px; }
.section-title > div { display: flex; flex-direction: column; }
.section-title span { margin-top: 2px; color: var(--muri-text-tertiary); font-size: 11px; }
.section-title b { display: grid; min-width: 24px; height: 24px; place-items: center; border-radius: 999px; background: var(--muri-surface-muted); font-size: 12px; }
.relation-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(180px, 1fr)); gap: 8px; }
.relation-card { display: flex; min-width: 0; padding: 11px; align-items: flex-start; flex-direction: column; border: 1px solid var(--muri-border); border-radius: 7px; background: white; text-align: left; cursor: pointer; transition: border-color 140ms ease, box-shadow 140ms ease; }
.relation-card:hover { border-color: var(--muri-border-strong); box-shadow: var(--muri-shadow-sm); }
.relation-card strong { margin-top: 8px; color: var(--muri-primary); }
.relation-card span { margin-top: 2px; color: var(--muri-text-secondary); font-size: 12px; }
.relation-card small { margin-top: 8px; color: var(--muri-text-tertiary); }
.workspace-grid,.master-detail { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 12px; align-items: start; }
.master-detail { grid-template-columns: minmax(270px, .72fr) minmax(0, 1.28fr); }
.workspace-card,.prediction-card { min-height: 330px; padding: 15px; }
.pair-list { display: flex; flex-direction: column; gap: 6px; }
.card-heading { display: flex; min-height: 38px; margin-bottom: 13px; align-items: flex-start; justify-content: space-between; gap: 10px; border-bottom: 1px solid var(--muri-border); padding-bottom: 11px; }
.card-heading > div { display: flex; min-width: 0; gap: 8px; }
.card-heading svg { flex: none; margin-top: 1px; color: var(--muri-primary); }
.card-heading span { display: flex; min-width: 0; flex-direction: column; }
.card-heading small { margin-top: 2px; color: var(--muri-text-tertiary); font-size: 11px; font-weight: 400; }
.entity-list { display: flex; flex-direction: column; gap: 7px; }
.entity-row { display: flex; padding: 10px; align-items: flex-start; justify-content: space-between; gap: 10px; border: 1px solid var(--muri-border); border-radius: 7px; }
.entity-row > div { display: flex; min-width: 0; flex-direction: column; }
.entity-row span { margin-top: 2px; color: var(--muri-text-secondary); font-size: 12px; overflow-wrap: anywhere; }
.entity-row small { margin-top: 5px; color: var(--muri-text-tertiary); font-size: 11px; }
.entity-row.archived,.entity-row.voided,.catalog-locus.archived,.allele-chips > span.archived { background: var(--muri-surface-muted); opacity: .72; }
.entity-row > .row-actions { align-items: flex-end; flex: none; gap: 5px; }
.catalog-list { display: flex; flex-direction: column; gap: 7px; }
.catalog-locus { padding: 9px 10px; border: 1px solid var(--muri-border); border-radius: 7px; }
.catalog-locus > header { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
.catalog-locus > header > span { display: flex; flex-direction: column; }
.catalog-locus small { color: var(--muri-text-tertiary); font-size: 10px; }
.allele-chips { display: flex; margin-top: 7px; flex-wrap: wrap; gap: 5px; }
.allele-chips > span { display: inline-flex; padding: 3px 6px; align-items: center; gap: 4px; border-radius: 5px; background: color-mix(in srgb, var(--muri-primary) 7%, white); color: var(--muri-text-secondary); font-size: 11px; }
.member-grid,.draft-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(155px, 1fr)); gap: 8px; }
.member-grid > div,.draft-card { display: flex; padding: 10px; align-items: flex-start; flex-direction: column; gap: 5px; border: 1px solid var(--muri-border); border-radius: 7px; }
.member-grid small,.draft-card small { color: var(--muri-text-tertiary); font-size: 11px; }
.draft-card { justify-content: space-between; }
.draft-card > div { display: flex; flex-direction: column; }
.draft-card span { color: var(--muri-text-secondary); font-size: 12px; }
.subheading { margin: 17px 0 9px; font-size: 13px; }
.prediction-card { max-width: 900px; }
.prediction-card > :deep(.n-alert) { margin-bottom: 14px; }
.prediction-form { display: grid; grid-template-columns: 1fr 1fr auto; align-items: end; gap: 12px; }
.prediction-locus { margin-top: 16px; }
.prediction-locus h3 { margin: 0 0 8px; font-size: 14px; }
.outcome-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(140px, 1fr)); gap: 8px; }
.outcome-grid > div { display: flex; padding: 10px; align-items: center; justify-content: space-between; border: 1px solid var(--muri-border); border-radius: 7px; }
.outcome-grid span { color: var(--muri-primary); font-weight: 600; }
.small-dialog { width: min(480px, calc(100vw - 28px)); }
.wide-dialog { width: min(760px, calc(100vw - 28px)); }
.small-dialog :deep(.n-alert),.wide-dialog :deep(.n-alert) { margin-bottom: 14px; }
.dialog-actions { display: flex; justify-content: flex-end; gap: 9px; }
.form-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; }
.component-editor { display: grid; grid-template-columns: 1fr 1fr; gap: 0 12px; margin-bottom: 10px; padding: 10px; border: 1px solid var(--muri-border); border-radius: 7px; }
.component-editor > strong { align-self: center; }
.component-editor > button { justify-self: end; }
.draft-editor-heading { display: flex; align-items: center; justify-content: space-between; margin-bottom: 8px; }
.draft-editor-row { display: grid; grid-template-columns: 1fr 160px auto; gap: 8px; margin-bottom: 8px; }
@media (max-width: 900px) {
  .breeding-layout,.workspace-grid,.master-detail { grid-template-columns: 1fr; }
  .animal-browser { position: static; }
  .animal-results { max-height: 260px; }
  .pedigree-panel { min-height: 320px; padding: 13px; }
  .pedigree-heading { align-items: flex-start; flex-direction: column; }
  .relation-grid { grid-template-columns: 1fr; }
  .prediction-form { grid-template-columns: 1fr; }
}
@media (max-width: 560px) {
  .form-grid,.component-editor { grid-template-columns: 1fr; gap: 0; }
  .draft-editor-row { grid-template-columns: 1fr; }
  .card-heading { align-items: stretch; flex-direction: column; }
}
@media (prefers-reduced-motion: reduce) {
  .animal-option,.select-row,.relation-card { transition: none; }
}
</style>
