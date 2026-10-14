// Local UI fixtures only. This file is never part of the plugin package.
(() => {
 const qs=new URLSearchParams(location.search);document.documentElement.dataset.theme=qs.get('theme')==='light'?'light':'dark'
 const models=['claude-fable-5-1','claude-opus-5-5','deepseek-flash','deepseek-v4-pro','gemini-3.8-flash-high','vendor-model-with-a-very-long-name-for-layout-regression-testing']
 const accounts=[{account_id:'fixture_gpt',name:'GPT',email:null,provider_id:'openai',enabled:true},{account_id:'fixture_claude',name:'CLAUDE',email:null,provider_id:'openai',enabled:true},{account_id:'fixture_long',name:'Account with a very long display name',email:'layout-only-long-account-label@example.test',provider_id:'openai',enabled:true}]
 if(qs.get('state')==='long'){models.reverse();accounts.reverse()}
 const plan={keyId:'fixture_key',accounts:accounts.map(a=>a.account_id),models,efforts:['high','low','xhigh'],prompt:'请只用一个数字回答：1 + 1 等于多少？',concurrency:10,timeoutSeconds:180}
 const task={id:0,accountId:accounts[0].account_id,accountName:'GPT',model:models[0],effort:'high',status:'success',durationMs:1450,verification:'matched'}
 let fixtureJob=['many','long'].includes(qs.get('state'))?{id:'fixture_job',plan,tasks:[task],paused:true}:null
 const interactive=qs.get('state')==='interactive'
 const sends=[],batches=[]
 let panel
 if(interactive){panel=document.createElement('pre');panel.id='fixture-report';panel.textContent='本地模拟 · 不连接任何模型上游';document.addEventListener('DOMContentLoaded',()=>document.body.appendChild(panel))}
 const receipt=()=>{if(panel)panel.textContent='本地模拟 · 不连接任何模型上游\n'+JSON.stringify({selectedTaskIndexes:fixtureJob?.plan.selectedTaskIndexes,batches,sent:sends},null,2)}
 const presets={version:1,presets:[{id:'fixture_arithmetic',name:'简单算术',prompt:plan.prompt},{id:'fixture_cutoff',name:'知识截止日期',prompt:'你的知识截止日期是什么？请简短回答。'}]}
 if(qs.has('longPreset'))presets.presets[0]={id:'fixture_arithmetic',name:'长问题预设',prompt:'在一个黑色的袋子里放有三种口味的糖果，每种糖果有两种不同的形状。请回答最终结果。'.repeat(60)}
 window.codexProxyPlugin={version:2,theme:document.documentElement.dataset.theme,async request({path,body}){
  let data;const input=body?JSON.parse(body):null
  if(interactive&&path==='api/plan'){
   const tasks=[];let index=0
   for(const a of input.accounts)for(const m of input.models)for(const e of input.efforts){if(input.selectedTaskIndexes.includes(index))tasks.push({id:tasks.length,accountId:a,accountName:accounts.find(x=>x.account_id===a).name,model:m,effort:e,status:'pending'});index++}
   fixtureJob={id:'fixture_interactive',plan:input,tasks,paused:true};data=fixtureJob;receipt()
  }
  else if(interactive&&path==='api/control'){fixtureJob.paused=input.action!=='start';if(input.action==='cancel')fixtureJob.tasks.forEach(t=>{if(t.status==='pending')t.status='cancelled'});data=fixtureJob}
  else if(interactive&&path==='api/run-batch'){
   batches.push(input.taskIds)
   for(const id of input.taskIds){const t=fixtureJob.tasks[id];t.status='success';t.durationMs=10;t.result={text:'本地模拟答案：2'};sends.push({accountId:t.accountId,model:t.model,effort:t.effort})}
   data={job:fixtureJob,issues:[]};receipt()
  }
  else if(interactive&&path==='api/run'){
   const t=fixtureJob.tasks[input.taskId];t.status='success';t.durationMs=10;t.result={text:'本地模拟答案：2'};sends.push({accountId:t.accountId,model:t.model,effort:t.effort});data=t;receipt()
  }
  else if(interactive&&path==='api/presets'&&input){presets.version++;presets.presets=input.presets;data=presets}
  else if(interactive&&path==='api/task'){data=fixtureJob.tasks[input.taskId]}
  else
  if(path==='api/snapshot')data={accounts,keys:[{id:'fixture_key',name:'TEST',enabled:true}],efforts:['default','none','minimal','low','medium','high','xhigh','max','ultra'],job:fixtureJob,managementTimeoutSeconds:120,version:'0.4.1'}
  else if(path==='api/models')data={models}
  else if(path==='api/presets')data=presets
  else if(path==='api/job')data={job:fixtureJob}
  else if(path==='api/task')data={...task,result:{text:'2',verification:'matched',durationMs:1450,requestId:'fixture_request'}}
  else return {status:400,contentType:'application/json',body:new TextEncoder().encode(JSON.stringify({error:{message:'本地布局预览禁止真实执行或写入'}})).buffer}
  return {status:200,contentType:'application/json',body:new TextEncoder().encode(JSON.stringify(data)).buffer}
 }}
})()
