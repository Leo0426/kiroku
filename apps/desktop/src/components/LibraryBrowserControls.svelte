<script lang="ts">
  import { onMount, tick } from 'svelte'
  import { app } from '../lib/state.svelte'
  import { cleanQuery, filterCount, rememberSearch, type BrowserState } from '../lib/library-browser'
  import { CONTENT_TYPE_LABEL, FACET_LABEL, badgeLabel, type Facets, type FacetKind, type Query } from '../lib/types'
  import Icon from './Icon.svelte'

  let { value = $bindable(), facets, searchEl = $bindable(), onclear, onclearsearch, onrandom, randomBusy, randomDisabled, onexport, exporting, exportProgress, onstop }: {
    value: BrowserState; facets: Facets | null; searchEl?: HTMLInputElement; onclear: () => void; onclearsearch: () => void; onrandom: () => void; randomBusy: boolean; randomDisabled: boolean
    onexport: () => void; exporting: boolean; exportProgress: { done: number; total: number }; onstop: () => void
  } = $props()
  let layoutOpen = $state(false)
  let expanded = $state(false)
  let savedOpen = $state(false)
  let recentOpen = $state(false)
  let name = $state('')
  let savedId = $state('')
  let filterButton: HTMLButtonElement
  const groups = [{ kind: 'actor', key: 'actors' }, { kind: 'tag', key: 'tags' }, { kind: 'studio', key: 'studios' }, { kind: 'series', key: 'series' }, { kind: 'badge', key: 'badges' }] as const
  const filters = $derived(value.query.filters ?? [])
  const conditionCount = $derived(filterCount(value.query))
  const direction = $derived(value.query.sort === 'title' ? (value.query.reverse ? '标题降序' : '标题升序') : value.query.sort === 'year' ? (value.query.reverse ? '年份从旧到新' : '年份从新到旧') : value.query.sort === 'score' ? (value.query.reverse ? '评分从低到高' : '评分从高到低') : value.query.reverse ? '最早入库优先' : '最近入库优先')
  function patch(q: Partial<Query>) { value.query = { ...value.query, ...q }; value.scroll = 0 }
  function add(kind: FacetKind, entry: string) {
    if (entry && !filters.some(f => f.kind === kind && f.value === entry)) patch({ filters: [...filters, { kind, value: entry }] })
  }
  function removeCondition(event: MouseEvent, change: Partial<Query>) {
    const chip = event.currentTarget as HTMLButtonElement
    const next = chip.nextElementSibling ?? chip.previousElementSibling
    patch(change)
    void tick().then(() => {
      if (next instanceof HTMLButtonElement && next.isConnected) next.focus()
      else filterButton.focus()
    })
  }
  function save() {
    const label = name.trim()
    if (!label) return
    if (value.saved.some(s => s.name === label)) { app.toast('info', '已有同名视图，请换一个名称或更新该视图'); return }
    if (value.saved.length >= 20) { app.toast('info', '最多保存 20 个视图，请先删除不用的视图'); return }
    savedId = crypto.randomUUID()
    value.saved = [...value.saved, { id: savedId, name: label, query: cleanQuery(value.query) }]
    name = ''; app.toast('ok', '筛选视图已保存')
  }
  function selectSaved(id: string) {
    savedId = id
    const item = value.saved.find(s => s.id === id)
    if (item) { value.query = cleanQuery(item.query); value.scroll = 0 }
  }
  async function removeSaved() {
    const current = value
    const item = value.saved.find(s => s.id === savedId)
    if (!item) return
    if (await app.confirm({ title: '删除筛选视图？', body: `删除「${item.name}」只会移除保存的筛选条件。`, confirmLabel: '删除视图' })) {
      if (value !== current) return
      value.saved = value.saved.filter(s => s.id !== item.id); savedId = ''
    }
  }
  onMount(() => {
    const onkey = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement | null
      if (!e.isComposing && e.key === '/' && !e.metaKey && !e.ctrlKey && !e.altKey && !target?.closest('input, textarea, select, [contenteditable="true"], [role="dialog"], [role="alertdialog"], [aria-label="影片详情"]')) {
        e.preventDefault(); searchEl?.focus()
      }
    }
    window.addEventListener('keydown', onkey)
    return () => window.removeEventListener('keydown', onkey)
  })
</script>

