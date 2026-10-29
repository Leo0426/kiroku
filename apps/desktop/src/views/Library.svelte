<script lang="ts">
  import { onDestroy, untrack, tick } from 'svelte'
  import BatchEditor from '../components/BatchEditor.svelte'
  import LibraryOverview from '../components/LibraryOverview.svelte'
  import LibraryBrowserControls from '../components/LibraryBrowserControls.svelte'
  import { readBrowser, writeBrowser, cleanQuery } from '../lib/library-browser'
  import { cancellableJob, type CancellableJob } from '../lib/cancellable-job'
  import LibraryRefresh from '../components/LibraryRefresh.svelte'
  import LibraryHealth from '../components/LibraryHealth.svelte'
  import LanguageBatchExport from '../components/LanguageBatchExport.svelte'
  import DetailPanel from '../components/DetailPanel.svelte'
  import CardMenu from '../components/CardMenu.svelte'
  import ArtworkEditor from '../components/ArtworkEditor.svelte'
  import Count from '../components/Count.svelte'
  import Empty from '../components/Empty.svelte'
  import Icon from '../components/Icon.svelte'
  import PendingListView from '../components/PendingList.svelte'
  import PosterWall from '../components/PosterWall.svelte'
  import { api } from '../lib/api'
  import { listen } from '../lib/events'
  import { pendingRead } from '../lib/pending'
  import { rise } from '../lib/motion'
  import { app } from '../lib/state.svelte'
  import type { Facets, MovieCard, PendingList, PendingSummary, Query } from '../lib/types'

  let utilitiesOpen = $state(false)
  let shownRoot = app.settings?.library_root ?? ''
  let browser = $state(readBrowser(shownRoot))
  let debounced = $state(untrack(() => browser.query.text ?? ''))
  let randomBusy = $state(false)
  let exportTask = $state.raw<CancellableJob<number | null> | null>(null)
  let exportProgress = $state({ done: 0, total: 0 })
  let position = $state<number | null>(null)
  let openedVersion = $state(0)
  let navigationBusy = $state(false)
  let sequence = 0
  let storageWarned = false
  function persistBrowser() {
    if (!writeBrowser(shownRoot, $state.snapshot(browser)) && !storageWarned) {
      storageWarned = true; app.toast('info', '浏览偏好暂时无法保存，本次会话仍可继续使用')
    }
  }
  $effect(() => {
    const root = app.settings?.library_root ?? ''
    untrack(() => {
      if (root !== shownRoot) {
        persistBrowser(); shownRoot = root; browser = readBrowser(root); debounced = browser.query.text ?? ''
        open = null; menu = null; cropping = null; position = null
        sequence++; randomBusy = false; navigationBusy = false
        void exportTask?.cancel()
      }
    })
  })
  $effect(() => {
    JSON.stringify(browser)
    const timer = setTimeout(persistBrowser, 250)
    return () => clearTimeout(timer)
  })
  onDestroy(() => { persistBrowser(); sequence++; void exportTask?.cancel() })
  let total = $state<number | null>(null)
  let listLoading = $state(true)
  let facets = $state<Facets | null>(null)
  let open = $state<string | null>(null)
  /** 海报右键菜单 */
  let menu = $state<{ card: MovieCard; x: number; y: number } | null>(null)
  /** 从右键菜单打开的海报裁剪 */
  let cropping = $state<string | null>(null)
  /** 已整理 / 待整理 两个标签;库是空的而有待整理的视频时,默认先看待整理 */
  let tab = $state<'done' | 'pending'>('done')
  let pendingCount = $state<number | null>(null)
  let countLoading = $state(false)
  let pendingComplete = $state(false)
  let pendingCached = $state(false)
  let pendingError = $state('')
  /** 第一处读取失败的待整理文件夹;是已保存的来源时可直接移除 */
  let pendingIssue = $state<{ path: string; message: string; more: number } | null>(null)
  const issueIsSource = $derived(!!pendingIssue && !!app.settings?.source_dirs.includes(pendingIssue.path))
  function readIssues(p: Pick<PendingSummary, 'cancelled' | 'errors' | 'errorsTotal'>) {
    pendingError = p.cancelled ? '待整理检查已停止。' : ''
    const first = p.errors[0]
    pendingIssue = !p.cancelled && first ? { path: first.path, message: first.message, more: p.errorsTotal - 1 } : null
  }
  async function removeSource(path: string) {
    try {
      app.settings = await api.setSourceDirs((app.settings?.source_dirs ?? []).filter((d) => d !== path))
      app.toast('ok', '已从待整理中移除该文件夹')
    } catch (e) {
      app.toast('error', String(e), 5000)
    }
  }
  let countRequest: ReturnType<typeof pendingRead<PendingSummary>> | null = null
  let autoPicked = false

  // Preserve identity while raw typing leaves the debounced query unchanged.
  const queryKey = $derived(JSON.stringify({ ...browser.query, text: debounced || undefined }))
  const query = $derived<Query>(JSON.parse(queryKey))
  const filtered = $derived(!!(debounced || query.contentType || query.year || query.minScore !== undefined || query.missing || query.filters?.length))
  $effect(() => {
    const text = browser.query.text ?? ''
    const timer = setTimeout(() => debounced = text.trim(), 200)
    return () => clearTimeout(timer)
  })
  $effect(() => { JSON.stringify(query); untrack(() => { sequence++; open = null; position = null; randomBusy = false; navigationBusy = false }) })
  let returnFocus: HTMLElement | null = null
  function closeDetail() { sequence++; open = null; position = null; navigationBusy = false; randomBusy = false; void tick().then(() => { if (returnFocus?.isConnected) returnFocus.focus() }) }
  function openCard(code: string, index?: number) { if (!open) returnFocus = document.activeElement as HTMLElement; open = code; position = index ?? null; openedVersion = app.libraryVersion }
  async function openAt(index: number, random = false) {
    const current = ++sequence
    const root = shownRoot
    const version = app.libraryVersion
    if (random) randomBusy = true; else navigationBusy = true
    try {
      let page = await api.libraryList({ ...$state.snapshot(query), offset: Math.max(0, index), limit: 1 })
      if (current !== sequence || root !== shownRoot) return
      // A concurrent library edit may shrink the last page.
      if (!page.items.length && page.total > 0) {
        index = page.total - 1
        page = await api.libraryList({ ...$state.snapshot(query), offset: index, limit: 1 })
      }
      if (current !== sequence || root !== shownRoot) return
      if (version !== app.libraryVersion) throw '媒体库列表已更新，请重新选择影片'
      total = page.total
      if (page.items[0]) openCard(page.items[0].code, index)
      else app.toast('info', '当前筛选没有可浏览的影片')
    } catch (e) { if (current === sequence) app.toast('error', String(e), 5000) }
    finally { if (current === sequence) { randomBusy = false; navigationBusy = false } }
  }
  async function randomPick() {
    if (randomBusy) return
    // Get a fresh count; do not rely on the loaded viewport or an old count.
    const current = ++sequence, version = app.libraryVersion; randomBusy = true
    try {
      const page = await api.libraryList({ ...$state.snapshot(query), offset: 0, limit: 1 })
      if (current !== sequence) return
      if (version !== app.libraryVersion) throw '媒体库列表已更新，请重新挑选'
      if (!page.total) { app.toast('info', '当前筛选没有可挑选的影片'); return }
      await openAt(Math.floor(Math.random() * page.total), true)
    } catch (e) { if (current === sequence) app.toast('error', String(e), 5000) }
    finally { if (current === sequence) randomBusy = false }
  }
  async function exportCsv() {
    if (exportTask) return
    const root = shownRoot
    const captured = $state.snapshot(query)
    exportProgress = { done: 0, total: 0 }
    let off: (() => void) | undefined
    const task = cancellableJob(async id => {
      off = await listen('library:export', event => { if (event.jobId === task.jobId && event.root === root) exportProgress = event })
      if (task.cancelled) throw '导出已停止'
      return api.libraryExportCsv(captured, id)
    }, e => app.toast('error', `停止导出失败：${e}`, 0, { label: '重试停止', run: () => void task.cancel() }))
    exportTask = task
    try {
      const count = await task.promise
      if (root === shownRoot && count !== null) app.toast('ok', `已导出 ${count} 部影片的 CSV 清单`)
    } catch (e) { app.toast(task.cancelled ? 'info' : 'error', String(e), 5000) }
    finally { off?.(); if (exportTask === task) exportTask = null }
  }
  $effect(() => {
    app.libraryVersion
    app.settings?.library_root
    let alive = true
    api.libraryFacets().then((f) => { if (alive) facets = f }).catch(() => {})
    return () => { alive = false }
  })

  function onPending(p: PendingList) {
    pendingCount = p.total
    pendingComplete = p.complete
    pendingCached = p.cached
    readIssues(p)
  }

  // 库总数和待整理数都到齐后决定一次默认标签:库是空的而有待整理的视频,就先看待整理
  $effect(() => {
    if (autoPicked || total === null || pendingCount === null || (pendingCount === 0 && !pendingComplete)) return
    autoPicked = true
    if (total === 0 && pendingCount > 0) tab = 'pending'
  })

  // A bounded count read has no number parsing or full media rows.
  $effect(() => {
    app.libraryVersion
    JSON.stringify([app.settings?.source_dirs, app.settings?.library_root, app.settings?.scan])
    let alive = true
    countLoading = true
    pendingCount = null
    pendingComplete = false
    pendingCached = false
    pendingError = ''
    pendingIssue = null
    countRequest = pendingRead(id => api.pendingSummary(id))
    countRequest.promise.then(p => {
      if (!alive) return
      pendingCount = p.total
      pendingComplete = p.complete
      pendingCached = p.cached
      readIssues(p)
    }).catch(error => { if (alive) pendingError = String(error) })
      .finally(() => { if (alive) countLoading = false })
    return () => { alive = false; countRequest?.cancel() }
  })

  function clearFilters() { browser.query = cleanQuery({}); browser.scroll = 0; debounced = '' }
