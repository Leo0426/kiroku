<script lang="ts">
  import { onDestroy, untrack } from 'svelte'
  import { api } from '../lib/api'
  import { app } from '../lib/state.svelte'
  import { cancellableJob, type CancellableJob } from '../lib/cancellable-job'
  import { listen } from '../lib/events'
  import type { Query, MovieCard, BatchEdit, BatchEditResult, BatchEditPreview } from '../lib/types'
  let {query}:{query:Query}=$props()
  let opened=$state(false), busy=$state(false), error=$state('')
  let movies=$state<MovieCard[]>([]), selected=$state<string[]>([]), offset=$state(0), total=$state(0)
  let mode=$state('favorite'), favorite=$state(true), add=$state(''), remove=$state('')
  let preview=$state.raw<BatchEditPreview|null>(null), result=$state<BatchEditResult|null>(null)
  let task=$state.raw<CancellableJob<unknown>|null>(null)
  let completed=$state(0), failed=$state(0)
  let generation=0
  const locked=$derived(busy || app.rebuild.running)
  const changed=$derived(preview?.items.filter(item=>item.changes.length).length ?? 0)
  function discard(token=preview?.token) {
    if(token)void api.batchEditDiscard(token).catch(e=>app.toast('error',`释放批量预览失败：${e}`,5000))
    if(preview?.token===token)preview=null
  }
  function reset() {generation++;void task?.cancel();task=null;opened=false;discard();result=null;selected=[];movies=[];error='';busy=false}
  $effect(()=>{app.settings;JSON.stringify(query);untrack(reset)})
  onDestroy(()=>{generation++;void task?.cancel();discard()})
  async function page(at=0) {
    const current=generation;busy=true;error='';opened=true
    try{const rows=await api.libraryList({...$state.snapshot(query),offset:at,limit:50});if(current===generation){movies=rows.items;total=rows.total;offset=at}}
    catch(e){if(current===generation)error=String(e)}finally{if(current===generation)busy=false}
  }
  function toggle(code:string) {
    if(selected.includes(code))selected=selected.filter(v=>v!==code)
    else if(selected.length<200)selected=[...selected,code]
    else app.toast('info','每批最多选择 200 部，请分批处理')
  }
  async function inspect() {
    const current=generation;busy=true;error='';result=null
    const values=(text:string)=>text.split('\n').map(v=>v.trim()).filter(Boolean)
    const edit:BatchEdit=mode==='favorite'?{kind:'favorite',favorite}:{kind:'tags',add:values(add),remove:values(remove)}
    const request=cancellableJob(id=>api.batchEditPreview([...selected],edit,id),e=>{if(current===generation)error=String(e)});task=request
    try{const p=await request.promise;if(current===generation&&!request.cancelled)preview=p;else discard(p.token)}
    catch(e){if(current===generation)error=String(e)}finally{if(current===generation){busy=false;task=null}}
  }
  async function save() {
    if(!preview)return
    const current=generation,p=preview;busy=true;error='';completed=0;failed=0
    const request=cancellableJob(id=>api.batchEditApply(p.token,p.root,id),e=>{if(current===generation)error=String(e)});task=request
    try{const r=await request.promise;if(current===generation){result=r;preview=null;app.libraryVersion++;void app.recovery.refresh()}}
    catch(e){discard(p.token);if(current===generation)error=String(e)}
    finally{if(current===generation){busy=false;task=null}}
  }
  async function undo() {
    if(!result)return
    const current=generation,items=[...result.results].reverse();busy=true;error=''
    const remaining=new Set(items.map(item=>item.journalId))
    try{for(const item of items){if(current!==generation)break;const r=await api.rollback(item.journalId);if(r.failures.length||r.indexError)throw [...r.failures,r.indexError].filter(Boolean).join('；');remaining.delete(item.journalId)}
      if(current===generation){result={...result,results:result.results.filter(item=>remaining.has(item.journalId))};app.libraryVersion++;void app.recovery.refresh()}}
    catch(e){if(current===generation){error=String(e);result={...result,results:result.results.filter(item=>remaining.has(item.journalId))}}}
    finally{if(current===generation)busy=false}
  }
  $effect(()=>{let alive=true,stop:(()=>void)|undefined;listen('batch:edit',event=>{if(task?.jobId===event.jobId){completed=event.completed;failed=event.failed}}).then(fn=>{if(alive)stop=fn;else fn()});return()=>{alive=false;stop?.()}})
  const show=(value:unknown)=>Array.isArray(value)?value.join('、')||'（空）':typeof value==='boolean'?value?'收藏':'未收藏':String(value??'（空）')
