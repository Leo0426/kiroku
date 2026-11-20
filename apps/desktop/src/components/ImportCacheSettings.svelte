<script lang="ts">
 import {onMount,onDestroy} from 'svelte'
 import {api} from '../lib/api'
 import {app} from '../lib/state.svelte'
 import {cancellableJob,type CancellableJob} from '../lib/cancellable-job'
 import type {CacheUsage} from '../lib/types'
 let { settingsPath }: { settingsPath: string } = $props()
 const directory = $derived(settingsPath.replace(/settings\.json$/, 'cache/import-v1'))
 let usage=$state<CacheUsage|null>(null),busy=$state(false),error=$state(''),clearing=$state(false)
 let alive=true,task:CancellableJob<CacheUsage>|null=null
 const size=(n:number)=>`${(n/1024/1024).toFixed(1)} MiB`
 async function refresh(){
  if(busy)return
  busy=true;error=''
  const job=cancellableJob(id=>api.importCacheUsage(id),e=>{if(alive)error=String(e)});task=job
  try{const result=await job.promise;if(alive&&!job.cancelled)usage=result}
  catch(e){if(alive)error=String(e)}finally{if(alive){busy=false;task=null}}
 }
 async function clear(){
  if(busy||clearing)return
  if(!await app.confirm({title:'清空抓取缓存',body:'将清除本机已缓存的来源信息和下载图片，下次抓取会重新联网。媒体文件、整理日志和 AI 译文缓存不受影响。',confirmLabel:'清空缓存'}))return
  if(!alive)return
  clearing=true;error=''
  try{await api.clearImportCache();app.toast('ok','抓取缓存已清空');if(alive)await refresh()}
  catch(e){if(alive)error=String(e)}finally{if(alive)clearing=false}
 }
 onMount(()=>{void refresh()})
 onDestroy(()=>{alive=false;void task?.cancel()})
</script>
<section class="card" id="import-cache" aria-label="抓取缓存">
 <h2>抓取缓存</h2>
 <p class="hint">成功获取的来源信息和图片自动保存在本机。意外退出后，重新扫描并开始抓取即可复用；失败的部分会重试。清空缓存不会删除媒体、整理日志或译文。</p>
 {#if directory}<p class="mono directory" aria-label="抓取缓存目录">{directory}</p>{/if}
 {#if usage}<p aria-label="缓存占用">来源 {usage.sources.files} 项 · {size(usage.sources.bytes)}；图片 {usage.images.files} 项 · {size(usage.images.bytes)}（文件逻辑长度，不含系统缓存）</p>{/if}
 <button class="btn sm" disabled={busy||clearing} onclick={clear}>{clearing?'正在清空…':'清空抓取缓存'}</button>
 <button class="btn sm ghost" disabled={busy||clearing} onclick={refresh}>刷新占用</button>
 {#if busy}<button class="btn sm ghost" onclick={()=>task?.cancel()}>停止统计</button>{/if}
 {#if error}<p role="alert">{error}</p>{/if}
</section>
<style>section{padding:20px}p{margin:10px 0;font-size:var(--fs-sm);color:var(--text-2)}.directory{overflow-wrap:anywhere}button{margin-right:8px}[role=alert]{color:var(--danger)}</style>