</script>

{#snippet libraryTabs()}
<div class="tabs" role="tablist" aria-label="媒体库内容" style="--i:{tab === 'done' ? 0 : 1}">
      <span class="thumb" aria-hidden="true"></span>
      <button role="tab" aria-selected={tab === 'done'} class:on={tab === 'done'} onclick={() => (tab = 'done')}>已整理{#if total !== null}<span class="n"><Count value={total} /></span>{/if}</button>
      <button role="tab" title={pendingCached ? '上次检查的数量，可在待整理页刷新' : undefined} aria-selected={tab === 'pending'} class:on={tab === 'pending'} onclick={() => (tab = 'pending')}>待整理{#if pendingCount !== null}<span class="n" class:hot={pendingCount > 0}><Count value={pendingCount} />{!pendingComplete ? "+" : ""}</span>{/if}</button>
    </div>
{/snippet}

<div class="lib" in:rise>
  <header>
    <div class="title">
      <h1>媒体库</h1>
      <span class="count" role="status" aria-live="polite">{#if total !== null && !listLoading && (browser.query.text ?? '').trim() === debounced}<Count value={total} />{filtered ? ' 个结果' : ' 部'}{/if}</span>
      {#if app.rebuild.running}<span class="count"><Icon name="loader" size={12} /> 重建索引 {app.rebuild.done}/{app.rebuild.total}</span>{/if}
    </div>

    <div class="header-actions">
      <button class="btn icon ghost commands-mobile" aria-label="快捷操作" onclick={() => app.commandsOpen = true}><Icon name="search" size={14}/></button>
      <LibraryRefresh />
      <button class="btn" aria-expanded={utilitiesOpen} aria-controls="library-utilities" onclick={()=>utilitiesOpen=!utilitiesOpen} hidden={tab!=='done'}><Icon name="settings" size={14}/>媒体库工具</button>
      {#if total!==0}<button class="btn primary" onclick={()=>app.navigate('import')}><Icon name="download" size={14}/>导入影片</button>{/if}
    </div>
    {#if tab === 'pending'}{@render libraryTabs()}{/if}

    {#if countLoading}<span class="count">检查待整理数量… <button class="btn sm ghost" onclick={() => countRequest?.cancel()}>停止数量检查</button></span>{/if}
    {#if pendingError}<p role="alert" class="count">{pendingError}</p>{/if}
    {#if pendingIssue}
      <p role="alert" class="count issue">
        <Icon name="alert" size={14} />
        <span>待整理文件夹不可用：<span class="mono">{pendingIssue.path}</span>（{pendingIssue.message}）{#if pendingIssue.more > 0}另有 {pendingIssue.more} 处读取失败。{/if}</span>
        {#if issueIsSource}<button class="btn sm ghost" onclick={() => removeSource(pendingIssue!.path)}>从待整理中移除</button>{/if}
      </p>
    {/if}
    {#if tab === 'done'}
      <div class="controls"><LibraryBrowserControls tabs={libraryTabs} bind:value={browser} {facets} onrandom={randomPick} {randomBusy} randomDisabled={listLoading || total === 0 || (browser.query.text ?? '').trim() !== debounced} onexport={exportCsv} exporting={!!exportTask} {exportProgress} onstop={() => void exportTask?.cancel()}/></div>
    {/if}
  </header>

  <div id="library-utilities" class="utility-drawer" role="region" aria-label="媒体库工具" hidden={!utilitiesOpen||tab!=='done'}>
    <div class="utility-head"><div><h2>媒体库工具</h2><p>批量更新影片信息，或检查文件是否完整。</p></div><button class="btn sm ghost" onclick={()=>utilitiesOpen=false}>收起工具</button></div>
    {#if utilitiesOpen && tab === 'done'}<LibraryOverview {query} /><BatchEditor {query} />{/if}
    <div class="utility-grid"><div class="utility-block"><h3>语言导出</h3><LanguageBatchExport {query}/></div><div class="utility-block"><h3>文件健康</h3><LibraryHealth onopen={(code)=>open=code} /></div></div>
  </div>
  {#key tab}
  <div class="pane" in:rise={{ duration: 280, y: 6 }}>
  {#if tab === 'pending'}
    <PendingListView onloaded={onPending} />
  {:else}
  {#key app.settings?.library_root}
  <PosterWall {query} layout={browser.layout} density={browser.density} initialScroll={browser.scroll} onscrollposition={value => browser.scroll = value} version={app.libraryVersion} onopen={openCard} onmenu={(card, x, y) => (menu = { card, x, y })} ontotal={(n) => (total = n)} onloading={(busy) => (listLoading = busy)}>
    {#snippet empty()}
      {#if filtered}
        <Empty icon="search" title="没有匹配的影片" description="换个关键词,或清除筛选条件试试。">
          <button class="btn" onclick={clearFilters}>清除筛选</button>
        </Empty>
      {:else}
        <Empty icon="film" title="媒体库还是空的" description="导入待整理的文件夹,或者从磁盘上已有的 NFO 重建索引。">
          <button class="btn primary" onclick={() => app.navigate('import')}><Icon name="download" size={15} />导入影片</button>
          <button class="btn" onclick={() => app.rebuildIndex()} disabled={app.rebuild.running}><Icon name="refresh" size={15} />从磁盘重建索引</button>
        </Empty>
      {/if}
    {/snippet}
  </PosterWall>
  {/key}
  {/if}
  </div>
  {/key}
</div>

{#if menu}
  <CardMenu {...menu} onopen={(c) => (open = c)} oncrop={(c) => (cropping = c)} ontrashed={(c) => { if (open === c) open = null }} onclose={() => (menu = null)} />
{/if}

{#if cropping}
  <ArtworkEditor code={cropping} onsaved={() => {}} onclose={() => (cropping = null)} />
{/if}

{#if open}
  <DetailPanel code={open} onclose={closeDetail} navigation={position === null ? undefined : { position, total: total ?? 0, busy: navigationBusy, stale: openedVersion !== app.libraryVersion, move: step => void openAt(position! + step) }} />
{/if}

<style>
  .lib { height: 100%; display: flex; flex-direction: column; }
  .commands-mobile { display:none; }
  header { display:grid; grid-template-columns:minmax(0,1fr) auto; gap:8px 24px; padding:16px 24px 12px; }
  .title { display:flex; align-items:baseline; flex-wrap:wrap; gap:12px; }
  h1 { font-size:26px; font-weight:700; letter-spacing:-.025em; }
  .count { display:inline-flex; align-items:center; gap:5px; color:var(--text-2); font-size:var(--fs-sm); }
  .issue { color:var(--warn); flex-wrap:wrap; overflow-wrap:anywhere; }
  .issue > span { color:var(--text-2); }
  .header-actions { display:flex; gap:8px; align-items:center; justify-self:end; }
  .header-actions [hidden] { display:none; }
  .tabs { align-self:center; justify-self:start; }
  .pane { flex:1; min-height:0; display:flex; flex-direction:column; }
  .utility-drawer { flex-shrink:0; margin:0 24px 16px; border:1px solid var(--line); border-radius:var(--r-md); background:var(--surface-1); max-height:48vh; overflow:auto; }
  .utility-drawer[hidden] { display:none; }
  .utility-head { display:flex; align-items:center; justify-content:space-between; gap:16px; padding:16px 20px; border-bottom:1px solid var(--line); }
  .utility-head h2 { font-size:var(--fs-lg); }
  .utility-head p { margin-top:4px; color:var(--text-2); font-size:var(--fs-sm); }
  .utility-grid { display:grid; grid-template-columns:1fr 1fr; }
  .utility-block { padding:16px 20px; min-width:0; }
  .utility-block+ .utility-block { border-left:1px solid var(--line); }
  .utility-block h3 { margin-bottom:12px; color:var(--text-2); font-size:var(--fs-sm); font-weight:600; }
  /* 分段切换:滑块在两个标签之间弹性滑动 */
  .tabs { position: relative; display: grid; grid-template-columns: 1fr 1fr; padding: 3px; border-radius: var(--r-md); background: var(--surface-2); }
  .tabs .thumb {
    position: absolute; top: 3px; bottom: 3px; left: 3px; width: calc(50% - 3px); border-radius: 9px;
    background: var(--surface-hover); box-shadow: var(--shadow-1);
    transform: translateX(calc(var(--i) * 100%)); transition: transform var(--d-slow) var(--ease-back);
  }
  .tabs button {
    position: relative; z-index: 1; display: inline-flex; align-items: center; justify-content: center; gap: 6px; height: 28px; padding: 0 var(--s4);
    border: 0; border-radius: 9px; background: transparent; color: var(--text-2); font-weight: 500;
    transition: color var(--d-base) var(--ease-out), transform var(--d-base) var(--ease-back);
  }
  .tabs button:hover { color: var(--text-1); }
  .tabs button:active { transform: scale(0.95); }
  .tabs button.on { color: var(--text-1); }
  .n { font-size: var(--fs-xs); color: var(--text-3); }
  .n.hot { color: var(--accent); font-weight: 600; }
  .controls { grid-column:1/-1; min-width:0; }
  header { flex-shrink:0; max-height:55%; overflow:auto; scrollbar-gutter:stable; }
  @media (max-width: 720px) {
    .commands-mobile { display:inline-flex; }
    header { padding:16px 16px 12px; gap:12px; grid-template-columns:1fr; }
    .header-actions { grid-row:2; justify-self:start; flex-wrap:wrap; }
    .utility-drawer { margin-inline:16px; }
    .utility-grid { grid-template-columns:1fr; }
    .utility-block+ .utility-block { border-left:0; border-top:1px solid var(--line); }
  }
</style>
