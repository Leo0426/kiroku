<script lang="ts">
  import { onMount, type Snippet } from 'svelte'
  import { app } from '../lib/state.svelte'
  import { cleanQuery, rememberSearch, type BrowserState } from '../lib/library-browser'
  import { CONTENT_TYPE_LABEL, FACET_LABEL, badgeLabel, type Facets, type FacetKind, type Query } from '../lib/types'
  import Icon from './Icon.svelte'

  let { value = $bindable(), facets, tabs, onrandom, randomBusy, randomDisabled, onexport, exporting, exportProgress, onstop }: {
    value: BrowserState; facets: Facets | null; tabs: Snippet; onrandom: () => void; randomBusy: boolean; randomDisabled: boolean
    onexport: () => void; exporting: boolean; exportProgress: { done: number; total: number }; onstop: () => void
  } = $props()
  let expanded = $state(false)
  let savedOpen = $state(false)
  let recentOpen = $state(false)
  let name = $state('')
  let savedId = $state('')
  let searchEl: HTMLInputElement
  const groups = [{ kind: 'actor', key: 'actors' }, { kind: 'tag', key: 'tags' }, { kind: 'studio', key: 'studios' }, { kind: 'series', key: 'series' }, { kind: 'badge', key: 'badges' }] as const
  const filters = $derived(value.query.filters ?? [])
  const active = $derived(!!(value.query.text || value.query.contentType || value.query.year || value.query.minScore !== undefined || value.query.missing || filters.length))
  const direction = $derived(value.query.sort === 'title' ? (value.query.reverse ? '标题降序' : '标题升序') : value.query.sort === 'year' ? (value.query.reverse ? '年份从旧到新' : '年份从新到旧') : value.query.sort === 'score' ? (value.query.reverse ? '评分从低到高' : '评分从高到低') : value.query.reverse ? '最早入库优先' : '最近入库优先')
  function patch(q: Partial<Query>) { value.query = { ...value.query, ...q }; value.scroll = 0 }
  function add(kind: FacetKind, entry: string) {
    if (entry && !filters.some(f => f.kind === kind && f.value === entry)) patch({ filters: [...filters, { kind, value: entry }] })
  }
  function clear() { value.query = cleanQuery({ sort: value.query.sort, reverse: value.query.reverse }); value.scroll = 0 }
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
        e.preventDefault(); searchEl.focus()
      }
    }
    window.addEventListener('keydown', onkey)
    return () => window.removeEventListener('keydown', onkey)
  })
</script>

