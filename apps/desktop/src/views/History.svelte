<script lang="ts">
  import { untrack } from 'svelte'
  import { flip } from 'svelte/animate'
  import { fly } from 'svelte/transition'
  import HistoryOperations from '../components/HistoryOperations.svelte'
  import LibraryMaintenance from '../components/LibraryMaintenance.svelte'
  import Empty from '../components/Empty.svelte'
  import LoadingRows from '../components/LoadingRows.svelte'
  import LoadingStatus from '../components/LoadingStatus.svelte'
  import Icon from '../components/Icon.svelte'
  import InfoTip from '../components/InfoTip.svelte'
  import StatusPill from '../components/StatusPill.svelte'
  import { api } from '../lib/api'
  import { fmtDateTime } from '../lib/format'
  import { dur, rise } from '../lib/motion'
  import { app } from '../lib/state.svelte'
  import type { JournalStatus, JournalSummary } from '../lib/types'

  let search = $state('')
  let status = $state<JournalStatus | ''>('')
  let from = $state('')
  let until = $state('')
  let detail = $state<string | null>(null)
  let filter = $state<import('../lib/types').HistoryFilter>({text:'',status:null,from:null,until:null})
  const filtering = $derived(!!(search.trim() || status || from || until))
  const invalidDates = $derived(!!(from && until && from > until))
  let maintenanceOpen = $state(false)
  let list = $state<JournalSummary[] | null>(null)
  let confirming = $state<string | null>(null)
  let busyId = $state<string | null>(null)
  let loadError = $state('')
  let loading = $state(false)
  let loadingMore = $state(false)
  let total = $state(0)
  let next = $state<string | null>(null)
  const PAGE = 50
  let generation = 0
  let requestId:string|null=null
  let shownRoot:string|null=null

  async function load() {
    if (invalidDates) {
      if (requestId) void api.historyCancel(requestId).catch(()=>{})
      requestId=null; generation++; loading=false; loadingMore=false
      return
    }
    if(requestId)void api.historyCancel(requestId).catch(()=>{});
    const id=crypto.randomUUID();requestId=id;
    const current = ++generation
    loading = true
    loadingMore = false
    loadError = ''
    try {
      // 刷新时保留已加载的条数,撤销或删除后不跳回第一页;不让调用它的 $effect 依赖 list
      const result = await api.historyList(id, { filter: $state.snapshot(filter), limit: Math.min(Math.max(PAGE, untrack(() => list?.length ?? 0)), 1000) })
      if (current === generation) ({ items: list, total, next } = result)
    } catch (e) {
      if (current === generation) loadError = String(e)
    } finally {
      if (current === generation) {loading = false;requestId=null;}
    }
  }

  async function loadMore() {
    if (!next || loading || loadingMore || list === null) return
    const id = crypto.randomUUID(); requestId = id
    const current = generation
    loadingMore = true
    try {
      const result = await api.historyList(id, { before: next, limit: PAGE, filter: $state.snapshot(filter) })
      if (current !== generation) return
      const seen = new Set(list.map((j) => j.id))
      list = [...list, ...result.items.filter((j) => !seen.has(j.id))]
      total = result.total
      next = result.next
    } catch (e) {
      if (current === generation) app.toast('error', `无法读取更早的记录：${e}`, 5000)
    } finally {
      if (current === generation) { loadingMore = false; requestId = null }
    }
  }

  function stopRead(){if(requestId)void api.historyCancel(requestId).catch(()=>{});requestId=null;generation++;loading=false;loadingMore=false;loadError='读取已停止，可重新加载。';}
  $effect(() => {
    const root=app.settings?.library_root??null;
    if(root!==shownRoot){shownRoot=root;list=null;next=null;total=0;confirming=null;}
    app.libraryVersion
    filter
    untrack(() => void load())
    return () => { generation++;if(requestId){void api.historyCancel(requestId).catch(()=>{});requestId=null;} }
  })

  $effect(() => {
    const text = search.trim(), state = status, start = from, end = until
    const timer = setTimeout(() => {
      const fromDate = start ? new Date(`${start}T00:00:00`) : null
      const untilDate = end ? new Date(`${end}T00:00:00`) : null
      if (untilDate) untilDate.setDate(untilDate.getDate()+1)
      const nextFilter = {text,status:state || null,from:fromDate?.getTime()??null,until:untilDate?.getTime()??null}
      if (JSON.stringify(nextFilter) === JSON.stringify(untrack(() => filter))) return
      list = null; next = null; detail = null
      filter = nextFilter
    },200)
    return () => clearTimeout(timer)
  })

  const STATUS: Record<JournalStatus, { tone: 'ok' | 'warn' | 'neutral'; label: string }> = {
    applied: { tone: 'ok', label: '已完成' },
    partial: { tone: 'warn', label: '部分完成' },
    rolledBack: { tone: 'neutral', label: '已撤销' },
  }

  async function undo(j: JournalSummary) {
    busyId = j.id
    try {
      const r = await api.rollback(j.id)
      if (r.indexError) app.toast('error', `文件撤销结果已记录，但索引更新失败：${r.indexError}`, 6000)
      app.toast(r.failures.length ? 'error' : 'ok', r.failures.length ? `部分回滚失败:${r.failures[0]}` : '已撤销，文件已按操作日志恢复', 5000)
      await load()
      await app.recovery.refresh()
    } catch (e) {
      app.toast('error', String(e), 5000)
    } finally {
      busyId = null
      confirming = null
    }
  }

  async function revealLog(id: string) {
    try { await api.revealJournal(id) } catch (e) { app.toast('error', String(e), 5000) }
  }

  async function deleteRecord(j: JournalSummary) {
    if (busyId || j.status !== 'rolledBack' || j.error) return
    const current = generation
    const root = app.settings?.library_root
    busyId = j.id
    try {
      const preview = await api.maintenancePreview(true)
      if (current !== generation || root !== app.settings?.library_root) return
      const candidate = preview.candidates.find(c => c.id === `history:${j.id}`)
      if (!candidate) { app.toast('info', '记录仍有依赖或无法确认安全，已保留。'); return }
      const ok = await app.confirm({title:'删除已撤销记录', body:`将 ${fmtDateTime(j.ts)} 的记录及其 ${candidate.paths.length} 个日志、暂存或备份文件移到废纸篓。已整理影片和有效撤销记录继续保留，执行前会重新核对依赖。`, confirmLabel:'移到废纸篓'})
      if (!ok || current !== generation || root !== app.settings?.library_root) return
      const result = await api.maintenanceApply(preview.token, [candidate.id])
      if (root !== app.settings?.library_root) return
      app.toast(result.failed.length || result.skipped.length ? 'error' : 'ok', result.failed[0] ?? result.skipped[0] ?? (result.cancelled ? '删除已停止' : '已移到废纸篓'))
      await load()
      await app.recovery.refresh()
    } catch (e) {
      if (root === app.settings?.library_root) app.toast('error', String(e), 5000)
    } finally { busyId = null }
  }
