import {api} from './api'
import {suspendPendingReads} from './pending'
import {cancellableJob, type CancellableJob} from './cancellable-job'
import type {PreserveResult, RecoveryProgress} from './types'
import {listen} from './events'
import type {RecoveryEntry} from './types'
const acknowledgedKey=(root:string)=>`kiroku-recovery-acknowledged-v1:${root}`
const fingerprint=(e:RecoveryEntry)=>JSON.stringify([e.id,e.kind,e.codes,e.pendingOperations,e.message,e.canUndo,e.canPreserve??false])
const cacheKey=(root:string)=>`kiroku-recovery-v1:${root}`
export class RecoveryStore {
 root=$state<string|null>(null)
 entries=$state<RecoveryEntry[]>([])
 private acknowledged=$state<string[]>([])
 visibleEntries=$derived(this.entries.filter(e=>!this.acknowledged.includes(fingerprint(e))))
 loading=$state(false)
 stale=$state(true)
 error=$state('')
 busyId=$state<string|null>(null)
 private generation=0
 private inspection: {root:string; done:Promise<void>} | null = null
 preserving=$state(false)
 progress=$state<RecoveryProgress|null>(null)
 stopping=$state(false)
 private preserveJob:CancellableJob<PreserveResult>|null=null
 async cancelPreserve(){this.stopping=true;await this.preserveJob?.cancel()}
 async preserve(entry:RecoveryEntry):Promise<boolean>{
  const root=this.root,g=this.generation
  if(this.stale||this.loading||!entry.canPreserve||!root||root!==this.currentRoot()||this.busyId)throw '请先核对当前媒体库'
  this.busyId=entry.id;this.error='';this.progress=null;this.stopping=false
  let releaseReads:(()=>void)|null=null
  let unlisten:(()=>void)|null=null
  const job:CancellableJob<PreserveResult>=cancellableJob(async requestId=>{
   releaseReads=await suspendPendingReads()
   if(job.cancelled||root!==this.currentRoot()||g!==this.generation)throw new Error('恢复已取消或媒体库已变化')
   unlisten=await listen('recovery:progress',event=>{
    if(event.jobId!==job.jobId||event.root!==root||root!==this.currentRoot()||g!==this.generation)return
    this.progress={stage:event.stage,done:event.done,total:event.total,elapsedMs:event.elapsedMs}
   })
   return api.recoveryPreserve(root,entry.id,requestId)
  },e=>{if(root===this.currentRoot())this.error=`停止失败，可重试：${e}`})
  this.preserveJob=job;this.preserving=true
  try{
   const result=await job.promise
   if(root!==this.currentRoot()||g!==this.generation||result.root!==root)return false
   const pending=result.indexPending??0
   const failure=[result.recoveryError,...result.indexErrors,pending?`文件成果已保留，${pending} 个目录的索引待更新，可重试核对`:null,result.cancelled?'已停止后续整理':null].filter(Boolean).join('；')
   await this.refresh()
   if(root!==this.currentRoot())return false
   if(failure)this.error=failure
   return !failure&&!job.cancelled&&!result.cancelled
  }catch(e){if(root===this.currentRoot())this.error=String(e);return false}
  finally{(unlisten as (()=>void)|null)?.();(releaseReads as (()=>void)|null)?.();if(this.preserveJob===job){this.preserveJob=null;this.preserving=false;this.busyId=null}}
 }
 constructor(private currentRoot:()=>string|null){}
 private cached(root:string):RecoveryEntry[]{
  try{const value=JSON.parse(localStorage.getItem(cacheKey(root))??'null');if(!Array.isArray(value))return [];
   const ids=new Set<string>()
   return value.filter((e:RecoveryEntry)=>e&&typeof e.id==='string'&&['partial','pending','corrupt'].includes(e.kind)&&Array.isArray(e.codes)&&e.codes.every(c=>typeof c==='string')&&typeof e.message==='string'&&typeof e.canUndo==='boolean'&&Number.isInteger(e.pendingOperations)&&e.pendingOperations>=0).filter((e:RecoveryEntry)=>{if(ids.has(e.id))return false;ids.add(e.id);return true}).slice(0,1000)
  }catch{return []}
 }
 private readAcknowledged(root:string):string[]{
  try{const value=JSON.parse(localStorage.getItem(acknowledgedKey(root))??'[]');return Array.isArray(value)?value.filter((v:unknown):v is string=>typeof v==='string'):[]}catch{return []}
 }
 acknowledge(){
  const root=this.root
  if(!root||root!==this.currentRoot()||this.loading||this.busyId)return
  const next=[...new Set([...this.acknowledged,...this.visibleEntries.map(fingerprint)])]
  // Persist before hiding: storage failure must not claim a durable acknowledgement.
  localStorage.setItem(acknowledgedKey(root),JSON.stringify(next))
  this.acknowledged=next
 }
 refresh(): Promise<void> {
  const root = this.currentRoot()
  if (root && this.root === root && this.inspection?.root === root) return this.inspection.done
  const generation = ++this.generation
  const done = this.inspect(root, generation).finally(() => {
   if (generation === this.generation) this.inspection = null
  })
  this.inspection = root ? {root, done} : null
  return done
 }
 private async inspect(root:string|null,g:number){
  if(this.root!==root){this.root=root;this.acknowledged=root?this.readAcknowledged(root):[];this.entries=root?this.cached(root):[];this.error='';this.stale=true}
  if(!root){this.loading=false;return}
  this.loading=true;this.stale=true;this.error=''
  try{const result=await api.recoveryInspect();if(g!==this.generation||root!==this.currentRoot())return;if(result.root!==root)throw '媒体库已变化，请重新检查';this.entries=result.entries;this.stale=false;try{localStorage.setItem(cacheKey(root),JSON.stringify(this.entries))}catch{ /* Display still works when browser storage is unavailable. */ }}
  catch(e){if(g===this.generation&&root===this.currentRoot())this.error=String(e)}
  finally{if(g===this.generation)this.loading=false}
 }
 async undo(entry:RecoveryEntry){
  const root=this.root;if(this.stale||this.loading||!entry.canUndo||!root||root!==this.currentRoot()||this.busyId)throw '请先核对当前媒体库'
  this.busyId=entry.id;this.error='';this.progress=null;this.stopping=false
  try{const result=await api.rollback(entry.id);if(root!==this.currentRoot())return;if(result.failures.length||result.indexError)this.error=[...result.failures,result.indexError].filter(Boolean).join('；');const failure=this.error;await this.refresh();if(failure&&root===this.currentRoot())this.error=failure}
  catch(e){if(root===this.currentRoot())this.error=String(e)}
  finally{this.busyId=null}
 }
}
