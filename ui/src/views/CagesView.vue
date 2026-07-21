<script setup lang="ts">
import { computed, onMounted, reactive, ref } from 'vue'
import { useMessage } from 'naive-ui'
import { Plus, RefreshCw, Search, UsersRound } from '@lucide/vue'
import { useRoute, useRouter } from 'vue-router'
import type { Animal, Cage } from '@/domain/models'
import { gateway } from '@/services/gateway'
import PageHeader from '@/components/PageHeader.vue'

const message = useMessage()
const route = useRoute()
const router = useRouter()
const cages = ref<Cage[]>([])
const animals = ref<Animal[]>([])
const loading = ref(true)
const search = ref('')
const room = ref<string | null>(null)
const attentionOnly = ref(false)
const selectedAnimalIds = ref<string[]>([])
const showAdd = ref(false)
const showMove = ref(false)
const moveTarget = ref<string | null>(null)
const busy = ref(false)
const highlightedCage = ref<string | null>(null)
const newCage = reactive({ code: '', room: 'SPF-A', rack: 'R1', capacity: 5 })

const animalById = computed(() => new Map(animals.value.map((animal) => [animal.id, animal])))
const roomOptions = computed(() => [...new Set(cages.value.map((cage) => cage.room))].map((value) => ({ label: value, value })))
const targetOptions = computed(() => cages.value.map((cage) => ({ label: `${cage.code} · ${cage.animalIds.length}/${cage.capacity}`, value: cage.id, disabled: cage.animalIds.length >= cage.capacity })))
const filteredCages = computed(() => {
  const query = search.value.trim().toLowerCase()
  return cages.value.filter((cage) => {
    const cageAnimals = cage.animalIds.map((id) => animalById.value.get(id))
    const matchesQuery = !query || cage.code.toLowerCase().includes(query) || cage.summary.toLowerCase().includes(query)
      || cageAnimals.some((animal) => animal?.code.toLowerCase().includes(query) || animal?.genotype.toLowerCase().includes(query))
    return matchesQuery
      && (!room.value || cage.room === room.value)
      && (!attentionOnly.value || cage.status === 'attention')
  })
})
const attentionCount = computed(() => cages.value.filter((cage) => cage.status === 'attention').length)
const totalAnimals = computed(() => cages.value.reduce((sum, cage) => sum + cage.animalIds.length, 0))

async function load() {
  loading.value = true
  try {
    ;[cages.value, animals.value] = await Promise.all([gateway.listCages(), gateway.listAnimals()])
    const focus = route.query.focus
    if (typeof focus === 'string') {
      highlightedCage.value = focus
      window.setTimeout(() => { highlightedCage.value = null }, 1800)
    }
  } finally { loading.value = false }
}

function animalLabel(id: string) { return animalById.value.get(id)?.code ?? id }
function openAnimal(id: string) { void router.push({ path: '/animals', query: { animal: id } }) }

function onDragStart(event: DragEvent, animalId: string) {
  event.dataTransfer?.setData('text/muriarc-animal', animalId)
  if (event.dataTransfer) event.dataTransfer.effectAllowed = 'move'
}

async function onDrop(event: DragEvent, cageId: string) {
  const animalId = event.dataTransfer?.getData('text/muriarc-animal')
  if (!animalId) return
  await move([animalId], cageId)
}

async function move(ids = selectedAnimalIds.value, target = moveTarget.value) {
  if (!target || !ids.length) return
  busy.value = true
  try {
    await gateway.moveAnimals(ids, target)
    const targetCode = cages.value.find((cage) => cage.id === target)?.code
    selectedAnimalIds.value = []
    showMove.value = false
    await load()
    highlightedCage.value = target
    message.success(`已移动至笼位 ${targetCode ?? ''}`)
    window.setTimeout(() => { highlightedCage.value = null }, 1000)
  } catch (error) { message.error(error instanceof Error ? error.message : '移动失败') }
  finally { busy.value = false }
}

async function createCage() {
  if (!newCage.code.trim()) { message.warning('请输入笼位编号'); return }
  busy.value = true
  try {
    await gateway.createCage({ ...newCage, code: newCage.code.trim().toUpperCase() })
    showAdd.value = false
    newCage.code = ''
    await load()
    message.success('笼位已创建')
  } catch (error) { message.error(error instanceof Error ? error.message : '创建失败') }
  finally { busy.value = false }
}

onMounted(load)
</script>

