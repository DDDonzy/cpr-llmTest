<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { ZButton, ZCard, ZCheckbox, ZScrollbar, ZEmpty, ZDialog, ZForm, ZFormItem, ZInput, ZInputNumber, ZPagination, ZSelect, ZTable, ZTag, ZTextarea, ZNotification, type TableColumn } from '@codex-proxy/ui'
import { Copy, Download, Pause, Play, Plus, Save, Settings2, X } from '@lucide/vue'
import { api, ApiError, type Account, type Job, type Key, type Plan, type Preset, type Presets, type Snapshot, type Task } from './api'
import { buildDraftTasks, selectionKey, selectedIndexes, type DraftTask } from './selection'
import { useVisibleDialogViewport } from './dialog-viewport'
const accounts=ref<Account[]>([]),keys=ref<Key[]>([]),models=ref<string[]>([]),efforts=ref<string[]>([])
const keyId=ref<string>(),accountIds=ref<string[]>([]),modelIds=ref<string[]>([]),effortIds=ref<string[]>(['high'])
const prompt=ref(''),concurrency=ref(10),timeout=ref(180),job=ref<Job|null>(null),busy=ref(false),catalogLoading=ref(false),dispatching=ref(false)
const results=ref<Record<number,Task>>({}),page=ref(1),pageSize=ref(20),message=ref(''),messageType=ref<'danger'|'info'|'success'>('info')
const inFlight=new Set<number>();let sequence=0,polling=false,fetching=false,timer:ReturnType<typeof setInterval>|undefined,disposed=false
const viewportDialog=useVisibleDialogViewport()
const presets=ref<Presets>({version:null,presets:[]}),presetId=ref<string>(''),presetsReady=ref(false),presetBusy=ref(false)
const manageOpen=viewportDialog(),editOpen=viewportDialog(),editId=ref(''),editName=ref(''),editPrompt=ref(''),deleteOpen=viewportDialog(),deletePreset=ref<Preset|null>(null)
const startOpen=viewportDialog(),prepared=ref<Plan|null>(null),manualOpen=viewportDialog(),manualModels=ref(''),detailOpen=viewportDialog(),detailTask=ref<Task|null>(null),exportOpen=viewportDialog(),exportText=ref('')
const hostTimeoutSeconds=ref<number|null>(null)
const effectiveTimeout=computed(()=>hostTimeoutSeconds.value===null?null:Math.min(timeout.value,Math.max(0,hostTimeoutSeconds.value-8)))
const validSettings=computed(()=>Number.isSafeInteger(concurrency.value)&&concurrency.value>=1&&Number.isSafeInteger(timeout.value)&&timeout.value>=10)
const accountSearch=ref(''),modelSearch=ref(''),excludedTasks=ref(new Set<string>())
const labels:Record<string,string>={pending:'待测试',running:'测试中',success:'成功',incomplete:'未完整',error:'失败',timeout:'超时',unknown:'结果未知',cancelled:'已取消'}
const verification:Record<string,string>={verified:'完整核对',matched:'主记录匹配',unverified:'未核实',mismatch:'账号不匹配'}
const locked=computed(()=>!!job.value?.tasks.some(t=>['pending','running'].includes(t.status)))
const hasPending=computed(()=>!!job.value?.tasks.some(t=>t.status==='pending'))
const hasRunning=computed(()=>!!job.value?.tasks.some(t=>t.status==='running')||inFlight.size>0)
const count=computed(()=>accountIds.value.length*modelIds.value.length*effortIds.value.length)
const draftTasks=computed(()=>buildDraftTasks(accounts.value,accountIds.value,modelIds.value,effortIds.value))
const selectedTaskIndexes=computed(()=>selectedIndexes(draftTasks.value,excludedTasks.value))
const previewTasks=computed<DraftTask[]>(()=>locked.value&&job.value?job.value.tasks.map(t=>({...t,key:selectionKey(t.accountId,t.model,t.effort),index:t.id})):draftTasks.value)
const previewCount=computed(()=>locked.value?previewTasks.value.length:count.value)
const selectedCount=computed(()=>locked.value?previewTasks.value.length:selectedTaskIndexes.value.length)
const canStart=computed(()=>hasPending.value?!dispatching.value:validSettings.value&&!hasRunning.value&&!!keyId.value&&selectedCount.value>0&&count.value<=500&&!!prompt.value.trim())
const visibleAccounts=computed(()=>accounts.value.filter(a=>`${a.email||''} ${a.name} ${a.provider_id}`.toLowerCase().includes(accountSearch.value.trim().toLowerCase())))
const visibleModels=computed(()=>models.value.filter(m=>m.toLowerCase().includes(modelSearch.value.trim().toLowerCase())))
const enabledAccounts=computed(()=>accounts.value.filter(a=>a.enabled))
const allAccounts=computed(()=>enabledAccounts.value.length>0&&enabledAccounts.value.every(a=>accountIds.value.includes(a.account_id)))
const allModels=computed(()=>models.value.length>0&&models.value.every(m=>modelIds.value.includes(m)))
const allTasks=computed(()=>selectedCount.value>0&&selectedCount.value===previewTasks.value.length)
const taskColumns:TableColumn<DraftTask>[]=[{key:'selected',kind:'selection',width:40},{key:'index',label:'#',kind:'custom',width:44},{key:'accountName',label:'账号',kind:'custom',minWidth:140},{key:'model',label:'模型',kind:'custom',minWidth:180},{key:'effort',label:'思考等级',kind:'text',width:90}]
function toggleAccount(id:string,checked:boolean){accountIds.value=checked?[...new Set([...accountIds.value,id])]:accountIds.value.filter(x=>x!==id)}
function toggleModel(id:string,checked:boolean){modelIds.value=checked?[...new Set([...modelIds.value,id])]:modelIds.value.filter(x=>x!==id)}
function toggleAllAccounts(){accountIds.value=allAccounts.value?[]:enabledAccounts.value.map(a=>a.account_id)}
function toggleAllModels(){modelIds.value=allModels.value?[]:[...models.value]}
function toggleTask(row:DraftTask,checked:boolean){const next=new Set(excludedTasks.value);if(checked)next.delete(row.key);else next.add(row.key);excludedTasks.value=next}
function toggleAllTasks(){excludedTasks.value=allTasks.value?new Set(draftTasks.value.map(t=>t.key)):new Set()}
watch(draftTasks,rows=>{const keys=new Set(rows.map(t=>t.key));excludedTasks.value=new Set([...excludedTasks.value].filter(k=>keys.has(k)))})
const keyOptions=computed(()=>keys.value.filter(k=>k.enabled).map(k=>({label:k.name,value:k.id})))
const effortOptions=computed(()=>efforts.value.map(e=>({label:e==='default'?'默认（不指定）':e,value:e})))
const presetOptions=computed(()=>[{label:'自定义问题',value:''},...presets.value.presets.map(p=>({label:p.name,value:p.id}))])
const stats=computed(()=>{const tasks=job.value?.tasks||[];return {total:tasks.length,done:tasks.filter(t=>!['pending','running'].includes(t.status)).length,success:tasks.filter(t=>t.status==='success').length,error:tasks.filter(t=>['error','timeout','unknown','incomplete'].includes(t.status)).length,running:tasks.filter(t=>t.status==='running').length}})
const rows=computed(()=>(job.value?.tasks||[]).slice((page.value-1)*pageSize.value,page.value*pageSize.value).map(t=>({...t,result:results.value[t.id]?.result})))
const columns:TableColumn<Task>[]=[{key:'index',label:'#',kind:'custom',width:48},{key:'accountName',label:'账号',kind:'custom',minWidth:120},{key:'model',label:'模型',kind:'mono',minWidth:180},{key:'effort',label:'思考等级',kind:'text',width:88},{key:'status',label:'状态',kind:'custom',width:80},{key:'durationMs',label:'耗时',kind:'custom',width:80},{key:'result',label:'答案',kind:'custom',minWidth:320}]
const presetColumns:TableColumn<Preset>[]=[{key:'name',label:'名称',kind:'text',minWidth:140},{key:'prompt',label:'问题',kind:'custom',minWidth:240},{key:'actions',label:'操作',kind:'actions',width:140}]
function statusType(status:string){return status==='success'?'success':status==='running'?'primary':['error','timeout','unknown'].includes(status)?'danger':status==='incomplete'?'warning':'default'}
function notify(text:string,type:'danger'|'info'|'success'='danger'){message.value=text;messageType.value=type}
function error(e:unknown){notify(e instanceof Error?e.message:'操作未完成，请刷新确认')}
function applyJob(next:Job|null){if(job.value?.id!==next?.id){results.value={};page.value=1;if(job.value&&dispatching.value){dispatching.value=false;notify('批次已变更，停止本页派发，请核对当前任务。')}}job.value=next}
async function loadPresets(){const p=await api<Presets>('api/presets');presets.value=p;presetsReady.value=true}
async function load(){busy.value=true;try{const v=await api<Snapshot>('api/snapshot');hostTimeoutSeconds.value=v.managementTimeoutSeconds;accounts.value=v.accounts;keys.value=v.keys;efforts.value=v.efforts;const first=job.value===null;applyJob(v.job)
 if(first&&v.job){const p=v.job.plan;keyId.value=p.keyId;accountIds.value=[...p.accounts];modelIds.value=[...p.models];effortIds.value=[...p.efforts];prompt.value=p.prompt;concurrency.value=p.concurrency;timeout.value=p.timeoutSeconds;const selected=new Set(p.selectedTaskIndexes??draftTasks.value.map(t=>t.index));excludedTasks.value=new Set(draftTasks.value.filter(t=>!selected.has(t.index)).map(t=>t.key))}
 if(!keys.value.some(k=>k.id===keyId.value&&k.enabled))keyId.value=keys.value.find(k=>k.enabled)?.id
 accountIds.value=accountIds.value.filter(id=>accounts.value.some(a=>a.account_id===id&&a.enabled))
 await Promise.all([loadModels(false),loadPresets()]);await fetchVisibleResults();message.value=''
 }catch(e){error(e)}finally{busy.value=false}}
