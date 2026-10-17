<script lang="ts">
 import { onDestroy } from 'svelte'
 import { listen } from '../lib/events'
 import { api } from '../lib/api'
 import { app } from '../lib/state.svelte'
 import type { Query,MovieCard,NfoLanguage,LanguageBatchPreview,LanguageBatchResult } from '../lib/types'
 let {query}:{query:Query}=$props()
 let opened=$state(false),movies=$state<MovieCard[]>([]),selected=$state<string[]>([]),language=$state<NfoLanguage>('current'),preview=$state<LanguageBatchPreview|null>(null),result=$state<LanguageBatchResult|null>(null),error=$state(''),busy=$state(false),more=$state(false)
 let completed=$state(0),failures=$state(0)
 let generation=0;onDestroy(()=>generation++)
 $effect(()=>{app.settings?.library_root;generation++;opened=false;preview=null;result=null;selected=[];error=''})
 async function start(){const current=generation;opened=true;busy=true;error='';try{const page=await api.libraryList({...query,offset:0,limit:200});if(current===generation){movies=page.items;more=page.total>movies.length;selected=[];preview=null;result=null}}catch(e){if(current===generation)error=String(e)}finally{if(current===generation)busy=false}}
 async function inspect(){const current=generation;busy=true;error='';try{const p=await api.nfoExportPreview(selected,language);if(current===generation)preview=p}catch(e){if(current===generation)error=String(e)}finally{if(current===generation)busy=false}}
 async function save(){if(!preview)return;const current=generation,p=preview;busy=true;completed=0;failures=0;error='';try{const r=await api.nfoExportApply(p.token,p.root);if(current===generation){result=r;preview=null;app.libraryVersion++}}catch(e){if(current===generation){error=String(e);preview=null}}finally{if(current===generation)busy=false}}
 async function undo(){if(!result)return;busy=true;error='';try{for(const item of result.results){const r=await api.rollback(item.journalId);if(r.failures.length||r.indexError)throw [...r.failures,r.indexError].filter(Boolean).join('；')}result=null;app.libraryVersion++}catch(e){error=String(e)}finally{busy=false}}
 $effect(()=>{let alive=true,stop:(()=>void)|undefined;listen('nfo:export',event=>{if(busy){completed=event.completed;failures=event.failed}}).then(fn=>{if(alive)stop=fn;else fn()});return()=>{alive=false;stop?.()}})
</script>
<section aria-label="批量 NFO 语言导出">
 <button class="btn" onclick={start}>批量导出 NFO 语言</button>
 {#if opened}
 <h3>选择影片与 NFO 语言</h3>
 {#if more}<p>仅列出当前筛选的前 200 部，请用媒体库搜索缩小范围。</p>{/if}
 <select aria-label="批量 NFO 语言" bind:value={language} disabled={busy}><option value="current">当前文本</option><option value="zh-Hans">简体中文</option><option value="original">来源原文</option></select>
 {#if preview}
 <div aria-label="批量 NFO 差异">{#each preview.items as item}<details open><summary>{item.code}</summary>{#each item.changes as change}<p>{change.field}：{String(change.before??'（空）')} → {String(change.after??'（空）')}</p>{/each}{#each item.warnings??[] as warning}<p>{warning}</p>{/each}</details>{/each}</div>
 <button class="btn primary" disabled={busy} onclick={save}>确认批量导出</button>
 {:else if result}
 <p>已完成 {result.results.length} 部{result.cancelled?'，任务已取消':''}</p>{#each [...result.failed,...result.results.flatMap(r=>[r.error,r.indexError].filter(Boolean))] as failure}<p role="alert">{failure}</p>{/each}<button class="btn" disabled={busy} onclick={undo}>撤销本批导出</button>
 {:else}
 <fieldset disabled={busy}><legend>影片</legend>{#each movies as movie}<label><input type="checkbox" value={movie.code} bind:group={selected}/>{movie.code} {movie.title??''}</label>{/each}</fieldset>
 <button class="btn" disabled={busy||!selected.length} onclick={inspect}>预览批量 NFO 导出</button>
 {/if}
 {#if busy}<p role="status">已导出 {completed} 部，失败 {failures} 部</p><button class="btn" onclick={()=>api.jobCancel()}>取消导出任务</button>{/if}<button class="btn ghost" disabled={busy} onclick={()=>opened=false}>关闭语言导出</button>
 {/if}
 {#if error}<p role="alert">{error}</p>{/if}
</section>
<style>fieldset{max-height:240px;overflow:auto}label{display:block;margin:6px}details{margin:8px}</style>