{#snippet condition(label: string, accessible: string, change: () => Partial<Query>)}
  <button class="chip" aria-label={`移除筛选 ${accessible}`} title={`移除${label}`} onclick={e => removeCondition(e, change())}><span>{label}</span><Icon name="x" size={12}/></button>
{/snippet}

<div class="browser-controls">
  <div class="search-row">
    <div class="search">
      <Icon name="search" size={16}/>
      <input class="input" type="search" aria-label="搜索" aria-keyshortcuts="/" placeholder="搜索编号、标题、演员…" maxlength="300" bind:this={searchEl}
        value={value.query.text ?? ''} oninput={e => patch({ text: e.currentTarget.value })}
        onkeydown={e => {
          if (e.isComposing) return
          if (e.key === 'Enter') { rememberSearch(value, value.query.text ?? ''); recentOpen = false }
          if (e.key === 'Escape' && value.query.text) { e.preventDefault(); e.stopPropagation(); onclearsearch() }
        }}
        onblur={() => rememberSearch(value, value.query.text ?? '')}/>
      {#if value.query.text}
        <button class="clear-search" aria-label="清空搜索框" title="清空搜索框 (Esc)" onclick={onclearsearch}><Icon name="x" size={14}/></button>
      {:else}<kbd>/</kbd>{/if}
    </div>
    <button class="btn recent-toggle" aria-expanded={recentOpen} onclick={() => recentOpen = !recentOpen}><Icon name="history" size={14}/>最近搜索</button>
    <select class="input" aria-label="排序" value={value.query.sort ?? 'added'} onchange={e => patch({ sort: e.currentTarget.value as Query['sort'] })}>
      <option value="added">入库时间</option><option value="title">标题</option><option value="year">年份</option><option value="score">评分</option>
    </select>
    <button class="btn direction" aria-label="切换排序方向" aria-pressed={value.query.reverse ?? false} title={direction} onclick={() => patch({ reverse: !value.query.reverse })}>{value.query.reverse ? '↑' : '↓'}<span>{direction}</span></button>
    <button class="btn filter-toggle" bind:this={filterButton} aria-label="筛选" title={conditionCount ? `已选择 ${conditionCount} 项筛选` : '按影片资料筛选'} aria-expanded={expanded} aria-controls="browser-filters" onclick={() => expanded = !expanded}><Icon name="sliders" size={14}/>筛选{#if conditionCount}<span class="filter-count">{conditionCount}</span>{/if}</button>
  </div>
  {#if recentOpen}
    <div class="tray" aria-label="最近搜索">
      <span class="muted">最近搜索</span>
      {#each value.recent as word}<button class="btn sm" title={word} onclick={() => { patch({ text: word }); rememberSearch(value, word); recentOpen = false }}>{word}</button>{/each}
      {#if !value.recent.length}<span class="muted">搜索后按 Enter 或离开搜索框时记录。</span>{:else}<button class="btn sm ghost" onclick={() => value.recent = []}>清除搜索记录</button>{/if}
    </div>
  {/if}
  {#if expanded}
    <div class="tray filters" id="browser-filters" role="group" aria-label="维度筛选">
      <label class="filter-field"><span>内容类型</span><select class="input" aria-label="内容类型" value={value.query.contentType ?? ''} onchange={e => patch({ contentType: e.currentTarget.value })}><option value="">全部类型</option>{#each facets?.contentTypes ?? [] as [k, n]}<option value={k}>{CONTENT_TYPE_LABEL[k] ?? k} · {n}</option>{/each}</select></label>
      <label class="filter-field"><span>年份</span><select class="input" aria-label="年份" value={value.query.year ?? ''} onchange={e => patch({ year: e.currentTarget.value ? Number(e.currentTarget.value) : undefined })}><option value="">全部年份</option>{#each facets?.years ?? [] as [y, n]}<option value={y}>{y} · {n}</option>{/each}</select></label>
      <label class="filter-field"><span>最低评分</span><select class="input" aria-label="最低评分" value={value.query.minScore ?? ''} onchange={e => patch({ minScore: e.currentTarget.value ? Number(e.currentTarget.value) : undefined })}><option value="">全部评分</option>{#each [50, 60, 70, 80, 90] as n}<option value={n}>{n / 10} 分及以上</option>{/each}</select></label>
      <label class="filter-field"><span>资料完整性</span><select class="input" aria-label="资料完整性" title="按媒体库索引筛选；实时磁盘检查请使用文件健康工具" value={value.query.missing ?? ''} onchange={e => patch({ missing: (e.currentTarget.value || undefined) as Query['missing'] })}><option value="">全部资料</option><option value="poster">缺少海报</option><option value="score">缺少评分</option><option value="year">缺少年份</option></select></label>

      {#each groups as g}{#if facets?.[g.key].length}<label class="filter-field"><span>{FACET_LABEL[g.kind]}</span><select class="input" aria-label="按{FACET_LABEL[g.kind]}筛选" value="" onchange={e => { add(g.kind, e.currentTarget.value); e.currentTarget.value = '' }}><option value="">选择{FACET_LABEL[g.kind]}…</option>{#each facets?.[g.key] ?? [] as [v, n]}<option value={v} disabled={filters.some(f => f.kind === g.kind && f.value === v)}>{g.kind === 'badge' ? badgeLabel(v) : v} · {n}</option>{/each}</select></label>{/if}{/each}
      <label class="switch inline"><input class="checkbox" type="checkbox" checked={filters.some(f => f.kind === 'favorite')} onchange={e => e.currentTarget.checked ? add('favorite', 'yes') : patch({ filters: filters.filter(f => f.kind !== 'favorite') })}/><span>只看收藏 · {facets?.favorites ?? 0}</span></label>
    </div>
  {/if}
  {#if conditionCount}
    <div class="chips" role="group" aria-label="当前筛选">
      {#if value.query.contentType}
        {@const text = CONTENT_TYPE_LABEL[value.query.contentType] ?? value.query.contentType}
        {@render condition(`类型：${text}`, `类型 ${text}`, () => ({ contentType: '' }))}
      {/if}
      {#if value.query.year}{@render condition(`${value.query.year} 年`, `${value.query.year} 年`, () => ({ year: undefined }))}{/if}
      {#if value.query.minScore !== undefined}{@render condition(`${value.query.minScore / 10} 分及以上`, `${value.query.minScore / 10} 分及以上`, () => ({ minScore: undefined }))}{/if}
      {#if value.query.missing}
        {@const text = `缺少${{ poster: '海报', score: '评分', year: '年份' }[value.query.missing]}`}
        {@render condition(text, text, () => ({ missing: undefined }))}
      {/if}
      {#each filters as f}
        {@const text = f.kind === 'badge' ? badgeLabel(f.value) : f.kind === 'favorite' ? '是' : f.value}
        {@render condition(f.kind === 'favorite' ? '只看收藏' : `${FACET_LABEL[f.kind]}：${text}`, `${FACET_LABEL[f.kind]} ${text}`, () => ({ filters: filters.filter(x => x !== f) }))}
      {/each}
      <button class="btn sm ghost" onclick={onclear}>清空筛选</button>
    </div>
  {/if}
  <div class="browse-row">
    <button class="btn sm ghost" aria-expanded={savedOpen} onclick={() => savedOpen = !savedOpen}><Icon name="star" size={13}/>筛选视图{#if value.saved.length} · {value.saved.length}{/if}</button>
    <div class="display" role="group" aria-label="显示方式"><button aria-label="海报视图" aria-pressed={value.layout === 'posters'} onclick={() => value.layout = 'posters'}><Icon name="grid" size={14}/></button><button aria-label="列表视图" aria-pressed={value.layout === 'list'} onclick={() => value.layout = 'list'}>☰</button></div>
    {#if value.layout === 'posters'}
      <label class="zoom">缩小 <input type="range" min="60" max="180" step="5" aria-label="海报缩放" bind:value={value.posterScale} oninput={() => value.gridMode=false}/> 放大 <output>{value.posterScale}%</output></label>
      <button class="btn sm ghost" aria-expanded={layoutOpen} onclick={() => layoutOpen = !layoutOpen}><Icon name="sliders" size={14}/>海报布局</button>
    {/if}
    <div class="spacer"></div>
    <button class="btn sm ghost" onclick={onrandom} disabled={randomBusy || randomDisabled} title={randomDisabled ? '等待查询完成且有匹配影片后可挑选' : '从当前筛选的全部影片中随机打开一部'}>{randomBusy ? '正在挑选…' : '随机挑选'}</button>
    {#if exporting}<span class="muted" role="status">导出 {exportProgress.done} / {exportProgress.total || '…'}</span>{/if}
    <button class="btn sm ghost" onclick={exporting ? onstop : onexport}>{exporting ? '停止导出' : '导出 CSV'}</button>
  </div>
  {#if value.layout === 'posters' && layoutOpen}
    <div class="tray poster-layout" role="group" aria-label="海报布局设置">
      <label>排列方式 <select class="input" aria-label="海报排列方式" value={value.gridMode ? 'grid' : 'scale'} onchange={e => { value.gridMode = e.currentTarget.value === 'grid' }}><option value="scale">按大小缩放</option><option value="grid">按行列排列</option></select></label>
      {#if value.gridMode}
        <label>列数 <select class="input" aria-label="海报列数" bind:value={value.columns}>{#each Array.from({length:12},(_,i)=>i+1) as n}<option value={n}>{n} 列</option>{/each}</select></label>
        <label>每屏行数 <select class="input" aria-label="海报每屏行数" bind:value={value.visibleRows}>{#each [1,2,3,4,5,6] as n}<option value={n}>{n} 行</option>{/each}</select></label>
        <span class="muted">保持海报比例；窗口过小时自动减少行列，更多影片向下滚动。</span>

      {/if}
      <button class="btn sm ghost" onclick={() => { value.posterScale=100; value.gridMode=false; value.columns=4; value.visibleRows=2 }}>重置布局</button>
    </div>
  {/if}
  {#if savedOpen}
    <div class="tray saved" aria-label="筛选视图管理">
      <select class="input" aria-label="已保存视图" value={savedId} onchange={e => selectSaved(e.currentTarget.value)}><option value="">选择已保存视图…</option>{#each value.saved as s}<option value={s.id}>{s.name}</option>{/each}</select>
      {#if value.saved.some(s => s.id === savedId)}<button class="btn sm" onclick={() => { value.saved = value.saved.map(s => s.id === savedId ? { ...s, query: cleanQuery(value.query) } : s); app.toast('ok', '视图条件已更新') }}>更新为当前条件</button><button class="btn sm ghost" onclick={removeSaved}>删除视图</button>{/if}
      <form onsubmit={e => { e.preventDefault(); save() }}><input class="input" aria-label="视图名称" placeholder="保存当前筛选，如「高评分收藏」" maxlength="40" bind:value={name}/><button class="btn sm" disabled={!name.trim()}>保存视图</button></form>
    </div>
  {/if}
</div>

<style>
  .browser-controls { display:flex; flex-direction:column; gap:10px; min-width:0; }
  .search-row,.browse-row,.tray,.chips { display:flex; align-items:center; flex-wrap:wrap; gap:8px; min-width:0; }
  .search { position:relative; flex:1; min-width:180px; }
  .search .input { width:100%; padding-left:34px; padding-right:30px; }
  .search > :global(svg) { position:absolute; left:11px; top:10px; color:var(--text-2); }
  kbd { position:absolute; right:10px; top:8px; color:var(--text-3); pointer-events:none; }
  .direction { min-width:40px; }.direction span { font-size:var(--fs-xs); }
  .tray { padding:12px; border:1px solid var(--line); border-radius:var(--r-md); background:var(--surface-1); }
  .tray .input { min-width:0; max-width:220px; }.tray .btn { max-width:100%; overflow:hidden; text-overflow:ellipsis; }
  .search input::-webkit-search-cancel-button { display:none; }
  .clear-search { position:absolute; inset-block:2px; right:3px; width:30px; display:grid; place-items:center; border:0; border-radius:var(--r-sm); background:transparent; color:var(--text-2); }
  .clear-search:hover { color:var(--text-1); background:var(--surface-hover); }
  .chip { min-width:0; max-width:100%; min-height:28px; display:inline-flex; align-items:center; gap:7px; padding:3px 9px; border:1px solid transparent; border-radius:var(--r-pill); font-size:var(--fs-sm); background:var(--accent-soft); color:var(--text-1); }
  .chip span { overflow:hidden; white-space:nowrap; text-overflow:ellipsis; }
  .chip :global(svg) { color:var(--text-2); }
  .chip:hover { border-color:var(--accent); }
  .filter-count { min-width:1.45em; border-radius:var(--r-pill); background:var(--accent-soft); color:var(--text-1); text-align:center; font-size:var(--fs-xs); }
  .filter-toggle[aria-expanded="true"] { box-shadow:inset 0 0 0 1px var(--line-strong); }
  .browse-row { border-top:1px solid var(--line); padding-top:8px; }
  .tray.filters { display:grid; grid-template-columns:repeat(auto-fit,minmax(min(135px,100%),1fr)); }
  .tray.filters .input { width:100%; min-width:0; max-width:none; }
  .filter-field { min-width:0; display:grid; gap:4px; }
  .filter-field > span { font-size:var(--fs-xs); color:var(--text-2); }
  .filters .switch { align-self:end; min-height:34px; }
  .display { display:flex; padding:2px; background:var(--surface-2); border-radius:8px; }.display button { display:grid; place-items:center; min-width:32px; height:28px; border:0; border-radius:6px; background:none; color:var(--text-2); }.display button[aria-pressed="true"] { color:var(--accent); background:var(--accent-soft); }
  .spacer { flex:1; }
  .poster-layout label, .zoom { display:flex; flex-wrap:wrap; align-items:center; gap:8px; font-size:var(--fs-sm); }
  .zoom { flex:0 1 200px; min-width:170px; }
  .zoom input { flex:1; min-width:80px; max-width:240px; accent-color:var(--accent); }
  .zoom output { min-width:4ch; }
  .poster-layout .muted { flex:1 1 240px; font-size:var(--fs-xs); }
  form { display:flex; flex-wrap:wrap; gap:8px; flex:1; }form .input { flex:1; min-width:180px !important; max-width:none !important; }
  @media(max-width:1000px) { .direction span { display:none; } }
  @media(max-width:560px) { .search { flex-basis:100%; }.search-row>select { flex:1 1 110px; min-width:110px; }.tray .input { flex:1 1 120px; min-width:120px; max-width:100%; }.browse-row { gap:5px; }.spacer { display:none; }.recent-toggle { font-size:var(--fs-sm); } }
</style>
