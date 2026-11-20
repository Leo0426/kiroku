<script lang="ts">
  import { onDestroy, untrack } from 'svelte'
  import { listen } from '../lib/events'
  import { cancellableJob, type CancellableJob } from '../lib/cancellable-job'
  import { resolveExportCodes, type ExportScope as Scope } from '../lib/export-scope'
  import { api } from '../lib/api'
  import { app } from '../lib/state.svelte'
  import type { Query, NfoLanguage, LanguageBatchPreview, LanguageBatchResult } from '../lib/types'
  import ExportScope from './ExportScope.svelte'
  import LoadingStatus from './LoadingStatus.svelte'

  let { query }: { query:Query }=$props()
  let opened=$state(false), busy=$state(''), error=$state('')
  let scope=$state<Scope>({kind:'selection',codes:[]})
  let language=$state<NfoLanguage>('current')
  let preview=$state.raw<LanguageBatchPreview|null>(null), result=$state.raw<LanguageBatchResult|null>(null)
  let completed=$state(0), failures=$state(0), page=$state(0)
  const pageSize=30
  let task=$state.raw<CancellableJob<unknown>|null>(null)
  let controller=$state.raw<AbortController|null>(null)
  let generation=0

  function stop() { controller?.abort(); void task?.cancel() }
  function reset() {
    generation++; stop(); controller=null; task=null
    opened=false; busy=''; error=''; preview=null; result=null; page=0
    scope={kind:'selection',codes:[]}
  }
  onDestroy(reset)
  $effect(()=>{ app.settings?.library_root; untrack(reset) })
  async function inspect() {
    if(busy) return
    const current=generation, abort=new AbortController()
    controller=abort; busy='正在读取影片范围…'; error=''
    try {
      const codes=await resolveExportCodes($state.snapshot(scope),$state.snapshot(query),abort.signal)
      abort.signal.throwIfAborted()
      if(!codes.length) throw '当前范围没有影片'
      if(current!==generation)return
      controller=null
      busy='正在核对 NFO 写入位置与差异…'
      const request=cancellableJob(id=>api.nfoExportPreview(codes,language,id),e=>{if(current===generation)error=String(e)})
      task=request
      const value=await request.promise
      if(current===generation&&!request.cancelled){preview=value;page=0}
    }catch(e){if(current===generation)error=abort.signal.aborted?'预览已停止':String(e)}
    finally{if(current===generation){controller=null;task=null;busy=''}}
  }
  async function save() {
    if(!preview||busy)return
    const current=generation,p=preview
    busy='正在写回影片目录…'; completed=0; failures=0; error=''
    const request=cancellableJob(id=>api.nfoExportApply(p.token,p.root,id),e=>{if(current===generation)error=String(e)})
    task=request
    try {
      const done=await request.promise
      if(current===generation){result=done;preview=null;app.libraryVersion++}
    }catch(e){if(current===generation){error=String(e);preview=null}}
    finally{if(current===generation){task=null;busy=''}}
  }
  async function undo() {
    if(!result||busy)return
    const current=generation, batch=result
    const undone=new Set<string>()
    busy='正在撤销本批导出…';error=''
    try {
      for(const item of batch.results){
        if(current!==generation)break
        const r=await api.rollback(item.journalId)
        if(r.failures.length||r.indexError)throw [...r.failures,r.indexError].filter(Boolean).join(' · ')
        undone.add(item.journalId)
      }
    }catch(e){if(current===generation)error=String(e)}
    finally{if(current===generation){
      const remaining=batch.results.filter(item=>!undone.has(item.journalId))
      result=remaining.length?{...batch,results:remaining}:null
      if(undone.size)app.libraryVersion++
      busy=''
    }}
  }
  $effect(()=>{
    let alive=true,off:(()=>void)|undefined
    listen('nfo:export',event=>{if(task?.jobId===event.jobId){completed=event.completed;failures=event.failed}})
      .then(fn=>{if(alive)off=fn;else fn()})
    return ()=>{alive=false;off?.()}
  })
</script>

<section aria-label="批量 NFO 语言导出">
  <button class="btn" aria-expanded={opened} onclick={()=>opened=true} disabled={!!busy}>批量导出 NFO 语言</button>
  {#if opened}
    <div class="body">
      <p>将所选语言的文本写回每部影片目录中的 NFO，同时更新 kiroku.json。</p>
      {#if preview}
        <p><strong>将写回 {preview.items.length} 部影片</strong></p>
        <div aria-label="批量 NFO 差异">
          {#each preview.items.slice(page*pageSize,(page+1)*pageSize) as item}
            <details open={preview.items.length<=5}><summary>{item.code} · {item.nfoPath??item.dir}</summary>
              {#each item.changes as change}<p>{change.field}：{String(change.before??'（空）')} → {String(change.after??'（空）')}</p>{/each}
              {#each item.warnings??[] as warning}<p>{warning}</p>{/each}
            </details>
          {/each}
          {#if preview.items.length>pageSize}
            <div class="actions"><button class="btn sm" disabled={page===0} onclick={()=>page--}>上一页差异</button>
              <span>{page+1} / {Math.ceil(preview.items.length/pageSize)}</span>
              <button class="btn sm" disabled={(page+1)*pageSize>=preview.items.length} onclick={()=>page++}>下一页差异</button></div>
          {/if}
        </div>
        <div class="actions"><button class="btn primary" disabled={!!busy} onclick={save}>确认批量导出</button>
          <button class="btn" disabled={!!busy} onclick={()=>preview=null}>返回修改范围</button></div>
      {:else if result}
        <p>已完成 {result.results.length} 部{result.cancelled?'，任务已取消':''}</p>
        {#each [...result.failed,...result.results.flatMap(r=>[r.error,r.indexError].filter(Boolean))] as failure}<p role="alert">{failure}</p>{/each}
        <div class="actions">
          {#if result.results.length}<button class="btn" disabled={!!busy} onclick={undo}>撤销本批导出</button>{/if}
          <button class="btn" disabled={!!busy} onclick={()=>result=null}>选择其他影片</button>
        </div>
      {:else}
        <ExportScope {query} bind:scope disabled={!!busy} labelPrefix="NFO "/>
        <label class="language">导出语言<select class="input" aria-label="批量 NFO 语言" bind:value={language} disabled={!!busy}>
          <option value="current">当前文本</option><option value="zh-Hans">简体中文</option><option value="original">来源原文</option></select></label>
        <button class="btn" disabled={!!busy||(scope.kind==='selection'&&!scope.codes.length)} onclick={inspect}>预览批量 NFO 导出</button>
      {/if}
      {#if busy}
        <LoadingStatus label={busy==='正在写回影片目录…' ? `已导出 ${completed} 部，失败 ${failures} 部` : busy}/>
        {#if task||controller}<button class="btn" onclick={stop}>取消导出任务</button>{/if}
      {/if}
      {#if error}<p role="alert">{error}</p>{/if}
      <button class="btn ghost" disabled={!!busy} onclick={reset}>关闭语言导出</button>
    </div>
  {/if}
</section>
<style>
.body { min-width:0; margin-top:var(--s3); display:grid; gap:var(--s3); }
.body > button { justify-self:start; }
.actions,.language { display:flex; align-items:center; flex-wrap:wrap; gap:var(--s2); }
details { padding:var(--s2) 0; border-bottom:1px solid var(--line); overflow-wrap:anywhere; }
details p { margin:var(--s1) 0 var(--s1) var(--s3); }
</style>
