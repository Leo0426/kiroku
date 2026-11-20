<script lang="ts">
  import { onDestroy, untrack, tick } from 'svelte'
  import BatchEditor from '../components/BatchEditor.svelte'
  import LibraryOverview from '../components/LibraryOverview.svelte'
  import LibraryBrowserControls from '../components/LibraryBrowserControls.svelte'
  import { readBrowser, writeBrowser, cleanQuery, filterCount } from '../lib/library-browser'
  import { cancellableJob, type CancellableJob } from '../lib/cancellable-job'
  import LibraryRefresh from '../components/LibraryRefresh.svelte'
  import LibraryHealth from '../components/LibraryHealth.svelte'
  import PackageExport from '../components/PackageExport.svelte'
  import PackageRestore from '../components/PackageRestore.svelte'
  import CsvExport from '../components/CsvExport.svelte'
  import type { CsvOptions } from '../lib/types'
  import LanguageBatchExport from '../components/LanguageBatchExport.svelte'
  import DetailPanel from '../components/DetailPanel.svelte'
  import CardMenu from '../components/CardMenu.svelte'
  import ArtworkEditor from '../components/ArtworkEditor.svelte'
  import Count from '../components/Count.svelte'
  import Empty from '../components/Empty.svelte'
  import Icon from '../components/Icon.svelte'
  import LoadingStatus from '../components/LoadingStatus.svelte'
  import PopupNotice from '../components/PopupNotice.svelte'
  import PendingListView from '../components/PendingList.svelte'
  import PosterWall from '../components/PosterWall.svelte'
  import type { PosterMenu } from '../components/PosterCard.svelte'
  import { api } from '../lib/api'
  import { listen } from '../lib/events'
  import { pendingRead } from '../lib/pending'
  import { rise } from '../lib/motion'
  import { app } from '../lib/state.svelte'
  import type { Facets, PendingList, PendingSummary, Query } from '../lib/types'

  let libraryRefresh: ReturnType<typeof LibraryRefresh>
  let posterRetryEpoch = $state(0)
  let posterNotice: number | null = null

  function dismissPosterNotice() {
    if (posterNotice !== null) app.dismiss(posterNotice)
    posterNotice = null
  }
  function posterUnavailable(code: string) {
    dismissPosterNotice()
    posterNotice = app.toast('info',
      '请确认媒体盘已连接，海报文件可以打开。如果影片或海报换过位置，可以刷新媒体库重新查找。', 0,
      { label: '刷新媒体库', run: () => { dismissPosterNotice(); void libraryRefresh.refresh() } },
      `仍无法读取 ${code} 的海报`)
  }

  let utilitiesOpen = $state(false)
  let shownRoot = app.settings?.library_root ?? ''
  let browser = $state(readBrowser(shownRoot))
  let searchEl: HTMLInputElement | undefined = $state()
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
        dismissPosterNotice()
        persistBrowser(); shownRoot = root; browser = readBrowser(root); debounced = browser.query.text ?? ''
        autoPicked = false; tab = 'done'; total = null; facets = null
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
  onDestroy(() => { dismissPosterNotice(); persistBrowser(); sequence++; void exportTask?.cancel() })
  let total = $state<number | null>(null)
  let listLoading = $state(true)
  let facets = $state<Facets | null>(null)
  let open = $state<string | null>(null)
  /** 海报右键菜单 */
  let menu = $state<(PosterMenu & { version: number }) | null>(null)
  /** 从右键菜单打开的海报裁剪 */
  let cropping = $state<string | null>(null)
  /** 已整理 / 待整理 两个标签;库是空的而有待整理的视频时,默认先看待整理 */
  let tab = $state<'done' | 'pending'>('done')
  let pendingCount = $state<number | null>(null)
  let countLoading = $state(false)
  let pendingComplete = $state(false)
  let pendingCached = $state(false)
  let pendingError = $state('')
  let pendingIssue = $state<{ path: string; message: string; errorsTotal: number } | null>(null)
  const issueIsSource = $derived(!!pendingIssue && !!app.settings?.source_dirs.includes(pendingIssue.path))
  function readIssues(p: Pick<PendingSummary, 'cancelled' | 'errors' | 'errorsTotal'>) {
    pendingError = p.cancelled ? '待整理检查已停止。' : ''
    const first = p.errors[0]
    pendingIssue = !p.cancelled && first ? { ...first, errorsTotal: p.errorsTotal } : null
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
  const conditions = $derived(filterCount(query))
  const filtered = $derived(!!debounced || conditions > 0)
  $effect(() => {
    const text = browser.query.text ?? ''
    const timer = setTimeout(() => debounced = text.trim(), 200)
    return () => clearTimeout(timer)
  })
  $effect(() => { JSON.stringify(query); untrack(() => { sequence++; open = null; position = null; randomBusy = false; navigationBusy = false }) })
  // A menu action belongs to the result position it was opened from.
  $effect(() => { queryKey; untrack(closeMenu) })
  let returnFocus: HTMLElement | null = null
  function closeDetail() {
    sequence++
    open = null
    position = null
    navigationBusy = false
    randomBusy = false
    const trigger = returnFocus
    returnFocus = null
    void tick().then(() => (trigger?.isConnected ? trigger : searchEl)?.focus())
  }
  function openCard(code: string, index?: number, trigger = document.activeElement as HTMLElement | null, version = app.libraryVersion) {
    if (!open) returnFocus = trigger
    open = code
    position = index ?? null
    openedVersion = version
  }
  function openMenuDetail() {
    if (!menu) return
    openCard(menu.card.code, menu.index, menu.trigger, menu.version)
    menu = null
  }
  function closeMenu() {
    if (menu?.trigger.isConnected) menu.trigger.focus({ preventScroll: true })
    menu = null
  }
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
  async function exportCsv(options?: CsvOptions, selectedQuery?: Query) {
    if (exportTask) return
    const root = shownRoot
    const captured = selectedQuery ?? $state.snapshot(query)
    exportProgress = { done: 0, total: 0 }
    let off: (() => void) | undefined
    const task = cancellableJob(async id => {
      off = await listen('library:export', event => { if (event.jobId === task.jobId && event.root === root) exportProgress = event })
      if (task.cancelled) throw '导出已停止'
      return api.libraryExportCsv(captured, id, options)
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
    if (app.rebuild.running || app.rebuildMessage || listLoading || autoPicked || total === null || pendingCount === null || (pendingCount === 0 && !pendingComplete)) return
    autoPicked = true
    if (total === 0 && pendingCount > 0 && !filtered && !browser.query.text?.trim()) tab = 'pending'
  })

  // A bounded count read has no number parsing or full media rows.
  $effect(() => {
    app.libraryVersion
    JSON.stringify([app.settings?.source_dirs, app.settings?.library_root, app.settings?.scan])
    let alive = true
    pendingCount = null
    pendingComplete = false
    pendingCached = false
    pendingError = ''
    pendingIssue = null
    countLoading = !app.rebuild.running
    if (app.rebuild.running) return
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

  function clearFilters(text = browser.query.text ?? '') {
    browser.query = cleanQuery({ text, sort: browser.query.sort, reverse: browser.query.reverse })
    browser.scroll = 0
    debounced = text.trim()
    searchEl?.focus()
  }
  function clearSearch() {
    browser.query.text = ''
    debounced = ''
    browser.scroll = 0
    searchEl?.focus()
  }
  function selectTab(next: 'done' | 'pending') {
    autoPicked = true
    tab = next
  }
  function switchTab(event: KeyboardEvent) {
    if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return
    event.preventDefault()
    selectTab(event.key === 'Home' ? 'done' : event.key === 'End' ? 'pending' : tab === 'done' ? 'pending' : 'done')
    const list = event.currentTarget as HTMLElement
    list.querySelector<HTMLButtonElement>(`[data-tab="${tab}"]`)?.focus()
  }
</script>

<svelte:window onpagehide={persistBrowser} />

<div class="lib" in:rise>
  <header>
    <div class="title">
      <h1>媒体库</h1>
      {#if tab === 'done' && filtered}<span class="count" role="status" aria-live="polite">{#if total !== null && !listLoading && (browser.query.text ?? '').trim() === debounced}<Count value={total} /> 个结果{/if}</span>{/if}
      {#if app.rebuild.running}<LoadingStatus label={app.rebuild.total ? `重建索引 ${app.rebuild.done}/${app.rebuild.total}` : '正在准备重建索引…'} />{/if}
    </div>

    <div class="tabs" role="tablist" tabindex="-1" aria-label="媒体库内容" onkeydown={switchTab} style="--i:{tab === 'done' ? 0 : 1}">
      <span class="thumb" aria-hidden="true"></span>
      <button role="tab" data-tab="done" aria-selected={tab === 'done'} tabindex={tab === 'done' ? 0 : -1} class:on={tab === 'done'} onclick={() => selectTab('done')}>已整理{#if total !== null}<span class="n"><Count value={total} /></span>{/if}</button>
      <button role="tab" data-tab="pending" title={pendingCached ? '上次检查的数量，可在待整理页刷新' : undefined} aria-selected={tab === 'pending'} tabindex={tab === 'pending' ? 0 : -1} class:on={tab === 'pending'} onclick={() => selectTab('pending')}>待整理{#if pendingCount !== null}<span class="n" class:hot={pendingCount > 0}><Count value={pendingCount} />{!pendingComplete ? "+" : ""}</span>{/if}</button>
    </div>

    <div class="header-actions">
      <LibraryRefresh bind:this={libraryRefresh} onrefreshed={() => { dismissPosterNotice(); posterRetryEpoch++ }} />
      <button class="btn" aria-expanded={utilitiesOpen} aria-controls="library-utilities" onclick={()=>utilitiesOpen=!utilitiesOpen} hidden={tab!=='done'}><Icon name="settings" size={14}/>媒体库工具</button>
      {#if total !== 0 || filtered || tab === 'pending'}<button class="btn primary" onclick={()=>app.navigate('import')}><Icon name="download" size={14}/>导入影片</button>{/if}
    </div>
    {#if countLoading}<div class="count count-loading"><LoadingStatus label="检查待整理数量…" /><button class="btn sm ghost" onclick={() => countRequest?.cancel()}>停止数量检查</button></div>{/if}
    {#if pendingError}<PopupNotice text={pendingError} />{/if}
    {#if pendingIssue}
      {@const {path, message, errorsTotal} = pendingIssue}
      {#if issueIsSource}
        <PopupNotice title="待整理文件夹不可用" text={`${path}（${message}）${errorsTotal > 1 ? `\n另有 ${errorsTotal - 1} 处读取失败。` : ''}`}
          action={{ label: '从待整理中移除', run: () => void removeSource(path) }} />
      {:else if tab === 'done'}
        <PopupNotice title="待整理扫描结果不完整" text={`有 ${errorsTotal} 处路径未能读取。\n${path}（${message}）`}
          action={{ label: '查看待整理', run: () => selectTab('pending') }} />
      {/if}
    {/if}
    {#if tab === 'done'}
      <div class="controls"><LibraryBrowserControls bind:value={browser} bind:searchEl {facets} onclear={() => clearFilters()} onclearsearch={clearSearch} onrandom={randomPick} {randomBusy} randomDisabled={listLoading || total === 0 || (browser.query.text ?? '').trim() !== debounced} onexport={() => void exportCsv()} exporting={!!exportTask} {exportProgress} onstop={() => void exportTask?.cancel()}/></div>
    {/if}
  </header>

  <div id="library-utilities" class="utility-drawer" role="region" aria-label="媒体库工具" hidden={!utilitiesOpen||tab!=='done'}>
    <div class="utility-head"><div><h2>媒体库工具</h2><p>将影片资料复制到其他目录、导回已有影片，或批量更新媒体库。</p></div><button class="btn sm ghost" onclick={()=>utilitiesOpen=false}>收起工具</button></div>
    {#if utilitiesOpen && tab === 'done'}<LibraryOverview {query} /><BatchEditor {query} />{/if}
    {#key app.settings?.library_root}
      <div class="utility-grid"><div class="utility-block"><h3>导出到其他目录</h3><PackageExport {query}/><CsvExport {query} busy={!!exportTask} onexport={exportCsv} onstop={() => void exportTask?.cancel()}/></div>
        <div class="utility-block"><h3>导回当前媒体库</h3><PackageRestore/></div></div>
    {/key}
    <div class="utility-grid"><div class="utility-block"><h3>写回影片目录</h3><LanguageBatchExport {query}/></div><div class="utility-block"><h3>文件健康</h3><LibraryHealth onopen={openCard} /></div></div>
  </div>
  {#key tab}
  <div class="pane" in:rise={{ duration: 280, y: 6 }}>
  {#if tab === 'pending'}
    <PendingListView onloaded={onPending} />
  {:else}
  {#key app.settings?.library_root}
  <PosterWall {query} retryEpoch={posterRetryEpoch} onretryfailed={posterUnavailable} layout={browser.layout} posterScale={browser.posterScale} gridMode={browser.gridMode} columns={browser.columns} visibleRows={browser.visibleRows} initialScroll={browser.scroll} onscrollposition={value => browser.scroll = value} version={app.libraryVersion} onopen={openCard} onmenu={entry => menu = { ...entry, version: app.libraryVersion }} ontotal={(n) => (total = n)} onloading={(busy) => (listLoading = busy)}>
    {#snippet empty()}
      {#if app.rebuild.running}
        <Empty icon="folder" title="正在从已有影片建立索引" description="完成后，影片会自动出现在这里。" />
      {:else if listLoading}
        <!-- The poster wall announces its pending read above the empty slot. -->
      {:else if !filtered && app.rebuildMessage}
        <Empty icon="folder" title="媒体库索引尚未更新" description={app.rebuildMessage}>
          <button class="btn primary" onclick={() => app.rebuildIndex()}><Icon name="refresh" size={15} />重试重建索引</button>
        </Empty>
      {:else if filtered}
        <Empty icon="search" title="没有匹配的影片" description={debounced ? '可以换个关键词，或重新浏览当前范围内的影片。' : '可以移除上方的单个条件，逐步扩大范围。'}>
          {#if debounced}<button class="btn primary" onclick={clearSearch}>清除关键词</button>{/if}
          {#if conditions}<button class="btn" class:primary={!debounced} onclick={() => clearFilters('')}>{debounced ? '查看全部影片' : '清除筛选'}</button>{/if}
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
  <CardMenu {...menu} onopen={openMenuDetail} oncrop={(c) => (cropping = c)} ontrashed={(c) => { if (open === c) open = null }} onclose={closeMenu} />
{/if}

{#if cropping}
  <ArtworkEditor code={cropping} onsaved={() => {}} onclose={() => (cropping = null)} />
{/if}

{#if open}
  <DetailPanel code={open} retryEpoch={posterRetryEpoch} onclose={closeDetail} navigation={position === null ? undefined : { position, total: total ?? 0, busy: navigationBusy, stale: openedVersion !== app.libraryVersion, move: step => void openAt(position! + step) }} />
{/if}

<style>
  .lib { height: 100%; min-height:360px; display: flex; flex-direction: column; }
  header { display:grid; grid-template-columns:auto minmax(0,1fr) auto; gap:10px 18px; padding:16px 24px 12px; }
  .title { display:flex; align-items:center; flex-wrap:wrap; gap:12px; }
  h1 { font-size:26px; font-weight:700; letter-spacing:-.025em; }
  .count { display:inline-flex; align-items:center; gap:5px; color:var(--text-2); font-size:var(--fs-sm); }
  .count-loading { grid-column:1/-1; display: flex; flex-wrap: wrap; align-items: center; gap: var(--s2); }
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
  @media (max-width: 1000px) {
    header { grid-template-columns:auto minmax(0,1fr); }
    .header-actions { grid-column:1/-1; justify-self:start; flex-wrap:wrap; }
  }
  @media (max-width: 720px) {
    header { padding:16px 16px 12px; gap:10px; }
    .tabs { justify-self:end; }
    .tabs button { padding-inline:9px; }
    .utility-drawer { margin-inline:16px; }
    .utility-grid { grid-template-columns:1fr; }
    .utility-block+ .utility-block { border-left:0; border-top:1px solid var(--line); }
  }
</style>