<template>
  <div class="page cages-page">
    <PageHeader title="笼位视图" section="动物管理" description="按房间与笼架查看动物，桌面端可直接拖放转笼。">
      <template #actions>
        <n-button type="primary" @click="showAdd = true"><template #icon><Plus :size="17" /></template>新增笼位</n-button>
        <n-button secondary :loading="loading" @click="load"><template #icon><RefreshCw :size="16" /></template>刷新</n-button>
      </template>
    </PageHeader>

    <div v-if="attentionCount" class="attention-bar">
      <span><strong>{{ attentionCount }}</strong> 个笼位需要关注</span>
      <span>·</span><span>当前共 {{ totalAnimals }} 只动物</span>
      <button type="button" @click="attentionOnly = !attentionOnly">{{ attentionOnly ? '显示全部' : '仅看需关注' }}</button>
    </div>

    <section class="toolbar surface">
      <n-input v-model:value="search" clearable placeholder="搜索小鼠编号、笼位或基因型">
        <template #prefix><Search :size="16" /></template>
      </n-input>
      <n-select v-model:value="room" clearable :options="roomOptions" placeholder="全部区域" />
      <span class="result-count">{{ filteredCages.length }} 个笼位{{ attentionOnly ? ' · 仅需关注' : '' }}</span>
    </section>

    <n-spin :show="loading">
      <div class="cage-grid">
        <article
          v-for="cage in filteredCages" :key="cage.id" class="cage-card surface"
          :class="[`status-${cage.status}`, { highlighted: highlightedCage === cage.id }]"
          @dragover.prevent @drop.prevent="onDrop($event, cage.id)"
        >
          <header>
            <div><span class="cage-code">{{ cage.code }}</span><small>{{ cage.room }} · {{ cage.rack }}</small></div>
            <n-tag :type="cage.status === 'attention' ? 'warning' : cage.status === 'empty' ? 'default' : 'success'" size="small" round :bordered="false">
              {{ cage.status === 'attention' ? '需关注' : cage.status === 'empty' ? '空笼' : '正常' }}
            </n-tag>
          </header>
          <div class="capacity-row"><span>{{ cage.summary }}</span><strong>{{ cage.animalIds.length }} / {{ cage.capacity }}</strong></div>
          <n-progress type="line" :percentage="Math.round(cage.animalIds.length / cage.capacity * 100)" :show-indicator="false" :height="5" :color="cage.status === 'attention' ? '#d98216' : '#0f5faa'" />

          <div v-if="cage.animalIds.length" class="animal-list">
            <button
              v-for="animalId in cage.animalIds" :key="animalId" type="button" class="animal-chip"
              draggable="true" @dragstart="onDragStart($event, animalId)" @click="openAnimal(animalId)"
            >
              <n-checkbox :checked="selectedAnimalIds.includes(animalId)" @click.stop @update:checked="(checked: boolean) => selectedAnimalIds = checked ? [...selectedAnimalIds, animalId] : selectedAnimalIds.filter((id) => id !== animalId)" />
              <span>{{ animalLabel(animalId) }}</span>
              <small>{{ animalById.get(animalId)?.sex === 'male' ? '♂' : '♀' }} · {{ animalById.get(animalId)?.genotype }}</small>
            </button>
          </div>
          <div v-else class="empty-cage"><UsersRound :size="22" /><span>可接收动物</span></div>
          <footer v-if="cage.note"><span>{{ cage.note }}</span></footer>
        </article>
      </div>
    </n-spin>

    <n-empty v-if="!loading && !filteredCages.length" description="没有找到匹配的笼位" class="empty-result"><n-button @click="search = ''; room = null">清除筛选</n-button></n-empty>

    <transition name="selection">
      <div v-if="selectedAnimalIds.length" class="selection-bar">
        <span>已选择 <strong>{{ selectedAnimalIds.length }}</strong> 只小鼠</span>
        <n-button quaternary size="small" @click="selectedAnimalIds = []">取消</n-button>
        <n-button type="primary" size="small" @click="showMove = true">移动到笼位</n-button>
      </div>
    </transition>

    <n-modal v-model:show="showAdd" preset="card" title="新增笼位" class="dialog-card" :bordered="false">
      <n-form label-placement="top">
        <n-form-item label="笼位编号" required><n-input v-model:value="newCage.code" placeholder="例如 A04" /></n-form-item>
        <div class="form-grid"><n-form-item label="区域"><n-input v-model:value="newCage.room" /></n-form-item><n-form-item label="笼架"><n-input v-model:value="newCage.rack" /></n-form-item></div>
        <n-form-item label="建议容量"><n-input-number v-model:value="newCage.capacity" :min="1" :max="20" /></n-form-item>
      </n-form>
      <template #footer><div class="dialog-actions"><n-button @click="showAdd = false">取消</n-button><n-button type="primary" :loading="busy" @click="createCage">创建笼位</n-button></div></template>
    </n-modal>

    <n-modal v-model:show="showMove" preset="card" title="移动小鼠" class="dialog-card" :bordered="false">
      <p class="move-copy">将 {{ selectedAnimalIds.length }} 只小鼠移动到：</p>
      <n-select v-model:value="moveTarget" filterable :options="targetOptions" aria-label="目标笼位" placeholder="选择目标笼位" />
      <template #footer><div class="dialog-actions"><n-button @click="showMove = false">取消</n-button><n-button type="primary" :disabled="!moveTarget" :loading="busy" @click="move()">确认移动</n-button></div></template>
    </n-modal>
  </div>
