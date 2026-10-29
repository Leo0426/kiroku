<script lang="ts">
  import { flip } from 'svelte/animate'
  import { scale } from 'svelte/transition'
  import { api } from '../lib/api'
  import { pendingRead } from '../lib/pending'
  import { dur } from '../lib/motion'
  import { baseName, fmtBytes, shortPath } from '../lib/format'
  import { flow } from '../lib/importflow.svelte'
  import { app } from '../lib/state.svelte'
  import { CONTENT_TYPE_LABEL, type PendingList } from '../lib/types'
  import Empty from './Empty.svelte'
  import Icon from './Icon.svelte'
  import StatusPill from './StatusPill.svelte'
  import LoadingRows from './LoadingRows.svelte'
  import LoadingStatus from './LoadingStatus.svelte'

  let { onloaded }: { onloaded?: (p: PendingList) => void } = $props()

  let data = $state<PendingList | null>(null)
  let loading = $state(true)
  let loadError = $state('')
  let generation = 0
  let lastConfig = ''
  let text = $state('')
  let recognition = $state('all')
  let source = $state('')
  let sort = $state('name')
  let selected = $state<string[]>([])
  let shown = $state(300)
  const importLocked = $derived(loading || flow.busy || app.rebuild.running || (flow.step !== 'source' && flow.step !== 'done'))
  const MAX_ROWS = 300
  let request: ReturnType<typeof pendingRead<PendingList>> | null = null

  async function load(force = false) {
    request?.cancel()
    const current = ++generation
    loading = true
    loadError = ''
    try {
      request = pendingRead(id => api.pendingList(id, force))
      const result = await request.promise
      if (current !== generation) return
      data = result
      selected = selected.filter(path => result.items.some(item => item.path === path))
      onloaded?.(data)
    } catch (e) {
      if (current === generation) loadError = String(e)
    } finally {
      if (current === generation) loading = false
    }
  }

  // 库有变化(整理/撤销)后重新列出
  $effect(() => {
    app.libraryVersion
    const config = JSON.stringify([app.settings?.source_dirs, app.settings?.library_root, app.settings?.scan])
    if (config !== lastConfig) { data = null; selected = []; source = ''; lastConfig = config }
    void load()
    return () => { generation++; request?.cancel() }
  })

  async function addDir() {
    const d = await api.pickDirectory('选择待整理的文件夹', app.media?.downloads)
    if (!d) return
    try {
      app.settings = await api.setSourceDirs([...new Set([...(data?.dirs ?? []), d])])
      await load()
    } catch (e) {
      app.toast('error', String(e), 5000)
    }
  }

  async function removeDir(d: string) {
    try { app.settings = await api.setSourceDirs((data?.dirs ?? []).filter((x) => x !== d)); await load() }
    catch (e) { app.toast('error', String(e), 5000) }
  }

  async function start(onlySelected = false) {
    if (importLocked) return
    const paths = [...selected]
    app.navigate('import')
    if (app.view !== 'import') return
    if (onlySelected) await flow.startFromFiles(paths)
    else await flow.startFromSources()
  }
  function toggle(path: string) { selected = selected.includes(path) ? selected.filter(p => p !== path) : [...selected, path] }
  const within = (path: string, dir: string) => path.replaceAll('\\', '/').startsWith(dir.replaceAll('\\', '/').replace(/\/$/, '') + '/')
  const matches = $derived.by(() => {
    const needle = text.trim().toLocaleLowerCase()
    return (data?.items ?? []).filter(it => (!needle || `${it.path} ${it.code?.id ?? ''}`.toLocaleLowerCase().includes(needle)) && (!source || within(it.path, source)) && (recognition === 'all' || (recognition === 'recognized' ? !!it.code : !it.code)))
      .sort((a,b) => sort === 'size' ? b.size - a.size || a.path.localeCompare(b.path) : sort === 'code' ? (a.code?.id ?? '\uffff').localeCompare(b.code?.id ?? '\uffff') || a.path.localeCompare(b.path) : baseName(a.path).localeCompare(baseName(b.path)) || a.path.localeCompare(b.path))
  })
  $effect(() => { text; recognition; source; sort; shown = MAX_ROWS })

  const parentOf = (p: string) => p.slice(0, Math.max(0, p.length - baseName(p).length)).replace(/[\\/]$/, '')
  const rows = $derived(matches.slice(0, shown))
  /** 读取失败的已保存来源(常见于文件夹被移走或外接盘未连接) */
  const broken = $derived(new Map((data?.errors ?? []).map((issue) => [issue.path, issue.message])))
</script>

