<script setup lang="ts">
import { computed } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { zhCN, dateZhCN, type GlobalThemeOverrides } from 'naive-ui'
import { Bot, BookOpen, Boxes, Dna, FlaskConical, Images, Menu, Rat, ScrollText, Settings, TableProperties, Users } from '@lucide/vue'
import { branding } from './branding'
import { currentAuthSession, gateway } from '@/services/gateway'
import {
  availableProjects,
  currentProjectId,
  hasLabRegistryAccess,
  isLabAdmin as sessionIsLabAdmin,
  setCurrentProject,
} from '@/services/projectContext'
import AiDrawer from '@/components/AiDrawer.vue'

const route = useRoute()
const router = useRouter()
const themeOverrides: GlobalThemeOverrides = {
  common: {
    primaryColor: branding.primaryColor, primaryColorHover: '#0b70c7', primaryColorPressed: '#0a4e8a',
    infoColor: branding.primaryColor, successColor: '#38866b', warningColor: '#d98216', errorColor: '#c34b51',
    borderRadius: '8px', fontSize: '14px', textColorBase: '#1d2935', bodyColor: '#f4f6f8',
  },
  Button: { heightMedium: '36px', borderRadiusMedium: '7px' },
  Input: { heightMedium: '36px', borderRadius: '7px' },
  Card: { borderRadius: '8px' },
}

const labRegistryAvailable = computed(() => gateway.mode === 'local' || hasLabRegistryAccess())
const animalItems = computed(() => [
  ...(labRegistryAvailable.value ? [{ to: '/cages', label: '笼位视图', icon: Boxes }] : []),
  { to: '/animals', label: '小鼠档案', icon: Rat },
  ...(labRegistryAvailable.value ? [{ to: '/animal-data', label: '动物数据', icon: TableProperties }] : []),
  ...(labRegistryAvailable.value ? [{ to: '/breeding', label: '繁育管理', icon: Dna }] : []),
])
const isLabAdmin = computed(() => gateway.mode === 'remote' && sessionIsLabAdmin())
const mainItems = computed(() => [
  { to: '/experiments', label: '实验管理', icon: FlaskConical },
  { to: '/data', label: '数据中心', icon: TableProperties },
  { to: '/library', label: '项目资料库', icon: BookOpen },
  { to: '/operations', label: '操作与审计', icon: ScrollText },
  { to: '/ai', label: 'AI 助手', icon: Bot },
  { to: '/ai/images', label: '私人 AI 图片', icon: Images },
  ...(isLabAdmin.value ? [
    { to: '/members', label: '成员管理', icon: Users },
    { to: '/admin/ai', label: 'AI 管理', icon: Bot },
  ] : []),
])
const mobileItems = computed(() => labRegistryAvailable.value
  ? [
      { to: '/cages', label: '笼位', icon: Boxes },
      { to: '/animals', label: '小鼠', icon: Rat },
      { to: '/experiments', label: '实验', icon: FlaskConical },
      { to: '/ai', label: 'AI', icon: Bot },
      { to: '/settings', label: '更多', icon: Menu },
    ]
  : [
      { to: '/animals', label: '小鼠', icon: Rat },
      { to: '/experiments', label: '实验', icon: FlaskConical },
      { to: '/data', label: '数据', icon: TableProperties },
      { to: '/ai', label: 'AI', icon: Bot },
      { to: '/settings', label: '更多', icon: Menu },
    ])
const LAB_REGISTRY = '__lab_registry__'
const projectOptions = computed(() => [
  ...(labRegistryAvailable.value ? [{ label: '实验室 Animal Registry', value: LAB_REGISTRY }] : []),
  ...availableProjects.value.map((project) => ({ label: project.name, value: project.id })),
])
const selectedProjectValue = computed(() => currentProjectId.value ?? LAB_REGISTRY)
const pageTitle = computed(() => String(route.meta.title ?? branding.productName))
const operatorName = computed(() => gateway.mode === 'local'
  ? '本地'
  : (currentAuthSession.value?.user.displayName || '共享用户'))
const avatarInitial = computed(() => operatorName.value.trim().charAt(0) || '用')
const isActive = (to: string) => route.path === to || route.path.startsWith(`${to}/`)