<div class="browser-controls">
  <div class="search-row">
    <label class="search"><Icon name="search" size={16}/><input class="input" type="search" aria-label="搜索" placeholder="搜索编号、标题、演员…" maxlength="300" bind:this={searchEl} value={value.query.text ?? ''} oninput={e => patch({ text: e.currentTarget.value })} onkeydown={e => { if (!e.isComposing && e.key === 'Enter') { rememberSearch(value, value.query.text ?? ''); recentOpen = false } }} onblur={() => rememberSearch(value, value.query.text ?? '')}/><kbd>/</kbd></label>
    <button class="btn recent-toggle" aria-expanded={recentOpen} onclick={() => recentOpen = !recentOpen}>最近搜索</button>
    <select class="input" aria-label="排序" value={value.query.sort ?? 'added'} onchange={e => patch({ sort: e.currentTarget.value as Query['sort'] })}>
      <option value="added">入库时间</option><option value="title">标题</option><option value="year">年份</option><option value="score">评分</option>
    </select>
    <button class="btn direction" aria-label="切换排序方向" aria-pressed={value.query.reverse ?? false} title={direction} onclick={() => patch({ reverse: !value.query.reverse })}>{value.query.reverse ? '↑' : '↓'}<span>{direction}</span></button>
    <button class="btn" aria-expanded={expanded} aria-controls="browser-filters" onclick={() => expanded = !expanded}><Icon name="sliders" size={14}/>筛选{#if active}<span class="active-dot"></span>{/if}</button>
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
      <select class="input" aria-label="内容类型" value={value.query.contentType ?? ''} onchange={e => patch({ contentType: e.currentTarget.value })}><option value="">全部类型</option>{#each facets?.contentTypes ?? [] as [k, n]}<option value={k}>{CONTENT_TYPE_LABEL[k] ?? k} · {n}</option>{/each}</select>
      <select class="input" aria-label="年份" value={value.query.year ?? ''} onchange={e => patch({ year: e.currentTarget.value ? Number(e.currentTarget.value) : undefined })}><option value="">全部年份</option>{#each facets?.years ?? [] as [y, n]}<option value={y}>{y} · {n}</option>{/each}</select>
      <select class="input" aria-label="最低评分" value={value.query.minScore ?? ''} onchange={e => patch({ minScore: e.currentTarget.value ? Number(e.currentTarget.value) : undefined })}><option value="">全部评分</option>{#each [50, 60, 70, 80, 90] as n}<option value={n}>{n / 10} 分及以上</option>{/each}</select>
      <select class="input" aria-label="资料完整性" title="按媒体库索引筛选；实时磁盘检查请使用文件健康工具" value={value.query.missing ?? ''} onchange={e => patch({ missing: (e.currentTarget.value || undefined) as Query['missing'] })}><option value="">全部资料</option><option value="poster">缺少海报</option><option value="score">缺少评分</option><option value="year">缺少年份</option></select>
      <label class="switch inline"><input class="checkbox" type="checkbox" checked={filters.some(f => f.kind === 'favorite')} onchange={e => e.currentTarget.checked ? add('favorite', 'yes') : patch({ filters: filters.filter(f => f.kind !== 'favorite') })}/><span>只看收藏 · {facets?.favorites ?? 0}</span></label>
      {#each groups as g}{#if facets?.[g.key].length}<select class="input" aria-label="按{FACET_LABEL[g.kind]}筛选" value="" onchange={e => { add(g.kind, e.currentTarget.value); e.currentTarget.value = '' }}><option value="">{FACET_LABEL[g.kind]}…</option>{#each facets?.[g.key] ?? [] as [v, n]}<option value={v} disabled={filters.some(f => f.kind === g.kind && f.value === v)}>{g.kind === 'badge' ? badgeLabel(v) : v} · {n}</option>{/each}</select>{/if}{/each}
    </div>
  {/if}
  {#if active}
    <div class="chips" aria-label="当前筛选">
      {#if value.query.text}<span class="chip">搜索：{value.query.text}</span>{/if}
      {#if value.query.contentType}<span class="chip">类型：{CONTENT_TYPE_LABEL[value.query.contentType] ?? value.query.contentType}</span>{/if}
      {#if value.query.year}<span class="chip">{value.query.year} 年</span>{/if}
      {#if value.query.minScore !== undefined}<span class="chip">≥ {value.query.minScore / 10} 分</span>{/if}
      {#if value.query.missing}<span class="chip">缺少{{ poster: '海报', score: '评分', year: '年份' }[value.query.missing]}</span>{/if}
      {#each filters as f}<span class="chip">{FACET_LABEL[f.kind]}：{f.kind === 'badge' ? badgeLabel(f.value) : f.kind === 'favorite' ? '是' : f.value}<button aria-label="移除筛选 {FACET_LABEL[f.kind]} {f.kind === 'badge' ? badgeLabel(f.value) : f.kind === 'favorite' ? '是' : f.value}" onclick={() => patch({ filters: filters.filter(x => x !== f) })}>×</button></span>{/each}
      <button class="btn sm ghost" onclick={clear}>清空筛选</button>
    </div>
  {/if}
  <div class="browse-row">
    {@render tabs()}
    <button class="btn sm ghost" aria-expanded={savedOpen} onclick={() => savedOpen = !savedOpen}><Icon name="star" size={13}/>筛选视图{#if value.saved.length} · {value.saved.length}{/if}</button>
    <div class="display" role="group" aria-label="显示方式"><button aria-label="海报视图" aria-pressed={value.layout === 'posters'} onclick={() => { value.layout = 'posters'; value.scroll = 0 }}><Icon name="grid" size={14}/></button><button aria-label="列表视图" aria-pressed={value.layout === 'list'} onclick={() => { value.layout = 'list'; value.scroll = 0 }}>☰</button></div>
    {#if value.layout === 'posters'}<select class="input density" aria-label="海报大小" bind:value={value.density}><option value="compact">紧凑海报</option><option value="comfortable">标准海报</option><option value="large">大海报</option></select>{/if}
    <div class="spacer"></div>
    <button class="btn sm ghost" onclick={onrandom} disabled={randomBusy || randomDisabled} title={randomDisabled ? '等待查询完成且有匹配影片后可挑选' : '从当前筛选的全部影片中随机打开一部'}>{randomBusy ? '正在挑选…' : '随机挑选'}</button>
    {#if exporting}<span class="muted" role="status">导出 {exportProgress.done} / {exportProgress.total || '…'}</span>{/if}
    <button class="btn sm ghost" onclick={exporting ? onstop : onexport}>{exporting ? '停止导出' : '导出 CSV'}</button>
  </div>
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
  .search :global(svg) { position:absolute; left:11px; top:10px; color:var(--text-2); }
  kbd { position:absolute; right:10px; top:8px; color:var(--text-3); pointer-events:none; }
  .direction { min-width:40px; }.direction span { font-size:var(--fs-xs); }
  .tray { padding:12px; border:1px solid var(--line); border-radius:var(--r-md); background:var(--surface-1); }
  .tray .input { min-width:0; max-width:220px; }.tray .btn { max-width:100%; overflow:hidden; text-overflow:ellipsis; }
  .chip { max-width:100%; overflow-wrap:anywhere; display:inline-flex; align-items:center; gap:6px; padding:3px 10px; border-radius:var(--r-pill); font-size:var(--fs-sm); background:var(--accent-soft); color:var(--accent); }
  .chip button { border:0; background:none; color:inherit; min-width:24px; min-height:24px; }
  .active-dot { width:5px; height:5px; background:var(--accent); border-radius:50%; }
  .display { display:flex; padding:2px; background:var(--surface-2); border-radius:8px; }.display button { display:grid; place-items:center; min-width:32px; height:28px; border:0; border-radius:6px; background:none; color:var(--text-2); }.display button[aria-pressed="true"] { color:var(--accent); background:var(--accent-soft); }
  .density { min-height:30px; height:30px; font-size:var(--fs-xs); }.spacer { flex:1; }
  form { display:flex; flex-wrap:wrap; gap:8px; flex:1; }form .input { flex:1; min-width:180px !important; max-width:none !important; }
  @media(max-width:1000px) { .direction span { display:none; } }
  @media(max-width:560px) { .search { flex-basis:100%; }.search-row>select { flex:1 1 110px; min-width:110px; }.tray .input { flex:1 1 120px; min-width:120px; max-width:100%; }.browse-row { gap:5px; }.spacer { display:none; }.recent-toggle { font-size:var(--fs-sm); } }
</style>