<div class="pending" aria-busy={loading}>
  <div class="bar">
    <div class="dirs">
      {#each data?.dirs ?? [] as d (d)}
        <span class="dir" class:broken={broken.has(d)} title={broken.has(d) ? `${d}\n${broken.get(d)}` : d} animate:flip={{ duration: dur(240) }} in:scale={{ start: 0.7, duration: dur(220) }} out:scale={{ start: 0.7, duration: dur(160) }}>
          <Icon name={broken.has(d) ? 'alert' : 'folder'} size={13} /><span class="mono">{shortPath(d, 44)}</span>
          <button aria-label="移除 {d}" onclick={() => removeDir(d)}><Icon name="x" size={12} /></button>
        </span>
      {/each}
      <button class="btn sm" onclick={addDir}><Icon name="plus" size={14} />添加待整理文件夹</button>
    </div>
    <div class="go"><button class="btn sm" onclick={() => load(true)} disabled={loading}>刷新待整理</button>{#if loading}<button class="btn sm" onclick={() => request?.cancel()}>停止检查</button>{/if}</div>
    {#if data && data.items.length}
      <div class="go">
        <span class="faint tnum">{data.items.length}{!data.complete ? '+' : ''} 个文件 · {data.codes} 个番号{#if data.unrecognized} · <span class="warn">{data.unrecognized} 未识别</span>{/if}</span>
        <button class="btn primary" onclick={() => start()} disabled={importLocked}>开始整理<Icon name="arrow-right" size={15} /></button>
      </div>
    {/if}
  </div>

  {#if data?.items.length}
    <div class="filters" aria-label="待整理预览筛选">
      <input class="input" aria-label="搜索待整理预览" type="search" placeholder="搜索文件名、路径或编号" bind:value={text} />
      <select class="input" aria-label="识别状态" bind:value={recognition}><option value="all">全部识别状态</option><option value="recognized">已识别</option><option value="unknown">未识别</option></select>
      <select class="input" aria-label="待整理来源筛选" bind:value={source}><option value="">全部来源</option>{#each data.dirs as dir}<option value={dir}>{dir}</option>{/each}</select>
      <select class="input" aria-label="待整理排序" bind:value={sort}><option value="name">文件名</option><option value="code">编号</option><option value="size">体积从大到小</option></select>
    </div>
    {#if text || recognition !== 'all' || source}<button class="btn sm ghost" onclick={() => { text='';recognition='all';source='' }}>清除预览筛选</button>{/if}
    <p class="faint">仅筛选本次预览中的 {data.items.length} 个文件，匹配 {matches.length} 个；“开始整理”仍扫描全部来源。</p>
    <div class="selection" aria-label="待整理文件选择">
      <button class="btn sm" disabled={importLocked || !matches.length} onclick={() => selected = [...new Set([...selected, ...matches.map(it => it.path)])]}>选择匹配项</button>
      <button class="btn sm ghost" disabled={!selected.length || importLocked} onclick={() => selected = []}>清空选择</button>
      <span role="status">已选 {selected.length} 个{selected.length > matches.filter(it => selected.includes(it.path)).length ? '（含筛选外文件）' : ''}</span>
      <button class="btn primary" disabled={!selected.length || importLocked} onclick={() => start(true)}>仅整理所选文件</button>
    </div>
    {#if flow.step !== 'source' && flow.step !== 'done'}<p class="muted">请先完成或重置当前导入，再开始新的整理。 <button class="btn sm" onclick={() => app.navigate('import')}>返回当前导入</button></p>{/if}
  {/if}
  {#if data?.cached}<p class="faint">显示上次检查结果；新增文件后可刷新。</p>{/if}
  {#if data && !data.complete}
    <div class="load-error" role="alert"><div><p>{data.cancelled ? '检查已停止，结果不完整。' : data.errorsTotal ? '部分文件夹未能读取，结果不完整。' : '仅检查了部分文件；完整整理会重新扫描全部来源。'}</p>
      {#each data.errors as issue}<p class="issue-row"><span>{issue.path}：{issue.message}</span>{#if data.dirs.includes(issue.path)}<button class="btn sm ghost" onclick={() => removeDir(issue.path)}>从待整理中移除</button>{/if}</p>{/each}
      {#if data.errorsTotal > data.errors.length}<p>另有 {data.errorsTotal - data.errors.length} 处读取错误。</p>{/if}
    </div></div>
  {/if}
  {#if loadError}
    <div class="load-error" role="alert"><p>无法读取待整理文件。{loadError}</p><button class="btn" onclick={() => load(true)} disabled={loading}>重新读取</button></div>
  {/if}
  {#if loading && data}<LoadingStatus label="正在更新待整理文件…" />{/if}
  {#if loading && !data}
    <div class="pad"><LoadingRows label="正在查找待整理的视频…" /></div>
  {:else if !data && loadError}
    <p class="muted pad">请确认待整理文件夹可访问，再重新读取。</p>
  {:else if !data || data.dirs.length === 0}
    <Empty icon="folder" title="还没有待整理的文件夹" description="添加一个存放视频的文件夹,这里会列出里面还没整理的视频。">
      <button class="btn primary" onclick={addDir}><Icon name="plus" size={15} />添加文件夹</button>
    </Empty>
  {:else if data.items.length === 0 && !data.complete}
    <p class="muted pad">尚未完成检查，无法确认是否有待整理视频。请刷新或开始完整扫描。</p>
    <button class="btn" onclick={() => start()} disabled={importLocked}>完整扫描</button>
  {:else if data.items.length === 0}
    <Empty icon="film" title="没有待整理的视频" description="这些文件夹里没有找到视频,或者都已经整理到媒体库里了。" />
  {:else}
    <section class="card table" aria-label="待整理的视频">
      {#each rows as it (it.id)}
        <div class="trow">
          <input class="checkbox" type="checkbox" aria-label="选择 {baseName(it.path)}" checked={selected.includes(it.path)} disabled={importLocked} onchange={() => toggle(it.path)} />
          <div class="file">
            <span class="name" title={it.path}>{baseName(it.path)}</span>
            <span class="faint sub">{shortPath(parentOf(it.path), 60)}</span>
          </div>
          <div class="cell">
            {#if it.code}<span class="mono code">{it.code.id}</span>{:else}<StatusPill tone="warn">未识别番号</StatusPill>{/if}
          </div>
          <div class="cell tags">
            {#if it.code && it.code.content_type !== 'censored'}<StatusPill tone="info">{CONTENT_TYPE_LABEL[it.code.content_type] ?? it.code.content_type}</StatusPill>{/if}
            {#if it.info.cd}<StatusPill>CD{it.info.cd}</StatusPill>{/if}
            {#if it.info.has_subtitle}<StatusPill tone="accent">中字</StatusPill>{/if}
            {#if it.info.definition}<StatusPill>{it.info.definition}</StatusPill>{/if}
          </div>
          <div class="size tnum faint">{fmtBytes(it.size)}</div>
        </div>
      {/each}
      {#if !matches.length}<p class="more muted">当前预览中没有匹配文件。</p>{/if}
      {#if matches.length > shown}<button class="btn more" onclick={() => shown += MAX_ROWS}>再显示 {Math.min(MAX_ROWS, matches.length - shown)} 个文件</button>{/if}
    </section>
  {/if}
</div>

<style>
  .filters, .selection { display:flex; gap:var(--s2); flex-wrap:wrap; align-items:center; }
  .filters input { flex:1; min-width:180px; }
  .filters select { max-width:100%; min-width:130px; }
  .selection { position:sticky; top:0; background:var(--bg); padding-block:var(--s2); z-index:2; }
  .go { flex-wrap:wrap; }
  @media (max-width:480px) { .trow { flex-wrap:wrap; } .file { min-width:150px; } }

  .load-error { display:flex; align-items:center; gap:var(--s3); padding:var(--s4); background:var(--danger-soft); border-radius:var(--r-sm); }
  .load-error p { flex:1; overflow-wrap:anywhere; }
  .pending { display: flex; flex-direction: column; gap: var(--s4); padding: var(--s2) var(--s5) var(--s7); overflow-y: auto; min-height: 0; flex: 1; }
  .bar { display: flex; align-items: flex-start; justify-content: space-between; gap: var(--s4); flex-wrap: wrap; }
  .dirs { display: flex; flex-wrap: wrap; gap: var(--s2); align-items: center; min-width: 0; }
  .dir { display: inline-flex; align-items: center; gap: 6px; height: 28px; padding: 0 4px 0 10px; border-radius: var(--r-pill); background: var(--surface-2); font-size: var(--fs-sm); max-width: 100%; transition: background var(--d-fast), transform var(--d-base) var(--ease-back); }
  .dir:hover { background: var(--surface-3); transform: translateY(-1px); }
  .dir.broken { background: var(--warn-soft); color: var(--warn); }
  .load-error .issue-row { display: flex; align-items: center; gap: var(--s2); flex-wrap: wrap; }
  .issue-row > span { flex: 1; min-width: 0; }
  .dir button { display: grid; place-items: center; width: 22px; height: 22px; border: 0; border-radius: 50%; background: transparent; color: var(--text-3); }
  .dir button { transition: background var(--d-fast), color var(--d-fast), transform var(--d-base) var(--ease-back); }
  .dir button:hover { background: var(--surface-hover); color: var(--danger); transform: rotate(90deg); }
  .go { display: flex; align-items: center; gap: var(--s4); font-size: var(--fs-sm); }
  .warn { color: var(--warn); }
  .pad { padding: var(--s5); }
  .table { padding: var(--s1); }
  .trow { display: flex; align-items: center; gap: var(--s3); padding: var(--s2) var(--s3); border-radius: var(--r-sm); }
  .trow:hover { background: var(--surface-2); }
  .file { flex: 1; min-width: 0; display: flex; flex-direction: column; }
  .name, .sub { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .name { font-size: var(--fs-sm); }
  .sub { font-size: var(--fs-xs); }
  .cell { width: 150px; flex-shrink: 0; min-width: 0; }
  .code { color: var(--accent); font-weight: 600; }
  .tags { display: flex; gap: var(--s1); flex-wrap: wrap; width: 170px; }
  .size { width: 72px; text-align: right; flex-shrink: 0; font-size: var(--fs-sm); }
  .more { padding: var(--s3); text-align: center; font-size: var(--fs-sm); }
  @media (max-width: 720px) { .pending { padding-inline: var(--s4); } .tags { display: none; } .cell { width: auto; } }
</style>