async function loadModels(reset=true){const seq=++sequence;if(reset)modelIds.value=[];models.value=[...modelIds.value];if(!keyId.value)return;catalogLoading.value=true;try{const v=await api<{models:string[]}>('api/models',{keyId:keyId.value});if(seq===sequence)models.value=[...new Set([...v.models,...modelIds.value])].sort()}catch(e){if(seq===sequence)notify(`模型目录加载失败，可手动添加模型。${e instanceof Error?e.message:''}`)}finally{if(seq===sequence)catalogLoading.value=false}}
function choosePreset(id:unknown){const p=presets.value.presets.find(p=>p.id===id);if(p)prompt.value=p.prompt}
function openPreset(p?:Preset){editId.value=p?.id||'';editName.value=p?.name||'';editPrompt.value=p?.prompt??prompt.value;editOpen.value=true}
async function writePresets(next:Preset[]){presets.value=await api<Presets>('api/presets',{expectedVersion:presets.value.version,presets:next})}
async function savePreset(){if(!editName.value.trim()||!editPrompt.value.trim()){notify('请填写预设名称和问题');return}if(presets.value.presets.some(p=>p.name.trim()===editName.value.trim()&&p.id!==editId.value)){notify('预设名称已存在，请换一个名称');return}
 presetBusy.value=true;const id=editId.value||`preset_${Date.now()}_${Math.random().toString(36).slice(2,10)}`
 try{const p={id,name:editName.value.trim(),prompt:editPrompt.value};const next=editId.value?presets.value.presets.map(x=>x.id===id?p:x):[...presets.value.presets,p];await writePresets(next);if(!locked.value){presetId.value=id;prompt.value=p.prompt}editOpen.value=false;ZNotification.success({title:'预设已保存',message:p.name});message.value=''}catch(e){if(e instanceof ApiError&&e.status===409){await loadPresets();notify('预设已被其他页面修改，已重新加载。请核对后再保存。')}else error(e)}finally{presetBusy.value=false}}
