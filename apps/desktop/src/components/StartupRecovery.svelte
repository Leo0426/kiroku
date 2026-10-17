<script lang="ts">
 import Icon from './Icon.svelte'
 import {app} from '../lib/state.svelte'
 import {api} from '../lib/api'
 import type {RecoveryEntry} from '../lib/types'
 const recovery=app.recovery
 let expanded=$state(false)
 $effect(()=>{if(recovery.error)expanded=true})
 async function undo(entry:RecoveryEntry){
  const root=recovery.root
  if(await app.confirm({title:'检查并撤销中断操作',body:'将重新检查日志与磁盘，只撤销本次拥有且未被外部修改的产物。损坏或无法确认的文件会保留并报告。',confirmLabel:'确认撤销'})){
   if(root!==app.settings?.library_root)return
   try{await recovery.undo(entry)}catch(e){app.toast('error',String(e))}
  }
 }
 async function reveal(id:string){try{await api.revealJournal(id)}catch(e){app.toast('error',String(e))}}
</script>
{#if recovery.root===app.settings?.library_root&&(recovery.entries.length||recovery.error||recovery.loading)}
<section aria-label="启动恢复" class="recovery">
 <div class="summary">
  <Icon name="alert" size={16}/>
  <div class="summary-text"><strong>{recovery.loading&&!recovery.entries.length?'正在检查整理日志':`${recovery.entries.length} 条整理记录待处理`}</strong>
   {#if recovery.stale}<span>上次记录，尚未核对</span>{:else if recovery.entries.length}<span>{recovery.entries.slice(0,2).flatMap(e=>e.codes).join('、')||'请检查中断操作'}</span>{/if}
  </div>
  <button class="btn sm ghost" aria-expanded={expanded} aria-controls="recovery-details" onclick={()=>expanded=!expanded}>{expanded?'收起恢复事项':'展开恢复事项'}</button>
 </div>
 {#if recovery.error}<p class="error" role="alert">{recovery.error}</p>{/if}
 <div id="recovery-details" class="details" hidden={!expanded}>
  <div class="detail-actions"><button class="btn sm" onclick={()=>recovery.refresh()} disabled={recovery.loading||!!recovery.busyId}>{recovery.loading?'正在核对…':'重新检查恢复事项'}</button><button class="btn sm" onclick={()=>app.navigate('history')}>查看完整历史</button></div>
  {#if recovery.stale}<p>媒体库不可用或检查未完成时，不执行撤销。</p>{/if}
  <p class="muted root">媒体库：{recovery.root}</p>
 {#each recovery.entries as entry(entry.id)}
 <div class="entry"><strong>{entry.codes.join('、')||`日志 ${entry.id}`}</strong><p>{entry.message}</p>
 <button class="btn sm" onclick={()=>reveal(entry.id)} disabled={recovery.stale||recovery.loading||!/^\d+$/.test(entry.id)}>查看日志位置</button>
 {#if entry.canUndo}<button class="btn sm" disabled={recovery.stale||recovery.loading||!!recovery.busyId} onclick={()=>undo(entry)}>{recovery.busyId===entry.id?'正在撤销…':'检查并撤销'}</button>{/if}</div>
 {/each}
 </div>
</section>
{/if}
<style>
 .recovery { flex-shrink:0; margin:16px 24px 0; border:1px solid color-mix(in srgb,var(--warn) 24%,var(--line)); border-radius:var(--r-md); background:color-mix(in srgb,var(--warn-soft) 45%,var(--surface-1)); font-size:var(--fs-sm); }
 .summary { display:flex; align-items:center; gap:12px; padding:10px 14px; min-height:44px; }
 .summary :global(svg) { color:var(--warn); flex-shrink:0; }
 .summary-text { display:flex; align-items:baseline; gap:12px; flex:1; min-width:0; flex-wrap:wrap; }
 .summary-text strong { font-weight:600; }
 .summary-text span { color:var(--text-2); overflow-wrap:anywhere; }
 .details { padding:4px 16px 16px; max-height:32vh; overflow:auto; border-top:1px solid var(--line); }
 .details[hidden] { display:none; }
 .detail-actions { display:flex; gap:8px; flex-wrap:wrap; margin:10px 0; }
 .entry { display:flex; align-items:center; flex-wrap:wrap; gap:8px; padding:12px 0; border-top:1px solid var(--line); }
 .entry strong { flex-basis:100%; }
 .entry p { flex-basis:100%; color:var(--text-2); }
 p { margin:4px 0; overflow-wrap:anywhere; }
 .error { padding:0 16px 12px; color:var(--danger); }
 .root { font-size:var(--fs-xs); }
 @media(max-width:720px) { .recovery { margin:12px 16px 0; } .summary-text { gap:4px; } }
</style>
