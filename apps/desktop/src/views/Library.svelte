<script lang="ts">
  import { onMount } from 'svelte'
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
  import { pendingRead } from '../lib/pending'
  import { rise } from '../lib/motion'
  import { app } from '../lib/state.svelte'
  import { CONTENT_TYPE_LABEL, FACET_LABEL, badgeLabel, type FacetFilter, type FacetKind, type Facets, type MovieCard, type PendingList, type PendingSummary, type Query, type Sort } from '../lib/types'

  let utilitiesOpen = $state(false)
  let text = $state('')
  let debounced = $state('')
  let contentType = $state('')
  let year = $state('')
  let sort = $state<Sort>('added')
  /** 维度筛选(全部满足);切换媒体库时清空 */
  let filters = $state<FacetFilter[]>([])
  let facetsOpen = $state(false)
  let shownRoot = app.settings?.library_root ?? null
  $effect(() => {
    const root = app.settings?.library_root ?? null
    if (root !== shownRoot) { shownRoot = root; filters = [] }
  })
  const FACET_GROUPS: { kind: FacetKind; key: 'tags' | 'actors' | 'studios' | 'series' | 'badges' }[] = [
    { kind: 'actor', key: 'actors' }, { kind: 'tag', key: 'tags' }, { kind: 'studio', key: 'studios' }, { kind: 'series', key: 'series' }, { kind: 'badge', key: 'badges' },
  ]
  const valueLabel = (f: FacetFilter) => (f.kind === 'badge' ? badgeLabel(f.value) : f.kind === 'favorite' ? '是' : f.value)
  function addFilter(kind: FacetKind, value: string) {
    if (value && !filters.some((f) => f.kind === kind && f.value === value)) filters = [...filters, { kind, value }]
  }
  const removeFilter = (f: FacetFilter) => (filters = filters.filter((x) => x !== f))
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
  let countRequest: ReturnType<typeof pendingRead<PendingSummary>> | null = null
  let autoPicked = false
  let searchEl: HTMLInputElement

  const query = $derived<Query>({
    text: debounced || undefined,
    contentType: contentType || undefined,
    year: year ? Number(year) : undefined,
    sort,
    filters: filters.length ? filters.map((f) => ({ ...f })) : undefined,
  })
  const filtered = $derived(!!(debounced || contentType || year || filters.length))

  // 输入停顿 200ms 后才真正查询
  $effect(() => {
    const t = text
    const h = setTimeout(() => (debounced = t.trim()), 200)
    return () => clearTimeout(h)
  })

  $effect(() => {
    app.libraryVersion
    let alive = true
    api.libraryFacets().then((f) => { if (alive) facets = f }).catch(() => {})
    return () => { alive = false }
  })

  function onPending(p: PendingList) {
    pendingCount = p.total
    pendingComplete = p.complete
    pendingCached = p.cached
    pendingError = p.cancelled ? '待整理检查已停止。' : p.errorsTotal ? `待整理目录读取失败：${p.errors[0]?.path ?? ''} ${p.errors[0]?.message ?? ''}` : ''
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
    countRequest = pendingRead(id => api.pendingSummary(id))
    countRequest.promise.then(p => {
      if (!alive) return
      pendingCount = p.total
      pendingComplete = p.complete
      pendingCached = p.cached
      pendingError = p.cancelled ? '待整理检查已停止。' : p.errorsTotal ? `待整理目录读取失败：${p.errors[0]?.path ?? ''} ${p.errors[0]?.message ?? ''}` : ''
    }).catch(error => { if (alive) pendingError = String(error) })
      .finally(() => { if (alive) countLoading = false })
    return () => { alive = false; countRequest?.cancel() }
  })

  function clearFilters() {
    text = ''
    debounced = ''
    contentType = ''
    year = ''
    filters = []
  }

  onMount(() => {
    // 按 / 聚焦搜索
    const onkey = (e: KeyboardEvent) => {
      const tag = (e.target as HTMLElement | null)?.tagName
      if (e.key === '/' && tag !== 'INPUT' && tag !== 'SELECT' && tag !== 'TEXTAREA') {
        e.preventDefault()
        searchEl?.focus()
      }
    }
    window.addEventListener('keydown', onkey)
    return () => window.removeEventListener('keydown', onkey)
  })
</script>

