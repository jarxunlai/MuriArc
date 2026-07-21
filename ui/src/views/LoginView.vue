<script setup lang="ts">
import { computed, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { LockKeyhole, Mail } from '@lucide/vue'
import { branding } from '@/branding'
import { gateway } from '@/services/gateway'
import { hasLabRegistryAccess } from '@/services/projectContext'

const route = useRoute()
const router = useRouter()
const email = ref('')
const password = ref('')
const loading = ref(false)
const error = ref('')

const redirect = computed(() => {
  const value = route.query.redirect
  return typeof value === 'string'
    && value.startsWith('/')
    && !value.startsWith('//')
    && !value.startsWith('/login')
    ? value
    : '/cages'
})

async function submit() {
  if (!gateway.login || loading.value) return
  const normalizedEmail = email.value.trim()
  if (!normalizedEmail || !password.value) {
    error.value = '请输入邮箱和密码'
    password.value = ''
    return
  }
  loading.value = true
  error.value = ''
  try {
    const session = await gateway.login({ email: normalizedEmail, password: password.value })
    const target = redirect.value === '/cages' && !hasLabRegistryAccess(session)
      ? '/animals'
      : redirect.value
    await router.replace(session.user.mustChangePassword
      ? { name: 'change-password', query: { redirect: target } }
      : target)
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : '登录失败，请重试'
  } finally {
    password.value = ''
    loading.value = false
  }
}
</script>

<template>
  <main class="login-page">
    <section class="login-card surface" aria-labelledby="login-title">
      <div class="brand-block">
        <img :src="branding.logoMarkPath" alt="" />
        <div><strong>{{ branding.productName }}</strong><span>共享实验室 Server</span></div>
      </div>
      <div class="heading">
        <h1 id="login-title">登录实验室</h1>
        <p>使用实验室账号访问动物、实验与数据。</p>
      </div>
      <n-form label-placement="top" @submit.prevent="submit">
          <n-form-item label="邮箱">
            <n-input v-model:value="email" type="text" :input-props="{ autocomplete: 'username', name: 'email', inputmode: 'email' }" autofocus placeholder="name@example.org">
              <template #prefix><Mail :size="16" /></template>
            </n-input>
          </n-form-item>
          <n-form-item label="密码">
            <n-input v-model:value="password" type="password" :input-props="{ autocomplete: 'current-password', name: 'password' }" show-password-on="click" placeholder="输入密码">
              <template #prefix><LockKeyhole :size="16" /></template>
            </n-input>
          </n-form-item>
        <n-alert v-if="error" type="error" :bordered="false" class="login-error">{{ error }}</n-alert>
        <n-button type="primary" attr-type="submit" block :loading="loading">登录</n-button>
      </n-form>
      <p class="security-note">会话使用 HttpOnly Cookie 与 CSRF 保护；密码不会保存在浏览器本地存储中。</p>
    </section>
  </main>
</template>

<style scoped>
.login-page { display: grid; min-height: 100vh; padding: 24px; place-items: center; background: var(--muri-bg); }
.login-card { width: min(100%, 420px); padding: 28px; }
.brand-block { display: flex; align-items: center; gap: 11px; margin-bottom: 28px; }
.brand-block img { width: 48px; height: 48px; object-fit: contain; }
.brand-block div { display: flex; flex-direction: column; }
.brand-block strong { font-size: 21px; letter-spacing: -.03em; }
.brand-block span { color: var(--muri-text-tertiary); font-size: 11px; }
.heading { margin-bottom: 20px; }
.heading h1 { margin: 0 0 5px; font-size: 22px; }
.heading p { margin: 0; color: var(--muri-text-secondary); }
.login-error { margin: -2px 0 15px; }
.security-note { margin: 18px 0 0; color: var(--muri-text-tertiary); font-size: 11px; line-height: 1.6; }
@media (max-width: 520px) { .login-page { padding: 14px; }.login-card { padding: 22px 18px; } }
</style>
