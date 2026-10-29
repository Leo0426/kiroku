<script lang="ts">
 import {onDestroy,untrack} from 'svelte'
 import InfoTip from './InfoTip.svelte'
 import {api} from '../lib/api'
 import {cancellableJob,type CancellableJob} from '../lib/cancellable-job'
 import {app} from '../lib/state.svelte'
 import {fmtBytes} from '../lib/format'
 import type {MaintenancePreview,MaintenanceResult} from '../lib/types'
 let task=$state<CancellableJob<unknown>|null>(null)
 let {dirty=false,onpolicySaved}:{dirty?:boolean;onpolicySaved?:()=>void}=$props()
 let preview=$state<MaintenancePreview|null>(null),result=$state<MaintenanceResult|null>(null),selected=$state<string[]>([]),busy=$state(false),error=$state(''),historyDays=$state(90),logDays=$state(90)
 let generation=0;onDestroy(()=>{generation++;void task?.cancel()});$effect(()=>{app.settings;untrack(()=>{generation++;void task?.cancel();task=null;preview=null;result=null;selected=[];error='';busy=false;historyDays=app.settings?.retention.historyDays??90;logDays=app.settings?.retention.logDays??90})})
 let includeRecent=$state(false)
 function changeScope(){generation++;preview=null;result=null;selected=[];error=''}
 async function savePolicy(){if(!app.settings)return;const g=generation;busy=true;error='';try{if(!Number.isInteger(historyDays)||!Number.isInteger(logDays)||historyDays<0||logDays<0)throw '保留期必须为非负整数';await api.saveSettings({...$state.snapshot(app.settings),retention:{historyDays,logDays}});await app.refreshSettings();onpolicySaved?.()}catch(e){if(g===generation)error=String(e)}finally{if(g===generation)busy=false}}
 async function inspect(){
  if(busy)return
  const g=generation;busy=true;error='';result=null
  const request=cancellableJob(id=>api.maintenancePreview(includeRecent,id),e=>{if(g===generation)error=String(e)});task=request
  try{const next=await request.promise;if(g===generation){preview=next;selected=[]}}
  catch(e){if(g===generation)error=String(e)}finally{if(g===generation){busy=false;task=null}}
 }
 async function collect(ids:string[]=[...selected]){if(busy||!preview||!ids.length)return;const g=generation,p=preview;const wanted=new Set(ids);const targets=p.candidates.filter(c=>wanted.has(c.id));const ok=await app.confirm({title:'确认删除生成数据',body:`将 ${targets.length} 项生成数据移到系统废纸篓：\n${targets.slice(0,5).map(c=>`${c.description}：${c.paths[0]??''}`).join('\n')+(targets.length>5?`\n另有 ${targets.length-5} 项，详见预览列表。`:'')}\n\n旧记录移走后不再出现在操作历史；有效撤销记录及其依赖继续保留。执行前会重新核对文件与引用。`,confirmLabel:'移到废纸篓'});if(!ok||g!==generation||preview!==p)return;busy=true;error='';const request=cancellableJob(id=>api.maintenanceApply(p.token,ids,id),e=>{if(g===generation)error=String(e)});task=request;try{const next=await request.promise;if(g===generation){result=next;preview=null;selected=[];app.libraryVersion++;void app.recovery.refresh()}}catch(e){if(g===generation){error=String(e);preview=null}}finally{if(g===generation){busy=false;task=null}}}
</script>
<section aria-label="安全空间维护"><div class="tipped"><h2>安全空间维护</h2><InfoTip label="安全空间维护说明"><p>只提供无引用旧原图、已完整撤销且无依赖的历史与备份、旧文件日志；不会自动清理。</p><p>影片视频、字幕、当前 NFO 与海报、仍用于裁剪或撤销的数据和当日日志会保留。删除的文件移到废纸篓。</p></InfoTip></div>
 <label>历史与原图保留天数 <input class="input" type="number" min="0" step="1" bind:value={historyDays} disabled={busy||dirty}/></label>
 <label>文件日志保留天数 <input class="input" type="number" min="0" step="1" bind:value={logDays} disabled={busy||dirty}/></label>
 <button class="btn" onclick={savePolicy} disabled={busy||dirty}>保存保留期</button><button class="btn" onclick={inspect} disabled={busy||dirty}>预览空间清理</button>{#if dirty}<p>请先保存设置页中的改动，再进行维护。</p>{/if}
 <label><input class="checkbox" type="checkbox" bind:checked={includeRecent} onchange={changeScope} disabled={busy||dirty}/>包含近期可删除数据</label>
 {#if includeRecent}<p class="muted">本次预览不受保留天数限制，仍检查所有引用；不会修改保留期设置。</p>{/if}
 {#if busy&&task}<button class="btn" onclick={()=>task?.cancel()}>停止维护任务</button>{/if}
 {#if preview}<p>已识别资源文件长度：{fmtBytes(preview.resourceBytes??preview.candidateBytes)} · 保留：{fmtBytes(preview.protectedBytes??0)}</p><p>可清理文件大小合计：{fmtBytes(preview.candidateBytes)}（按文件长度统计，硬链接不代表额外磁盘占用）</p>
 {#if preview.candidates.length}<button class="btn sm" disabled={busy} onclick={()=>selected=preview!.candidates.map(c=>c.id)}>全选可删除项</button><button class="btn sm" disabled={busy||!selected.length} onclick={()=>selected=[]}>取消选择</button>{/if}
 {#each preview.candidates as candidate}<div class="candidate"><div class="candidate-head"><label><input type="checkbox" aria-label={`选择 ${candidate.id}`} bind:group={selected} value={candidate.id} disabled={busy}/>{candidate.description} · {fmtBytes(candidate.bytes)}</label><button class="btn danger sm" aria-label={`删除 ${candidate.description}`} disabled={busy||dirty} onclick={()=>collect([candidate.id])}>删除</button></div>{#each candidate.paths as path}<p class="muted mono">{path}</p>{/each}</div>{/each}
 {#if !preview.candidates.length}<p>没有可安全清理的候选。</p>{/if}
 <details><summary>受保护资源 · {preview.protected.length}</summary>{#each preview.protected as reason}<p class="muted">{reason}</p>{/each}</details>
 <button class="btn primary" onclick={()=>collect()} disabled={busy||dirty||!selected.length}>清理选中项</button>{/if}
 {#if result}<p role="status">已移到废纸篓：{result.removed} 个文件{result.cancelled?' · 已取消':''}</p>{#each result.skipped as reason}<p>{reason}</p>{/each}{#each result.failed as failure}<p role="alert">{failure}</p>{/each}{/if}
 {#if error}<p role="alert">{error}</p>{/if}</section>
<style>.tipped{display:flex;align-items:center;gap:4px}section{margin:16px 0;padding:20px;background:var(--surface-2);border-radius:12px}label{display:inline-flex;gap:8px;align-items:center;margin:8px;min-width:0;max-width:100%;flex-wrap:wrap}input[type=number]{width:90px}.candidate{margin:12px 0;padding:12px;border:1px solid var(--line);border-radius:var(--r-sm)}.candidate-head{display:flex;align-items:center;justify-content:space-between;gap:8px;flex-wrap:wrap}.candidate-head label{flex:1;margin:0}.candidate-head .btn{flex-shrink:0}p{overflow-wrap:anywhere}button{margin:4px}details{margin:12px 0;max-height:240px;overflow:auto}</style>
