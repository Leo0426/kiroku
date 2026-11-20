import {invoke} from '@tauri-apps/api/core'
import {mount,tick,unmount} from 'svelte'
import StartupRecovery from '../../src/components/StartupRecovery.svelte'
import ConfirmDialog from '../../src/components/ConfirmDialog.svelte'
import Import from '../../src/views/Import.svelte'
import {app} from '../../src/lib/state.svelte'
import {flow} from '../../src/lib/importflow.svelte'
import {api} from '../../src/lib/api'
import type {Settings} from '../../src/lib/types'
function assert(value:unknown,message:string):asserts value{if(!value)throw new Error(message)}
async function until(predicate:()=>boolean,message:string){const end=Date.now()+10000;while(!predicate()){if(Date.now()>end)throw new Error(message);await new Promise(r=>setTimeout(r,10))}await tick()}
export async function runResume(passed:string[]){
 const fixture=await invoke<{settings:Settings;journalId:string}>('bridge_resume_prepare')
 app.settings=fixture.settings;flow.reset();await app.recovery.refresh()
 const host=document.createElement('main');host.style.cssText='height:100vh;overflow:auto';document.body.append(host)
 const recovery=mount(StartupRecovery,{target:host}),dialog=mount(ConfirmDialog,{target:host}),view=mount(Import,{target:host})
 async function click(text:string){await tick();const buttons=[...host.querySelectorAll<HTMLButtonElement>('button')].filter(b=>b.textContent?.trim()===text);assert(buttons.length===1&&!buttons[0].disabled,`missing/enabled button ${text}`);buttons[0].scrollIntoView({block:'center'});await new Promise<void>((resolve,reject)=>{const timer=setTimeout(()=>reject(new Error('WebKit frame unavailable')),5000);requestAnimationFrame(()=>{clearTimeout(timer);resolve()})});buttons[0].click();await tick()}
 try{
  assert(app.recovery.entries.some(e=>e.id===fixture.journalId&&e.canPreserve),'native pending record not preservable')
  await click('保留成果后继续');await click('核对并继续')
  await until(()=>!app.recovery.busyId&&flow.cachedSourcesOnly,'preserve did not open cached import')
  assert(!app.recovery.error,'preserve failed')
  await invoke('bridge_resume_verify',{journalId:fixture.journalId,phase:'preserved'})
  const requests=await invoke<{requests:string[]}>('bridge_network_snapshot')
  await click('开始扫描');await until(()=>!flow.scanRunning&&!!flow.scanSummary,'remaining scan')
  assert(flow.items.length===2,'scan repeated completed source')
  await click('查找影片信息（2 部）');await until(()=>!flow.scrape.running&&flow.found.length===2,'cached remaining metadata')
  assert(flow.scrapeSummary?.cache?.reused===2,'source cache not reused')
  assert(JSON.stringify((await invoke<{requests:string[]}>('bridge_network_snapshot')).requests)===JSON.stringify(requests.requests),'cached resume unexpectedly requested sources')
  await click('预览整理');await until(()=>flow.step==='plan'&&!flow.planning,'remaining preview')
  await click('确认整理（2 个文件）');await until(()=>flow.step==='done'&&!flow.apply.running,'remaining apply')
  assert(flow.applyDone?.applied===2&&!flow.applyDone.failed.length&&!flow.applyDone.indexError,'remaining apply failed')
  await invoke('bridge_resume_verify',{journalId:fixture.journalId,phase:'applied'})
  await click('撤销本批');await click('确认撤销');await until(()=>host.textContent?.includes('本批已撤销')===true,'remaining undo')
  const result=await api.rollback(fixture.journalId);assert(!result.failures.length&&!result.indexError,'original undo failed')
  await invoke('bridge_resume_verify',{journalId:fixture.journalId,phase:'restored'})
  passed.push('resume_preserve_remaining_undo')
 }finally{await unmount(recovery);await unmount(dialog);await unmount(view);host.remove()}
}
