import {api} from './api'
import type {RecoveryEntry} from './types'
const cacheKey=(root:string)=>`kiroku-recovery-v1:${root}`
export class RecoveryStore {
 root=$state<string|null>(null)
 entries=$state<RecoveryEntry[]>([])
 loading=$state(false)
 stale=$state(true)
 error=$state('')
 busyId=$state<string|null>(null)
 private generation=0
 constructor(private currentRoot:()=>string|null){}
 private cached(root:string):RecoveryEntry[]{
  try{const value=JSON.parse(localStorage.getItem(cacheKey(root))??'null');if(!Array.isArray(value))return [];
   const ids=new Set<string>()
   return value.filter((e:RecoveryEntry)=>e&&typeof e.id==='string'&&['partial','pending','corrupt'].includes(e.kind)&&Array.isArray(e.codes)&&e.codes.every(c=>typeof c==='string')&&typeof e.message==='string'&&typeof e.canUndo==='boolean'&&Number.isInteger(e.pendingOperations)&&e.pendingOperations>=0).filter((e:RecoveryEntry)=>{if(ids.has(e.id))return false;ids.add(e.id);return true}).slice(0,1000)
  }catch{return []}
 }
 async refresh(){
  const root=this.currentRoot(),g=++this.generation
  if(this.root!==root){this.root=root;this.entries=root?this.cached(root):[];this.error='';this.stale=true}
  if(!root){this.loading=false;return}
  this.loading=true;this.stale=true;this.error=''
  try{const result=await api.recoveryInspect();if(g!==this.generation||root!==this.currentRoot())return;if(result.root!==root)throw '媒体库已变化，请重新检查';this.entries=result.entries;this.stale=false;try{localStorage.setItem(cacheKey(root),JSON.stringify(this.entries))}catch{ /* Display still works when browser storage is unavailable. */ }}
  catch(e){if(g===this.generation&&root===this.currentRoot())this.error=String(e)}
  finally{if(g===this.generation)this.loading=false}
 }
 async undo(entry:RecoveryEntry){
  const root=this.root;if(this.stale||this.loading||!entry.canUndo||!root||root!==this.currentRoot()||this.busyId)throw '请先核对当前媒体库'
  this.busyId=entry.id;this.error=''
  try{const result=await api.rollback(entry.id);if(root!==this.currentRoot())return;if(result.failures.length||result.indexError)this.error=[...result.failures,result.indexError].filter(Boolean).join('；');const failure=this.error;await this.refresh();if(failure&&root===this.currentRoot())this.error=failure}
  catch(e){if(root===this.currentRoot())this.error=String(e)}
  finally{this.busyId=null}
 }
}
