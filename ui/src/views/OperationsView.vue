<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { useMessage } from 'naive-ui'
import PageHeader from '@/components/PageHeader.vue'
import { gateway, type OperationRecord } from '@/services/gateway'
const message=useMessage(),loading=ref(false),items=ref<OperationRecord[]>([])
const scope=ref<string|null>(null),source=ref<string|null>(null),code=ref('')
const scopes=[{label:'全部范围',value:null},{label:'项目',value:'project'},{label:'实验',value:'experiment'},{label:'动物',value:'animal'},{label:'用户',value:'user'},{label:'AI',value:'ai'}]
const sources=[{label:'全部来源',value:null},{label:'Web',value:'web'},{label:'桌面端',value:'desktop'},{label:'API',value:'api'},{label:'AI',value:'ai'},{label:'MCP',value:'mcp'},{label:'迁移',value:'migration'}]
async function load(){if(!gateway.listOperations)return;loading.value=true;try{const q=new URLSearchParams();if(scope.value)q.set('scope',scope.value);if(source.value)q.set('source',source.value);if(code.value.trim())q.set('operation_code',code.value.trim());items.value=await gateway.listOperations(q)}catch(e){message.error(e instanceof Error?e.message:'无法读取审计')}finally{loading.value=false}}
function date(v:string){return new Intl.DateTimeFormat('zh-CN',{dateStyle:'short',timeStyle:'medium'}).format(new Date(v))}
onMounted(load)
</script>
<template><div class="page"><PageHeader title="操作与审计" description="确定性中文事件目录；保留 UUID、actor、source、request_id、revision 与前后差异。"/>
<section class="filters surface"><n-select v-model:value="scope" :options="scopes"/><n-select v-model:value="source" :options="sources"/><n-input v-model:value="code" placeholder="operation code"/><n-button type="primary" :loading="loading" @click="load">筛选</n-button></section>
<section class="surface table desktop-only"><n-data-table :loading="loading" :data="items" :row-key="(r:OperationRecord)=>r.id" :columns="[
{title:'时间',key:'occurredAt',render:(r:OperationRecord)=>date(r.occurredAt)},
{title:'操作',key:'title'},{title:'摘要',key:'summary'},{title:'operation code',key:'operationCode'},
{title:'操作者',key:'actor',render:(r:OperationRecord)=>r.actor.display_name},{title:'来源',key:'source'}]"/></section>
<section class="cards mobile-only"><article v-for="r in items" :key="r.id" class="surface"><header><strong>{{r.title}}</strong><n-tag size="small" :bordered="false">{{r.source}}</n-tag></header><p>{{r.summary}}</p><code>{{r.operationCode}} · {{r.entityId}}</code><small>{{date(r.occurredAt)}} · revision {{r.entityRevision??'—'}}</small></article><n-empty v-if="!loading&&!items.length" description="没有匹配操作"/></section>
</div></template>
<style scoped>.filters{display:grid;grid-template-columns:160px 160px minmax(200px,1fr) auto;gap:10px;padding:12px;margin-bottom:12px}.table{overflow:hidden}.cards{display:none;gap:9px}.cards article{padding:13px}.cards header{display:flex;justify-content:space-between}.cards p{margin:8px 0;color:var(--muri-text-secondary)}.cards code,.cards small{display:block;overflow-wrap:anywhere;color:var(--muri-text-tertiary);font-size:11px}.cards small{margin-top:7px}@media(max-width:900px){.filters{grid-template-columns:1fr 1fr}.cards{display:grid}}</style>