async function changeProject(value: string) {
  const projectId = value === LAB_REGISTRY ? undefined : value
  setCurrentProject(projectId)
  const query = { ...route.query }
  delete query.animal
  if (projectId) query.project_id = projectId
  else delete query.project_id
  const path = projectId && (route.name === 'cages' || route.name === 'breeding')
    ? '/animals'
    : route.path
  await router.replace({ path, query })
}
</script>

<template>
  <n-config-provider :locale="zhCN" :date-locale="dateZhCN" :theme-overrides="themeOverrides">
    <n-message-provider><n-dialog-provider><n-notification-provider>
      <div v-if="route.meta.authShell" class="auth-shell"><router-view /></div>
      <div v-else class="app-shell">
        <aside class="sidebar desktop-only">
          <router-link to="/cages" class="brand" :aria-label="`${branding.productName} 笼位首页`">
            <img :src="branding.logoMarkPath" alt="" />
            <div><strong>{{ branding.productName }}</strong><span>Animal research manager</span></div>
          </router-link>

          <nav aria-label="主导航">
            <div class="nav-label">动物管理</div>
            <router-link v-for="item in animalItems" :key="item.to" :to="item.to" class="nav-item" :class="{ active: isActive(item.to) }">
              <component :is="item.icon" :size="18" /><span>{{ item.label }}</span>
            </router-link>
            <div class="nav-label nav-gap">工作区</div>
            <router-link v-for="item in mainItems" :key="item.to" :to="item.to" class="nav-item" :class="{ active: isActive(item.to) }">
              <component :is="item.icon" :size="18" /><span>{{ item.label }}</span>
            </router-link>
          </nav>

          <div class="sidebar-footer">
            <router-link to="/settings" class="nav-item" :class="{ active: isActive('/settings') }"><Settings :size="18" /><span>设置</span></router-link>
            <div class="mode-card"><span class="mode-dot" /> <div><strong>{{ gateway.displayName }}</strong><small>{{ gateway.mode === 'local' ? 'SQLite · 离线可用' : 'Server · 已安全连接' }}</small></div></div>
          </div>
        </aside>

        <div class="workspace">
          <header class="topbar">
            <div class="mobile-brand mobile-only"><img :src="branding.logoMarkPath" alt="" /><strong>{{ branding.productName }}</strong></div>
            <div class="desktop-only"><span class="section">{{ route.meta.section ?? branding.productName }}</span><strong>{{ pageTitle }}</strong></div>
            <div class="topbar-meta">
              <n-select
                v-if="gateway.mode === 'remote' && projectOptions.length"
                class="project-switcher"
                data-testid="project-switcher"
                :value="selectedProjectValue"
                :options="projectOptions"
                size="small"
                @update:value="changeProject"
              />
              <span class="connection"><i />{{ gateway.mode === 'local' ? '本地模式' : '共享模式' }}</span><span class="operator-name desktop-only">{{ operatorName }}</span><span class="avatar" :title="operatorName">{{ avatarInitial }}</span>
            </div>
          </header>

          <main class="main-content">
            <router-view v-slot="{ Component }">
              <transition name="route" mode="out-in"><component :is="Component" :key="`${route.path}:${currentProjectId ?? 'registry'}`" /></transition>
            </router-view>
          </main>
        </div>

        <nav class="bottom-nav mobile-only" aria-label="移动端主导航">
          <router-link v-for="item in mobileItems" :key="item.to" :to="item.to" :class="{ active: isActive(item.to) }">
            <component :is="item.icon" :size="20" /><span>{{ item.label }}</span>
          </router-link>
        </nav>
        <AiDrawer />
      </div>
    </n-notification-provider></n-dialog-provider></n-message-provider>
  </n-config-provider>
</template>

