<script lang="ts">
 import {onDestroy} from 'svelte'
 import {api} from '../lib/api'
 import {app} from '../lib/state.svelte'
 import type {AppendPreview,AppendResult} from '../lib/types'
 let {code,onsaved}:{code:string;onsaved:()=>void}=$props()
 let opened=$state(false),busy=$state(false),error=$state(''),inputs=$state<string[]>([]),targets=$state<string[]>([]),target=$state(''),preview=$state<AppendPreview|null>(null),result=$state<AppendResult|null>(null),notice=$state('')
 let videos=$state<string[]>([]),video=$state('');
 let generation=0;onDestroy(()=>generation++);$effect(()=>{code;app.settings;generation++;preview=null;inputs=[];opened=false;result=null;error='';busy=false})
 async function start(){opened=true;const g=generation;busy=true;try{const dirs=await api.appendTargets(code);if(g===generation){targets=dirs;target=dirs.length===1?dirs[0]:'';if(target){const choices=await api.appendTargetVideos(code,target);if(g===generation)videos=choices}}}catch(e){if(g===generation)error=String(e)}finally{if(g===generation)busy=false}}
 async function chooseTarget(next:string){const g=++generation;target=next;video='';preview=null;result=null;error='';videos=[];busy=true;try{if(next){const choices=await api.appendTargetVideos(code,next);if(g===generation)videos=choices}}catch(e){if(g===generation)error=String(e)}finally{if(g===generation)busy=false}}
 function chooseVideo(next:string){video=next;preview=null;result=null;error=''}
 async function pick(folder=false){const g=generation;busy=true;preview=null;try{const next=folder?[await api.pickDirectory('选择本影片追加文件所在目录')].filter(Boolean) as string[]:await api.pickMediaFiles();if(g===generation)inputs=next}catch(e){if(g===generation)error=String(e)}finally{if(g===generation)busy=false}}
 async function inspect(){const g=generation;busy=true;error='';preview=null;result=null;try{const next=await api.appendPreview(code,inputs,target||undefined,video||undefined);if(g===generation)preview=next}catch(e){if(g===generation)error=String(e)}finally{if(g===generation)busy=false}}
 async function save(){if(!preview)return;const g=generation,p=preview;busy=true;error='';try{const next=await api.appendApply(p.token);if(g===generation){result=next;preview=null;onsaved()}}catch(e){if(g===generation){error=String(e);preview=null}}finally{if(g===generation)busy=false}}
 async function undo(){if(!result)return;const g=generation;busy=true;try{const r=await api.rollback(result.journalId);if(g!==generation)return;if(r.failures.length||r.indexError)throw r.failures.join('; ')+String(r.indexError??'');result=null;notice='本次追加已撤销';onsaved()}catch(e){if(g===generation)error=String(e)}finally{if(g===generation)busy=false}}
</script>
<section class:expanded={opened} aria-label="追加影片文件">
 {#if !opened}<button class="btn" onclick={start}>追加视频或字幕</button>{:else}
 <h3>追加视频或字幕</h3><p>保留现有元数据和海报</p>
 <select class="input" aria-label="追加目标目录" value={target} onchange={e=>chooseTarget(e.currentTarget.value)} disabled={busy}><option value="">选择已有影片目录</option>{#each targets as dir}<option value={dir}>{dir}</option>{/each}</select>
 <select class="input" aria-label="字幕目标视频" value={video} onchange={e=>chooseVideo(e.currentTarget.value)} disabled={busy}><option value="">按字幕番号和分片自动匹配</option>{#each videos as v}<option value={v}>{v}</option>{/each}</select>
 <button class="btn" onclick={()=>pick()} disabled={busy}>选择文件</button><button class="btn" onclick={()=>pick(true)} disabled={busy}>选择追加文件夹</button>
 {#each inputs as input}<p class="muted">{input}</p>{/each}
 <button class="btn" onclick={inspect} disabled={busy||!inputs.length||!target}>预览追加</button>
 {#if preview}{#each preview.ops as op}<p>{Object.values(op)[0]?.from} → {Object.values(op)[0]?.to}</p>{/each}{#each preview.skipped as reason}<p>{reason}</p>{/each}{#each preview.conflicts as conflict}<p role="alert">{conflict}</p>{/each}<button class="btn primary" onclick={save} disabled={busy||!preview.ops.length||!!preview.conflicts.length}>确认追加</button>{/if}
 {#if result}<p role="status">{result.cancelled?'追加已取消':'追加完成'}：{result.completed} 个文件</p>{#each result.failed as failure}<p role="alert">{failure}</p>{/each}{#if result.completed}<button class="btn" onclick={undo} disabled={busy}>撤销本次追加</button>{/if}{/if}
 {#if busy}<button class="btn" onclick={()=>api.jobCancel()}>取消追加任务</button>{/if}
 <button class="btn ghost" onclick={()=>{opened=false;preview=null}} disabled={busy}>关闭追加</button>
 {/if}
 {#if notice}<p role="status">{notice}</p>{/if}{#if error}<p role="alert">{error}</p>{/if}
</section>
<style>section{margin:0}p{overflow-wrap:anywhere}button{margin:4px 4px 4px 0}select{max-width:100%}</style>