</template>

<style scoped>
.attention-bar { display: flex; align-items: center; gap: 7px; min-height: 38px; margin-bottom: 12px; padding: 8px 13px; border: 1px solid #efd6ad; border-radius: var(--muri-radius); color: #79511a; background: #fff9ee; font-size: 13px; }
.attention-bar button { margin-left: auto; border: 0; color: #9a6217; background: transparent; cursor: pointer; font-weight: 600; }
.toolbar { display: grid; grid-template-columns: minmax(260px, 430px) 180px 1fr; align-items: center; gap: 10px; margin-bottom: 14px; padding: 10px; }
.result-count { justify-self: end; padding-right: 4px; color: var(--muri-text-tertiary); font-size: 12px; }
.cage-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(285px, 1fr)); gap: 13px; min-height: 180px; }
.cage-card { position: relative; min-height: 230px; padding: 15px; overflow: hidden; transition: border-color var(--muri-transition-fast), box-shadow var(--muri-transition-fast), transform var(--muri-transition-fast); }
.cage-card:hover { border-color: var(--muri-border-strong); box-shadow: var(--muri-shadow); transform: translateY(-1px); }
.cage-card::before { position: absolute; inset: 0 auto 0 0; width: 3px; background: var(--muri-primary); content: ''; }
.cage-card.status-attention::before { background: var(--muri-warning); }
.cage-card.status-empty::before { background: var(--muri-border-strong); }
.cage-card.highlighted { border-color: var(--muri-primary); box-shadow: 0 0 0 3px rgba(15,95,170,.14); }
.cage-card header { display: flex; align-items: flex-start; justify-content: space-between; }
.cage-card header > div { display: flex; flex-direction: column; }
.cage-code { font-size: 18px; font-weight: 700; }
.cage-card header small { margin-top: 2px; color: var(--muri-text-tertiary); }
.capacity-row { display: flex; justify-content: space-between; gap: 8px; margin: 14px 0 7px; color: var(--muri-text-secondary); font-size: 12px; }
.capacity-row strong { color: var(--muri-text); }
.animal-list { display: grid; grid-template-columns: 1fr 1fr; gap: 6px; margin-top: 12px; }
.animal-chip { display: grid; grid-template-columns: 18px 1fr; column-gap: 5px; min-width: 0; padding: 7px; text-align: left; border: 1px solid var(--muri-border); border-radius: 6px; background: var(--muri-surface-muted); cursor: grab; transition: border-color var(--muri-transition-fast), background var(--muri-transition-fast); }
.animal-chip:hover { border-color: #a9c8e1; background: var(--muri-primary-soft); }
.animal-chip:active { cursor: grabbing; }
.animal-chip > span { overflow: hidden; font-size: 12px; font-weight: 600; text-overflow: ellipsis; }
.animal-chip small { grid-column: 2; overflow: hidden; color: var(--muri-text-tertiary); font-size: 10px; text-overflow: ellipsis; white-space: nowrap; }
.empty-cage { display: flex; height: 88px; align-items: center; justify-content: center; flex-direction: column; gap: 5px; margin-top: 12px; border: 1px dashed var(--muri-border-strong); border-radius: 7px; color: var(--muri-text-tertiary); }
.cage-card footer { margin: 12px -15px -15px; padding: 8px 15px; color: #8a5c1e; background: #fff9ee; font-size: 11px; }
.empty-result { padding: 70px 0; }
.selection-bar { position: fixed; z-index: 40; inset: auto 24px 22px calc(var(--muri-sidebar-width) + 24px); display: flex; width: fit-content; max-width: calc(100% - var(--muri-sidebar-width) - 48px); margin: auto; align-items: center; gap: 9px; padding: 9px 10px 9px 15px; border: 1px solid var(--muri-border-strong); border-radius: 10px; background: white; box-shadow: var(--muri-shadow); }
.selection-enter-active,.selection-leave-active { transition: opacity var(--muri-transition-panel), transform var(--muri-transition-panel); }
.selection-enter-from,.selection-leave-to { opacity: 0; transform: translateY(8px); }
.dialog-card { width: min(480px, calc(100vw - 28px)); }
.form-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; }
.dialog-actions { display: flex; justify-content: flex-end; gap: 9px; }
.move-copy { margin: 0 0 12px; color: var(--muri-text-secondary); }
@media (max-width: 900px) {
  .toolbar { grid-template-columns: 1fr 130px; }
  .result-count { display: none; }
  .attention-bar { flex-wrap: wrap; }
  .attention-bar button { width: 100%; margin-left: 0; text-align: left; }
  .cage-grid { grid-template-columns: 1fr; }
  .cage-card:hover { transform: none; }
  .animal-chip { cursor: pointer; }
  .selection-bar { inset: auto 12px 73px; width: calc(100% - 24px); max-width: none; justify-content: flex-end; }
  .selection-bar > span { margin-right: auto; }
}
</style>