async function removePreset(){if(!deletePreset.value)return;presetBusy.value=true;try{const id=deletePreset.value.id;await writePresets(presets.value.presets.filter(p=>p.id!==id));if(presetId.value===id)presetId.value='';deleteOpen.value=false;deletePreset.value=null;ZNotification.success({title:'预设已删除',message:''})}catch(e){if(e instanceof ApiError&&e.status===409)await loadPresets();error(e)}finally{presetBusy.value=false}}
function addModels(){const xs=manualModels.value.split(/[,，\s]+/).filter(Boolean);if(xs.some(m=>m.length>200)){notify('模型名称过长');return}models.value=[...new Set([...models.value,...xs])].sort();modelIds.value=[...new Set([...modelIds.value,...xs])];manualModels.value='';manualOpen.value=false}
function prepareStart(){if(!canStart.value||busy.value)return;if(hasPending.value){prepared.value=null}else{prepared.value={keyId:keyId.value||'',accounts:[...accountIds.value],models:[...modelIds.value],efforts:[...effortIds.value],prompt:prompt.value,concurrency:concurrency.value,timeoutSeconds:timeout.value,selectedTaskIndexes:[...selectedTaskIndexes.value]}}startOpen.value=true}
async function start(){busy.value=true;try{if(prepared.value){applyJob(await api<Job>('api/plan',prepared.value));prepared.value=null}if(!job.value)return;applyJob(await api<Job>('api/control',{jobId:job.value.id,action:'start'}));startOpen.value=false;dispatching.value=true;message.value='';pump()}catch(e){error(e)}finally{busy.value=false}}
async function control(action:'pause'|'cancel'){dispatching.value=false;if(!job.value)return;try{applyJob(await api<Job>('api/control',{jobId:job.value.id,action}));message.value=''}catch(e){error(e)}}
function pump(){if(disposed||!dispatching.value||!job.value||job.value.paused||inFlight.size)return;const available=Math.max(0,job.value.plan.concurrency-job.value.tasks.filter(t=>t.status==='running').length);const ids=job.value.tasks.filter(t=>t.status==='pending').slice(0,available).map(t=>t.id);if(ids.length){ids.forEach(id=>inFlight.add(id));void runBatch(job.value.id,ids)}else if(!hasPending.value&&!hasRunning.value)dispatching.value=false}
async function runBatch(jobId:string,taskIds:number[]){try{const v=await api<{job:Job;issues:{taskId:number;message:string}[]}>('api/run-batch',{jobId,taskIds});if(job.value?.id===jobId)applyJob(v.job);if(v.issues.length){dispatching.value=false;notify(`部分任务未执行，已停止新派发，不自动重试：${v.issues[0]!.message}`)}}catch(e){dispatching.value=false;notify(`批次调用未返回完整结果，已停止派发；不会自动重试。${e instanceof Error?e.message:''}`)}finally{taskIds.forEach(id=>inFlight.delete(id));try{applyJob((await api<{job:Job|null}>('api/job')).job);await fetchVisibleResults()}catch(e){dispatching.value=false;error(e)}pump()}}
async function fetchVisibleResults(){if(fetching||!job.value)return;fetching=true;const id=job.value.id;const ts=rows.value.filter(t=>!['pending','cancelled'].includes(t.status)&&(!results.value[t.id]||results.value[t.id]!.status!==t.status||t.status==='running'))
 try{for(let i=0;i<ts.length;i+=4){await Promise.all(ts.slice(i,i+4).map(async t=>{const v=await api<Task>('api/task',{jobId:id,taskId:t.id});if(job.value?.id===id)results.value[t.id]=v}))}}catch(e){error(e)}finally{fetching=false}}
