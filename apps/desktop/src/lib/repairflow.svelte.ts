import {api} from './api'
import {listen} from './events'
import {app} from './state.svelte'
import type {RepairDone} from './types'
/** App-lifetime task ownership survives moving between health reports and movie details. */
class RepairFlow {
 running=$state(false)
 origin=$state<'health'|'details'>('details')
 code=$state('')
 root=$state('')
 jobId=$state<number|null>(null)
 result=$state<RepairDone|null>(null)
 error=$state('')
 private ready:Promise<unknown>|null=null
 private buffered:RepairDone[]=[]
 private cancelRequested=false
 private generation=0
 private init(){return this.ready??=listen('repair:done',e=>{if(!this.running||e.code!==this.code||e.root!==this.root)return;if(this.jobId===null)this.buffered.push(e);else this.complete(e)})}
 private complete(e:RepairDone){if(e.jobId!==this.jobId||e.code!==this.code||e.root!==this.root)return;this.running=false;this.result=e;this.buffered=[];if(app.settings?.library_root===e.root)app.libraryVersion++}
 async start(code:string,origin:'health'|'details'='details'){if(this.running)throw '已有补图任务正在运行';const generation=++this.generation;this.origin=origin;this.code=code;this.root=app.settings?.library_root??'';this.result=null;this.error='';this.running=true;this.jobId=null;this.buffered=[];this.cancelRequested=false;try{await this.init();const id=await api.repairImagesStart(code);if(generation!==this.generation)return;this.jobId=id;if(this.cancelRequested)await api.jobCancel(id);for(const e of this.buffered)this.complete(e);this.buffered=[]}catch(e){if(generation===this.generation){this.running=false;this.error=String(e)}}}
 async cancel(){this.cancelRequested=true;if(this.jobId!==null)await api.jobCancel(this.jobId)}
 clearResult(){if(!this.running){this.result=null;this.error=''}}
}
export const repairFlow=new RepairFlow()
