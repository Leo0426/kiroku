<script lang="ts">
  import { setFavorite } from '../lib/favorite'
  import { fade } from 'svelte/transition'
  import { untrack,onDestroy } from 'svelte'
  import {listen} from '../lib/events'
  import {cancellableJob,type CancellableJob} from '../lib/cancellable-job'
  import { repairFlow } from '../lib/repairflow.svelte'
  import { api } from '../lib/api'
  import { baseName, fmtBytes } from '../lib/format'
  import { dur, slideIn } from '../lib/motion'
  import { app } from '../lib/state.svelte'
  import { CONTENT_TYPE_LABEL, SOURCE_NAMES, type MovieDetail } from '../lib/types'
  import Icon from './Icon.svelte'
  import FieldSources from './FieldSources.svelte'
  import MetadataEditor from './MetadataEditor.svelte'
  import ArtworkEditor from './ArtworkEditor.svelte'
  import AppendFiles from './AppendFiles.svelte'
  import MetadataUpdater from './MetadataUpdater.svelte'
  import LanguageExport from './LanguageExport.svelte'
  import LoadingStatus from './LoadingStatus.svelte'

  let { code, onclose }: { code: string; onclose: () => void } = $props()

  /** 海报裁剪弹窗;换影片时收起 */
  let cropping = $state(false)
  $effect(() => { code; cropping = false })

  let detail = $state<MovieDetail | null>(null)
  let loading = $state(true)
  let error = $state('')
  let attempt = $state(0)
  const repairHere=$derived(repairFlow.code===code && repairFlow.root===app.settings?.library_root)
  const repairRunning=$derived(repairHere && repairFlow.running)
  const repairResult=$derived(repairHere?repairFlow.result:null)
  const repairError=$derived(repairHere?repairFlow.error:'')
  let undoError=$state('')
  let generation=0
  $effect(()=>{code;app.settings?.library_root;untrack(()=>{
    generation++;undoError='';void mediaTask?.cancel();mediaTask=null;mediaBusy=false;mediaDone=0;mediaTotal=0
  })})
  async function repair(){await repairFlow.start(code)}
  async function undoRepair(){if(!repairResult?.journalId)return;const current=generation;try{const result=await api.rollback(repairResult.journalId);if(current!==generation)return;if(result.failures.length||result.indexError)throw [...result.failures,result.indexError].filter(Boolean).join('; ');repairFlow.clearResult();attempt++;app.toast('ok','本次补图已撤销')}catch(e){if(current===generation)undoError=String(e)}}

  let panel: HTMLElement | undefined = $state()
  /**
   * 面板滑入期间不响应按钮:双击海报时,第二下会落在刚滑到光标下的「播放」上,误打开视频。
   * 滑入结束(或兜底 450ms)后才可点。
   */
  let armed = $state(false)
  $effect(() => {
    const t = setTimeout(() => (armed = true), 450)
    return () => clearTimeout(t)
  })

  $effect(() => {
    const c = code
    app.settings?.library_root
    attempt
    app.libraryVersion
    if (untrack(() => detail?.code) !== c) { loading = true; detail = null }
    error = ''
    let alive = true
    api.libraryGet(c).then((d) => {
      if (!alive) return
      detail = d
      loading = false
    }).catch((e) => {
      if (!alive) return
      error = String(e)
      loading = false
    })
    return () => {
      alive = false
    }
  })

  $effect(() => {
    panel?.focus()
  })

  const poster = $derived(detail ? api.posterUrl(detail.poster, detail.posterVersion) : null)
  const fanart = $derived(detail ? api.posterUrl(detail.fanart ?? detail.poster, detail.fanart ? detail.fanartVersion : detail.posterVersion) : null)
  const meta = $derived(detail?.meta)

  let mediaBusy=$state(false),mediaDone=$state(0),mediaTotal=$state(0)
  let mediaTask=$state<CancellableJob<void>|null>(null)
  onDestroy(()=>{generation++;void mediaTask?.cancel()})
  let favoriteBusy = $state(false)
  async function toggleFavorite() {
    if (!detail || favoriteBusy) return
    const { code, favorite } = detail
    favoriteBusy = true
    try {
      await setFavorite(code, !favorite)
      app.toast('ok', favorite ? '已取消收藏，可从历史撤销' : '已收藏，可从历史撤销')
    } catch (e) { app.toast('error', String(e), 5000) }
    finally { favoriteBusy = false }
  }

  async function act(kind:'play'|'reveal',part?:number){
    if(!armed||mediaBusy)return
    const current=generation,requested=code,root=app.settings?.library_root;mediaBusy=true;mediaDone=0;mediaTotal=0
    let unlisten:(()=>void)|undefined
    let cancelNotice:number|null=null
    function clearCancelNotice(){if(cancelNotice!==null){app.dismiss(cancelNotice);cancelNotice=null}}
    const request=cancellableJob(async id=>{
      unlisten=await listen('media:progress',e=>{if(generation===current&&request.jobId===e.jobId&&e.code===requested&&e.root===root&&e.kind===kind){mediaDone=e.doneBytes;mediaTotal=e.totalBytes}})
      if(current!==generation||request.cancelled)throw '媒体校验已取消'
      await (kind==='play'?api.play(requested,part,id):api.reveal(requested,part,id))
    },e=>{
      // A failed stop still matters after this detail has closed or changed.
      // The retry captures this request, never the current panel's task.
      clearCancelNotice()
      cancelNotice=app.toast('error',`${requested}：停止媒体校验失败：${String(e)}`,0,{
        label:'重试停止',run:()=>{clearCancelNotice();void request.cancel()},
      })
    })
    mediaTask=request
    try{
      await request.promise
      if(current===generation&&kind==='play'&&!request.cancelled)app.toast('info','已交给系统播放器打开')
    }catch(e){if(current===generation)app.toast('error',String(e),5000)}
    finally{unlisten?.();clearCancelNotice();if(current===generation){mediaBusy=false;mediaTask=null;}}
  }

  function closeDetails() {
    // Destruction (and even outrostart) can lag behind the close action.
    // Record cancellation before a fast backend response can settle the job.
    generation++
    void mediaTask?.cancel()
    onclose()
  }

  function onkey(e: KeyboardEvent) {
    // 裁剪弹窗开着时 Esc 只关弹窗
    if (e.key === 'Escape' && !cropping) closeDetails()
  }
