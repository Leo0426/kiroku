<script lang="ts">
 import {onDestroy,untrack} from 'svelte'
 import {fmtDateTime} from '../lib/format'
 import InfoTip from './InfoTip.svelte'
 import {api} from '../lib/api'
 import {cancellableJob,type CancellableJob} from '../lib/cancellable-job'
 import {app} from '../lib/state.svelte'
 import {SOURCE_NAMES,type SourceCheck} from '../lib/types'
 let task=$state<CancellableJob<unknown>|null>(null)
 let {dirty=false}:{dirty?:boolean}=$props()
 let checks=$state<SourceCheck[]>([]),busy=$state(false),error=$state(''),snapshot=$state('')
 let generation=0;onDestroy(()=>{generation++;void task?.cancel()});$effect(()=>{app.settings;untrack(()=>{generation++;void task?.cancel();task=null;checks=[];snapshot='';error='';busy=false})})
 const reasons:Record<string,string>={ok:'可达',authenticated:'可达 · 已登录',unauthenticated:'可达 · 未登录，请填写或更新 Cookie',authUnknown:'可达 · 登录状态未确认',needLogin:'需要登录或更新 Cookie',blocked:'站点拦截，请检查出口',geoBlocked:'地域限制，请检查代理',rateLimited:'请求过于频繁，稍后重试',timeout:'连接超时',network:'网络或上游异常',unexpectedResponse:'响应内容不符合预期',disabledOrUnknown:'来源未启用或不存在',invalidProxy:'代理配置无效',cancelled:'已取消',undeclared:'来源未声明探测方式',endpointNotFound:'探测接口不存在'}
 async function check(ids?:string[]){
  if(busy)return
  const g=generation;busy=true;error='';snapshot=JSON.stringify(app.settings)
  const request=cancellableJob(id=>api.sourcesCheck(ids,undefined,id),e=>{if(g===generation)error=String(e)});task=request
  try{const result=await request.promise;if(g!==generation)return;checks=ids?[...checks.filter(c=>!ids.includes(c.sourceId)),...result]:result}
  catch(e){if(g===generation)error=String(e)}finally{if(g===generation){busy=false;task=null}}
 }
</script>
<section aria-label="来源自检">
 <header><div class="tipped"><h2>来源自检</h2><InfoTip label="来源自检说明"><p>使用已保存的路由、代理与 Cookie。可达不保证每个番号都能找到。</p></InfoTip></div><div class="actions"><button class="btn sm" disabled={busy||dirty} onclick={()=>check()}>{busy?'检测中…':'检测已用来源'}</button>{#if busy}<button class="btn sm ghost" onclick={()=>task?.cancel()}>停止检测</button>{/if}</div></header>
 <p class="description">检查当前启用来源的连接和登录状态。</p>
 {#if dirty}<p>请先保存配置，再检测来源。</p>{/if}
 {#if snapshot&&snapshot!==JSON.stringify(app.settings)}<p>配置已变化，请重新检测。</p>{/if}
 {#if checks.length}<div class="results">{#each checks as c}<div class="result-row"><strong>{SOURCE_NAMES[c.sourceId]??c.sourceId}</strong><div class="result-info"><span class="outcome" class:ok={['ok','authenticated'].includes(c.outcome.reason)}>{reasons[c.outcome.reason]??c.outcome.reason}</span><span class="checked">{fmtDateTime(c.checkedAt*1000)}</span></div><span class="latency">{c.elapsedMs} ms</span><button class="btn sm ghost" aria-label="重测 {SOURCE_NAMES[c.sourceId]??c.sourceId}" disabled={busy||dirty} onclick={()=>check([c.sourceId])}>重测</button></div>{/each}</div>{/if}
 {#if error}<p role="alert">{error}</p>{/if}
</section>
<style>
 section{padding:16px;margin-bottom:16px;background:var(--surface-1);border-radius:var(--r-lg);container-type:inline-size}
 header,.tipped,.actions{display:flex;align-items:center;gap:8px}header{justify-content:space-between;flex-wrap:wrap}h2{font-size:var(--fs-lg);font-weight:600}.description{font-size:var(--fs-sm);color:var(--text-2);margin-top:6px}
 .results{margin-top:16px}.result-row{display:grid;grid-template-columns:100px minmax(0,1fr) 72px auto;align-items:center;gap:16px;padding:12px 0;border-top:1px solid var(--line);font-size:var(--fs-sm)}.result-info{display:flex;flex-wrap:wrap;align-items:center;gap:8px 16px}.outcome{color:var(--text-2)}.outcome.ok{color:var(--ok)}.checked{font-size:var(--fs-xs);color:var(--text-3)}.latency{font-variant-numeric:tabular-nums;color:var(--text-2);text-align:right}
 @container(max-width:480px){.result-row{grid-template-columns:minmax(0,1fr) auto;gap:6px 12px}.result-info{grid-column:1;grid-row:2}.latency{grid-column:2;grid-row:2}.result-row button{grid-column:2;grid-row:1}}
</style>
