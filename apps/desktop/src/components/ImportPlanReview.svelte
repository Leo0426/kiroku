<script lang="ts">
  import { baseName } from '../lib/format'
  import { flow } from '../lib/importflow.svelte'
  import Icon from './Icon.svelte'
  import SupplementFiles from './SupplementFiles.svelte'
  import StatusPill from './StatusPill.svelte'

  let { root, omittedCount, supplementBusy=$bindable(false) }: { root: string; omittedCount: number; supplementBusy?:boolean } = $props()
  type Scope = 'all' | 'conflicts' | 'versions'
  let scope = $state<Scope>('all')
  let query = $state('')
  let expanded = $state<number | null>(null)
  let suffix = $state('')
  function toggleItem(id: number) {
    expanded = expanded === id ? null : id
    suffix = flow.renames[id] ?? ''
  }
  const needle = $derived(query.trim().toLowerCase())
  const versions = $derived(flow.plans.filter(p => p.newVersion && !p.conflict).length)
  const filtered = $derived(scope !== 'all' || !!needle)
  const shown = $derived(flow.plans.filter(p => {
    if (scope === 'conflicts' && !p.conflict) return false
    if (scope === 'versions' && (!p.newVersion || p.conflict)) return false
    return !needle || [p.code, p.title, p.from, p.to].some(value => value?.toLowerCase().includes(needle))
  }))
  const selectable = $derived(shown.filter(p => !p.conflict && p.actions?.length !== 0))
  const checked = $derived(selectable.filter(p => flow.picked[p.itemId]).length)
  const hiddenChecked = $derived(flow.selectedCount - checked)
  const rootPrefix = $derived(root.replace(/[\\/]+$/, ''))
  const relative = (path: string) => root && (path.startsWith(rootPrefix + '/') || path.startsWith(rootPrefix + '\\'))
    ? path.slice(rootPrefix.length + 1) : path

  function selectShown(value: boolean) {
    if (!filtered) return flow.setAll(value)
    for (const item of selectable) flow.picked[item.itemId] = value
  }
</script>

