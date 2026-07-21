<script setup lang="ts">
import { computed, onMounted, reactive, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { Database, UserRound } from '@lucide/vue'
import { branding } from '@/branding'
import type { WorkspaceSettings } from '@/domain/models'
import { gateway } from '@/services/gateway'
import { markLocalSpaceEntered } from '@/services/localWelcome'

const route = useRoute()
const router = useRouter()
const loading = ref(true)
const error = ref('')
const workspace = reactive<WorkspaceSettings>({
  labName: '本地实验室',
  operatorName: '本地操作者',
})

const redirect = computed(() => {
  const value = route.query.redirect
  return typeof value === 'string'
    && value.startsWith('/')
    && !value.startsWith('//')
    && !value.startsWith('/welcome')
    && !value.startsWith('/login')
    && !value.startsWith('/change-password')
    ? value
    : '/cages'
})

async function enter() {
  markLocalSpaceEntered()
  await router.replace(redirect.value)
}

onMounted(async () => {
  try {
    if (gateway.getWorkspaceSettings) {
      Object.assign(workspace, await gateway.getWorkspaceSettings())
    }
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : '无法读取本地工作空间信息'
  } finally {
    loading.value = false
  }
})
</script>

<template>
  <main class="welcome-page">
    <section class="welcome-card surface" aria-labelledby="welcome-title">
      <div class="brand-block">
        <img :src="branding.logoMarkPath" alt="" />
        <div><strong>{{ branding.productName }}</strong><span>Desktop · SQLite 本地空间</span></div>
      </div>
      <div class="heading">
        <h1 id="welcome-title">进入本地空间</h1>
        <p>确认本次使用的实验室与操作者信息。</p>
      </div>
      <div class="identity-list" :class="{ loading }">
        <div><Database :size="20" /><span>实验室</span><strong data-testid="local-lab-name">{{ workspace.labName }}</strong></div>
        <div><UserRound :size="20" /><span>操作者</span><strong data-testid="local-operator-name">{{ workspace.operatorName }}</strong></div>
      </div>
      <n-alert v-if="error" type="warning" :bordered="false" class="welcome-alert">{{ error }}；仍可进入并在设置中检查。</n-alert>
      <n-alert type="info" :bordered="false" class="welcome-alert">
        这是无密码的本地欢迎步骤，用于确认工作空间，不是安全锁。能够访问本机账号和数据目录的人仍可能访问本地数据。
      </n-alert>
      <n-button data-testid="enter-local-space" type="primary" block :loading="loading" @click="enter">进入本地空间</n-button>
    </section>
  </main>
</template>

<style scoped>
.welcome-page { display: grid; min-height: 100vh; padding: 24px; place-items: center; background: var(--muri-bg); }
.welcome-card { width: min(100%, 460px); padding: 28px; }
.brand-block { display: flex; align-items: center; gap: 11px; margin-bottom: 28px; }
.brand-block img { width: 48px; height: 48px; object-fit: contain; }
.brand-block div { display: flex; flex-direction: column; }
.brand-block strong { font-size: 21px; letter-spacing: -.03em; }
.brand-block span { color: var(--muri-text-tertiary); font-size: 11px; }
.heading { margin-bottom: 18px; }.heading h1 { margin: 0 0 5px; font-size: 22px; }.heading p { margin: 0; color: var(--muri-text-secondary); }
.identity-list { display: grid; gap: 8px; margin-bottom: 15px; transition: opacity var(--muri-transition-fast); }.identity-list.loading { opacity: .5; }
.identity-list > div { display: grid; grid-template-columns: 28px 82px minmax(0, 1fr); align-items: center; padding: 12px; border: 1px solid var(--muri-border); border-radius: 7px; }.identity-list svg { color: var(--muri-primary); }.identity-list span { color: var(--muri-text-tertiary); font-size: 11px; }.identity-list strong { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.welcome-alert { margin-bottom: 14px; }
@media (max-width: 520px) { .welcome-page { padding: 14px; }.welcome-card { padding: 22px 18px; }.identity-list > div { grid-template-columns: 26px 68px minmax(0, 1fr); } }
</style>
