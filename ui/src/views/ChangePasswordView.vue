<script setup lang="ts">
import { computed, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { KeyRound, LockKeyhole } from '@lucide/vue'
import { branding } from '@/branding'
import { currentAuthSession, gateway } from '@/services/gateway'
import { passwordPolicyError, passwordStrength } from '@/services/passwordStrength'

const route = useRoute()
const router = useRouter()
const currentPassword = ref('')
const newPassword = ref('')
const confirmation = ref('')
const loading = ref(false)
const loggingOut = ref(false)
const error = ref('')
const strength = computed(() => passwordStrength(newPassword.value))

const redirect = computed(() => {
  const value = route.query.redirect
  return typeof value === 'string'
    && value.startsWith('/')
    && !value.startsWith('//')
    && !value.startsWith('/login')
    && !value.startsWith('/change-password')
    ? value
    : '/cages'
})

function clearPasswords() {
  currentPassword.value = ''
  newPassword.value = ''
  confirmation.value = ''
}

async function submit() {
  if (!gateway.changePassword || loading.value) return
  error.value = ''
  const policyError = passwordPolicyError(newPassword.value)
  if (!currentPassword.value) error.value = '请输入当前临时密码'
  else if (policyError) error.value = policyError
  else if (newPassword.value !== confirmation.value) error.value = '两次输入的新密码不一致'
  else if (newPassword.value === currentPassword.value) error.value = '新密码必须与当前密码不同'
  if (error.value) {
    clearPasswords()
    return
  }

  loading.value = true
  try {
    await gateway.changePassword({
      currentPassword: currentPassword.value,
      newPassword: newPassword.value,
    })
    await router.replace(redirect.value)
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : '密码修改失败，请重试'
  } finally {
    clearPasswords()
    loading.value = false
  }
}

async function logout() {
  if (!gateway.logout || loggingOut.value) return
  loggingOut.value = true
  try {
    await gateway.logout()
    await router.replace('/login')
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : '退出失败，请重试'
  } finally {
    clearPasswords()
    loggingOut.value = false
  }
}
</script>

<template>
  <main class="password-page">
    <section class="password-card surface" aria-labelledby="password-title">
      <div class="brand-block">
        <img :src="branding.logoMarkPath" alt="" />
        <div><strong>{{ branding.productName }}</strong><span>共享实验室账号安全</span></div>
      </div>
      <div class="heading">
        <h1 id="password-title">首次登录需要修改密码</h1>
        <p>{{ currentAuthSession?.user.displayName || '当前账号' }}，完成修改前不会开放业务导航和 API。</p>
      </div>
      <n-form label-placement="top" @submit.prevent="submit">
        <n-form-item label="当前临时密码">
          <n-input v-model:value="currentPassword" type="password" show-password-on="click" :input-props="{ autocomplete: 'current-password' }" autofocus>
            <template #prefix><LockKeyhole :size="16" /></template>
          </n-input>
        </n-form-item>
        <n-form-item label="新密码">
          <n-input v-model:value="newPassword" type="password" show-password-on="click" :input-props="{ autocomplete: 'new-password' }" maxlength="1024">
            <template #prefix><KeyRound :size="16" /></template>
          </n-input>
        </n-form-item>
        <div class="strength-row"><span>建议强度：{{ strength.label }}</span><n-progress type="line" :show-indicator="false" :percentage="strength.percentage" :status="strength.status" /></div>
        <n-form-item label="确认新密码">
          <n-input v-model:value="confirmation" type="password" show-password-on="click" :input-props="{ autocomplete: 'new-password' }" maxlength="1024" />
        </n-form-item>
        <n-alert v-if="error" type="error" :bordered="false" class="form-alert">{{ error }}</n-alert>
        <n-alert type="info" :bordered="false" class="form-alert">只要求至少 8 个字符且不含控制字符；弱/中/强仅为建议，不会要求特定字符组合。</n-alert>
        <n-button type="primary" attr-type="submit" block :loading="loading">修改密码并继续</n-button>
      </n-form>
      <n-button text block class="logout-button" :loading="loggingOut" @click="logout">退出当前账号</n-button>
    </section>
  </main>
</template>

<style scoped>
.password-page { display: grid; min-height: 100vh; padding: 24px; place-items: center; background: var(--muri-bg); }
.password-card { width: min(100%, 440px); padding: 28px; }
.brand-block { display: flex; align-items: center; gap: 11px; margin-bottom: 26px; }.brand-block img { width: 48px; height: 48px; object-fit: contain; }.brand-block div { display: flex; flex-direction: column; }.brand-block strong { font-size: 21px; letter-spacing: -.03em; }.brand-block span { color: var(--muri-text-tertiary); font-size: 11px; }
.heading { margin-bottom: 20px; }.heading h1 { margin: 0 0 5px; font-size: 22px; }.heading p { margin: 0; color: var(--muri-text-secondary); line-height: 1.55; }
.strength-row { display: grid; grid-template-columns: auto minmax(100px, 1fr); align-items: center; gap: 12px; margin: -10px 0 14px; color: var(--muri-text-tertiary); font-size: 11px; }.strength-row .n-progress { width: 100%; }
.form-alert { margin: -2px 0 14px; }.logout-button { margin-top: 14px; }
@media (max-width: 520px) { .password-page { padding: 14px; }.password-card { padding: 22px 18px; } }
</style>
