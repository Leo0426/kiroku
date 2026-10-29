<script lang="ts">
  import { overviewRequest } from '../lib/overview-requests'
  import { app } from '../lib/state.svelte'
  import { fmtBytes } from '../lib/format'
  import type { LibraryOverview, Query } from '../lib/types'
  let { query }: { query:Query } = $props()
  let data = $state<LibraryOverview | null>(null)
  let error = $state('')
  let retry = $state(0)
  $effect(() => {
    const q = $state.snapshot(query)
    app.settings?.library_root; app.libraryVersion; retry
    let alive = true
    const controller = new AbortController()
    data = null; error = ''
    overviewRequest(q,controller.signal).then(result => { if (alive) data = result }).catch(e => { if (alive) error = String(e) })
    return () => { alive = false; controller.abort() }
  })
</script>
<section aria-label="当前筛选概览">
  <h3>当前筛选概览</h3>
  {#if error}<p role="alert">{error}</p><button class="btn sm" onclick={() => retry++}>重试概览</button>
  {:else if !data}<p role="status">正在统计全部匹配影片…</p>
  {:else}
    <dl>
      <div><dt>影片</dt><dd>{data.movies.toLocaleString()}</dd></div>
      <div><dt>文件</dt><dd>{data.files.toLocaleString()}</dd></div>
      <div><dt>逻辑体积</dt><dd>{fmtBytes(data.bytes)}</dd></div>
      <div><dt>收藏</dt><dd>{data.favorites.toLocaleString()}</dd></div>
      <div><dt>平均评分</dt><dd>{data.averageScore === null ? '—' : (data.averageScore / 10).toFixed(1)}</dd></div>
    </dl>
    <div class="quality" aria-label="信息完整度">
      {#each [['缺海报', data.missingPoster], ['缺年份', data.missingYear], ['缺评分', data.missingScore]] as [label,count]}
        <label>{label}<span>{count} / {data.movies}</span><meter min="0" max={Math.max(1,data.movies)} value={Number(count)} aria-label={String(label)}></meter></label>
      {/each}
    </div>
    <p class="faint">统计全部匹配结果，不受分页影响。体积来自索引；硬链接可能共享磁盘空间。</p>
  {/if}
</section>
<style>
  section { padding:16px 20px; border-bottom:1px solid var(--line); }
  h3 { font-size:var(--fs-sm); margin:0 0 12px; }
  dl { display:grid; grid-template-columns:repeat(auto-fit,minmax(100px,1fr)); gap:12px; margin:0 0 16px; }
  dt { color:var(--text-2); font-size:var(--fs-xs); } dd { margin:2px 0 0; font-size:var(--fs-lg); font-variant-numeric:tabular-nums; }
  .quality { display:flex; gap:16px; flex-wrap:wrap; margin-bottom:12px; }
  label { flex:1; min-width:120px; display:grid; grid-template-columns:1fr auto; gap:4px; font-size:var(--fs-xs); }
  meter { grid-column:1/-1; width:100%; } p { font-size:var(--fs-xs); }
</style>
