<script lang="ts">
  import { onDestroy, untrack } from 'svelte'
  import { api } from '../lib/api'
  import { app } from '../lib/state.svelte'
  import { cancellableJob, type CancellableJob } from '../lib/cancellable-job'
  import type { SupplementKind, SupplementResult, SupplementReview } from '../lib/types'
  import Icon from './Icon.svelte'
  import LoadingStatus from './LoadingStatus.svelte'

  let { code, itemId, onsaved, onactive, onbusy }: { code: string; itemId?: number; onsaved?: () => void; onactive?: (active: boolean) => void; onbusy?: (active: boolean) => void } = $props()
  const roles: { kind: SupplementKind; label: string }[] = [{kind:'nfo',label:'NFO'}, {kind:'poster',label:'海报'}, {kind:'fanart',label:'背景图'}, {kind:'actors',label:'演员头像'}]
  let opened = $state(false), busy = $state(''), error = $state('')
  let review = $state<SupplementReview | null>(null), result = $state<SupplementResult | null>(null)
  let selected = $state<SupplementKind[]>([])
  let task = $state<CancellableJob<unknown> | null>(null)
  let generation = 0
  const count = $derived(review?.entries.filter(e => e.status === 'missing' && selected.includes(e.kind)).length ?? 0)
  function retire() {
    generation++
    void task?.cancel()
    opened = false; busy = ''; error = ''; task = null; result = null; review = null; selected = []
  }
  onDestroy(() => { retire(); onbusy?.(false) })
  $effect(() => { code; itemId; app.settings?.library_root; untrack(retire) })
  $effect(() => { onactive?.(opened || !!busy) })
  $effect(() => { onbusy?.(!!busy) })
  async function inspect() {
    if (busy) return
    const current=++generation
    busy='正在检查影片目录…'; error=''; review=null; result=null
    const request=cancellableJob(id=>api.supplementPreview(code,itemId,id),e=>app.toast('error',String(e)))
    task=request
    try {
      const value=await request.promise
      if (current!==generation || request.cancelled) return
      review=value
      selected=roles.filter(r=>value.entries.some(e=>e.kind===r.kind && e.status==='missing')).map(r=>r.kind)
    } catch(e) { if(current===generation) error=String(e) }
    finally { if(current===generation){task=null;busy=''} }
  }
  async function apply() {
    if(!review || !count || busy) return
    const current=++generation, token=review.token, kinds=[...selected]
    busy='正在写入缺少的资料…'; error=''
    const request=cancellableJob(id=>api.supplementApply(token,kinds,id),e=>app.toast('error',String(e)))
    task=request
    try {
      const done=await request.promise
      if(current!==generation) return
      review=null; result=done
      if (done.written.length) onsaved?.()
    } catch(e) { if(current===generation){review=null;error=String(e)} }
    finally { if(current===generation){task=null;busy=''} }
  }
  async function undo() {
    if(!result?.journalId || busy) return
    const current=++generation, journal=result.journalId
    busy='正在撤销本次补资料…'; error=''
    try {
      const done=await api.rollback(journal)
      if(current!==generation) return
      if(done.failures.length || done.indexError) throw [...done.failures,done.indexError].filter(Boolean).join(' · ')
      result=null; app.toast('ok','本次补资料已撤销'); onsaved?.()
    } catch(e){if(current===generation) error=String(e)}
    finally{if(current===generation) busy=''}
  }
  function toggle() {
    opened=!opened
    if(opened && !result && !review) void inspect()
  }
</script>

<section aria-label="只补资料">
  <button type="button" class="btn" aria-expanded={opened} disabled={!!busy} onclick={toggle}><Icon name="plus" size={15}/>只补资料<Icon name="chevron-down" size={15}/></button>
  {#if opened}
    <div class="tool-content supplement">
      <h3>只补资料</h3>
      <p class="muted">勾选要补的文件。已有文件和原视频保留在原处。</p>
      {#if busy}
        <LoadingStatus label={busy} />
        {#if task}<button class="btn sm" onclick={() => task?.cancel()}>停止补资料任务</button>{/if}
      {:else if review}
        <p class="path mono">{review.dir}</p>
        <fieldset><legend>补充项目</legend>
          {#each roles as role}
            {@const entries=review.entries.filter(e=>e.kind===role.kind)}
            {@const missing=entries.some(e=>e.status==='missing')}
            <div class="entry">
              <label><input type="checkbox" class="checkbox" value={role.kind} bind:group={selected} disabled={!missing}/>{role.label}</label>
              {#each entries as entry}
                <div class="file"><span class:ready={entry.status==='missing'}>{entry.reason}</span>{#if entry.path}<span class="mono">{entry.path}</span>{/if}</div>
              {/each}
            </div>
          {/each}
        </fieldset>
        <div class="tool-actions"><button class="btn primary" disabled={!count} onclick={apply}>确认补齐 {count} 个文件</button><button class="btn" onclick={inspect}>重新检查</button></div>
      {:else}
        {#if result}
          <p role="status">{result.cancelled ? '补资料已停止' : '检查与写入已结束'}，已写入 {result.written.length} 个文件。</p>
          {#each result.skipped as message}<p class="muted">{message}</p>{/each}
          {#if result.journalId}<button class="btn" onclick={undo}>撤销本次补资料</button>{/if}
        {/if}
        <button class="btn" onclick={inspect}>重新检查</button>
      {/if}
      {#if error}<p role="alert">{error}</p>{/if}
    </div>
  {/if}
</section>
<style>
  .supplement { min-width:0; }
  .supplement p { margin:var(--s2) 0; overflow-wrap:anywhere; }
  fieldset { min-width:0; margin:var(--s3) 0; border:0; padding:0; }
  legend { color:var(--text-2); font-size:var(--fs-sm); }
  .entry { display:grid; gap:var(--s1); padding:var(--s2) 0; border-bottom:1px solid var(--line); }
  .entry label { display:flex; gap:var(--s2); align-items:center; font-weight:600; }
  .file { display:grid; gap:2px; padding-inline-start:24px; overflow-wrap:anywhere; font-size:var(--fs-xs); color:var(--text-2); }
  .file .ready { color:var(--accent); }
  .file .mono,.path { color:var(--text-3); font-size:var(--fs-xs); }
  .tool-actions { display:flex; align-items:center; flex-wrap:wrap; gap:var(--s2); }
</style>