</script>

<svelte:window onkeydown={onkey} />

{#if cropping}
  <ArtworkEditor {code} onsaved={() => attempt++} onclose={() => (cropping = false)} />
{/if}

<div class="scrim" role="presentation" onclick={closeDetails} transition:fade={{ duration: dur(200) }}></div>

<aside class="panel" class:armed bind:this={panel} tabindex="-1" aria-label="影片详情" aria-busy={loading} transition:slideIn onintroend={() => (armed = true)}>
  <button class="close btn icon ghost" aria-label="关闭详情" onclick={closeDetails}><Icon name="x" size={18} /></button>

  {#if loading}
    <div class="skeleton"><div class="hero-sk" aria-hidden="true"></div><div class="body"><LoadingStatus label="正在读取影片详情…" /><div aria-hidden="true"><div class="l w60"></div><div class="l"></div><div class="l w80"></div></div></div></div>
  {:else if error}
    <div class="gone" role="alert"><p class="muted">无法读取影片详情：{error}</p><button class="btn" onclick={() => attempt++}>重试</button></div>
  {:else if !detail || !meta}
    <div class="gone"><p class="muted">这部影片已不在媒体库中。</p></div>
  {:else}
    <div class="hero" style:background-image={fanart ? `url("${fanart}")` : 'none'}>
      <div class="hero-fade"></div>
    </div>

    <div class="content stagger">
      <div class="top">
        <button class="poster" title="重新裁剪海报" aria-label="重新裁剪海报" onclick={() => (cropping = true)}>{#if poster}<img src={poster} alt="" />{/if}<span class="crop-hint" aria-hidden="true"><Icon name="pencil" size={14} />裁剪</span></button>
        <div class="head">
          <span class="code mono">{detail.code}</span>
          <h2>{meta.title ?? detail.code}</h2>
          {#if meta.original_title && meta.original_title !== meta.title}<p class="orig">{meta.original_title}</p>{/if}
          <div class="facts">
            {#if detail.year}<span>{meta.release_date ?? detail.year}</span>{/if}
            {#if meta.runtime_min}<span>{meta.runtime_min} 分钟</span>{/if}
            {#if detail.contentType}<span>{CONTENT_TYPE_LABEL[detail.contentType] ?? detail.contentType}</span>{/if}
            {#if meta.scores.length}
              <span class="score tnum"><Icon name="star" size={11} />{(meta.scores.reduce((a, s) => a + s.score, 0) / meta.scores.length / 10).toFixed(1)}</span>
            {/if}
          </div>
        </div>
      </div>

      <div class="cta">
        <button class="btn primary" onclick={() => act('play', detail!.files.length > 1 ? 1 : undefined)} disabled={!detail.files.length||mediaBusy}><Icon name="play" size={14} />播放</button>
        <button class="btn" disabled={mediaBusy} onclick={() => act('reveal')}><Icon name="folder" size={15} />在文件夹中显示</button>
        <button class="btn" onclick={repair} disabled={repairFlow.running}>{repairRunning ? '正在补图…' : '补缺图'}</button>
        <button class="btn" class:on={detail.favorite} aria-pressed={detail.favorite} disabled={favoriteBusy} onclick={toggleFavorite}><Icon name="star" size={14} />{detail.favorite ? '取消收藏' : '收藏'}</button>
        {#if repairRunning}<button class="btn" onclick={() => repairFlow.cancel()}>停止补图</button>{/if}
      </div>
      {#if mediaBusy}<div role="status" aria-label="媒体校验进度"><p>正在校验媒体来源…{mediaTotal?` ${Math.min(100,Math.round(mediaDone/mediaTotal*100))}%`:''}</p><button class="btn" onclick={()=>mediaTask?.cancel()}>取消媒体校验</button></div>{/if}
      {#if repairError||undoError}<p role="alert">补图失败：{repairError||undoError}</p>{/if}
      {#if repairResult}
        <div role="status" aria-label="补图结果">
          <p>{repairResult.cancelled ? '补图已停止' : repairResult.failed.length ? '补图失败' : '补图完成'}：写入 {repairResult.written} 张图片。</p>
          {#each repairResult.failed as failure}<p>{failure[0]}：{failure[1]}</p>{/each}
          {#each repairResult.warnings as warning}<p class="muted">{warning}</p>{/each}
          {#if repairResult.indexError}<p>图片已落盘，索引更新失败：{repairResult.indexError}</p>{/if}
          {#if repairResult.journalId && repairResult.written}<button class="btn" onclick={undoRepair}>撤销本次补图</button>{/if}
        </div>
      {/if}

      <div class="toolbox" role="region" aria-label="影片工具">
        <h3>影片工具</h3>
        <div class="tool-grid">
          {#key `${code}:${app.settings?.library_root}`}
            <MetadataEditor {meta} onsaved={() => attempt++} />
            <MetadataUpdater {meta} onsaved={() => attempt++} />
            <section aria-label="海报裁剪"><button class="btn" onclick={() => (cropping = true)}>裁剪海报</button></section>
            <AppendFiles {code} onsaved={() => attempt++} />
            <LanguageExport {meta} onsaved={() => attempt++} />
          {/key}
        </div>
      </div>

      {#if meta.plot}
        <section><h3>简介</h3><p class="plot">{meta.plot}</p></section>
      {/if}

      {#if meta.actors.length}
        <section>
          <h3>演员</h3>
          <div class="chips">{#each meta.actors as a}<span class="chip" class:male={a.gender === 'male'}>{#if a.thumb}<img class="avatar" src={a.thumb} alt="" referrerpolicy="no-referrer" loading="lazy" onerror={e => e.currentTarget.remove()} />{/if}{a.name}</span>{/each}</div>
        </section>
      {/if}

      {#if meta.tags.length}
        <section>
          <h3>类别</h3>
          <div class="chips">{#each meta.tags as t}<span class="chip subtle">{t}</span>{/each}</div>
        </section>
      {/if}

      <section>
        <h3>信息</h3>
        <dl>
          {#if meta.studio}<dt>片商</dt><dd>{meta.studio}</dd>{/if}
          {#if meta.publisher && meta.publisher !== meta.studio}<dt>发行商</dt><dd>{meta.publisher}</dd>{/if}
          {#if meta.series}<dt>系列</dt><dd>{meta.series}</dd>{/if}
          {#if meta.directors.length}<dt>导演</dt><dd>{meta.directors.join('、')}</dd>{/if}
          {#each meta.scores as s (s.site)}<dt>{SOURCE_NAMES[s.site] ?? s.site} 评分</dt><dd class="tnum">{(s.score / 10).toFixed(1)} / 10</dd>{/each}
        </dl>
      </section>

      <section>
        <h3>文件</h3>
        <ul class="files">
          {#each detail.files as f (f.path)}
            <li>
              <span class="fname" title={f.path}>{baseName(f.path)}</span>
              <span class="faint tnum">{fmtBytes(f.size)}</span>
              {#if detail.files.length > 1}<button class="btn sm ghost" onclick={() => act('play', f.part ?? undefined)}><Icon name="play" size={11} />播放</button>{/if}
            </li>
          {/each}
        </ul>
        <p class="faint dir mono" title={detail.dir}>{detail.dir}</p>
      </section>
      <FieldSources {meta} />
    </div>
  {/if}
</aside>

<style>
  .scrim { position: fixed; inset: 0; z-index: 40; background: var(--scrim); backdrop-filter: blur(3px); }
  /* 滑入完成前按钮不接收点击(见 armed) */
  .panel:not(.armed) :global(button) { pointer-events: none; }
  .panel {
    position: fixed; z-index: 41; top: 0; right: 0; bottom: 0; width: min(540px, 100%);
    background: var(--surface-1); box-shadow: var(--shadow-3);
    overflow-y: auto; outline: none;
  }
  .close:hover :global(svg) { transform: rotate(90deg); }
  .close { position: absolute; z-index: 3; top: var(--s3); right: var(--s3); background: rgb(12 12 16 / 0.5); backdrop-filter: blur(10px); color: #fff; }
  .hero { height: 220px; background-size: cover; background-position: center 30%; position: relative; filter: saturate(1.1); }
  .hero::after { content: ''; position: absolute; inset: 0; backdrop-filter: blur(22px) brightness(0.7); }
  .hero-fade { position: absolute; inset: 0; z-index: 1; background: linear-gradient(to bottom, transparent 30%, var(--surface-1)); }
  .content { position: relative; z-index: 2; margin-top: -92px; padding: 0 var(--s5) var(--s7); display: flex; flex-direction: column; gap: var(--s4); }
  .top { display: flex; gap: var(--s4); align-items: flex-end; }
  .poster { flex-shrink: 0; width: 132px; aspect-ratio: var(--poster-ratio); border-radius: var(--r-md); overflow: hidden; background: var(--surface-3); box-shadow: var(--shadow-3); transition: transform var(--d-slow) var(--ease-back); }
  .poster:hover { transform: translateY(-4px) rotate(-1.5deg) scale(1.03); }
  .poster img { width: 100%; height: 100%; object-fit: cover; display: block; }
  .poster { position: relative; padding: 0; border: 0; cursor: pointer; }
  .crop-hint { position: absolute; left: 50%; bottom: var(--s2); transform: translateX(-50%); display: inline-flex; align-items: center; gap: 4px; padding: 3px 9px; border-radius: var(--r-pill); font-size: var(--fs-xs); color: #fff; background: rgb(12 12 16 / 0.62); backdrop-filter: blur(8px); opacity: 0; transition: opacity var(--d-base) var(--ease-out); }
  .poster:hover .crop-hint, .poster:focus-visible .crop-hint { opacity: 1; }
  .head { min-width: 0; padding-bottom: 2px; }
  .code { color: var(--accent); font-weight: 600; font-size: var(--fs-sm); }
  h2 { font-size: var(--fs-xl); line-height: 1.3; font-weight: 700; margin: 2px 0; letter-spacing: -0.005em; }
  .orig { color: var(--text-2); font-size: var(--fs-sm); }
  .facts { display: flex; flex-wrap: wrap; gap: var(--s1) var(--s3); margin-top: var(--s2); color: var(--text-2); font-size: var(--fs-sm); }
  .facts span + span::before { content: '·'; margin-right: var(--s3); color: var(--text-3); }
  .score { color: var(--accent); display: inline-flex; align-items: center; gap: 3px; }
  .toolbox { padding: var(--s3); border: 1px solid var(--line); border-radius: var(--r-md); }
  .tool-grid { display: grid; grid-template-columns: minmax(0, 1fr) minmax(0, 1fr); gap: var(--s2); align-items: start; }
  .tool-grid > :global(section) { min-width: 0; margin: 0; }
  .tool-grid > :global(section.expanded) { grid-column: 1 / -1; padding: var(--s3); border-radius: var(--r-sm); background: var(--surface-2); }
  .tool-grid > :global(section:not(.expanded) > button:first-child) { width: 100%; margin: 0; text-align: left; justify-content: flex-start; }
  .tool-grid :global(input:not([type=checkbox])), .tool-grid :global(textarea), .tool-grid :global(select) { max-width: 100%; min-width: 0; }
  .tool-grid :global(p), .tool-grid :global(ins), .tool-grid :global(del) { overflow-wrap: anywhere; }
  .tool-grid :global(button) { max-width: 100%; white-space: normal; }
  .cta { display: flex; gap: var(--s2); flex-wrap: wrap; }
  h3 { font-size: var(--fs-xs); font-weight: 600; letter-spacing: 0.06em; color: var(--text-3); margin-bottom: var(--s2); text-transform: uppercase; }
  .plot { color: var(--text-1); font-size: var(--fs-md); line-height: 1.75; white-space: pre-wrap; }
  .chips { display: flex; flex-wrap: wrap; gap: var(--s2); }
  .chip { padding: 4px 11px; border-radius: var(--r-pill); background: var(--accent-soft); color: var(--accent); font-size: var(--fs-sm); font-weight: 500; transition: transform var(--d-base) var(--ease-back), background var(--d-fast); }
  .chip:hover { transform: translateY(-2px); }
  .chip.male { background: var(--info-soft); color: var(--info); }
  .chip:has(.avatar) { display: inline-flex; align-items: center; gap: 6px; padding-left: 4px; }
  .avatar { width: 20px; height: 20px; border-radius: 50%; object-fit: cover; }
  .chip.subtle { background: var(--surface-3); color: var(--text-2); }
  dl { display: grid; grid-template-columns: auto 1fr; gap: var(--s2) var(--s5); margin: 0; font-size: var(--fs-sm); }
  dt { color: var(--text-3); }
  dd { margin: 0; }
  .files { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: var(--s1); }
  .files li { display: flex; align-items: center; gap: var(--s3); padding: var(--s2) var(--s3); background: var(--surface-2); border-radius: var(--r-sm); font-size: var(--fs-sm); transition: background var(--d-fast), transform var(--d-base) var(--ease-out); }
  .files li:hover { background: var(--surface-3); transform: translateX(2px); }
  .fname { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .dir { margin-top: var(--s2); font-size: var(--fs-xs); word-break: break-all; }
  .skeleton .hero-sk { height: 220px; background: var(--surface-2); animation: pulse 1.4s ease-in-out infinite; }
  .skeleton .body { padding: var(--s5); display: flex; flex-direction: column; gap: var(--s3); }
  .l { height: 14px; border-radius: 7px; background: var(--surface-3); animation: pulse 1.4s ease-in-out infinite; }
  .w60 { width: 60%; } .w80 { width: 80%; }
  .gone { padding: var(--s7) var(--s5); text-align: center; }
  @media (max-width: 720px) { .panel { bottom: calc(64px + env(safe-area-inset-bottom)); } .top { flex-direction: column; align-items: flex-start; } .poster { width: 112px; } }
</style>