<section class="card review" aria-label="整理计划">
  <div class="review-head">
    <div class="title-line">
      <h2>整理清单</h2>
      <span class="muted total">{filtered ? `显示 ${shown.length} / ${flow.plans.length}` : `共 ${flow.plans.length}`} 个文件{#if omittedCount > 0} · 另有 {omittedCount} 部资料未就绪{/if}</span>
    </div>
    {#if flow.conflictCount || versions || flow.plans.length > 1}
      <div class="tools">
        {#if flow.conflictCount || versions}
          <div class="scopes" role="group" aria-label="计划筛选">
            <button class="scope" aria-pressed={scope === 'all'} onclick={() => scope = 'all'}>全部 <span>{flow.plans.length}</span></button>
            {#if flow.conflictCount}
              <button class="scope" aria-pressed={scope === 'conflicts'} onclick={() => scope = 'conflicts'}>有冲突 <span>{flow.conflictCount}</span></button>
            {/if}
            {#if versions}
              <button class="scope" aria-pressed={scope === 'versions'} onclick={() => scope = 'versions'}>新版本 <span>{versions}</span></button>
            {/if}
          </div>
        {/if}
        {#if flow.plans.length > 1}
          <label class="search"><Icon name="search" size={15} /><input type="search" aria-label="搜索整理清单" placeholder="查找番号、片名或路径" bind:value={query} /></label>
        {/if}
      </div>
    {/if}
  </div>
  <div class="selection">
    <span aria-live="polite">已选 <b class="tnum">{flow.selectedCount}</b> 个{#if hiddenChecked}，其中 {hiddenChecked} 个在筛选结果外{/if}</span>
    <div class="selection-actions">
      <button class="btn sm ghost" disabled={checked === selectable.length} onclick={() => selectShown(true)}>{filtered ? '全选当前结果' : '全选'}</button>
      <button class="btn sm ghost" disabled={!checked} onclick={() => selectShown(false)}>{filtered ? '取消选择当前结果' : '全不选'}</button>
    </div>
  </div>

  <div class="plan-list">
    {#each shown as p (p.itemId)}
      <div class="planrow" class:off={!flow.picked[p.itemId]} class:conflict={p.conflict}>
        <input id={"plan-pick-"+p.itemId} class="checkbox" type="checkbox" disabled={!!p.conflict || p.actions?.length === 0 || flow.busy} bind:checked={flow.picked[p.itemId]} aria-label="整理 {p.code}" />
        <label class="head" for={"plan-pick-"+p.itemId}>
          <span class="mono code">{p.code}</span>
          <span class="ttl" title={p.title ?? ''}>{p.title ?? ''}</span>
        </label>
        <label class="paths" for={"plan-pick-"+p.itemId}>
          <span class="from" title={p.from}>{baseName(p.from)}</span>
          <span class="mono to" title={p.to}>→ {relative(p.to)}</span>
          {#each p.conflicts ?? [] as c (c.path)}<span class="conflict-path" title={c.path}>{c.kind === 'invalidTemplate' ? '命名规则' : c.kind === 'invalidLibrary' ? '库不可用' : c.kind === 'targetExists' ? '已占用' : '本批重复'}：{relative(c.path)}</span>{/each}
        </label>
        <div class="end">
          {#if p.conflict}
            <StatusPill tone="warn" title={p.notices?.join('\n') || (p.conflicts ?? []).map(c => c.path).join('\n') || p.to}>{flow.skipped[p.itemId] ? '已跳过' : p.conflict === 'invalidTemplate' ? '请调整本批规则' : p.conflict === 'invalidLibrary' ? '请先检查媒体库' : p.conflict === 'targetExists' ? '目标已存在' : '批内重名'}</StatusPill>
          {:else if p.newVersion}
            <StatusPill tone="info" title={p.notices?.join('\n') ?? ''}>{flow.picked[p.itemId] ? '新版本 · 已选' : '新版本 · 需手动勾选'}</StatusPill>
          {:else if p.notices?.length}
            <span class="extras" title={p.notices.join('\n')}>{p.notices[0]}</span>
          {:else if p.extras.length}
            <span class="extras" title={p.extras.join('\n')}>+ {p.extras.length} 个文件</span>
          {/if}
          <button class="btn sm ghost" disabled={flow.busy || supplementBusy} aria-expanded={expanded === p.itemId} onclick={() => toggleItem(p.itemId)}>{p.conflict ? '处理此项' : '操作明细'}</button>
        </div>
        {#if expanded === p.itemId}
          <div class="item-detail">
            <p class="mono">{p.from} → {p.to}</p>
            {#if p.actions?.length}<ul>{#each p.actions as action}<li>{action}</li>{/each}</ul>{/if}
            {#each p.notices ?? [] as notice}<p>{notice}</p>{/each}
            {#if p.conflict === 'targetExists' || p.conflict === 'duplicateInBatch' || flow.renames[p.itemId]}
              <form onsubmit={e => { e.preventDefault(); void flow.renameItem(p.itemId, suffix.trim()) }}>
                <label>另存文件名后缀<input class="input" bind:value={suffix} placeholder="例如 修订版" /></label>
                <button class="btn sm" disabled={flow.busy || supplementBusy || !suffix.trim()}>改名并重新预览</button>
              </form>
            {/if}
            {#if p.existingMovie || p.conflict === 'targetExists' || p.newVersion}
              <SupplementFiles code={p.code} itemId={p.itemId} onbusy={busy=>supplementBusy=busy} onsaved={() => void flow.buildPlan()} />
            {/if}
            {#if p.conflict}
              <button class="btn sm" disabled={flow.busy || supplementBusy || flow.skipped[p.itemId]} onclick={() => { flow.skipped[p.itemId] = true; flow.picked[p.itemId] = false; expanded = null }}>跳过此项</button>
            {/if}
          </div>
        {/if}
      </div>
    {:else}
      <div class="no-match">
        {#if filtered}
          <span>没有匹配的文件</span><button class="btn sm" onclick={() => { query = ''; scope = 'all' }}>查看全部文件</button>
        {:else}
          <span>没有可预览的文件，请返回核对资料。</span>
        {/if}
      </div>
    {/each}
  </div>
</section>

<style>
  .review { min-width: 0; container-type: inline-size; }
  .review-head { padding: var(--s4); display: grid; gap: var(--s3); }
  h2 { font-size: var(--fs-lg); font-weight: 600; }
  .title-line, .tools, .selection, .selection-actions, .scopes, .scope, .search, .no-match { display: flex; align-items: center; }
  .title-line, .tools, .selection { gap: var(--s2) var(--s3); flex-wrap: wrap; justify-content: space-between; }
  .total, .selection { font-size: var(--fs-sm); }
  .scopes { flex-wrap: wrap; gap: var(--s1); }
  .scope { gap: var(--s1); border: 0; border-radius: var(--r-sm); background: transparent; padding: var(--s1) var(--s3); min-height: 30px; color: var(--text-2); font-size: var(--fs-sm); }
  .scope:hover { background: var(--surface-2); }
  .scope[aria-pressed="true"] { background: var(--accent-soft); color: var(--text-1); font-weight: 600; }
  .scope span { font-variant-numeric: tabular-nums; font-size: var(--fs-xs); }
  .search { flex: 1; max-width: 270px; min-width: min(180px,100%); gap: var(--s2); background: var(--surface-2); color: var(--text-2); padding: 0 var(--s3); border-radius: var(--r-sm); }
  .search:focus-within { box-shadow: inset 0 0 0 1.5px var(--accent); }
  .search input { width: 100%; min-width: 0; height: 32px; outline: none; border: 0; background: transparent; font-size: var(--fs-sm); }
  .selection { padding: var(--s2) var(--s4); background: var(--surface-2); color: var(--text-2); }
  .selection-actions { flex-wrap: wrap; gap: var(--s1); }
  .selection b { color: var(--text-1); }
  .plan-list { max-height: 52vh; overflow: auto; padding: 0 var(--s4); }
  .planrow { display: grid; grid-template-columns: auto minmax(100px, .65fr) minmax(0, 1.5fr) minmax(100px, .5fr); gap: var(--s3); align-items: center; padding: var(--s3) var(--s1); cursor: pointer; }
  .planrow + .planrow { border-top: 1px solid var(--line); }
  .planrow:hover { background: var(--surface-2); }
  .planrow.conflict { cursor: default; }
  .head, .paths { display: grid; gap: 2px; min-width: 0; }
  .code { color: var(--accent); font-weight: 600; }
  .planrow.off .code, .from { color: var(--text-2); }
  .ttl, .paths, .extras { color: var(--text-2); font-size: var(--fs-xs); }
  .ttl, .paths span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .paths .conflict-path { white-space: normal; overflow-wrap: anywhere; color: var(--warn); }
  .to { color: var(--text-1); }
  .end { display:flex; flex-wrap:wrap; justify-content:flex-end; align-items:center; gap:var(--s1); text-align:right; }
  .item-detail { grid-column:1 / -1; display:grid; gap:var(--s2); padding:var(--s3); background:var(--surface-2); border-radius:var(--r-sm); font-size:var(--fs-sm); overflow-wrap:anywhere; }
  .item-detail ul { margin:0; padding-inline-start:1.4rem; color:var(--text-2); }
  .item-detail form { display:flex; align-items:end; flex-wrap:wrap; gap:var(--s2); }
  .item-detail label { display:grid; gap:3px; }
  .item-detail > button { justify-self:start; }
  .no-match { justify-content: center; flex-wrap: wrap; gap: var(--s3); padding: var(--s5) var(--s2); font-size: var(--fs-sm); }
  @container (max-width: 590px) {
    .planrow { grid-template-columns: auto minmax(0,1fr) auto; gap: var(--s2); }
    .paths { grid-column: 2 / -1; grid-row: 2; }
    .end:empty { display: none; }
    .end { max-width: 130px; }
  }
  @container (max-width: 430px) {
    .search { max-width: none; flex-basis: 100%; }
    .selection { justify-content: flex-start; }
  }
</style>