async function showDetail(t:Task){if(!job.value)return;try{detailTask.value=await api<Task>('api/task',{jobId:job.value.id,taskId:t.id});detailOpen.value=true}catch(e){error(e)}}
async function auditDetail(){if(!job.value||!detailTask.value)return;try{const t=await api<Task>('api/audit',{jobId:job.value.id,taskId:detailTask.value.id});detailTask.value=t;results.value[t.id]=t;applyJob((await api<{job:Job|null}>('api/job')).job)}catch(e){error(e)}}
async function copy(text:string){try{await navigator.clipboard.writeText(text);ZNotification.success({title:'已复制',message:''})}catch{notify('浏览器不允许剪贴板访问，请手动选择文字复制。','info')}}
async function exportResults(){if(!job.value)return;busy.value=true;dispatching.value=false;try{await control('pause');const id=job.value.id;const out:Task[]=[];for(const t of job.value.tasks)out.push(await api<Task>('api/task',{jobId:id,taskId:t.id}));exportText.value=JSON.stringify({exportedAt:new Date().toISOString(),job:job.value,results:out},null,2);exportOpen.value=true}catch(e){error(e)}finally{busy.value=false}}
watch([page,pageSize],()=>void fetchVisibleResults())
onMounted(async()=>{await load();timer=setInterval(async()=>{if(polling||!job.value||disposed)return;polling=true;try{applyJob((await api<{job:Job|null}>('api/job')).job);await fetchVisibleResults();pump()}catch(e){dispatching.value=false;error(e)}finally{polling=false}},1500)})
onBeforeUnmount(()=>{disposed=true;dispatching.value=false;if(timer)clearInterval(timer)})
</script>