<div class="lib" in:rise>
  <header>
    <div class="title">
      <h1>媒体库</h1>
      {#if total !== null && !listLoading && text.trim() === debounced}<span class="count"><Count value={total} />{filtered ? ' 个结果' : ' 部'}</span>{/if}
      {#if app.rebuild.running}<span class="count"><Icon name="loader" size={12} /> 重建索引 {app.rebuild.done}/{app.rebuild.total}</span>{/if}
    </div>

    <div class="header-actions">
      <button class="btn" aria-expanded={utilitiesOpen} aria-controls="library-utilities" onclick={()=>utilitiesOpen=!utilitiesOpen} hidden={tab!=='done'}><Icon name="settings" size={14}/>媒体库工具</button>
      {#if total!==0}<button class="btn primary" onclick={()=>app.navigate('import')}><Icon name="download" size={14}/>导入影片</button>{/if}
    </div>
    <div class="tabs" role="tablist" aria-label="媒体库内容" style="--i:{tab === 'done' ? 0 : 1}">
      <span class="thumb" aria-hidden="true"></span>
      <button role="tab" aria-selected={tab === 'done'} class:on={tab === 'done'} onclick={() => (tab = 'done')}>已整理{#if total !== null}<span class="n"><Count value={total} /></span>{/if}</button>
      <button role="tab" title={pendingCached ? '上次检查的数量，可在待整理页刷新' : undefined} aria-selected={tab === 'pending'} class:on={tab === 'pending'} onclick={() => (tab = 'pending')}>待整理{#if pendingCount !== null}<span class="n" class:hot={pendingCount > 0}><Count value={pendingCount} />{!pendingComplete ? "+" : ""}</span>{/if}</button>
    </div>

    {#if countLoading}<span class="count">检查待整理数量… <button class="btn sm ghost" onclick={() => countRequest?.cancel()}>停止数量检查</button></span>{/if}
    {#if pendingError}<p role="alert" class="count">{pendingError}</p>{/if}
    <div class="tools" hidden={tab !== 'done'}>
      <label class="search">
        <Icon name="search" size={15} />
        <input bind:this={searchEl} bind:value={text} class="input" type="search" placeholder="搜索番号、标题或演员" aria-label="搜索" title="按 / 快速搜索" />
      </label>
      <select class="input" bind:value={contentType} aria-label="内容类型">
        <option value="">全部类型</option>
        {#each facets?.contentTypes ?? [] as [k, n] (k)}<option value={k}>{CONTENT_TYPE_LABEL[k] ?? k} · {n}</option>{/each}
      </select>
      <select class="input" bind:value={year} aria-label="年份">
        <option value="">全部年份</option>
        {#each facets?.years ?? [] as [y, n] (y)}<option value={y}>{y} · {n}</option>{/each}
      </select>
      <select class="input" bind:value={sort} aria-label="排序">
        <option value="added">最近入库</option>
        <option value="title">标题</option>
        <option value="year">年份</option>
        <option value="score">评分</option>
      </select>
      <button class="btn" aria-expanded={facetsOpen} aria-controls="facet-panel" onclick={() => (facetsOpen = !facetsOpen)}><Icon name="sliders" size={14} />筛选</button>
    </div>
    {#if tab === 'done' && (facetsOpen || filters.length)}
      <div id="facet-panel" class="facets" role="group" aria-label="维度筛选">
        {#if facetsOpen}
          <label class="switch inline"><input class="checkbox" type="checkbox" checked={filters.some((f) => f.kind === 'favorite')}
            onchange={(e) => e.currentTarget.checked ? addFilter('favorite', 'yes') : (filters = filters.filter((f) => f.kind !== 'favorite'))} /><span>只看收藏 · {facets?.favorites ?? 0}</span></label>
          {#each FACET_GROUPS as g (g.kind)}
            {@const values = facets?.[g.key] ?? []}
            {#if values.length}
              <select class="input" aria-label="按{FACET_LABEL[g.kind]}筛选" value="" onchange={(e) => { addFilter(g.kind, e.currentTarget.value); e.currentTarget.value = '' }}>
                <option value="">{FACET_LABEL[g.kind]}…</option>
                {#each values as [v, n] (v)}<option value={v} disabled={filters.some((f) => f.kind === g.kind && f.value === v)}>{g.kind === 'badge' ? badgeLabel(v) : v} · {n}</option>{/each}
              </select>
            {/if}
          {/each}
        {/if}
        {#each filters as f (f.kind + '\u0000' + f.value)}
          <span class="chip">{FACET_LABEL[f.kind]}：{valueLabel(f)}<button class="x" aria-label="移除筛选 {FACET_LABEL[f.kind]} {valueLabel(f)}" onclick={() => removeFilter(f)}><Icon name="x" size={12} /></button></span>
        {/each}
        {#if filters.length}<button class="btn sm ghost" onclick={() => (filters = [])}>清空筛选</button>{/if}
      </div>
    {/if}
  </header>

  <div id="library-utilities" class="utility-drawer" role="region" aria-label="媒体库工具" hidden={!utilitiesOpen||tab!=='done'}>
    <div class="utility-head"><div><h2>媒体库工具</h2><p>批量更新影片信息，或检查文件是否完整。</p></div><button class="btn sm ghost" onclick={()=>utilitiesOpen=false}>收起工具</button></div>
    <div class="utility-grid"><div class="utility-block"><h3>语言导出</h3><LanguageBatchExport {query}/></div><div class="utility-block"><h3>文件健康</h3><LibraryHealth onopen={(code)=>open=code} /></div></div>
  </div>
  {#key tab}
  <div class="pane" in:rise={{ duration: 280, y: 6 }}>
  {#if tab === 'pending'}
    <PendingListView onloaded={onPending} />
  {:else}
  <PosterWall {query} version={app.libraryVersion} onopen={(c) => (open = c)} onmenu={(card, x, y) => (menu = { card, x, y })} ontotal={(n) => (total = n)} onloading={(busy) => (listLoading = busy)}>
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
  <DetailPanel code={open} onclose={() => (open = null)} />
{/if}

<style>
  .lib { height: 100%; display: flex; flex-direction: column; }
  header { display:grid; grid-template-columns:minmax(0,1fr) auto; gap:12px 24px; padding:20px 24px 16px; }
  .title { display:flex; align-items:baseline; flex-wrap:wrap; gap:12px; }
  h1 { font-size:26px; font-weight:700; letter-spacing:-.025em; }
  .count { display:inline-flex; align-items:center; gap:5px; color:var(--text-2); font-size:var(--fs-sm); }
  .header-actions { display:flex; gap:8px; align-items:center; justify-self:end; }
  .header-actions [hidden] { display:none; }
  .tabs { align-self:center; justify-self:start; }
  .tools { display:flex; gap:8px; align-items:center; justify-content:flex-end; min-width:0; }
  .tools[hidden] { display:none; }
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
  .search { position: relative; display: block; }
  .search :global(svg) { position: absolute; left: 11px; top: 0; bottom: 0; margin: auto 0; color: var(--text-3); pointer-events: none; transition: color var(--d-base), transform var(--d-base) var(--ease-back); }
  .search:focus-within :global(svg) { color: var(--accent); }
  /* 聚焦时搜索框展开 */
  .search { flex:1; min-width:160px; max-width:360px; }
  .search .input { width:100%; padding-left:34px; }
  .search .input:focus { width:100%; }
  .tools select { transition: transform var(--d-base) var(--ease-back), background var(--d-fast), box-shadow var(--d-base); }
  .tools select:active { transform: scale(0.97); }
  @media (max-width: 1200px) {
    .tools { grid-column:1/-1; justify-content:flex-start; flex-wrap:wrap; }
    .tabs { grid-column:1; grid-row:2; }
    .tools { grid-row:3; }
  }
  @media (max-width: 720px) {
    header { padding:20px 16px 12px; gap:12px; }
    .header-actions { flex-wrap:wrap; }
    .header-actions .btn { padding-inline:10px; }
    .search { max-width:none; flex-basis:100%; }
    .tools select { flex:1; min-width:0; }
    .utility-drawer { margin-inline:16px; }
    .utility-grid { grid-template-columns:1fr; }
    .utility-block+ .utility-block { border-left:0; border-top:1px solid var(--line); }
  }
  .facets { display: flex; flex-wrap: wrap; align-items: center; gap: var(--s2); margin-top: var(--s3); }
  .facets select { max-width: 200px; }
  .chip { display: inline-flex; align-items: center; gap: 4px; padding: 2px 4px 2px 10px; border-radius: 999px; background: var(--accent-soft); color: var(--accent); font-size: var(--fs-sm); max-width: 100%; overflow-wrap: anywhere; }
  .chip .x { display: grid; place-items: center; width: 20px; height: 20px; border: 0; border-radius: 50%; background: transparent; color: inherit; cursor: pointer; }
  .chip .x:hover { background: color-mix(in srgb, var(--accent) 18%, transparent); }
</style>
