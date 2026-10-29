<script lang="ts">
  import { api } from '../lib/api'
  import { app } from '../lib/state.svelte'
  import type { HistoryDetail } from '../lib/types'
  let { id }: { id:string } = $props()
  let data = $state<HistoryDetail | null>(null)
  let busy = $state(false)
  let error = $state('')
  let request:string | null = null
  let generation = 0
  let offset = $state(0)
  function stop() { generation++; if (request) void api.historyCancel(request).catch(()=>{}); request=null; busy=false }
  async function load(at=0) {
    stop(); const current=++generation, requestId=crypto.randomUUID(); request=requestId
    busy=true; error=''
    try { const result=await api.historyDetail(requestId,id,at); if(current===generation){data=result;offset=at} }
    catch(e){if(current===generation)error=String(e)}
    finally{if(current===generation){busy=false;request=null}}
  }
  $effect(() => { id;app.settings?.library_root;app.libraryVersion;void load();return stop })
  const labels = { recorded:'日志已记录',undone:'已撤销',pending:'中断待核对',trashed:'废纸篓中，需手动恢复' }
</script>
<section aria-label="历史操作详情" aria-busy={busy}>
  <p class="muted">文件操作摘要；“日志已记录”不代表当前磁盘状态已重新检查。</p>
  {#if busy}<p role="status">正在读取详情… <button class="btn sm" onclick={() => {stop();error='读取已停止，可重试。'}}>停止读取详情</button></p>{/if}
  {#if error}<p role="alert">{error}</p><button class="btn sm" disabled={busy} onclick={() => load(offset)}>重试详情</button>{/if}
  {#if data}
    <ol start={offset+1}>
      {#each data.operations as op (op.index)}<li><span>{op.description}</span><strong>{labels[op.state]}</strong></li>{/each}
    </ol>
    <div class="pager"><span>{data.total ? offset+1 : 0}–{Math.min(offset+data.operations.length,data.total)} / {data.total} 项</span><button class="btn sm" disabled={busy||offset===0} onclick={() => load(Math.max(0,offset-100))}>上一页操作</button><button class="btn sm" disabled={busy||data.next===null} onclick={() => load(data!.next!)}>下一页操作</button></div>
  {/if}
</section>
<style>
  section { flex-basis:100%; min-width:0; padding:var(--s4); border-radius:var(--r-sm); background:var(--surface-2); font-size:var(--fs-sm); }
  ol { padding-left:24px; max-height:320px; overflow:auto; } li { padding:8px 0; overflow-wrap:anywhere; }
  li strong { display:block; color:var(--text-2); font-weight:400; font-size:var(--fs-xs); }
  .pager { display:flex; align-items:center; flex-wrap:wrap; gap:var(--s2); }
</style>