<style scoped>
.app-shell { display: flex; min-height: 100vh; background: var(--muri-bg); }
.auth-shell { min-height: 100vh; }
.sidebar { position: fixed; z-index: 20; inset: 0 auto 0 0; display: flex; width: var(--muri-sidebar-width); flex-direction: column; border-right: 1px solid var(--muri-border); background: #fff; }
.brand { display: flex; align-items: center; gap: 10px; height: 74px; padding: 13px 17px; border-bottom: 1px solid var(--muri-border); }
.brand img { width: 44px; height: 44px; object-fit: contain; }
.brand div { display: flex; min-width: 0; flex-direction: column; }
.brand strong { font-size: 20px; letter-spacing: -0.03em; }
.brand span { overflow: hidden; color: var(--muri-text-tertiary); font-size: 10px; text-overflow: ellipsis; white-space: nowrap; }
nav { padding: 13px 10px; }
.nav-label { padding: 7px 10px 6px; color: var(--muri-text-tertiary); font-size: 11px; font-weight: 600; letter-spacing: 0.08em; }
.nav-gap { margin-top: 8px; }
.nav-item { display: flex; align-items: center; gap: 11px; min-height: 40px; margin: 2px 0; padding: 0 11px; border-radius: 7px; color: var(--muri-text-secondary); font-weight: 500; transition: color var(--muri-transition-fast), background var(--muri-transition-fast); }
.nav-item:hover { color: var(--muri-primary); background: #f5f8fb; }
.nav-item.active { color: var(--muri-primary); background: var(--muri-primary-soft); }
.sidebar-footer { margin-top: auto; padding: 10px; border-top: 1px solid var(--muri-border); }
.mode-card { display: flex; align-items: center; gap: 9px; margin: 7px 4px 1px; padding: 9px; border-radius: 7px; background: var(--muri-surface-muted); }
.mode-dot, .connection i { width: 7px; height: 7px; border-radius: 50%; background: var(--muri-success); box-shadow: 0 0 0 3px rgba(56, 134, 107, 0.12); }
.mode-card div { display: flex; flex-direction: column; }
.mode-card strong { font-size: 12px; }
.mode-card small { color: var(--muri-text-tertiary); font-size: 10px; }
.workspace { display: flex; width: calc(100% - var(--muri-sidebar-width)); min-height: 100vh; margin-left: var(--muri-sidebar-width); flex-direction: column; }
.topbar { position: sticky; z-index: 15; top: 0; display: flex; height: var(--muri-topbar-height); align-items: center; justify-content: space-between; padding: 0 24px; border-bottom: 1px solid var(--muri-border); background: rgba(255, 255, 255, 0.94); backdrop-filter: blur(10px); }
.topbar > div { display: flex; align-items: center; gap: 9px; }
.topbar .section { color: var(--muri-text-tertiary); font-size: 12px; }
.topbar strong { font-size: 14px; }
.topbar-meta { gap: 16px !important; }
.project-switcher { width: min(260px, 30vw); }
.connection { display: flex; align-items: center; gap: 7px; color: var(--muri-text-secondary); font-size: 12px; }
.operator-name { max-width: 150px; overflow: hidden; color: var(--muri-text-secondary); font-size: 12px; text-overflow: ellipsis; white-space: nowrap; }
.connection i { display: inline-block; width: 6px; height: 6px; }
.avatar { display: grid; width: 30px; height: 30px; place-items: center; border-radius: 50%; color: #fff; background: var(--muri-primary); font-size: 12px; }
.main-content { flex: 1; min-width: 0; }
.bottom-nav { display: none; }
@media (max-width: 900px) {
  .workspace { width: 100%; margin-left: 0; }
  .topbar { height: 54px; padding: 0 14px; }
  .mobile-brand { gap: 7px !important; }
  .mobile-brand img { width: 30px; height: 30px; object-fit: contain; }
  .mobile-brand strong { font-size: 17px; }
  .connection { font-size: 11px; }
  .project-switcher { width: min(210px, 44vw); }
  .avatar { display: none; }
  .bottom-nav { position: fixed; z-index: 30; inset: auto 0 0; display: flex; height: calc(62px + env(safe-area-inset-bottom)); align-items: flex-start; justify-content: space-around; padding: 6px 5px env(safe-area-inset-bottom); border-top: 1px solid var(--muri-border); background: rgba(255,255,255,.97); box-shadow: 0 -6px 22px rgba(30,53,76,.06); backdrop-filter: blur(12px); }
  .bottom-nav a { display: flex; min-width: 52px; min-height: 48px; align-items: center; justify-content: center; flex-direction: column; gap: 2px; border-radius: 7px; color: var(--muri-text-tertiary); font-size: 10px; }
  .bottom-nav a.active { color: var(--muri-primary); background: var(--muri-primary-soft); }
}
@media (max-width: 430px) {
  .connection { display: none; }
  .project-switcher { width: min(200px, 55vw); }
}
</style>
