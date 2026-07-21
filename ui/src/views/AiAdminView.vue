<script setup lang="ts">
import { onMounted,reactive,ref } from 'vue'
import { useMessage } from 'naive-ui'
import PageHeader from '@/components/PageHeader.vue'
import { gateway,type AiDiagnostics,type AiLabSettings } from '@/services/gateway'
const msg=useMessage(),loading=ref(false),diagnostics=ref<AiDiagnostics>(),settings=reactive<AiLabSettings>({enabled:true,customUrlApprovalRequired:true,configuredUserCount:0,enabledUserCount:0,visionUserCount:0,revision:0})
async function load(){loading.value=true;try{if(gateway.getAiDiagnostics)diagnostics.value=await gateway.getAiDiagnostics();if(gateway.getAiLabSettings)Object.assign(settings,await gateway.getAiLabSettings())}catch(e){msg.error(e instanceof Error?e.message:'无法读取 AI 管理状态')}finally{loading.value=false}}
async function save(){if(!gateway.saveAiLabSettings)return;loading.value=true;try{Object.assign(settings,await gateway.saveAiLabSettings({enabled:settings.enabled,customUrlApprovalRequired:settings.customUrlApprovalRequired}));msg.success('实验室 AI 策略已保存')}catch(e){msg.error(e instanceof Error?e.message:'保存失败')}finally{loading.value=false}}
onMounted(load)
</script>
<template><div class="page"><PageHeader title="AI 管理" description="LabAdmin 只查看总开关、状态摘要和 Provider 配置数量；永不读取用户 API Key、明文地址或冒充用户调用。"/>
<n-alert type="warning" :bordered="false">普通用户受实验室总开关限制；LabAdmin 的个人 AI 配置不受总开关限制。自定义地址仍受 Server allowlist 与批准策略约束。</n-alert>
<section class="cards"><article class="surface"><span>运行时</span><strong>{{diagnostics?.runtimeConfigured?'已配置':'未配置'}}</strong><small>master key 仅以“是否配置”呈现</small></article><article class="surface"><span>用户配置</span><strong>{{settings.enabledUserCount}} / {{settings.configuredUserCount}}</strong><small>已启用 / 已配置</small></article><article class="surface"><span>视觉用户</span><strong>{{settings.visionUserCount}}</strong><small>已启用 supports_vision</small></article><article class="surface"><span>URL allowlist</span><strong>{{diagnostics?.localAllowlistCount??0}} + {{diagnostics?.cloudAllowlistCount??0}}</strong><small>本地 + 云端条目数量</small></article></section>
<section class="policy surface"><div><strong>实验室 AI 总开关</strong><span>关闭后普通用户不能解析 Provider；LabAdmin 保留管理与个人使用能力。</span></div><n-switch v-model:value="settings.enabled"/><div><strong>自定义 URL 必须管理员批准</strong><span>预置 Provider 仍按 allowlist 精确匹配。</span></div><n-switch v-model:value="settings.customUrlApprovalRequired"/><n-button type="primary" :loading="loading" @click="save">保存策略</n-button></section>
<section class="boundary surface"><h3>明确边界</h3><p>不提供实验室共享 Key；每位用户的凭据独立加密。诊断接口不返回 Key、master key、模型名或完整 URL。高风险策略保持折叠，不开放空公网接口。</p></section>
</div></template>
<style scoped>.cards{display:grid;grid-template-columns:repeat(4,1fr);gap:10px;margin:14px 0}.cards article{display:flex;padding:16px;flex-direction:column}.cards strong{font-size:25px}.cards span,.cards small,.policy span,.boundary p{color:var(--muri-text-tertiary)}.policy{display:grid;grid-template-columns:1fr auto;gap:18px;padding:18px;align-items:center}.policy div{display:flex;flex-direction:column}.policy button{grid-column:1/-1;justify-self:start}.boundary{margin-top:12px;padding:16px}@media(max-width:800px){.cards{grid-template-columns:1fr 1fr}}@media(max-width:460px){.cards{grid-template-columns:1fr}.policy{grid-template-columns:1fr auto}}</style>