</script>
<section aria-label="批量收藏与标签">
  <div class="bar"><h3>批量收藏与标签</h3><button class="btn" disabled={locked} onclick={()=>opened?reset():page()}>{opened?'关闭批量编辑':'选择影片批量编辑'}</button></div>
  {#if opened}
    {#if preview}
      <p>已选 {preview.items.length} 部，将修改 {changed} 部；其余保持原样。</p>
      <div class="diff" aria-label="批量编辑差异">{#each preview.items as item}<details open><summary>{item.code}{item.changes.length?'':' · 无需修改'}</summary>{#each item.changes as change}<p>{change.field==='tags'?'标签':'收藏'}：{show(change.before)} → {show(change.after)}</p>{/each}</details>{/each}</div>
      <div class="bar"><button class="btn" disabled={locked} onclick={()=>discard()}>返回修改选择</button><button class="btn primary" disabled={locked||!changed} onclick={save}>确认保存批量修改</button></div>
    {:else if result}
      <p role="status">已保存 {result.results.filter(item=>!item.error).length} 部，失败 {result.failed.length+result.results.filter(item=>!!item.error).length} 部，跳过 {result.skipped.length} 部{result.cancelled?'，任务已停止':''}。每部影片均可在历史中撤销。</p>
      {#each [...result.failed,...result.results.flatMap(item=>[item.error,item.indexError].filter(Boolean))] as failure}<p role="alert">{failure}</p>{/each}
      <div class="bar"><button class="btn" disabled={locked||!result.results.length} onclick={undo}>撤销本批修改</button><button class="btn" disabled={locked} onclick={()=>{result=null;selected=[];void page()}}>继续编辑</button></div>
    {:else}
      <p class="muted">从当前筛选结果分页选择，每批最多 200 部。标签按完整文字匹配，修改后锁定标签字段。</p>
      <fieldset disabled={locked}>
        <legend>修改内容</legend>
        <label>操作<select class="input" aria-label="批量编辑操作" bind:value={mode}><option value="favorite">收藏</option><option value="tags">标签增删</option></select></label>
        {#if mode==='favorite'}<label>收藏状态<select class="input" aria-label="批量收藏状态" bind:value={favorite}><option value={true}>设为收藏</option><option value={false}>取消收藏</option></select></label>
        {:else}<div class="tags"><label>添加标签（每行一个）<textarea class="input" aria-label="批量添加标签" rows="3" bind:value={add}></textarea></label><label>移除标签（每行一个）<textarea class="input" aria-label="批量移除标签" rows="3" bind:value={remove}></textarea></label></div>{/if}
      </fieldset>
      <div class="bar"><button class="btn sm" disabled={locked||!movies.length} onclick={()=>{const combined=[...new Set([...selected,...movies.map(m=>m.code)])];selected=combined.slice(0,200);if(combined.length>200)app.toast('info','已达到每批 200 部的上限')}}>选择本页影片</button><button class="btn sm ghost" disabled={locked||!selected.length} onclick={()=>selected=[]}>清空影片选择</button><span>已选 {selected.length} 部</span></div>
      <fieldset class="movies" disabled={locked}><legend>选择影片</legend>{#each movies as movie}<label><input class="checkbox" type="checkbox" aria-label="批量选择 {movie.code}" checked={selected.includes(movie.code)} onchange={()=>toggle(movie.code)} /><span>{movie.code} {movie.title??''}</span></label>{/each}</fieldset>
      <div class="bar"><span>{total?offset+1:0}–{offset+movies.length} / {total}</span><button class="btn sm" disabled={locked||offset===0} onclick={()=>page(Math.max(0,offset-50))}>上一页影片</button><button class="btn sm" disabled={locked||offset+movies.length>=total} onclick={()=>page(offset+50)}>下一页影片</button><button class="btn primary" disabled={locked||!selected.length} onclick={inspect}>预览批量修改</button></div>
    {/if}
    {#if busy}<p role="status">{preview?`已保存 ${completed} 部，失败 ${failed} 部`:'正在读取或核对影片…'}{#if task}<button class="btn sm" onclick={()=>task?.cancel()}>停止批量任务</button>{/if}</p>{/if}
  {/if}
  {#if error}<p role="alert">{error}</p>{/if}
</section>
<style>
  section { padding:16px 20px; border-bottom:1px solid var(--line); display:grid; gap:12px; }
  h3 { font-size:var(--fs-sm); margin:0; }.bar { display:flex; flex-wrap:wrap; align-items:center; gap:8px; }
  fieldset { border:1px solid var(--line); border-radius:var(--r-sm); min-width:0; } label { display:flex; align-items:center; gap:12px; margin:8px 0; }
  .movies { max-height:240px; overflow:auto; } .movies label span { overflow-wrap:anywhere; min-width:0; }
  .tags { display:grid; grid-template-columns:1fr 1fr; gap:12px; }.tags label { display:grid; gap:4px; }.tags textarea { height:auto; padding:8px; resize:vertical; width:100%; }
  .diff { max-height:280px; overflow:auto; } details { padding:8px; border-bottom:1px solid var(--line); } p { overflow-wrap:anywhere; }
  @media(max-width:600px){.tags{grid-template-columns:1fr}}
</style>
