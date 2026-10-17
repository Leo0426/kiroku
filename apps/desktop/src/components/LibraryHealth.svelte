<script lang="ts">
 import {onDestroy} from 'svelte'
 import {api} from '../lib/api'
 import {app} from '../lib/state.svelte'
 import {repairFlow} from '../lib/repairflow.svelte'
 import type {HealthReport,HealthIssue} from '../lib/types'
 let {onopen}:{onopen:(code:string)=>void}=$props()
 let report=$state<HealthReport|null>(null),busy=$state(false),error=$state(''),filter=$state('')
 let generation=0;onDestroy(()=>generation++);$effect(()=>{app.settings?.library_root;generation++;report=null;error='';busy=false})
 const repairHere=$derived(repairFlow.root===app.settings?.library_root&&repairFlow.origin==='health')
 async function scan(){const g=generation;busy=true;error='';try{const next=await api.libraryHealth();if(g===generation)report=next}catch(e){if(g===generation)error=String(e)}finally{if(g===generation)busy=false}}
 async function action(i:HealthIssue){const g=generation;try{if(i.action==='rebuildIndex'){await app.rebuildIndex();if(g===generation)app.toast('info','索引重建完成后，请重新检查')}else if(i.action==='repairImages'&&i.code)await repairFlow.start(i.code,'health');else await api.revealLibraryPath(i.dir)}catch(e){if(g===generation)error=String(e)}}
 async function reveal(i:HealthIssue){try{await api.revealLibraryPath(i.dir)}catch(e){error=String(e)}}
</script>
<section aria-label="媒体库健康检查"><button class="btn" disabled={busy||repairFlow.running} onclick={scan}>{busy?'正在检查媒体库…':'检查媒体库'}</button>{#if busy}<button class="btn" onclick={()=>api.jobCancel()}>停止检查</button>{/if}
 {#if repairHere&&repairFlow.running}<p role="status">{repairFlow.code} 正在补图…</p><button class="btn" onclick={()=>repairFlow.cancel()}>停止补图</button>{/if}
 {#if repairHere&&repairFlow.error}<p role="alert">补图失败：{repairFlow.error}</p>{/if}
 {#if repairHere&&repairFlow.result}{@const r=repairFlow.result}<div aria-label="健康检查补图结果"><p role="status">{r.code} {r.cancelled?'补图已停止':r.failed.length?'补图失败':'补图完成'}：写入 {r.written} 张图片。</p>{#each r.failed as failure}<p role="alert">{failure[1]}</p>{/each}{#each r.warnings as warning}<p>{warning}</p>{/each}{#if r.indexError}<p role="alert">索引更新失败：{r.indexError}</p>{/if}</div>{/if}
 {#if report}<p role="status">检查 {report.scanned} 个影片目录 · {report.issues.length} 个问题{report.cancelled?' · 已取消，报告不完整':''} · {new Date(report.scannedAt*1000).toLocaleString()}</p>
 <label>问题类型 <select class="input" bind:value={filter}><option value="">全部问题</option>{#each [...new Set(report.issues.map(i=>i.kind))] as kind}<option value={kind}>{kind}</option>{/each}</select></label>
 {#each report.issues.filter(i=>!filter||i.kind===filter) as i}<div class="issue"><strong>{i.code??'媒体库'} · {i.message}</strong><p class="muted">{i.path}</p>
 {#if i.action!=='reveal'}<button class="btn sm" disabled={busy||repairFlow.running} onclick={()=>action(i)}>{i.action==='rebuildIndex'?'重建索引':'补缺图'}</button>{/if}
 <button class="btn sm" onclick={()=>reveal(i)}>打开目录</button>{#if i.code}<button class="btn sm" onclick={()=>onopen(i.code!)}>查看影片详情</button>{/if}</div>{/each}
 {#if !report.issues.length&&!report.cancelled}<p>本次检查未发现问题。</p>{/if}{/if}
 {#if error}<p role="alert">检查失败：{error}</p>{/if}</section>
<style>section{margin:12px 0}p{overflow-wrap:anywhere}.issue{padding:12px;border-bottom:1px solid var(--line)}select{margin:8px}button{margin:4px}</style>
