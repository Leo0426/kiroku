<script lang="ts">
  import { onDestroy, untrack } from 'svelte'
  import { api } from '../lib/api'
  import { app } from '../lib/state.svelte'
  import { fmtBytes } from '../lib/format'
  import { listen } from '../lib/events'
  import { cancellableJob, type CancellableJob } from '../lib/cancellable-job'
  import { resolveExportCodes, type ExportScope as Scope } from '../lib/export-scope'
  import type { PackageExportPreview, PackageExportResult, PackageFileKind } from '../lib/package-export'
  import type { Query } from '../lib/types'
  import ExportScope from './ExportScope.svelte'
  import LoadingStatus from './LoadingStatus.svelte'
  import ProgressBar from './ProgressBar.svelte'
  import Icon from './Icon.svelte'

  let {query={}, code, onactive}:{query?:Query;code?:string;onactive?:(active:boolean)=>void}=$props()
  const options:{kind:PackageFileKind;label:string}[]=[
    {kind:'nfo',label:'NFO'},{kind:'sidecar',label:'kiroku.json（含收藏与人工锁定）'},
    {kind:'poster',label:'海报'},{kind:'fanart',label:'背景图'},{kind:'avatar',label:'本地演员头像'},
    {kind:'subtitle',label:'字幕'},{kind:'video',label:'视频'},
  ]
  let opened=$state(false), busy=$state(false), error=$state(''),directory=$state('')
  let scope=$state<Scope>({kind:'filtered'})
  let kinds=$state<PackageFileKind[]>(['nfo','sidecar','poster','fanart','avatar'])
  let preview=$state.raw<PackageExportPreview|null>(null),result=$state.raw<PackageExportResult|null>(null)
  let task=$state.raw<CancellableJob<unknown>|null>(null),controller:AbortController|null=null
  let progress=$state(0),status=$state('')
  let generation=0
  function stop(){controller?.abort();void task?.cancel()}
  function close(){generation++;stop();opened=false;busy=false;preview=null;result=null;error=''}
  $effect(()=>{code;JSON.stringify(query);app.settings?.library_root;untrack(close)})
  $effect(()=>{onactive?.(opened || busy)})
  onDestroy(()=>{generation++;stop()})
  $effect(()=>{
    let alive=true,off:(()=>void)|undefined
    listen('package:progress',e=>{if(task?.jobId===e.jobId)progress=e.done}).then(fn=>{if(alive)off=fn;else fn()})
    return ()=>{alive=false;off?.()}
  })
  async function choose(){
    const g=generation
    try{const path=await api.pickDirectory('选择资料包保存位置',directory||undefined);if(g===generation&&path)directory=path}
    catch(e){if(g===generation)error=String(e)}
  }
  async function inspect(){
    if(busy||!directory||!kinds.length)return
    const g=generation
    busy=true;error='';status='正在读取所选影片…'
    const abort=new AbortController();controller=abort
    try{
      const codes=code?[code]:await resolveExportCodes($state.snapshot(scope),$state.snapshot(query),abort.signal)
      if(g!==generation)return
      abort.signal.throwIfAborted()
      status='正在核对文件和容量…'
      const job=cancellableJob(id=>api.packagePreview(codes,[...kinds],directory,id),e=>{if(g===generation)error=String(e)});task=job
      const p=await job.promise
      if(g===generation&&!job.cancelled)preview=p
    }catch(e){if(g===generation)error=abort.signal.aborted?'已停止预览':String(e)}
    finally{if(g===generation){busy=false;task=null;controller=null}}
  }
  async function save(){
    if(!preview||busy)return
    const g=generation, token=preview.token
    busy=true;error='';status='正在复制并校验文件…';progress=0
    const job=cancellableJob(id=>api.packageExport(token,id),e=>{if(g===generation)error=String(e)});task=job
    try{const completed=await job.promise;if(g===generation){result=completed;preview=null}}
    catch(e){if(g===generation){error=String(e);preview=null}}
    finally{if(g===generation){busy=false;task=null}}
  }
</script>

<section aria-label="导出资料包" class="package-export" class:expanded={opened}>
  <button class="btn trigger" aria-expanded={opened} onclick={()=>opened?close():opened=true}><Icon name="folder" size={15}/>导出资料包<Icon name="chevron-down" size={15}/></button>
  {#if opened}
    <div class="tool-content body">
      {#if result}
        <p role="status">已导出 {result.movies} 部影片，{result.files} 个文件 · {fmtBytes(result.bytes)}</p>
        <p class="mono path">{result.destination}</p>
        <button class="btn" onclick={()=>{result=null;error=''}}>继续导出</button>
      {:else if preview}
        <p><strong>将复制 {preview.movies} 部影片</strong> · {preview.files} 个文件 · {fmtBytes(preview.bytes)}</p>
        <p class="mono path">{preview.destination}</p>
        <details><summary>查看文件示例</summary>{#each preview.examples as path}<p class="mono path">{path}</p>{/each}</details>
        {#if preview.warningCount}<details><summary>有 {preview.warningCount} 项缺失或跳过</summary>{#each preview.warnings as line}<p>{line}</p>{/each}</details>{/if}
        <div class="actions"><button class="btn" disabled={busy} onclick={()=>preview=null}>返回修改</button>
          <button class="btn primary" disabled={busy} onclick={save}>确认导出副本</button></div>
      {:else}
        {#if !code}<ExportScope {query} bind:scope disabled={busy}/>{/if}
        <fieldset disabled={busy}><legend>复制内容</legend>
          <div class="choices">{#each options as opt}<label><input class="checkbox" type="checkbox" value={opt.kind} bind:group={kinds}/>{opt.label}</label>{/each}</div>
        </fieldset>
        <div class="destination"><button class="btn" disabled={busy} onclick={choose}><Icon name="folder" size={15}/>{directory?'更换保存位置':'选择保存位置'}</button>
          {#if directory}<span class="mono path">{directory}</span>{/if}</div>
        <button class="btn primary" disabled={busy||!directory||!kinds.length||(!code&&scope.kind==='selection'&&!scope.codes.length)} onclick={inspect}>预览资料包</button>
      {/if}
      {#if busy}<LoadingStatus label={status}/>{#if preview}<ProgressBar label="资料包导出进度" active value={preview.bytes?progress/preview.bytes:0}/>{/if}
        <button class="btn sm" onclick={stop}>停止任务</button>{/if}
      {#if error}<p role="alert">{error}</p>{/if}
    </div>
  {/if}
</section>

<style>
.package-export { min-width:0; }
.body { display:grid; gap:var(--s3); margin-top:var(--s2); padding:var(--s3); background:var(--surface-2); border-radius:var(--r-sm); }
.body>button { justify-self:start; }
fieldset { margin:0; padding:0; border:0; min-width:0; }
legend { font-weight:600; margin-bottom:var(--s2); }
.choices { display:grid; grid-template-columns:repeat(auto-fit,minmax(min(100%,180px),1fr)); gap:var(--s1) var(--s3); }
label,.destination,.actions { display:flex; align-items:center; gap:var(--s2); }
label { min-height:28px; }
.destination,.actions { flex-wrap:wrap; }
.path { overflow-wrap:anywhere; font-size:var(--fs-sm); }
summary { cursor:pointer; }
[role=alert] { color:var(--danger); }
</style>