</script>

<div class="page" in:rise>
  <header>
    <div class="tipped"><h1>历史</h1><InfoTip label="历史说明"><p>查看整理与修订记录，按批次安全撤销。</p><p><b>撤销范围与注意事项</b><br />修订撤销会恢复原 NFO、元数据和图片；发现外部改动时保留内容并报告冲突。原文件夹里的其他文件需要从废纸篓手动找回。</p></InfoTip></div>
    <button class="btn" aria-expanded={maintenanceOpen} aria-controls="history-maintenance" onclick={()=>maintenanceOpen=!maintenanceOpen}><Icon name="sliders" size={14}/>空间维护</button>
  </header>
  <div id="history-maintenance" class="maintenance" hidden={!maintenanceOpen}><LibraryMaintenance /></div>

  <div class="history-filters" aria-label="历史筛选">
    <input class="input" type="search" aria-label="搜索历史编号" maxlength="512" placeholder="搜索全部影片编号或日志编号" bind:value={search} />
    <select class="input" aria-label="历史状态" bind:value={status}><option value="">全部状态</option><option value="applied">已完成</option><option value="partial">部分完成</option><option value="rolledBack">已撤销</option></select>
    <label>开始日期<input class="input" type="date" aria-label="历史开始日期" bind:value={from} /></label>
    <label>结束日期<input class="input" type="date" aria-label="历史结束日期" bind:value={until} /></label>
    {#if filtering}<button class="btn ghost" onclick={() => { search='';status='';from='';until='' }}>清除历史筛选</button>{/if}
  </div>
  {#if invalidDates}<p role="alert">结束日期不能早于开始日期。</p>{/if}
  {#if !loading && list}<p class="muted" role="status">{total} 条{filtering ? '匹配记录' : '记录'}</p>{/if}
  {#if loading}<button class="btn" onclick={stopRead}>停止读取</button>{/if}
  {#if loading && list !== null}<LoadingStatus label="正在更新操作记录…" />{/if}
  {#if loadError}
    <div role="alert"><p class="muted">无法读取整理历史：{loadError}</p><button class="btn" onclick={load} disabled={loading}>重试</button></div>
  {/if}
  {#if invalidDates}
    <p class="muted">调整日期范围后会自动重新查询。</p>
  {:else if list === null && !loadError}
    <LoadingRows label="正在读取整理与修订记录…" />
  {:else if list === null}
    <p class="muted">请确认媒体库可访问，再重试。</p>
  {:else if list.length === 0 && filtering}
    <Empty icon="search" title="没有匹配的历史记录" description="试试其他编号、状态或日期。" />
  {:else if list.length === 0}
    <Empty icon="history" title="还没有操作记录" description="整理影片或保存信息修订后，可以在这里查看记录并撤销。">
      <button class="btn primary" onclick={() => app.navigate('import')}><Icon name="download" size={15} />去导入</button>
    </Empty>
  {:else}
    <ul aria-label="操作记录">
      {#each list as j, k (j.id)}
        <li class="card item" animate:flip={{ duration: dur(260) }} in:fly={{ y: 10, delay: Math.min(k, 8) * 45, duration: dur(320) }}>
          <div class="main">
            <div class="row1">
              <span class="when tnum">{j.error && !j.ts ? '日期未知' : fmtDateTime(j.ts)}</span>
              <StatusPill tone={STATUS[j.status].tone}>{STATUS[j.status].label}</StatusPill>
              <span class="faint tnum">{j.itemCount} 部 · {j.opCount} 项操作{#if j.trashed} · 清理了 {j.trashed} 个原文件夹{/if}</span>
            </div>
            <div class="codes">
              {#each j.codes as c (c)}<span class="mono code">{c}</span>{/each}
              {#if j.itemCount > j.codes.length}<span class="faint">+{j.itemCount - j.codes.length}</span>{/if}
            </div>
          </div>
          <div class="act">
          <button class="btn sm" aria-expanded={detail === j.id} onclick={() => detail = detail === j.id ? null : j.id}>{detail === j.id ? '收起详情' : '操作详情'}</button>
          <button class="btn sm" onclick={() => revealLog(j.id)}>查看日志</button>
          {#if j.status === 'rolledBack' && !j.error}
            <button class="btn danger sm" disabled={busyId !== null} onclick={() => deleteRecord(j)}>{busyId === j.id ? '检查中…' : '删除记录'}</button>
          {/if}
          {#if j.status !== 'rolledBack' && !j.error}
              {#if confirming === j.id}
                <button class="btn danger sm" disabled={busyId === j.id} onclick={() => undo(j)}>{busyId === j.id ? '撤销中…' : '确认撤销'}</button>
                <button class="btn ghost sm" onclick={() => (confirming = null)}>取消</button>
              {:else}
                <button class="btn sm" onclick={() => (confirming = j.id)}><Icon name="undo" size={13} />撤销</button>
              {/if}
          {/if}
          </div>
          {#if detail === j.id}<HistoryOperations id={j.id} />{/if}
          {#if j.pendingOperations}<p class="muted">有 {j.pendingOperations} 项中断操作待核对；撤销时将先检查磁盘状态。</p>{/if}
          {#if j.error}<p class="muted" role="alert">日志无法读取：{j.error}。请保留日志与文件，修复日志后再尝试。</p>{/if}
        </li>
      {/each}
    </ul>
    {#if next}
      <button class="btn more" disabled={loading || loadingMore} onclick={loadMore}>
        {#if loadingMore}<Icon name="loader" size={14} />正在读取…{:else}加载更早的记录（还有 {Math.max(total - list.length, 0)} 条）{/if}
      </button>
    {/if}
  {/if}
</div>

<style>
  .history-filters { display:flex; align-items:end; gap:var(--s2); flex-wrap:wrap; margin-bottom:var(--s4); }
  .history-filters > input { flex:1; min-width:200px; }
  .history-filters label { display:grid; gap:4px; font-size:var(--fs-xs); }

  .page { max-width: 860px; margin: 0 auto; padding: var(--s6) var(--s5) var(--s7); }
  header { margin-bottom:var(--s5); display:grid; grid-template-columns:1fr auto; gap:8px 20px; align-items:start; }
  header h1 { grid-column:1; }
  header>.btn { grid-column:2; grid-row:1; }
  .tipped { display:flex; align-items:center; gap:var(--s2); }
  .maintenance[hidden] { display:none; }
  h1 { font-size: var(--fs-2xl); font-weight: 700; letter-spacing: -0.015em; }
  header p { font-size: var(--fs-sm); max-width: 560px; }
  .more { display: flex; margin: var(--s4) auto 0; }
  ul { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: var(--s3); }
  .item { display: grid; grid-template-columns:minmax(0,1fr) auto; align-items: center; gap:12px 20px; padding: var(--s4) var(--s5); transition: transform var(--d-base) var(--ease-back), background var(--d-fast); }
  .item:hover { transform: translateY(-2px); background: var(--surface-2); }
  .code { transition: color var(--d-fast); }
  .item:hover .code { color: var(--accent-hover); }
  .item>.muted { grid-column:1/-1; }
  .act { grid-column:2; }
  .main { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: var(--s2); }
  .row1 { display: flex; align-items: center; gap: var(--s3); flex-wrap: wrap; }
  .when { font-weight: 600; white-space: nowrap; }
  .codes { display: flex; flex-wrap: wrap; gap: var(--s1) var(--s3); font-size: var(--fs-sm); }
  .code { color: var(--accent); }
  .act { display: flex; flex-wrap: wrap; gap: var(--s2); justify-content: flex-end; }
  @media (max-width: 720px) { .page { padding: var(--s4) var(--s4) 96px; } .item { grid-template-columns:1fr; align-items:stretch; } .act { grid-column:1; } }
</style>
