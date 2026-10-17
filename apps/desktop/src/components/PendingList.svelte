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
    if (config !== lastConfig) { data = null; lastConfig = config }
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
    app.settings = await api.setSourceDirs((data?.dirs ?? []).filter((x) => x !== d))
    await load()
  }

  async function start() {
    app.navigate('import')
    await flow.startFromSources()
  }

  const parentOf = (p: string) => p.slice(0, Math.max(0, p.length - baseName(p).length)).replace(/[\\/]$/, '')
  const rows = $derived((data?.items ?? []).slice(0, MAX_ROWS))
</script>

<div class="pending" aria-busy={loading}>
  <div class="bar">
    <div class="dirs">
      {#each data?.dirs ?? [] as d (d)}
        <span class="dir" title={d} animate:flip={{ duration: dur(240) }} in:scale={{ start: 0.7, duration: dur(220) }} out:scale={{ start: 0.7, duration: dur(160) }}>
          <Icon name="folder" size={13} /><span class="mono">{shortPath(d, 44)}</span>
          <button aria-label="移除 {d}" onclick={() => removeDir(d)}><Icon name="x" size={12} /></button>
        </span>
      {/each}
      <button class="btn sm" onclick={addDir}><Icon name="plus" size={14} />添加待整理文件夹</button>
    </div>
    <div class="go"><button class="btn sm" onclick={() => load(true)} disabled={loading}>刷新待整理</button>{#if loading}<button class="btn sm" onclick={() => request?.cancel()}>停止检查</button>{/if}</div>
    {#if data && data.items.length}
      <div class="go">
        <span class="faint tnum">{data.items.length}{!data.complete ? '+' : ''} 个文件 · {data.codes} 个番号{#if data.unrecognized} · <span class="warn">{data.unrecognized} 未识别</span>{/if}</span>
        <button class="btn primary" onclick={start}>开始整理<Icon name="arrow-right" size={15} /></button>
      </div>
    {/if}
  </div>

  {#if data?.cached}<p class="faint">显示上次检查结果；新增文件后可刷新。</p>{/if}
  {#if data && !data.complete}
    <div class="load-error" role="alert"><div><p>{data.cancelled ? '检查已停止，结果不完整。' : data.errorsTotal ? '部分文件夹未能读取，结果不完整。' : '仅检查了部分文件；完整整理会重新扫描全部来源。'}</p>
      {#each data.errors as issue}<p>{issue.path}：{issue.message}</p>{/each}
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
    <button class="btn" onclick={start}>完整扫描</button>
  {:else if data.items.length === 0}
    <Empty icon="film" title="没有待整理的视频" description="这些文件夹里没有找到视频,或者都已经整理到媒体库里了。" />
  {:else}
    <section class="card table" aria-label="待整理的视频">
      {#each rows as it (it.id)}
        <div class="trow">
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
      {#if data.items.length > MAX_ROWS}<p class="more faint">还有 {data.items.length - MAX_ROWS} 个文件未列出(不影响整理)</p>{/if}
    </section>
  {/if}
</div>

<style>
  .load-error { display:flex; align-items:center; gap:var(--s3); padding:var(--s4); background:var(--danger-soft); border-radius:var(--r-sm); }
  .load-error p { flex:1; overflow-wrap:anywhere; }
  .pending { display: flex; flex-direction: column; gap: var(--s4); padding: var(--s2) var(--s5) var(--s7); overflow-y: auto; min-height: 0; flex: 1; }
  .bar { display: flex; align-items: flex-start; justify-content: space-between; gap: var(--s4); flex-wrap: wrap; }
  .dirs { display: flex; flex-wrap: wrap; gap: var(--s2); align-items: center; min-width: 0; }
  .dir { display: inline-flex; align-items: center; gap: 6px; height: 28px; padding: 0 4px 0 10px; border-radius: var(--r-pill); background: var(--surface-2); font-size: var(--fs-sm); max-width: 100%; transition: background var(--d-fast), transform var(--d-base) var(--ease-back); }
  .dir:hover { background: var(--surface-3); transform: translateY(-1px); }
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
