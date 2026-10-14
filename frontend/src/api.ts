interface Host { version:number; request:(v:{method:'GET'|'POST';path:string;contentType?:string;body?:string})=>Promise<{status:number;contentType:string;body:ArrayBuffer}> }
declare global { interface Window {codexProxyPlugin?:Host} }
export class ApiError extends Error { constructor(message:string, public status:number, public code?:string){super(message)} }
// A long-running batch occupies one bridge request, not one per model.
// Keep two lanes for polling/results and avoid the host page's RPC burst limit.
let active=0
const waiters:(()=>void)[]=[]
async function acquire(){if(active<3){active++;return}await new Promise<void>(resolve=>waiters.push(resolve))}
function release(){const next=waiters.shift();if(next)next();else active--}
export async function api<T>(path:string,data?:unknown):Promise<T>{
 const host=window.codexProxyPlugin
 if(!host||host.version!==2)throw new Error('请从 CPR 的插件菜单打开此页面')
 await acquire()
 try{
 const r=await host.request({method:data===undefined?'GET':'POST',path,...(data===undefined?{}:{contentType:'application/json',body:JSON.stringify(data)})})
 if(r.contentType.split(';')[0]?.trim()!=='application/json')throw new Error('插件返回格式无效')
 const value=JSON.parse(new TextDecoder().decode(r.body))
 if(r.status<200||r.status>=300)throw new ApiError(value.error?.message||`操作失败 HTTP ${r.status}`,r.status,value.error?.code)
 return value as T
 }finally{release()}
}
export interface Account {account_id:string;name:string;email:string|null;provider_id:string;enabled:boolean}
export interface Key {id:string;name:string;enabled:boolean}
export interface Plan {keyId:string;accounts:string[];models:string[];efforts:string[];prompt:string;concurrency:number;timeoutSeconds:number;selectedTaskIndexes?:number[]}
export interface Result {text?:string;error?:{code:string;message:string};durationMs?:number;verification?:string;requestId?:string;truncated?:boolean;[key:string]:unknown}
export interface Task {id:number;accountId:string;accountName:string;model:string;effort:string;status:string;durationMs?:number;verification?:string;error?:{code:string;message:string};result?:Result}
export interface Job {id:string;plan:Plan;paused:boolean;tasks:Task[]}
export interface Snapshot {accounts:Account[];keys:Key[];efforts:string[];job:Job|null;version:string;managementTimeoutSeconds:number}
export interface Preset {id:string;name:string;prompt:string}
export interface Presets {version:number|null;presets:Preset[]}