<template>
 <div class="dialogue-page">
  <ZTag v-if="message" :type="messageType" class="feedback" role="status">{{message}}</ZTag>
  <div class="test-board" data-testid="test-board">
   <ZCard title="账号" padding="compact" class="target-card account-card" data-testid="account-panel">
    <template #description>已选 {{accountIds.length}} / {{enabledAccounts.length}}</template>
    <template #actions><ZButton size="small" variant="text" :disabled="locked||busy||!enabledAccounts.length" @click="toggleAllAccounts">{{allAccounts?'取消全选':'全选'}}</ZButton></template>
    <div class="list-body">
     <ZInput v-model="accountSearch" class="field-control" clearable placeholder="搜索账号" aria-label="搜索账号" />
     <ZScrollbar height="308px" class="selection-scroll" data-testid="account-list">
      <div class="selection-list">
       <ZCheckbox v-for="a in visibleAccounts" :key="a.account_id" class="selection-row" :model-value="accountIds.includes(a.account_id)" :disabled="locked||busy||!a.enabled" :aria-label="`选择账号 ${a.email||a.name}`" @change="checked=>toggleAccount(a.account_id,checked)">
        <template #label><span class="list-label" :title="a.email||a.name">{{a.email||a.name}}</span><span class="list-meta">{{a.provider_id}}<span v-if="!a.enabled"> · 已停用</span></span></template>
       </ZCheckbox>
       <ZEmpty v-if="!visibleAccounts.length" :description="busy?'正在加载账号…':'没有匹配的账号'" />
      </div>
     </ZScrollbar>
    </div>
   </ZCard>
   <ZCard title="模型" padding="compact" class="target-card model-card" data-testid="model-panel">
    <template #description>已选 {{modelIds.length}} / {{models.length}}</template>
    <template #actions><ZButton size="small" variant="text" :disabled="locked||busy||!models.length" @click="toggleAllModels">{{allModels?'取消全选':'全选'}}</ZButton></template>
    <div class="list-body">
     <ZInput v-model="modelSearch" class="field-control" clearable placeholder="搜索模型" aria-label="搜索模型" />
     <ZScrollbar height="268px" class="selection-scroll" data-testid="model-list">
      <div class="selection-list">
       <ZCheckbox v-for="m in visibleModels" :key="m" class="selection-row" :model-value="modelIds.includes(m)" :disabled="locked||busy" :aria-label="`选择模型 ${m}`" @change="checked=>toggleModel(m,checked)"><template #label><span class="list-label model-label" :title="m">{{m}}</span></template></ZCheckbox>
       <ZEmpty v-if="!visibleModels.length" :description="catalogLoading?'正在加载模型…':'没有匹配的模型'" />
      </div>
     </ZScrollbar>
     <div class="model-toolbar"><ZButton size="small" variant="text" :disabled="locked" @click="manualOpen=true"><template #icon><Plus :size="14" /></template>手动添加模型</ZButton></div>
    </div>
   </ZCard>
   <ZCard title="测试题" padding="compact" class="target-card question-card" data-testid="question-panel">
    <div class="question-body">
     <div class="question-options">
      <ZFormItem label="预设问题"><ZSelect v-model="presetId" class="field-control" :options="presetOptions" filterable :disabled="locked" placeholder="自定义问题" @change="choosePreset" /></ZFormItem>
      <ZFormItem label="思考等级" required><ZSelect v-model="effortIds" class="field-control" :options="effortOptions" multiple collapse-tags collapse-tags-tooltip :max-collapse-tags="1" :disabled="locked||busy" placeholder="可多选" /></ZFormItem>
     </div>
     <ZFormItem label="测试问题" required><ZTextarea v-model="prompt" class="field-control" :rows="9" resize="none" :disabled="locked" placeholder="输入你想测试的问题，或选择上方预设。" aria-label="测试问题" /></ZFormItem>
     <div class="preset-actions"><ZButton size="small" variant="text" :disabled="!presetsReady||!prompt.trim()" @click="openPreset()"><template #icon><Save :size="14" /></template>保存为预设</ZButton><ZButton size="small" variant="text" :disabled="!presetsReady" @click="manageOpen=true"><template #icon><Settings2 :size="14" /></template>管理预设</ZButton></div>
    </div>
   </ZCard>
   <ZCard title="测试任务列表" padding="compact" class="tasks-card" data-testid="tasks-panel">
    <template #description><span role="status">已选 {{selectedCount}} / {{previewCount}} 项<span class="matrix-description"> · {{accountIds.length}} 账号 × {{modelIds.length}} 模型 × {{effortIds.length}} 等级</span></span></template>
    <template #actions><ZButton size="small" variant="text" :disabled="locked||!draftTasks.length" @click="toggleAllTasks">{{allTasks?'取消全选':'全选'}}</ZButton></template>
    <p v-if="count>500" class="capacity-warning" role="alert">组合超过 500 项，请减少账号、模型或思考等级后再测试。</p>
    <ZScrollbar height="240px" class="task-scroll" data-testid="task-list">
     <ZTable :columns="taskColumns" :data="previewTasks" row-key="key" size="small" empty-text="选择账号、模型和思考等级后，任务将自动显示" show-header-when-empty>
      <template #header-selected><ZCheckbox :model-value="allTasks" :indeterminate="selectedCount>0&&!allTasks" :disabled="locked||!draftTasks.length" aria-label="全选测试任务" @change="toggleAllTasks" /></template>
      <template #selected="{row}"><ZCheckbox :model-value="locked||!excludedTasks.has(row.key)" :disabled="locked" :aria-label="`选择任务 ${row.index+1}`" @change="checked=>toggleTask(row,checked)" /></template>
      <template #index="{row}">{{row.index+1}}</template>
      <template #accountName="{row}"><span class="task-label" :title="row.accountName">{{row.accountName}}</span></template>
      <template #model="{row}"><span class="task-label model-label" :title="row.model">{{row.model}}</span></template>
     </ZTable>
    </ZScrollbar>
    <p class="footnote">任务自动生成，默认全部选择；取消勾选的任务不会执行。{{locked?'当前批次已锁定，取消待测或执行结束后可重新配置。':''}}</p>
   </ZCard>
   <ZCard title="运行设置" padding="compact" class="run-card" data-testid="run-panel">
    <ZForm class="run-form" @submit.prevent="prepareStart">
     <ZFormItem label="执行 Key" required><ZSelect v-model="keyId" class="field-control" :options="keyOptions" filterable :disabled="locked||busy" placeholder="选择执行 Key" @change="loadModels(true)" /></ZFormItem>
     <div class="run-numbers">
      <ZFormItem label="并发数量"><ZInputNumber v-model="concurrency" class="field-control" :min="1" :precision="0" :disabled="locked" aria-label="并发数量" /></ZFormItem>
      <ZFormItem label="单项超时"><ZInputNumber v-model="timeout" class="field-control" :min="10" :precision="0" unit="秒" :disabled="locked" aria-label="单项超时" /></ZFormItem>
     </div>
     <div class="run-actions">
      <ZButton type="primary" class="start-button" :loading="busy" :disabled="!canStart" @click="prepareStart"><template #icon><Play :size="16" /></template>{{hasPending?'继续测试':'开始测试'}}<span v-if="!hasPending&&selectedCount"> · {{selectedCount}} 项</span></ZButton>
      <div v-if="hasPending||hasRunning" class="actions"><ZButton :disabled="busy||(!dispatching&&job?.paused)" @click="control('pause')"><template #icon><Pause :size="14" /></template>暂停</ZButton><ZButton v-if="hasPending" :disabled="busy" @click="control('cancel')"><template #icon><X :size="14" /></template>取消待测</ZButton></div>
     </div>
     <p v-if="effectiveTimeout!==null&&effectiveTimeout<timeout" class="footnote timeout-notice" role="note">当前 CPR 宿主上限 {{hostTimeoutSeconds}} 秒，本次有效测试期限约 {{effectiveTimeout}} 秒；{{timeout}} 秒需宿主支持。</p>
     <p class="footnote">并发仍受 CPR / 账号 / 上游容量约束。真实调用消耗额度，每批最多 500 项。保持页面打开以持续派发；暂停仅停止新请求。</p>
    </ZForm>
   </ZCard>
  </div>
  <ZCard title="测试结果" padding="compact" class="results-card" data-testid="results-panel">
   <template #description><span v-if="job">{{stats.done}} / {{stats.total}} 已结束 · 成功 {{stats.success}} · 失败 {{stats.error}}<span v-if="stats.running"> · 运行中 {{stats.running}}</span></span><span v-else>答案会直接显示在表格最后一栏</span></template>
   <template #actions><ZButton size="small" :disabled="!job||busy" @click="exportResults"><template #icon><Download :size="14" /></template>导出 JSON</ZButton></template>
   <ZTable :columns="columns" :data="rows" row-key="id" size="small" :loading="busy&&!job" empty-text="选择账号、模型和思考等级，填写问题后开始测试" show-header-when-empty>
    <template #index="{row}">{{row.id+1}}</template>
    <template #accountName="{row}"><div>{{row.accountName}}</div><div v-if="row.verification&&row.verification!=='unverified'" class="footnote" :title="row.verification==='matched'?'已保存日志账号一致，尝试明细不完整，不等于完整审计':''">{{verification[row.verification]}}</div></template>
    <template #status="{row}"><ZTag :type="statusType(row.status)" size="small">{{labels[row.status]||row.status}}</ZTag></template>
    <template #durationMs="{row}">{{row.durationMs==null?'—':`${(row.durationMs/1000).toFixed(1)}s`}}</template>
    <template #result="{row}"><div class="answer-cell"><div class="answer-text" :class="{'error-text':row.status==='error'||row.status==='timeout'}">{{row.result?.text||row.result?.error?.message||row.error?.message||(row.status==='pending'?'等待测试':row.status==='running'?'等待回答…':row.status==='cancelled'?'已取消':'加载结果…')}}</div><ZTag v-if="row.result?.truncated" type="warning" size="small">答案超过显示上限，已截断</ZTag><div v-if="row.result?.text||row.result?.requestId" class="answer-actions"><ZButton v-if="row.result?.text" size="small" variant="text" @click="copy(row.result.text!)"><template #icon><Copy :size="12" /></template>复制</ZButton><ZButton size="small" variant="text" @click="showDetail(row)">详情</ZButton></div></div></template>
   </ZTable>
   <ZPagination v-if="stats.total>20" v-model:current-page="page" v-model:page-size="pageSize" :total="stats.total" :page-sizes="[20,50]" />
  </ZCard>
  <ZDialog :draggable="false" v-model="startOpen" :title="hasPending?'继续测试':'开始测试'" type="warning" :close-on-click-modal="false"><p>将执行 {{hasPending?job?.tasks.filter(t=>t.status==='pending').length:prepared?.selectedTaskIndexes?.length??selectedCount}} 项真实模型测试，并发 {{prepared?.concurrency||job?.plan.concurrency||concurrency}}。</p><p class="secondary">会消耗上游额度。指定账号仍受 Key 权限和账号模型规则限制，不会转给其他账号代测。</p><template #footer><ZButton :disabled="busy" @click="startOpen=false">取消</ZButton><ZButton type="primary" :loading="busy" @click="start">确认开始</ZButton></template></ZDialog>
  <ZDialog :draggable="false" v-model="manualOpen" title="添加模型"><ZFormItem label="模型名称"><ZInput class="field-control" v-model="manualModels" placeholder="可输入多个模型名，用逗号或空格分隔" aria-label="模型名称" /></ZFormItem><template #footer><ZButton @click="manualOpen=false">取消</ZButton><ZButton type="primary" :disabled="!manualModels.trim()" @click="addModels">添加</ZButton></template></ZDialog>
  <ZDialog :draggable="false" v-model="manageOpen" title="预设问题" width="760px"><ZTable :columns="presetColumns" :data="presets.presets" row-key="id" size="small" empty-text="尚未保存预设问题"><template #prompt="{row}"><ZScrollbar max-height="100px" class="preset-scroll"><div class="preset-preview">{{row.prompt}}</div></ZScrollbar></template><template #actions="{row}"><div class="actions"><ZButton size="small" variant="text" @click="openPreset(row)">编辑</ZButton><ZButton type="danger" size="small" variant="text" @click="deletePreset=row;deleteOpen=true">删除</ZButton></div></template></ZTable><template #footer><ZButton @click="manageOpen=false">关闭</ZButton><ZButton type="primary" @click="openPreset()"><template #icon><Plus :size="14" /></template>新增预设</ZButton></template></ZDialog>
  <ZDialog :draggable="false" v-model="editOpen" :title="editId?'编辑预设':'保存预设问题'" :close-on-click-modal="false"><ZFormItem label="预设名称" required><ZInput class="field-control" v-model="editName" placeholder="例如：模型知识截止日期" aria-label="预设名称" /></ZFormItem><ZFormItem label="预设问题" required><ZTextarea v-model="editPrompt" :rows="5" aria-label="预设问题" /></ZFormItem><p class="footnote">保存到插件私有状态，所有管理员共享；最多 32 条。编辑测试问题不会自动覆盖预设。</p><template #footer><ZButton :disabled="presetBusy" @click="editOpen=false">取消</ZButton><ZButton type="primary" :loading="presetBusy" @click="savePreset">保存</ZButton></template></ZDialog>
  <ZDialog :draggable="false" v-model="deleteOpen" title="删除预设" type="warning"><p>确定删除「{{deletePreset?.name}}」？已生成的测试任务不受影响。</p><template #footer><ZButton :disabled="presetBusy" @click="deleteOpen=false">取消</ZButton><ZButton type="danger" :loading="presetBusy" @click="removePreset">删除预设</ZButton></template></ZDialog>
  <ZDialog :draggable="false" v-model="detailOpen" :title="`${detailTask?.model||''} / ${detailTask?.effort||''}`" width="760px"><p class="secondary">{{detailTask?.accountName}} · {{labels[detailTask?.status||'']}}</p><pre class="detail-json">{{JSON.stringify(detailTask?.result,null,2)}}</pre><p class="footnote">主记录匹配不等于完整审计；请求的等级和上游回显不能证明内部实际思考强度。</p><template #footer><ZButton @click="detailOpen=false">关闭</ZButton><ZButton :disabled="!detailTask?.result?.requestId||detailTask?.status==='running'" @click="auditDetail">核对账号</ZButton></template></ZDialog>
  <ZDialog :draggable="false" v-model="exportOpen" title="导出测试结果" width="760px"><p class="secondary">复制 JSON 后保存；包含问题、答案和账号标签，不含密钥。</p><ZTextarea v-model="exportText" :rows="16" readonly aria-label="结果 JSON" /><template #footer><ZButton @click="exportOpen=false">关闭</ZButton><ZButton type="primary" @click="copy(exportText)">复制 JSON</ZButton></template></ZDialog>
 </div>
</template>

